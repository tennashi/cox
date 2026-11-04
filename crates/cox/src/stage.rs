//! The stage: where the human and the agent of one chat work together,
//! beside the chat's conversation. It holds tabs, each showing a page of
//! the browser (see [`crate::stage::page`]), a Markdown file (see
//! [`crate::stage::document`]), a file as it is written, such as code (see
//! [`crate::stage::code`]), the project's files as a tree (see [`crate::stage::tree`]),
//! what changed in them (see [`crate::stage::changes`]), or a terminal
//! (see [`crate::stage::terminal`]); and the pages of the stage's
//! plugins (see [`crate::stage::plugins`]), such as the map of the
//! project (see [`crate::stage::map`]), are pages of the browser. The first shows the page the
//! stage starts on; the human opens more pages and terminals, and the
//! tree, and closes tabs, and the agent opens and closes pages
//! too, through the chat's relay. A page that asks for a window of its own
//! gets a tab instead, and a link in a page to a file opens the file on a
//! tab, as a link from the conversation does. With Cox's own tools (see [`crate::agent::mcp`]), the
//! agent shows the human Markdown files and code, a tab for each file, and
//! runs commands in terminals: a command gets the tab of the agent's last
//! one that ended, or a new one. A link to a file opens it on a tab too:
//! as Markdown if it is a Markdown file and the link names no lines, and
//! as it is written otherwise. A click on a file in the tree opens it the
//! same way.
//!
//! The stage tells the relay of every page it opens, so that the agent sees
//! each of its chat's pages, whoever opened it. When the agent works on a
//! page, shows a file or runs a command, the stage shows that tab, and
//! opens beside the conversation if it was closed.
//!
//! An open chat's stage keeps its tabs as they change. A closed chat's
//! stage opens again on the pages and files kept, for the human to look
//! at; its terminals are not started again. Once the human sends to the
//! chat again, its agent works on those pages.
//!
//! - [`tab`] is a tab, and what it shows, whichever kind it is.
//! - [`links`] is what opens on a tab, and which: a link, a file, or the
//!   tree of the project's files.
//! - [`asked`] is what the agent asks of the stage, through the relay and
//!   its tools.
//! - [`page`] shows a page of the browser, and sends the human's input
//!   to it.
//! - [`document`] shows a Markdown file, and [`code`] a file as it is
//!   written, such as code: both [`follow`] the file as it changes.
//! - [`servers`] holds the language servers a tab of code asks what the
//!   code means, and [`outline`] asks them what a file defines, and
//!   [`calls`] what a line of it calls, for the map.
//! - [`tree`] shows the project's files, for the human to open one.
//! - [`changes`] shows what changed in them, for the human to read, and
//!   to open a file where it changed.
//! - [`terminal`] is a terminal the human types into, and the agent runs
//!   commands in.
//! - [`plugins`] are pages that draw what Cox knows, as a tab would, which
//!   the stage serves its pages itself at `cox:` addresses: [`map`] tells
//!   the project's map, as an agent wrote it, with what changed laid over
//!   its boxes, for the human to see where things are.

mod asked;
pub mod calls;
pub mod changes;
pub mod code;
pub mod document;
pub mod flow;
pub mod follow;
pub mod lines;
mod links;
pub mod map;
pub mod outline;
pub mod page;
pub mod plugins;
pub mod servers;
pub mod tab;
pub mod terminal;
pub mod tree;

use std::path::PathBuf;

