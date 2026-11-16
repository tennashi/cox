//! What waits for the human in the chats they are not looking at, in a
//! list that opens beside the rail, over the window, from the bell at
//! its head: each chat that asks them something, with the card it asks
//! on, which they answer there as in the chat, and then each chat whose
//! turn ended unseen, with the start of its last reply. The chat shown
//! and its stage stay as they are while they answer; a click on a chat's
//! name, or on Open, shows it.

use gpui::{
    AnyElement, App, Context, Focusable as _, Window, actions, anchored, deferred, div, point,
    prelude::*, px, rgb,
};

use super::{Chat, RAIL_WIDTH, Workspace};
use crate::chat::view::ChatView;
use crate::ui::ellipsis::ellipsis;
use crate::ui::tip;
use crate::vim::hints::Hinted as _;
use coxd::chat::conversation::{Entry, State};
use coxd::daemon::wire::Background;
use coxd::github::CheckState;
use coxd::theme::{ASKING, BUTTON, BUTTON_HOVER, DIM, FAINT, FOREGROUND, GAP};

actions!(
    waits,
    [
        /// Closes the list of what waits for the human.
        Close,
    ]
);

/// The key context of the list of what waits for the human.
pub(super) const WAITS: &str = "Waits";

/// How wide the list is.
const WIDTH: f32 = 420.;

/// How many lines of a chat's last reply the list shows, and how many
/// characters of them at most.
const REPLY_LINES: usize = 3;
const REPLY_CHARACTERS: usize = 240;

/// How a chat waits for the human, the more pressing first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wait {
    /// Its agent asks them something, and does nothing until they answer.
    Asks,
    /// A turn ended while it was not shown, and it has not been since.
    Unread,
    /// CI ended on a pull request its agent opened while it was not
    /// shown, and it has not been since.
    Checked,
}

impl Wait {
    /// How a chat in `state`, `unread` or not, and `checked` or not, as
    /// when CI ended unseen, waits for the human, if it does.
    fn of(state: State, unread: bool, checked: bool) -> Option<Self> {
        if state == State::Asking {
            Some(Self::Asks)
        } else if unread {
            Some(Self::Unread)
        } else if checked {
            Some(Self::Checked)
        } else {
            None
        }
    }
}

/// Those of `chats` that wait, each with how, the more pressing first,
/// and each kind in the order given.
fn in_order<T>(chats: impl IntoIterator<Item = (T, Option<Wait>)>) -> Vec<(T, Wait)> {
    let mut waiting: Vec<(T, Wait)> = chats
        .into_iter()
        .filter_map(|(chat, wait)| Some((chat, wait?)))
        .collect();
    waiting.sort_by_key(|&(_, wait)| wait);
    waiting
}

