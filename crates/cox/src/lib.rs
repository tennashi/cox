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
//! - [`daemon`] is what runs the chats and their agents, apart from the
//!   window, which only shows them and tells it what the human does.
//! - [`chat`] is one chat: what starts it, its conversation, and how the
//!   conversation is drawn.
//! - [`stage`] is a chat's stage, beside its conversation: tabs of pages,
//!   files, a tree of the project's files, and terminals.
//! - [`settings`] is the settings screen, and the window following what
//!   the human sets as it changes.
//! - [`notices`] tells the human of chats that need them while they look
//!   elsewhere.
//! - [`inspect`] answers for the window to an agent that debugs Cox, on a
//!   socket, and is the client that asks it.
//! - [`vim`] is the keys as in Vim, which the human turns on, read ahead
//!   of the window's own.
//! - [`ui`] is what all of the window's parts share: the text the human
//!   types into, text that can be selected, Markdown, and how it all
//!   looks.
//!
//! Below them all are [`coxd`], what runs the chats apart from the
//! window, with what the two share: a chat's agent and the tools Cox
//! hands it, its conversation, the agents a chat can talk to, what the
//! human sets, the wiki, what the agents read, the workflow, the
//! schedule, programs on a pseudoterminal, where Cox keeps what lasts,
//! and how it looks; and [`cox_browser`]: CEF, which renders the stages'
//! pages, and what keeps the agent to its chat's pages. Each is a crate
//! of its own, with no GPUI in it.

pub mod app;
pub mod busy;
pub mod chat;
pub mod daemon;
pub mod inspect;
pub mod notices;
pub mod settings;
pub mod stage;
pub mod trigger;
pub mod ui;
pub mod vim;
pub mod workspace;
