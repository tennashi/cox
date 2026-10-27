//! The `cox` command: a GPUI window where a human talks to coding agents
//! through `cox-acp`, in chats under projects: each chat works in its
//! project's folder, with its project's instructions.
//!
//! - [`workspace`] is the window: the projects and the chats open in them,
//!   and the chat shown. [`session`] starts a chat's agent and stage, and
//!   [`instructions`] reads what the agent gets told of the project.
//! - [`conversation`] holds what the window shows and decides what the human's
//!   input means, in plain Rust, so it is tested without GPUI. `cox-core`
//!   keeps what it shows, and the chats, where [`place`] says this Cox
//!   keeps what lasts across its runs.
//! - [`agent`] runs the ACP connection on a tokio runtime of its own, and talks
//!   to the window over channels.
//! - [`input`] is the text the human types into, which an input method can
//!   compose into.
//! - [`view`] draws a chat's conversation with GPUI and turns keystrokes
//!   and clicks into input. [`text`] draws the conversation's text so that
//!   the human can select it and click its links, replies as the Markdown
//!   that [`markdown`] reads; [`selection`] is what the human selected.
//! - [`browser`] runs CEF, which renders pages with no window of their
//!   own. Each chat's [`stage`] holds tabs of them, which open beside the
//!   conversation when they are used, and each tab's [`page`] shows its
//!   page in the window and sends the human's input to it. The chat's
//!   agent drives the same pages over CDP. On Linux and macOS; on macOS,
//!   inside the app bundle `cargo xtask bundle` makes. A tab can show a
//!   Markdown file as a [`document`] instead, or a file as it is written,
//!   such as code, as [`code`]: both [`follow`] the file as it changes.
//!   A tab of code asks the project's language server, of [`servers`],
//!   what the code means. Or a tab is a [`terminal`], which the human types
//!   into, and the agent runs commands in. [`profile`] is where the
//!   browser keeps logins and the like.
//! - [`relay`] stands between a chat's agent's browser tools and the
//!   browser, keeps the agent to its chat's pages, opens and closes tabs
//!   for it, and tells the stage where the agent points, clicks and types,
//!   which [`pointer`] turns into what the tab draws over the page.
//! - [`tools`] says which MCP servers the agent gets: the stage's, and
//!   Cox's own, which [`mcp`] serves to each chat's agent.
//! - [`quit`] lets signals, and CEF on macOS, ask the app to quit.

pub mod agent;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod browser;
pub mod code;
pub mod conversation;
pub mod document;
pub mod follow;
pub mod input;
pub mod instructions;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod markdown;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod mcp;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod page;
pub mod place;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod pointer;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod profile;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod quit;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod relay;
pub mod selection;
pub mod servers;
pub mod session;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod stage;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod terminal;
pub mod text;
pub mod tools;
pub mod view;
pub mod workspace;
