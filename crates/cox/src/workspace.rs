//! The window: the projects on a rail down its left side, the chats of
//! the project chosen in tabs along its top, and the one shown under
//! them. A project is a folder the human opened: each chat in it works
//! there, and its agent gets the human's instructions, then those in the
//! folder's `COX.md` (see [`crate::agent::instructions`]). Chats in no
//! project work in Cox's home, with the instructions there.
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
//! A chat's tab tells whether its agent is at work, waits for the human,
//! or finished a turn the human has not seen yet, and whether it has a
//! goal, and the project's button on the rail counts its chats that wait
//! for the human. The menu of the right button on a tab renames the chat,
//! as a double click does, sets or clears its goal, ends a sketch (see
//! [`crate::chat::sketch`]), or archives it: an archived chat leaves the
//! list for "Archived" at the end of the tabs, from where a click brings
//! it back.
//!
//! The gear at the foot of the rail opens the settings screen over the
//! window (see [`crate::settings::view`]), where the human sets Cox's
//! home: the chats in no project opened from then on work there.
//!
//! A chat that comes to wait for the human, or whose turn ends, while the
//! human is not looking at it, tells them so (see [`crate::notices`]),
//! and the app's icon counts the chats that wait for them or that they
//! have not seen since a turn ended. A click on what a chat told shows it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cox_core::goal::Goal;
use cox_core::list::List;
use cox_core::store::{ChatId, Named, ProjectId};
use cox_core::tabs::KeptTabs;
use gpui::{
    App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, MouseDownEvent, PathPromptOptions, Pixels, Point, SharedString, Stateful,
    Subscription, Task, Window, actions, anchored, deferred, div, prelude::*, px, rgb,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::agent::mcp::ToolRequest;
use crate::agent::setup::{Setup, keeps_in_wiki};
use crate::agents::Agents;
use crate::chat::Stages;
use crate::chat::conversation::State;
use crate::chat::sketch::{self, Sketch};
use crate::chat::view::{ChatView, OpenLink, Row, StageView};
use crate::daemon::Link;
use crate::daemon::wire::{ChatRow, FromDaemon, Listing, StageAsk, ToDaemon};
use crate::notices::AppNotices;
use crate::settings::view::{Dismissed, HomeChanged, OpenSettings, SettingsView};
use crate::stage::{NewPage, NewTerminal, OpenFile, Stage};
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT, ACCENT_GROUND, ASKING, BORDER, BORDER_STRONG, DIM, DISABLED, FAINT, FOREGROUND, GAP,
    GROUND, HOVER, LINK, RAISED, SELECTED, SURFACE, UI_FONT,
};
use crate::ui::{card, tip};
use crate::workflow;

actions!(
    cox,
    [
        /// Opens a chat in no project.
        NewChat,
        /// Asks for a folder, and opens a chat in its project.
        OpenFolder,
        NextChat,
        PreviousChat,
        /// Leaves the chat's name as it was.
        CancelRename,
        /// Opens the stage of the chat shown, or closes it.
        ToggleStage,
    ]
);

/// How wide the rail of projects is, and how large and how round each
/// project's square on it is.
const RAIL_WIDTH: f32 = 76.;
const PROJECT_SIZE: f32 = 42.;
const PROJECT_RADIUS: f32 = 13.;

/// How tall the row of tabs is, and how tall, how wide at most and how
/// round each tab is.
const TABS_HEIGHT: f32 = 40.;
const TAB_HEIGHT: f32 = 34.;
const TAB_WIDTH: f32 = 240.;
const TAB_RADIUS: f32 = 11.;

/// The ground over the window while the settings are open: the window's
/// own, mostly opaque.
const BACKDROP: u32 = 0x0e0e_0fb8;

/// What the project of chats with no folder is called, until the daemon
/// says.
const NO_PROJECT: &str = "No project";

/// The key context of the input that renames a chat.
const RENAMING: &str = "Renaming";

/// Why a command the agent asks to run does not run, in a chat with no
/// stage.
const NO_TERMINAL: &str = "its terminal went away";

