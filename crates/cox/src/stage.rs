//! The stage: where the human and the agent of one chat work together,
//! beside the chat's conversation. It holds tabs, each showing a page of
//! the browser (see [`crate::stage::page`]), a Markdown file (see
//! [`crate::stage::document`]), a file as it is written, such as code (see
//! [`crate::stage::code`]), the project's files as a tree (see [`crate::stage::tree`]),
//! or a terminal (see [`crate::stage::terminal`]). The first shows the page the
//! stage starts on; the human opens more pages and terminals, and the
//! tree, and closes tabs, and the agent opens and closes pages
//! too, through the chat's relay. A page that asks for a window of its own
//! gets a tab instead. With Cox's own tools (see [`crate::agent::mcp`]), the
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
//! - [`page`] shows a page of the browser, and sends the human's input
//!   to it.
//! - [`document`] shows a Markdown file, and [`code`] a file as it is
//!   written, such as code: both [`follow`] the file as it changes.
//! - [`servers`] holds the language servers a tab of code asks what the
//!   code means.
//! - [`tree`] shows the project's files, for the human to open one.
//! - [`terminal`] is a terminal the human types into, and the agent runs
//!   commands in.

pub mod code;
pub mod document;
pub mod follow;
pub mod page;
pub mod servers;
pub mod terminal;
pub mod tree;

use std::path::{Path, PathBuf};

use cox_core::store::Record;
use cox_core::tabs::{Lines, Tab as KeptTab, Tabs};
use gpui::{
    AnyView, App, Context, Div, Entity, FocusHandle, Focusable, Stateful, Subscription, Task,
    Window, div, prelude::*, px, rgb,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::agent::mcp::Ran;
use crate::agent::mcp::Request;
use crate::browser::relay::{FromAgent, Pages, Relay};
use crate::browser::{Cef, TargetId};

use crate::stage::code::Code;
use crate::stage::document::Document;
use crate::stage::page::{OpenTab, Page};
use crate::stage::servers::Servers;
use crate::stage::terminal::{Program, Running, Terminal};
use crate::stage::tree::{Chosen, Tree};
use crate::ui::input::LineInput;
use crate::ui::markdown::FileLink;
use crate::ui::{LinkClicked, Panel};

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
    tabs: Vec<Tab>,
    /// The index of the tab shown. There is always one: closing the last
    /// tab is refused.
    shown: usize,
    /// The chat's pages, as the relay knows them, if the relay started.
    pages: Option<Pages>,
    /// Whether the stage is open beside the conversation.
    panel: Entity<Panel>,
    /// Where the tabs are kept as they change, if they are.
    record: Option<Record>,
    /// The tabs as they were last kept.
    kept: Option<Tabs>,
    /// What carries out what the relay and the agent's tools ask, once
    /// the stage works with the agent.
    heard: Vec<Task<()>>,
}

/// A tab, and what it shows.
struct Tab {
    shown: Shown,
    /// Redraws the tab as the title of what it shows changes.
    _changes: Subscription,
    /// Opens the tabs that what it shows asks for, if it asks for any.
    _opens: Option<Subscription>,
}

enum Shown {
    Page {
        page: Entity<Page>,
        /// The page's id, which the relay names it by.
        target: TargetId,
    },
    Document(Entity<Document>),
    Code(Entity<Code>),
    Tree(Entity<Tree>),
    Terminal(Entity<Terminal>),
}

impl Shown {
    fn page(&self) -> Option<&Entity<Page>> {
        match self {
            Self::Page { page, .. } => Some(page),
            Self::Document(_) | Self::Code(_) | Self::Tree(_) | Self::Terminal(_) => None,
        }
    }

