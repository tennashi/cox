//! The stage: where the human and the agent of one chat work together,
//! beside the chat's conversation. It holds tabs, each showing a page of
//! the browser (see [`crate::page`]), or a Markdown file (see
//! [`crate::document`]). The first shows the page the stage starts on; the
//! human opens more pages and closes tabs, and so does the agent, through
//! the chat's relay. A page that asks for a window of its own gets a tab
//! instead. The agent shows the human Markdown files with Cox's own tools
//! (see [`crate::mcp`]), a tab for each file.
//!
//! The stage tells the relay of every page it opens, so that the agent sees
//! each of its chat's pages, whoever opened it. When the agent works on a
//! page, or shows a file, the stage shows that tab, and opens beside the
//! conversation if it was closed.

use std::path::PathBuf;

use gpui::{
    AnyView, App, Context, Div, Entity, FocusHandle, Focusable, Stateful, Subscription, Task,
    Window, div, prelude::*, px, rgb,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::browser::{Cef, TargetId};
use crate::document::{Document, LinkClicked};
use crate::input::LineInput;
use crate::mcp::Request;
use crate::page::{OpenTab, Page};
use crate::relay::{FromAgent, Pages, Relay};
use crate::view::Panel;

/// What a new tab shows when the human opens it.
const BLANK: &str = "about:blank";

/// How wide a tab grows, at most.
const TAB_WIDTH: f32 = 200.;

/// The most of a tab's title that its tab shows.
const TITLE_LENGTH: usize = 40;

pub struct Stage {
    cef: Cef,
    tabs: Vec<Tab>,
    /// The index of the tab shown. There is always one: closing the last
    /// tab is refused.
    shown: usize,
    /// The chat's pages, as the relay knows them, if the relay started.
    pages: Option<Pages>,
    /// Whether the stage is open beside the conversation.
    panel: Entity<Panel>,
    _from_agent: Option<Task<()>>,
    _from_tools: Option<Task<()>>,
}

/// A tab, and what it shows.
struct Tab {
    shown: Shown,
    /// Redraws the tab as the title of what it shows changes.
    _changes: Subscription,
    /// Opens the tabs that what it shows asks for.
    _opens: Subscription,
}

enum Shown {
    Page {
        page: Entity<Page>,
        /// The page's id, which the relay names it by.
        target: TargetId,
    },
    Document(Entity<Document>),
}

impl Shown {
    fn page(&self) -> Option<&Entity<Page>> {
        match self {
            Self::Page { page, .. } => Some(page),
            Self::Document(_) => None,
        }
    }

    fn view(&self) -> AnyView {
        match self {
            Self::Page { page, .. } => page.clone().into(),
            Self::Document(document) => document.clone().into(),
        }
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Page { page, .. } => page.focus_handle(cx),
            Self::Document(document) => document.focus_handle(cx),
        }
    }
}

impl Stage {
    /// A stage of one tab showing `url`, which tells `relay` of the pages
    /// it opens, carries out what the agent's tools ask in `from_tools`,
    /// and opens `panel` when the agent works on it.
    pub fn new(
        cef: Cef,
        url: &str,
        relay: Option<Relay>,
        from_tools: Option<UnboundedReceiver<Request>>,
        panel: Entity<Panel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (pages, from_agent) = match relay {
            Some(relay) => (Some(relay.pages), Some(relay.from_agent)),
            None => (None, None),
        };
        let from_agent = from_agent.map(|from_agent| Self::hear(from_agent, window, cx));
        let from_tools = from_tools.map(|from_tools| Self::hear_tools(from_tools, window, cx));
        let mut stage = Self {
            cef,
            tabs: Vec::new(),
            shown: 0,
            pages,
            panel,
            _from_agent: from_agent,
            _from_tools: from_tools,
        };
        stage.open_tab(url, true, window, cx);
        stage
    }

