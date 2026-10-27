//! A tab of the stage that is a terminal: a shell the human types into,
//! or a command the agent runs with Cox's tool for that (see
//! [`crate::mcp`]), which the human watches run, and can type into too.
//!
//! The terminal is `alacritty_terminal`'s: it runs the program on a
//! pseudoterminal, reads what the program writes on a thread of its own,
//! and keeps the grid of cells that makes. The tab draws the grid, and
//! sends the program the human's keys, the text their input method
//! commits, and what they paste. The wheel scrolls back through what
//! scrolled off, and a drag selects text to copy.
//!
//! A command the agent runs answers, once it ends, what its terminal shows
//! and how it ended. When the agent stops waiting for it, as when the
//! human stops the agent's turn, the command gets an interrupt, as from
//! Control-C.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::os::fd::{FromRawFd as _, OwnedFd};
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as Cell, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui::{
    App, Bounds, ClipboardItem, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, FontStyle, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, KeyBinding, KeyDownEvent, Keystroke, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    ShapedLine, SharedString, Style, Task, TextRun, UTF16Selection, UnderlineStyle, Window, div,
    fill, font, outline, point, prelude::*, px, relative, rgb, size,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::input::Paste;
use crate::mcp::{Ended, Ran};
use crate::ui::Copy;
use crate::ui::paths::shortened;
use crate::ui::theme::{BACKGROUND, DIM, FAILED, FONT, SELECTION};

const KEY_CONTEXT: &str = "Terminal";

const FONT_SIZE: f32 = 13.;
const LINE_HEIGHT: f32 = 1.3;

/// Room between the grid and the tab's edges.
const PADDING: f32 = 6.;

/// How many lines the terminal keeps of what scrolled off.
const SCROLLBACK: usize = 10_000;

/// How often a command the agent runs looks whether the agent still waits
/// for it.
const WATCH_EVERY: Duration = Duration::from_millis(250);

/// The size a terminal starts at, before the tab is laid out.
const START_SIZE: Size = Size {
    columns: 80,
    lines: 24,
};

/// Brighter than the window's, as a terminal's text is.
const FOREGROUND: u32 = 0xe0_e0e0;
const CURSOR: u32 = 0xc0_c0c0;
const SUCCEEDED: u32 = 0x80_d080;

/// The sixteen colors programs name: black, red, green, yellow, blue,
/// magenta, cyan and white, then the bright of each.
const PALETTE: [u32; 16] = [
    0x3a_3a3a, 0xe0_6c75, 0x98_c379, 0xe5_c07b, 0x61_afef, 0xc6_78dd, 0x56_b6c2, 0xc8_c8c8,
    0x6a_6a6a, 0xff_7b86, 0xb5_e890, 0xff_d68a, 0x7c_c4ff, 0xde_8ef5, 0x6f_d3df, 0xff_ffff,
];

/// Binds the keys of the tab: copying and pasting with the secondary key,
/// which is Command on macOS. Elsewhere it is Control, which the program
/// gets as it does in any terminal, so there Shift goes with it, and the
/// window's own Control keys go to the program too.
pub fn bind_keys(cx: &mut App) {
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(KEY_CONTEXT)),
    ]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-v", Paste, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-n", gpui::NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-o", gpui::NoAction, Some(KEY_CONTEXT)),
    ]);
}

/// What a terminal runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    /// The human's shell, to type into.
    Shell,
    /// A command the agent runs, with the human's shell.
    Command(String),
}

/// The terminal's size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    columns: usize,
    lines: usize,
}

