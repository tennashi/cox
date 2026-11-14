//! The sheet over the window the human drags a region of the screen on.

use gpui::{
    Context, FocusHandle, Focusable, MouseButton, MouseDownEvent, Pixels, Point, Window, div,
    prelude::*, px, rgb,
};

use super::{Chat, Workspace};
use crate::chat::region::{self, CancelRegion};
use crate::ui::theme::{ACCENT, BORDER_STRONG, FAINT, RAISED};

/// The sheet over the window the human drags a region on: where the drag
/// started, if it has, and where the mouse is.
pub(super) struct Sheet {
    from: Option<Point<Pixels>>,
    to: Point<Pixels>,
    focus: FocusHandle,
}

impl Workspace {
    /// Lays the sheet over the window, for the human to drag a region of
    /// it for the chat shown, when a chat is shown.
    pub(super) fn lay_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown().and_then(Chat::view).is_none() {
            return;
        }
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.region = Some(Sheet {
            from: None,
            to: window.mouse_position(),
            focus,
        });
        cx.notify();
    }

    /// Takes the sheet away, with no region.
    fn lift_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.region.take().is_some() {
            self.give_back_keys(window, cx);
            cx.notify();
        }
    }

    /// Gives the keyboard to the input of the chat shown, as after the
    /// sheet is lifted.
    fn give_back_keys(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.shown().and_then(Chat::view) {
            window.focus(&view.read(cx).input().focus_handle(cx));
        }
    }

    /// The drag ended at `at`: the region it covers goes to the chat
    /// shown, once its picture is taken, if it is big enough to be one.
    fn drop_sheet(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.region.take() else {
            return;
        };
        self.give_back_keys(window, cx);
        cx.notify();
        let Some(region) = sheet.from.and_then(|from| region::between(from, at)) else {
            return;
        };
        let described = region::described(region, cx);
        region::take(region, described, window, cx);
    }

    /// The sheet, while it is laid: the window dimmed but for the region
    /// dragged, and what to do on it.
    pub(super) fn render_sheet(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let sheet = self.region.as_ref()?;
        let dragged = sheet.from.and_then(|from| region::between(from, sheet.to));
        let hint = format!(
            "Drag over what to hand the chat · Escape gives up · {}",
            region::keys()
        );
        Some(
            div()
                .id("region-sheet")
                .debug_selector(|| "region-sheet".to_owned())
                .key_context(region::KEY_CONTEXT)
                .track_focus(&sheet.focus)
                .on_action(cx.listener(|workspace, _: &CancelRegion, window, cx| {
                    workspace.lift_sheet(window, cx);
                }))
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .cursor_crosshair()
                .bg(gpui::rgba(0x0000_0040))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|workspace, event: &MouseDownEvent, _, cx| {
                        if let Some(sheet) = &mut workspace.region {
                            sheet.from = Some(event.position);
                            sheet.to = event.position;
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_move(
                    cx.listener(|workspace, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some(sheet) = &mut workspace.region {
                            sheet.to = event.position;
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|workspace, event: &gpui::MouseUpEvent, window, cx| {
                        workspace.drop_sheet(event.position, window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|workspace, _, window, cx| workspace.lift_sheet(window, cx)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(12.))
                        .left_0()
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px(px(12.))
                                .py(px(6.))
                                .rounded(px(8.))
                                .bg(rgb(RAISED))
                                .border_1()
                                .border_color(rgb(BORDER_STRONG))
                                .text_size(px(12.))
                                .text_color(rgb(FAINT))
                                .child(hint),
                        ),
                )
                .children(dragged.map(|region| {
                    div()
                        .absolute()
                        .left(region.origin.x)
                        .top(region.origin.y)
                        .w(region.size.width)
                        .h(region.size.height)
                        .border_1()
                        .border_color(rgb(ACCENT))
                        .bg(gpui::rgba(0xffff_ff14))
                })),
        )
    }
}
