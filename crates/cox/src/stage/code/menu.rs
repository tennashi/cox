//! The menu of the right button, for the word under the mouse: go to
//! where it is defined, or list where it is used; and, with text
//! selected, translate it.

use gpui::{Context, IntoElement, anchored, deferred, prelude::*, px, rgb};

use super::Code;
use crate::ui::selected::Selected as _;
use crate::ui::theme::{BORDER_STRONG, FOREGROUND, RAISED};
use crate::vim::hints::Hinted as _;

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
        let translate = if cfg!(target_os = "macos") {
            "⌘⇧T"
        } else {
            "Ctrl-Shift-T"
        };
        let selected = self.chosen_text();
        let handed = self.selected_attachment();
        let item = |id: &'static str, label: &'static str, key: &'static str| {
            crate::ui::menu::item(id, label, key)
        };
        let menu = crate::ui::menu::menu()
            .min_w(px(220.))
            .text_color(rgb(FOREGROUND))
            .on_mouse_down_out(cx.listener(|code, _, _, cx| {
                code.menu = None;
                cx.notify();
            }))
            .child(
                item("go-to-definition", "Go to definition", click).on_hint_click(cx.listener(
                    move |code, _, _, cx| {
                        code.menu = None;
                        code.go_to_definition(place, cx);
                    },
                )),
            )
            .child(
                item("find-references", "Find references", "").on_hint_click(cx.listener(
                    move |code, _, _, cx| {
                        code.menu = None;
                        code.find_references(place, cx);
                    },
                )),
            )
            .when(!selected.trim().is_empty(), |menu| {
                menu.child(
                    item("translate-selection", "Translate", translate).on_hint_click(cx.listener(
                        move |code, _, window, cx| {
                            code.menu = None;
                            crate::ui::translation::show(&selected, window, cx);
                            cx.notify();
                        },
                    )),
                )
            })
            .when(!handed.text.trim().is_empty(), |menu| {
                menu.child(
                    item("add-to-chat", "Add to chat", crate::chat::hand::keys()).on_hint_click(
                        cx.listener(move |code, _, _, cx| {
                            code.menu = None;
                            crate::chat::hand::hand(handed.clone(), cx);
                            cx.notify();
                        }),
                    ),
                )
            });
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}