/// Binds the keys that open chats and move between them.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewChat, Some("Workspace")),
        KeyBinding::new("secondary-o", OpenFolder, Some("Workspace")),
        KeyBinding::new("secondary-,", OpenSettings, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextChat, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousChat, Some("Workspace")),
        // What the menu of the stage's + opens, on the stage of the chat
        // shown.
        KeyBinding::new("secondary-t", NewPage, Some("Workspace")),
        KeyBinding::new("ctrl-`", NewTerminal, Some("Workspace")),
        KeyBinding::new("secondary-p", OpenFile, Some("Workspace")),
        KeyBinding::new("secondary-\\", ToggleStage, Some("Workspace")),
        KeyBinding::new("escape", CancelRename, Some(RENAMING)),
    ]);
}

/// A folder the chats of a project work in. The chats themselves are in
/// the window's list, under it.
pub struct Project {
    /// How it is kept, or `None` for no project, or a project that could
    /// not be kept.
    id: Option<ProjectId>,
    name: SharedString,
    folder: PathBuf,
    /// The key of the chat of it shown last, which choosing the project
    /// shows again.
    last: Option<u64>,
}

impl Project {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The directory its chats work in.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
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
    /// Whether the menu of the archived chats is open.
    archived_open: bool,
    /// The project chosen on the rail, whose chats the tabs show: the
    /// project of the chat shown, or one with none shown.
    chosen: usize,
    /// How many chats the app's icon counts.
    badged: usize,
    /// The menu of the right button, where it opened in the window, for
    /// the chat at an index in the list.
    menu: Option<(Point<Pixels>, usize)>,
    /// The chat at an index in the list being renamed, in an input.
    renaming: Option<(usize, Entity<LineInput>, Subscription)>,
    /// Whether `--home` gave the home, the folder of no project, for this
    /// run, ahead of what the settings keep.
    home_given: bool,
    /// The settings screen, while it is open.
    settings: Option<(Entity<SettingsView>, [Subscription; 2])>,
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
    _from_daemon: Task<()>,
}

