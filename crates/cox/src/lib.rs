//! The `cox` command: a GPUI window where a human talks to coding agents
//! through `cox-acp`, in chats under projects: each chat works in its
//! project's folder, with its project's instructions.
//!
//! - [`workspace`] is the window: the projects and the chats open in them,
//!   and the chat shown. [`session`] starts a chat's agent and stage, and
//!   [`instructions`] reads what the agent gets told of the project.
//! - [`conversation`] holds what the window shows and decides what the human's
//!   input means, in plain Rust, so it is tested without GPUI. [`diff`]
//!   works out the lines a change to a file shows.
//! - [`agent`] runs the ACP connection on a tokio runtime of its own, and talks
//!   to the window over channels.
//! - [`input`] is the text the human types into, which an input method can
//!   compose into.
//! - [`view`] draws a chat's conversation with GPUI and turns keystrokes
//!   and clicks into input. [`text`] draws the conversation's text so that
//!   the human can select it and click its links, replies as the Markdown
//!   that [`markdown`] reads; [`selection`] is what the human selected.
//! - [`browser`] runs CEF, which renders a page for each chat with no
//!   window of its own, and [`stage`] shows that page in the window and
//!   sends the human's input to it. The chat's agent drives the same
//!   page over CDP. On Linux and macOS; on macOS, inside the app bundle
//!   `cargo xtask bundle` makes. [`profile`] is where the browser keeps
//!   logins and the like.
//! - [`relay`] stands between a chat's agent's browser tools and the
//!   browser, keeps the agent to its chat's page, and tells the stage
//!   where the agent points, clicks and types, which [`pointer`] turns
//!   into what the stage draws over the page.
//! - [`tools`] says which MCP servers the agent gets: the stage's.
//! - [`quit`] lets signals, and CEF on macOS, ask the app to quit.

pub mod agent;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod browser;
pub mod conversation;
pub mod diff;
pub mod input;
pub mod instructions;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod markdown;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod pointer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod profile;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod quit;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod relay;
pub mod selection;
pub mod session;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod stage;
pub mod text;
pub mod tools;
pub mod view;
pub mod workspace;