/// The size of a cell, before the tab is laid out.
fn start_cell() -> gpui::Size<Pixels> {
    size(px(8.), (px(FONT_SIZE) * LINE_HEIGHT).round())
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl Size {
    /// As the pseudoterminal is told it, with cells of `cell`.
    fn window_size(self, cell: gpui::Size<Pixels>) -> WindowSize {
        let whole = |count: usize| u16::try_from(count).unwrap_or(u16::MAX);
        WindowSize {
            num_lines: whole(self.lines),
            num_cols: whole(self.columns),
            cell_width: whole_pixels(cell.width),
            cell_height: whole_pixels(cell.height),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a cell is a few pixels; rounding is the point"
)]
fn whole_pixels(pixels: Pixels) -> u16 {
    f32::from(pixels).round().max(0.) as u16
}

/// Hands what the terminal tells of itself to the tab.
#[derive(Clone)]
struct Listener(UnboundedSender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

/// A program running on a pseudoterminal, before a tab shows it.
pub struct Running {
    program: Program,
    cwd: PathBuf,
    term: Arc<FairMutex<Term<Listener>>>,
    pty: EventLoopSender,
    events: UnboundedReceiver<Event>,
    /// The program's end of the pseudoterminal, held open.
    _held: OwnedFd,
}

impl Running {
    /// Starts `program` working in `cwd`.
    ///
    /// # Errors
    ///
    /// When the pseudoterminal cannot be made or the program cannot start.
    pub fn start(program: Program, cwd: &Path) -> std::io::Result<Self> {
        let (sent, events) = unbounded_channel();
        let listener = Listener(sent);
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &START_SIZE,
            listener.clone(),
        )));
        let shell = match &program {
            // The human's login shell, as a terminal starts it.
            Program::Shell => None,
            Program::Command(command) => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned());
                Some(tty::Shell::new(
                    shell,
                    vec!["-c".to_owned(), command.clone()],
                ))
            }
        };
        let options = tty::Options {
            shell,
            working_directory: Some(cwd.to_owned()),
            // All the output, before the command answers what it printed.
            drain_on_exit: true,
            env: environment(),
        };
        let (controller, user) = open_pty(START_SIZE.window_size(start_cell()))?;
        // Cox holds the program's end open as well, so that its own end
        // never reads as hung up: on Linux, the terminal then drops what
        // the program wrote last, when it reads that after the hang-up.
        let held = user.try_clone()?;
        crate::agent::close_on_exec();
        let pty = tty::from_fd(&options, 0, controller, user)?;
        let event_loop = EventLoop::new(Arc::clone(&term), listener, pty, true, false)?;
        let sender = event_loop.channel();
        // The thread ends as the program does, or as the tab goes.
        drop(event_loop.spawn());
        Ok(Self {
            program,
            cwd: cwd.to_owned(),
            term,
            pty: sender,
            events,
            _held: held,
        })
    }
}

/// What a terminal's program finds in its environment, past Cox's own:
/// what kind of terminal it is in, and that characters are UTF-8 when
/// nothing says what they are, as with a Cox started from the Finder.
/// Without that, a shell takes each byte of a Japanese character for a
/// character of its own.
fn environment() -> HashMap<String, String> {
    let mut environment = HashMap::from([
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
    ]);
    let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if !set("LC_ALL") && !set("LC_CTYPE") && !set("LANG") {
        let utf8 = if cfg!(target_os = "macos") {
            "UTF-8"
        } else {
            "C.UTF-8"
        };
        environment.insert("LC_CTYPE".to_owned(), utf8.to_owned());
    }
    environment
}

/// Opens a pseudoterminal of `size`: the terminal's end, and the
/// program's.
fn open_pty(size: WindowSize) -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut window = libc::winsize {
        ws_row: size.num_lines,
        ws_col: size.num_cols,
        ws_xpixel: size.num_cols.saturating_mul(size.cell_width),
        ws_ypixel: size.num_lines.saturating_mul(size.cell_height),
    };
    let (mut controller, mut user) = (-1, -1);
    // SAFETY: `openpty` writes the two descriptors it opens, and reads the
    // size; no name is asked for, nor terminal settings given.
    let opened = unsafe {
        libc::openpty(
            &raw mut controller,
            &raw mut user,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut window,
        )
    };
    if opened != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `openpty` opened both, and nothing else owns them.
    let (controller, user) =
        unsafe { (OwnedFd::from_raw_fd(controller), OwnedFd::from_raw_fd(user)) };
    Ok((controller, user))
}

/// Where the grid was last drawn, to find the cell under the mouse.
#[derive(Clone, Copy, Debug)]
struct Laid {
    origin: Point<Pixels>,
    cell: gpui::Size<Pixels>,
}

pub struct Terminal {
    program: Program,
    cwd: PathBuf,
    term: Arc<FairMutex<Term<Listener>>>,
    pty: EventLoopSender,
    size: Size,
    window_size: WindowSize,
    laid: Option<Laid>,
    /// What the program calls its terminal, if it said.
    title: Option<String>,
    /// How the program exited, once it did.
    status: Option<ExitStatus>,
    /// How the program ended, once it did.
    ended: Option<Ended>,
    /// Where a command the agent runs answers, until it ends.
    answer: Option<oneshot::Sender<Result<Ran, String>>>,
    /// Text the input method is composing, if any.
    composing: Option<String>,
    /// Lines of a wheel scroll not scrolled yet.
    scrolling: f32,
    focus_handle: FocusHandle,
    /// The program's end of the pseudoterminal, held open while the tab
    /// lasts.
    _held: OwnedFd,
    _events: Task<()>,
    _watch: Option<Task<()>>,
}

