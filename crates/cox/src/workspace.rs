//! The window: the projects on a rail down its left side, the stage of
//! the chat shown in its middle, and the chat itself in a panel at its
//! right, which a list at its head switches among the chats of the
//! project chosen. A project is a folder the human opened: each chat in
//! it works there, and its agent gets the human's instructions, then
//! those in the folder's `COX.md` (see [`crate::agent::instructions`]).
//! Cox's own chats, of no project, work in Cox's home, with the
//! instructions there.
//!
//! The window runs no chat itself: the daemon does (see
//! [`crate::daemon`]), which tells it the projects and chats, and how the
//! conversation of each chat the window shows changes, and which the
//! window tells what the human does. What the window keeps of its own is
//! what it shows: which chat, the menus and inputs open, and the stage of
//! each folder the chats work in, which the chats there share, and which
//! carries out what each chat's agent asks of it, as the daemon passes it
//! on.
//!
//! The stage stays as it is while the human switches between chats that
//! work in one folder, which share it. The head of the panel names the
//! chat shown, and counts the project's other chats that wait for the
//! human; a click on it opens the list of the project's chats, each
//! telling whether its agent is at work, waits for the human, or finished
//! a turn the human has not seen yet, and whether it has a goal, and the
//! project's button on the rail counts its chats that wait for the human.
//! The panel folds to the window's right edge, where a dot for each chat
//! stays, which shows it. The menu of the right button on a chat renames
//! it, as a double click on the head does, sets or clears its goal, ends
//! a sketch (see [`crate::chat::sketch`]), or archives it: an archived
//! chat leaves the list for "Archived" at its foot, from where a click
//! brings it back.
//!
//! The gear at the foot of the rail opens the settings screen over the
//! window (see [`crate::settings::view`]), where the human sets Cox's
//! home: Cox's own chats opened from then on work there.
//!
//! A chat that comes to wait for the human, or whose turn ends, while the
//! human is not looking at it, tells them so (see [`crate::notices`]),
//! and the app's icon counts the chats that wait for them or that they
//! have not seen since a turn ended. A click on what a chat told shows it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use cox_core::goal::Goal;
use cox_core::list::List;
use cox_core::store::{ChatId, Named, ProjectId};
use cox_core::tabs::KeptTabs;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight, Image,
    KeyBinding, MouseButton, MouseDownEvent, PathPromptOptions, Pixels, Point, SharedString,
    Stateful, Subscription, Task, Window, actions, anchored, deferred, div, img, prelude::*, px,
    rgb,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::agent::mcp::ToolRequest;
use crate::agent::setup::{Setup, keeps_in_wiki};
use crate::agents::Agents;
use crate::chat::Stages;
use crate::chat::conversation::State;
use crate::chat::region::{self, AddRegionToChat, CancelRegion};
use crate::chat::sketch::{self, Sketch};
use crate::chat::view::{ChatView, OpenLink, Row, StageView};
use crate::daemon::Link;
use crate::daemon::wire::{ChatRow, FromDaemon, Listing, ProjectRow, StageAsk, ToDaemon};
use crate::notices::AppNotices;
use crate::settings::view::{Dismissed, HomeChanged, OpenSettings, SettingsView, UnpairPhones};

pub(crate) mod project_card;
mod vim;

use crate::stage::changes::ShowTurn;
use crate::stage::owners::ShowChat;
use crate::stage::{NewPage, NewTerminal, OpenFile, Stage};
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, app_icon, icon};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT, ACCENT_GROUND, ASKING, BORDER, BORDER_STRONG, DIM, DISABLED, FAINT, FOREGROUND, GAP,
    GROUND, HOVER, LINK, RAISED, SELECTED, SURFACE, UI_FONT, chat_color,
};
use crate::ui::{card, tip};
use crate::workflow;
pub use project_card::Gives;
use project_card::{Drawing, ProjectCard, Rename, SetAbout, Standing, ToIcon};

actions!(
    cox,
    [
        /// Opens a chat of Cox's own, in no project.
        NewChat,
        /// Makes a project with nothing set yet, and opens a chat in it.
        NewProject,
        NextChat,
        PreviousChat,
        /// Leaves the chat's name as it was.
        CancelRename,
        /// Folds the panel of the chat shown to the window's right edge, or
        /// unfolds it.
        ToggleChat,
    ]
);

/// How wide the rail of projects is, and how large and how round each
/// project's square on it is.
const RAIL_WIDTH: f32 = 76.;
const PROJECT_SIZE: f32 = 42.;
const PROJECT_RADIUS: f32 = 13.;

/// How wide the panel of the chat shown is, at the window's right, and
/// how wide it is folded to its chats' dots.
const PANEL_WIDTH: f32 = 420.;
const FOLDED_WIDTH: f32 = 48.;

/// How tall and how round each row of the panel's head and of the list
/// of chats is.
const ROW_HEIGHT: f32 = 34.;
const ROW_RADIUS: f32 = 11.;

/// The ground over the window while the settings are open: the window's
/// own, mostly opaque.
const BACKDROP: u32 = 0x0e0e_0fb8;

/// What the project of chats with no folder is called, until the daemon
/// says.
const COX: &str = "Cox";

/// The key context of the input that renames a chat.
pub(crate) const RENAMING: &str = "Renaming";

/// The key context of the rail, while Vim's keys have it.
pub(crate) const RAIL: &str = "Rail";

/// The key context of the list of chats, while Vim's keys have it.
pub(crate) const LIST: &str = "Chats";

/// Why a command the agent asks to run does not run, in a chat with no
/// stage.
const NO_TERMINAL: &str = "its terminal went away";

/// Why the agent sees nothing of a Mac window, or an Android device,
/// whose stage went before it answered.
const NO_TAB: &str = "the stage went away before it answered";

/// Binds the keys that open chats and move between them, and the one
/// that closes the card of a project.
pub fn bind_keys(cx: &mut App) {
    project_card::bind_keys(cx);
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewChat, Some("Workspace")),
        KeyBinding::new("secondary-shift-n", NewProject, Some("Workspace")),
        KeyBinding::new("secondary-,", OpenSettings, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextChat, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousChat, Some("Workspace")),
        // What the menu of the stage's + opens, on the stage of the chat
        // shown.
        KeyBinding::new("secondary-t", NewPage, Some("Workspace")),
        KeyBinding::new("ctrl-`", NewTerminal, Some("Workspace")),
        KeyBinding::new("secondary-p", OpenFile, Some("Workspace")),
        KeyBinding::new("secondary-\\", ToggleChat, Some("Workspace")),
        KeyBinding::new("escape", CancelRename, Some(RENAMING)),
    ]);
}

/// A folder the chats of a project work in. The chats themselves are in
/// the window's list, under it.
pub struct Project {
    /// How it is kept, or `None` for Cox's own, or a project that could
    /// not be kept.
    id: Option<ProjectId>,
    name: SharedString,
    folder: PathBuf,
    /// The key of the chat of it shown last, which choosing the project
    /// shows again.
    last: Option<u64>,
    /// The icon the human gave it, which the rail draws in place of a
    /// folder (see [`crate::project_icon`]).
    icon: Option<Arc<Image>>,
    /// Whether its folder is not there, as when the human moved it.
    missing: bool,
    /// Whether the human put it away, off the rail.
    archived: bool,
    /// Whether the human deleted it, while Cox runs.
    deleted: bool,
    /// What it is, as each chat's agent is told, if the human set it.
    about: Option<String>,
}

impl Project {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the human put it away, off the rail, and has not deleted
    /// it since.
    #[must_use]
    pub fn is_archived(&self) -> bool {
        self.archived && !self.deleted
    }

    /// The directory its chats work in.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Whether the human gave it an icon.
    #[must_use]
    pub const fn has_icon(&self) -> bool {
        self.icon.is_some()
    }
}

/// A chat in the list, as the daemon lists it, with its view once the
/// window has shown it.
#[expect(
    clippy::struct_excessive_bools,
    reason = "how a chat stands, each apart"
)]
pub struct Chat {
    /// What names it for this run, as it moves along the list, or out of
    /// it.
    key: u64,
    /// The index of its project.
    project: usize,
    /// How it is kept, or `None` when it could not be.
    kept: Option<ChatId>,
    name: SharedString,
    named: Named,
    /// Whether its agent runs.
    open: bool,
    /// Whether a line sent to it, closed, starts its agent again.
    again: bool,
    /// How the chat stands.
    state: State,
    /// Whether a turn ended while the chat was not shown, and it has not
    /// been shown since.
    unread: bool,
    /// What it works until, if the human set a goal.
    goal: Option<Goal>,
    /// The key of the chat whose agent opened it, if an agent did and
    /// that chat is in the list or archived.
    parent: Option<u64>,
    /// How its agent is set up, when the chat that opened it said, or
    /// once it became a sketch.
    setup: Option<Setup>,
    /// Whether it is a sketch on its way to an end.
    ending: bool,
    /// The id of the agent it talks to.
    agent: String,
    /// The folder it works in, when it is not its project's.
    folder: Option<PathBuf>,
    /// Its view, once the window showed it, or something asked of its
    /// stage.
    view: Option<Entity<ChatView>>,
}

