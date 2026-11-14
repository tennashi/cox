//! A program running on a pseudoterminal, as `alacritty_terminal` runs
//! it: the pseudoterminal is opened, the program started in it, working
//! where the chat does, and a thread of its own reads what the program
//! writes into the terminal's grid. A tab shows it from there, or no
//! one does (see [`super::run_unseen`]).

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
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::START_CELL;
use coxd::process::close_on_exec;

/// How many lines the terminal keeps of what scrolled off.
const SCROLLBACK: usize = 10_000;

/// The size a terminal starts at, before the tab is laid out.
pub(crate) const START_SIZE: Size = Size {
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
pub(crate) struct Size {
    pub(crate) columns: usize,
    pub(crate) lines: usize,
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
    /// As the pseudoterminal is told it, with cells `width` by `height`
    /// pixels.
    pub(crate) fn window_size(self, width: f32, height: f32) -> WindowSize {
        let whole = |count: usize| u16::try_from(count).unwrap_or(u16::MAX);
        WindowSize {
            num_lines: whole(self.lines),
            num_cols: whole(self.columns),
            cell_width: whole_pixels(width),
            cell_height: whole_pixels(height),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a cell is a few pixels; rounding is the point"
)]
fn whole_pixels(pixels: f32) -> u16 {
    pixels.round().max(0.) as u16
}

/// Hands what the terminal tells of itself to the tab.
#[derive(Clone)]
pub(crate) struct Listener(UnboundedSender<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event);
    }
}

/// A program running on a pseudoterminal, before a tab shows it.
pub struct Running {
    pub(crate) program: Program,
    pub(crate) cwd: PathBuf,
    pub(crate) term: Arc<FairMutex<Term<Listener>>>,
    pub(crate) pty: EventLoopSender,
    pub(crate) events: UnboundedReceiver<Event>,
    /// The program's end of the pseudoterminal, held open.
    pub(crate) _held: OwnedFd,
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
        let (controller, user) = open_pty(START_SIZE.window_size(START_CELL.0, START_CELL.1))?;
        // Cox holds the program's end open as well, so that its own end
        // never reads as hung up: on Linux, the terminal then drops what
        // the program wrote last, when it reads that after the hang-up.
        let held = user.try_clone()?;
        close_on_exec();
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
    environment_past(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// What [`environment`] adds to an environment in which `set` tells
/// which variables are set.
fn environment_past(set: impl Fn(&str) -> bool) -> HashMap<String, String> {
    let mut environment = HashMap::from([
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
    ]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_told_it_runs_in_a_terminal_of_true_colors() {
        let environment = environment_past(|_| true);

        assert_eq!(environment["TERM"], "xterm-256color");
        assert_eq!(environment["COLORTERM"], "truecolor");
    }

    #[test]
    fn a_program_is_told_characters_are_utf_8_when_nothing_says_what_they_are() {
        let environment = environment_past(|_| false);

        assert!(
            environment["LC_CTYPE"].ends_with("UTF-8"),
            "{:?}",
            environment.get("LC_CTYPE")
        );
    }

    #[test]
    fn a_program_keeps_the_locale_cox_was_given() {
        for given in ["LC_ALL", "LC_CTYPE", "LANG"] {
            let environment = environment_past(|name| name == given);

            assert!(!environment.contains_key("LC_CTYPE"), "with {given} set");
        }
    }
}
