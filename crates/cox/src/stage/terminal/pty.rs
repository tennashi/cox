//! A program running on a pseudoterminal, as `alacritty_terminal` runs
//! it: the pseudoterminal is opened, the program started in it, working
//! where the chat does, and a thread of its own reads what the program
//! writes into the terminal's grid. The tab shows it from there.

use std::collections::HashMap;
use std::os::fd::{FromRawFd as _, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;
use gpui::Pixels;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::start_cell;

/// How many lines the terminal keeps of what scrolled off.
const SCROLLBACK: usize = 10_000;

/// The size a terminal starts at, before the tab is laid out.
pub(super) const START_SIZE: Size = Size {
    columns: 80,
    lines: 24,
};

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
pub(super) struct Size {
    pub(super) columns: usize,
    pub(super) lines: usize,
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
    pub(super) fn window_size(self, cell: gpui::Size<Pixels>) -> WindowSize {
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
pub(super) struct Listener(UnboundedSender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

/// A program running on a pseudoterminal, before a tab shows it.
pub struct Running {
    pub(super) program: Program,
    pub(super) cwd: PathBuf,
    pub(super) term: Arc<FairMutex<Term<Listener>>>,
    pub(super) pty: EventLoopSender,
    pub(super) events: UnboundedReceiver<Event>,
    /// The program's end of the pseudoterminal, held open.
    pub(super) _held: OwnedFd,
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
