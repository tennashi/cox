//! What every part of the window shares, whatever it shows.
//!
//! - [`input`] is the text the human types into, which an input method
//!   can compose into.
//! - [`text`] draws text so that the human can select it and click its
//!   links, replies as the Markdown that [`markdown`] reads, with the
//!   Mermaid diagrams in it that [`diagram`] draws; [`selection`] is what
//!   the human selected.
//! - [`theme`] is how things look, [`icons`] the icons drawn, and
//!   [`paths`] how paths are written.
//! - The [`Copy`] action and the [`LinkClicked`] event are what the parts
//!   tell the window alike, and a [`Panel`] is what the window and a
//!   stage share: whether the stage is in sight.
//! - A [`card`] is what the window's parts sit in, on its ground.
//! - [`translation`] shows what the human selected in their language.
//! - [`screen`] captures what the screen shows of the window.

pub mod button;
pub mod composing;
pub mod diagram;
pub mod ellipsis;
pub mod icons;
pub mod input;
pub mod markdown;
pub mod menu;
pub mod paths;
pub mod screen;
pub mod selection;
pub mod text;
pub mod theme;
pub mod translation;

use gpui::{
    AnyView, App, AppContext, Context, Div, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, actions, div, px, rgb,
};

use theme::{BORDER, BORDER_STRONG, CARD_RADIUS, FOREGROUND, RAISED, SURFACE, UI_FONT};

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

/// Whether a stage is in sight, in the middle of the window.
#[derive(Debug, Default)]
pub struct Panel {
    open: bool,
}

impl Panel {
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
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

/// What the mouse resting on a button says of it.
struct Tip(SharedString);

/// Builds the tip that says `text`, for an element's `tooltip`: what
/// the mouse resting on it says of it.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

impl Render for Tip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(BORDER_STRONG))
            .rounded(px(8.))
            .text_color(rgb(FOREGROUND))
            .font_family(UI_FONT)
            .text_size(px(12.))
            .child(self.0.clone())
    }
}
