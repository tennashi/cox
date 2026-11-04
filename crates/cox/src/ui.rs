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
//! - A [`card`] is what the window's parts sit in, on its ground.

pub mod input;
pub mod markdown;
pub mod paths;
pub mod selection;
pub mod text;
pub mod theme;

use gpui::{Context, Div, Styled, actions, div, px, rgb};

use theme::{BORDER, CARD_RADIUS, SURFACE};

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

/// A card: a part of the window, such as a chat's conversation or its
/// stage, drawn apart from the ground and from the cards beside it.
///
/// Its corners are round, but what it holds is not cut to them, so a
/// part that fills its card to a corner leaves its ground to the card,
/// or rounds its own corner by [`theme::CARD_INNER_RADIUS`].
#[must_use]
pub fn card() -> Div {
    div()
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded(px(CARD_RADIUS))
}
