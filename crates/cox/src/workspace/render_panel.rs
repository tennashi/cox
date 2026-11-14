//! The panel of the chat shown: its head, the list of the project's
//! chats, the panel folded to the window's edge, and the menu of a chat.

use gpui::{
    AnyElement, ClickEvent, Context, Div, Focusable, FontWeight, MouseButton, MouseDownEvent,
    Stateful, Window, anchored, deferred, div, prelude::*, px, rgb,
};

use super::render::keys_edge;
use super::{
    CancelRename, Chat, FOLDED_WIDTH, LIST, PANEL_WIDTH, RENAMING, ROW_HEIGHT, ROW_RADIUS,
    Workspace,
};
use crate::chat::conversation::State;
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::{card, tip};
use crate::vim::hints::{self, Hinted as _, Part};
use coxd::theme::{
    ACCENT, ASKING, BORDER, BORDER_STRONG, DIM, DISABLED, FAINT, FOREGROUND, GAP, GROUND, HOVER,
    LINK, RAISED, SELECTED, chat_color,
};

impl Workspace {
    /// The panel of the chat shown, at the window's right: at its head,
    /// the button that opens the list of the project's chats, naming the
    /// chat shown, and the button that folds the panel; under it, the pill
    /// of the agent and the settings of the chat; then its conversation,
    /// under the list while it is open.
    pub(super) fn render_panel(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.shown().and_then(Chat::view).cloned();
        let settings = view
            .as_ref()
            .and_then(|view| view.update(cx, |view, cx| view.render_settings(cx)));
        let key = if cfg!(target_os = "macos") {
            "⌘\\"
        } else {
            "Ctrl+\\"
        };
        let body = match view {
            Some(view) => view.into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p(px(GAP * 2.))
                .text_color(rgb(FAINT))
                .flex_col()
                .gap(px(GAP))
                .child(if self.chats(self.chosen).next().is_some() {
                    "Choose a chat from the list above."
                } else {
                    "No chat here yet: the list above opens one."
                })
                .into_any_element(),
        };
        card()
            .debug_selector(|| "chat-panel".to_owned())
            .track_focus(&self.chat_card)
            .relative()
            .child(hints::part(Part::Chat))
            .w(px(PANEL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .p(px(6.))
                    .child(self.render_switcher(cx))
                    .children(self.render_goal_button(cx))
                    .child(
                        div()
                            .id("fold-chat")
                            .debug_selector(|| "fold-chat".to_owned())
                            .flex_shrink_0()
                            .size(px(ROW_HEIGHT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(ROW_RADIUS))
                            .text_color(rgb(FAINT))
                            .text_size(px(16.))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                            .tooltip(tip(format!("Fold the chat ({key})")))
                            .child("›")
                            .on_hint_click(cx.listener(|workspace, _, window, cx| {
                                workspace.fold(true, window, cx);
                            })),
                    ),
            )
            .children(settings.map(|settings| {
                div()
                    .flex_shrink_0()
                    .flex()
                    .px(px(GAP))
                    .pb(px(6.))
                    .child(settings)
            }))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(body)
                    .children(self.switching.then(|| self.render_list(window, cx))),
            )
            .children(keys_edge(&self.chat_card, window, cx))
    }

    /// The button at the head of the panel that sets the goal of the chat
    /// shown, as "Set goal…" in its menu does, so that what the work is to
    /// come to is a click away, as on the phone: in the accent while the
    /// chat has a goal.
    fn render_goal_button(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let index = self.chats.shown_index()?;
        let has_goal = self.chats.get(index)?.goal().is_some();
        Some(
            div()
                .id("goal-button")
                .debug_selector(|| "goal-button".to_owned())
                .flex_shrink_0()
                .size(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(ROW_RADIUS))
                .text_color(rgb(if has_goal { ACCENT } else { FAINT }))
                .text_size(px(15.))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                .tooltip(tip(if has_goal {
                    "Change the goal"
                } else {
                    "Set goal…"
                }))
                .child("◎")
                .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                    workspace.edit_goal(index, window, cx);
                })),
        )
    }

    /// The button at the head of the panel that opens the list of the
    /// project's chats, or closes it: the chat shown, as its row in the
    /// list names it, or the input that renames it, and how many of the
    /// project's other chats wait for the human. A double click renames
    /// the chat shown, and the right button opens its menu.
    fn render_switcher(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let shown = self.chats.shown_index();
        let shown_key = self.shown().map(Chat::key);
        let others: Vec<&Chat> = self
            .chats(self.chosen)
            .filter(|chat| Some(chat.key) != shown_key)
            .collect();
        let asking = others
            .iter()
            .filter(|chat| chat.state == State::Asking)
            .count();
        let waiting = others
            .iter()
            .filter(|chat| chat.unread || chat.state == State::Asking)
            .count();
        let switching = self.switching;
        div()
            .id("chat-switcher")
            .debug_selector(|| "chat-switcher".to_owned())
            .flex_1()
            .min_w_0()
            .h(px(ROW_HEIGHT))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(ROW_RADIUS))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .when(switching, |button| button.bg(rgb(SELECTED)))
            .hover(|style| style.bg(rgb(HOVER)))
            .on_hint_click(cx.listener(
                move |workspace, event: &ClickEvent, window, cx| match shown {
                    Some(index) if event.click_count() == 2 => {
                        workspace.rename(index, window, cx);
                    }
                    _ => {
                        workspace.switching = !workspace.switching;
                        // The keyboard stays in the chat's input.
                        window.focus(&workspace.focus_handle(cx));
                        cx.notify();
                    }
                },
            ))
            .when_some(shown, |button, index| {
                button.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                        workspace.menu = Some((event.position, index));
                        cx.notify();
                    }),
                )
            })
            .map(
                |button| match shown.and_then(|index| Some((index, self.chats.get(index)?))) {
                    Some((index, chat)) => {
                        let name = self.render_name(index, chat, true, cx);
                        button.children(self.render_marks(chat, "shown", name))
                    }
                    None => button.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(FAINT))
                            .child("No chat"),
                    ),
                },
            )
            .child(div().flex_1())
            .when(waiting > 0, |button| {
                button.child(
                    div()
                        .debug_selector(|| "chats-waiting".to_owned())
                        .flex_shrink_0()
                        .px(px(7.))
                        .h(px(18.))
                        .flex()
                        .items_center()
                        .rounded(px(9.))
                        .bg(rgb(if asking > 0 { ASKING } else { ACCENT }))
                        .text_color(rgb(GROUND))
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .child(format!("{waiting} waiting")),
                )
            })
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(FAINT))
                    .child(if switching { "▴" } else { "▾" }),
            )
    }

    /// The list of the chats of the project chosen, over the conversation
    /// while it is open, with the row that opens another chat in the
    /// project, and "Archived" at its foot, if the human archived a chat.
    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let chosen = self.chosen;
        let folder = self.projects[chosen].folder.clone();
        let chats = self
            .chats
            .iter()
            .enumerate()
            .filter(|(_, chat)| chat.project == chosen);
        let picked = self.list.picked(chats.clone().count(), window);
        let rows = chats
            .enumerate()
            .map(|(row, (index, chat))| {
                self.render_row(index, chat, cx)
                    .when(picked == Some(row), |row| {
                        row.relative().child(crate::ui::picked(ROW_RADIUS))
                    })
            })
            .collect::<Vec<_>>();
        div()
            .id("chats")
            .when(crate::vim::mode(cx).is_some(), |list| {
                list.track_focus(&self.list.focus).key_context(LIST)
            })
            .debug_selector(|| "chats".to_owned())
            .child(hints::cover())
            .child(hints::part(Part::List))
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .max_h_full()
            .overflow_y_scroll()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .bg(rgb(RAISED))
            .border_b_1()
            .border_color(rgb(BORDER_STRONG))
            .shadow_lg()
            .children(rows)
            .child(
                div()
                    .id("new-chat-in")
                    .debug_selector(move || format!("new-chat-in-{}", chosen + 1))
                    .flex_shrink_0()
                    .h(px(ROW_HEIGHT))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(ROW_RADIUS))
                    .text_size(px(13.))
                    .text_color(rgb(FAINT))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                    .child(icon(Icon::Plus, 12., FAINT))
                    .child("New chat")
                    .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.open(Some(folder.clone()), window, cx);
                    })),
            )
            .children(self.render_archived(cx))
    }

    /// The row of the chat at `index` in the list, which numbers its chats
    /// from 1, marked when it is the chat shown: a click shows it, and the
    /// right button opens its menu.
    fn render_row(&self, index: usize, chat: &Chat, cx: &mut Context<Self>) -> Stateful<Div> {
        let number = index + 1;
        let shown = self.chats.shown_index() == Some(index);
        let name = self.render_name(index, chat, false, cx);
        div()
            .id(("chat", number))
            .debug_selector(move || format!("chat-{number}"))
            .flex_shrink_0()
            .h(px(ROW_HEIGHT))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(ROW_RADIUS))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .map(|row| {
                if shown {
                    row.bg(rgb(SELECTED)).text_color(rgb(FOREGROUND))
                } else {
                    row.text_color(rgb(DIM)).hover(|style| style.bg(rgb(HOVER)))
                }
            })
            .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                workspace.show(index, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                    workspace.menu = Some((event.position, index));
                    cx.notify();
                }),
            )
            .children(self.render_marks(chat, &format!("chat-{number}"), name))
    }

    /// What tells of `chat`, on its row or at the head of the panel: a dot
    /// of how it stands, whether an agent opened it, its `name`, and
    /// whether it is a sketch and has a goal, each named after `at`.
    fn render_marks(&self, chat: &Chat, at: &str, name: Div) -> Vec<AnyElement> {
        // A chat an agent opened comes after the chat that opened it.
        let opened = chat.parent.is_some() && self.depth(chat) > 0;
        let small = |text: &'static str, selector: String| {
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .debug_selector(move || selector.clone())
                .child(text)
                .into_any_element()
        };
        let mut marks = Vec::new();
        if opened {
            marks.push(small("↳", format!("{at}-opened")));
        }
        if let Some((color, filled, mark)) = dot(chat) {
            let selector = format!("{at}-{mark}");
            let circle = div()
                .debug_selector(move || selector.clone())
                .flex_shrink_0()
                .size(px(8.))
                .rounded_full();
            marks.push(
                if filled {
                    circle.bg(rgb(color))
                } else {
                    circle.border_1().border_color(rgb(color))
                }
                .into_any_element(),
            );
        }
        marks.push(name.into_any_element());
        if chat.is_sketch() {
            marks.push(small("sketch", format!("{at}-sketch")));
        }
        if chat.goal.is_some() {
            marks.push(small("◎", format!("{at}-goal")));
        }
        marks
    }

    /// The panel folded to the window's right edge: the button that
    /// unfolds it, and a dot for each chat of the project chosen, which
    /// shows the chat.
    pub(super) fn render_folded(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = self.chosen;
        let dots = self
            .chats
            .iter()
            .enumerate()
            .filter(|(_, chat)| chat.project == chosen)
            .map(|(index, chat)| {
                let number = index + 1;
                let shown = self.chats.shown_index() == Some(index);
                let (color, filled) =
                    dot(chat).map_or((BORDER_STRONG, false), |(color, filled, _)| (color, filled));
                let circle = div().size(px(10.)).rounded_full();
                div()
                    .id(("chat-dot", number))
                    .debug_selector(move || format!("chat-dot-{number}"))
                    .flex_shrink_0()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(9.))
                    .cursor_pointer()
                    .when(shown, |button| button.bg(rgb(SELECTED)))
                    .hover(|style| style.bg(rgb(HOVER)))
                    .tooltip(tip(chat.name.clone()))
                    .child(if filled {
                        circle.bg(rgb(color))
                    } else {
                        circle.border_1().border_color(rgb(color))
                    })
                    .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.show(index, window, cx);
                    }))
            })
            .collect::<Vec<_>>();
        let key = if cfg!(target_os = "macos") {
            "⌘\\"
        } else {
            "Ctrl+\\"
        };
        card()
            .id("folded-chat")
            .debug_selector(|| "folded-chat".to_owned())
            .relative()
            .child(hints::part(Part::Chat))
            .w(px(FOLDED_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .py(px(6.))
            .overflow_y_scroll()
            .child(
                div()
                    .id("unfold-chat")
                    .debug_selector(|| "unfold-chat".to_owned())
                    .flex_shrink_0()
                    .size(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ROW_RADIUS))
                    .text_color(rgb(FAINT))
                    .text_size(px(16.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                    .tooltip(tip(format!("Show the chat ({key})")))
                    .child("‹")
                    .on_hint_click(cx.listener(|workspace, _, window, cx| {
                        workspace.fold(false, window, cx);
                    })),
            )
            .children(dots)
    }
    /// The name of the chat at `index`, or, at the head of the panel, as
    /// `head` says, the input that renames it.
    fn render_name(&self, index: usize, chat: &Chat, head: bool, cx: &mut Context<Self>) -> Div {
        match &self.renaming {
            Some((renaming, input, _)) if head && *renaming == index => div()
                .key_context(RENAMING)
                .debug_selector(|| "renaming".to_owned())
                .flex_1()
                .min_w_0()
                .px_1()
                .border_1()
                .border_color(rgb(LINK))
                .rounded_sm()
                // A click in the input stays there, not opening the list.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_action(cx.listener(|workspace, _: &CancelRename, window, cx| {
                    workspace.end_renaming(false, window, cx);
                }))
                .on_mouse_down_out(cx.listener(|workspace, _, window, cx| {
                    workspace.end_renaming(true, window, cx);
                }))
                .child(input.clone()),
            _ => div()
                .min_w_0()
                .overflow_hidden()
                // A closed chat is there to read.
                .when(!chat.open, |name| name.text_color(rgb(FAINT)))
                .child(ellipsis(chat.name.clone())),
        }
    }

    /// "Archived" at the foot of the list of chats, if the human archived
    /// a chat, and under it, while it is open, the archived chats: each
    /// with its project's name, brought back by a click.
    fn render_archived(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.archived.is_empty() {
            return None;
        }
        let open = self.archived_open;
        let button = div()
            .id("archived")
            .debug_selector(|| "archived".to_owned())
            .flex_shrink_0()
            .h(px(ROW_HEIGHT))
            .px(px(12.))
            .flex()
            .items_center()
            .gap_1()
            .rounded(px(ROW_RADIUS))
            .text_size(px(13.))
            .text_color(rgb(FAINT))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
            .child(format!("Archived ({})", self.archived.len()))
            .child(if open { "▴" } else { "▾" })
            .on_hint_click(cx.listener(|workspace, _, window, cx| {
                workspace.archived_open = !workspace.archived_open;
                window.focus(&workspace.focus_handle(cx));
                cx.notify();
            }));
        let rows = self
            .archived
            .iter()
            .enumerate()
            .filter(|_| open)
            .map(|(index, chat)| {
                let number = index + 1;
                div()
                    .id(("archived-chat", number))
                    .debug_selector(move || format!("archived-{number}"))
                    .flex_shrink_0()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(24.))
                    .pr(px(12.))
                    .rounded(px(ROW_RADIUS))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .text_color(rgb(DIM))
                    .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(ellipsis(chat.name.clone())),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(rgb(FAINT))
                            .child(self.projects[chat.project].name.clone()),
                    )
                    .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.archived_open = false;
                        workspace.unarchive(index, window, cx);
                    }))
            })
            .collect::<Vec<_>>();
        Some(
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .mt(px(4.))
                .pt(px(4.))
                .border_t_1()
                .border_color(rgb(BORDER))
                .child(button)
                .children(rows),
        )
    }

    /// The menu of the right button on a chat, if it is open.
    pub(super) fn render_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (at, index) = self.menu?;
        let has_goal = self
            .chats
            .get(index)
            .is_some_and(|chat| chat.goal.is_some());
        let endings = self.endings(index);
        let starts_sketch = self.starts_sketch(index);
        let saves_to_wiki = self.saves_to_wiki(index);
        let merges = self.merges(index);
        let item = |id: &'static str, label: &'static str| crate::ui::menu::item(id, label, "");
        let menu = crate::ui::menu::menu()
            .min_w(px(160.))
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| {
                workspace.menu = None;
                cx.notify();
            }))
            .child(item("rename-chat", "Rename").on_hint_click(
                cx.listener(move |workspace, _, window, cx| workspace.rename(index, window, cx)),
            ))
            .child(item("set-goal", "Set goal…").on_hint_click(cx.listener(
                move |workspace, _, window, cx| {
                    workspace.edit_goal(index, window, cx);
                },
            )))
            .when(has_goal, |menu| {
                menu.child(item("clear-goal", "Clear goal").on_hint_click(cx.listener(
                    move |workspace, _, window, cx| workspace.clear_goal(index, window, cx),
                )))
            })
            .when(starts_sketch, |menu| {
                menu.child(
                    item("start-sketch", "Start sketch…").on_hint_click(cx.listener(
                        move |workspace, _, window, cx| workspace.edit_sketch(index, window, cx),
                    )),
                )
            })
            .when(saves_to_wiki, |menu| {
                menu.child(
                    item("save-to-wiki", "Save to wiki").on_hint_click(cx.listener(
                        move |workspace, _, window, cx| workspace.save_to_wiki(index, window, cx),
                    )),
                )
            })
            .when(merges, |menu| {
                menu.child(item("merge", "Merge").on_hint_click(
                    cx.listener(move |workspace, _, window, cx| workspace.merge(index, window, cx)),
                ))
            })
            .children(endings.into_iter().enumerate().map(|(at, ending)| {
                crate::ui::menu::item(("sketch-ending", at), ending.name.clone(), "").on_hint_click(
                    cx.listener(move |workspace, _, window, cx| {
                        workspace.end_sketch_as(index, ending.clone(), window, cx);
                    }),
                )
            }))
            .child(item("archive-chat", "Archive").on_hint_click(
                cx.listener(move |workspace, _, window, cx| workspace.archive(index, window, cx)),
            ));
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}

/// The dot of how `chat` stands, its color, whether it is filled, and
/// what it is called: filled, it asks the human to look; a ring, it says
/// only whether the agent is at work. A closed chat has none. Its agent's
/// dots are in the chat's color, as the tabs it works on are, so that the
/// human tells which tabs are whose.
fn dot(chat: &Chat) -> Option<(u32, bool, &'static str)> {
    let color = chat_color(chat.key);
    match chat.open.then_some(chat.state) {
        Some(State::Asking) => Some((ASKING, true, "asking")),
        _ if chat.unread => Some((color, true, "unread")),
        Some(State::Busy | State::Stopping | State::Checking) => Some((color, false, "working")),
        Some(State::Idle) => Some((DISABLED, false, "idle")),
        Some(State::Closed) | None => None,
    }
}
