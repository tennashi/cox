//! What every part of the window shares, whatever it shows.
//!
//! - [`input`] is the text the human types into, which an input method
//!   can compose into.
//! - [`text`] draws text so that the human can select it and click its
//!   links, replies as the Markdown that [`markdown`] reads;
//!   [`selection`] is what the human selected.
//! - [`theme`] is how things look, and [`paths`] how paths are written.
//! - The [`Copy`] action and the [`LinkClicked`] event are what the parts
//!   tell the window alike, and a [`Panel`] is what the conversation and
//!   the stage of a chat share: whether the stage is open.

pub mod input;
pub mod markdown;
pub mod paths;
pub mod selection;
pub mod text;
pub mod theme;

use gpui::{Context, actions};

actions!(
    cox,
    [
        /// Copies the text selected in the part with the keyboard.
        Copy,
    ]
);

/// The human clicked a link to the address it holds, which the window
/// opens on the chat's stage.
pub struct LinkClicked(pub String);

/// Whether a chat's stage is open beside its conversation.
#[derive(Debug, Default)]
pub struct Panel {
    open: bool,
}

impl Panel {
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.set(true, cx);
    }

    pub fn set(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open != open {
            self.open = open;
            cx.notify();
        }
    }
}