impl Terminal {
    /// A tab showing `running`, which answers `answer` once it ends.
    pub fn new(
        running: Running,
        answer: Option<oneshot::Sender<Result<Ran, String>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let Running {
            program,
            cwd,
            term,
            pty,
            mut events,
            _held: held,
        } = running;
        let heard = cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |terminal, cx| terminal.heard(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let watch = answer.is_some().then(|| {
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(WATCH_EVERY).await;
                    match this.update(cx, |terminal, _| terminal.watch()) {
                        Ok(true) => {}
                        _ => break,
                    }
                }
            })
        });
        Self {
            program,
            cwd,
            term,
            pty,
            size: START_SIZE,
            window_size: START_SIZE.window_size(start_cell()),
            laid: None,
            title: None,
            status: None,
            ended: None,
            answer,
            composing: None,
            scrolling: 0.,
            focus_handle: cx.focus_handle(),
            _held: held,
            _events: heard,
            _watch: watch,
        }
    }

    #[must_use]
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// How the program ended, once it did.
    #[must_use]
    pub fn ended(&self) -> Option<Ended> {
        self.ended
    }

    /// The name the tab goes by: the command, or what the shell calls its
    /// terminal.
    #[must_use]
    pub fn title(&self) -> String {
        match &self.program {
            Program::Command(command) => command.lines().next().unwrap_or_default().to_owned(),
            Program::Shell => self
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| "Terminal".to_owned()),
        }
    }

    /// What the terminal shows, with what scrolled off, as text: a line
    /// the terminal wrapped is one line.
    #[must_use]
    pub fn text(&self) -> String {
        let term = self.term.lock();
        let start = Cell::new(term.topmost_line(), Column(0));
        let end = Cell::new(term.bottommost_line(), term.last_column());
        term.bounds_to_string(start, end).trim_end().to_owned()
    }

    /// The text selected.
    #[must_use]
    pub fn selected_text(&self) -> Option<String> {
        self.term.lock().selection_to_string()
    }

    /// What the terminal tells of itself.
    fn heard(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
            Event::Title(title) => self.title = Some(title),
            Event::ResetTitle => self.title = None,
            Event::PtyWrite(text) => self.write(text.into_bytes()),
            Event::ClipboardStore(_, text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            // Programs do not read the clipboard: the default keeps them
            // from asking.
            Event::ClipboardLoad(..) | Event::Bell => return,
            Event::ColorRequest(index, format) => {
                let color = color_at(index, self.term.lock().colors());
                self.write(format(color).into_bytes());
            }
            Event::TextAreaSizeRequest(format) => self.write(format(self.window_size).into_bytes()),
            Event::ChildExit(status) => self.status = Some(status),
            Event::Exit => self.exited(),
        }
        cx.notify();
    }

    /// The program ended, and the terminal holds all it wrote.
    fn exited(&mut self) {
        let ended = match self.status {
            Some(status) => status.code().map_or_else(
                || status.signal().map_or(Ended::Unknown, Ended::Signal),
                Ended::Exited,
            ),
            None => Ended::Unknown,
        };
        self.ended = Some(ended);
        self.answer(ended);
    }

    /// Answers the agent how its command went, if it waits.
    fn answer(&mut self, ended: Ended) {
        if let Some(answer) = self.answer.take() {
            let output = self.text();
            let _ = answer.send(Ok(Ran { output, ended }));
        }
    }

    /// Looks whether the agent still waits for its command, and interrupts
    /// the command if not. Answers whether to look again.
    fn watch(&mut self) -> bool {
        match &self.answer {
            Some(answer) if answer.is_closed() => {
                self.answer = None;
                self.write(b"\x03".to_vec());
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    /// Sends `bytes` to the program.
    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.pty.send(Msg::Input(bytes.into()));
    }

    /// Sends what the human typed to the program: the terminal shows its
    /// end again, with nothing selected.
    fn type_in(&mut self, bytes: impl Into<Cow<'static, [u8]>>, cx: &mut Context<Self>) {
        {
            let mut term = self.term.lock();
            term.selection = None;
            term.scroll_display(Scroll::Bottom);
        }
        self.write(bytes);
        cx.notify();
    }

    /// Fits the terminal to `size`, in cells of `cell`.
    fn resize(&mut self, size: Size, cell: gpui::Size<Pixels>) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.window_size = size.window_size(cell);
        self.term.lock().resize(size);
        let _ = self.pty.send(Msg::Resize(self.window_size));
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        // While the input method composes, the keys are its own.
        if self.composing.is_some() {
            return;
        }
        let mode = *self.term.lock().mode();
        if let Some(bytes) = key_bytes(&event.keystroke, mode) {
            self.type_in(bytes, cx);
            cx.stop_propagation();
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text().filter(|text| !text.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        self.type_in(pasted(&text, bracketed), cx);
    }

    /// The cell under `position`, and which half of it.
    fn cell_at(&self, position: Point<Pixels>) -> Option<(Cell, Side)> {
        let laid = self.laid?;
        let x = (position.x - laid.origin.x) / laid.cell.width;
        let y = (position.y - laid.origin.y) / laid.cell.height;
        let term = self.term.lock();
        let column = whole_cells(x).min(term.columns() - 1);
        let line = whole_cells(y).min(term.screen_lines() - 1);
        let line = i32::try_from(line).unwrap_or(0)
            - i32::try_from(term.grid().display_offset()).unwrap_or(0);
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        Some((Cell::new(Line(line), Column(column)), side))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let Some((cell, side)) = self.cell_at(event.position) else {
            return;
        };
        let kind = match event.click_count {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.term.lock().selection = Some(Selection::new(kind, cell, side));
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some((cell, side)) = self.cell_at(event.position) else {
            return;
        };
        if let Some(selection) = self.term.lock().selection.as_mut() {
            selection.update(cell, side);
        }
        cx.notify();
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mut term = self.term.lock();
        // A click with no drag selects nothing.
        if term.selection.as_ref().is_some_and(Selection::is_empty) {
            term.selection = None;
            cx.notify();
        }
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line = self
            .laid
            .map_or(start_cell().height, |laid| laid.cell.height);
        self.scrolling += match event.delta {
            ScrollDelta::Lines(lines) => lines.y,
            ScrollDelta::Pixels(pixels) => pixels.y / line,
        };
        let lines = self.scrolling.trunc();
        if lines == 0. {
            return;
        }
        self.scrolling -= lines;
        let lines = whole_lines(lines);
        let mut term = self.term.lock();
        let mode = *term.mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // A full-screen program, such as a pager, scrolls itself.
            let key = if lines > 0 { "A" } else { "B" };
            let prefix = if mode.contains(TermMode::APP_CURSOR) {
                "\x1bO"
            } else {
                "\x1b["
            };
            drop(term);
            let times = usize::try_from(lines.unsigned_abs()).unwrap_or(0);
            self.write(format!("{prefix}{key}").repeat(times).into_bytes());
        } else {
            term.scroll_display(Scroll::Delta(lines));
        }
        cx.notify();
    }

    fn render_bar(&self) -> impl IntoElement {
        let (what, state) = match &self.program {
            Program::Command(command) => {
                let state = match self.ended {
                    None => ("running".to_owned(), DIM),
                    Some(Ended::Exited(0)) => (Ended::Exited(0).to_string(), SUCCEEDED),
                    Some(ended) => (ended.to_string(), FAILED),
                };
                (
                    format!("$ {}", command.lines().next().unwrap_or_default()),
                    Some(state),
                )
            }
            Program::Shell => {
                let state = self.ended.map(|_| ("exited".to_owned(), DIM));
                (shortened(&self.cwd), state)
            }
        };
        div()
            .flex()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(0x40_4040))
            .text_color(rgb(DIM))
            .child(
                div()
                    .debug_selector(|| "terminal-program".to_owned())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(what),
            )
            .children(state.map(|(state, color)| {
                div()
                    .debug_selector(|| "terminal-state".to_owned())
                    .flex_shrink_0()
                    .text_color(rgb(color))
                    .child(state)
            }))
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a count of cells on the screen, floored, and none below 0"
)]
fn whole_cells(cells: f32) -> usize {
    cells.floor().max(0.) as usize
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a wheel scrolls a few lines at a time"
)]
fn whole_lines(lines: f32) -> i32 {
    lines as i32
}