use cox_core::list::List;
use cox_core::store::Record;
use cox_core::tabs::{KeptTab, KeptTabs};
use gpui::{
    App, Context, Div, Entity, Focusable, SharedString, Stateful, Subscription, Task, Window, div,
    prelude::*, px, rgb,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::agent::mcp::ToolRequest;
use cox_browser::relay::{Pages, Relay};
use cox_browser::{Cef, TargetId};

use crate::stage::calls::Calls;
use crate::stage::code::Code;
use crate::stage::document::Document;
use crate::stage::outline::Outlines;
use crate::stage::page::{Opens, Page};
use crate::stage::plugins::Plugins;
use crate::stage::tab::{Tab, TabContent};
use crate::stage::terminal::{Program, Running, Terminal};
use crate::ui::Panel;
use crate::ui::input::LineInput;

/// What a new tab shows when the human opens it.
const BLANK: &str = "about:blank";

/// How wide a tab grows, at most.
const TAB_WIDTH: f32 = 200.;

/// The most of a tab's title that its tab shows.
const TITLE_LENGTH: usize = 40;

pub struct Stage {
    cef: Cef,
    /// Where the chat works, which its terminals start in.
    cwd: PathBuf,
    /// The tabs, and which is shown. There is always one once the stage
    /// has started: closing the last tab is refused.
    tabs: List<Tab>,
    /// The chat's pages, as the relay knows them, if the relay started.
    pages: Option<Pages>,
    /// Whether the stage is open beside the conversation.
    panel: Entity<Panel>,
    /// Where the tabs are kept as they change, if they are.
    record: Option<Record>,
    /// The tabs as they were last kept.
    kept: Option<KeptTabs>,
    /// What carries out what the relay and the agent's tools ask, once
    /// the stage works with the agent.
    heard: Vec<Task<()>>,
    /// The plugins the stage shows as pages, and what they read of the
    /// chat.
    plugins: Plugins,
}

impl Stage {
    /// A stage of one tab showing `url`, for a chat working in `cwd`, which
    /// opens `panel` when the agent works on it. Until it [works](Self::work)
    /// with the chat's agent, nothing of it is kept.
    pub fn new(
        cef: Cef,
        cwd: PathBuf,
        url: &str,
        panel: Entity<Panel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut stage = Self::empty(cef, cwd, panel, cx);
        stage.open_tab(url, true, window, cx);
        stage
    }

    /// A stage with no tab yet, whose pages can ask what the project's
    /// files define (see [`outline`]) and call (see [`calls`]).
    fn empty(cef: Cef, cwd: PathBuf, panel: Entity<Panel>, cx: &mut Context<Self>) -> Self {
        let (outlines, asked) = Outlines::new();
        let answering = outline::answer(asked, outlines.clone(), cwd.clone(), cx);
        let (calls, asked_calls) = Calls::new();
        let answering_calls = calls::answer(asked_calls, calls.clone(), cwd.clone(), cx);
        Self {
            cef,
            plugins: Plugins::new(cwd.clone(), outlines, calls),
            cwd,
            tabs: List::new(),
            pages: None,
            panel,
            record: None,
            kept: None,
            heard: vec![answering, answering_calls],
        }
    }

    /// The chat's agent starts: from now on, the stage
    /// tells `relay` of its pages, those open and those it opens, carries
    /// out what the agent's tools ask in `from_tools`, and keeps its tabs
    /// in `record`.
    pub fn work(
        &mut self,
        relay: Option<Relay>,
        from_tools: Option<UnboundedReceiver<ToolRequest>>,
        record: Option<Record>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(relay) = relay {
            for tab in &self.tabs {
                if let TabContent::Page { target, .. } = &tab.content {
                    relay.pages.opening().is(target.clone());
                }
            }
            self.pages = Some(relay.pages);
            self.heard.push(Self::hear(relay.from_agent, window, cx));
        }
        if let Some(from_tools) = from_tools {
            self.heard.push(Self::hear_tools(from_tools, window, cx));
        }
        self.record = record;
        self.changed(cx);
    }

    /// The stage of a closed chat that worked in `cwd`, on the pages and
    /// files of `kept`, or of one tab showing `url` when none of them is
    /// left. Nothing of it is kept, until it [works](Self::work) with the
    /// chat's agent started again.
    pub fn reading(
        cef: Cef,
        cwd: PathBuf,
        kept: Option<KeptTabs>,
        url: &str,
        panel: Entity<Panel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut stage = Self::empty(cef, cwd, panel, cx);
        let KeptTabs { tabs, shown } = kept.unwrap_or_default();
        for (index, tab) in tabs.into_iter().enumerate() {
            // The tab shown is the one that was, or else the last before
            // it that opens again, or else the first after it.
            let show = index <= shown || stage.tabs.shown_index().is_none();
            match tab {
                KeptTab::Page { url, .. } => {
                    stage.open_tab(&url, show, window, cx);
                }
                KeptTab::Markdown { path } => {
                    // What the file says now; the tab says so if it cannot
                    // be read.
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    stage.push_file::<Document>(path, &text, None, show, window, cx);
                }
                KeptTab::File { path, lines } => {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    stage.push_file::<Code>(path, &text, lines, show, window, cx);
                }
                KeptTab::Files { open } => {
                    stage.push_tree(open, show, window, cx);
                }
                KeptTab::Changes { folder } => {
                    stage.push_changes(folder, show, window, cx);
                }
                KeptTab::Flow { call } => {
                    stage.push_flow(call, show, window, cx);
                }
                // The tabs Cox drew in earlier runs open as the pages that
                // draw them now: the map at its top, since its tiers are
                // not the crates the tab was open at.
                KeptTab::Map { .. } => {
                    stage.open_tab(links::MAP_URL, show, window, cx);
                }
                // The tab of the parts the chat ran with, which Cox no
                // longer draws, does not open again.
                KeptTab::Terminal { .. } | KeptTab::Parts => {}
            }
        }
        if stage.tabs.is_empty() {
            stage.open_tab(url, true, window, cx);
        }
        stage
    }

    /// The tabs, as they are kept.
    #[must_use]
    pub fn tabs(&self, cx: &App) -> KeptTabs {
        KeptTabs {
            tabs: self.tabs.iter().map(|tab| tab.content.kept(cx)).collect(),
            shown: self.tabs.shown_index().unwrap_or(0),
        }
    }

    /// The tabs, or what one of them shows, changed: keep them, if they are
    /// kept, and draw them again.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(record) = &self.record {
            let tabs = self.tabs(cx);
            if self.kept.as_ref() != Some(&tabs) {
                if let Err(error) = record.tabs(&tabs) {
                    tracing::error!("the stage's tabs are not kept: {error:#}");
                }
                self.kept = Some(tabs);
            }
        }
        cx.notify();
    }

    /// A terminal the human opens, with the keyboard in it.
    fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let running = match Running::start(Program::Shell, &self.cwd) {
            Ok(running) => running,
            Err(error) => {
                tracing::error!("cannot start a terminal: {error}");
                return;
            }
        };
        let terminal = cx.new(|cx| Terminal::new(running, None, cx));
        let changes = cx.observe(&terminal, |stage, _, cx| stage.changed(cx));
        self.push(TabContent::Terminal(terminal), changes, None, true, cx);
        self.focus_shown(window, cx);
    }

    fn show_panel(&self, cx: &mut Context<Self>) {
        self.panel.update(cx, Panel::open);
    }

    /// The index of the tab showing `page`.
    fn index_of(&self, page: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| match &tab.content {
            TabContent::Page { target, .. } => target.borrow().as_deref() == Some(page),
            TabContent::Document(_)
            | TabContent::Code(_)
            | TabContent::Tree(_)
            | TabContent::Changes(_)
            | TabContent::Flow(_)
            | TabContent::Terminal(_) => false,
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
        let (browser, events) = self.cef.open(url, Some(self.plugins.serve()));
        let target = browser.target_id();
        if let Some(opening) = opening {
            opening.is(target.clone());
        }
        let page = cx.new(|cx| Page::new(browser, events, window, cx));
        let changes = cx.observe(&page, |stage, _, cx| stage.changed(cx));
        let popups = cx.subscribe_in(&page, window, |stage, _, opens, window, cx| match opens {
            Opens::Tab(url) => {
                stage.open_tab(url, true, window, cx);
            }
            Opens::File(url) => stage.open(url, window, cx),
        });
        let content = TabContent::Page {
            page,
            target: target.clone(),
        };
        self.push(content, changes, Some(popups), show, cx);
        target
    }

    /// Puts a tab after the others, shown if `show` is.
    fn push(
        &mut self,
        content: TabContent,
        changes: Subscription,
        opens: Option<Subscription>,
        show: bool,
        cx: &mut Context<Self>,
    ) {
        self.tabs.push(
            Tab {
                content,
                _changes: changes,
                _opens: opens,
            },
            show,
        );
        self.changed(cx);
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
        let had_focus = self.has_focus(window, cx);
        self.tabs.remove(index);
        if had_focus {
            self.focus_shown(window, cx);
        }
        self.changed(cx);
        Ok(())
    }

    /// Shows the tab at `index`, with the keyboard in its page.
    fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.show(index) {
            self.focus_shown(window, cx);
            self.changed(cx);
        }
    }

    /// Whether the keyboard is in the tab shown.
    fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.tabs
            .shown()
            .is_some_and(|tab| tab.content.focus_handle(cx).contains_focused(window, cx))
    }

    fn focus_shown(&self, window: &mut Window, cx: &App) {
        if let Some(tab) = self.tabs.shown() {
            window.focus(&tab.content.focus_handle(cx));
        }
    }

    /// A tab the human opens: blank, with the keyboard in its address line,
    /// where what they type replaces the address.
    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(BLANK, true, window, cx);
        if let Some(page) = self.tabs.shown().and_then(|tab| tab.content.page()) {
            let address = page.read(cx).address().clone();
            window.focus(&address.focus_handle(cx));
            address.update(cx, LineInput::select_whole);
        }
    }

    fn render_tab(&self, index: usize, tab: &Tab, cx: &mut Context<Self>) -> Stateful<Div> {
        let number = index + 1;
        let title = tab.content.title(cx);
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
        if self.tabs.shown_index() == Some(index) {
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
            .child(
                div()
                    .id("new-terminal")
                    .debug_selector(|| "new-terminal".to_owned())
                    .flex_shrink_0()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x30_3030)).text_color(rgb(0xe0_e0e0)))
                    .child(">_")
                    .on_click(cx.listener(|stage, _, window, cx| stage.new_terminal(window, cx))),
            )
            .child(
                div()
                    .id("show-tree")
                    .debug_selector(|| "show-tree".to_owned())
                    .flex_shrink_0()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x30_3030)).text_color(rgb(0xe0_e0e0)))
                    .child("Files")
                    .on_click(cx.listener(|stage, _, window, cx| stage.show_tree(window, cx))),
            )
            .child(
                div()
                    .id("show-changes")
                    .debug_selector(|| "show-changes".to_owned())
                    .flex_shrink_0()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x30_3030)).text_color(rgb(0xe0_e0e0)))
                    .child("Changes")
                    .on_click(cx.listener(|stage, _, window, cx| {
                        let cwd = stage.cwd.clone();
                        stage.show_changes(cwd, window, cx);
                    })),
            )
            .children(self.render_plugins(cx))
    }

    /// A button for each plugin, after the tabs Cox draws, which opens its
    /// page, or shows the tab that does, wherever the page has gone.
    fn render_plugins(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        self.plugins
            .list()
            .into_iter()
            .map(|plugin| {
                let name = plugin.name.clone();
                div()
                    .id(SharedString::from(format!("plugin-{}", plugin.name)))
                    .debug_selector(|| format!("plugin-{}", plugin.name))
                    .flex_shrink_0()
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x30_3030)).text_color(rgb(0xe0_e0e0)))
                    .child(plugin.title)
                    .on_click(cx.listener(move |stage, _, window, cx| {
                        stage.show_plugin(&name, "", window, cx);
                    }))
            })
            .collect()
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
                    .children(self.tabs.shown().map(|tab| tab.content.view())),
            )
    }
}
