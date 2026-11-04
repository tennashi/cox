//! One chat: a human and an agent working in a folder, with a stage of
//! their own. What the window shows of it, and what starts it: the window
//! opens chats through [`Opens`], and reads a [`Closed`] one again.
//!
//! - [`launcher`] starts a chat: its stage, the relay and the tools
//!   between them, and its agent; and starts a closed chat again.
//! - [`view`] draws a chat's conversation with GPUI, its stage beside it,
//!   and turns keystrokes and clicks into input.
//! - [`conversation`] holds what the view shows and decides what the
//!   human's input means, in plain Rust, so it is tested without GPUI.
//!   `cox-core` keeps what it shows.

pub mod conversation;
pub mod judge;
pub mod launcher;
pub mod trial;
pub mod view;

use std::path::PathBuf;

use cox_core::store::Record;
use cox_core::tabs::KeptTabs;
use gpui::{App, Entity, Window};

use crate::agent::instructions::Instructions;
use crate::agent::setup::Setup;
use crate::chat::conversation::Entry;
use crate::chat::view::ChatView;

/// What opens chats for the window: the [`Launcher`](launcher::Launcher)
/// of the agents, or a fake one in tests.
pub trait Opens {
    /// Starts a chat's agent working in `cwd`, an absolute path, with
    /// `instructions`, set up as `setup` says if given, keeping the chat
    /// in `record` if given, and returns the chat's view.
    fn open(
        &self,
        cwd: PathBuf,
        instructions: Option<Instructions>,
        setup: Option<Setup>,
        record: Option<Record>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<ChatView>;

    /// Returns the view of `closed`, a chat that has closed, from what is
    /// kept of it, for the human to read, and to send to again.
    fn read(&self, closed: Closed, window: &mut Window, cx: &mut App) -> Entity<ChatView>;
}

/// What is kept of a closed chat.
pub struct Closed {
    /// The directory it worked in.
    pub cwd: PathBuf,
    /// What it showed.
    pub entries: Vec<Entry>,
    /// The tabs its stage had, if it had a stage.
    pub tabs: Option<KeptTabs>,
    /// The id its agent gave its session, to go on with, if it had one.
    pub session: Option<String>,
    /// What it works until, if the human set a goal.
    pub goal: Option<cox_core::goal::Goal>,
    /// The settings of its session, as they were set.
    pub settings: Vec<cox_core::settings::Setting>,
    /// How its agent is set up, if the chat that opened it said.
    pub setup: Option<Setup>,
    /// Where it is kept, to go on keeping it once the human sends to it.
    pub record: Option<Record>,
}