impl Drop for Terminal {
    /// The program goes with its tab: the terminal hangs up on it. A
    /// command the agent waits for answers what it printed so far.
    fn drop(&mut self) {
        self.answer(Ended::Closed);
        let _ = self.pty.send(Msg::Shutdown);
    }
}

impl Focusable for Terminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// What the program gets for the key `keystroke`, in a terminal in
/// `mode`, or `None` for a key that types text, which reaches the tab as
/// text, and for the window's own shortcuts.
fn key_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Cow<'static, [u8]>> {
    let modifiers = keystroke.modifiers;
    if modifiers.platform || modifiers.function {
        return None;
    }
    // As xterm adds them to a key's sequence: 1, plus Shift 1, Alt 2,
    // Control 4.
    let added = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let cursor = |key: char| -> Cow<'static, [u8]> {
        if added > 1 {
            format!("\x1b[1;{added}{key}").into_bytes().into()
        } else if mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{key}").into_bytes().into()
        } else {
            format!("\x1b[{key}").into_bytes().into()
        }
    };
    let function = |number: u8| -> Cow<'static, [u8]> {
        if added > 1 {
            format!("\x1b[{number};{added}~").into_bytes().into()
        } else {
            format!("\x1b[{number}~").into_bytes().into()
        }
    };
    let fixed = |bytes: &'static [u8]| Some(Cow::Borrowed(bytes));
    match keystroke.key.as_str() {
        "up" => Some(cursor('A')),
        "down" => Some(cursor('B')),
        "right" => Some(cursor('C')),
        "left" => Some(cursor('D')),
        "home" => Some(cursor('H')),
        "end" => Some(cursor('F')),
        "f1" => Some(cursor('P')),
        "f2" => Some(cursor('Q')),
        "f3" => Some(cursor('R')),
        "f4" => Some(cursor('S')),
        "insert" => Some(function(2)),
        "delete" => Some(function(3)),
        "pageup" => Some(function(5)),
        "pagedown" => Some(function(6)),
        "f5" => Some(function(15)),
        "f6" => Some(function(17)),
        "f7" => Some(function(18)),
        "f8" => Some(function(19)),
        "f9" => Some(function(20)),
        "f10" => Some(function(21)),
        "f11" => Some(function(23)),
        "f12" => Some(function(24)),
        "enter" if modifiers.alt => fixed(b"\x1b\r"),
        "enter" => fixed(b"\r"),
        "tab" if modifiers.shift => fixed(b"\x1b[Z"),
        "tab" if modifiers.control => None,
        "tab" => fixed(b"\t"),
        "escape" => fixed(b"\x1b"),
        "backspace" if modifiers.control => fixed(b"\x08"),
        "backspace" if modifiers.alt => fixed(b"\x1b\x7f"),
        "backspace" => fixed(b"\x7f"),
        "space" if modifiers.control => fixed(b"\0"),
        key if modifiers.control => {
            let mut chars = key.chars();
            let (Some(key), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let byte = control(key)?;
            Some(if modifiers.alt {
                vec![0x1b, byte].into()
            } else {
                vec![byte].into()
            })
        }
        // Option types characters on macOS; elsewhere Alt is the Meta key
        // that shells read words with.
        _ if modifiers.alt && !cfg!(target_os = "macos") => {
            let typed = keystroke.key_char.as_deref().unwrap_or(&keystroke.key);
            Some(format!("\x1b{typed}").into_bytes().into())
        }
        _ => None,
    }
}

