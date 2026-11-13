//! The menu of the window, one kind wherever a menu opens, as the menu
//! of the stage's plus draws it: raised, with a strong edge, rounded, its
//! rows the height of a button, the row under the mouse lit, and the keys
//! that do the same at a row's right.

use gpui::{Div, ElementId, Stateful, div, prelude::*, px, rgb};

use crate::ui::theme::{BORDER, BORDER_STRONG, FAINT, FOREGROUND, RAISED, SELECTED};

/// How round a menu is, and a row of it.
const RADIUS: f32 = 14.;
pub const ROW_RADIUS: f32 = 9.;
/// How tall a row is.
pub const ROW_HEIGHT: f32 = 30.;

/// A menu, which its rows go in, one under another.
#[must_use]
pub fn menu() -> Div {
    div()
        .occlude()
        .child(crate::vim::hints::cover())
        .p(px(6.))
        .flex()
        .flex_col()
        .bg(rgb(RAISED))
        .border_1()
        .border_color(rgb(BORDER_STRONG))
        .rounded(px(RADIUS))
        .shadow_lg()
        .text_size(px(13.))
        .text_color(rgb(FOREGROUND))
}

/// A row of a menu that says `label`, with `keys` at its right, those
/// that do the same, if any, which a test finds by `id`.
pub fn item(id: impl Into<ElementId>, label: impl IntoElement, keys: &str) -> Stateful<Div> {
    let id = id.into();
    let selector = id.to_string();
    div()
        .id(id)
        .debug_selector(move || selector)
        .h(px(ROW_HEIGHT))
        .px(px(10.))
        .flex()
        .items_center()
        .gap_4()
        .rounded(px(ROW_RADIUS))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SELECTED)))
        .child(div().flex_1().child(label))
        .when(!keys.is_empty(), |row| {
            row.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(FAINT))
                    .child(keys.to_owned()),
            )
        })
}

/// A line that parts the rows of a menu.
#[must_use]
pub fn parting() -> Div {
    div().my(px(4.)).mx(px(6.)).h(px(1.)).bg(rgb(BORDER))
}
