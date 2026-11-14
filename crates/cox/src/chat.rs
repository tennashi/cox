//! One chat: a human and an agent working in a folder, on the stage of the
//! folder, which they share with the other chats there. The daemon runs it
//! (see [`crate::daemon`]); this is what starts it, what it is, and how
//! the window shows it: the window makes each folder's stage through
//! [`Stages`].
//!
//! - [`launcher`] starts a chat's stage and the relay to it, for the
//!   window; the daemon starts its agent.
//! - [`view`] draws a chat's conversation with GPUI, its stage beside it,
//!   and turns keystrokes and clicks into what the human does.
//! - [`hand`] is what the human hands a chat from elsewhere on the
//!   screen, as text selected in a tab, and [`region`] a region of the
//!   window they drag, as a picture.
//! - [`conversation`] holds what the view shows and decides what the
//!   human's input means, in plain Rust, so it is tested without GPUI.
//!   `cox-core` keeps what it shows.

pub mod conversation;
pub mod hand;
pub mod judge;
pub mod launcher;
pub mod region;
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
    /// The stage of the chats working in `cwd`, an absolute path, on the
    /// tabs it had, if it had any, each of a chat by its key, with what
    /// opens the links of their conversations there, or `None` for chats
    /// with no stage. A [`Stage`] works with the chats' agents; any other
    /// view only shows.
    fn stage(
        &self,
        cwd: PathBuf,
        tabs: Option<KeptTabs<u64>>,
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