/// The byte Control with `key` makes.
fn control(key: char) -> Option<u8> {
    let key = key.to_ascii_lowercase();
    Some(match key {
        'a'..='z' => key as u8 - b'a' + 1,
        '@' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// What a paste of `text` sends: between the marks that tell a program it
/// is pasted, for one that asked for them, or with the line breaks as
/// Enter.
fn pasted(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // The text must not end the paste early.
        let text = text.replace('\x1b', "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// The color at `index` of the terminal's: the program's own if it set
/// one, or Cox's.
fn color_at(index: usize, colors: &Colors) -> Rgb {
    if let Some(color) = colors[index] {
        return color;
    }
    let named = |color: u32| {
        let [_, r, g, b] = color.to_be_bytes();
        Rgb { r, g, b }
    };
    match index {
        0..16 => named(PALETTE[index]),
        16..232 => {
            let level = |value: usize| [0, 95, 135, 175, 215, 255][value % 6];
            let index = index - 16;
            Rgb {
                r: level(index / 36),
                g: level(index / 6),
                b: level(index),
            }
        }
        232..256 => {
            let gray = 8 + 10 * u8::try_from(index - 232).unwrap_or(0);
            Rgb {
                r: gray,
                g: gray,
                b: gray,
            }
        }
        _ => match index {
            i if i == NamedColor::Background as usize => named(BACKGROUND),
            i if i == NamedColor::Cursor as usize => named(CURSOR),
            i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
                dim(color_at(i - NamedColor::DimBlack as usize, colors))
            }
            i if i == NamedColor::DimForeground as usize => dim(named(FOREGROUND)),
            _ => named(FOREGROUND),
        },
    }
}

fn dim(color: Rgb) -> Rgb {
    let dim = |value: u8| u8::try_from(u16::from(value) * 2 / 3).unwrap_or(value);
    Rgb {
        r: dim(color.r),
        g: dim(color.g),
        b: dim(color.b),
    }
}

/// The color `color` names, as the terminal's colors are now.
fn resolve(color: Color, colors: &Colors) -> Rgb {
    match color {
        Color::Spec(color) => color,
        Color::Named(name) => color_at(name as usize, colors),
        Color::Indexed(index) => color_at(usize::from(index), colors),
    }
}

fn hsla(color: Rgb) -> Hsla {
    rgb(u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)).into()
}

/// What the input method tells the tab, or GPUI itself for plain typing.
/// The program keeps the text, so the tab only remembers what is being
/// composed.
impl EntityInputHandler for Terminal {
    fn text_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .composing
            .as_ref()
            .map_or(0, |text| text.encode_utf16().count());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.composing
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.composing = None;
        cx.notify();
    }

    /// Final text: typed characters, or a composition the input method
    /// committed.
    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = None;
        if !text.is_empty() {
            self.type_in(text.as_bytes().to_vec(), cx);
        }
        cx.notify();
    }

    /// Text the input method is still composing.
    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    /// Where the input method puts its candidate window: at the cursor.
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let laid = self.laid?;
        let term = self.term.lock();
        let cursor = term.grid().cursor.point;
        let line = cursor.line.0 + i32::try_from(term.grid().display_offset()).unwrap_or(0);
        #[expect(
            clippy::cast_precision_loss,
            reason = "cell counts are far below what f32 keeps exactly"
        )]
        let origin = point(
            laid.cell.width * cursor.column.0 as f32,
            laid.cell.height * line as f32,
        );
        Some(Bounds {
            origin: laid.origin + origin,
            size: laid.cell,
        })
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for Terminal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_bar())
            .child(
                div()
                    .id("terminal")
                    .debug_selector(|| "terminal".to_owned())
                    .flex_1()
                    .min_h_0()
                    .bg(rgb(BACKGROUND))
                    .track_focus(&self.focus_handle)
                    .key_context(KEY_CONTEXT)
                    .on_key_down(cx.listener(Self::key_down))
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(Self::paste))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_scroll_wheel(cx.listener(Self::scroll))
                    .child(Grid {
                        terminal: cx.entity(),
                    }),
            )
    }
}

