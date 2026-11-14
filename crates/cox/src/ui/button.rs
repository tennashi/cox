//! The button of the window, one kind wherever a button is, so that each
//! reads as the others do: 28 points high, rounded by 8, filled; the one
//! the human most likely wants filled in the accent, and one that takes
//! something away quiet until the mouse rests on it.

use gpui::{Div, ElementId, FontWeight, Stateful, div, prelude::*, px, rgb};

use coxd::theme::{ACCENT, BUTTON, BUTTON_HOVER, DIM, FOREGROUND, GROUND};

/// How tall a button is.
pub const HEIGHT: f32 = 28.;
/// How round a button is.
pub const RADIUS: f32 = 8.;
/// How tall a line's input is, and a button beside one.
pub const INPUT_HEIGHT: f32 = 32.;

/// Which of the kinds a button is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A button as most are.
    Plain,
    /// The button the human most likely wants, of those beside it.
    Primary,
    /// A button that takes something away, which waits to be wanted.
    Quiet,
}

/// A button of `kind` that shows `label`, which a test finds by `id`.
/// Beside an input, a button is as tall as the input, which a caller
/// sets on it after.
pub fn button(id: impl Into<ElementId>, label: impl IntoElement, kind: Kind) -> Stateful<Div> {
    let id = id.into();
    let selector = id.to_string();
    let button = div()
        .id(id)
        .debug_selector(move || selector)
        .flex_shrink_0()
        .h(px(HEIGHT))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(RADIUS))
        .text_size(px(13.))
        .whitespace_nowrap()
        .cursor_pointer()
        .child(label);
    match kind {
        Kind::Plain => button
            .bg(rgb(BUTTON))
            .text_color(rgb(FOREGROUND))
            .hover(|style| style.bg(rgb(BUTTON_HOVER))),
        Kind::Primary => button
            .bg(rgb(ACCENT))
            .text_color(rgb(GROUND))
            .font_weight(FontWeight::SEMIBOLD)
            .hover(|style| style.opacity(0.9)),
        Kind::Quiet => button
            .text_color(rgb(DIM))
            .hover(|style| style.bg(rgb(BUTTON)).text_color(rgb(FOREGROUND))),
    }
}