impl Workspace {
    /// A window with no chat open, which shows the projects and chats the
    /// daemon, reached through `link`, tells of in `from_daemon`, and
    /// tells the daemon what the human does. Chats in no project work in
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
                name: NO_PROJECT.into(),
                folder: home,
                last: None,
            }],
            chats: List::new(),
            archived: Vec::new(),
            archived_open: false,
            chosen: 0,
            badged: 0,
            menu: None,
            renaming: None,
            home_given,
            settings: None,
            link,
            stages,
            agents,
            staged: HashMap::new(),
            folders: HashMap::new(),
            waiting: HashMap::new(),
            running: HashMap::new(),
            syncs: HashMap::new(),
            last_sync: 0,
            focus_handle: cx.focus_handle(),
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
            FromDaemon::Reveal { key } => self.reveal(key, window, cx),
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
            // The greeting is over before the window hears the daemon.
            FromDaemon::Welcome | FromDaemon::Taken | FromDaemon::Outdated => {}
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
        } = listing;
        let lasts: Vec<_> = self.projects.iter().map(|project| project.last).collect();
        self.projects = projects
            .into_iter()
            .enumerate()
            .map(|(index, row)| Project {
                id: row.id,
                name: row.name.into(),
                folder: row.folder,
                last: lasts.get(index).copied().flatten(),
            })
            .collect();
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
        if self.chosen >= self.projects.len() {
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
        self.badge(cx);
        cx.notify();
    }

    /// How `chat` stands, as its view shows it.
    fn row_of(&self, chat: &Chat) -> Row {
        Row {
            open: chat.open,
            again: chat.again,
            agent: chat.agent.clone(),
            cwd: self.cwd_of(chat),
        }
    }

    /// Where `chat` works: the folder of its sketch, or its project's.
    fn cwd_of(&self, chat: &Chat) -> PathBuf {
        chat.setup
            .as_ref()
            .and_then(|setup| setup.folder.clone())
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
        // The tabs redraw as the chat changes.
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
        let made = match self.folders.get(&cwd) {
            Some((shown, open_link)) => Some((shown.clone(), open_link.clone(), false)),
            None => self
                .stages
                .stage(cwd.clone(), tabs, window, cx)
                .map(|(shown, open_link)| {
                    self.folders
                        .insert(cwd.clone(), (shown.clone(), open_link.clone()));
                    (shown, open_link, true)
                }),
        };
        let shown_key = self.shown().map(Chat::key);
        let tools = made.and_then(|(shown, open_link, new)| {
            let stage = shown.view.clone().downcast::<Stage>().ok();
            view.update(cx, |view, cx| view.set_stage(shown, open_link, cx));
            // A stage works with the chat's agent: it carries out what the
            // agent's tools ask. A new one keeps its tabs, as those of its
            // folder.
            let stage = stage?;
            let (tools, from_tools) = tokio::sync::mpsc::unbounded_channel();
            let keep = new.then(|| {
                let link = self.link.clone();
                let keep: crate::stage::Keep = Rc::new(move |tabs: &KeptTabs<u64>| {
                    link.send(ToDaemon::Tabs {
                        folder: cwd.clone(),
                        tabs: tabs.clone(),
                    });
                });
                keep
            });
            stage.update(cx, |stage, cx| {
                stage.work(key, None, Some(from_tools), keep, window, cx);
                if shown_key == Some(key) {
                    stage.showing(key, cx);
                }
            });
            Some(tools)
        });
        self.staged.insert(key, Staged { tools });
        for asked in self.waiting.remove(&key).unwrap_or_default() {
            self.ask_stage(asked, window, cx);
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
                        stage.work(key, Some(relay), None, None, window, cx);
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
        }
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

    /// The projects, no project first.
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

    /// Does `act` on the stage of the chat shown, which opens.
    fn on_stage(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Stage, &mut Window, &mut Context<Stage>),
    ) {
        if let Some(view) = self.shown().and_then(|chat| chat.view.clone()) {
            view.update(cx, |view, cx| view.on_stage(window, cx, act));
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

    /// Opens a chat in the project of `folder`, or in no project when there
    /// is none, and shows it.
    pub fn open(&mut self, folder: Option<PathBuf>, _: &mut Window, _: &mut Context<Self>) {
        self.tell(ToDaemon::Open { folder });
    }

    /// Asks the human for a folder, and opens a chat in its project.
    pub fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let folder = match chosen.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.swap_remove(0),
                Ok(Err(error)) => {
                    tracing::error!("no folder chosen: {error:#}");
                    return;
                }
                _ => return,
            };
            this.update_in(cx, |workspace, window, cx| {
                workspace.open(Some(folder), window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Where chats in no project work: Cox's home.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.projects[0].folder
    }

    /// Has the chats in no project opened from now on work in `home`, as
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
        self.archived_open = false;
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
            self.settings = Some((view.clone(), [changed, dismissed]));
            view
        };
        window.focus(&view.focus_handle(cx));
        cx.notify();
    }

    /// Closes the settings screen, giving the keyboard back to the chat
    /// shown.
    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((view, _)) = self.settings.take() {
            view.update(cx, SettingsView::save_all);
            window.focus(&self.focus_handle(cx));
            cx.notify();
        }
    }

    /// Shows the chat at `index` in the list, with the keyboard in its
    /// input.
    pub fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.chats.show(index) {
            return;
        }
        let chat = &mut self.chats[index];
        let key = chat.key;
        self.chosen = chat.project;
        self.projects[chat.project].last = Some(key);
        self.tell(ToDaemon::Shown { key: Some(key) });
        self.view(key, cx);
        self.stage_shows(key, cx);
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Counts on the app's icon the chats that wait for the human, or that
    /// they have not seen since a turn ended.
    fn badge(&mut self, cx: &App) {
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

    /// Opens an input in place of the name of the chat at `index`, for the
    /// human to rename it.
    pub fn rename(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
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
            self.tell(ToDaemon::Archive {
                key: chat.key,
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
            self.tell(ToDaemon::Shown { key: None });
            window.focus(&self.focus_handle);
            cx.notify();
        }
    }

    /// The rail down the window's left side: a button for each project,
    /// which chooses it, marked when it is the one chosen and counting its
    /// chats that wait for the human, and at its foot the button that
    /// opens a folder, and the gear that opens the settings.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let projects = self
            .projects
            .iter()
            .enumerate()
            .map(|(index, project)| self.render_project(index, project, cx));
        div()
            .id("projects")
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
            .child(
                div()
                    .id("open-folder")
                    .debug_selector(|| "open-folder".to_owned())
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
                    .tooltip(tip("Open folder…"))
                    .child(icon(Icon::Plus, 16., FAINT))
                    .on_click(cx.listener(|workspace, _, window, cx| {
                        workspace.choose_folder(window, cx);
                    })),
            )
            .child(self.render_settings_button(cx))
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

    /// The button of the project at `index` on the rail: a folder in a
    /// square, or `~` for no project, and its name under it, with its
    /// folder where the mouse rests.
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
        let (ground, edge, ink, label) = if chosen {
            (ACCENT_GROUND, ACCENT, ACCENT, LINK)
        } else {
            (SURFACE, BORDER, FAINT, FAINT)
        };
        let folder = shortened(&project.folder);
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
            .tooltip(tip(folder))
            .on_click(cx.listener(move |workspace, _, window, cx| {
                workspace.choose(index, window, cx);
            }))
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
            .child(
                div()
                    .relative()
                    .size(px(PROJECT_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(PROJECT_RADIUS))
                    .bg(rgb(ground))
                    .border_1()
                    .border_color(rgb(edge))
                    .text_color(rgb(ink))
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(!chosen, |square| square.hover(|style| style.bg(rgb(HOVER))))
                    .map(|square| match project.id {
                        None => square.child("~"),
                        Some(_) => square.child(icon(Icon::Folder, 18., ink)),
                    })
                    .when(waiting > 0, |square| {
                        square.child(
                            div()
                                .debug_selector(move || format!("project-{}-waiting", index + 1))
                                .absolute()
                                .top(px(-5.))
                                .right(px(-5.))
                                .min_w(px(17.))
                                .h(px(17.))
                                .px(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(9.))
                                .border_2()
                                .border_color(rgb(GROUND))
                                .bg(rgb(if asking > 0 { ASKING } else { ACCENT }))
                                .text_color(rgb(GROUND))
                                .text_size(px(10.))
                                .font_weight(FontWeight::BOLD)
                                .child(waiting.to_string()),
                        )
                    }),
            )
            .child(
                div()
                    .max_w(px(RAIL_WIDTH - 8.))
                    .overflow_hidden()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(label))
                    .child(ellipsis(project.name.clone())),
            )
    }

    /// The tabs of the chats of the project chosen, along the top of the
    /// window, with the button that opens another chat in it, "Archived"
    /// at their end, if the human archived a chat, and at the right the
    /// pill of the agent and the settings of the chat shown.
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = self.chosen;
        let folder = self.projects[chosen].folder.clone();
        let tabs = self
            .chats
            .iter()
            .enumerate()
            .filter(|(_, chat)| chat.project == chosen)
            .map(|(index, chat)| self.render_tab(index, chat, cx))
            .collect::<Vec<_>>();
        div()
            .relative()
            .h(px(TABS_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .id("chats")
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(
                div()
                    .id("new-chat-in")
                    .debug_selector(move || format!("new-chat-in-{}", chosen + 1))
                    .flex_shrink_0()
                    .size(px(TAB_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(TAB_RADIUS))
                    .text_color(rgb(FAINT))
                    .text_size(px(16.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                    .tooltip(tip("New chat here"))
                    .child("+")
                    .on_click({
                        let folder = folder.clone();
                        cx.listener(move |workspace, _, window, cx| {
                            workspace.open(Some(folder.clone()), window, cx);
                        })
                    }),
            )
            .child(div().flex_1())
            .children(self.render_archived(cx))
            // The agent and the settings of the chat shown, at the right,
            // and the button that opens its stage.
            .children(
                self.shown()
                    .and_then(Chat::view)
                    .and_then(|view| view.update(cx, |view, cx| view.render_settings(cx))),
            )
            .children(
                self.shown()
                    .and_then(Chat::view)
                    .and_then(|view| view.update(cx, |view, cx| view.render_stage_toggle(cx))),
            )
    }

    /// The tab of the chat at `index` in the list, which numbers its chats
    /// from 1: a dot of how it stands, its name, or the input that renames
    /// it, and whether it is a sketch and has a goal.
    fn render_tab(&self, index: usize, chat: &Chat, cx: &mut Context<Self>) -> Stateful<Div> {
        let number = index + 1;
        let shown = self.chats.shown_index() == Some(index);
        let sketch = chat
            .setup
            .as_ref()
            .is_some_and(|setup| setup.sketch.is_some());
        let state = chat.open.then_some(chat.state);
        // Filled, the dot asks the human to look; a ring, it says only
        // whether the agent is at work. A closed chat has none.
        let dot = match state {
            Some(State::Asking) => Some((ASKING, true, "asking")),
            _ if chat.unread => Some((ACCENT, true, "unread")),
            Some(State::Busy | State::Stopping | State::Checking) => {
                Some((ACCENT, false, "working"))
            }
            Some(State::Idle) => Some((DISABLED, false, "idle")),
            Some(State::Closed) | None => None,
        };
        let name = self.render_tab_name(index, chat, cx);
        // A chat an agent opened comes after the chat that opened it.
        let opened = chat.parent.is_some() && self.depth(chat) > 0;
        let small = |text: &'static str, selector: String| {
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .debug_selector(move || selector.clone())
                .child(text)
        };
        div()
            .id(("chat", number))
            .debug_selector(move || format!("chat-{number}"))
            .flex_shrink_0()
            .max_w(px(TAB_WIDTH))
            .h(px(TAB_HEIGHT))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(TAB_RADIUS))
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .map(|tab| {
                if shown {
                    tab.bg(rgb(SELECTED)).text_color(rgb(FOREGROUND))
                } else {
                    tab.text_color(rgb(DIM)).hover(|style| style.bg(rgb(HOVER)))
                }
            })
            .on_click(
                cx.listener(move |workspace, event: &ClickEvent, window, cx| {
                    if event.click_count() == 2 {
                        workspace.rename(index, window, cx);
                    } else {
                        workspace.show(index, window, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                    workspace.menu = Some((event.position, index));
                    cx.notify();
                }),
            )
            .when(opened, |tab| {
                tab.child(small("↳", format!("opened-{number}")))
            })
            .children(dot.map(|(color, filled, mark)| {
                let circle = div()
                    .debug_selector(move || format!("chat-{number}-{mark}"))
                    .flex_shrink_0()
                    .size(px(8.))
                    .rounded_full();
                if filled {
                    circle.bg(rgb(color))
                } else {
                    circle.border_1().border_color(rgb(color))
                }
            }))
            .child(name)
            .when(sketch, |tab| {
                tab.child(small("sketch", format!("sketch-{number}")))
            })
            .when(chat.goal.is_some(), |tab| {
                tab.child(small("◎", format!("goal-{number}")))
            })
    }
}

impl Workspace {
    /// The name of the chat at `index` on its tab, or the input that
    /// renames it.
    fn render_tab_name(&self, index: usize, chat: &Chat, cx: &mut Context<Self>) -> Div {
        match &self.renaming {
            Some((renaming, input, _)) if *renaming == index => div()
                .key_context(RENAMING)
                .debug_selector(|| "renaming".to_owned())
                .w(px(180.))
                .px_1()
                .border_1()
                .border_color(rgb(LINK))
                .rounded_sm()
                // A click in the input stays there, not showing the chat.
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

    /// "Archived" at the end of the tabs, if the human archived a chat,
    /// and under it, while it is open, the archived chats: each with its
    /// project's name, brought back by a click.
    fn render_archived(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.archived.is_empty() {
            return None;
        }
        let open = self.archived_open;
        let button = div()
            .id("archived")
            .debug_selector(|| "archived".to_owned())
            .flex_shrink_0()
            .h(px(TAB_HEIGHT))
            .px(px(12.))
            .flex()
            .items_center()
            .gap_1()
            .rounded(px(TAB_RADIUS))
            .text_size(px(13.))
            .text_color(rgb(FAINT))
            .cursor_pointer()
            .when(open, |button| button.bg(rgb(SELECTED)))
            .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
            .child(format!("Archived ({})", self.archived.len()))
            .child(if open { "▴" } else { "▾" })
            .on_click(cx.listener(|workspace, _, _, cx| {
                workspace.archived_open = !workspace.archived_open;
                cx.notify();
            }));
        let menu = open.then(|| {
            let rows = self.archived.iter().enumerate().map(|(index, chat)| {
                let number = index + 1;
                div()
                    .id(("archived-chat", number))
                    .debug_selector(move || format!("archived-{number}"))
                    .flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_color(rgb(DIM))
                    .hover(|style| style.bg(rgb(ACCENT_GROUND)).text_color(rgb(FOREGROUND)))
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
                            .text_xs()
                            .text_color(rgb(FAINT))
                            .child(self.projects[chat.project].name.clone()),
                    )
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.archived_open = false;
                        workspace.unarchive(index, window, cx);
                    }))
            });
            deferred(
                div()
                    .id("archived-chats")
                    .occlude()
                    .absolute()
                    .top(px(TABS_HEIGHT))
                    .right_0()
                    .w(px(280.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .bg(rgb(RAISED))
                    .border_1()
                    .border_color(rgb(BORDER_STRONG))
                    .rounded(px(14.))
                    .shadow_lg()
                    .children(rows),
            )
            .with_priority(2)
        });
        Some(div().flex_shrink_0().child(button).children(menu))
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
        let item = |id: &'static str, label: &'static str| {
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
        let menu = div()
            .occlude()
            .min_w(px(160.))
            .p_1()
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(BORDER_STRONG))
            .rounded_md()
            .shadow_md()
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
            .children(endings.into_iter().enumerate().map(|(at, ending)| {
                div()
                    .id(("sketch-ending", at))
                    .debug_selector(move || format!("sketch-ending-{at}"))
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(ACCENT_GROUND)))
                    .child(ending.name.clone())
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.end_sketch_as(index, ending.clone(), window, cx);
                    }))
            }))
            .child(item("archive-chat", "Archive").on_click(
                cx.listener(move |workspace, _, window, cx| workspace.archive(index, window, cx)),
            ));
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}