/// Draws the terminal's grid, and takes the human's typing for it.
struct Grid {
    terminal: Entity<Terminal>,
}

/// What a frame of the grid paints.
#[derive(Default)]
struct Painted {
    /// Backgrounds other than the terminal's own, and the selection.
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    /// Text, at where it starts.
    text: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, CursorShape)>,
    /// What the input method composes, at the cursor.
    composing: Option<(Point<Pixels>, ShapedLine)>,
    line_height: Pixels,
}

/// A run of cells drawn alike, as it builds up.
struct Run {
    line: usize,
    column: usize,
    text: String,
    look: Look,
    /// Whether the next cell may join it: not after a wide character, so
    /// that the text after one starts where its cell does, whatever the
    /// font makes of it, nor after the cursor.
    open: bool,
}

impl Run {
    /// Puts the run after `runs`: as part of the last, if it goes on from
    /// it alike.
    fn add_to(self, runs: &mut Vec<Self>) {
        match runs.last_mut() {
            Some(last)
                if last.open
                    && self.open
                    && last.line == self.line
                    && last.look == self.look
                    && last.column + last.text.chars().count() == self.column =>
            {
                last.text.push_str(&self.text);
            }
            _ => runs.push(self),
        }
    }
}

/// How a cell's text looks.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    color: Hsla,
    weight: FontWeight,
    style: FontStyle,
    underline: bool,
    strikethrough: bool,
}

/// The cell size and the font of the grid.
struct Metrics {
    font: gpui::Font,
    font_size: Pixels,
    cell: gpui::Size<Pixels>,
}

impl Metrics {
    fn of(window: &Window) -> Self {
        let font = font(FONT);
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system();
        let width = text_system
            .advance(text_system.resolve_font(&font), font_size, 'm')
            .map_or(px(8.), |advance| advance.width);
        Self {
            font,
            font_size,
            cell: size(width, (font_size * LINE_HEIGHT).round()),
        }
    }

    /// How many cells fit in `bounds`.
    fn fitting(&self, bounds: Bounds<Pixels>) -> Size {
        let area = bounds.size - size(px(PADDING * 2.), px(PADDING * 2.));
        Size {
            columns: whole_cells(area.width / self.cell.width).max(2),
            lines: whole_cells(area.height / self.cell.height).max(1),
        }
    }

    /// What the input method composes, shaped to paint over the grid.
    fn shape_composing(&self, text: &str, window: &Window) -> ShapedLine {
        let look = Look {
            color: rgb(FOREGROUND).into(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            underline: true,
            strikethrough: false,
        };
        self.shape(text, look, Some(rgb(BACKGROUND).into()), window)
    }

    /// `text` as it looks with `look`, shaped to paint.
    fn shape(
        &self,
        text: &str,
        look: Look,
        background: Option<Hsla>,
        window: &Window,
    ) -> ShapedLine {
        let font = gpui::Font {
            weight: look.weight,
            style: look.style,
            ..self.font.clone()
        };
        window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            self.font_size,
            &[TextRun {
                len: text.len(),
                font,
                color: look.color,
                background_color: background,
                underline: look.underline.then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(look.color),
                    wavy: false,
                }),
                strikethrough: look.strikethrough.then_some(gpui::StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(look.color),
                }),
            }],
            None,
        )
    }
}

