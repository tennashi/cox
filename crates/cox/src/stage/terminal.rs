//! A tab of the stage that is a terminal: a shell the human types into,
//! or a command the agent runs with Cox's tool for that (see
//! [`crate::agent::mcp`]), which the human watches run, and can type into too.
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
//!
//! - [`pty`] starts the program on the pseudoterminal, and reads what it
//!   writes.
//! - [`keys`] says what the program gets for the human's keys and pastes.
//! - [`colors`] holds the terminal's colors, and what a program's names.
//! - [`grid`] draws the grid.
//! - [`unseen`] runs a command the agent asks for while no window is
//!   there, with no tab to show it.

mod colors;
mod grid;
mod keys;
mod pty;
mod unseen;

use std::borrow::Cow;
use std::ops::Range;
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt as _;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;
use std::time::Duration;

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::event_loop::{EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as Cell, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Term, TermMode};
use cox_core::entry::Attachment;
use cox_core::tabs::KeptTab;
use gpui::{
    App, Bounds, ClipboardItem, Context, EntityInputHandler, FocusHandle, Focusable, IntoElement,
    KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, ScrollDelta, ScrollWheelEvent, Task, UTF16Selection, Window, div, point, prelude::*, px,
    rgb, size,
};
use tokio::sync::oneshot;

use self::colors::color_at;
use self::grid::{FONT_SIZE, Grid, LINE_HEIGHT, Laid, whole_cells};
use self::keys::{key_bytes, pasted};
use self::pty::{Listener, START_SIZE, Size};
pub use self::pty::{Program, Running};
pub use self::unseen::run_unseen;
use crate::agent::mcp::{Ended, Ran};
use crate::chat::hand::AddToChat;
use crate::chat::region::{self, Described};
use crate::stage::tab::Shows;
use crate::ui::Copy;
use crate::ui::ellipsis::ellipsis;
use crate::ui::input::Paste;
use crate::ui::paths::shortened;
use crate::ui::theme::{ACCENT, BORDER, DIM, FAILED};
use crate::ui::translation::Translate;

const KEY_CONTEXT: &str = "Terminal";

/// How often a command the agent runs looks whether the agent still waits
/// for it.
const WATCH_EVERY: Duration = Duration::from_millis(250);

/// What `term` shows, with what scrolled off, as text: a line the
/// terminal wrapped is one line.
fn text_of(term: &Term<Listener>) -> String {
    let start = Cell::new(term.topmost_line(), Column(0));
    let end = Cell::new(term.bottommost_line(), term.last_column());
    term.bounds_to_string(start, end).trim_end().to_owned()
}

/// How a program ended, as it exited with `status`, if it was learned.
fn ended_of(status: Option<ExitStatus>) -> Ended {
    match status {
        Some(status) => status.code().map_or_else(
            || status.signal().map_or(Ended::Unknown, Ended::Signal),
            Ended::Exited,
        ),
        None => Ended::Unknown,
    }
}

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
        KeyBinding::new("ctrl-t", gpui::NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-p", gpui::NoAction, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-\\", gpui::NoAction, Some(KEY_CONTEXT)),
    ]);
}

