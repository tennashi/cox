//! The window's side of what the human sets of Cox itself, which
//! [`coxd::settings`] reads and keeps.
//!
//! - [`view`] is the settings screen.
//! - [`window`] follows `settings.json` and `agents.json` as they change.

pub mod view;
pub mod window;