impl Chat {
    /// The chat `row` lists, with `view`, if it has one.
    fn listed(row: ChatRow, view: Option<Entity<ChatView>>) -> Self {
        let ChatRow {
            key,
            project,
            kept,
            name,
            named,
            open,
            again,
            state,
            unread,
            goal,
            parent,
            setup,
            ending,
            agent,
            folder,
        } = row;
        Self {
            key,
            project,
            kept,
            name: name.into(),
            named,
            open,
            again,
            state,
            unread,
            goal,
            parent,
            setup,
            ending,
            agent,
            folder,
            view,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where its name came from.
    #[must_use]
    pub fn named(&self) -> Named {
        self.named
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// What names it for this run.
    #[must_use]
    pub fn key(&self) -> u64 {
        self.key
    }

    /// The index of its project.
    #[must_use]
    pub fn project(&self) -> usize {
        self.project
    }

    /// How it stands.
    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// Whether it is a sketch, working in a folder of its own.
    #[must_use]
    pub fn is_sketch(&self) -> bool {
        self.setup
            .as_ref()
            .is_some_and(|setup| setup.sketch.is_some())
    }

    /// Whether a turn ended while the chat was not shown, and it has not
    /// been shown since.
    #[must_use]
    pub fn is_unread(&self) -> bool {
        self.unread
    }

    /// What it works until, if the human set a goal.
    #[must_use]
    pub fn goal(&self) -> Option<&Goal> {
        self.goal.as_ref()
    }

    /// The number it is kept by, if it is kept.
    #[must_use]
    pub fn kept(&self) -> Option<ChatId> {
        self.kept
    }

    /// The key of the chat whose agent opened it, if an agent did.
    #[must_use]
    pub fn parent(&self) -> Option<u64> {
        self.parent
    }

    /// How the chat's agent is set up, when the chat that opened it said.
    #[must_use]
    pub fn setup(&self) -> Option<&Setup> {
        self.setup.as_ref()
    }

    /// The id of the agent it talks to.
    #[must_use]
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// Its view, once it has been shown.
    #[must_use]
    pub fn view(&self) -> Option<&Entity<ChatView>> {
        self.view.as_ref()
    }
}

/// A chat's stage, and what it hears of the agent's tools through.
struct Staged {
    /// Where the chat's agent's tools ask the stage, when the chat has a
    /// stage.
    tools: Option<UnboundedSender<ToolRequest>>,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "what the window shows, each apart"
)]
pub struct Workspace {
    /// The projects, in the order they were first opened, after no
    /// project, which comes first.
    projects: Vec<Project>,
    /// The chats, in the order of the list: under their projects, each
    /// after the ones opened before it; and which is shown, if any is.
    chats: List<Chat>,
    /// The chats the human archived, out of the list, in the order they
    /// were archived.
    archived: Vec<Chat>,
    /// Whether the archived chats are open, at the foot of the list of
    /// chats.
    archived_open: bool,
    /// Whether the list of chats is open, at the head of the panel.
    switching: bool,
    /// Whether the panel of the chat is folded to the window's right
    /// edge.
    folded: bool,
    /// The project chosen on the rail, whose chats the list shows: the
    /// project of the chat shown, or one with none shown.
    chosen: usize,
    /// How many chats the app's icon counts.
    badged: usize,
    /// The menu of the right button, where it opened in the window, for
    /// the chat at an index in the list.
    menu: Option<(Point<Pixels>, usize)>,
    /// The menu of the right button on the rail, where it opened in the
    /// window, for the project at an index among the projects.
    project_menu: Option<(Point<Pixels>, usize)>,
    /// The project of the menu whose "Delete project…" was clicked once,
    /// which a second click deletes, by its index among the projects.
    deleting: Option<usize>,
    /// The menu of the projects put away, where it opened in the window,
    /// while it is open.
    archived_projects: Option<Point<Pixels>>,
    /// The projects an agent draws an icon for now, by how they are kept.
    drawing: HashSet<ProjectId>,
    /// Why the agent drew no icon for a project, the last time it was
    /// asked, until it is asked again.
    not_drawn: HashMap<ProjectId, String>,
    /// The chat at an index in the list being renamed, in an input.
    renaming: Option<(usize, Entity<LineInput>, Subscription)>,
    /// Whether `--home` gave the home, where Cox's own chats work, for this
    /// run, ahead of what the settings keep.
    home_given: bool,
    /// The settings screen, while it is open.
    settings: Option<(Entity<SettingsView>, [Subscription; 3])>,
    /// The card where the human edits a project, while it is open.
    project_card: Option<(Entity<ProjectCard>, [Subscription; 4])>,
    /// The sheet over the window the human drags a region on, for the
    /// chat shown, while it is laid.
    region: Option<Sheet>,
    /// Where what the human does goes.
    link: Link,
    /// What makes the chats' stages.
    stages: Rc<dyn Stages>,
    /// The agents the chats can talk to.
    agents: Option<Agents>,
    /// The chats whose stages are made, by their keys.
    staged: HashMap<u64, Staged>,
    /// The stages made, by the folder whose chats work on each, with what
    /// opens the links of their conversations there.
    folders: HashMap<PathBuf, (StageView, OpenLink)>,
    /// The folders whose stages the daemon was asked for, to show with no
    /// chat, until they are made.
    asked_stages: HashSet<PathBuf>,
    /// What the daemon asked of the stages of chats whose stages are not
    /// made yet, by their keys, to carry out once they are.
    waiting: HashMap<u64, Vec<FromDaemon>>,
    /// The commands the stages run for the daemon, by their numbers.
    /// Dropping one lets go of its answer, which stops it.
    running: HashMap<u64, Task<()>>,
    /// What waits for the daemon to settle, by the number it was asked
    /// with.
    syncs: HashMap<u64, tokio::sync::oneshot::Sender<()>>,
    last_sync: u64,
    focus_handle: FocusHandle,
    /// The rail, and the list of chats, as Vim's keys select along them.
    rail: vim::Picker,
    list: vim::Picker,
    /// The line at the window's foot that Vim's `:` and `/` open, while
    /// Vim's keys are on.
    line: Option<Entity<crate::vim::line::Line>>,
    /// Show the chat of a stage's tab that the human asks a stage to.
    shows_chats: Vec<Subscription>,
    _from_daemon: Task<()>,
}

impl Workspace {
    /// A window with no chat open, which shows the projects and chats the
    /// daemon, reached through `link`, tells of in `from_daemon`, and
    /// tells the daemon what the human does. Cox's own chats work in
    /// `home`, which `--home` gave when `home_given` says so, until the
    /// daemon says. `stages` makes the chats' stages, and the chats talk
    /// to `agents`, when given.
    #[expect(clippy::too_many_arguments, reason = "what the window starts with")]
    pub fn new(
        link: Link,
        from_daemon: UnboundedReceiver<FromDaemon>,
        home: PathBuf,
        home_given: bool,
        stages: Rc<dyn Stages>,
        agents: Option<Agents>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The translation of what the human selects shows over the window.
        cx.observe_global::<crate::ui::translation::Shown>(|_, cx| cx.notify())
            .detach();
        // What the human hands the chat shown from elsewhere on the screen.
        cx.observe_global_in::<crate::chat::hand::Handed>(window, |workspace, window, cx| {
            workspace.take_handed(window, cx);
        })
        .detach();
        let mut from_daemon = from_daemon;
        let heard = cx.spawn_in(window, async move |this, cx| {
            while let Some(message) = from_daemon.recv().await {
                let heard = this.update_in(cx, |workspace, window, cx| {
                    workspace.hear(message, window, cx);
                });
                if heard.is_err() {
                    break;
                }
            }
        });
        Self {
            projects: vec![Project {
                id: None,
                name: COX.into(),
                folder: home,
                last: None,
                icon: None,
                missing: false,
                archived: false,
                deleted: false,
                about: None,
            }],
            chats: List::new(),
            archived: Vec::new(),
            archived_open: false,
            switching: false,
            folded: false,
            chosen: 0,
            badged: 0,
            menu: None,
            project_menu: None,
            deleting: None,
            archived_projects: None,
            drawing: HashSet::new(),
            not_drawn: HashMap::new(),
            renaming: None,
            home_given,
            settings: None,
            project_card: None,
            region: None,
            link,
            stages,
            agents,
            staged: HashMap::new(),
            folders: HashMap::new(),
            asked_stages: HashSet::new(),
            waiting: HashMap::new(),
            running: HashMap::new(),
            syncs: HashMap::new(),
            last_sync: 0,
            focus_handle: cx.focus_handle(),
            rail: vim::Picker::new(cx),
            list: vim::Picker::new(cx),
            line: crate::vim::mode(cx)
                .is_some()
                .then(|| cx.new(|cx| crate::vim::line::Line::new(window, cx))),
            shows_chats: Vec::new(),
            _from_daemon: heard,
        }
    }

    /// Tells the daemon what the human did.
    fn tell(&self, message: ToDaemon) {
        self.link.send(message);
    }

    /// Answers once the daemon has done all the window asked of it before,
    /// and the window has heard all it said of it.
    pub fn sync(&mut self) -> tokio::sync::oneshot::Receiver<()> {
        self.last_sync += 1;
        let (settled, heard) = tokio::sync::oneshot::channel();
        self.syncs.insert(self.last_sync, settled);
        self.tell(ToDaemon::Sync { id: self.last_sync });
        heard
    }

    /// Does what the daemon says.
    fn hear(&mut self, message: FromDaemon, window: &mut Window, cx: &mut Context<Self>) {
        match message {
            FromDaemon::Listing(listing) => self.list(listing, window, cx),
            FromDaemon::Patch { key, patch } => {
                if let Some(view) = self.chat(key).and_then(Chat::view) {
                    view.update(cx, |view, cx| view.apply(patch, cx));
                }
            }
            FromDaemon::Tabs { key, tabs } => self.make_stage(key, tabs, window, cx),
            FromDaemon::StageOf { folder, tabs } => {
                self.new_stage(folder, tabs, window, cx);
                self.sight(cx);
                cx.notify();
            }
            FromDaemon::Reveal { key } => self.reveal(key, window, cx),
            FromDaemon::Icon { project } => self.read_icon(project, cx),
            FromDaemon::News { key, name, body } => {
                let shown = self.shown().map(Chat::key) == Some(key);
                let seen = shown && window.is_window_active();
                if !seen && let Some(notices) = cx.try_global::<AppNotices>() {
                    notices.0.notify(key, &name, &body);
                }
            }
            asked @ (FromDaemon::Stage { .. } | FromDaemon::Prepare { .. }) => {
                self.ask_stage(asked, window, cx);
            }
            FromDaemon::Cancel { id } => {
                self.running.remove(&id);
            }
            FromDaemon::Settled { id } => {
                if let Some(settled) = self.syncs.remove(&id) {
                    let _ = settled.send(());
                }
            }
            FromDaemon::Unpaired { failed } => {
                if let Some((settings, _)) = &self.settings {
                    settings.update(cx, |settings, cx| settings.phones_unpaired(failed, cx));
                }
            }
            // The greeting is over before the window hears the daemon.
            FromDaemon::Joined
            | FromDaemon::Welcome
            | FromDaemon::Taken
            | FromDaemon::Outdated
            | FromDaemon::Busy => {}
        }
    }

    /// Shows the projects and chats as the daemon lists them now, each
    /// chat with the view it had, and the chat shown still shown, or, once
    /// it is out of the list, the one in its place.
    fn list(&mut self, listing: Listing, window: &mut Window, cx: &mut Context<Self>) {
        let Listing {
            projects,
            chats,
            archived,
            ..
        } = listing;
        self.list_projects(projects);
        self.project_menu = self
            .project_menu
            .filter(|(_, index)| self.projects.get(*index).is_some_and(|p| p.id.is_some()));
        let shown = self.chats.shown_index();
        let shown_key = self.shown().map(Chat::key);
        let menu = self
            .menu
            .and_then(|(at, index)| Some((at, self.chats.get(index)?.key)));
        let renaming = self
            .renaming
            .take()
            .and_then(|(index, input, subscription)| {
                Some((self.chats.get(index)?.key, input, subscription))
            });
        let mut views: HashMap<u64, Entity<ChatView>> = self
            .chats
            .iter_mut()
            .chain(&mut self.archived)
            .filter_map(|chat| Some((chat.key, chat.view.take()?)))
            .collect();
        let mut list = List::new();
        for row in chats {
            let view = views.remove(&row.key);
            list.push(Chat::listed(row, view), false);
        }
        self.chats = list;
        self.archived = archived
            .into_iter()
            .map(|row| {
                let view = views.remove(&row.key);
                Chat::listed(row, view)
            })
            .collect();
        self.menu = menu.and_then(|(at, key)| Some((at, self.index_of(key)?)));
        self.renaming = renaming.and_then(|(key, input, subscription)| {
            Some((self.index_of(key)?, input, subscription))
        });
        if self
            .projects
            .get(self.chosen)
            .is_none_or(|project| project.deleted)
        {
            self.chosen = 0;
        }
        // The chat shown stays shown, or the one in its place is.
        match (shown_key.and_then(|key| self.index_of(key)), shown) {
            (Some(index), _) => {
                self.chats.show(index);
            }
            (None, Some(was)) if !self.chats.is_empty() => {
                self.show(was.min(self.chats.len() - 1), window, cx);
            }
            (None, Some(_)) => {
                self.tell(ToDaemon::Shown { key: None });
                window.focus(&self.focus_handle);
            }
            (None, None) => {}
        }
        let rows: Vec<(Entity<ChatView>, Row)> = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter_map(|chat| Some((chat.view.clone()?, self.row_of(chat))))
            .collect();
        for (view, row) in rows {
            view.update(cx, |view, cx| view.follow_row(row, cx));
        }
        // What the daemon asked of the stage of a chat it had not listed
        // yet waits for the chat's view, which it gets now.
        let waiting: Vec<u64> = self.waiting.keys().copied().collect();
        for key in waiting {
            self.view(key, cx);
        }
        self.stage_chosen();
        self.follow_project_card(cx);
        self.sight(cx);
        self.at_work(cx);
        self.badge(cx);
        cx.notify();
    }

    /// Shows the projects as the daemon lists them now, `projects`, each
    /// with the chat of it shown last and its icon.
    fn list_projects(&mut self, projects: Vec<ProjectRow>) {
        let lasts: Vec<_> = self.projects.iter().map(|project| project.last).collect();
        self.forget_moved(&projects);
        // The icons read already, read again only when they change.
        let icons: HashMap<ProjectId, Option<Arc<Image>>> = self
            .projects
            .iter_mut()
            .filter_map(|project| Some((project.id?, project.icon.take())))
            .collect();
        self.projects = projects
            .into_iter()
            .enumerate()
            .map(|(index, row)| Project {
                id: row.id,
                name: row.name.into(),
                folder: row.folder,
                last: lasts.get(index).copied().flatten(),
                icon: row.id.and_then(|id| {
                    icons
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| crate::project_icon::read(id))
                }),
                missing: row.missing,
                archived: row.archived,
                deleted: row.deleted,
                about: row.about,
            })
            .collect();
    }

    /// Lets go of the stages of the projects whose folders moved, or that
    /// were deleted, as `projects` lists them now, with the chats' hold on
    /// them, so that each chat gets the stage of where its project is now,
    /// as the daemon tells it its tabs again.
    fn forget_moved(&mut self, projects: &[ProjectRow]) {
        // A project deleted is gone from where it was too.
        let moved: Vec<(usize, PathBuf)> = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, project)| project.id.is_some() && !project.deleted)
            .filter_map(|(index, project)| {
                let now = projects.get(index).filter(|row| row.id == project.id)?;
                (now.folder != project.folder || now.deleted)
                    .then(|| (index, project.folder.clone()))
            })
            .collect();
        for (index, from) in moved {
            let keys: Vec<u64> = self
                .chats
                .iter()
                .chain(&self.archived)
                .filter(|chat| chat.project == index && self.cwd_of(chat) == from)
                .map(|chat| chat.key)
                .collect();
            for key in keys {
                self.staged.remove(&key);
            }
            self.folders.remove(&from);
            self.asked_stages.remove(&from);
        }
    }

    /// How `chat` stands, as its view shows it.
    fn row_of(&self, chat: &Chat) -> Row {
        Row {
            open: chat.open,
            again: chat.again,
            agent: chat.agent.clone(),
            cwd: self.cwd_of(chat),
            gone: chat
                .setup
                .as_ref()
                .is_none_or(|setup| setup.folder.is_none())
                && chat.folder.is_none()
                && self.projects[chat.project].missing,
            setup: chat.setup.clone(),
        }
    }

    /// Where `chat` works: the folder of its sketch, or the folder of its
    /// project it keeps to, or its project's.
    /// What tells the stage of `folder` which files the agents of the
    /// chats working there wrote: of the chats whose conversations are
    /// read, as those the window showed.
    fn wrote_in(folder: PathBuf, cx: &Context<Self>) -> crate::stage::tree::Wrote {
        let workspace = cx.entity().downgrade();
        Rc::new(move |cx: &App| {
            let Some(workspace) = workspace.upgrade() else {
                return Vec::new();
            };
            let workspace = workspace.read(cx);
            workspace
                .chats
                .iter()
                .filter(|chat| workspace.cwd_of(chat) == folder)
                .filter_map(|chat| {
                    let view = chat.view.as_ref()?.read(cx);
                    let written = cox_core::entry::written(view.conversation().entries());
                    Some((chat.key, written))
                })
                .collect()
        })
    }

    fn cwd_of(&self, chat: &Chat) -> PathBuf {
        chat.setup
            .as_ref()
            .and_then(|setup| setup.folder.clone())
            .or_else(|| chat.folder.clone())
            .unwrap_or_else(|| self.projects[chat.project].folder.clone())
    }

    /// The index in the list of the chat `key`, if it is in the list.
    fn index_of(&self, key: u64) -> Option<usize> {
        self.chats.iter().position(|chat| chat.key == key)
    }

    fn chat_mut(&mut self, key: u64) -> Option<&mut Chat> {
        self.chats
            .iter_mut()
            .chain(&mut self.archived)
            .find(|chat| chat.key == key)
    }

    /// The view of the chat `key`, given it unless it had one, as a chat
    /// shown is, for what reads a chat the human does not see.
    pub fn watch(&mut self, key: u64, cx: &mut Context<Self>) -> Option<Entity<ChatView>> {
        self.view(key, cx);
        self.chat(key)?.view.clone()
    }

    /// Gives the chat `key` its view, unless it has one, and has the
    /// daemon tell it the conversation, and the tabs its stage had.
    fn view(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(chat) = self.chat(key) else {
            return;
        };
        if chat.view.is_some() {
            return;
        }
        let row = self.row_of(chat);
        let link = self.link.clone();
        let agents = self.agents.clone();
        let view = cx.new(|cx| ChatView::new(key, link, row, agents, cx));
        // The list of chats redraws as the chat changes.
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        if let Some(chat) = self.chat_mut(key) {
            chat.view = Some(view);
        }
        self.tell(ToDaemon::Watch { key });
    }

    /// Gives the chat `key` the stage of the folder it works in, unless it
    /// has one: the stage made already for another chat there, or one made
    /// on `tabs`, as the stage had them; and carries out what was asked of
    /// it until then.
    fn make_stage(
        &mut self,
        key: u64,
        tabs: Option<KeptTabs<u64>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.staged.contains_key(&key) {
            return;
        }
        let Some(chat) = self.chat(key) else {
            return;
        };
        let Some(view) = chat.view.clone() else {
            return;
        };
        let cwd = self.cwd_of(chat);
        if !self.folders.contains_key(&cwd) {
            self.new_stage(cwd.clone(), tabs, window, cx);
        }
        let shown_key = self.shown().map(Chat::key);
        let tools = self
            .folders
            .get(&cwd)
            .cloned()
            .and_then(|(shown, open_link)| {
                let stage = shown.view.clone().downcast::<Stage>().ok();
                view.update(cx, |view, cx| view.set_stage(shown, open_link, cx));
                // A stage works with the chat's agent: it carries out what the
                // agent's tools ask.
                let stage = stage?;
                let (tools, from_tools) = tokio::sync::mpsc::unbounded_channel();
                stage.update(cx, |stage, cx| {
                    stage.work(key, None, Some(from_tools), window, cx);
                    if shown_key == Some(key) {
                        stage.showing(key, cx);
                    }
                });
                Some(tools)
            });
        self.staged.insert(key, Staged { tools });
        self.sight(cx);
        for asked in self.waiting.remove(&key).unwrap_or_default() {
            self.ask_stage(asked, window, cx);
        }
    }

    /// Makes the stage of `folder`, on `tabs`, as the stage had them,
    /// which keeps its tabs, as those of its folder, names the chats, to
    /// hand a tab to one, and shows the chat of a tab the human asks it
    /// to.
    fn new_stage(
        &mut self,
        folder: PathBuf,
        tabs: Option<KeptTabs<u64>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.asked_stages.remove(&folder);
        if self.folders.contains_key(&folder) {
            return;
        }
        let Some((made, open_link)) = self.stages.stage(folder.clone(), tabs, window, cx) else {
            return;
        };
        self.folders
            .insert(folder.clone(), (made.clone(), open_link));
        let Ok(stage) = made.view.downcast::<Stage>() else {
            return;
        };
        let wrote = Self::wrote_in(folder.clone(), cx);
        let link = self.link.clone();
        let keep: crate::stage::Keep = Rc::new(move |tabs: &KeptTabs<u64>| {
            link.send(ToDaemon::Tabs {
                folder: folder.clone(),
                tabs: tabs.clone(),
            });
        });
        let workspace = cx.entity().downgrade();
        let names: crate::stage::owners::Names = Rc::new(move |key, cx: &App| {
            Some(workspace.upgrade()?.read(cx).chat(key)?.name.clone())
        });
        stage.update(cx, |stage, cx| {
            stage.set_names(names);
            stage.set_wrote(wrote, cx);
            stage.keep_with(keep, cx);
        });
        let shows = cx.subscribe_in(&stage, window, |workspace, _, ShowChat(key), window, cx| {
            workspace.reveal(*key, window, cx);
        });
        self.shows_chats.push(shows);
        let turns = cx.subscribe_in(
            &stage,
            window,
            |workspace, _, &ShowTurn { chat, entry }, window, cx| {
                workspace.show_turn(chat, entry, window, cx);
            },
        );
        self.shows_chats.push(turns);
        self.at_work(cx);
    }

    /// Asks the daemon for the stage of the project chosen, while no chat
    /// is shown, unless it is made or asked for already: the stage shows
    /// with no chat, as the home's does where Cox lands.
    fn stage_chosen(&mut self) {
        if self.shown().is_some() {
            return;
        }
        let Some(project) = self.projects.get(self.chosen) else {
            return;
        };
        let folder = project.folder.clone();
        if self.folders.contains_key(&folder) || !self.asked_stages.insert(folder.clone()) {
            return;
        }
        self.tell(ToDaemon::StageOf { folder });
    }

    /// The stage in the middle of the window: that of the folder the chat
    /// shown works in, or, with none shown, of the project chosen, once
    /// it is made.
    fn centered(&self) -> Option<&StageView> {
        let folder = match self.shown() {
            Some(chat) => self.cwd_of(chat),
            None => self.projects.get(self.chosen)?.folder.clone(),
        };
        self.folders.get(&folder).map(|(stage, _)| stage)
    }

    /// Tells each stage which chats' agents are at work, taking a turn,
    /// so that the others do not act on their pages.
    fn at_work(&self, cx: &mut App) {
        let working: HashSet<u64> = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter(|chat| {
                matches!(
                    chat.state,
                    State::Busy | State::Asking | State::Stopping | State::Checking
                )
            })
            .map(|chat| chat.key)
            .collect();
        for (stage, _) in self.folders.values() {
            if let Ok(stage) = stage.view.clone().downcast::<Stage>() {
                let working = working.clone();
                stage.update(cx, |stage, cx| stage.set_working(working, cx));
            }
        }
    }

    /// Tells each stage whether it is in sight now.
    fn sight(&self, cx: &mut App) {
        let centered = self.centered().map(|stage| stage.panel.clone());
        for (stage, _) in self.folders.values() {
            let in_sight = centered.as_ref() == Some(&stage.panel);
            stage.panel.update(cx, |panel, cx| panel.set(in_sight, cx));
        }
    }

    /// Tells the stage of the chat `key`, if it has one, that the chat is
    /// shown.
    fn stage_shows(&self, key: u64, cx: &mut Context<Self>) {
        if let Some(stage) = self
            .chat(key)
            .and_then(Chat::view)
            .and_then(|view| view.read(cx).stage())
        {
            stage.update(cx, |stage, cx| stage.showing(key, cx));
        }
    }

    /// Carries out what the daemon asks of a chat's stage: what the agent's
    /// tools ask, or a relay for an agent that starts; once the stage is
    /// made, when it is not yet.
    fn ask_stage(&mut self, asked: FromDaemon, window: &mut Window, cx: &mut Context<Self>) {
        let key = match &asked {
            FromDaemon::Stage { key, .. } | FromDaemon::Prepare { key, .. } => *key,
            _ => return,
        };
        if !self.staged.contains_key(&key) {
            self.waiting.entry(key).or_default().push(asked);
            self.view(key, cx);
            return;
        }
        let stage = self
            .chat(key)
            .and_then(Chat::view)
            .and_then(|view| view.read(cx).stage());
        match asked {
            FromDaemon::Prepare { id, .. } => {
                let relay = stage.as_ref().and_then(|_| self.stages.relay());
                let port = relay.as_ref().map(|relay| relay.port);
                if let (Some(stage), Some(relay)) = (stage, relay) {
                    stage.update(cx, |stage, cx| {
                        stage.work(key, Some(relay), None, window, cx);
                    });
                }
                self.tell(ToDaemon::Prepared { id, port });
            }
            FromDaemon::Stage { ask, .. } => {
                let request = self.request(ask, cx);
                match self
                    .staged
                    .get(&key)
                    .and_then(|staged| staged.tools.as_ref())
                {
                    Some(tools) => {
                        let _ = tools.send(request);
                    }
                    None => self.stages.unstaged(request),
                }
            }
            _ => {}
        }
    }

    /// What the daemon's `ask` asks of a stage, as the stage takes it: a
    /// command's answer goes back to the daemon once it ends.
    fn request(&mut self, ask: StageAsk, cx: &mut Context<Self>) -> ToolRequest {
        match ask {
            StageAsk::ShowMarkdown { path, text } => ToolRequest::ShowMarkdown { path, text },
            StageAsk::ShowFile { path, text, lines } => ToolRequest::ShowFile { path, text, lines },
            StageAsk::ShowChanges { folder } => ToolRequest::ShowChanges { folder },
            StageAsk::ShowMap { map, at } => ToolRequest::ShowMap { map, at },
            StageAsk::RunCommand { id, command, cwd } => {
                let (answer, ran) = tokio::sync::oneshot::channel();
                let link = self.link.clone();
                let waits = cx.spawn(async move |this, cx| {
                    let ran = ran.await.unwrap_or_else(|_| Err(NO_TERMINAL.to_owned()));
                    link.send(ToDaemon::Ran { id, ran });
                    let _ = this.update(cx, |workspace, _| workspace.running.remove(&id));
                });
                self.running.insert(id, waits);
                ToolRequest::RunCommand {
                    command,
                    cwd,
                    answer,
                    asks_human: false,
                }
            }
            StageAsk::MacWindow { id, act } => ToolRequest::MacWindow {
                act,
                answer: self.awaiting_sight(id, cx),
            },
            StageAsk::Android { id, act } => ToolRequest::Android {
                act,
                answer: self.awaiting_sight(id, cx),
            },
        }
    }

    /// Where what the agent sees goes, for the daemon's ask `id`: back to
    /// the daemon once the stage answers.
    fn awaiting_sight(
        &mut self,
        id: u64,
        cx: &mut Context<Self>,
    ) -> tokio::sync::oneshot::Sender<Result<crate::agent::mcp::Seen, String>> {
        let (answer, seen) = tokio::sync::oneshot::channel();
        let link = self.link.clone();
        let waits = cx.spawn(async move |this, cx| {
            let seen = seen.await.unwrap_or_else(|_| Err(NO_TAB.to_owned()));
            link.send(ToDaemon::Seen { id, seen });
            let _ = this.update(cx, |workspace, _| workspace.running.remove(&id));
        });
        self.running.insert(id, waits);
        answer
    }

    /// How many chats `chat` is under, in the list: none for a chat the
    /// human opened, one for a chat opened from such a chat, and so on.
    fn depth(&self, chat: &Chat) -> u8 {
        let mut depth = 0_u8;
        let mut parent = chat.parent;
        while let Some(key) = parent {
            let Some(found) = self.chats.iter().find(|other| other.key == key) else {
                break;
            };
            depth = depth.saturating_add(1);
            parent = found.parent;
        }
        depth
    }

    /// The projects, Cox's own first.
    #[must_use]
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    /// The chats of the project at `project`, in the order of the list.
    pub fn chats(&self, project: usize) -> impl Iterator<Item = &Chat> {
        self.chats
            .iter()
            .filter(move |chat| chat.project == project)
    }

    /// The index of the project chosen on the rail.
    #[must_use]
    pub fn chosen(&self) -> usize {
        self.chosen
    }

    /// The chat shown, if any is.
    #[must_use]
    pub fn shown(&self) -> Option<&Chat> {
        self.chats.shown()
    }

    /// Gives what the human handed to the chat shown, unfolding its panel
    /// for them to see it there; with no chat shown, it goes nowhere.
    fn take_handed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handed = crate::chat::hand::take(cx);
        if handed.is_empty() {
            return;
        }
        let Some(view) = self.shown().and_then(Chat::view).cloned() else {
            return;
        };
        self.folded = false;
        self.switching = false;
        view.update(cx, |view, cx| {
            for attachment in handed {
                view.attach(attachment, window, cx);
            }
        });
        cx.notify();
    }

    /// Whether the list of chats is open, at the head of the panel.
    #[must_use]
    pub fn is_switching(&self) -> bool {
        self.switching
    }

    /// Whether the panel of the chat is folded to the window's right
    /// edge.
    #[must_use]
    pub fn is_folded(&self) -> bool {
        self.folded
    }

    /// Does `act` on the stage in the middle of the window, which opens.
    fn on_stage(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Stage, &mut Window, &mut Context<Stage>),
    ) {
        if let Some(stage) = self
            .centered()
            .and_then(|stage| stage.view.clone().downcast::<Stage>().ok())
        {
            stage.update(cx, |stage, cx| act(stage, window, cx));
        }
    }

    /// The chats in the order of the list, each with how many chats it is
    /// under (see [`Self::depth`]).
    pub fn listed(&self) -> impl Iterator<Item = (&Chat, u8)> {
        self.chats.iter().map(|chat| (chat, self.depth(chat)))
    }

    /// The chat that goes by `key` for this run, in the list or archived.
    #[must_use]
    pub fn chat(&self, key: u64) -> Option<&Chat> {
        self.chats
            .iter()
            .chain(&self.archived)
            .find(|chat| chat.key == key)
    }

    /// The chats the human archived, in the order they were archived.
    #[must_use]
    pub fn archived(&self) -> &[Chat] {
        &self.archived
    }

    /// Opens a chat in the project of `folder`, or of Cox's own when there
    /// is none, and shows it.
    pub fn open(&mut self, folder: Option<PathBuf>, _: &mut Window, cx: &mut Context<Self>) {
        crate::vim::enter(crate::vim::Mode::Insert, cx);
        self.tell(ToDaemon::Open { folder });
    }

    /// Asks the human for a picture, and makes it the icon of the project
    /// at `index` (see [`crate::project_icon`]).
    pub fn choose_icon(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.project_menu = None;
        cx.notify();
        let Some(id) = self.projects.get(index).and_then(|project| project.id) else {
            return;
        };
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let from = match chosen.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.swap_remove(0),
                Ok(Err(error)) => {
                    tracing::error!("no picture chosen: {error:#}");
                    return;
                }
                _ => return,
            };
            let set = cx
                .background_spawn(async move { crate::project_icon::set(id, &from) })
                .await;
            if let Err(error) = set {
                tracing::error!("the project's icon was not set: {error:#}");
            }
            this.update(cx, |workspace, cx| workspace.read_icon(id, cx))
                .ok();
        })
        .detach();
    }

    /// Has the agent a new chat talks to read the project at `index` and
    /// draw it an icon, in the background, its square marked meanwhile
    /// (see [`crate::project_icon::generate`]).
    fn generate_icon(&mut self, index: usize, cx: &mut Context<Self>) {
        self.project_menu = None;
        cx.notify();
        let Some(project) = self.projects.get(index) else {
            return;
        };
        let (Some(id), Some(agents)) = (project.id, self.agents.clone()) else {
            return;
        };
        if !self.drawing.insert(id) {
            return;
        }
        self.not_drawn.remove(&id);
        let name = project.name.to_string();
        let folder = project.folder.clone();
        self.follow_project_card(cx);
        // The agent takes as long as it reads the project, which is no
        // work for the executor's own threads.
        let (drawn, heard) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("project-icon".to_owned())
            .spawn(move || {
                let _ = drawn.send(crate::project_icon::generate(&agents, id, &name, &folder));
            })
            .expect("a thread starts");
        cx.spawn(async move |this, cx| {
            let drawn = heard
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the drawing stopped")));
            this.update(cx, |workspace, cx| {
                workspace.drawing.remove(&id);
                if let Err(error) = drawn {
                    tracing::error!("no icon was drawn for the project: {error:#}");
                    workspace.not_drawn.insert(id, format!("{error:#}"));
                }
                workspace.read_icon(id, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Takes away the icon of the project at `index`, which the rail then
    /// draws as a folder.
    pub fn remove_icon(&mut self, index: usize, cx: &mut Context<Self>) {
        self.project_menu = None;
        let Some(id) = self.projects.get(index).and_then(|project| project.id) else {
            return;
        };
        if let Err(error) = crate::project_icon::remove(id) {
            tracing::error!("the project's icon was not removed: {error:#}");
        }
        self.read_icon(id, cx);
    }

    /// Puts the project at `index` away, off the rail, its chats going on
    /// as they were, or brings it back, choosing it; putting the project
    /// chosen away chooses Cox's own.
    pub fn archive_project(
        &mut self,
        index: usize,
        archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.project_menu = None;
        self.archived_projects = None;
        let Some(project) = self.projects.get_mut(index) else {
            return;
        };
        let Some(id) = project.id else {
            return;
        };
        project.archived = archived;
        self.tell(ToDaemon::ArchiveProject {
            project: id,
            archived,
        });
        if !archived {
            self.choose(index, window, cx);
        } else if self.chosen == index {
            self.choose(0, window, cx);
        }
        cx.notify();
    }

    /// Deletes the project at `index`, as the second click on "Delete
    /// project…" in its menu asks, unless one of its chats is at work: its
    /// folder goes to the trash, and its chats stop and are forgotten.
    pub fn delete_project(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.deleting != Some(index) {
            self.deleting = Some(index);
            cx.notify();
            return;
        }
        if self.project_at_work(index) {
            return;
        }
        self.project_menu = None;
        self.deleting = None;
        let Some(id) = self.projects.get(index).and_then(|project| project.id) else {
            return;
        };
        self.tell(ToDaemon::DeleteProject { project: id });
        if self.chosen == index {
            self.choose(0, window, cx);
        }
        cx.notify();
    }

    /// Whether a chat of the project at `index` is at work.
    fn project_at_work(&self, index: usize) -> bool {
        self.chats
            .iter()
            .chain(&self.archived)
            .any(|chat| chat.project == index && crate::daemon::working(chat.state))
    }

    /// Reads again the icon of the project kept as `id`, as it is now.
    fn read_icon(&mut self, id: ProjectId, cx: &mut Context<Self>) {
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == Some(id)) {
            project.icon = crate::project_icon::read(id);
        }
        self.follow_project_card(cx);
        cx.notify();
    }

    /// Makes a project with nothing set yet, in a folder of its own in
    /// the projects of Cox's home, and opens a chat in it, for the human
    /// and the agent to work out what the project is to be.
    pub fn new_project(&self) {
        self.tell(ToDaemon::NewProject);
    }

    /// Where Cox's own chats work: Cox's home.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.projects[0].folder
    }

    /// Has Cox's own chats opened from now on work in `home`, as
    /// `settings.json` names it now, unless `--home` gave the home for
    /// this run.
    pub fn follow_home(&mut self, home: PathBuf, _: &mut Context<Self>) {
        self.tell(ToDaemon::Home { home, set: false });
    }

    /// The settings screen, while it is open.
    #[must_use]
    pub fn settings(&self) -> Option<&Entity<SettingsView>> {
        self.settings.as_ref().map(|(view, _)| view)
    }

    /// Opens the settings screen over the window, with the keyboard in it.
    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.switching = false;
        let view = if let Some((view, _)) = &self.settings {
            view.clone()
        } else {
            let home = self.home().to_owned();
            let given = self.home_given;
            let view = cx.new(|cx| SettingsView::new(home, given, window, cx));
            let changed = cx.subscribe(&view, |workspace, _, HomeChanged(home), cx| {
                workspace.home_given = false;
                workspace.tell(ToDaemon::Home {
                    home: home.clone(),
                    set: true,
                });
                cx.notify();
            });
            let dismissed =
                cx.subscribe_in(&view, window, |workspace, _, Dismissed, window, cx| {
                    workspace.close_settings(window, cx);
                });
            let unpair = cx.subscribe(&view, |workspace, _, UnpairPhones, _| {
                workspace.tell(ToDaemon::UnpairPhones);
            });
            self.settings = Some((view.clone(), [changed, dismissed, unpair]));
            view
        };
        window.focus(&view.focus_handle(cx));
        cx.notify();
    }

    /// The card where the human edits a project, while it is open.
    #[must_use]
    pub fn project_card(&self) -> Option<&Entity<ProjectCard>> {
        self.project_card.as_ref().map(|(card, _)| card)
    }

    /// How the project at `index` stands, for its card.
    fn standing(&self, index: usize) -> Standing {
        let project = &self.projects[index];
        Standing {
            folder: project.folder.clone(),
            missing: project.missing,
            icon: project.icon.clone(),
            about: project.about.clone(),
            drawing: match project.id {
                _ if self.agents.is_none() => Drawing::Unable,
                Some(id) if self.drawing.contains(&id) => Drawing::Now,
                Some(id) => self
                    .not_drawn
                    .get(&id)
                    .map_or(Drawing::Ready, |why| Drawing::NotDrawn(why.clone())),
                None => Drawing::Unable,
            },
            at_work: self
                .chats
                .iter()
                .chain(&self.archived)
                .any(|chat| chat.project == index && crate::daemon::working(chat.state)),
        }
    }

    /// Opens the card of the project at `index` over the window, with the
    /// keyboard in its input.
    pub fn open_project_card(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.project_menu = None;
        self.menu = None;
        self.switching = false;
        let Some(project) = self.projects.get(index) else {
            return;
        };
        let Some(id) = project.id else {
            return;
        };
        let standing = self.standing(index);
        let card = cx.new(|cx| ProjectCard::new(id, standing, window, cx));
        let renamed = cx.subscribe(&card, move |workspace, _, Rename(name), _| {
            workspace.tell(ToDaemon::RenameProject {
                project: id,
                name: name.clone(),
            });
        });
        let told = cx.subscribe(&card, move |workspace, _, SetAbout(about), _| {
            workspace.tell(ToDaemon::SetAbout {
                project: id,
                about: about.clone(),
            });
        });
        let dismissed = cx.subscribe_in(
            &card,
            window,
            |workspace, _, project_card::Dismissed, window, cx| {
                workspace.close_project_card(window, cx);
            },
        );
        let iconed = cx.subscribe_in(
            &card,
            window,
            move |workspace, _, to: &ToIcon, window, cx| {
                let Some(index) = workspace
                    .projects
                    .iter()
                    .position(|project| project.id == Some(id))
                else {
                    return;
                };
                match to {
                    ToIcon::Choose => workspace.choose_icon(index, window, cx),
                    ToIcon::Generate => workspace.generate_icon(index, cx),
                    ToIcon::Remove => workspace.remove_icon(index, cx),
                }
            },
        );
        window.focus(&card.focus_handle(cx));
        self.project_card = Some((card, [renamed, told, dismissed, iconed]));
        cx.notify();
    }

    /// Closes the card of the project, renaming it as the human typed in
    /// it, and giving the keyboard back to the chat shown.
    pub fn close_project_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((card, _)) = self.project_card.take() {
            let card = card.read(cx);
            let project = card.project();
            if let Some(SetAbout(about)) = card.told(cx) {
                self.tell(ToDaemon::SetAbout { project, about });
            }
            if let Some(Rename(name)) = card.renamed(cx) {
                self.tell(ToDaemon::RenameProject { project, name });
            }
            window.focus(&self.focus_handle(cx));
            cx.notify();
        }
    }

    /// Has the card of the project follow it as the daemon lists it now,
    /// or closes it once the project is gone.
    fn follow_project_card(&mut self, cx: &mut Context<Self>) {
        let Some((card, _)) = &self.project_card else {
            return;
        };
        let card = card.clone();
        let id = card.read(cx).project();
        let Some(index) = self
            .projects
            .iter()
            .position(|project| project.id == Some(id))
        else {
            self.project_card = None;
            return;
        };
        let standing = self.standing(index);
        card.update(cx, |card, cx| card.follow(standing, cx));
    }

    /// Closes the settings screen, giving the keyboard back to the chat
    /// shown.
    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((view, subscriptions)) = self.settings.take() {
            view.update(cx, SettingsView::save_all);
            // The screen, and what hears it, stay until what keeping told,
            // as a home set, has reached the window.
            cx.defer(move |_| drop((view, subscriptions)));
            window.focus(&self.focus_handle(cx));
            cx.notify();
        }
    }

    /// Shows the chat at `index` in the list, in the panel, unfolded,
    /// with the keyboard in its input, and the stage of its folder.
    pub fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.chats.show(index) {
            return;
        }
        self.switching = false;
        self.folded = false;
        let chat = &mut self.chats[index];
        let key = chat.key;
        self.chosen = chat.project;
        self.projects[chat.project].last = Some(key);
        self.tell(ToDaemon::Shown { key: Some(key) });
        self.view(key, cx);
        self.stage_shows(key, cx);
        self.sight(cx);
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Folds the panel of the chat to the window's right edge, or unfolds
    /// it, with the keyboard back in the chat's input.
    fn fold(&mut self, folded: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.folded = folded;
        self.switching = false;
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Counts on the app's icon the chats that wait for the human, or that
    /// they have not seen since a turn ended, and tells the stages' pages
    /// how the chats stand (see [`crate::stage::activity`]).
    fn badge(&mut self, cx: &App) {
        crate::stage::activity::tell(crate::stage::activity::Told {
            chats: self
                .chats
                .iter()
                .chain(&self.archived)
                .map(|chat| crate::stage::activity::Chat {
                    key: chat.key,
                    name: chat.name.to_string(),
                    cwd: self.cwd_of(chat),
                    project: self.projects[chat.project].folder.clone(),
                    asking: chat.state == State::Asking,
                    unread: chat.unread,
                    goal: chat.goal.as_ref().map(Goal::input),
                })
                .collect(),
            projects: self
                .projects
                .iter()
                .map(|project| (project.folder.clone(), project.name.to_string()))
                .collect(),
            home: self.projects[0].folder.clone(),
        });
        let count = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter(|chat| chat.unread || chat.state == State::Asking)
            .count();
        if count != self.badged {
            self.badged = count;
            if let Some(notices) = cx.try_global::<AppNotices>() {
                notices.0.badge(count);
            }
        }
    }

    /// Shows the chat `key` names, bringing it back to the list if it was
    /// archived, as a click on what it told the human does.
    pub fn reveal(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.index_of(key) {
            self.show(index, window, cx);
        } else if self.archived.iter().any(|chat| chat.key == key) {
            self.tell(ToDaemon::Archive {
                key,
                archived: false,
            });
        }
    }

    /// Shows the chat kept as `chat` at the entry `entry`, the line that
    /// started a turn, as a commit made in the turn was clicked.
    pub fn show_turn(
        &mut self,
        chat: i64,
        entry: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self
            .chats
            .iter()
            .chain(&self.archived)
            .find(|shown| shown.kept.map(ChatId::number) == Some(chat))
            .map(|shown| shown.key)
        else {
            return;
        };
        self.reveal(key, window, cx);
        if let Some(view) = self.watch(key, cx) {
            view.update(cx, |view, cx| view.show_entry(entry, cx));
        }
    }

    /// Shows the chat at `index`, and opens an input in place of its name
    /// at the head of the panel, for the human to rename it.
    pub fn rename(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.chats.shown_index() != Some(index) {
            self.show(index, window, cx);
        }
        self.switching = false;
        let Some(chat) = self.chats.get(index) else {
            return;
        };
        let name = chat.name.clone();
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_text(&name, cx);
            input.select_whole(cx);
            input
        });
        let submitted = cx.subscribe_in(&input, window, |workspace, _, Submitted, window, cx| {
            workspace.end_renaming(true, window, cx);
        });
        window.focus(&input.focus_handle(cx));
        self.menu = None;
        self.renaming = Some((index, input, submitted));
        cx.notify();
    }

    /// Closes the input that renames a chat, naming the chat what it holds
    /// when `keep` says to and it holds a name.
    fn end_renaming(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((index, input, _)) = self.renaming.take() else {
            return;
        };
        let name = input.read(cx).text().trim().to_owned();
        if keep
            && !name.is_empty()
            && let Some(chat) = self.chats.iter_mut().nth(index)
        {
            // Named at once, as the daemon will.
            chat.name = name.clone().into();
            chat.named = Named::Human;
            let key = chat.key;
            self.tell(ToDaemon::Rename { key, name });
        }
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Shows the chat at `index`, and opens the input that sets its goal.
    pub fn edit_goal(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.show(index, window, cx);
        if let Some(view) = self.chats.get(index).and_then(Chat::view) {
            view.update(cx, |view, cx| view.edit_goal(window, cx));
        }
    }

    /// Shows the chat at `index`, and opens the input that makes it a
    /// sketch, of what the human says it is to try.
    pub fn edit_sketch(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if !self.starts_sketch(index) {
            return;
        }
        self.show(index, window, cx);
        let Some(chat) = self.chats.get(index) else {
            return;
        };
        let from = sketch::from_at_first(&self.projects[chat.project].folder);
        if let Some(view) = chat.view() {
            view.update(cx, |view, cx| view.edit_sketch(from, window, cx));
        }
    }

    /// Shows the chat at `index`, and asks its agent, in the human's
    /// place, to keep what the chat learned in the human's wiki, as their
    /// workflow says to (see [`workflow::WIKI_SAVE`]).
    pub fn save_to_wiki(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if !self.saves_to_wiki(index) {
            return;
        }
        self.show(index, window, cx);
        if let Some(chat) = self.chats.get(index) {
            self.tell(ToDaemon::SaveToWiki { key: chat.key });
        }
        cx.notify();
    }

    /// Asks the agent of the chat at `index`, in the human's place, to
    /// merge what it made, the human being done looking at it.
    pub fn merge(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if !self.merges(index) {
            return;
        }
        self.show(index, window, cx);
        if let Some(chat) = self.chats.get(index) {
            self.tell(ToDaemon::Merge { key: chat.key });
        }
        cx.notify();
    }

    /// Whether the human can ask the agent of the chat at `index` to
    /// merge what it made: it is not at work, nor a sketch's.
    fn merges(&self, index: usize) -> bool {
        self.chats.get(index).is_some_and(|chat| {
            !crate::daemon::working(chat.state) && crate::daemon::merges(chat.setup.as_ref())
        })
    }

    /// Whether the human can ask the agent of the chat at `index` to keep
    /// what it learned in the wiki: it is not at work, and has the tool.
    fn saves_to_wiki(&self, index: usize) -> bool {
        self.chats.get(index).is_some_and(|chat| {
            !crate::daemon::working(chat.state) && keeps_in_wiki(chat.setup.as_ref())
        })
    }

    /// Whether the human can open a sketch beside the chat at `index`: it
    /// is not a sketch itself.
    fn starts_sketch(&self, index: usize) -> bool {
        self.chats.get(index).is_some_and(|chat| {
            chat.setup
                .as_ref()
                .is_none_or(|setup| setup.sketch.is_none())
        })
    }

    /// Shows the chat at `index`, and clears its goal.
    pub fn clear_goal(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.show(index, window, cx);
        if let Some(chat) = self.chats.get(index) {
            self.tell(ToDaemon::SetGoal {
                key: chat.key,
                goal: None,
            });
        }
    }

    /// The sketch the chat at `index` is, what it started from and the
    /// folder it works in, while the folder is there.
    fn sketch_of(&self, index: usize) -> Option<(Sketch, PathBuf)> {
        let setup = self.chats.get(index)?.setup.as_ref()?;
        let folder = setup.folder.clone().filter(|folder| folder.exists())?;
        Some((setup.sketch?, folder))
    }

    /// Whether the sketch at `index` can end: its folder is there, and was
    /// not opened as a project, which the human goes on with, and it is
    /// not on its way to an end already.
    fn ends_sketch(&self, index: usize) -> bool {
        self.chats.get(index).is_some_and(|chat| !chat.ending)
            && self.sketch_of(index).is_some_and(|(_, folder)| {
                !self.projects.iter().any(|project| project.folder == folder)
            })
    }

    /// The ways the sketch at `index` can end, as the human's workflow
    /// says them (see [`workflow::sketch`]), in the order the menu offers
    /// them; none, unless it can end now.
    #[must_use]
    pub fn endings(&self, index: usize) -> Vec<workflow::Ending> {
        let Some((from, _)) = self.sketch_of(index).filter(|_| self.ends_sketch(index)) else {
            return Vec::new();
        };
        workflow::sketch()
            .endings
            .into_iter()
            .filter(|ending| ending.ends(from))
            .collect()
    }

    /// Shows the sketch at `index`, and ends it as `ending` says (see
    /// [`crate::daemon`]).
    pub fn end_sketch_as(
        &mut self,
        index: usize,
        ending: workflow::Ending,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let ends = self.ends_sketch(index)
            && self
                .sketch_of(index)
                .is_some_and(|(from, _)| ending.ends(from));
        if !ends {
            return;
        }
        self.show(index, window, cx);
        if let Some(chat) = self.chats.get(index) {
            self.tell(ToDaemon::EndSketch {
                key: chat.key,
                ending,
            });
        }
    }

    /// Takes the chat at `index` out of the list, to "Archived" at its
    /// foot. Its agent, if it runs, goes on.
    pub fn archive(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.renaming = None;
        if let Some(chat) = self.chats.get(index) {
            let key = chat.key;
            // Its tabs are the human's from now on.
            if let Some(stage) = chat.view().and_then(|view| view.read(cx).stage()) {
                stage.update(cx, |stage, cx| stage.release(key, cx));
            }
            self.tell(ToDaemon::Archive {
                key,
                archived: true,
            });
        }
        cx.notify();
    }

    /// Brings the archived chat at `index` among the archived back to the
    /// list, and shows it.
    pub fn unarchive(&mut self, index: usize, _: &mut Window, _: &mut Context<Self>) {
        if let Some(chat) = self.archived.get(index) {
            self.tell(ToDaemon::Archive {
                key: chat.key,
                archived: false,
            });
        }
    }

    /// Shows the chat `step` places along the list, going round.
    fn step(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.chats.along(step) {
            self.show(index, window, cx);
        }
    }

    /// Chooses the project at `index` on the rail, showing the chat of it
    /// shown last, or its first, or no chat when it has none.
    pub fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.projects.get(index) else {
            return;
        };
        let mut chats = self
            .chats
            .iter()
            .enumerate()
            .filter(|(_, chat)| chat.project == index);
        let last = project.last;
        let first = chats.clone().next().map(|(chat, _)| chat);
        let shown = chats
            .find(|(_, chat)| Some(chat.key) == last)
            .map(|(chat, _)| chat)
            .or(first);
        if let Some(chat) = shown {
            self.show(chat, window, cx);
        } else {
            self.chosen = index;
            self.chats.hide();
            self.switching = false;
            self.stage_chosen();
            self.sight(cx);
            self.tell(ToDaemon::Shown { key: None });
            window.focus(&self.focus_handle);
            cx.notify();
        }
    }

    /// The rail down the window's left side: a button for each project,
    /// which chooses it, marked when it is the one chosen and counting its
    /// chats that wait for the human, and at its foot the button that
    /// opens a folder, and the gear that opens the settings.
    fn render_rail(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Cox's own, a place of its own rather than a project, stands apart
        // from the projects under it, past a line.
        // A project put away, or deleted, is off the rail.
        let shown = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, project)| !project.archived && !project.deleted);
        let picked = self.rail.picked(shown.clone().count(), window);
        let projects = shown
            .enumerate()
            .flat_map(|(row, (index, project))| {
                let apart = project.id.is_none().then(|| {
                    div()
                        .flex_shrink_0()
                        .w(px(PROJECT_SIZE * 0.6))
                        .h(px(1.))
                        .bg(rgb(BORDER_STRONG))
                        .into_any_element()
                });
                let button = self
                    .render_project(index, project, cx)
                    .when(picked == Some(row), |button| {
                        button.child(crate::ui::picked(PROJECT_RADIUS))
                    });
                std::iter::once(button.into_any_element()).chain(apart)
            })
            .collect::<Vec<_>>();
        div()
            .id("projects")
            // With Vim's keys only: a click would take the keyboard here.
            .when(crate::vim::mode(cx).is_some(), |rail| {
                rail.track_focus(&self.rail.focus).key_context(RAIL)
            })
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .py(px(16.))
            .overflow_y_scroll()
            .children(projects)
            .child(div().flex_1())
            .children(self.render_archived_projects_button(cx))
            .child(
                div()
                    .id("new-project")
                    .debug_selector(|| "new-project".to_owned())
                    .flex_shrink_0()
                    .size(px(PROJECT_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(PROJECT_RADIUS))
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(BORDER_STRONG))
                    .text_color(rgb(FAINT))
                    .text_size(px(18.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                    .tooltip(tip("New project"))
                    .child(icon(Icon::Plus, 16., FAINT))
                    .on_click(cx.listener(|workspace, _, _, _| {
                        workspace.new_project();
                    })),
            )
            .child(self.render_settings_button(cx))
    }

    /// The button at the foot of the rail that opens the menu of the
    /// projects put away, while there are any, lit while it is open.
    fn render_archived_projects_button(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let count = self
            .projects
            .iter()
            .filter(|project| project.archived && !project.deleted)
            .count();
        if count == 0 {
            return None;
        }
        let open = self.archived_projects.is_some();
        let ink = if open { FOREGROUND } else { FAINT };
        Some(
            div()
                .id("archived-projects")
                .debug_selector(|| "archived-projects".to_owned())
                .flex_shrink_0()
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .cursor_pointer()
                .when(open, |button| button.bg(rgb(SELECTED)))
                .hover(|style| style.bg(rgb(HOVER)))
                // Not over its menu, while that is open.
                .when(!open, |button| {
                    button.tooltip(tip(format!("Archived projects ({count})")))
                })
                .child(icon(Icon::Inbox, 18., ink))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|workspace, event: &MouseDownEvent, _, cx| {
                        workspace.archived_projects = match workspace.archived_projects {
                            Some(_) => None,
                            None => Some(event.position),
                        };
                        cx.stop_propagation();
                        cx.notify();
                    }),
                ),
        )
    }

    /// The menu of the projects put away, if it is open: a click on one
    /// brings it back to the rail, and chooses it.
    fn render_archived_projects(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let at = self.archived_projects?;
        let rows = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, project)| project.archived && !project.deleted)
            .map(|(index, project)| {
                crate::ui::menu::item(("archived-project", index), project.name.clone(), "")
                    .tooltip(tip(format!(
                        "{}\nBring it back to the rail",
                        shortened(&project.folder)
                    )))
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.archive_project(index, false, window, cx);
                    }))
            });
        let menu = crate::ui::menu::menu()
            .min_w(px(160.))
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| {
                workspace.archived_projects = None;
                cx.notify();
            }))
            .children(rows);
        // It opens upward, from the foot of the rail.
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Corner::BottomLeft)
                    .position(at)
                    .child(menu),
            )
            .with_priority(2),
        )
    }

    /// The gear at the foot of the rail, which opens the settings, lit
    /// while they are open.
    fn render_settings_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let open = self.settings.is_some();
        let key = if cfg!(target_os = "macos") {
            "⌘,"
        } else {
            "Ctrl+,"
        };
        let ink = if open { FOREGROUND } else { FAINT };
        div()
            .id("open-settings")
            .debug_selector(|| "open-settings".to_owned())
            .flex_shrink_0()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .cursor_pointer()
            .when(open, |button| button.bg(rgb(SELECTED)))
            .hover(|style| style.bg(rgb(HOVER)))
            .tooltip(tip(format!("Settings ({key})")))
            .child(icon(Icon::Settings, 18., ink))
            .on_click(cx.listener(|workspace, _, window, cx| {
                workspace.open_settings(window, cx);
            }))
    }

    /// The settings screen, over the window and dimming it, while it is
    /// open: a click beside it closes it.
    fn render_settings(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (view, _) = self.settings.as_ref()?;
        Some(
            deferred(
                div()
                    .id("settings-backdrop")
                    .occlude()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(GAP * 2.))
                    .bg(gpui::rgba(BACKDROP))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|workspace, _, window, cx| {
                            workspace.close_settings(window, cx);
                        }),
                    )
                    .child(view.clone()),
            )
            .with_priority(3),
        )
    }

    /// The card of a project, over the window and dimming it, while it is
    /// open: a click beside it closes it.
    fn render_project_card(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (card, _) = self.project_card.as_ref()?;
        Some(
            deferred(
                div()
                    .id("project-card-backdrop")
                    .occlude()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(GAP * 2.))
                    .bg(gpui::rgba(BACKDROP))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|workspace, _, window, cx| {
                            workspace.close_project_card(window, cx);
                        }),
                    )
                    .child(card.clone()),
            )
            .with_priority(3),
        )
    }

    /// The button of the project at `index` on the rail: its icon, or a
    /// folder, in a square, and its name under it, or Cox's icon alone for
    /// Cox's own, with its folder where the mouse rests, after its name
    /// for Cox's own. The right button opens its menu, but on Cox's own.
    fn render_project(
        &self,
        index: usize,
        project: &Project,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let chosen = self.chosen == index;
        let chats = self.chats.iter().filter(|chat| chat.project == index);
        let asking = chats
            .clone()
            .filter(|chat| chat.state == State::Asking)
            .count();
        let waiting = chats
            .filter(|chat| chat.unread || chat.state == State::Asking)
            .count();
        let label = if chosen { LINK } else { FAINT };
        let folder = shortened(&project.folder);
        let home = project.id.is_none();
        let drawing = project.id.is_some_and(|id| self.drawing.contains(&id));
        div()
            .id(("project", index))
            .debug_selector(move || format!("project-{}", index + 1))
            .relative()
            .flex_shrink_0()
            .w(px(RAIL_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(5.))
            .cursor_pointer()
            // Not over its menu, while that is open.
            .when(self.project_menu.is_none(), |button| {
                button.tooltip(tip(if home {
                    format!("{} ({folder})", project.name)
                } else if project.missing {
                    format!("{folder}\nThis folder is not there: Edit project… says where it went.")
                } else if drawing {
                    format!("{folder}\nAn agent draws its icon…")
                } else if let Some(why) = project.id.and_then(|id| self.not_drawn.get(&id)) {
                    format!("{folder}\nNo icon was drawn: {why}")
                } else {
                    folder
                }))
            })
            .on_click(cx.listener(move |workspace, _, window, cx| {
                workspace.choose(index, window, cx);
            }))
            .when(!home, |button| {
                button.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                        workspace.project_menu = Some((event.position, index));
                        workspace.deleting = None;
                        cx.notify();
                    }),
                )
            })
            .when(chosen, |button| {
                button.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(10.))
                        .w(px(3.))
                        .h(px(22.))
                        .rounded_r(px(2.))
                        .bg(rgb(ACCENT)),
                )
            })
            // The badges are the square's siblings, not its children: gpui
            // paints a div's border over its children, and the square's
            // would cross them.
            .child(
                div()
                    .relative()
                    .size(px(PROJECT_SIZE))
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .map(|square| project_square(square, project, home, chosen, drawing)),
                    )
                    .when(waiting > 0, |square| {
                        square.child(waiting_badge(index, waiting, asking > 0))
                    })
                    .when(project.missing, |square| square.child(missing_badge(index))),
            )
            .when(!home, |button| {
                button.child(
                    div()
                        .max_w(px(RAIL_WIDTH - 8.))
                        .overflow_hidden()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(label))
                        .child(ellipsis(project.name.clone())),
                )
            })
    }

    /// The panel of the chat shown, at the window's right: at its head,
    /// the button that opens the list of the project's chats, naming the
    /// chat shown, and the button that folds the panel; under it, the pill
    /// of the agent and the settings of the chat; then its conversation,
    /// under the list while it is open.
    fn render_panel(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                .child(if self.chats(self.chosen).next().is_some() {
                    "Choose a chat from the list above."
                } else {
                    "No chat here yet: the list above opens one."
                })
                .into_any_element(),
        };
        card()
            .debug_selector(|| "chat-panel".to_owned())
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
                            .on_click(cx.listener(|workspace, _, window, cx| {
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
                .on_click(cx.listener(move |workspace, _, window, cx| {
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
            .on_click(cx.listener(
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
                    .on_click(cx.listener(move |workspace, _, window, cx| {
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
            .on_click(cx.listener(move |workspace, _, window, cx| {
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
    fn render_folded(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .on_click(cx.listener(move |workspace, _, window, cx| {
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
                    .on_click(cx.listener(|workspace, _, window, cx| {
                        workspace.fold(false, window, cx);
                    })),
            )
            .children(dots)
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

/// What the square of `project` on the rail shows: Cox's icon for Cox's
/// own, the home's, or else the icon the human gave the project, filling
/// the square inside its edge, or else a folder; the edge marks it
/// chosen, and, dashed in the accent, that an agent draws it an icon.
fn project_square(square: Div, project: &Project, home: bool, chosen: bool, drawing: bool) -> Div {
    let (ground, edge, ink) = if chosen {
        (ACCENT_GROUND, ACCENT, ACCENT)
    } else {
        (SURFACE, BORDER, FAINT)
    };
    if home {
        return square.child(app_icon(PROJECT_SIZE));
    }
    let square = square
        .rounded(px(PROJECT_RADIUS))
        .border_1()
        .border_color(rgb(edge))
        .when(drawing, |square| {
            square.border_dashed().border_color(rgb(ACCENT))
        })
        .when(project.missing, |square| {
            square
                .border_dashed()
                .border_color(rgb(ASKING))
                .opacity(0.7)
        });
    match project.icon.clone() {
        Some(picture) => square.child(
            img(picture)
                .size(px(PROJECT_SIZE - 2.))
                .rounded(px(PROJECT_RADIUS - 1.)),
        ),
        None => square
            .bg(rgb(ground))
            .when(!chosen, |square| square.hover(|style| style.bg(rgb(HOVER))))
            .child(icon(Icon::Folder, 18., ink)),
    }
}

/// The mark over the bottom right of the square of the project at
/// `index`, on the rail, whose folder is not there.
fn missing_badge(index: usize) -> Div {
    badge(ASKING, px(11.), "?")
        .debug_selector(move || format!("project-{}-missing", index + 1))
        .bottom(px(-5.))
        .right(px(-5.))
}

/// The count over the top right of the project's square, on the rail, at
/// `index`: how many of its chats wait for the human or that they have
/// not seen, in amber when one of them `asks`.
fn waiting_badge(index: usize, waiting: usize, asks: bool) -> Div {
    badge(
        if asks { ASKING } else { ACCENT },
        px(10.),
        waiting.to_string(),
    )
    .debug_selector(move || format!("project-{}-waiting", index + 1))
    .top(px(-5.))
    .right(px(-5.))
}

/// A badge over a corner of a project's square, placed by the caller: a
/// disc in `color`, or a pill when `label` is wider than a digit, ringed
/// in the ground so that it stands off the square.
///
/// The ring is a disc of the ground under the colored one, not a border:
/// gpui smooths the edges of a border and of the background under it
/// apart, and the background's color would show as a thin halo outside
/// the ring.
fn badge(color: u32, size: Pixels, label: impl Into<SharedString>) -> Div {
    div()
        .absolute()
        .p(px(2.))
        .rounded_full()
        .bg(rgb(GROUND))
        .child(
            div()
                .min_w(px(13.))
                .h(px(13.))
                .px(px(3.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(color))
                .text_color(rgb(GROUND))
                .text_size(size)
                .font_weight(FontWeight::BOLD)
                .child(label.into()),
        )
}

impl Workspace {
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
            .on_click(cx.listener(|workspace, _, window, cx| {
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
                    .on_click(cx.listener(move |workspace, _, window, cx| {
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

    /// The menu of the right button on a project of the rail, if it is
    /// open: to edit it, to put it away, and to delete it.
    fn render_project_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (at, index) = self.project_menu?;
        self.projects.get(index)?;
        // Deleting takes a second click, so that one by mistake does not.
        let delete = match self.deleting {
            Some(deleting) if deleting == index && self.project_at_work(index) => {
                "A chat of it is at work: delete it once the turn ends"
            }
            Some(deleting) if deleting == index => "Click again to move it to the Trash",
            _ => "Delete project…",
        };
        let item = |id: &'static str, label: &'static str| crate::ui::menu::item(id, label, "");
        let menu = crate::ui::menu::menu()
            .min_w(px(160.))
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| {
                workspace.project_menu = None;
                cx.notify();
            }))
            .child(item("edit-project", "Edit project…").on_click(cx.listener(
                move |workspace, _, window, cx| workspace.open_project_card(index, window, cx),
            )))
            .child(
                item("archive-project", "Archive project").on_click(cx.listener(
                    move |workspace, _, window, cx| {
                        workspace.archive_project(index, true, window, cx);
                    },
                )),
            )
            .child(item("delete-project", delete).on_click(cx.listener(
                move |workspace, _, window, cx| {
                    workspace.delete_project(index, window, cx);
                },
            )));
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }

    /// The menu of the right button on a chat, if it is open.
    fn render_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
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
            .child(item("rename-chat", "Rename").on_click(
                cx.listener(move |workspace, _, window, cx| workspace.rename(index, window, cx)),
            ))
            .child(item("set-goal", "Set goal…").on_click(cx.listener(
                move |workspace, _, window, cx| {
                    workspace.edit_goal(index, window, cx);
                },
            )))
            .when(has_goal, |menu| {
                menu.child(item("clear-goal", "Clear goal").on_click(cx.listener(
                    move |workspace, _, window, cx| workspace.clear_goal(index, window, cx),
                )))
            })
            .when(starts_sketch, |menu| {
                menu.child(item("start-sketch", "Start sketch…").on_click(cx.listener(
                    move |workspace, _, window, cx| workspace.edit_sketch(index, window, cx),
                )))
            })
            .when(saves_to_wiki, |menu| {
                menu.child(item("save-to-wiki", "Save to wiki").on_click(cx.listener(
                    move |workspace, _, window, cx| workspace.save_to_wiki(index, window, cx),
                )))
            })
            .when(merges, |menu| {
                menu.child(item("merge", "Merge").on_click(
                    cx.listener(move |workspace, _, window, cx| workspace.merge(index, window, cx)),
                ))
            })
            .children(endings.into_iter().enumerate().map(|(at, ending)| {
                crate::ui::menu::item(("sketch-ending", at), ending.name.clone(), "").on_click(
                    cx.listener(move |workspace, _, window, cx| {
                        workspace.end_sketch_as(index, ending.clone(), window, cx);
                    }),
                )
            }))
            .child(item("archive-chat", "Archive").on_click(
                cx.listener(move |workspace, _, window, cx| workspace.archive(index, window, cx)),
            ));
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}

impl Focusable for Workspace {
    /// The settings' input while they are open, or else the shown chat's
    /// input, or the window itself while none is open, or the chat is
    /// folded away.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if let Some((settings, _)) = &self.settings {
            return settings.focus_handle(cx);
        }
        if let Some((card, _)) = &self.project_card {
            return card.focus_handle(cx);
        }
        if self.folded {
            return self.focus_handle.clone();
        }
        match self.shown().and_then(Chat::view) {
            Some(view) => view.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The parts mark again where they are drawn, for a region dragged.
        region::forget_marks();
        // The stage of the chat shown, or an empty card until it has one.
        let stage = card()
            .flex_1()
            .min_w_0()
            .h_full()
            .map(|card| match self.centered() {
                Some(stage) => card
                    .debug_selector(|| "stage-panel".to_owned())
                    .child(stage.view.clone()),
                None => card,
            });
        let chat = if self.folded {
            self.render_folded(cx).into_any_element()
        } else {
            self.render_panel(window, cx).into_any_element()
        };
        div()
            .track_focus(&self.focus_handle)
            .key_context("Workspace")
            // GPUI still hands a key an interceptor stopped to the first key
            // listener on the focus path. This empty one is first, so the
            // conversation's, which answers the agent's options, never sees it.
            .capture_key_down(|_, _, _| {})
            .on_action(cx.listener(|workspace, _: &NewChat, window, cx| {
                workspace.open(None, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &NewProject, _, _| {
                workspace.new_project();
            }))
            .on_action(cx.listener(|workspace, _: &OpenSettings, window, cx| {
                workspace.open_settings(window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &NextChat, window, cx| {
                workspace.step(1, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &PreviousChat, window, cx| {
                workspace.step(-1, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &NewPage, window, cx| {
                workspace.on_stage(window, cx, Stage::new_tab);
            }))
            .on_action(cx.listener(|workspace, _: &NewTerminal, window, cx| {
                workspace.on_stage(window, cx, Stage::new_terminal);
            }))
            .on_action(cx.listener(|workspace, _: &ToggleChat, window, cx| {
                workspace.fold(!workspace.folded, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &OpenFile, window, cx| {
                workspace.on_stage(window, cx, Stage::new_files);
            }))
            .on_action(cx.listener(|workspace, _: &AddRegionToChat, window, cx| {
                workspace.lay_sheet(window, cx);
            }))
            .on_action(cx.listener(Self::open_line))
            .on_action(cx.listener(Self::run))
            .relative()
            .size_full()
            .flex()
            .bg(rgb(GROUND))
            .text_color(rgb(FOREGROUND))
            .font_family(UI_FONT)
            .text_size(px(14.))
            .child(self.render_rail(window, cx))
            // The stage, and the chat at its right, as cards on the ground
            // the rail is on.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .gap(px(GAP))
                    .py(px(GAP))
                    .pr(px(GAP))
                    .child(stage)
                    .child(chat),
            )
            .children(self.render_menu(cx))
            .children(self.render_project_menu(cx))
            .children(self.render_archived_projects(cx))
            .children(self.render_project_card(cx))
            .children(self.render_settings(cx))
            .children(self.render_sheet(cx))
            .children(
                cx.try_global::<crate::ui::translation::Shown>()
                    .and_then(|shown| shown.0.clone()),
            )
            .children(self.line.clone())
    }
}

/// The sheet over the window the human drags a region on: where the drag
/// started, if it has, and where the mouse is.
struct Sheet {
    from: Option<Point<Pixels>>,
    to: Point<Pixels>,
    focus: FocusHandle,
}

impl Workspace {
    /// Lays the sheet over the window, for the human to drag a region of
    /// it for the chat shown, when a chat is shown.
    fn lay_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown().and_then(Chat::view).is_none() {
            return;
        }
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.region = Some(Sheet {
            from: None,
            to: window.mouse_position(),
            focus,
        });
        cx.notify();
    }

    /// Takes the sheet away, with no region.
    fn lift_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.region.take().is_some() {
            self.give_back_keys(window, cx);
            cx.notify();
        }
    }

    /// Gives the keyboard to the input of the chat shown, as after the
    /// sheet is lifted.
    fn give_back_keys(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.shown().and_then(Chat::view) {
            window.focus(&view.read(cx).input().focus_handle(cx));
        }
    }

    /// The drag ended at `at`: the region it covers goes to the chat
    /// shown, once its picture is taken, if it is big enough to be one.
    fn drop_sheet(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sheet) = self.region.take() else {
            return;
        };
        self.give_back_keys(window, cx);
        cx.notify();
        let Some(region) = sheet.from.and_then(|from| region::between(from, at)) else {
            return;
        };
        let described = region::described(region, cx);
        region::take(region, described, window, cx);
    }

    /// The sheet, while it is laid: the window dimmed but for the region
    /// dragged, and what to do on it.
    fn render_sheet(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let sheet = self.region.as_ref()?;
        let dragged = sheet.from.and_then(|from| region::between(from, sheet.to));
        let hint = format!(
            "Drag over what to hand the chat · Escape gives up · {}",
            region::keys()
        );
        Some(
            div()
                .id("region-sheet")
                .debug_selector(|| "region-sheet".to_owned())
                .key_context(region::KEY_CONTEXT)
                .track_focus(&sheet.focus)
                .on_action(cx.listener(|workspace, _: &CancelRegion, window, cx| {
                    workspace.lift_sheet(window, cx);
                }))
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .occlude()
                .cursor_crosshair()
                .bg(gpui::rgba(0x0000_0040))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|workspace, event: &MouseDownEvent, _, cx| {
                        if let Some(sheet) = &mut workspace.region {
                            sheet.from = Some(event.position);
                            sheet.to = event.position;
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_move(
                    cx.listener(|workspace, event: &gpui::MouseMoveEvent, _, cx| {
                        if let Some(sheet) = &mut workspace.region {
                            sheet.to = event.position;
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|workspace, event: &gpui::MouseUpEvent, window, cx| {
                        workspace.drop_sheet(event.position, window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|workspace, _, window, cx| workspace.lift_sheet(window, cx)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(12.))
                        .left_0()
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .px(px(12.))
                                .py(px(6.))
                                .rounded(px(8.))
                                .bg(rgb(RAISED))
                                .border_1()
                                .border_color(rgb(BORDER_STRONG))
                                .text_size(px(12.))
                                .text_color(rgb(FAINT))
                                .child(hint),
                        ),
                )
                .children(dragged.map(|region| {
                    div()
                        .absolute()
                        .left(region.origin.x)
                        .top(region.origin.y)
                        .w(region.size.width)
                        .h(region.size.height)
                        .border_1()
                        .border_color(rgb(ACCENT))
                        .bg(gpui::rgba(0xffff_ff14))
                })),
        )
    }
}
