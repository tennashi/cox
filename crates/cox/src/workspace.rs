//! The window: the projects on a rail down its left side, the chats of
//! the project chosen in tabs along its top, and the one shown under
//! them. A project is a folder
//! the human opened: each chat in it works there, and its agent gets the
//! human's instructions, then those in the folder's `COX.md` (see
//! [`crate::agent::instructions`]). Chats in no project work in Cox's
//! home, with the instructions there.
//!
//! The projects and chats are kept as they go (see [`cox_core::store`]).
//! A chat's agent runs until Cox quits; the chats of earlier runs stay in
//! the list, closed, for the human to read what they showed, and to send
//! to again, which starts the chat's agent again.
//!
//! A chat is named by a number at first, then after the first line the
//! human sends in it, then by the title its agent gives it, until the
//! human names it. Its tab tells whether its agent is at work, waits for
//! the human, or finished a turn the human has not seen yet, and whether
//! it has a goal, and the project's button on the rail counts its chats
//! that wait for the human. The menu of the right button on a tab
//! renames the chat, as a double click does, sets or clears its goal,
//! ends a trial (see [`crate::chat::trial`]), or archives it: an archived
//! chat leaves the list for "Archived" at the end of the tabs, from where
//! a click brings it back.
//!
//! The gear at the foot of the rail opens the settings screen over the
//! window (see [`crate::settings::view`]), where the human sets Cox's
//! home: the chats in no project opened from then on work there.
//!
//! A chat that comes to wait for the human, or whose turn ends, while the
//! human is not looking at it, tells them so (see [`crate::notices`]),
//! and the app's icon counts the chats that wait for them or that they
//! have not seen since a turn ended. A click on what a chat told shows it.
//!
//! A chat's agent can open a chat too, with Cox's `open_chat` tool: the
//! chat opens under its own in the list, in the same project, set up as
//! the agent says (see [`crate::agent::setup`]), and works on the line
//! the agent gives it. Which chat opened it, and how, is kept with it,
//! and the agent waits for it with `wait_chat`.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use cox_core::goal::Goal;
use cox_core::list::List;
use cox_core::store::{ChatId, Named, ProjectId, Record, Store};
use gpui::{
    App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, MouseDownEvent, PathPromptOptions, Pixels, Point, SharedString, Stateful,
    Subscription, Window, actions, anchored, deferred, div, prelude::*, px, rgb,
};

use crate::agent::instructions::Instructions;
use crate::workflow;
use tokio::sync::oneshot;

use crate::agent::mcp::{ChatRequest, OpenedChat, Waited};
use crate::agent::setup::{Policy, Setup};
use crate::chat::conversation::{self, Entry, State};
use crate::chat::trial::{self, Trial};
use crate::chat::view::{Asked, ChatView, Reopened, SwitchAgent};
use crate::chat::{Closed, Opens};
use crate::notices::AppNotices;
use crate::settings::view::{Dismissed, HomeChanged, OpenSettings, SettingsView};
use crate::stage::{NewPage, NewTerminal, OpenFile, Stage};
use crate::storage;
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT, ACCENT_GROUND, ASKING, BORDER, BORDER_STRONG, DIM, DISABLED, FAINT, FOREGROUND, GAP,
    GROUND, HOVER, LINK, RAISED, SELECTED, SURFACE, UI_FONT,
};
use crate::ui::{card, tip};

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

/// What the project of chats with no folder is called.
const NO_PROJECT: &str = "No project";

/// How long a chat's name grows, in characters, when the agent or the
/// first line names it.
const NAME_LENGTH: usize = 80;

/// The key context of the input that renames a chat.
const RENAMING: &str = "Renaming";

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
    /// How many chats have been opened in it, to name the next.
    opened: usize,
    /// The key of the chat of it shown last, which choosing the project
    /// shows again.
    last: Option<u64>,
}

impl Project {
    fn new(id: Option<ProjectId>, name: impl Into<SharedString>, folder: PathBuf) -> Self {
        Self {
            id,
            name: name.into(),
            folder,
            opened: 0,
            last: None,
        }
    }

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

/// A chat in the list: open, with its agent, or closed, from an earlier
/// run, for the human to read, until they send to it.
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
    /// Whose move it was when the chat last changed.
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
    /// once it became a trial.
    setup: Option<Setup>,
    /// What its agent is told to go on with once it moves into the trial
    /// the chat became, which it does when its turn ends.
    moving: Option<String>,
    /// The way a trial the human ends ends, and how far it is on its way
    /// there, while its agent sums it up first.
    ending: Option<(Ends, workflow::Ending)>,
    /// Its view, or `None` for a closed chat the human has not read yet.
    view: Option<Entity<ChatView>>,
    /// Redraws the list as the chat's state changes, and hears a closed
    /// chat's agent start again.
    _changes: Vec<Subscription>,
}

impl Chat {
    /// A closed chat of the project at `project`, kept as `kept`, which
    /// the human has not read yet, named `key` for this run, working
    /// toward `goal` if the human set one, opened by the chat
    /// `parent` names if an agent opened it, and set up as `setup` says.
    #[expect(clippy::too_many_arguments, reason = "what is kept of a chat")]
    fn closed(
        key: u64,
        project: usize,
        kept: ChatId,
        name: impl Into<SharedString>,
        named: Named,
        goal: Option<Goal>,
        parent: Option<u64>,
        setup: Option<Setup>,
    ) -> Self {
        Self {
            key,
            project,
            kept: Some(kept),
            name: name.into(),
            named,
            open: false,
            state: State::Closed,
            unread: false,
            goal,
            parent,
            setup,
            moving: None,
            ending: None,
            view: None,
            _changes: Vec::new(),
        }
    }