/// The line of the screen that `line` of the grid is on, scrolled back by
/// `offset`, if it is on the screen.
fn on_screen(line: Line, offset: usize) -> Option<usize> {
    usize::try_from(i64::from(line.0) + i64::try_from(offset).ok()?).ok()
}

/// How a cell looks: its background, if not the terminal's, and its text.
fn look_of(cell: &alacritty_terminal::term::cell::Cell, colors: &Colors) -> (Option<Hsla>, Look) {
    let flags = cell.flags;
    let mut foreground = resolve(cell.fg, colors);
    let mut background = resolve(cell.bg, colors);
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut foreground, &mut background);
    }
    if flags.contains(Flags::DIM) {
        foreground = dim(foreground);
    }
    if flags.contains(Flags::HIDDEN) {
        foreground = background;
    }
    let background = hsla(background);
    let look = Look {
        color: hsla(foreground),
        weight: if flags.contains(Flags::BOLD) {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if flags.contains(Flags::ITALIC) {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        underline: flags.intersects(Flags::ALL_UNDERLINES),
        strikethrough: flags.contains(Flags::STRIKEOUT),
    };
    let background = (background != Hsla::from(rgb(BACKGROUND))).then_some(background);
    (background, look)
}

impl Grid {
    /// Fits the terminal to `bounds`, and works out what to paint there.
    #[expect(
        clippy::cast_precision_loss,
        reason = "cell counts are far below what f32 keeps exactly"
    )]
    fn lay_out(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Painted {
        let metrics = Metrics::of(window);
        let cell = metrics.cell;
        let fits = metrics.fitting(bounds);
        let origin = bounds.origin + point(px(PADDING), px(PADDING));
        let focused = self.terminal.read(cx).focus_handle.is_focused(window);
        // Nothing types once the program has ended.
        let ended = self.terminal.read(cx).ended.is_some();
        let (term, composing) = self.terminal.update(cx, |terminal, _| {
            terminal.resize(fits, cell);
            terminal.laid = Some(Laid { origin, cell });
            (Arc::clone(&terminal.term), terminal.composing.clone())
        });
        let term = term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset;
        let at = |line: usize, column: usize| {
            origin + point(cell.width * column as f32, cell.height * line as f32)
        };
        let cursor = content.cursor;
        let cursor_line =
            on_screen(cursor.point.line, offset).filter(|&line| line < fits.lines && !ended);
        let block_cursor = focused && cursor.shape == CursorShape::Block;

        let mut painted = Painted::default();
        let mut runs: Vec<Run> = Vec::new();
        for indexed in content.display_iter {
            if indexed
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let Some(line) = on_screen(indexed.point.line, offset) else {
                continue;
            };
            let column = indexed.point.column.0;
            let wide = indexed.flags.contains(Flags::WIDE_CHAR);
            let (mut background, mut look) = look_of(indexed.cell, content.colors);
            if content
                .selection
                .is_some_and(|selection| selection.contains(indexed.point))
            {
                background = Some(rgb(SELECTION).into());
            }
            let under_cursor = cursor_line == Some(line) && cursor.point.column.0 == column;
            if under_cursor && block_cursor {
                background = Some(rgb(CURSOR).into());
                look.color = rgb(BACKGROUND).into();
            }
            if let Some(background) = background {
                let width = if wide { cell.width * 2. } else { cell.width };
                painted.backgrounds.push((
                    Bounds::new(at(line, column), size(width, cell.height)),
                    background,
                ));
            }
            let mut text = String::from(indexed.c);
            if let Some(zerowidth) = indexed.zerowidth() {
                text.extend(zerowidth);
            }
            Run {
                line,
                column,
                text,
                look,
                open: !wide && !under_cursor,
            }
            .add_to(&mut runs);
        }
        for run in runs {
            if run.text.trim().is_empty() && !run.look.underline {
                continue;
            }
            let shaped = metrics.shape(&run.text, run.look, None, window);
            painted.text.push((at(run.line, run.column), shaped));
        }
        if let Some(line) = cursor_line
            && cursor.shape != CursorShape::Hidden
        {
            let wide = term.grid()[cursor.point].flags.contains(Flags::WIDE_CHAR);
            let cells = if wide { 2. } else { 1. };
            let at = at(line, cursor.point.column.0);
            let shape = if focused {
                cursor.shape
            } else {
                CursorShape::HollowBlock
            };
            painted.cursor = Some((
                Bounds::new(at, size(cell.width * cells, cell.height)),
                shape,
            ));
            painted.composing =
                composing.map(|composing| (at, metrics.shape_composing(&composing, window)));
        }
        painted.line_height = cell.height;
        painted
    }
}