    fn view(&self) -> AnyView {
        match self {
            Self::Page { page, .. } => page.clone().into(),
            Self::Document(document) => document.clone().into(),
            Self::Code(code) => code.clone().into(),
            Self::Tree(tree) => tree.clone().into(),
            Self::Terminal(terminal) => terminal.clone().into(),
        }
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Page { page, .. } => page.focus_handle(cx),
            Self::Document(document) => document.focus_handle(cx),
            Self::Code(code) => code.focus_handle(cx),
            Self::Tree(tree) => tree.focus_handle(cx),
            Self::Terminal(terminal) => terminal.focus_handle(cx),
        }
    }

    /// Whether it is a terminal where a command of the agent's ended.
    fn is_agents_ended(&self, cx: &App) -> bool {
        match self {
            Self::Terminal(terminal) => {
                let terminal = terminal.read(cx);
                matches!(terminal.program(), Program::Command(_)) && terminal.ended().is_some()
            }
            Self::Page { .. } | Self::Document(_) | Self::Code(_) | Self::Tree(_) => false,
        }
    }
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
        let mut stage = Self::empty(cef, cwd, panel);
        stage.open_tab(url, true, window, cx);
        stage
    }

    /// A stage with no tab yet.
    fn empty(cef: Cef, cwd: PathBuf, panel: Entity<Panel>) -> Self {
        Self {
            cef,
            cwd,
            tabs: Vec::new(),
            shown: 0,
            pages: None,
            panel,
            record: None,
            kept: None,
            heard: Vec::new(),
        }
    }

    /// The chat's agent starts: from now on, the stage tells `relay` of
    /// its pages, those open and those it opens, carries out what the
    /// agent's tools ask in `from_tools`, and keeps its tabs in `record`.
    pub fn work(
        &mut self,
        relay: Option<Relay>,
        from_tools: Option<UnboundedReceiver<Request>>,
        record: Option<Record>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(relay) = relay {
            for tab in &self.tabs {
                if let Shown::Page { target, .. } = &tab.shown {
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
        kept: Option<Tabs>,
        url: &str,
        panel: Entity<Panel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut stage = Self::empty(cef, cwd, panel);
        let Tabs { tabs, shown } = kept.unwrap_or_default();
        // The tab shown is the one that was, or else the last before it
        // that opens again.
        let mut shown_now = None;
        for (index, tab) in tabs.into_iter().enumerate() {
            let opened = match tab {
                KeptTab::Page { url, .. } => {
                    stage.open_tab(&url, false, window, cx);
                    true
                }
                KeptTab::Markdown { path } => {
                    // What the file says now; the tab says so if it cannot
                    // be read.
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    stage.push_document(path, text, false, window, cx);
                    true
                }
                KeptTab::File { path, lines } => {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    stage.push_code(path, &text, lines, false, window, cx);
                    true
                }
                KeptTab::Files { open } => {
                    stage.push_tree(open, false, window, cx);
                    true
                }
                KeptTab::Terminal { .. } => false,
            };
            if opened && (index <= shown || shown_now.is_none()) {
                shown_now = Some(stage.tabs.len() - 1);
            }
        }
        match shown_now {
            Some(shown) => stage.shown = shown,
            None => {
                stage.open_tab(url, true, window, cx);
            }
        }
        stage
    }

    /// The tabs, as they are kept.
    fn tabs(&self, cx: &App) -> Tabs {
        let tabs = self
            .tabs
            .iter()
            .map(|tab| match &tab.shown {
                Shown::Page { page, .. } => {
                    let navigation = page.read(cx).navigation();
                    KeptTab::Page {
                        url: navigation.url.clone(),
                        title: navigation.title.clone(),
                    }
                }
                Shown::Document(document) => KeptTab::Markdown {
                    path: document.read(cx).path().to_owned(),
                },
                Shown::Code(code) => {
                    let code = code.read(cx);
                    KeptTab::File {
                        path: code.path().to_owned(),
                        lines: code.marked(),
                    }
                }
                Shown::Tree(tree) => KeptTab::Files {
                    open: tree.read(cx).open(),
                },
                Shown::Terminal(terminal) => KeptTab::Terminal {
                    title: terminal.read(cx).title(),
                },
            })
            .collect();
        Tabs {
            tabs,
            shown: self.shown,
        }
    }

    /// The tabs, or what one of them shows, changed: keep them, if they are
    /// kept, and draw them again.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(record) = &self.record {
            let tabs = self.tabs(cx);
            if self.kept.as_ref() != Some(&tabs) {
                if let Err(error) = record.tabs(&tabs) {
                    log::error!("the stage's tabs are not kept: {error:#}");
                }
                self.kept = Some(tabs);
            }
        }
        cx.notify();
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
                    Request::ShowFile { path, text, lines } => {
                        stage.show_code(path, &text, lines, window, cx);
                    }
                    Request::RunCommand {
                        command,
                        cwd,
                        answer,
                    } => stage.run_command(command, &cwd, answer, window, cx),
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
            Shown::Page { .. } | Shown::Code(_) | Shown::Tree(_) | Shown::Terminal(_) => false,
        });
        if let Some(index) = showing {
            if let Shown::Document(document) = &self.tabs[index].shown {
                document.update(cx, |document, cx| document.show(text, cx));
            }
            self.shown = index;
            self.changed(cx);
        } else {
            self.push_document(path, text, true, window, cx);
        }
        self.show_panel(cx);
    }

    /// Puts a tab showing the file at `path`, which says `text`, after the
    /// others, shown if `show` is.
    fn push_document(
        &mut self,
        path: PathBuf,
        text: String,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = cx.new(|cx| Document::new(path, text, cx));
        let changes = cx.observe(&document, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(
            &document,
            window,
            |stage, document, LinkClicked(url), window, cx| {
                // A link to a file is from where the document is.
                let path = document.read(cx).path();
                let from = path.parent().unwrap_or(path).to_owned();
                stage.open_from(url, &from, window, cx);
            },
        );
        self.push(Shown::Document(document), changes, Some(opens), show, cx);
    }

    /// Shows the file at `path`, which says `text`, as it is written, at
    /// `lines` if any: on the tab that shows it already, or a new one. The
    /// stage opens if it was closed.
    fn show_code(
        &mut self,
        path: PathBuf,
        text: &str,
        lines: Option<Lines>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let showing = self.tabs.iter().position(|tab| match &tab.shown {
            Shown::Code(code) => code.read(cx).path() == path,
            Shown::Page { .. } | Shown::Document(_) | Shown::Tree(_) | Shown::Terminal(_) => false,
        });
        if let Some(index) = showing {
            if let Shown::Code(code) = &self.tabs[index].shown {
                code.update(cx, |code, cx| code.show(text, lines, cx));
            }
            self.shown = index;
            self.changed(cx);
        } else {
            self.push_code(path, text, lines, true, window, cx);
        }
        self.show_panel(cx);
    }

    /// Puts a tab showing the file at `path`, which says `text`, as it is
    /// written, at `lines` if any, after the others, shown if `show` is.
    fn push_code(
        &mut self,
        path: PathBuf,
        text: &str,
        lines: Option<Lines>,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The project's server for the file's language, when Cox has them.
        let server = cx
            .try_global::<Servers>()
            .cloned()
            .and_then(|servers| servers.of(&self.cwd, &path, cx));
        let code = cx.new(|cx| Code::new(path, text, lines, server, cx));
        let changes = cx.observe(&code, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(
            &code,
            window,
            |stage, code, LinkClicked(url), window, cx| {
                // A link to a file is from where the file is.
                let path = code.read(cx).path();
                let from = path.parent().unwrap_or(path).to_owned();
                stage.open_from(url, &from, window, cx);
            },
        );
        self.push(Shown::Code(code), changes, Some(opens), show, cx);
    }

    /// Runs `command`, which the agent asked for, in `cwd`, and answers
    /// `answer` once it ends: on the tab of the agent's last command if
    /// that ended, or a new one. The stage opens if it was closed.
    fn run_command(
        &mut self,
        command: String,
        cwd: &std::path::Path,
        answer: tokio::sync::oneshot::Sender<Result<Ran, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let running = match Running::start(Program::Command(command), cwd) {
            Ok(running) => running,
            Err(error) => {
                let _ = answer.send(Err(format!("cannot start a terminal: {error}")));
                return;
            }
        };
        let terminal = cx.new(|cx| Terminal::new(running, Some(answer), cx));
        let changes = cx.observe(&terminal, |stage, _, cx| stage.changed(cx));
        let ended = self
            .tabs
            .iter()
            .rposition(|tab| tab.shown.is_agents_ended(cx));
        if let Some(index) = ended {
            let had_focus = self.tabs[index]
                .shown
                .focus_handle(cx)
                .contains_focused(window, cx);
            self.tabs[index] = Tab {
                shown: Shown::Terminal(terminal),
                _changes: changes,
                _opens: None,
            };
            self.shown = index;
            if had_focus {
                self.focus_shown(window, cx);
            }
            self.changed(cx);
        } else {
            self.push(Shown::Terminal(terminal), changes, None, true, cx);
        }
        self.show_panel(cx);
    }

    /// A terminal the human opens, with the keyboard in it.
    fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let running = match Running::start(Program::Shell, &self.cwd) {
            Ok(running) => running,
            Err(error) => {
                log::error!("cannot start a terminal: {error}");
                return;
            }
        };
        let terminal = cx.new(|cx| Terminal::new(running, None, cx));
        let changes = cx.observe(&terminal, |stage, _, cx| stage.changed(cx));
        self.push(Shown::Terminal(terminal), changes, None, true, cx);
        self.focus_shown(window, cx);
    }

    /// The agent worked on `page`, sending it `input` if any: show its tab,
    /// and the stage.
    fn worked(
        &mut self,
        page: Option<String>,
        input: Option<crate::browser::relay::AgentInput>,
        cx: &mut Context<Self>,
    ) {
        let index = page
            .and_then(|page| self.index_of(&page))
            .unwrap_or(self.shown);
        if index != self.shown {
            self.shown = index;
            self.changed(cx);
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
            Shown::Document(_) | Shown::Code(_) | Shown::Tree(_) | Shown::Terminal(_) => false,
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
        let changes = cx.observe(&page, |stage, _, cx| stage.changed(cx));
        let popups = cx.subscribe_in(&page, window, |stage, _, OpenTab(url), window, cx| {
            stage.open_tab(url, true, window, cx);
        });
        let shown = Shown::Page {
            page,
            target: target.clone(),
        };
        self.push(shown, changes, Some(popups), show, cx);
        target
    }

    /// Puts a tab after the others, shown if `show` is.
    fn push(
        &mut self,
        shown: Shown,
        changes: Subscription,
        opens: Option<Subscription>,
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
        let had_focus = self.tabs[self.shown]
            .shown
            .focus_handle(cx)
            .contains_focused(window, cx);
        self.tabs.remove(index);
        self.shown = after_closing(self.shown, index, self.tabs.len());
        if had_focus {
            self.focus_shown(window, cx);
        }
        self.changed(cx);
        Ok(())
    }

    /// Shows the tab at `index`, with the keyboard in its page.
    fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() {
            self.shown = index;
            self.focus_shown(window, cx);
            self.changed(cx);
        }
    }

    fn focus_shown(&self, window: &mut Window, cx: &App) {
        window.focus(&self.tabs[self.shown].shown.focus_handle(cx));
    }

    /// Opens `url`, a link the human clicked: on the tab that shows it
    /// already, or a new one. A link to a file is from where the chat
    /// works.
    pub fn open(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.cwd.clone();
        self.open_from(url, &cwd, window, cx);
    }

    /// Opens `url`, a link the human clicked in what is in `from`, which a
    /// link to a file is from.
    fn open_from(&mut self, url: &str, from: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(link) = crate::ui::markdown::file_link(url) {
            self.open_file(link, from, window, cx);
            return;
        }
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

    /// Opens the file `link` links to, from `from`, with the keyboard in
    /// its tab.
    fn open_file(
        &mut self,
        link: FileLink<'_>,
        from: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = match (link.path.strip_prefix("~/"), std::env::var_os("HOME")) {
            (Some(rest), Some(home)) => Path::new(&home).join(rest),
            _ => from.join(link.path),
        };
        if let Err(error) = self.open_path(&path, link.lines, window, cx) {
            log::warn!("cannot open {}: {error}", path.display());
        }
    }

    /// Opens the file at `path`, at `lines` if any, with the keyboard in
    /// its tab: as Markdown if it is a Markdown file and no lines are
    /// given, and as it is written otherwise. Opens nothing if the file
    /// cannot be read.
    fn open_path(
        &mut self,
        path: &Path,
        lines: Option<Lines>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> std::io::Result<()> {
        let path = std::fs::canonicalize(path)?;
        let text = std::fs::read_to_string(&path)?;
        let is_markdown = path
            .extension()
            .is_some_and(|extension| extension == "md" || extension == "markdown");
        if is_markdown && lines.is_none() {
            self.show_document(path, text, window, cx);
        } else {
            self.show_code(path, &text, lines, window, cx);
        }
        self.focus_shown(window, cx);
        Ok(())
    }

    /// Shows the tree of the project's files: on its tab, or a new one,
    /// with the keyboard in it.
    fn show_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let showing = self
            .tabs
            .iter()
            .position(|tab| matches!(tab.shown, Shown::Tree(_)));
        if let Some(index) = showing {
            self.show(index, window, cx);
        } else {
            self.push_tree(Vec::new(), true, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// Puts a tab showing the tree of the project's files, with the
    /// folders `open` open, after the others, shown if `show` is.
    fn push_tree(
        &mut self,
        open: Vec<PathBuf>,
        show: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tree = cx.new(|cx| Tree::new(self.cwd.clone(), open, cx));
        let changes = cx.observe(&tree, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(&tree, window, |stage, _, Chosen(path), window, cx| {
            // A file that cannot be read gets a tab all the same, which
            // says why.
            if stage.open_path(path, None, window, cx).is_err() {
                stage.show_code(path.clone(), "", None, window, cx);
                stage.focus_shown(window, cx);
            }
        });
        self.push(Shown::Tree(tree), changes, Some(opens), show, cx);
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
            Shown::Code(code) => code.read(cx).title(),
            Shown::Tree(tree) => tree.read(cx).title(),
            Shown::Terminal(terminal) => terminal.read(cx).title(),
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
