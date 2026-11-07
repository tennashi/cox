//! The stage: where the human and the agents of the chats that work in one
//! folder, as a project's, work together, beside the conversation of the
//! chat shown. It holds tabs, each showing a page of
//! the browser (see [`crate::stage::page`]), a Markdown file (see
//! [`crate::stage::document`]), a file as it is written, such as code (see
//! [`crate::stage::code`]), the project's files as a tree (see [`crate::stage::tree`]),
//! what changed in them (see [`crate::stage::changes`]), or a terminal
//! (see [`crate::stage::terminal`]); and the pages of the stage's
//! plugins (see [`crate::stage::plugins`]), such as the map of the
//! project (see [`crate::stage::map`]), are pages of the browser. The first shows the page the
//! stage starts on; the human opens more pages and terminals, and the
//! tree, and closes tabs, and each agent opens and closes pages
//! too, through its chat's relay. A page that asks for a window of its own
//! gets a tab instead, and a link in a page to a file opens the file on a
//! tab, as a link from the conversation does. With Cox's own tools (see [`crate::agent::mcp`]), the
//! agent shows the human Markdown files and code, a tab for each file, and
//! runs commands in terminals: a command gets the tab of the agent's last
//! one that ended, or a new one. A link to a file opens it on a tab too:
//! as Markdown if it is a Markdown file and the link names no lines, and
//! as it is written otherwise. A click on a file in the tree opens it the
//! same way.
//!
//! Each tab is the human's, as every tab they open is, or of one of the
//! chats, as every tab its agent opens is, or one the human handed to it
//! (see [`owners`]). The stage tells each chat's relay of the pages of
//! that chat, so that its agent sees those, and none of the human's or
//! the other chats'; an agent's command gets the tab of its own last one
//! that ended. When an agent works on a page, shows a file or runs a
//! command, the stage shows that tab, if the agent's chat is the one
//! shown, or else opens the tab behind the one shown; the tabs of the
//! chats not shown are drawn dimmer.
//!
//! The stage keeps its tabs as they change, with the chat each is of. A
//! stage opens again on the pages and files kept, for the human to look
//! at; its terminals are not started again. Once the human sends to a
//! chat again, its agent works on its own pages.
//!
//! - [`tab`] is a tab, and what it shows, whichever kind it is, [`owners`]
//!   whose it is, and [`bar`] the bar of the tabs, which keeps the views
//!   of the project, its files, what changed and the plugins' pages,
//!   apart from the rest.
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
//! - [`android`] shows an Android emulator, which the human touches and
//!   types into, and [`devices`] lists the emulator's virtual devices, for
//!   the human to start one.
//! - [`mac_window`] shows a window of another app on the Mac, which the
//!   human clicks and types into, and [`windows`] lists those windows, for
//!   the human to show one.
//! - [`plugins`] are pages that draw what Cox knows, as a tab would, which
//!   the stage serves its pages itself at `cox:` addresses: [`map`] tells
//!   the project's map, as an agent wrote it, with what changed laid over
//!   its boxes, for the human to see where things are.

pub mod activity;
pub mod android;
mod asked;
mod bar;
pub mod calls;
pub mod changes;
pub mod code;
pub mod devices;
pub mod document;
pub mod flow;
pub mod follow;
pub mod lines;
pub mod links;
pub mod mac_window;
pub mod map;
mod mirror;
pub mod outline;
pub mod owners;
pub mod page;
pub mod plugins;
pub mod servers;
pub mod tab;
pub mod terminal;
pub mod tree;
mod typed;
pub mod windows;

use std::collections::HashMap;
use std::path::PathBuf;

use cox_core::list::List;
use cox_core::tabs::{KeptTab, KeptTabs};
use gpui::{App, Context, Entity, Focusable, Subscription, Task, Window, actions, div, prelude::*};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::agent::mcp::ToolRequest;
use cox_browser::relay::{Pages, Relay};
use cox_browser::{Cef, TargetId};

use cox_android::device::Boot;