impl IntoElement for Grid {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Grid {
    type RequestLayoutState = ();
    type PrepaintState = Painted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.lay_out(bounds, window, cx)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        painted: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.terminal.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.terminal.clone()),
            cx,
        );
        let line_height = painted.line_height;
        for (at, color) in &painted.backgrounds {
            window.paint_quad(fill(*at, *color));
        }
        for (origin, line) in &painted.text {
            line.paint(*origin, line_height, window, cx).ok();
        }
        if let Some((at, shape)) = painted.cursor {
            let color = rgb(CURSOR);
            match shape {
                // Painted under the text, as its cell's background.
                CursorShape::Block | CursorShape::Hidden => {}
                CursorShape::HollowBlock => {
                    window.paint_quad(outline(at, color, gpui::BorderStyle::Solid));
                }
                CursorShape::Beam => {
                    window.paint_quad(fill(
                        Bounds::new(at.origin, size(px(2.), at.size.height)),
                        color,
                    ));
                }
                CursorShape::Underline => {
                    let origin = at.origin + point(px(0.), at.size.height - px(2.));
                    window.paint_quad(fill(
                        Bounds::new(origin, size(at.size.width, px(2.))),
                        color,
                    ));
                }
            }
        }
        if let Some((origin, line)) = &painted.composing {
            line.paint_background(*origin, line_height, window, cx).ok();
            line.paint(*origin, line_height, window, cx).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(keystroke: &str, mode: TermMode) -> Option<Vec<u8>> {
        let keystroke = Keystroke::parse(keystroke).expect("a keystroke GPUI reads");
        key_bytes(&keystroke, mode).map(Cow::into_owned)
    }

    #[test]
    fn keys_that_type_text_reach_the_tab_as_text() {
        assert_eq!(keys("a", TermMode::NONE), None);
        assert_eq!(keys("shift-a", TermMode::NONE), None);
        assert_eq!(keys("cmd-c", TermMode::NONE), None);
    }

    #[test]
    fn control_with_a_letter_is_its_control_character() {
        assert_eq!(keys("ctrl-c", TermMode::NONE), Some(b"\x03".to_vec()));
        assert_eq!(keys("ctrl-d", TermMode::NONE), Some(b"\x04".to_vec()));
        assert_eq!(keys("ctrl-[", TermMode::NONE), Some(b"\x1b".to_vec()));
        assert_eq!(keys("ctrl-space", TermMode::NONE), Some(b"\0".to_vec()));
    }

    #[test]
    fn keys_that_type_nothing_send_their_sequences() {
        assert_eq!(keys("enter", TermMode::NONE), Some(b"\r".to_vec()));
        assert_eq!(keys("backspace", TermMode::NONE), Some(b"\x7f".to_vec()));
        assert_eq!(keys("tab", TermMode::NONE), Some(b"\t".to_vec()));
        assert_eq!(keys("shift-tab", TermMode::NONE), Some(b"\x1b[Z".to_vec()));
        assert_eq!(keys("escape", TermMode::NONE), Some(b"\x1b".to_vec()));
        assert_eq!(keys("delete", TermMode::NONE), Some(b"\x1b[3~".to_vec()));
        assert_eq!(keys("f5", TermMode::NONE), Some(b"\x1b[15~".to_vec()));
    }

    #[test]
    fn the_cursor_keys_follow_the_programs_mode_and_the_modifiers() {
        assert_eq!(keys("up", TermMode::NONE), Some(b"\x1b[A".to_vec()));
        assert_eq!(keys("up", TermMode::APP_CURSOR), Some(b"\x1bOA".to_vec()));
        assert_eq!(
            keys("ctrl-left", TermMode::NONE),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            keys("shift-end", TermMode::APP_CURSOR),
            Some(b"\x1b[1;2F".to_vec())
        );
    }

    #[test]
    fn a_paste_is_marked_for_a_program_that_asks() {
        assert_eq!(pasted("a\nb", false), b"a\rb");
        assert_eq!(pasted("a\r\nb", false), b"a\rb");
        assert_eq!(pasted("a\nb\x1b", true), b"\x1b[200~a\nb\x1b[201~");
    }

    #[test]
    fn colors_come_from_the_palette_the_cube_and_the_grays() {
        let colors = Colors::default();
        assert_eq!(
            color_at(1, &colors),
            Rgb {
                r: 0xe0,
                g: 0x6c,
                b: 0x75
            }
        );
        assert_eq!(
            color_at(16 + 36 * 5 + 6 * 2 + 1, &colors),
            Rgb {
                r: 255,
                g: 135,
                b: 95
            }
        );
        assert_eq!(
            color_at(255, &colors),
            Rgb {
                r: 238,
                g: 238,
                b: 238
            }
        );
        assert_eq!(
            color_at(NamedColor::Background as usize, &colors),
            Rgb {
                r: 0x1e,
                g: 0x1e,
                b: 0x1e
            }
        );
    }
}
