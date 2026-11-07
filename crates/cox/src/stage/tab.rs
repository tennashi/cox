//! A tab of the stage, and what it shows: a page of the browser, a
//! Markdown file, a file as it is written, the project's files, what
//! changed in them, the map of the project, the flow of a tool call, a
//! terminal, or an Android emulator. What the stage asks of every kind alike, the name the tab
//! goes by, how it is kept for a closed chat's stage to open again, the
//! keyboard's place in it and the view drawn, each kind answers as
//! [`Shows`], and [`TabContent`] turns to that in one place. What the stage
//! asks of one kind alone, such as the page a tab shows, it asks of that
//! kind, matching on [`TabContent`].

use std::path::{Path, PathBuf};

use cox_core::tabs::{KeptTab, Lines};
use gpui::{
    AnyView, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Render, Subscription,
};

use super::android::Android;
use super::changes::Changes;
use super::code::Code;
use super::devices::Devices;
use super::document::Document;
use super::flow::Flow;
use super::mac_window::MacWindow;
use super::page::Page;
use super::terminal::{Program, Terminal};
use super::tree::Tree;
use super::windows::Windows;
use crate::ui::LinkClicked;
use cox_browser::TargetId;

/// What a tab shows, whichever kind it is.
pub trait Shows: Render + Focusable + 'static {
    /// The name the tab goes by.
    fn title(&self) -> String;

    /// The tab as it is kept, for a closed chat's stage to open again.
    fn kept(&self) -> KeptTab;
}

/// What a tab showing a file shows, as Markdown or as it is written: the
/// stage shows a file the same way whichever it is, on the tab showing it
/// already or a new one, and opens the links clicked in it.
pub(super) trait ShowsFile: Shows + EventEmitter<LinkClicked> {
    /// A tab showing the file at `path`, an absolute path, which says
    /// `text`, at `lines` if it shows lines, for a chat working in `cwd`.
    fn open(
        cwd: &Path,
        path: PathBuf,
        text: &str,
        lines: Option<Lines>,
        cx: &mut Context<Self>,
    ) -> Self;

    /// The file, as an absolute path.
    fn path(&self) -> &Path;

    /// Shows `text`, which the file says now, at `lines` if it shows lines.
    fn show(&mut self, text: &str, lines: Option<Lines>, cx: &mut Context<Self>);

    /// The tab showing `this`, as the stage holds it.
    fn content(this: Entity<Self>) -> TabContent;

    /// What `content` shows, if it is one of these.
    fn of(content: &TabContent) -> Option<&Entity<Self>>;
}

/// A tab, and what it shows.
pub(super) struct Tab {
    pub(super) content: TabContent,
    /// The chat it is of, by its key: the chat whose agent opened it, or
    /// that the human opened it from; `None` for a tab of no chat.
    pub(super) owner: Option<u64>,
    /// Redraws the tab as the title of what it shows changes.
    pub(super) _changes: Subscription,
    /// Opens the tabs that what it shows asks for, if it asks for any.
    pub(super) _opens: Option<Subscription>,
}

/// What a tab shows, by kind.
pub(super) enum TabContent {
    Page {
        page: Entity<Page>,
        /// The page's id, which the relay names it by.
        target: TargetId,
    },
    Document(Entity<Document>),
    Code(Entity<Code>),
    Tree(Entity<Tree>),
    Changes(Entity<Changes>),
    Flow(Entity<Flow>),
    Terminal(Entity<Terminal>),
    Android(Entity<Android>),
    Devices(Entity<Devices>),
    MacWindow(Entity<MacWindow>),
    Windows(Entity<Windows>),
}

/// An entity that [`Shows`], asked as the stage asks of every kind alike.
trait AnyShows {
    fn title(&self, cx: &App) -> String;
    fn kept(&self, cx: &App) -> KeptTab;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn view(&self) -> AnyView;
}

impl<T: Shows> AnyShows for Entity<T> {
    fn title(&self, cx: &App) -> String {
        self.read(cx).title()
    }

    fn kept(&self, cx: &App) -> KeptTab {
        self.read(cx).kept()
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn view(&self) -> AnyView {
        self.clone().into()
    }
}

impl TabContent {
    /// What the tab shows, as any kind shows it.
    fn shows(&self) -> &dyn AnyShows {
        match self {
            Self::Page { page, .. } => page,
            Self::Document(document) => document,
            Self::Code(code) => code,
            Self::Tree(tree) => tree,
            Self::Changes(changes) => changes,
            Self::Flow(flow) => flow,
            Self::Terminal(terminal) => terminal,
            Self::Android(android) => android,
            Self::Devices(devices) => devices,
            Self::MacWindow(window) => window,
            Self::Windows(windows) => windows,
        }
    }

    /// The name the tab goes by.
    pub(super) fn title(&self, cx: &App) -> String {
        self.shows().title(cx)
    }

    /// The tab as it is kept.
    pub(super) fn kept(&self, cx: &App) -> KeptTab {
        self.shows().kept(cx)
    }

    /// Where the keyboard goes in the tab.
    pub(super) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.shows().focus_handle(cx)
    }

    /// The view the tab draws.
    pub(super) fn view(&self) -> AnyView {
        self.shows().view()
    }

    /// The page the tab shows, if it shows one.
    pub(super) fn page(&self) -> Option<&Entity<Page>> {
        match self {
            Self::Page { page, .. } => Some(page),
            Self::Document(_)
            | Self::Code(_)
            | Self::Tree(_)
            | Self::Changes(_)
            | Self::Flow(_)
            | Self::Terminal(_)
            | Self::Android(_)
            | Self::Devices(_)
            | Self::MacWindow(_)
            | Self::Windows(_) => None,
        }
    }

    /// Whether it is a terminal where a command of the agent's ended.
    pub(super) fn is_agents_ended(&self, cx: &App) -> bool {
        match self {
            Self::Terminal(terminal) => {
                let terminal = terminal.read(cx);
                matches!(terminal.program(), Program::Command(_)) && terminal.ended().is_some()
            }
            Self::Page { .. }
            | Self::Document(_)
            | Self::Code(_)
            | Self::Tree(_)
            | Self::Changes(_)
            | Self::Flow(_)
            | Self::Android(_)
            | Self::Devices(_)
            | Self::MacWindow(_)
            | Self::Windows(_) => false,
        }
    }
}
