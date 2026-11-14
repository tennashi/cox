//! coxd, `cox --daemon`: what runs the chats and their agents apart from
//! the window, which only shows them and tells coxd what the human does.
//! It holds no GPUI and no CEF; the window, the `cox` crate, depends on
//! it, never the other way.
//!
//! The modules go from the top down, each using only what is below it:
//!
//! - [`theme`] is how Cox looks, which the window and the page coxd
//!   serves a phone share, and [`paths`] how paths are written.
//! - [`process`] is what Cox does as it starts a program, the agent's or
//!   a terminal's.
//! - [`storage`] says where this Cox keeps what lasts across its runs,
//!   [`scratch`] what it keeps only while it runs, and [`trash`] where
//!   what the human takes away goes.
//! - [`login`] takes in the login shell's environment, for a Cox the
//!   system opened, as from the Finder.
//! - [`home`] is the human's home directory.

pub mod home;
pub mod login;
pub mod paths;
pub mod process;
pub mod scratch;
pub mod storage;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod theme;
pub mod trash;
