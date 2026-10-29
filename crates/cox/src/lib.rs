//! The `cox` command: a GPUI window where a human talks to coding agents
//! through `cox-acp`, in chats under projects: each chat works in its
//! project's folder, with its project's instructions. It runs on Linux
//! and macOS, where CEF does.
//!
//! The modules go from the top down, each using only what is below it:
//!
//! - [`app`] is where it all starts, in order: what the command line
//!   asks for, what the app depends on, the app's parts, and the window.
//! - [`workspace`] is the window: the projects and the chats open in them,
//!   and the chat shown.
//! - [`chat`] is one chat: what starts it, its conversation, and how the
//!   conversation is drawn.
//! - [`stage`] is a chat's stage, beside its conversation: tabs of pages,
//!   files, a tree of the project's files, and terminals.
//! - [`agent`] is a chat's agent: the ACP connection, and the tools Cox
//!   hands the agent.
//! - [`browser`] is CEF, which renders the stages' pages, and what keeps
//!   the agent to its chat's pages.
//! - [`ui`] is what all of the window's parts share: the text the human
//!   types into, text that can be selected, Markdown, and how it all
//!   looks.
//! - [`storage`] says where this Cox keeps what lasts across its runs.

pub mod agent;
pub mod app;
pub mod browser;
pub mod chat;
pub mod stage;
pub mod storage;
pub mod ui;
pub mod workspace;
