//! coxd, `cox --daemon`: what runs the chats and their agents apart from
//! the window, which only shows them and tells coxd what the human does.
//! It holds no GPUI and no CEF; the window, the `cox` crate, depends on
//! it, never the other way.
//!
//! The modules go from the top down, each using only what is below it:
//!
//! - [`daemon`] is coxd itself: it runs the chats, answers the windows
//!   and the phones, and starts each chat's agent; its `main` is what
//!   `cox --daemon` runs, and its `client` the window's side of the wire.
//! - [`trigger`] is what runs, unasked, when something happens in a
//!   folder.
//! - [`chat`] is one chat, apart from how the window draws it.
//! - [`agent`] is a chat's agent: the ACP connection, and the tools Cox
//!   hands the agent.
//! - [`workflow`] is what Cox tells a chat's agent in the human's place
//!   at the turns of a sketch, which the human's workflow can say instead.
//! - [`schedule`] is the agents the human has run on a schedule, a chat
//!   that coxd opens at the times they say.
//! - [`terminal`] is a program on a pseudoterminal, as a tab of the
//!   window's stage shows it, or as coxd runs it with none.
//! - [`translate`] is text read in the human's language.
//! - [`project_icon`] is the icons the human gives the projects.
//! - [`agents`] is the agents a chat can talk to: the command line's,
//!   the human's, and the ACP Registry's.
//! - [`settings`] is what the human sets of Cox itself.
//! - [`wiki`] is the human's wiki, where the agents keep what they
//!   learned, for those of every project to find again.
//! - [`raw`] is what the agents read, kept as they read it, as the
//!   sources of research, apart from what is made of it.
//! - [`repositories`] is the repositories of a folder Cox works in.
//! - [`media`] is the pictures and files the conversations show, kept as
//!   they were shown.
//! - [`theme`] is how Cox looks, which the window and the page coxd
//!   serves a phone share, [`diagram`] the Mermaid diagrams drawn in its
//!   colors, and [`paths`] how paths are written.
//! - [`process`] is what Cox does as it starts a program, the agent's or
//!   a terminal's.
//! - [`storage`] says where this Cox keeps what lasts across its runs,
//!   [`scratch`] what it keeps only while it runs, and [`trash`] where
//!   what the human takes away goes.
//! - [`login`] takes in the login shell's environment, for a Cox the
//!   system opened, as from the Finder.
//! - [`logs`] is what Cox says of itself as it runs.
//! - [`home`] is the human's home directory.

pub mod agent;
pub mod agents;
pub mod chat;
pub mod daemon;
pub mod diagram;
pub mod home;
pub mod login;
pub mod logs;
pub mod media;
pub mod paths;
pub mod process;
pub mod project_icon;
pub mod raw;
pub mod repositories;
pub mod schedule;
pub mod scratch;
pub mod settings;
pub mod storage;
pub mod terminal;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
pub mod theme;
pub mod translate;
pub mod trash;
pub mod trigger;
pub mod wiki;
pub mod workflow;
