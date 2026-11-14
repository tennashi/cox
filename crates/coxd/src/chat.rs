//! One chat, as coxd runs it, apart from how the window draws it.
//!
//! - [`judge`] is the agent that judges whether a chat's goal is met.
//! - [`sketch`] is a chat that tries something out apart from its
//!   project.
//! - [`mention`] is the files the human points the agent to with `@`.

pub mod judge;
pub mod mention;
pub mod sketch;
