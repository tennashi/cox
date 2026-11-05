//! The bar above the code.

use gpui::{Context, IntoElement, div, prelude::*, px, rgb};

use super::Code;
use crate::ui::ellipsis::ellipsis;
use crate::ui::paths::shortened;
use crate::ui::theme::{BORDER, BUTTON_HOVER, DIM, DISABLED, FAILED, FOREGROUND};

impl Code {
    /// The bar above the code: back and forward, the file's path, the
    /// lines it is shown at, and what the tab says of what the human asked,
    /// the server, and the file when it cannot be read.
    pub(super) fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.look.marked.map(|lines| {
            if lines.first == lines.last {
                format!("line {}", lines.first)
            } else {
                format!("lines {}–{}", lines.first, lines.last)
            }
        });
        let server = self
            .server
            .as_ref()
            .and_then(|server| server.read(cx).said());
        let button = |id: &'static str, label: &'static str, enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .flex_shrink_0()
                .px_1()
                .rounded_sm()
                .map(|button| {
                    if enabled {
                        button
                            .cursor_pointer()
                            .text_color(rgb(FOREGROUND))
                            .hover(|style| style.bg(rgb(BUTTON_HOVER)))
                    } else {
                        button.text_color(rgb(DISABLED))
                    }
                })
                .child(label)
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .text_color(rgb(DIM))
            .when(
                self.history.has_back() || self.history.has_forward(),
                |bar| {
                    bar.child(
                        button("code-back", "‹", self.history.has_back())
                            .on_click(cx.listener(|code, _, _, cx| code.go_back(cx))),
                    )
                    .child(
                        button("code-forward", "›", self.history.has_forward())
                            .on_click(cx.listener(|code, _, _, cx| code.go_forward(cx))),
                    )
                },
            )
            .child(
                div()
                    .debug_selector(|| "code-path".to_owned())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(ellipsis(shortened(&self.file.path))),
            )
            .children(lines.map(|lines| div().flex_shrink_0().child(lines)))
            .children(self.notice.as_ref().map(|notice| {
                div()
                    .debug_selector(|| "code-notice".to_owned())
                    .flex_shrink_0()
                    .child(notice.clone())
            }))
            .children(server.map(|server| {
                div()
                    .debug_selector(|| "code-server".to_owned())
                    .min_w_0()
                    .max_w(px(240.))
                    .overflow_hidden()
                    .child(ellipsis(server))
            }))
            .children(self.file.unread.as_ref().map(|error| {
                div()
                    .flex_shrink_0()
                    .text_color(rgb(FAILED))
                    .child(format!("cannot read: {error}"))
            }))
    }
}