use crate::stage::android::Android;
use crate::stage::calls::Calls;
use crate::stage::code::Code;
use crate::stage::devices::{Devices, OpenDevice};
use crate::stage::document::Document;
use crate::stage::mac_window::MacWindow;
use crate::stage::outline::Outlines;
use crate::stage::page::{Opens, Page};
use crate::stage::plugins::Plugins;
use crate::stage::tab::{Shows, Tab, TabContent};
use crate::stage::terminal::{Program, Running, Terminal};
use crate::stage::windows::{ShowWindow, Windows};
use crate::ui::Panel;
use crate::ui::input::LineInput;

actions!(
    stage,
    [
        /// Opens a blank page, with the keyboard in its address line.
        NewPage,
        /// Opens a terminal with the human's shell.
        NewTerminal,
        /// Asks for a file, and opens it on a tab.
        OpenFile,
    ]
);

/// Binds the keys of the menu of the stage's +: Escape closes it.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([gpui::KeyBinding::new(
        "escape",
        bar::CloseMenu,
        Some(bar::MENU_CONTEXT),
    )]);
}

/// What keeps a stage's tabs as they change, each of a chat by its key.
pub type Keep = std::rc::Rc<dyn Fn(&KeptTabs<u64>)>;

/// What a new tab shows when the human opens it.
const BLANK: &str = "about:blank";

/// How wide a tab grows, at most.
const TAB_WIDTH: f32 = 200.;

/// The most of a tab's title that its tab shows.
const TITLE_LENGTH: usize = 40;

pub struct Stage {
    cef: Cef,
    /// Where its chats work, which its terminals start in.
    cwd: PathBuf,
    /// The tabs, and which is shown. There is always one once the stage
    /// has started: closing the last tab is refused.
    tabs: List<Tab>,
    /// The pages of each chat whose relay started, as the relay knows
    /// them, by the chat's key.
    pages: HashMap<u64, Pages>,
    /// The chat whose agent the stage carries out what it asks for, while
    /// it does.
    acting: Option<u64>,
    /// The chat shown last of those that work on the stage, which the
    /// human hands their tabs to.
    showing: Option<u64>,
    /// What names the chats, for the human to hand a tab to one.
    names: Option<owners::Names>,
    /// The menu of the right button on a tab, while it is open: where the
    /// mouse went down, and the tab's index.
    tab_menu: Option<(gpui::Point<gpui::Pixels>, usize)>,
    /// Whether the stage is in sight, in the middle of the window.
    panel: Entity<Panel>,
    /// What keeps the tabs as they change, if they are kept.
    keep: Option<Keep>,
    /// The tabs as they were last kept.
    kept: Option<KeptTabs<u64>>,
    /// What carries out what the relay and the agent's tools ask, once
    /// the stage works with the agent.
    heard: Vec<Task<()>>,
    /// The plugins the stage shows as pages, and what they read of the
    /// folder.
    plugins: Plugins,
    /// The menu that opens a page, a terminal or a file, while it is open.
    menu: Option<bar::Menu>,
    /// How many files changed in the project, as the bar last counted
    /// them while the stage was open; none until then, or out of Git.
    changed: Option<usize>,
    /// Whether the project has a map, as the bar last looked while the
    /// stage was open.
    mapped: bool,
    /// How many chats of the folder wait for the human, as the bar last
    /// counted them while the stage was open.
    waiting: usize,
    _counting: Task<()>,
    /// Where the tabs scroll along the bar.
    strip: bar::Strip,
}

