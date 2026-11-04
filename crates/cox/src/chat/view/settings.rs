//! The agent of a chat and the settings of its session in the bar above
//! its conversation: each a button that names the setting and what it is
//! set to, as the agent names them, and opens a list of what it can be
//! set to, under the groups the agent puts them in, with the one it is
//! set to marked. A click on one sets it; a click anywhere else, its
//! button too, closes the list. The agent comes first, listing the
//! agents the chat can talk to, those of the ACP Registry under its name,
//! and is chosen only while no turn runs.

use gpui::{
    AnchoredPositionMode, AnyElement, Context, ElementId, Focusable as _, MouseButton,
    MouseDownEvent, anchored, deferred, div, point, prelude::*, px, rgb,
};

use cox_core::settings::{Choice, Setting};

use super::ChatView;
use crate::agents::REGISTRY_GROUP;
use crate::chat::conversation::State;
use crate::ui::theme::{ACCENT_GROUND, BORDER_STRONG, BUTTON, DIM, RAISED};

/// How wide the list of a setting's choices is.
const CHOICES_WIDTH: f32 = 260.;

/// How tall the list of a setting's choices grows before it scrolls.
const CHOICES_HEIGHT: f32 = 360.;

/// How tall a setting's button is, which its list opens under.
const BUTTON_HEIGHT: f32 = 20.;

/// A button of the bar, whose list of choices opens under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Menu {
    /// The agent the chat talks to.
    Agent,
    /// A setting of the session, by its place in the conversation's
    /// settings.
    Setting(usize),
}

impl ChatView {
    /// A button for the agent, when the chat can choose one, and for each
    /// setting of the session, with the list of its choices under the one
    /// that is open.
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let idle = matches!(self.conversation.state(), State::Idle | State::Closed);
        self.agent_setting()
            .map(|agent| self.render_setting(Menu::Agent, &agent, idle, cx))
            .into_iter()
            .chain(
                self.conversation
                    .settings()
                    .iter()
                    .enumerate()
                    .map(|(index, setting)| {
                        self.render_setting(Menu::Setting(index), setting, true, cx)
                    }),
            )
            .collect()
    }

    /// The agent the chat talks to, as a setting whose choices are the
    /// agents it can talk to, when it can choose.
    fn agent_setting(&self) -> Option<Setting> {
        let (agents, chosen) = self.agents.as_ref()?;
        let choices = agents
            .all()
            .into_iter()
            .map(|known| Choice {
                id: known.id,
                name: known.name,
                // An agent whose program Cox downloads says so, as it takes
                // a while the first time.
                description: known
                    .download
                    .map(|download| format!("Downloads {}", download.version)),
                group: known.listed.then(|| REGISTRY_GROUP.to_owned()),
            })
            .collect();
        Some(Setting {
            id: "agent".to_owned(),
            name: "Agent".to_owned(),
            description: None,
            category: None,
            value: chosen.clone(),
            choices,
        })
    }

    /// The button of `setting`, which `menu` says which it is, with the
    /// list of its choices under it while it is open. A button not
    /// `enabled` opens nothing.
    fn render_setting(
        &self,
        menu: Menu,
        setting: &Setting,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.setting_open == Some(menu);
        let chosen = setting
            .chosen()
            .map_or(setting.value.as_str(), |choice| choice.name.as_str());
        let (id, selector) = match menu {
            Menu::Agent => (ElementId::Name("agent".into()), "agent".to_owned()),
            Menu::Setting(index) => (
                ElementId::NamedInteger("setting".into(), index as u64),
                format!("setting-{index}"),
            ),
        };
        let button = div()
            .id(id)
            .debug_selector(move || selector.clone())
            .h(px(24.))
            .flex()
            .items_center()
            .gap_1()
            .px(px(10.))
            .rounded_full()
            .bg(rgb(RAISED))
            .text_size(px(12.))
            .when(open, |button| button.bg(rgb(BUTTON)))
            .when(enabled, |button| {
                button.cursor_pointer().hover(|style| style.bg(rgb(BUTTON)))
            })
            .when(!enabled, |button| button.opacity(0.5))
            .child(div().text_color(rgb(DIM)).child(setting.name.clone()))
            .child(chosen.to_owned())
            .child(div().text_color(rgb(DIM)).child("▾"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, window, cx| {
                    // The press that closed the list, on its own button,
                    // leaves it closed.
                    let closed = view.setting_closed_at.take() == Some(event.position);
                    view.setting_open =
                        (enabled && !closed && view.setting_open != Some(menu)).then_some(menu);
                    // The keyboard stays in the input, as the window would
                    // take it otherwise.
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
                column.child(Self::render_choices(menu, setting, cx))
            })
            .into_any_element()
    }

    /// The list of what `setting` can be set to, drawn over the
    /// conversation, under its button.
    fn render_choices(menu: Menu, setting: &Setting, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .id("setting-choices")
            .debug_selector(|| "setting-choices".to_owned())
            .occlude()
            .w(px(CHOICES_WIDTH))
            .max_h(px(CHOICES_HEIGHT))
            .overflow_y_scroll()
            .p_1()
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(BORDER_STRONG))
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
                        .hover(|style| style.bg(rgb(ACCENT_GROUND)))
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
                            match menu {
                                Menu::Agent => view.choose_agent(&value, cx),
                                Menu::Setting(_) => view.set(&id, &value, cx),
                            }
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
