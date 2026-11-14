//! A program running on a pseudoterminal, as `alacritty_terminal` runs
//! it, and what its terminal shows, apart from any window: a tab of the
//! stage shows one (see [`crate::stage::terminal`]), and coxd runs a
//! command the agent asks for with none (see [`run_unseen`]).
//!
//! - [`pty`] starts the program on the pseudoterminal, and reads what it
//!   writes.
//! - [`colors`] holds the terminal's colors, and what a program's names.
//! - [`unseen`] runs a command the agent asks for while no window is
//!   there, with no tab to show it.

pub mod colors;
pub mod pty;
mod unseen;

use std::os::unix::process::ExitStatusExt as _;
use std::process::ExitStatus;

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Point as Cell};
use alacritty_terminal::term::Term;

use self::pty::Listener;
pub use self::pty::{Program, Running};
pub use self::unseen::run_unseen;
use coxd::agent::mcp::Ended;

/// The size of the terminal's font, which a tab draws its cells in.
pub const FONT_SIZE: f32 = 13.;
/// How high a line of cells is, for each point of [`FONT_SIZE`].
pub const LINE_HEIGHT: f32 = 1.3;

/// How wide and high a cell is, in pixels, before a tab lays the
/// terminal out.
pub const START_CELL: (f32, f32) = (8., (FONT_SIZE * LINE_HEIGHT).round());

/// What `term` shows, with what scrolled off, as text: a line the
/// terminal wrapped is one line.
pub(crate) fn text_of(term: &Term<Listener>) -> String {
    let start = Cell::new(term.topmost_line(), Column(0));
    let end = Cell::new(term.bottommost_line(), term.last_column());
    term.bounds_to_string(start, end).trim_end().to_owned()
}

/// How a program ended, as it exited with `status`, if it was learned.
pub(crate) fn ended_of(status: Option<ExitStatus>) -> Ended {
    match status {
        Some(status) => status.code().map_or_else(
            || status.signal().map_or(Ended::Unknown, Ended::Signal),
            Ended::Exited,
        ),
        None => Ended::Unknown,
    }
}
