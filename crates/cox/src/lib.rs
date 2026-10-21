//! The `cox` command: a GPUI window where a human talks to a coding agent
//! through `cox-acp`.
//!
//! - [`conversation`] holds what the window shows and decides what the human's
//!   input means, in plain Rust, so it is tested without GPUI.
//! - [`agent`] runs the ACP connection on a tokio runtime of its own, and talks
//!   to the window over channels.
//! - [`input`] is the line the human types into, which an input method can
//!   compose into.
//! - [`view`] draws the conversation with GPUI and turns keystrokes and clicks
//!   into input.
//! - [`browser`] runs CEF, which renders a page with no window of its own,
//!   and [`stage`] shows that page in the window and sends the human's
//!   input to it. An agent drives the same page over CDP. Linux only for
//!   now.

pub mod agent;
#[cfg(target_os = "linux")]
pub mod browser;
pub mod conversation;
pub mod input;
#[cfg(target_os = "linux")]
pub mod stage;
pub mod view;
