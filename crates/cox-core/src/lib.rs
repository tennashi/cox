//! What Cox keeps of the human's work, without GPUI or CEF: the projects,
//! the chats in them, and what each chat showed, for the human to read
//! again once it is closed.
//!
//! - [`entry`] is what a chat's conversation shows, entry by entry, and
//!   [`diff`] the lines a change to a file shows in it.
//! - [`goal`] is what a chat works until, when the human set a goal.
//! - [`settings`] is what a chat's agent lets the human choose of its
//!   session, such as its model.
//! - [`tabs`] is what is kept of a chat's stage.
//! - [`list`] is a list of which one item is shown, as a stage's tabs
//!   are, and a window's chats.
//! - [`store`] keeps all of it in `SQLite`.
//! - [`moved`] makes what is kept of a folder name where it moved, and
//!   [`folder`] says when a folder is taken.
//! - [`address`] decodes an address's escapes, and [`quit`] is how a
//!   signal, or anything else that cannot quit right away, asks Cox to.

pub mod address;
pub mod diff;
pub mod entry;
pub mod folder;
pub mod goal;
pub mod list;
pub mod moved;
pub mod quit;
pub mod settings;
pub mod store;
pub mod tabs;
