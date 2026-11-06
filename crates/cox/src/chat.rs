//! One chat: a human and an agent working in a folder, with a stage of
//! their own. The daemon runs it (see [`crate::daemon`]); this is what
//! starts it, what it is, and how the window shows it: the window makes
//! each chat's stage through [`Stages`].
//!
//! - [`launcher`] starts a chat: its agent, with the tools between it and
//!   its stage, for the daemon; and its stage and the relay to it, for
//!   the window.
//! - [`view`] draws a chat's conversation with GPUI, its stage beside it,
//!   and turns keystrokes and clicks into what the human does.
//! - [`conversation`] holds what the view shows and decides what the
//!   human's input means, in plain Rust, so it is tested without GPUI.
//!   `cox-core` keeps what it shows.

pub mod conversation;
pub mod judge;
pub mod launcher;
pub mod mention;
pub mod sketch;
pub mod view;

use std::path::PathBuf;

use cox_core::tabs::KeptTabs;
use gpui::{App, Window};

use crate::agent::mcp::ToolRequest;
use crate::chat::view::{OpenLink, StageView};
#[cfg(doc)]
use crate::stage::Stage;
use cox_browser::relay::Relay;

/// What makes the chats' stages for the window: the
/// [`CefStages`](launcher::CefStages) of the browser, or none in tests.
pub trait Stages {
    /// The stage of a chat working in `cwd`, an absolute path, on the
    /// tabs it had, if it had any, with what opens the links of its
    /// conversation there, or `None` for a chat with no stage. A
    /// [`Stage`] works with the chat's agent; any other view only shows.
    fn stage(
        &self,
        cwd: PathBuf,
        tabs: Option<KeptTabs>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<(StageView, OpenLink)>;

    /// Starts a relay for the browser tools of an agent that drives a
    /// stage, or answers `None` when there is none.
    fn relay(&self) -> Option<Relay>;

    /// Carries out `request`, which the agent of a chat with no stage
    /// asks of it, as a test's terminal does; let go, by default, which
    /// answers that it cannot.
    fn unstaged(&self, request: ToolRequest) {
        drop(request);
    }
}