/// The start of `reply`: its first lines that say something, as many as
/// the list shows, cut short where they grow too long.
fn start_of(reply: &str) -> String {
    let lines: Vec<&str> = reply
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let mut start = lines
        .iter()
        .take(REPLY_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let mut cut = lines.len() > REPLY_LINES;
    if let Some((at, _)) = start.char_indices().nth(REPLY_CHARACTERS) {
        start.truncate(at);
        cut = true;
    }
    if cut {
        start.push('…');
    }
    start
}

impl Chat {
    /// How it waits for the human, if it does.
    fn wait(&self) -> Option<Wait> {
        Wait::of(self.state, self.unread, !self.ci_ended().is_empty())
    }

    /// How CI ended on each pull request its agent opened whose CI ended
    /// while it was not shown, as "CI passed on #423 Its title".
    fn ci_ended(&self) -> Vec<String> {
        self.pulls
            .iter()
            .filter_map(|row| {
                let outcome = match row.unseen? {
                    CheckState::Passed => "passed",
                    CheckState::Failed => "failed",
                    CheckState::Pending | CheckState::Skipped => "ended",
                };
                let title = row
                    .status
                    .as_ref()
                    .map_or("", |status| status.title.as_str());
                Some(format!("CI {outcome} on #{} {title}", row.pull.number))
            })
            .collect()
    }
}

impl Workspace {
    /// The chats that wait for the human, but the one shown, archived ones
    /// among them, each with how, those that ask first, each kind in the
    /// order of the list.
    #[must_use]
    pub fn waits(&self) -> Vec<(&Chat, Wait)> {
        let shown = self.shown().map(Chat::key);
        in_order(
            self.chats
                .iter()
                .chain(&self.archived)
                .map(|chat| (chat, chat.wait().filter(|_| Some(chat.key) != shown))),
        )
    }

    /// Whether the list of what waits for the human is open.
    #[must_use]
    pub fn waits_open(&self) -> bool {
        self.waits.is_some()
    }

    /// Opens the list of what waits for the human, with the keyboard in
    /// it, while anything waits.
    pub(super) fn open_waits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.waits.is_some() {
            return;
        }
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.waits = Some(focus);
        self.follow_waits(window, cx);
        cx.notify();
    }

    /// Whether the keyboard is in the list, while it is open.
    #[must_use]
    pub fn keys_in_waits(&self, window: &Window, cx: &App) -> bool {
        self.waits
            .as_ref()
            .is_some_and(|focus| focus.contains_focused(window, cx))
    }

    /// Whether the keyboard is in the input of a chat not shown, where a
    /// card answered in the list leaves it.
    fn keys_unseen(&self, window: &Window, cx: &App) -> bool {
        let shown = self.shown().map(Chat::key);
        self.chats.iter().chain(&self.archived).any(|chat| {
            Some(chat.key) != shown
                && chat
                    .view
                    .as_ref()
                    .is_some_and(|view| view.read(cx).input().focus_handle(cx).is_focused(window))
        })
    }

    /// Closes the list, giving the keyboard back to the chat shown if the
    /// list had it, or a chat not shown has it.
    pub(super) fn close_waits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.waits.is_none() {
            return;
        }
        if self.keys_in_waits(window, cx) || self.keys_unseen(window, cx) {
            self.give_back_keys(window, cx);
        }
        self.waits = None;
        cx.notify();
    }

    /// Follows the chats, while the list is open: each that waits gets a
    /// view, for its card to be drawn, and the list closes once nothing
    /// waits.
    pub(super) fn follow_waits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.waits.is_none() {
            return;
        }
        let keys: Vec<u64> = self.waits().iter().map(|(chat, _)| chat.key).collect();
        if keys.is_empty() {
            self.close_waits(window, cx);
            return;
        }
        for key in keys {
            self.view(key, cx);
        }
        if self.keys_unseen(window, cx)
            && let Some(focus) = &self.waits
        {
            window.focus(focus);
        }
    }

    /// Shows the chat `key`, closing the list.
    fn open_waiting(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.close_waits(window, cx);
        self.reveal(key, window, cx);
    }

    /// The list of what waits for the human, beside the rail, while it is
    /// open.
    pub(super) fn render_waits(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let focus = self.waits.as_ref()?;
        let rows: Vec<AnyElement> = self
            .waits()
            .into_iter()
            .enumerate()
            .flat_map(|(at, (chat, wait))| {
                let parting = (at > 0).then(|| crate::ui::menu::parting().into_any_element());
                parting
                    .into_iter()
                    .chain([self.render_waiting(chat, wait, cx)])
            })
            .collect();
        let height = window.viewport_size().height - px(GAP * 2.);
        let list = crate::ui::menu::menu()
            .id("waits")
            .debug_selector(|| "waits".to_owned())
            .track_focus(focus)
            .key_context(WAITS)
            .on_action(cx.listener(|workspace, _: &Close, window, cx| {
                workspace.close_waits(window, cx);
            }))
            .on_mouse_down_out(cx.listener(|workspace, _, window, cx| {
                workspace.close_waits(window, cx);
            }))
            .w(px(WIDTH))
            .max_h(height)
            .overflow_y_scroll()
            .child(
                div()
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(FAINT))
                    .child("Waiting for you"),
            )
            .children(rows);
        Some(
            deferred(
                anchored()
                    .position(point(px(RAIL_WIDTH - 8.), px(GAP)))
                    .snap_to_window()
                    .child(list),
            )
            .with_priority(2),
        )
    }

    /// The row of `chat`, which waits as `wait` says: its name, which
    /// shows it, and its project's, over the card it asks on, or the
    /// start of its last reply and a button that shows it.
    fn render_waiting(&self, chat: &Chat, wait: Wait, cx: &mut Context<Self>) -> AnyElement {
        let key = chat.key;
        let project = self.projects[chat.project].name.clone();
        let name = div()
            .id(gpui::SharedString::from(format!("waiting-{key}")))
            .debug_selector(move || format!("waiting-{key}"))
            .min_w_0()
            .overflow_hidden()
            .text_size(px(13.))
            .text_color(rgb(FOREGROUND))
            .cursor_pointer()
            .hover(gpui::Styled::underline)
            .tooltip(tip("Show this chat"))
            .child(ellipsis(chat.name.clone()))
            .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                workspace.open_waiting(key, window, cx);
            }));
        let head = div().flex().items_center().gap_2().child(name).child(
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(rgb(if wait == Wait::Asks { ASKING } else { DIM }))
                .child(project),
        );
        let body = match wait {
            Wait::Asks => Self::render_asks(chat, cx),
            Wait::Unread | Wait::Checked => Self::render_unread(chat, wait, cx),
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px(px(8.))
            .py(px(6.))
            .child(head)
            .child(body)
            .into_any_element()
    }

    /// The card `chat` asks the human on, drawn by its view, which hears
    /// the answer as it does in the chat; or what the chat waits for, in
    /// words, until its view has the conversation.
    fn render_asks(chat: &Chat, cx: &mut Context<Self>) -> AnyElement {
        let card = chat
            .view
            .as_ref()
            .and_then(|view| view.update(cx, ChatView::render_asked));
        card.unwrap_or_else(|| {
            let waiting = chat
                .view
                .as_ref()
                .and_then(|view| view.read(cx).waiting())
                .unwrap_or_else(|| "It asks you something…".to_owned());
            div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(waiting)
                .into_any_element()
        })
    }

    /// The start of the last reply of `chat`, whose turn ended unseen, and
    /// what it runs in the background still, if anything, and a button
    /// that shows it.
    fn render_unread(chat: &Chat, wait: Wait, cx: &mut Context<Self>) -> AnyElement {
        let key = chat.key;
        let ci = chat
            .ci_ended()
            .into_iter()
            .enumerate()
            .map(move |(at, ended)| {
                div()
                    .debug_selector(move || format!("waiting-{key}-ci-{at}"))
                    .text_color(rgb(FOREGROUND))
                    .child(ended)
            });
        let reply = (wait == Wait::Unread).then(|| {
            chat.view
                .as_ref()
                .and_then(|view| {
                    view.read(cx)
                        .conversation()
                        .entries()
                        .iter()
                        .rev()
                        .find_map(|entry| match entry {
                            Entry::Reply(text) => Some(start_of(text)),
                            _ => None,
                        })
                })
                .unwrap_or_else(|| "Its turn ended.".to_owned())
        });
        let open = div()
            .id(gpui::SharedString::from(format!("waiting-{key}-open")))
            .debug_selector(move || format!("waiting-{key}-open"))
            .flex_shrink_0()
            .h(px(28.))
            .px(px(12.))
            .flex()
            .items_center()
            .rounded(px(9.))
            .bg(rgb(BUTTON))
            .hover(|style| style.bg(rgb(BUTTON_HOVER)))
            .text_size(px(13.))
            .cursor_pointer()
            .child("Open")
            .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                workspace.open_waiting(key, window, cx);
            }));
        let background = chat.in_background().then(|| {
            let what: Vec<String> = chat.background().iter().map(Background::labeled).collect();
            div()
                .debug_selector(move || format!("waiting-{key}-in-background"))
                .text_color(rgb(FAINT))
                .child(format!(
                    "Still at work in the background: {}",
                    what.join("; ")
                ))
        });
        div()
            .flex()
            .items_end()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(rgb(DIM))
                    .whitespace_normal()
                    .children(reply)
                    .children(ci)
                    .children(background),
            )
            .child(open)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_that_asks_waits_before_one_unread_then_one_whose_ci_ended_and_one_at_rest_does_not_wait()
     {
        assert_eq!(Wait::of(State::Asking, true, true), Some(Wait::Asks));
        assert_eq!(Wait::of(State::Asking, false, false), Some(Wait::Asks));
        assert_eq!(Wait::of(State::Idle, true, true), Some(Wait::Unread));
        assert_eq!(Wait::of(State::Idle, false, true), Some(Wait::Checked));
        assert_eq!(Wait::of(State::Idle, false, false), None);
        assert_eq!(Wait::of(State::Busy, false, false), None);
    }

    #[test]
    fn the_chats_that_ask_come_first_each_kind_in_the_list_s_order() {
        let chats = [
            ("a", Some(Wait::Unread)),
            ("b", None),
            ("c", Some(Wait::Asks)),
            ("d", Some(Wait::Unread)),
            ("e", Some(Wait::Asks)),
        ];
        assert_eq!(
            in_order(chats),
            [
                ("c", Wait::Asks),
                ("e", Wait::Asks),
                ("a", Wait::Unread),
                ("d", Wait::Unread),
            ]
        );
    }

    #[test]
    fn the_start_of_a_reply_is_its_first_lines_that_say_something() {
        assert_eq!(
            start_of("Done.\n\nThe tests pass."),
            "Done.\nThe tests pass."
        );
        assert_eq!(start_of("one\ntwo\nthree\nfour"), "one\ntwo\nthree…");
        let long = "x".repeat(REPLY_CHARACTERS + 10);
        assert_eq!(
            start_of(&long),
            format!("{}…", "x".repeat(REPLY_CHARACTERS))
        );
    }
}
