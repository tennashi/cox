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
//! - [`agent`] is a chat's agent: the ACP connection, and the tools Cox
//!   hands the agent.
//! - [`agents`] is the agents a chat can talk to: the command line's,
//!   the human's, and the ACP Registry's.
//! - [`settings`] is what the human sets of Cox itself, on its settings
//!   screen: the folder Cox's own chats work in.
//! - [`wiki`] is the human's wiki, where the agents keep what they
//!   learned, for those of every project to find again.
//! - [`raw`] is what the agents read, kept as they read it, as the
//!   sources of research, apart from what is made of it.
//! - [`schedule`] is the agents the human has run on a schedule, a chat
//!   that coxd opens at the times they say.
//! - [`workflow`] is what Cox tells a chat's agent in the human's place
//!   at the turns of a sketch, which the human's workflow can say instead.
//! - [`notices`] tells the human of chats that need them while they look
//!   elsewhere.
//! - [`inspect`] answers for the window to an agent that debugs Cox, on a
//!   socket, and is the client that asks it.
//! - [`vim`] is the keys as in Vim, which the human turns on, read ahead
//!   of the window's own.
//! - [`ui`] is what all of the window's parts share: the text the human
//!   types into, text that can be selected, Markdown, and how it all
//!   looks.
//! - [`terminal`] is a program on a pseudoterminal, as a tab of the
//!   stage shows it, or as coxd runs it with none.
//! - [`logs`] is what Cox says of itself as it runs.
//!
//! Below them all are [`coxd`], what runs the chats apart from the
//! window, with what the two share: where Cox keeps what lasts, how it
//! starts programs, and how it looks; and [`cox_browser`]: CEF, which
//! renders the stages' pages, and what keeps the agent to its chat's
//! pages. Each is a crate of its own, with no GPUI in it.

pub mod agent;
pub mod agents;
pub mod app;
pub mod busy;
pub mod chat;
pub mod daemon;
pub mod inspect;
pub mod logs;
pub mod media;
pub mod notices;
pub mod project_icon;
pub mod raw;
pub mod repositories;
pub mod schedule;
pub mod settings;
pub mod stage;
pub mod terminal;
pub mod translate;
pub mod trigger;
pub mod ui;
pub mod vim;
pub mod wiki;
pub mod workflow;
pub mod workspace;
