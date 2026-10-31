//! Telling the human of chats that need them while they look elsewhere:
//! with the system's notifications, which a click on brings the chat up,
//! and with a count on the app's icon. Only macOS has them so far
//! ([`macos`]); elsewhere, Cox tells of nothing.

use std::rc::Rc;

use gpui::Global;

/// Where Cox tells the human of its chats.
pub trait Notices {
    /// Tells the human `body` of the chat `chat`, a number that names it
    /// for this run, named `title`, in place of what it told of the chat
    /// before.
    fn notify(&self, chat: u64, title: &str, body: &str);

    /// Shows how many chats need the human, or nothing for none.
    fn badge(&self, count: usize);
}

/// The app's [`Notices`], if it has any.
pub struct AppNotices(pub Rc<dyn Notices>);

impl Global for AppNotices {}

#[cfg(target_os = "macos")]
pub mod macos;
