//! What Cox keeps of the human's work, without GPUI or CEF: the projects,
//! the chats in them, and what each chat showed, for the human to read
//! again once it is closed.
//!
//! - [`entry`] is what a chat's conversation shows, entry by entry, and
//!   [`diff`] the lines a change to a file shows in it.
//! - [`tabs`] is what is kept of a chat's stage.
//! - [`store`] keeps all of it in `SQLite`.

pub mod diff;
pub mod entry;
pub mod store;
pub mod tabs;
