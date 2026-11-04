//! The settings of a chat's session in the bar above its conversation:
//! each a button that names the setting and what it is set to, as the
//! agent names them, and opens a list of what it can be set to, under
//! the groups the agent puts them in, with the one it is set to marked.
//! A click on one sets it; a click anywhere else, its button too, closes
//! the list.

use gpui::{
    AnchoredPositionMode, AnyElement, Context, ElementId, Focusable as _, MouseButton,
    MouseDownEvent, anchored, deferred, div, point, prelude::*, px, rgb,
};

use cox_core::settings::Setting;

use super::ChatView;
use crate::ui::theme::DIM;

/// How wide the list of a setting's choices is.
const CHOICES_WIDTH: f32 = 260.;

/// How tall a setting's button is, which its list opens under.
const BUTTON_HEIGHT: f32 = 20.;

impl ChatView {
    /// A button for each setting of the session, with the list of its
    /// choices under the one that is open.
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.conversation
            .settings()
            .iter()
            .enumerate()
            .map(|(index, setting)| {
                let open = self.setting_open == Some(index);
                let chosen = setting
                    .chosen()
                    .map_or(setting.value.as_str(), |choice| choice.name.as_str());
                let button = div()
                    .id(ElementId::NamedInteger("setting".into(), index as u64))
                    .debug_selector(move || format!("setting-{index}"))
                    .flex()
                    .gap_1()
                    .px_2()
                    .rounded_sm()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(open, |button| button.bg(rgb(0x30_3030)))
                    .hover(|style| style.bg(rgb(0x30_3030)))
                    .child(div().text_color(rgb(DIM)).child(setting.name.clone()))
                    .child(chosen.to_owned())
                    .child(div().text_color(rgb(DIM)).child("▾"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                            // The press that closed the list, on its own
                            // button, leaves it closed.
                            let closed = view.setting_closed_at.take() == Some(event.position);
                            view.setting_open =
                                (!closed && view.setting_open != Some(index)).then_some(index);
                            // The keyboard stays in the input, as the window
                            // would take it otherwise.
                            window.focus(&view.input.focus_handle(cx));
                            window.prevent_default();
                            cx.notify();
                        }),
                    );
                div()
                    .flex()
                    .flex_col()
                    .child(button)
                    .when(open, |column| {
                        column.child(Self::render_choices(setting, cx))
                    })
                    .into_any_element()
            })
            .collect()
    }

    /// The list of what `setting` can be set to, drawn over the
    /// conversation, under its button.
    fn render_choices(setting: &Setting, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .debug_selector(|| "setting-choices".to_owned())
            .occlude()
            .w(px(CHOICES_WIDTH))
            .p_1()
            .bg(rgb(0x25_2526))
            .border_1()
            .border_color(rgb(0x45_4545))
            .rounded_md()
            .shadow_md()
            .text_size(px(12.))
            .on_mouse_down_out(cx.listener(|view, event: &MouseDownEvent, _, cx| {
                view.setting_open = None;
                view.setting_closed_at = Some(event.position);
                cx.notify();
            }));
        let mut group = None;
        for (index, choice) in setting.choices.iter().enumerate() {
            if choice.group.is_some() && choice.group != group {
                group.clone_from(&choice.group);
                list = list.child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_color(rgb(DIM))
                        .children(group.clone()),
                );
            }
            let chosen = choice.id == setting.value;
            let (id, value) = (setting.id.clone(), choice.id.clone());
            list =
                list.child(
                    div()
                        .id(ElementId::NamedInteger("choice".into(), index as u64))
                        .debug_selector(move || format!("choice-{index}"))
                        .flex()
                        .gap_2()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x04_395e)))
                        .child(
                            div()
                                .w(px(10.))
                                .flex_shrink_0()
                                .when(chosen, |mark| mark.child("✓")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(choice.name.clone())
                                .children(choice.description.clone().map(|description| {
                                    div().text_color(rgb(DIM)).child(description)
                                })),
                        )
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.set(&id, &value, cx);
                            window.focus(&view.input.focus_handle(cx));
                        })),
                );
        }
        // Under the button, which is a line of text high.
        let under = point(px(0.), px(BUTTON_HEIGHT));
        deferred(
            anchored()
                .position_mode(AnchoredPositionMode::Local)
                .position(under)
                .snap_to_window()
                .child(list),
        )
        .with_priority(2)
    }
}
