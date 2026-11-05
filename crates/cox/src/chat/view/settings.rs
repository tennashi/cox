//! The agent of a chat and the settings of its session, on one pill at
//! the right of the bar above its conversation: the agent's name, then
//! what each setting is set to, as the agent names them. A click on the
//! pill opens a list of the agent and the settings, each a row that names
//! it and what it is set to; a click on a row shows, in the list's place,
//! what it can be set to, under the groups the agent puts them in, with
//! the one it is set to marked, and a line back to the rows. A click on
//! one sets it; a click anywhere else, the pill too, closes the list. The
//! agent comes first, listing the agents the chat can talk to, those of
//! the ACP Registry under its name, and is chosen only while no turn
//! runs.

use gpui::{
    AnchoredPositionMode, AnyElement, Context, ElementId, Focusable as _, FontWeight, MouseButton,
    MouseDownEvent, anchored, deferred, div, point, prelude::*, px, rgb,
};

use cox_core::settings::{Choice, Setting};

use super::ChatView;
use crate::agents::REGISTRY_GROUP;
use crate::chat::conversation::State;
use crate::ui::theme::{ACCENT_GROUND, BORDER_STRONG, BUTTON, DIM, FOREGROUND, RAISED};

/// How wide the list of a setting's choices is.
const CHOICES_WIDTH: f32 = 260.;

/// How tall the list of a setting's choices grows before it scrolls.
const CHOICES_HEIGHT: f32 = 360.;

/// How tall the pill is, which the list opens under.
const PILL_HEIGHT: f32 = 28.;

/// What the list under the pill shows, while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Menu {
    /// The rows of the agent and the settings.
    Rows,
    /// What the agent the chat talks to can be.
    Agent,
    /// A setting of the session, by its place in the conversation's
    /// settings.
    Setting(usize),
}

impl ChatView {
    /// The pill of the agent and the settings of the session, with the
    /// list under it while it is open, when there is anything to show.
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let agent = self.agent_setting();
        let settings = self.conversation.settings();
        if agent.is_none() && settings.is_empty() {
            return None;
        }
        let idle = matches!(self.conversation.state(), State::Idle | State::Closed);
        let open = self.setting_open;
        let summary = settings
            .iter()
            .map(|setting| chosen_name(setting).to_owned())
            .collect::<Vec<_>>()
            .join(" · ");
        let pill = div()
            .id("settings")
            .debug_selector(|| "settings".to_owned())
            .h(px(PILL_HEIGHT))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(14.))
            .rounded_full()
            .border_1()
            .border_color(rgb(BUTTON))
            .bg(rgb(RAISED))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON)))
            .when(open.is_some(), |pill| pill.bg(rgb(BUTTON)))
            .children(agent.is_some().then(|| {
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(FOREGROUND))
                    .child(self.agent_name())
            }))
            .when(!summary.is_empty(), |pill| {
                pill.child(div().text_color(rgb(DIM)).child(summary))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    // The press that closed the list, on the pill, leaves
                    // it closed.
                    let closed = view.setting_closed_at.take() == Some(event.position);
                    view.setting_open =
                        (!closed && view.setting_open.is_none()).then_some(Menu::Rows);
                    // The keyboard stays in the input, as the window would
                    // take it otherwise.
                    window.focus(&view.input.focus_handle(cx));
                    window.prevent_default();
                    cx.notify();
                }),
            );
        let list = open.map(|menu| match menu {
            Menu::Rows => {
                let mut rows: Vec<AnyElement> = agent
                    .iter()
                    .map(|agent| Self::render_row(Menu::Agent, agent, idle, cx))
                    .collect();
                for (index, setting) in settings.iter().enumerate() {
                    rows.push(Self::render_row(Menu::Setting(index), setting, true, cx));
                }
                Self::render_list(rows, cx)
            }
            Menu::Agent => match &agent {
                Some(agent) => Self::render_choices(menu, agent, cx),
                None => Self::render_list(Vec::new(), cx),
            },
            Menu::Setting(index) => match settings.get(index) {
                Some(setting) => Self::render_choices(menu, setting, cx),
                None => Self::render_list(Vec::new(), cx),
            },
        });
        Some(
            div()
                .flex()
                .flex_col()
                .child(pill)
                .children(list.map(|list| {
                    // Under the pill, kept in the window, which takes it
                    // left of the pill at the window's right edge.
                    deferred(
                        anchored()
                            .position_mode(AnchoredPositionMode::Local)
                            .position(point(px(0.), px(PILL_HEIGHT + 4.)))
                            .snap_to_window()
                            .child(list),
                    )
                    .with_priority(2)
                }))
                .into_any_element(),
        )
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

    /// The row of `setting`, which `menu` says which it is, naming it and
    /// what it is set to, which shows its choices. A row not `enabled`
    /// shows nothing.
    fn render_row(
        menu: Menu,
        setting: &Setting,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (id, selector) = match menu {
            Menu::Setting(index) => (
                ElementId::NamedInteger("setting".into(), index as u64),
                format!("setting-{index}"),
            ),
            Menu::Agent | Menu::Rows => (ElementId::Name("agent".into()), "agent".to_owned()),
        };
        div()
            .id(id)
            .debug_selector(move || selector.clone())
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_sm()
            .when(enabled, |row| {
                row.cursor_pointer()
                    .hover(|style| style.bg(rgb(ACCENT_GROUND)))
            })
            .when(!enabled, |row| row.opacity(0.5))
            .child(div().text_color(rgb(DIM)).child(setting.name.clone()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_right()
                    .child(chosen_name(setting).to_owned()),
            )
            .child(div().text_color(rgb(DIM)).child("›"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, window, cx| {
                    if enabled {
                        view.setting_open = Some(menu);
                    }
                    window.focus(&view.input.focus_handle(cx));
                    window.prevent_default();
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    /// The list under the pill, holding `rows`, which a click anywhere
    /// else closes.
    fn render_list(rows: Vec<AnyElement>, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        div()
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
            }))
            .children(rows)
    }

    /// The list of what `setting` can be set to, after a line back to
    /// the rows.
    fn render_choices(
        menu: Menu,
        setting: &Setting,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let back = div()
            .id("settings-back")
            .debug_selector(|| "settings-back".to_owned())
            .flex()
            .gap_2()
            .px_2()
            .py_1()
            .rounded_sm()
            .text_color(rgb(DIM))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(ACCENT_GROUND)))
            .child("‹")
            .child(setting.name.clone())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    view.setting_open = Some(Menu::Rows);
                    window.focus(&view.input.focus_handle(cx));
                    window.prevent_default();
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        let mut list = Self::render_list(vec![back.into_any_element()], cx);
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
                                Menu::Rows => {}
                            }
                            window.focus(&view.input.focus_handle(cx));
                        })),
                );
        }
        list
    }
}

/// The name of what `setting` is set to.
fn chosen_name(setting: &Setting) -> &str {
    setting
        .chosen()
        .map_or(setting.value.as_str(), |choice| choice.name.as_str())
}
