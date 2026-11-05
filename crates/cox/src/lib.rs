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
//! - [`agents`] is the agents a chat can talk to: the command line's,
//!   the human's, and the ACP Registry's.
//! - [`settings`] is what the human sets of Cox itself, on its settings
//!   screen: the folder chats in no project work in.
//! - [`wiki`] is the human's wiki, where the agents keep what they
//!   learned, for those of every project to find again.
//! - [`workflow`] is what Cox tells a chat's agent in the human's place
//!   at the turns of a sketch, which the human's workflow can say instead.
//! - [`notices`] tells the human of chats that need them while they look
//!   elsewhere.
//! - [`inspect`] answers for the window to an agent that debugs Cox, on a
//!   socket, and is the client that asks it.
//! - [`ui`] is what all of the window's parts share: the text the human
//!   types into, text that can be selected, Markdown, and how it all
//!   looks.
//! - [`process`] is what Cox does as it starts a program, the agent's or
//!   a terminal's.
//! - [`storage`] says where this Cox keeps what lasts across its runs.
//! - [`logs`] is what Cox says of itself as it runs.
//!
//! Below them all is [`cox_browser`]: CEF, which renders the stages'
//! pages, and what keeps the agent to its chat's pages. It is a crate of
//! its own, with no GPUI in it.

pub mod agent;
pub mod agents;
pub mod app;
pub mod chat;
pub mod inspect;
pub mod logs;
pub mod notices;
pub mod process;
pub mod settings;
pub mod stage;
pub mod storage;
pub mod translate;
pub mod ui;
pub mod wiki;
pub mod workflow;
pub mod workspace;
