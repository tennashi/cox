//! Whose each tab of the stage is: the human's, as every tab they open
//! is, or a chat's, as every tab its agent opens is, which only that
//! chat's agent sees and works on. The human hands a tab of theirs to the
//! chat shown, for its agent to work on, and takes a chat's tab back,
//! from the menu of the right button on the tab, which also shows the
//! chat another chat's tab is of. A chat archived gives its tabs back.

use std::rc::Rc;

use gpui::{
    App, Context, EventEmitter, Pixels, Point, SharedString, anchored, deferred, div, prelude::*,
    px, rgb,
};

use super::Stage;
use super::tab::TabContent;
use crate::ui::theme::{ACCENT_GROUND, BORDER_STRONG, RAISED};

/// What names the chats the stage's tabs are of, by their keys, as the
/// tabs along the top of the window name them.
pub type Names = Rc<dyn Fn(u64, &App) -> Option<SharedString>>;

/// The human asks to show the chat whose key it holds, of a tab of it.
pub struct ShowChat(pub u64);

impl EventEmitter<ShowChat> for Stage {}

impl Stage {
    /// Names the chats of the tabs, from now on, with `names`.
    pub fn set_names(&mut self, names: Names) {
        self.names = Some(names);
    }

    /// The name of the chat `chat`, or else a name that says which.
    fn name_of(&self, chat: u64, cx: &App) -> SharedString {
        self.names
            .as_ref()
            .and_then(|names| names(chat, cx))
            .unwrap_or_else(|| "this chat".into())
    }

    /// Hands the tab at `index`, the human's, to the chat `chat`: from now
    /// on, its agent sees the tab and works on it, as on those it opened.
    pub(super) fn hand(&mut self, index: usize, chat: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().nth(index) else {
            return;
        };
        if tab.owner.is_some() {
            return;
        }
        tab.owner = Some(chat);
        // An agent that starts later is told of the chat's pages as it
        // starts.
        if let TabContent::Page { target, .. } = &tab.content
            && let Some(pages) = self.pages.get(&chat)
        {
            pages.hand(target.clone());
        }
        self.changed(cx);
    }

    /// Takes the tab at `index` back from the chat it is of: from now on,
    /// it is the human's, and its agent sees it no longer.
    pub(super) fn take_back(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().nth(index) else {
            return;
        };
        let Some(chat) = tab.owner.take() else {
            return;
        };
        if let TabContent::Page { target, .. } = &tab.content
            && let Some(page) = target.borrow().clone()
            && let Some(pages) = self.pages.get(&chat)
        {
            pages.forget(&page);
        }
        self.changed(cx);
    }

    /// The chat `chat` was archived: its tabs are the human's from now
    /// on.
    pub fn release(&mut self, chat: u64, cx: &mut Context<Self>) {
        let of_chat: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.owner == Some(chat))
            .map(|(index, _)| index)
            .collect();
        for index in of_chat {
            self.take_back(index, cx);
        }
    }

    /// Opens the menu of the right button on the tab at `index`, `at`
    /// where the mouse went down.
    pub(super) fn open_tab_menu(
        &mut self,
        index: usize,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.tab_menu = Some((at, index));
        cx.notify();
    }

    fn close_tab_menu(&mut self, cx: &mut Context<Self>) {
        self.tab_menu = None;
        cx.notify();
    }

    /// The menu of the right button on a tab, if it is open: the human's
    /// tab is handed to the chat shown, a chat's taken back, and another
    /// chat's shows that chat; and any tab but the last closes.
    pub(super) fn render_tab_menu(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let (at, index) = self.tab_menu?;
        let tab = self.tabs.get(index)?;
        let item = |id: &'static str, label: SharedString| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(ACCENT_GROUND)))
                .child(label)
        };
        let hand_to = tab
            .owner
            .is_none()
            .then_some(self.showing)
            .flatten()
            .map(|chat| (chat, self.name_of(chat, cx)));
        let of_other = tab
            .owner
            .filter(|&owner| Some(owner) != self.showing)
            .map(|chat| (chat, self.name_of(chat, cx)));
        let taken = tab.owner.is_some();
        let closes = self.tabs.len() > 1;
        let menu = div()
            .occlude()
            .min_w(px(160.))
            .p_1()
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(BORDER_STRONG))
            .rounded_md()
            .shadow_md()
            .text_size(px(13.))
            .on_mouse_down_out(cx.listener(|stage, _, _, cx| stage.close_tab_menu(cx)))
            .children(hand_to.map(|(chat, name)| {
                item("hand-tab", format!("Hand to {name}").into()).on_click(cx.listener(
                    move |stage, _, _, cx| {
                        stage.close_tab_menu(cx);
                        stage.hand(index, chat, cx);
                    },
                ))
            }))
            .when(taken, |menu| {
                menu.child(
                    item("take-tab-back", "Take back".into()).on_click(cx.listener(
                        move |stage, _, _, cx| {
                            stage.close_tab_menu(cx);
                            stage.take_back(index, cx);
                        },
                    )),
                )
            })
            .children(of_other.map(|(chat, name)| {
                item("show-tab-chat", format!("Show {name}").into()).on_click(cx.listener(
                    move |stage, _, _, cx| {
                        stage.close_tab_menu(cx);
                        cx.emit(ShowChat(chat));
                    },
                ))
            }))
            .when(closes, |menu| {
                menu.child(item("close-tab", "Close tab".into()).on_click(cx.listener(
                    move |stage, _, window, cx| {
                        stage.close_tab_menu(cx);
                        let _ = stage.close_tab(index, window, cx);
                    },
                )))
            });
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}
