//! The button of the window, one kind wherever a button is, so that each
//! reads as the others do: 28 points high, rounded as a control is,
//! filled, and standing a little up from its ground, as what the human
//! clicks does; the one the human most likely wants filled in the accent,
//! glowing in it, and one that takes something away quiet until the mouse
//! rests on it.

use gpui::{Div, ElementId, FontWeight, Stateful, div, prelude::*, px, rgb, rgba};

use coxd::theme::{
    ACCENT, BUTTON, BUTTON_HOVER, CONTROL, CONTROL_RADIUS, DIM, FOREGROUND, GROUND, PRIMARY_EDGE,
    PRIMARY_GLOW,
};

use crate::ui::depth::{self, Stand as _};

/// How tall a button is.
pub const HEIGHT: f32 = 28.;
/// How round a button is.
pub const RADIUS: f32 = CONTROL_RADIUS;
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
            .stand(CONTROL)
            .bg(rgb(BUTTON))
            .text_color(rgb(FOREGROUND))
            .hover(|style| style.bg(rgb(BUTTON_HOVER))),
        Kind::Primary => button
            .border_1()
            .border_color(rgba(PRIMARY_EDGE))
            .shadow(
                depth::shadows(CONTROL)
                    .into_iter()
                    .chain([depth::shadow(PRIMARY_GLOW)])
                    .collect(),
            )
            .bg(rgb(ACCENT))
            .text_color(rgb(GROUND))
            .font_weight(FontWeight::SEMIBOLD)
            .hover(|style| style.opacity(0.9)),
        Kind::Quiet => button
            .text_color(rgb(DIM))
            .hover(|style| style.bg(rgb(BUTTON)).text_color(rgb(FOREGROUND))),
    }
}
