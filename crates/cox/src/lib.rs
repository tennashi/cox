//! The `cox` command: a GPUI window where a human talks to coding agents
//! through `cox-acp`, in sessions that each work in a directory of their
//! own.
//!
//! - [`workspace`] is the window: the sessions open in it, and the one
//!   shown. [`session`] starts a session's agent and stage.
//! - [`conversation`] holds what the window shows and decides what the human's
//!   input means, in plain Rust, so it is tested without GPUI. [`diff`]
//!   works out the lines a change to a file shows.
//! - [`agent`] runs the ACP connection on a tokio runtime of its own, and talks
//!   to the window over channels.
//! - [`input`] is the text the human types into, which an input method can
//!   compose into.
//! - [`view`] draws a session's conversation with GPUI and turns keystrokes
//!   and clicks into input.
//! - [`browser`] runs CEF, which renders a page for each session with no
//!   window of its own, and [`stage`] shows that page in the window and
//!   sends the human's input to it. The session's agent drives the same
//!   page over CDP. On Linux and macOS; on macOS, inside the app bundle
//!   `cargo xtask bundle` makes. [`profile`] is where the browser keeps
//!   logins and the like.
//! - [`relay`] stands between a session's agent's browser tools and the
//!   browser, keeps the agent to its session's page, and tells the stage
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
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod pointer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod profile;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod quit;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod relay;
pub mod session;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod stage;
pub mod tools;
pub mod view;
pub mod workspace;