impl Stage {
    /// A stage of one tab showing `url`, for the chats working in `cwd`,
    /// in sight while `panel` says so. Until it is
    /// [kept](Self::keep_with), nothing of it is kept.
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
            menu: None,
            changed: None,
            mapped: false,
            waiting: 0,
            _counting: bar::count_changes(cx),
            strip: bar::Strip::default(),
            cwd,
            tabs: List::new(),
            pages: HashMap::new(),
            acting: None,
            showing: None,
            names: None,
            tab_menu: None,
            panel,
            keep: None,
            kept: None,
            heard: vec![answering, answering_calls],
        }
    }

    /// The stage works with the chat `chat`, or its agent starts: from
    /// now on, the stage tells `relay` of the chat's pages, those open and
    /// those it opens, and carries out what the agent's tools ask in
    /// `from_tools`, each when given.
    pub fn work(
        &mut self,
        chat: u64,
        relay: Option<Relay>,
        from_tools: Option<UnboundedReceiver<ToolRequest>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(relay) = relay {
            for tab in &self.tabs {
                if let TabContent::Page { target, .. } = &tab.content
                    && tab.owner == Some(chat)
                {
                    relay.pages.opening().is(target.clone());
                }
            }
            self.pages.insert(chat, relay.pages);
            self.heard
                .push(Self::hear(chat, relay.from_agent, window, cx));
        }
        if let Some(from_tools) = from_tools {
            self.heard
                .push(Self::hear_tools(chat, from_tools, window, cx));
        }
        self.changed(cx);
    }

    /// Keeps the tabs with `keep` from now on, as they change.
    pub fn keep_with(&mut self, keep: Keep, cx: &mut Context<Self>) {
        self.keep = Some(keep);
        self.changed(cx);
    }

    /// The stage of the chats that work in `cwd`, on the pages and files
    /// of `kept`, each of the chat it was of, or of one tab showing `url`
    /// when none of them is left. Nothing of it is kept, until it is
    /// [kept](Self::keep_with).
    pub fn reading(
        cef: Cef,
        cwd: PathBuf,
        kept: Option<KeptTabs<u64>>,
        url: &str,
        panel: Entity<Panel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut stage = Self::empty(cef, cwd, panel, cx);
        let kept = kept.unwrap_or_default();
        let owners: Vec<Option<u64>> = (0..kept.tabs.len())
            .map(|index| kept.owner(index))
            .collect();
        let KeptTabs { tabs, shown, .. } = kept;
        for ((index, tab), owner) in tabs.into_iter().enumerate().zip(owners) {
            stage.acting = owner;
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
                KeptTab::Android { device } => {
                    let android = android::reopened(device, cx);
                    stage.push_android(android, show, cx);
                }
                KeptTab::AndroidDevices => {
                    stage.push_devices(show, window, cx);
                }
                KeptTab::MacWindow { app, title } => {
                    let tab = mac_window::reopened(app, title, cx);
                    stage.push_mac_window(tab, show, cx);
                }
                KeptTab::MacWindows => {
                    stage.push_windows(show, window, cx);
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
        stage.acting = None;
        if stage.tabs.is_empty() {
            stage.open_tab(url, true, window, cx);
        }
        stage
    }

    /// The tabs, as they are kept, each of a chat by its key.
    #[must_use]
    pub fn tabs(&self, cx: &App) -> KeptTabs<u64> {
        KeptTabs {
            tabs: self.tabs.iter().map(|tab| tab.content.kept(cx)).collect(),
            shown: self.tabs.shown_index().unwrap_or(0),
            owners: self.tabs.iter().map(|tab| tab.owner).collect(),
        }
    }

    /// The chat `chat` is shown, of those that work on the stage: the human
    /// hands their tabs to it from now on, and the stage shows its last
    /// tab, unless the tab shown is of it or the human's.
    pub fn showing(&mut self, chat: u64, cx: &mut Context<Self>) {
        if self.showing == Some(chat) {
            return;
        }
        self.showing = Some(chat);
        let shown_is_others = self
            .tabs
            .shown()
            .is_some_and(|tab| tab.owner.is_some_and(|owner| owner != chat));
        if shown_is_others
            && let Some(index) = self.tabs.iter().rposition(|tab| tab.owner == Some(chat))
        {
            self.tabs.show(index);
            self.changed(cx);
        }
        cx.notify();
    }

    /// The chat the tabs opened now are of: the chat whose agent the
    /// stage carries out what it asks for, or else none, for the human.
    fn owner(&self) -> Option<u64> {
        self.acting
    }

    /// Does `act` for the chat `chat`, whose agent asked for it, or for
    /// the human when `None`.
    fn acting_for<R>(&mut self, chat: Option<u64>, act: impl FnOnce(&mut Self) -> R) -> R {
        let was = std::mem::replace(&mut self.acting, chat);
        let done = act(self);
        self.acting = was;
        done
    }

    /// Whether what is done now is in the human's sight: done by the human,
    /// or by the agent of the chat shown, or of any chat while none of
    /// those of the stage was shown yet. The tabs another chat's agent
    /// opens open behind the one shown, and the stage stays as it is.
    fn in_sight(&self) -> bool {
        self.acting.is_none() || self.showing.is_none() || self.acting == self.showing
    }

    /// The tabs, or what one of them shows, changed: keep them, if they are
    /// kept, and draw them again.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(keep) = &self.keep {
            let tabs = self.tabs(cx);
            if self.kept.as_ref() != Some(&tabs) {
                keep(&tabs);
                self.kept = Some(tabs);
            }
        }
        cx.notify();
    }

    /// A terminal the human opens, with the keyboard in it.
    pub fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_terminal(Program::Shell, window, cx);
    }

    /// A terminal running `program`, which the human started, with the
    /// keyboard in it.
    fn start_terminal(&mut self, program: Program, window: &mut Window, cx: &mut Context<Self>) {
        let running = match Running::start(program, &self.cwd) {
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

    /// Shows the tab of the Android emulator's virtual devices, opening it
    /// if there is none.
    pub fn show_devices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self
            .tabs
            .iter()
            .position(|tab| matches!(tab.content, TabContent::Devices(_)));
        if let Some(index) = index {
            self.show(index, window, cx);
            self.focus_shown(window, cx);
        } else {
            self.push_devices(true, window, cx);
            self.focus_shown(window, cx);
        }
    }

    /// Puts the tab of the Android emulator's virtual devices after the
    /// others, shown if `show` is.
    fn push_devices(&mut self, show: bool, window: &mut Window, cx: &mut Context<Self>) {
        let devices = cx.new(Devices::new);
        let changes = cx.observe(&devices, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(
            &devices,
            window,
            |stage, _, open: &OpenDevice, window, cx| {
                stage.open_android(&open.name, open.boot, window, cx);
            },
        );
        self.push(TabContent::Devices(devices), changes, Some(opens), show, cx);
    }

    /// Shows the tab of the windows of the Mac's other apps, opening it if
    /// there is none.
    pub fn show_windows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self
            .tabs
            .iter()
            .position(|tab| matches!(tab.content, TabContent::Windows(_)));
        if let Some(index) = index {
            self.show(index, window, cx);
        } else {
            self.push_windows(true, window, cx);
        }
        self.focus_shown(window, cx);
    }

    /// Puts the tab of the windows of the Mac's other apps after the
    /// others, shown if `show` is.
    fn push_windows(&mut self, show: bool, window: &mut Window, cx: &mut Context<Self>) {
        let windows = cx.new(Windows::new);
        let changes = cx.observe(&windows, |stage, _, cx| stage.changed(cx));
        let opens = cx.subscribe_in(
            &windows,
            window,
            |stage, _, shown: &ShowWindow, window, cx| {
                stage.open_mac_window(shown.0.clone(), window, cx);
            },
        );
        self.push(TabContent::Windows(windows), changes, Some(opens), show, cx);
    }

    /// Shows the tab of `app_window`, a window of another app on the Mac,
    /// with the keyboard in it: the tab showing it, or one kept of its app
    /// and title, which shows it again, or a new one.
    pub fn open_mac_window(
        &mut self,
        app_window: cox_mac::Window,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = self.tabs.iter().position(|tab| match &tab.content {
            TabContent::MacWindow(tab) => {
                let tab = tab.read(cx);
                match tab.window() {
                    Some(shown) => shown.id == app_window.id,
                    None => {
                        tab.kept()
                            == KeptTab::MacWindow {
                                app: app_window.app.clone(),
                                title: app_window.title.clone(),
                            }
                    }
                }
            }
            _ => false,
        });
        if let Some(index) = index {
            if let TabContent::MacWindow(tab) = &self.tabs[index].content {
                tab.update(cx, |tab, cx| {
                    if !tab.is_showing() {
                        tab.show(app_window, cx);
                    }
                });
            }
            self.show(index, window, cx);
        } else {
            let tab = cx.new(|cx| MacWindow::new(app_window, cx));
            self.push_mac_window(tab, true, cx);
        }
        self.focus_shown(window, cx);
    }

    /// Puts a tab of a window of another app after the others, shown if
    /// `show` is.
    fn push_mac_window(&mut self, tab: Entity<MacWindow>, show: bool, cx: &mut Context<Self>) {
        let changes = cx.observe(&tab, |stage, _, cx| stage.changed(cx));
        self.push(TabContent::MacWindow(tab), changes, None, show, cx);
    }

    /// Shows the tab of the Android emulator of the virtual device `name`,
    /// starting it as `boot` says, or reaching it if it runs, with the
    /// keyboard in it: the tab it has, if it has one, or a new one.
    pub fn open_android(
        &mut self,
        name: &str,
        boot: Boot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = self.tabs.iter().position(|tab| match &tab.content {
            TabContent::Android(android) => android.read(cx).name() == name,
            _ => false,
        });
        if let Some(index) = index {
            if let TabContent::Android(android) = &self.tabs[index].content {
                android.update(cx, |android, cx| {
                    if !android.is_running() {
                        android.open(boot, cx);
                    }
                });
            }
            self.show(index, window, cx);
            self.focus_shown(window, cx);
            return;
        }
        let name = name.to_owned();
        let android = cx.new(|cx| Android::new(name, Some(boot), cx));
        self.push_android(android, true, cx);
        self.focus_shown(window, cx);
    }

    /// Puts a tab of an Android emulator after the others, shown if `show`
    /// is.
    fn push_android(&mut self, android: Entity<Android>, show: bool, cx: &mut Context<Self>) {
        let changes = cx.observe(&android, |stage, _, cx| stage.changed(cx));
        self.push(TabContent::Android(android), changes, None, show, cx);
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
            | TabContent::Terminal(_)
            | TabContent::Android(_)
            | TabContent::Devices(_)
            | TabContent::MacWindow(_)
            | TabContent::Windows(_) => false,
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
        // The relay of the page's chat hears of it before CEF makes it.
        let opening = self
            .owner()
            .and_then(|owner| self.pages.get(&owner))
            .map(Pages::opening);
        let (browser, events) = self.cef.open(url, Some(self.plugins.serve()));
        let target = browser.target_id();
        if let Some(opening) = opening {
            opening.is(target.clone());
        }
        let page = cx.new(|cx| Page::new(browser, events, window, cx));
        let changes = cx.observe(&page, |stage, _, cx| stage.changed(cx));
        // What the page opens is of the page's chat.
        let popups = cx.subscribe_in(&page, window, |stage, page, opens, window, cx| {
            let owner = stage
                .tabs
                .iter()
                .find(|tab| tab.content.page() == Some(page))
                .and_then(|tab| tab.owner);
            stage.acting_for(owner, |stage| match opens {
                Opens::Tab(url) => {
                    stage.open_tab(url, true, window, cx);
                }
                Opens::File(url) => stage.open(url, window, cx),
                Opens::Chat(key) => cx.emit(owners::ShowChat(*key)),
            });
        });
        let content = TabContent::Page {
            page,
            target: target.clone(),
        };
        self.push(content, changes, Some(popups), show, cx);
        target
    }

    /// Puts a tab after the others, of the chat the tabs opened now are
    /// of, or the human's, shown if `show` is and it is in the human's
    /// sight, or if no tab is shown.
    fn push(
        &mut self,
        content: TabContent,
        changes: Subscription,
        opens: Option<Subscription>,
        show: bool,
        cx: &mut Context<Self>,
    ) {
        let show = (show && self.in_sight()) || self.tabs.shown_index().is_none();
        self.tabs.push(
            Tab {
                content,
                owner: self.owner(),
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
        self.tab_menu = None;
        self.tabs.remove(index);
        if had_focus {
            self.focus_shown(window, cx);
        }
        self.changed(cx);
        Ok(())
    }

    /// Shows the tab at `index`, with the keyboard in its page, if it is
    /// in the human's sight.
    fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_sight() && self.tabs.show(index) {
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
        if !self.in_sight() {
            return;
        }
        if let Some(tab) = self.tabs.shown() {
            window.focus(&tab.content.focus_handle(cx));
        }
    }

    /// A tab the human opens: blank, with the keyboard in its address line,
    /// where what they type replaces the address.
    pub fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tab(BLANK, true, window, cx);
        if let Some(page) = self.tabs.shown().and_then(|tab| tab.content.page()) {
            let address = page.read(cx).address().clone();
            window.focus(&address.focus_handle(cx));
            address.update(cx, LineInput::select_whole);
        }
    }
}

impl Render for Stage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .children(self.tabs.shown().map(|tab| tab.content.view())),
            )
    }
}