impl Focusable for Workspace {
    /// The settings' input while they are open, or else the shown chat's
    /// input, or the window itself while none is open.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if let Some((settings, _)) = &self.settings {
            return settings.focus_handle(cx);
        }
        match self.shown().and_then(Chat::view) {
            Some(view) => view.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = match self.shown().and_then(Chat::view) {
            Some(view) => view.clone().into_any_element(),
            None => card()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(FAINT))
                .child(if self.chats(self.chosen).next().is_some() {
                    "Choose a chat above, or open another with +."
                } else {
                    "No chat here yet: + above opens one."
                })
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .key_context("Workspace")
            .on_action(cx.listener(|workspace, _: &NewChat, window, cx| {
                workspace.open(None, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &OpenFolder, window, cx| {
                workspace.choose_folder(window, cx);
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
            .on_action(cx.listener(|workspace, _: &ToggleStage, window, cx| {
                if let Some(view) = workspace.shown().and_then(|chat| chat.view.clone()) {
                    view.update(cx, |view, cx| view.toggle_stage(window, cx));
                }
            }))
            .on_action(cx.listener(|workspace, _: &OpenFile, window, cx| {
                workspace.on_stage(window, cx, |_, window, cx| Stage::choose_file(window, cx));
            }))
            .relative()
            .size_full()
            .flex()
            .bg(rgb(GROUND))
            .text_color(rgb(FOREGROUND))
            .font_family(UI_FONT)
            .text_size(px(14.))
            .child(self.render_rail(cx))
            // The tabs, and the chat's cards under them, on the ground the
            // rail is on.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap(px(GAP))
                    .py(px(GAP))
                    .pr(px(GAP))
                    .child(self.render_tabs(cx))
                    .child(div().flex_1().min_h_0().child(shown)),
            )
            .children(self.render_menu(cx))
            .children(self.render_settings(cx))
            .children(
                cx.try_global::<crate::ui::translation::Shown>()
                    .and_then(|shown| shown.0.clone()),
            )
    }
}
