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
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Image, ImageFormat, KeyBinding,
    PathPromptOptions, Pixels, Point, SharedString, Subscription, Task, Window, actions,
    prelude::*,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::agent::mcp::ToolRequest;
use crate::agent::setup::Setup;
use crate::chat::Stages;
use crate::chat::conversation::State;
use crate::chat::sketch::{self, Sketch};
use crate::chat::view::{ChatView, OpenLink, Row, StageView};
use crate::daemon::Link;
use crate::daemon::wire::{ChatRow, FromDaemon, Listing, ProjectRow, ToDaemon};
use crate::notices::AppNotices;
use crate::settings::view::{Dismissed, HomeChanged, OpenSettings, SettingsView, UnpairPhones};
use coxd::agents::Agents;

pub(crate) mod project_card;
mod render;
mod render_panel;
mod render_rail;
mod sheet;
mod stages;
mod vim;

use sheet::Sheet;

use crate::stage::{NewPage, NewTerminal, OpenFile};
use crate::ui::input::{LineInput, Submitted};
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

/// The icon of the project kept as `id`, for GPUI to draw, if it has one
/// (see [`crate::project_icon`]).
fn icon_of(id: ProjectId) -> Option<Arc<Image>> {
    let bytes = std::fs::read(crate::project_icon::path(id)?).ok()?;
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
}

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
    /// The line at the window's foot that Vim's `:` and `/` open.
    line: Entity<crate::vim::line::Line>,
    /// What the stage's card and the chat's hold, for the edge of the one
    /// with the keyboard.
    stage_card: FocusHandle,
    chat_card: FocusHandle,
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
            line: cx.new(|cx| crate::vim::line::Line::new(window, cx)),
            stage_card: cx.focus_handle(),
            chat_card: cx.focus_handle(),
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
                icon: row
                    .id
                    .and_then(|id| icons.get(&id).cloned().unwrap_or_else(|| icon_of(id))),
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
            project.icon = icon_of(id);
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
                .filter(|project| !project.deleted)
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
        crate::vim::came(crate::vim::Region::Renaming, window, cx);
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
            crate::vim::came(crate::vim::Region::Goal, window, cx);
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
            crate::vim::came(crate::vim::Region::Sketch, window, cx);
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
        self.chats
            .get(index)
            .is_some_and(|chat| crate::daemon::saves_to_wiki(chat.state, chat.setup.as_ref()))
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
        crate::daemon::sketch_of(self.chats.get(index)?.setup.as_ref())
    }

    /// Whether the sketch at `index` can end: its folder is there, and was
    /// not opened as a project, which the human goes on with, and it is
    /// not on its way to an end already.
    fn ends_sketch(&self, index: usize) -> bool {
        self.chats.get(index).is_some_and(|chat| {
            crate::daemon::ends_sketch(chat.setup.as_ref(), chat.ending, |folder| {
                self.projects
                    .iter()
                    .any(|project| !project.deleted && project.folder == folder)
            })
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