    /// Carries out what the relay tells the stage for the agent.
    fn hear(
        mut from_agent: UnboundedReceiver<FromAgent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(heard) = from_agent.recv().await {
                let done = this.update_in(cx, |stage, window, cx| match heard {
                    FromAgent::Worked { page, input } => stage.worked(page, input, cx),
                    FromAgent::Open {
                        url,
                        background,
                        answer,
                    } => {
                        let page = stage.open_tab(&url, !background, window, cx);
                        if !background {
                            stage.show_panel(cx);
                        }
                        let _ = answer.send(Some(page));
                    }
                    FromAgent::Close { page, answer } => {
                        let closed = match stage.index_of(&page) {
                            Some(index) => stage.close_tab(index, window, cx),
                            None => Err("No target with given id found"),
                        };
                        let _ = answer.send(closed);
                    }
                });
                if done.is_err() {
                    break;
                }
            }
        })
    }

    /// Carries out what the agent's tools ask.
    fn hear_tools(
        mut from_tools: UnboundedReceiver<Request>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(request) = from_tools.recv().await {
                let done = this.update_in(cx, |stage, window, cx| match request {
                    Request::ShowMarkdown { path, text } => {
                        stage.show_document(path, text, window, cx);
                    }
                });
                if done.is_err() {
                    break;
                }
            }
        })
    }

    /// Shows the file at `path`, which says `text`: on the tab that shows
    /// it already, or a new one. The stage opens if it was closed.
    fn show_document(
        &mut self,
        path: PathBuf,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let showing = self.tabs.iter().position(|tab| match &tab.shown {
            Shown::Document(document) => document.read(cx).path() == path,
            Shown::Page { .. } => false,
        });
        if let Some(index) = showing {
            if let Shown::Document(document) = &self.tabs[index].shown {
                document.update(cx, |document, cx| document.show(text, cx));
            }
            self.shown = index;
            cx.notify();
        } else {
            let document = cx.new(|cx| Document::new(path, text, cx));
            let changes = cx.observe(&document, |_, _, cx| cx.notify());
            let opens = cx.subscribe_in(
                &document,
                window,
                |stage, _, LinkClicked(url), window, cx| stage.open(url, window, cx),
            );
            self.push(Shown::Document(document), changes, opens, true, cx);
        }
        self.show_panel(cx);
    }

    /// The agent worked on `page`, sending it `input` if any: show its tab,
    /// and the stage.
    fn worked(
        &mut self,
        page: Option<String>,
        input: Option<crate::relay::AgentInput>,
        cx: &mut Context<Self>,
    ) {
        let index = page
            .and_then(|page| self.index_of(&page))
            .unwrap_or(self.shown);
        if index != self.shown {
            self.shown = index;
            cx.notify();
        }
        self.show_panel(cx);
        if let Some(input) = input
            && let Some(page) = self.tabs[index].shown.page()
        {
            page.update(cx, |page, cx| page.see(input, cx));
        }
    }

    fn show_panel(&self, cx: &mut Context<Self>) {
        self.panel.update(cx, Panel::open);
    }

    /// The index of the tab showing `page`.
    fn index_of(&self, page: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| match &tab.shown {
            Shown::Page { target, .. } => target.borrow().as_deref() == Some(page),
            Shown::Document(_) => false,
        })
    }

    /// Opens a tab showing `url` after the others, shown if `show` is, and
    /// returns its page's id.
    pub fn open_tab(
        &mut self,
        url: &str,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TargetId {
        // The relay hears of the page before CEF makes it.
        let opening = self.pages.as_ref().map(Pages::opening);
        let (browser, events) = self.cef.open(url);
        let target = browser.target_id();
        if let Some(opening) = opening {
            opening.is(target.clone());
        }
        let page = cx.new(|cx| Page::new(browser, events, window, cx));
        let changes = cx.observe(&page, |_, _, cx| cx.notify());
        let popups = cx.subscribe_in(&page, window, |stage, _, OpenTab(url), window, cx| {
            stage.open_tab(url, true, window, cx);
        });
        let shown = Shown::Page {
            page,
            target: target.clone(),
        };
        self.push(shown, changes, popups, show, cx);
        target
    }

    /// Puts a tab after the others, shown if `show` is.
    fn push(
        &mut self,
        shown: Shown,
        changes: Subscription,
        opens: Subscription,
        show: bool,
        cx: &mut Context<Self>,
    ) {
        self.tabs.push(Tab {
            shown,
            _changes: changes,
            _opens: opens,
        });
        if show {
            self.shown = self.tabs.len() - 1;
        }
        cx.notify();
    }

    /// Closes the tab at `index`, which closes its page, and shows the one
    /// after it, or before it if it was the last. The last tab stays.
    ///
    /// # Errors
    ///
    /// When it is the last tab.
    pub fn close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        if self.tabs.len() <= 1 {
            return Err("The stage keeps its last tab open");
        }
        let had_focus = self.tabs[self.shown]
            .shown
            .focus_handle(cx)
            .contains_focused(window, cx);
        self.tabs.remove(index);
        self.shown = after_closing(self.shown, index, self.tabs.len());
        if had_focus {
            self.focus_shown(window, cx);
        }
        cx.notify();
        Ok(())
    }

    /// Shows the tab at `index`, with the keyboard in its page.
    fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.shown = index;
            self.focus_shown(window, cx);
            cx.notify();
        }
    }

    fn focus_shown(&self, window: &mut Window, cx: &App) {
        window.focus(&self.tabs[self.shown].shown.focus_handle(cx));
    }

    /// Opens `url`, a link the human clicked: on the tab that shows it
    /// already, or a new one.
    pub fn open(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let showing = self.tabs.iter().position(|tab| {
            tab.shown
                .page()
                .is_some_and(|page| page.read(cx).navigation().url == url)
        });
        if let Some(index) = showing {
            self.show(index, window, cx);
        } else {
            self.open_tab(url, true, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// A tab the human opens: blank, with the keyboard in its address line,
    /// where what they type replaces the address.
    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(BLANK, true, window, cx);
        if let Some(page) = self.tabs[self.shown].shown.page() {
            let address = page.read(cx).address().clone();
            window.focus(&address.focus_handle(cx));
            address.update(cx, LineInput::select_whole);
        }
    }

    fn render_tab(&self, index: usize, tab: &Tab, cx: &mut Context<Self>) -> Stateful<Div> {
        let number = index + 1;
        let title = match &tab.shown {
            Shown::Page { page, .. } => {
                let navigation = page.read(cx).navigation();
                if navigation.title.is_empty() {
                    navigation.url.clone()
                } else {
                    navigation.title.clone()
                }
            }
            Shown::Document(document) => document.read(cx).title(),
        };
        let title: String = title.chars().take(TITLE_LENGTH).collect();
        let mut tab = div()
            .id(("tab", number))
            .debug_selector(move || format!("tab-{number}"))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .max_w(px(TAB_WIDTH))
            .pl_2()
            .pr_1()
            .rounded_t_sm()
            .cursor_pointer()
            .on_click(cx.listener(move |stage, _, window, cx| stage.show(index, window, cx)))
            .child(div().flex_1().min_w_0().truncate().child(title));
        if self.tabs.len() > 1 {
            tab = tab.child(
                div()
                    .id(("close-tab", number))
                    .debug_selector(move || format!("close-tab-{number}"))
                    .px_1()
                    .rounded_sm()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x40_4040)).text_color(rgb(0xe0_e0e0)))
                    .child("×")
                    .on_click(cx.listener(move |stage, _, window, cx| {
                        cx.stop_propagation();
                        let _ = stage.close_tab(index, window, cx);
                    })),
            );
        }
        if index == self.shown {
            tab.bg(rgb(0x2a_2a2a))
        } else {
            tab.text_color(rgb(0xa0_a0a0))
                .hover(|style| style.bg(rgb(0x26_2626)))
        }
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| self.render_tab(index, tab, cx))
            .collect();
        div()
            .flex()
            .items_end()
            .gap_1()
            .px_2()
            .pt_1()
            .border_b_1()
            .border_color(rgb(0x40_4040))
            .children(tabs)
            .child(
                div()
                    .id("new-tab")
                    .debug_selector(|| "new-tab".to_owned())
                    .flex_shrink_0()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x30_3030)).text_color(rgb(0xe0_e0e0)))
                    .child("+")
                    .on_click(cx.listener(|stage, _, window, cx| stage.new_tab(window, cx))),
            )
    }
}

/// Which tab is shown once the tab at `closed` has gone, when the one at
/// `shown` was, leaving `left` tabs: the same tab, unless it was the one
/// closed, when it is the one after it, or before it if it was the last.
fn after_closing(shown: usize, closed: usize, left: usize) -> usize {
    if shown > closed {
        shown - 1
    } else {
        shown.min(left.saturating_sub(1))
    }
}

impl Render for Stage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_tabs(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.tabs[self.shown].shown.view()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::after_closing;

    #[test]
    fn closing_another_tab_keeps_the_one_shown() {
        // Tabs 0, 1, 2, with 1 shown.
        assert_eq!(after_closing(1, 0, 2), 0);
        assert_eq!(after_closing(1, 2, 2), 1);
    }

    #[test]
    fn closing_the_tab_shown_shows_the_next_or_else_the_one_before() {
        assert_eq!(after_closing(1, 1, 2), 1);
        assert_eq!(after_closing(2, 2, 2), 1);
        assert_eq!(after_closing(0, 0, 2), 0);
    }
}