/// The size of a cell, before the tab is laid out.
pub(super) fn start_cell() -> gpui::Size<Pixels> {
    size(px(8.), (px(FONT_SIZE) * LINE_HEIGHT).round())
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

    /// What the terminal shows, with what scrolled off, as text: a line
    /// the terminal wrapped is one line.
    #[must_use]
    pub fn text(&self) -> String {
        text_of(&self.term.lock())
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
        let ended = ended_of(self.status);
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

    /// Shows the translation of the text selected, or with nothing
    /// selected leaves the keys to what holds the part.
    fn translate(&mut self, _: &Translate, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text().unwrap_or_default();
        if text.trim().is_empty() {
            cx.propagate();
            return;
        }
        crate::ui::translation::show(&text, window, cx);
    }

    /// The text selected, with the terminal it is from, by what it runs,
    /// as it goes to the agent when handed to the chat.
    fn selected_attachment(&self) -> Attachment {
        Attachment {
            label: self.title(),
            uri: "cox://terminal".to_owned(),
            text: self.selected_text().unwrap_or_default(),
            picture: None,
        }
    }

    /// What the rows of the terminal under `over`, a stretch of a region
    /// in the window's points, say, as the last frame drew them.
    fn described(&self, over: Bounds<Pixels>) -> Option<Described> {
        let laid = self.laid?;
        let term = self.term.lock();
        // A cell is in the region when its middle is, and at least one is.
        let cells = |from: Pixels, to: Pixels, cell: Pixels, most: usize| {
            let first = whole_cells((from / cell).round()).min(most - 1);
            let last = whole_cells((to / cell).round() - 1.).clamp(first, most - 1);
            (first, last)
        };
        let (left, right) = cells(
            over.origin.x - laid.origin.x,
            over.origin.x + over.size.width - laid.origin.x,
            laid.cell.width,
            term.columns(),
        );
        let (top, bottom) = cells(
            over.origin.y - laid.origin.y,
            over.origin.y + over.size.height - laid.origin.y,
            laid.cell.height,
            term.screen_lines(),
        );
        let offset = i32::try_from(term.grid().display_offset()).unwrap_or(0);
        let rows: Vec<String> = (top..=bottom)
            .map(|row| {
                let line = Line(i32::try_from(row).unwrap_or(0) - offset);
                term.bounds_to_string(
                    Cell::new(line, Column(left)),
                    Cell::new(line, Column(right)),
                )
                .trim_end()
                .to_owned()
            })
            .collect();
        drop(term);
        let title = self.title();
        Some(Described {
            text: format!(
                "rows {}-{} and columns {}-{} of the terminal `{title}`, as it shows them:\n\n{}",
                top + 1,
                bottom + 1,
                left + 1,
                right + 1,
                region::fenced(&rows.join("\n"))
            ),
            name: title,
        })
    }

    /// Hands the text selected to the chat shown, or with nothing
    /// selected leaves the keys to what holds the part.
    fn add_to_chat(&mut self, _: &AddToChat, _: &mut Window, cx: &mut Context<Self>) {
        if !crate::chat::hand::hand(self.selected_attachment(), cx) {
            cx.propagate();
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
                    Some(Ended::Exited(0)) => (Ended::Exited(0).to_string(), ACCENT),
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
            .border_color(rgb(BORDER))
            .text_color(rgb(DIM))
            .child(
                div()
                    .debug_selector(|| "terminal-program".to_owned())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(ellipsis(what)),
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

impl Shows for Terminal {
    /// The command, or what the shell calls its terminal.
    fn title(&self) -> String {
        match &self.program {
            Program::Command(command) => command.lines().next().unwrap_or_default().to_owned(),
            Program::Shell => self
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| "Terminal".to_owned()),
        }
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Terminal {
            title: self.title(),
        }
    }
}

impl Focusable for Terminal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
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
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus_handle)
                    .key_context(KEY_CONTEXT)
                    .on_key_down(cx.listener(Self::key_down))
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(Self::translate))
                    .on_action(cx.listener(Self::add_to_chat))
                    .on_action(cx.listener(Self::paste))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|terminal, _, window, cx| {
                            let text = terminal.selected_text().unwrap_or_default();
                            let handed = Some(terminal.selected_attachment());
                            if crate::ui::translation::offer(&text, handed, window, cx) {
                                cx.stop_propagation();
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_scroll_wheel(cx.listener(Self::scroll))
                    .cursor_text()
                    .child(Grid {
                        terminal: cx.entity(),
                    })
                    .child(region::mark({
                        let terminal = cx.weak_entity();
                        move |_, over, cx| terminal.upgrade()?.read(cx).described(over)
                    })),
            )
    }
}
