//! One chat: a human and an agent working in a folder, with a stage of
//! their own. What the window shows of it, and what starts it.
//!
//! - [`session`] starts a chat: its agent, its stage, the relay and the
//!   tools between them; and starts a closed chat again.
//! - [`view`] draws a chat's conversation with GPUI, its stage beside it,
//!   and turns keystrokes and clicks into input.
//! - [`conversation`] holds what the view shows and decides what the
//!   human's input means, in plain Rust, so it is tested without GPUI.
//!   `cox-core` keeps what it shows.

pub mod conversation;
pub mod session;
pub mod view;
