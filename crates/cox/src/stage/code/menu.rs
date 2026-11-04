//! The menu of the right button, for the word under the mouse: go to
//! where it is defined, or list where it is used.

use gpui::{Context, IntoElement, anchored, deferred, div, prelude::*, px, rgb};

use super::Code;
use crate::ui::theme::{ACCENT_GROUND, BORDER_STRONG, DIM, FOREGROUND, RAISED};

pub(super) const MENU: u32 = RAISED;
pub(super) const MENU_BORDER: u32 = BORDER_STRONG;

impl Code {
    /// The menu of the right button, if it is open.
    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (at, place) = self.menu?;
        let click = if cfg!(target_os = "macos") {
            "⌘-click"
        } else {
            "Ctrl-click"
        };
        let item = |id: &'static str, label: &'static str, key: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .flex()
                .justify_between()
                .gap_4()
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(ACCENT_GROUND)))
                .child(label)
                .child(div().text_color(rgb(DIM)).child(key))
        };
        let menu = div()
            .occlude()
            .min_w(px(220.))
            .p_1()
            .bg(rgb(MENU))
            .border_1()
            .border_color(rgb(MENU_BORDER))
            .rounded_md()
            .shadow_md()
            .text_color(rgb(FOREGROUND))
            .on_mouse_down_out(cx.listener(|code, _, _, cx| {
                code.menu = None;
                cx.notify();
            }))
            .child(
                item("go-to-definition", "Go to definition", click).on_click(cx.listener(
                    move |code, _, _, cx| {
                        code.menu = None;
                        code.go_to_definition(place, cx);
                    },
                )),
            )
            .child(
                item("find-references", "Find references", "").on_click(cx.listener(
                    move |code, _, _, cx| {
                        code.menu = None;
                        code.find_references(place, cx);
                    },
                )),
            );
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}