    /// Names it `name`, which `named` says where it came from, and keeps
    /// the name in `store`.
    fn name_as(&mut self, name: &str, named: Named, store: &Store) {
        self.name = SharedString::from(name.to_owned());
        self.named = named;
        if let Some(kept) = self.kept
            && let Err(error) = store.name_chat(kept, name, named)
        {
            tracing::error!("the chat's name is not kept: {error:#}");
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
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

    /// Whether it is a trial, working in a folder of its own.
    #[must_use]
    pub fn is_trial(&self) -> bool {
        self.setup
            .as_ref()
            .is_some_and(|setup| setup.trial.is_some())
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

    /// Its view, once it has been shown.
    #[must_use]
    pub fn view(&self) -> Option<&Entity<ChatView>> {
        self.view.as_ref()
    }
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
    /// The key of the last chat opened.
    last_key: u64,
    /// How many chats the app's icon counts.
    badged: usize,
    /// The menu of the right button, where it opened in the window, for
    /// the chat at an index in the list.
    menu: Option<(Point<Pixels>, usize)>,
    /// The chat at an index in the list being renamed, in an input.
    renaming: Option<(usize, Entity<LineInput>, Subscription)>,
    /// The agents waiting for a chat, by its key, to end its turn or to
    /// come to wait for the human.
    waiting: Vec<(u64, oneshot::Sender<Result<Waited, String>>)>,
    /// Whether `--home` gave the home, the folder of no project, for this
    /// run, ahead of what the settings keep.
    home_given: bool,
    /// The settings screen, while it is open.
    settings: Option<(Entity<SettingsView>, [Subscription; 2])>,
    opens: Rc<dyn Opens>,
    store: Rc<Store>,
    focus_handle: FocusHandle,
}

impl Workspace {
    /// A window with no chat open, listing the chats kept in `store`.
    /// `opens` opens chats, and chats in no project work in `home`, which
    /// `--home` gave when `home_given` says so.
    pub fn new(
        home: PathBuf,
        home_given: bool,
        store: Rc<Store>,
        opens: Rc<dyn Opens>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut workspace = Self {
            projects: vec![Project::new(None, NO_PROJECT, home)],
            chats: List::new(),
            archived: Vec::new(),
            archived_open: false,
            chosen: 0,
            last_key: 0,
            badged: 0,
            menu: None,
            renaming: None,
            waiting: Vec::new(),
            home_given,
            settings: None,
            opens,
            store,
            focus_handle: cx.focus_handle(),
        };
        let kept = workspace
            .store
            .projects()
            .and_then(|kept| Ok((kept, workspace.store.chats()?)));
        match kept {
            Ok((kept, chats)) => {
                workspace.projects.extend(
                    kept.into_iter().map(|project| {
                        Project::new(Some(project.id), project.name, project.folder)
                    }),
                );
                for chat in chats {
                    let Some(index) = workspace
                        .projects
                        .iter()
                        .position(|project| project.id == chat.project)
                    else {
                        continue;
                    };
                    workspace.projects[index].opened += 1;
                    let key = workspace.next_key();
                    // The chats come in the order they were opened, so
                    // the parent is listed before its children.
                    let parent = chat.parent.and_then(|parent| {
                        workspace
                            .chats
                            .iter()
                            .chain(&workspace.archived)
                            .find(|other| other.kept == Some(parent))
                            .map(|other| other.key)
                    });
                    let setup = chat.setup.as_deref().and_then(|setup| {
                        Setup::from_json(setup)
                            .inspect_err(|error| {
                                tracing::error!("the chat's setup cannot be read: {error:#}");
                            })
                            .ok()
                    });
                    let closed = Chat::closed(
                        key, index, chat.id, chat.name, chat.named, chat.goal, parent, setup,
                    );
                    if chat.archived {
                        workspace.archived.push(closed);
                    } else {
                        workspace.add(closed);
                    }
                }
            }
            Err(error) => tracing::error!("the chats kept cannot be listed: {error:#}"),
        }
        workspace
    }

    /// A key for a chat, which no other chat has had in this run.
    fn next_key(&mut self) -> u64 {
        self.last_key += 1;
        self.last_key
    }

    /// Puts `chat` in the list after the other chats of its project, or,
    /// when a chat in the list opened it, after that chat and the chats
    /// opened under it; and returns its index.
    fn add(&mut self, chat: Chat) -> usize {
        // What was at an index may move along.
        self.menu = None;
        self.renaming = None;
        let parent = chat
            .parent
            .and_then(|parent| self.chats.iter().position(|other| other.key == parent));
        let after = match parent {
            Some(parent) => {
                let depth = self.depth(&self.chats[parent]);
                self.chats
                    .iter()
                    .enumerate()
                    .skip(parent + 1)
                    .find(|(_, other)| self.depth(other) <= depth)
                    .map_or(self.chats.len(), |(index, _)| index)
            }
            None => self
                .chats
                .iter()
                .rposition(|other| other.project <= chat.project)
                .map_or(0, |index| index + 1),
        };
        self.chats.insert(after, chat, false)
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

    /// The index of the project of `folder`, opened first if it is not
    /// open. Cox's home is no project's.
    fn project(&mut self, folder: PathBuf) -> usize {
        if let Some(index) = self
            .projects
            .iter()
            .position(|project| project.folder == folder)
        {
            return index;
        }
        let name = folder.file_name().map_or_else(
            || folder.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let id = self
            .store
            .project(&folder, &name)
            .inspect_err(|error| tracing::error!("the project is not kept: {error:#}"))
            .ok();
        self.projects.push(Project::new(id, name, folder));
        self.projects.len() - 1
    }

    /// Opens a chat in the project of `folder`, or in no project when there
    /// is none, and shows it.
    pub fn open(&mut self, folder: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let index = folder.map_or(0, |folder| self.project(folder));
        let chat = self.open_in(index, None, None, window, cx);
        self.show(chat, window, cx);
    }

    /// Opens a chat in the project at `index`, under the chat `from` names
    /// by its key, and how it is kept, when an agent opens it, set up as
    /// `setup` says; and returns its index in the list.
    fn open_in(
        &mut self,
        index: usize,
        from: Option<(u64, Option<ChatId>)>,
        setup: Option<Setup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let project = &mut self.projects[index];
        if index == 0 {
            // The agent's session fails to open, and says so, if the home
            // cannot be made.
            if let Err(error) = std::fs::create_dir_all(&project.folder) {
                tracing::error!("cannot make {}: {error}", project.folder.display());
            }
        }
        let instructions = Instructions::read(&project.folder);
        project.opened += 1;
        let name = format!("Chat {}", project.opened);
        let kept = match from {
            Some((_, Some(parent))) => self.store.add_chat_under(
                parent,
                project.id,
                &name,
                setup.as_ref().map(Setup::to_json).as_deref(),
            ),
            _ => self.store.add_chat(project.id, &name),
        }
        .inspect_err(|error| tracing::error!("the chat is not kept: {error:#}"))
        .ok();
        let record = kept.map(|kept| Record::new(Rc::clone(&self.store), kept));
        let cwd = setup
            .as_ref()
            .and_then(|setup| setup.folder.clone())
            .unwrap_or_else(|| project.folder.clone());
        let view = self
            .opens
            .open(cwd, Some(instructions), setup.clone(), record, window, cx);
        let changes = Self::follow(&view, window, cx);
        let key = self.next_key();
        self.add(Chat {
            key,
            project: index,
            kept,
            name: name.into(),
            named: Named::Numbered,
            open: true,
            state: State::Idle,
            unread: false,
            goal: None,
            parent: from.map(|(parent, _)| parent),
            setup,
            moving: None,
            ending: None,
            view: Some(view),
            _changes: changes,
        })
    }

    /// Follows the chat of `view`: redraws the list as it changes, and
    /// answers what its agent asks of the window.
    fn follow(
        view: &Entity<ChatView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        let changed = cx.observe_in(view, window, |workspace, view, window, cx| {
            workspace.changed(&view, window, cx);
        });
        let asked = cx.subscribe_in(
            view,
            window,
            |workspace, view, asked: &Asked, window, cx| {
                if let Some(request) = asked.take() {
                    workspace.asked(view, request, window, cx);
                }
            },
        );
        let switch = cx.subscribe_in(
            view,
            window,
            |workspace, view, SwitchAgent(id), window, cx| {
                workspace.switch_agent(view, id, window, cx);
            },
        );
        vec![changed, asked, switch]
    }

    /// Switches the chat of `view` to the agent kept as `id`, unless a turn
    /// runs: closes it, its agent stopped, keeps the agent as the chat's,
    /// and starts the agent at once, which goes on from the conversation
    /// so far, told it as a session that cannot be resumed is.
    fn switch_agent(
        &mut self,
        view: &Entity<ChatView>,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .chats
            .iter()
            .position(|chat| chat.view.as_ref() == Some(view))
        else {
            return;
        };
        let chat = &mut self.chats[index];
        let Some(kept) = chat.kept.filter(|_| !working(chat.state)) else {
            return;
        };
        if let Err(error) = self.store.keep_agent(kept, id) {
            tracing::error!("the chat goes on with the agent it had: {error:#}");
            return;
        }
        let Some(view) = chat.view.take() else {
            return;
        };
        let name = view.update(cx, |view, cx| {
            let name = view
                .agents()
                .map_or_else(|| id.to_owned(), |agents| agents.of(Some(id)).name);
            view.switch_agent(&name, cx);
            name
        });
        tracing::info!(agent = %name, "the chat switches to another agent");
        chat.open = false;
        chat.state = State::Closed;
        if self.chats.shown_index() == Some(index) {
            self.show(index, window, cx);
        } else {
            self.read(index, window, cx);
        }
        if let Some(view) = self.chats[index].view.clone() {
            view.update(cx, |view, cx| view.start_again(window, cx));
        }
    }

    /// Answers `request`, which the agent of the chat of `view` asks of the
    /// window: makes the chat a trial, shows a goal it proposes, opens a
    /// chat under it, as the agent says, or waits for one.
    fn asked(
        &mut self,
        view: &Entity<ChatView>,
        request: ChatRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            ChatRequest::StartTrial {
                trial,
                what,
                answer,
            } => {
                let started = self.start_trial(view, trial, &what, window, cx);
                answer.send(started).ok();
                cx.notify();
            }
            ChatRequest::ProposeGoal { goal, why, answer } => {
                let now = view.update(cx, |view, cx| {
                    view.propose_goal(&goal, why, cx);
                    view.conversation().goal().map(|goal| goal.what().clone())
                });
                answer.send(now).ok();
            }
            ChatRequest::OpenChat {
                text,
                mut setup,
                goal,
                trial,
                answer,
            } => {
                let parent = self
                    .chats
                    .iter()
                    .chain(&self.archived)
                    .find(|chat| chat.view.as_ref() == Some(view))
                    .map(|chat| {
                        let folder = chat.setup.as_ref().and_then(|setup| setup.folder.clone());
                        (
                            chat.project,
                            chat.key,
                            chat.kept,
                            chat.name.to_string(),
                            folder,
                        )
                    });
                let Some((project, key, kept, opened_by, folder)) = parent else {
                    answer.send(Err("the chat is gone".to_owned())).ok();
                    return;
                };
                // A trial works in a folder of its own; any other chat
                // works where the chat that opened it does.
                setup.folder = if let Some(trial) = trial {
                    let made = storage::path(trial::FOLDER)
                        .ok_or_else(|| "Cox cannot tell where to keep trials".to_owned())
                        .and_then(|trials| {
                            trial::make(
                                trial,
                                folder.as_ref().unwrap_or(&self.projects[project].folder),
                                &trials,
                            )
                        });
                    match made {
                        Ok(made) => {
                            setup.policy =
                                trial::allow(setup.policy, &made, &workflow::trial().allow);
                            setup.trial = Some(trial);
                            Some(made)
                        }
                        Err(error) => {
                            answer.send(Err(error)).ok();
                            return;
                        }
                    }
                } else {
                    folder
                };
                let index = self.open_in(project, Some((key, kept)), Some(setup), window, cx);
                let chat = &self.chats[index];
                let opened = OpenedChat {
                    number: chat.key,
                    name: chat.name.to_string(),
                    folder: trial
                        .and(chat.setup.as_ref())
                        .and_then(|setup| setup.folder.clone()),
                };
                if let Some(child) = &chat.view {
                    child.update(cx, |child, cx| child.tell(&opened_by, &text, goal, cx));
                }
                answer.send(Ok(opened)).ok();
                cx.notify();
            }
            ChatRequest::WaitChat { chat, answer } => self.wait_chat(chat, answer, cx),
        }
    }

    /// Answers `answer` with how the chat that goes by `key` stands once it
    /// is not at work, or at once when it is not.
    fn wait_chat(
        &mut self,
        key: u64,
        answer: oneshot::Sender<Result<Waited, String>>,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self
            .chats
            .iter()
            .chain(&self.archived)
            .find(|chat| chat.key == key)
        else {
            answer.send(Err(format!("no chat {key}"))).ok();
            return;
        };
        // The list may not have heard of the chat's last change yet.
        let state = chat
            .view
            .as_ref()
            .map_or(chat.state, |view| view.read(cx).conversation().state());
        if at_work(state) {
            self.waiting.retain(|(_, answer)| !answer.is_closed());
            self.waiting.push((key, answer));
        } else {
            answer.send(Ok(stands(chat, cx))).ok();
        }
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
                workspace.projects[0].folder.clone_from(home);
                workspace.home_given = false;
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
        if self.settings.take().is_some() {
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
        chat.unread = false;
        self.chosen = chat.project;
        self.projects[chat.project].last = Some(chat.key);
        self.badge(cx);
        self.read(index, window, cx);
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Gives the closed chat at `index` its view, from what is kept of it,
    /// unless it has one. Once the human sends to it, it is open.
    fn read(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[index];
        let project = &self.projects[chat.project];
        let (Some(kept), None) = (chat.kept, &chat.view) else {
            return;
        };
        let entries = self.store.entries(kept).unwrap_or_else(|error| {
            tracing::error!("the chat cannot be read: {error:#}");
            vec![Entry::Note(format!("Cox cannot read this chat: {error:#}"))]
        });
        let tabs = self
            .store
            .tabs(kept)
            .inspect_err(|error| tracing::error!("the chat's tabs cannot be read: {error:#}"))
            .ok()
            .flatten();
        let session = self
            .store
            .session(kept)
            .inspect_err(|error| tracing::error!("the chat's session cannot be read: {error:#}"))
            .ok()
            .flatten();
        let settings = self.store.settings(kept).unwrap_or_else(|error| {
            tracing::error!("the chat's settings cannot be read: {error:#}");
            Vec::new()
        });
        let closed = Closed {
            cwd: chat
                .setup
                .as_ref()
                .and_then(|setup| setup.folder.clone())
                .unwrap_or_else(|| project.folder.clone()),
            entries,
            tabs,
            session,
            goal: chat.goal.clone(),
            settings,
            setup: chat.setup.clone(),
            record: Some(Record::new(Rc::clone(&self.store), kept)),
        };
        let view = self.opens.read(closed, window, cx);
        let mut changes = Self::follow(&view, window, cx);
        let reopened = cx.subscribe(&view, move |workspace, _, Reopened, cx| {
            // The chat may have moved along the list since.
            if let Some(chat) = workspace
                .chats
                .iter_mut()
                .find(|chat| chat.kept == Some(kept))
            {
                chat.open = true;
            }
            cx.notify();
        });
        let chat = &mut self.chats[index];
        *chat = Chat {
            key: chat.key,
            project: chat.project,
            kept: Some(kept),
            name: chat.name.clone(),
            named: chat.named,
            open: false,
            state: State::Closed,
            unread: chat.unread,
            goal: chat.goal.take(),
            parent: chat.parent,
            setup: chat.setup.take(),
            moving: chat.moving.take(),
            ending: chat.ending.take(),
            view: Some(view),
            _changes: {
                changes.push(reopened);
                changes
            },
        };
    }

    /// Follows a change of the chat of `view`: a turn that ends while it is
    /// not shown leaves it unread, and what it is about names it, unless a
    /// name that lasts more has. A chat the human is not looking at tells
    /// them when it comes to wait for them, and when its turn ends.
    fn changed(&mut self, view: &Entity<ChatView>, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.chats.shown().and_then(Chat::view) == Some(view);
        let seen = shown && window.is_window_active();
        let Some(chat) = self
            .chats
            .iter_mut()
            .chain(&mut self.archived)
            .find(|chat| chat.view.as_ref() == Some(view))
        else {
            return;
        };
        let conversation = view.read(cx).conversation();
        let state = conversation.state();
        let ended = working(chat.state) && !working(state);
        if ended && !shown {
            chat.unread = true;
        }
        let news = if state == State::Asking && chat.state != state {
            let asked = conversation.asking_for().unwrap_or("permission");
            Some(format!("Asks you: {asked}"))
        } else if ended {
            let said = conversation.last_said().unwrap_or("The turn ended.");
            Some(said.to_owned())
        } else {
            None
        };
        chat.state = state;
        chat.goal = conversation.goal().map(|goal| goal.what().clone());
        let about = match (conversation.title(), conversation.first_line()) {
            (Some(title), _) => Some((title, Named::Agent)),
            (None, Some(line)) => Some((line, Named::FirstLine)),
            (None, None) => None,
        };
        if let Some((name, named)) = about {
            let name: String = name.chars().take(NAME_LENGTH).collect();
            // The agent may title its session anew.
            let lasts = named > chat.named || (named == Named::Agent && chat.named == named);
            if lasts && name != chat.name.as_ref() {
                chat.name_as(&name, named, &self.store);
            }
        }
        if let Some(body) = news.filter(|_| !seen)
            && let Some(notices) = cx.try_global::<AppNotices>()
        {
            notices.0.notify(chat.key, &chat.name, &body);
        }
        let waited = (!at_work(state)).then(|| (chat.key, stands(chat, cx)));
        let moves = chat.moving.is_some() && !working(state);
        let ends = chat
            .ending
            .as_ref()
            .map(|(ends, _)| *ends)
            .filter(|_| !working(state));
        if let Some((key, stands)) = waited {
            for (_, answer) in self.waiting.extract_if(.., |(waited, _)| *waited == key) {
                answer.send(Ok(stands.clone())).ok();
            }
        }
        let index = self
            .chats
            .iter()
            .position(|chat| chat.view.as_ref() == Some(view));
        // A chat that became a trial moves into it once its turn ends.
        if moves && let Some(index) = index {
            self.move_into_trial(index, window, cx);
        }
        // A trial the human ends is summed up once its turn ends, and
        // ends once that is done.
        match (ends, index) {
            (Some(Ends::Waiting), Some(index)) => self.sum_up(index, window, cx),
            (Some(Ends::Summing), Some(index)) => self.finish_trial(index, window, cx),
            _ => {}
        }
        self.badge(cx);
        cx.notify();
    }

    /// Makes the chat of `view` a trial that starts from `trial`, to try
    /// out `what`, and answers its folder, or why it cannot be one. It
    /// moves into the trial at once, unless its turn goes on, until it
    /// ends.
    fn start_trial(
        &mut self,
        view: &Entity<ChatView>,
        started: Trial,
        what: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PathBuf, String> {
        let index = self
            .chats
            .iter()
            .position(|chat| chat.view.as_ref() == Some(view))
            .ok_or("the chat is not in the list")?;
        let chat = &self.chats[index];
        let setup = chat.setup.clone().unwrap_or_default();
        if let (Some(_), Some(folder)) = (setup.trial, &setup.folder) {
            return Err(format!(
                "the chat is a trial already, working in {}",
                folder.display()
            ));
        }
        let from = setup
            .folder
            .clone()
            .unwrap_or_else(|| self.projects[chat.project].folder.clone());
        let trials = storage::path(trial::FOLDER).ok_or("Cox cannot tell where to keep trials")?;
        let folder = trial::make(started, &from, &trials)?;
        let setup = Setup {
            folder: Some(folder.clone()),
            trial: Some(started),
            policy: trial::allow(setup.policy, &folder, &workflow::trial().allow),
            ..setup
        };
        let chat = &mut self.chats[index];
        if let Some(kept) = chat.kept
            && let Err(error) = self.store.keep_setup(kept, Some(&setup.to_json()))
        {
            tracing::error!("the chat's setup is not kept: {error:#}");
        }
        // A chat nothing was said in yet, as one the human opens as a
        // trial, is named after what it tries, not after the line Cox
        // sends it on that.
        if chat.named == Named::Numbered {
            let name: String = what.chars().take(NAME_LENGTH).collect();
            chat.name_as(&name, Named::FirstLine, &self.store);
        }
        chat.setup = Some(setup);
        chat.moving = Some(workflow::TRIAL_START.say(&[
            ("folder", &folder.display().to_string()),
            ("from", started.what()),
            ("what", what),
        ]));
        if !working(chat.state) {
            self.move_into_trial(index, window, cx);
        }
        Ok(folder)
    }

    /// Moves the chat at `index`, which became a trial, into it: closes
    /// it, its agent stopped, and starts its agent again in the trial's
    /// folder, with the conversation so far, on what it goes on with.
    fn move_into_trial(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &mut self.chats[index];
        let Some(line) = chat.moving.take() else {
            return;
        };
        let folder = chat
            .setup
            .as_ref()
            .and_then(|setup| setup.folder.clone())
            .unwrap_or_default();
        if let Some(view) = chat.view.take() {
            view.update(cx, |view, cx| {
                view.close(conversation::trial_note(&folder), cx);
            });
        }
        chat.open = false;
        chat.state = State::Closed;
        if self.chats.shown_index() == Some(index) {
            self.show(index, window, cx);
        } else {
            self.read(index, window, cx);
        }
        if let Some(view) = self.chats[index].view.clone() {
            view.update(cx, |view, cx| view.go_on(&line, window, cx));
        }
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
        if let Some(index) = self.chats.iter().position(|chat| chat.key == key) {
            self.show(index, window, cx);
        } else if let Some(index) = self.archived.iter().position(|chat| chat.key == key) {
            self.unarchive(index, window, cx);
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
        if keep && !name.is_empty() {
            self.chats[index].name_as(&name, Named::Human, &self.store);
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
    /// trial, of what the human says it is to try.
    pub fn edit_trial(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        if !self.starts_trial(index) {
            return;
        }
        self.show(index, window, cx);
        let Some(chat) = self.chats.get(index) else {
            return;
        };
        let from = trial::from_at_first(&self.projects[chat.project].folder);
        if let Some(view) = chat.view() {
            view.update(cx, |view, cx| view.edit_trial(from, window, cx));
        }
    }

    /// Opens a chat in the project of `folder`, and the input that makes
    /// it a trial, for the human to say what it is to try.
    pub fn new_trial(&mut self, folder: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open(Some(folder), window, cx);
        if let Some(index) = self.chats.shown_index() {
            self.edit_trial(index, window, cx);
        }
    }

    /// Whether the human can make the chat at `index` a trial: it is not
    /// one already, and is not on its way into one.
    fn starts_trial(&self, index: usize) -> bool {
        self.chats.get(index).is_some_and(|chat| {
            chat.moving.is_none()
                && chat
                    .setup
                    .as_ref()
                    .is_none_or(|setup| setup.trial.is_none())
        })
    }

    /// Shows the chat at `index`, and clears its goal.
    pub fn clear_goal(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        self.show(index, window, cx);
        if let Some(view) = self.chats.get(index).and_then(Chat::view) {
            view.update(cx, |view, cx| view.set_goal(None, cx));
        }
    }

    /// The trial the chat at `index` is, what it started from and the
    /// folder it works in, while the folder is there.
    fn trial_of(&self, index: usize) -> Option<(Trial, PathBuf)> {
        let setup = self.chats.get(index)?.setup.as_ref()?;
        let folder = setup.folder.clone().filter(|folder| folder.exists())?;
        Some((setup.trial?, folder))
    }

    /// Whether the trial at `index` can end: its folder is there, and was
    /// not opened as a project, which the human goes on with, and it is
    /// not on its way to an end already.
    fn ends_trial(&self, index: usize) -> bool {
        self.chats[index].ending.is_none()
            && self.trial_of(index).is_some_and(|(_, folder)| {
                !self.projects.iter().any(|project| project.folder == folder)
            })
    }

    /// The ways the trial at `index` can end, as the human's workflow
    /// says them (see [`workflow::trial`]), in the order the menu offers
    /// them; none, unless it can end now.
    #[must_use]
    pub fn endings(&self, index: usize) -> Vec<workflow::Ending> {
        let Some((from, _)) = self.trial_of(index).filter(|_| self.ends_trial(index)) else {
            return Vec::new();
        };
        workflow::trial()
            .endings
            .into_iter()
            .filter(|ending| ending.ends(from))
            .collect()
    }

    /// Ends the trial at `index` as `ending` says: asks its agent to sum
    /// it up in the conversation first, if the ending says to, once its
    /// turn ends if one goes on, and ends it once that is done (see
    /// [`Self::finish_trial`]).
    pub fn end_trial_as(
        &mut self,
        index: usize,
        ending: workflow::Ending,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let ends = index < self.chats.len()
            && self.ends_trial(index)
            && self
                .trial_of(index)
                .is_some_and(|(from, _)| ending.ends(from));
        if !ends {
            return;
        }
        self.show(index, window, cx);
        let waits = ending.sum_up.is_some() && working(self.chats[index].state);
        let name = ending.name.clone();
        self.chats[index].ending = Some((Ends::Waiting, ending));
        if waits {
            if let Some(view) = self.chats[index].view.clone() {
                view.update(cx, |view, cx| {
                    view.note(
                        format!(
                            "The trial ends as \"{name}\" once this turn ends, and its agent \
                             has summed it up."
                        ),
                        cx,
                    );
                });
            }
            cx.notify();
        } else {
            self.sum_up(index, window, cx);
        }
    }

    /// Asks the agent of the trial at `index`, which the human ends, to
    /// sum it up in the conversation, as its ending says, starting it
    /// again if it is closed; the trial ends once it is done, or at once if
    /// the ending asks for none, or the agent cannot be asked.
    fn sum_up(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, ending)) = self.chats[index].ending.clone() else {
            return;
        };
        let Some(text) = ending.sum_up.clone() else {
            self.finish_trial(index, window, cx);
            return;
        };
        let Some((_, folder)) = self.trial_of(index) else {
            self.chats[index].ending = None;
            return;
        };
        let left_out = trial::left_out(&folder).unwrap_or_else(|error| {
            tracing::warn!("cannot tell what the trial changed: {error}");
            Vec::new()
        });
        if self.chats[index].view.is_none() {
            self.show(index, window, cx);
        }
        let Some(view) = self.chats[index].view.clone() else {
            self.chats[index].ending = None;
            return;
        };
        let Some(line) = workflow::say(&text, &[("left_out", &trial::left_out_lines(&left_out))])
        else {
            view.update(cx, |view, cx| {
                view.note(
                    format!("The trial is not summed up: the workflow has no {text}."),
                    cx,
                );
            });
            self.finish_trial(index, window, cx);
            return;
        };
        self.chats[index].ending = Some((Ends::Summing, ending));
        let asked = view.update(cx, |view, cx| {
            view.go_on(&line, window, cx);
            working(view.conversation().state())
        });
        if !asked {
            self.finish_trial(index, window, cx);
        }
    }

    /// Ends the trial at `index`, whose agent has summed it up if its
    /// ending asked, as the ending says. Its folder is opened as a project
    /// of its own, to go on with what the trial made, for an ending that
    /// opens it; or else it goes, what the trial changed kept apart first
    /// if the ending says to (see [`trial::discard`]), and the trial ends,
    /// its chat going on in the project toward the ending's goal, if it
    /// has one, its agent told what the ending says then.
    fn finish_trial(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, ending)) = self.chats[index].ending.take() else {
            return;
        };
        let Some((started, folder)) = self.trial_of(index).filter(|_| self.ends_trial(index))
        else {
            return;
        };
        if ending.open {
            // A project now, what changed in it is what changed since its
            // last commit.
            if let Err(error) = cox_editor::changes::forget(&folder) {
                tracing::warn!(
                    "{} still counts changes from nothing: {error}",
                    folder.display()
                );
            }
            self.open(Some(folder), window, cx);
            return;
        }
        let title = self.chats[index].name.to_string();
        let discarded = storage::path(trial::FOLDER)
            .map(|trials| trials.join(trial::DISCARDED))
            .ok_or_else(|| "Cox cannot tell where to keep discarded trials".to_owned())
            .and_then(|bundles| trial::discard(started, &folder, &title, &bundles, ending.keep));
        let kept = discarded
            .as_ref()
            .ok()
            .and_then(|discarded| discarded.kept.as_ref())
            .map(ToString::to_string)
            .unwrap_or_default();
        let ended = self.end_trial(
            index,
            &folder,
            discarded.map(|discarded| discarded.note()),
            window,
            cx,
        );
        let Some(view) = self.chats[index].view.clone().filter(|_| ended) else {
            return;
        };
        if let Some(goal) = &ending.goal {
            view.update(cx, |view, cx| view.set_goal(Goal::parse(goal), cx));
        }
        if let Some(text) = &ending.then {
            match workflow::say(text, &[("kept", &kept)]) {
                Some(line) => view.update(cx, |view, cx| view.go_on(&line, window, cx)),
                None => view.update(cx, |view, cx| {
                    view.note(
                        format!("The agent is told nothing: the workflow has no {text}."),
                        cx,
                    );
                }),
            }
        }
    }

    /// Shows the chat at `index`, a trial that worked in `folder`, and
    /// ends the trial as `ended` says, or says why it could not, and
    /// answers whether it ended. Ended,
    /// the trial's chat goes on in the project, as do the chats it opened,
    /// which worked in its folder too: each is closed, its agent stopped,
    /// and set up to work in the project, asking the human as any chat
    /// does, so that a line sent starts its agent again there, with the
    /// conversation so far.
    fn end_trial(
        &mut self,
        index: usize,
        folder: &Path,
        ended: Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.menu = None;
        self.show(index, window, cx);
        let note = match ended {
            Ok(note) => note,
            Err(error) => {
                if let Some(view) = self.chats.get(index).and_then(Chat::view) {
                    view.update(cx, |view, cx| {
                        view.note(format!("The trial could not end: {error}"), cx);
                    });
                }
                return false;
            }
        };
        let goes_on = "The chat goes on in the project: a line sent starts its agent again \
                       there, with the conversation so far.";
        let trial = self.chats[index].key;
        let mut ended = Vec::new();
        for chat in self.chats.iter_mut().chain(&mut self.archived) {
            let worked_there = chat.setup.as_ref().is_some_and(|setup| {
                setup
                    .folder
                    .as_ref()
                    .is_some_and(|works| works.starts_with(folder))
            });
            if !worked_there {
                continue;
            }
            let told = if chat.key == trial {
                format!("{note} {goes_on}")
            } else {
                format!(
                    "The trial this chat worked in has ended, and its folder is gone. {goes_on}"
                )
            };
            if let Some(view) = chat.view.take() {
                view.update(cx, |view, cx| view.close(told.clone(), cx));
            }
            chat.open = false;
            chat.state = State::Closed;
            chat.setup = chat.setup.take().map(|setup| Setup {
                folder: None,
                trial: None,
                policy: Policy::default(),
                ..setup
            });
            if let Some(kept) = chat.kept
                && let Err(error) = self
                    .store
                    .keep_setup(kept, chat.setup.as_ref().map(Setup::to_json).as_deref())
            {
                tracing::error!("the chat's setup is not kept: {error:#}");
            }
            ended.push((chat.key, told));
        }
        // What waits for them hears how they ended.
        for (key, told) in ended {
            for (_, answer) in self.waiting.extract_if(.., |(waited, _)| *waited == key) {
                let stands = Waited {
                    asking: None,
                    said: Some(told.clone()),
                };
                answer.send(Ok(stands)).ok();
            }
        }
        // The trial's chat, shown again, gets a view of it read again,
        // closed, to go on with.
        self.show(index, window, cx);
        cx.notify();
        true
    }

    /// Takes the chat at `index` out of the list, to "Archived" at its
    /// foot. Its agent, if it runs, goes on.
    pub fn archive(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let was_shown = self.chats.shown_index() == Some(index);
        self.menu = None;
        self.renaming = None;
        let Some(chat) = self.chats.remove(index) else {
            return;
        };
        keep_archived(&chat, true, &self.store);
        self.archived.push(chat);
        if was_shown {
            match self.chats.shown_index() {
                Some(shown) => self.show(shown, window, cx),
                None => window.focus(&self.focus_handle),
            }
        }
        cx.notify();
    }

    /// Brings the archived chat at `index` among the archived back to the
    /// list, and shows it.
    pub fn unarchive(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.archived.len() {
            return;
        }
        let chat = self.archived.remove(index);
        keep_archived(&chat, false, &self.store);
        let listed = self.add(chat);
        self.show(listed, window, cx);
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
            .child(
                div()
                    .id("new-trial-in")
                    .debug_selector(move || format!("new-trial-in-{}", chosen + 1))
                    .flex_shrink_0()
                    .h(px(TAB_HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(TAB_RADIUS))
                    .text_color(rgb(FAINT))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                    .tooltip(tip(
                        "A chat that tries something out, apart from the project",
                    ))
                    .child("New trial")
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.new_trial(folder.clone(), window, cx);
                    })),
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
    /// it, and whether it is a trial and has a goal.
    fn render_tab(&self, index: usize, chat: &Chat, cx: &mut Context<Self>) -> Stateful<Div> {
        let number = index + 1;
        let shown = self.chats.shown_index() == Some(index);
        let trial = chat
            .setup
            .as_ref()
            .is_some_and(|setup| setup.trial.is_some());
        let state = chat
            .view
            .as_ref()
            .filter(|_| chat.open)
            .map(|view| view.read(cx).conversation().state());
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
            .when(trial, |tab| {
                tab.child(small("trial", format!("trial-{number}")))
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
        let starts_trial = self.starts_trial(index);
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
            .when(starts_trial, |menu| {
                menu.child(item("start-trial", "Start trial…").on_click(cx.listener(
                    move |workspace, _, window, cx| workspace.edit_trial(index, window, cx),
                )))
            })
            .children(endings.into_iter().enumerate().map(|(at, ending)| {
                div()
                    .id(("trial-ending", at))
                    .debug_selector(move || format!("trial-ending-{at}"))
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(ACCENT_GROUND)))
                    .child(ending.name.clone())
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.end_trial_as(index, ending.clone(), window, cx);
                    }))
            }))
            .child(item("archive-chat", "Archive").on_click(
                cx.listener(move |workspace, _, window, cx| workspace.archive(index, window, cx)),
            ));
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}

/// How far a trial the human ends is on its way to its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ends {
    /// Its turn goes on; its agent is asked to sum it up once the turn
    /// ends.
    Waiting,
    /// Its agent sums it up; it ends once that turn ends.
    Summing,
}

/// Whether an agent that stands as `state` is at work.
fn working(state: State) -> bool {
    matches!(
        state,
        State::Busy | State::Asking | State::Stopping | State::Checking
    )
}

/// Whether a chat in `state` is at work, so that an agent waiting for it
/// waits on: not when it waits for the human.
fn at_work(state: State) -> bool {
    matches!(state, State::Busy | State::Stopping | State::Checking)
}

/// How `chat` stands, as an agent waiting for it is told.
fn stands(chat: &Chat, cx: &App) -> Waited {
    let conversation = chat.view.as_ref().map(|view| view.read(cx).conversation());
    Waited {
        asking: conversation
            .and_then(|conversation| conversation.asking_for())
            .map(str::to_owned),
        said: conversation
            .and_then(|conversation| conversation.last_told())
            .map(str::to_owned),
    }
}

/// Keeps in `store` whether `chat` is archived.
fn keep_archived(chat: &Chat, archived: bool, store: &Store) {
    if let Some(kept) = chat.kept
        && let Err(error) = store.archive_chat(kept, archived)
    {
        tracing::error!("whether the chat is archived is not kept: {error:#}");
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
    }
}
