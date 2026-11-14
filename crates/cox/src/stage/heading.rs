//! What the tabs of the stage that Cox draws have in common, so that each
//! reads as the others do: the heading they open with, as "Changes"
//! first drew it, with the tab's title, a line on what it shows, and,
//! at the right edge, where it shows it from; and the cards they hold
//! their rows in.

use gpui::{Div, FontWeight, IntoElement, SharedString, div, prelude::*, px, rgb};

use crate::ui::ellipsis::ellipsis;
use coxd::theme::{BORDER, CODE_FONT, DIM, FAINT, UI_FONT};

/// How far the cards are from the tab's edges; the heading's text, and
/// what a tab says outside its cards, are a little further in, in line
/// with the text in the cards.
pub const INSET: f32 = 16.;
/// How far in the heading's text is from the tab's edges.
pub const TEXT_INSET: f32 = INSET + 4.;
/// How round a card is.
pub const CARD_RADIUS: f32 = 12.;
/// The size of a tab's text.
pub const TEXT_SIZE: f32 = 13.;

/// A tab's heading: `title`, then `said`, dim unless it says otherwise,
/// and `place`, as a path, at the right edge, ending in an ellipsis
/// where the heading has no room for it all. A test finds it as
/// `selector`.
pub fn heading(
    selector: &'static str,
    title: impl Into<SharedString>,
    said: impl IntoElement,
    place: Option<String>,
) -> Div {
    div()
        .debug_selector(move || selector.to_owned())
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(TEXT_INSET))
        .pt_3()
        .pb_1()
        .whitespace_nowrap()
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_color(rgb(DIM))
                .child(said),
        )
        .child(
            // Grown to the right edge, rather than pushed there by an
            // auto margin, which the heading's gaps then go missing
            // beside.
            div()
                .flex_1()
                .flex()
                .justify_end()
                .min_w_0()
                .overflow_hidden()
                .font_family(CODE_FONT)
                .text_size(px(11.5))
                .text_color(rgb(FAINT))
                .children(place.map(ellipsis)),
        )
}

/// A tab's body, in the tab's font and size, which its heading and cards
/// go in, filling the tab.
#[must_use]
pub fn body() -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .font_family(UI_FONT)
        .text_size(px(TEXT_SIZE))
}

/// A card a tab holds rows in, which part themselves with [`parted`].
#[must_use]
pub fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .border_1()
        .border_color(rgb(BORDER))
        .rounded(px(CARD_RADIUS))
        .overflow_hidden()
}

/// `row`, parted from the row over it in its card, unless it is the
/// first.
pub fn parted<E: Styled>(row: E, first: bool) -> E {
    if first {
        row
    } else {
        row.border_t_1().border_color(rgb(BORDER))
    }
}

/// What a tab says outside its cards, as that it has nothing to show, in
/// `color`, wrapped to the tab's width.
pub fn note(selector: &'static str, text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .debug_selector(move || selector.to_owned())
        .px(px(TEXT_INSET))
        .py_2()
        .text_color(rgb(color))
        .child(text.into())
}
