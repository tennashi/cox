//! The daemon: what runs the chats, apart from the window that shows them,
//! so that a chat's agent goes on while no window shows it. It holds the
//! projects and their chats, keeps them as they go (see
//! [`cox_core::store`]), starts each chat's agent, and goes on with its
//! turns: the check of its goal, its judge, the chats its agent opens, and
//! its sketches (see [`crate::chat::sketch`]).
//!
//! A window tells it what the human does, as [`ToDaemon`], and hears from
//! it, as [`FromDaemon`], the projects and chats, how the conversations it
//! shows change, and what the agents' tools ask of a chat's stage, which
//! only a window has: the daemon draws nothing, and runs no browser.
//!
//! It runs on a thread of its own, with a tokio runtime and `LocalSet` of
//! its own, where everything it does happens in one loop ([`Daemon::serve`]):
//! a message from the window, or an [`Internal`] event its chats' tasks
//! heard, changes it, and what changed goes to the window.
//!
//! - [`chat`] runs one chat: its conversation, its agent, the check of its
//!   goal and its judge.
//! - [`wire`] says what the window and the daemon say to each other.
//! - [`process`] runs the daemon as a process of its own, coxd, which a
//!   window reaches on a socket, and [`client`] is how the window does.
//! - [`phone`] serves the human's phone a page of the chats, which watches
//!   them beside the window, and [`push`] tells the phone of the chats that
//!   need the human while they are away from the Mac.

pub mod chat;
pub mod client;
pub mod phone;
pub mod process;
pub mod push;
pub mod wire;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cox_core::entry::{Answer, Attachment};
use cox_core::goal::Goal;
use cox_core::store::{ChatId, Named, ProjectId, Record, Store};
use cox_core::tabs::KeptTabs;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::agent::instructions::Instructions;
use crate::agent::mcp::{ChatRequest, OpenedChat, Ran, Seen, ToolRequest, Waited, Whereabouts};
use crate::agent::setup::{Policy, Setup, StageUse, keeps_in_wiki};
use crate::agent::{self, Agent, AgentEvent, Start};
use crate::agents::Agents;
use crate::chat::conversation::{Conversation, Entry, Gathered, Patch, State};
use crate::chat::judge::{Heard, Starts};
use crate::chat::sketch::{self, Sketch};
use crate::stage::terminal::run_unseen;
use crate::storage;
use crate::trigger::By;
use crate::workflow;
use chat::{Again, Chat, Task, spawn};
use wire::{ChatRow, FromDaemon, Hello, Listing, ProjectRow, StageAsk, ToDaemon};

/// What the project of chats with no folder is called.
const COX: &str = "Cox";

/// How long a chat's name grows, in characters, when the agent or the
/// first line names it.
pub const NAME_LENGTH: usize = 80;

/// How long the human has not used the Mac for, before Cox takes them to
/// be away from it, and pushes to their phones what the window would
/// tell them.
const AWAY: std::time::Duration = std::time::Duration::from_mins(2);

/// Why a command an agent asks to run does not run while no window is
/// open.
const NO_WINDOW: &str = "Cox's window is not open, so there is no terminal to run it in";

/// Why the agent sees no window of the Mac's other apps, nor an Android
/// device, while Cox's own window is not there.
const NO_STAGE: &str = "Cox's window is not open, so there is no stage to show it on";

/// What starts the agents of the chats: the agents Cox knows, or fake ones
/// in tests.
pub trait Spawns {
    /// The agents a chat can talk to, the first being the one a new chat
    /// starts with.
    fn agents(&self) -> Agents;

    /// Starts the agent kept as `agent`, of a chat working in `cwd`, an
    /// absolute path, as `start` says, its browser tools reaching the
    /// chat's stage through the relay on `relay`, when given, and gives it
    /// Cox's own tools, unless the chat's setup says otherwise.
    fn chat(&self, cwd: PathBuf, start: Start, agent: &str, relay: Option<u16>) -> Spawned;

    /// What starts an agent with no chat, as the judge of a goal, of the
    /// agent kept as `agent`.
    fn starts(&self, agent: &str) -> Starts;
}

/// A chat's agent, started, with what its tools ask of the chat's stage,
/// when it has Cox's tools.
pub struct Spawned {
    pub agent: Agent,
    pub stage: Option<UnboundedReceiver<ToolRequest>>,
}

/// What the human may do of sketches in a chat, as the window's menu
/// offers it (see [`Internal::Sketching`]).
#[derive(Debug)]
pub enum Sketching {
    /// Start one beside it, from the project of `project`, or from nothing.
    Starts { project: PathBuf },
    /// End it, a sketch from `from`.
    Ends { from: Sketch },
}

/// What the daemon's chats' tasks heard, marked with the chat's key and
/// the generation of its agent.
pub enum Internal {
    /// The agent told this.
    Agent {
        key: u64,
        generation: u64,
        event: AgentEvent,
    },
    /// The agent's tools ask this of the daemon.
    Asked {
        key: u64,
        generation: u64,
        request: ChatRequest,
    },
    /// The agent's tools, or the chat, ask this of the chat's stage.
    Stage { key: u64, request: ToolRequest },
    /// The command of the chat's goal ran.
    Checked {
        key: u64,
        generation: u64,
        ran: Result<Ran, String>,
    },
    /// The command that logs the agent in ran.
    LoggedIn {
        key: u64,
        generation: u64,
        ran: Result<Ran, String>,
    },
    /// The judge of the chat's goal said this.
    Judge {
        key: u64,
        generation: u64,
        heard: Heard,
    },
    /// The agent of the chat's trigger of `file` said this.
    Trigger {
        key: u64,
        generation: u64,
        file: PathBuf,
        heard: Heard,
    },
    /// The trigger of `file` that gathers what the human needs to answer
    /// the permission request the chat's agent waits on came back so.
    Gathered {
        key: u64,
        generation: u64,
        file: PathBuf,
        gathered: Gathered,
    },
    /// The time the human waits on the triggers that gather is up.
    GatheredEnough { key: u64, generation: u64 },
    /// The command `id`, which the window runs, is not waited for any more.
    Cancel { id: u64 },
    /// A window came, as the connection `id`, saying `hello`, to hear
    /// what the daemon says on `window`, when the daemon's own program
    /// was `replaced` since it started, or not. `answer` hears how the
    /// daemon took it.
    Attach {
        id: u64,
        hello: Hello,
        window: UnboundedSender<FromDaemon>,
        replaced: bool,
        answer: oneshot::Sender<FromDaemon>,
    },
    /// The window of the connection `id` went.
    Detach { id: u64 },
    /// A phone came, as the connection `id`, to hear on `phone` the
    /// projects and chats, and how the conversation it watches changes
    /// (see [`phone`]).
    View {
        id: u64,
        phone: UnboundedSender<FromDaemon>,
    },
    /// The phone of the connection `id` watches the conversation of the
    /// chat `key` from now on, or none.
    ViewChat { id: u64, key: Option<u64> },
    /// The phone of the connection `id` went.
    Unview { id: u64 },
    /// The human did `message` on their phone, as they would in the
    /// window (see [`phone`]).
    Act { message: ToDaemon },
    /// Where the chat `key` works, for the phone to show what changed
    /// there, told on `answer`, or nothing of a chat there is none of.
    Folder {
        key: u64,
        answer: oneshot::Sender<Option<PathBuf>>,
    },
    /// What the human may do of sketches in the chat `key`, for the phone
    /// to offer it as the window's menu does, told on `answer`, or nothing
    /// of a chat that can neither start one nor end.
    Sketching {
        key: u64,
        answer: oneshot::Sender<Option<Sketching>>,
    },
    /// What the chats that need the human come to is to be pushed to
    /// their phones, on `to`, while they are away from the Mac (see
    /// [`push`]).
    Push { to: UnboundedSender<push::Notice> },
    /// The daemon is to stop, with its chats' agents.
    Quit,
    /// It is time to look which of the triggers on a schedule are due,
    /// and run them (see [`crate::schedule`]).
    Schedules,
}

/// A project: a folder the human opened, whose chats work there.
struct Project {
    /// How it is kept, or `None` for Cox's own.
    id: Option<ProjectId>,
    name: String,
    folder: PathBuf,
    /// How many chats were opened in it, which numbers the next.
    opened: usize,
    /// Whether the human put it away, off the rail.
    archived: bool,
    /// Whether the human deleted it: it stays in the projects, so that
    /// the chats of those after it keep the places they are listed by,
    /// with none of its own.
    deleted: bool,
}

/// How far a sketch the human ends is on its way to its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ends {
    /// Its turn goes on; its agent is asked to sum it up once the turn
    /// ends.
    Waiting,
    /// Its agent sums it up; it ends once that turn ends.
    Summing,
}

/// A chat in the list, and, once it is read or opened, the chat itself.
struct Slot {
    key: u64,
    /// Its project, by its place in the projects.
    project: usize,
    kept: Option<ChatId>,
    name: String,
    named: Named,
    /// How it stood when last looked at.
    state: State,
    unread: bool,
    /// How many times its agent had changed a step of the plan the human
    /// left it to when last looked at, as its conversation counts them.
    plan_changes: usize,
    /// Whether the human was told the chat's agent asks them, as it
    /// waits on them, once what the triggers gather for it came.
    asks: bool,
    goal: Option<Goal>,
    parent: Option<u64>,
    setup: Option<Setup>,
    /// The folder it works in, when it is not its project's, as one that
    /// opened before another folder of the project became the project's.
    folder: Option<PathBuf>,
    ending: Option<(Ends, workflow::Ending)>,
    /// The chat, once it was read or opened in this run.
    chat: Option<Chat>,
}

impl Slot {
    /// The chat named `name` as `named` says, keeping it in `store`.
    fn name_as(&mut self, name: &str, named: Named, store: &Store) {
        name.clone_into(&mut self.name);
        self.named = named;
        if let Some(kept) = self.kept
            && let Err(error) = store.name_chat(kept, name, named)
        {
            tracing::error!("the chat's name is not kept: {error:#}");
        }
    }

    fn row(&self, agents: &Agents, store: &Store) -> ChatRow {
        let agent = match &self.chat {
            Some(chat) => chat.agent_id().to_owned(),
            None => agent_of(agents, self.kept, store),
        };
        // A chat not read yet starts again, unless it is of a sketch whose
        // folder is gone from under it.
        let again = match &self.chat {
            Some(chat) => chat.starts_again(),
            None => {
                self.kept.is_some()
                    && self
                        .setup
                        .as_ref()
                        .and_then(|setup| setup.folder.as_ref())
                        .is_none_or(|folder| folder.exists())
            }
        };
        ChatRow {
            key: self.key,
            project: self.project,
            kept: self.kept,
            name: self.name.clone(),
            named: self.named,
            open: self.chat.as_ref().is_some_and(Chat::is_open),
            again,
            state: self.state,
            unread: self.unread,
            goal: self.goal.clone(),
            parent: self.parent,
            setup: self.setup.clone(),
            ending: self.ending.is_some(),
            agent,
            folder: self.folder.clone(),
        }
    }
}

/// A phone that looks at the chats beside the window (see [`phone`]).
struct Viewer {
    /// What it is told: the projects and chats, how the conversation it
    /// watches changes, and what the chats come to.
    tell: UnboundedSender<FromDaemon>,
    /// The chat whose conversation it watches, if it watches one.
    watches: Option<u64>,
}

/// An agent on its way to start, once the window gives its stage a relay.
struct Preparing {
    key: u64,
    generation: u64,
    cwd: PathBuf,
    start: Start,
}

pub struct Daemon {
    /// Cox's own first, then the human's.
    projects: Vec<Project>,
    /// The chats in the order of the tabs.
    chats: Vec<Slot>,
    archived: Vec<Slot>,
    last_key: u64,
    /// Whether `--home` gave the home for this run, so that the settings
    /// do not move it.
    home_given: bool,
    /// The chat the window shows, if it shows one.
    shown: Option<u64>,
    /// The agents that wait for a chat, by its key.
    waiting: Vec<(u64, oneshot::Sender<Result<Waited, String>>)>,
    store: Rc<Store>,
    spawns: Rc<dyn Spawns>,
    internal: UnboundedSender<Internal>,
    /// The window, while one is there.
    window: Option<UnboundedSender<FromDaemon>>,
    /// The chats whose conversations the window watches.
    window_watches: HashSet<u64>,
    /// The phones there, by their connections (see [`phone`]).
    viewers: HashMap<u64, Viewer>,
    /// Where what the chats that need the human come to is pushed to
    /// their phones, if anywhere (see [`push`]).
    push: Option<UnboundedSender<push::Notice>>,
    /// What the window was last told of the projects and chats.
    listed: Option<Listing>,
    /// The commands the window runs, by their numbers, with where their
    /// answers go.
    running: HashMap<u64, oneshot::Sender<Result<Ran, String>>>,
    /// What the agents do with the Mac's windows, by their numbers, with
    /// where what they see goes.
    seeing: HashMap<u64, oneshot::Sender<Result<Seen, String>>>,
    /// What waits for the commands' answers, and stops them once nothing
    /// waits.
    stops: HashMap<u64, Task>,
    /// The agents on their way to start, by the number the window answers
    /// their relay with.
    preparing: HashMap<u64, Preparing>,
    last_id: u64,
    /// The chats the window is to show, once it is told of them.
    reveals: Vec<u64>,
    /// The connection of the window there, when it came over a socket.
    window_id: Option<u64>,
    /// Whether the daemon is to stop.
    done: bool,
    /// When each of the human's schedules last ran (see
    /// [`crate::schedule`]), read as the daemon first looks.
    ran: Option<crate::schedule::Ran>,
    /// The chat of the last run of each schedule, by the path of its
    /// file.
    scheduled: HashMap<String, u64>,
}

/// How the daemon is reached: what to tell it.
#[derive(Clone)]
pub struct Link {
    pub to: UnboundedSender<ToDaemon>,
}

/// The daemon, running on its thread, until it is stopped.
pub struct Running {
    pub link: Link,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    /// Stops the daemon, and its chats' agents with it, and waits until
    /// they have.
    pub fn stop(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the daemon panicked as it stopped");
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Link {
    pub fn send(&self, message: ToDaemon) {
        if self.to.send(message).is_err() {
            tracing::error!("the daemon is gone");
        }
    }
}

/// Starts the daemon on a thread of its own, keeping what lasts in
/// `store`, Cox's own chats working in `home`, given by
/// `--home` when `home_given`, and its agents started by `spawns`, which
/// it makes on its thread. It tells `window` what it does, and answers
/// how to tell it what the human does, and to stop.
///
/// # Panics
///
/// If the system cannot start a thread for it.
pub fn start(
    home: PathBuf,
    home_given: bool,
    store: impl FnOnce() -> Rc<Store> + Send + 'static,
    spawns: impl FnOnce() -> Rc<dyn Spawns> + Send + 'static,
    window: UnboundedSender<FromDaemon>,
) -> Running {
    let (to, from_window) = unbounded_channel();
    let (stop, stopped) = oneshot::channel();
    let thread = std::thread::Builder::new()
        .name("daemon".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::error!("the daemon cannot start: {error}");
                    return;
                }
            };
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, async move {
                let (internal, heard) = unbounded_channel();
                let daemon = Daemon::new(home, home_given, store(), spawns(), internal);
                daemon
                    .serve(Some(window), from_window, heard, stopped)
                    .await;
            });
        })
        .expect("the daemon's thread starts");
    Running {
        link: Link { to },
        stop: Some(stop),
        thread: Some(thread),
    }
}

impl Daemon {
    /// The daemon of the projects and chats kept in `store`, Cox's own
    /// chats working in `home`.
    fn new(
        home: PathBuf,
        home_given: bool,
        store: Rc<Store>,
        spawns: Rc<dyn Spawns>,
        internal: UnboundedSender<Internal>,
    ) -> Self {
        let mut daemon = Self {
            projects: vec![Project {
                id: None,
                name: COX.to_owned(),
                folder: home,
                opened: 0,
                archived: false,
                deleted: false,
            }],
            chats: Vec::new(),
            archived: Vec::new(),
            last_key: 0,
            home_given,
            shown: None,
            waiting: Vec::new(),
            store,
            spawns,
            internal,
            window: None,
            window_watches: HashSet::new(),
            viewers: HashMap::new(),
            push: None,
            listed: None,
            running: HashMap::new(),
            seeing: HashMap::new(),
            stops: HashMap::new(),
            preparing: HashMap::new(),
            last_id: 0,
            reveals: Vec::new(),
            window_id: None,
            done: false,
            ran: None,
            scheduled: HashMap::new(),
        };
        daemon.read_kept();
        daemon
    }

    /// Lists the projects and chats kept, closed.
    fn read_kept(&mut self) {
        let kept = self
            .store
            .projects()
            .and_then(|kept| Ok((kept, self.store.chats()?)));
        let (projects, chats) = match kept {
            Ok(kept) => kept,
            Err(error) => {
                tracing::error!("the chats kept cannot be listed: {error:#}");
                return;
            }
        };
        self.projects
            .extend(projects.into_iter().map(|project| Project {
                id: Some(project.id),
                name: project.name,
                folder: project.folder,
                opened: 0,
                archived: project.archived,
                deleted: false,
            }));
        for chat in chats {
            let Some(index) = self
                .projects
                .iter()
                .position(|project| project.id == chat.project)
            else {
                continue;
            };
            self.projects[index].opened += 1;
            let key = self.next_key();
            // The chats come in the order they were opened, so the parent
            // is listed before its children.
            let parent = chat.parent.and_then(|parent| {
                self.chats
                    .iter()
                    .chain(&self.archived)
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
            let slot = Slot {
                key,
                project: index,
                kept: Some(chat.id),
                name: chat.name,
                named: chat.named,
                state: State::Closed,
                unread: false,
                plan_changes: 0,
                asks: false,
                goal: chat.goal,
                parent,
                setup,
                folder: chat.folder,
                ending: None,
                chat: None,
            };
            if chat.archived {
                self.archived.push(slot);
            } else {
                self.add(slot);
            }
        }
    }

    /// Goes on until the window that started it is gone: hears what the
    /// window says, `from_window`, and what the chats' tasks heard,
    /// `heard`, and tells `window` what changed.
    async fn serve(
        mut self,
        window: Option<UnboundedSender<FromDaemon>>,
        mut from_window: UnboundedReceiver<ToDaemon>,
        mut heard: UnboundedReceiver<Internal>,
        mut stop: oneshot::Receiver<()>,
    ) {
        self.window = window;
        self.tell_listing();
        loop {
            tokio::select! {
                message = from_window.recv() => match message {
                    Some(message) => self.act(message),
                    None => break,
                },
                Some(event) = heard.recv() => self.hear(event),
                _ = &mut stop => break,
            }
            if self.done {
                break;
            }
            self.settle();
        }
        // The chats' agents stop with them.
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            slot.chat = None;
        }
    }

    fn next_key(&mut self) -> u64 {
        self.last_key += 1;
        self.last_key
    }

    fn next_id(&mut self) -> u64 {
        self.last_id += 1;
        self.last_id
    }

    fn tell(&self, message: FromDaemon) {
        if let Some(window) = &self.window {
            let _ = window.send(message);
        }
    }

    /// Adds `slot` to the list, after the chat that opened it and those it
    /// opened before, or at the end of its project's, and answers where.
    fn add(&mut self, slot: Slot) -> usize {
        let parent = slot
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
                .rposition(|other| other.project <= slot.project)
                .map_or(0, |index| index + 1),
        };
        self.chats.insert(after, slot);
        after
    }

    /// How many chats opened `slot`, one under another.
    fn depth(&self, slot: &Slot) -> u8 {
        let mut depth = 0_u8;
        let mut parent = slot.parent;
        while let Some(key) = parent {
            let Some(found) = self.chats.iter().find(|other| other.key == key) else {
                break;
            };
            depth = depth.saturating_add(1);
            parent = found.parent;
        }
        depth
    }

    fn slot(&self, key: u64) -> Option<&Slot> {
        self.chats
            .iter()
            .chain(&self.archived)
            .find(|slot| slot.key == key)
    }

    fn slot_mut(&mut self, key: u64) -> Option<&mut Slot> {
        self.chats
            .iter_mut()
            .chain(&mut self.archived)
            .find(|slot| slot.key == key)
    }

    /// The chat `key`, read from what is kept of it if it was not yet.
    fn chat(&mut self, key: u64) -> Option<&mut Chat> {
        self.read(key);
        self.slot_mut(key)?.chat.as_mut()
    }

    /// Where the chat of `slot` works: the folder of its sketch, or its
    /// project's.
    fn cwd(&self, slot: &Slot) -> PathBuf {
        slot.setup
            .as_ref()
            .and_then(|setup| setup.folder.clone())
            .or_else(|| slot.folder.clone())
            .unwrap_or_else(|| self.projects[slot.project].folder.clone())
    }

    /// Reads the closed chat `key` from what is kept of it, unless it was
    /// read or opened in this run. Once the human sends to it, its agent
    /// starts again, going on with its session if it can.
    fn read(&mut self, key: u64) {
        let Some(slot) = self.slot(key) else {
            return;
        };
        let (Some(kept), None) = (slot.kept, &slot.chat) else {
            return;
        };
        let cwd = self.cwd(slot);
        let setup = slot.setup.clone();
        let goal = slot.goal.clone();
        let entries = self.store.entries(kept).unwrap_or_else(|error| {
            tracing::error!("the chat cannot be read: {error:#}");
            vec![Entry::Note(format!("Cox cannot read this chat: {error:#}"))]
        });
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
        // A chat of a sketch whose folder is gone from under it does not
        // start again.
        let gone = setup.as_ref().is_some_and(|setup| setup.folder.is_some()) && !cwd.exists();
        let keeps = keeps_in_wiki(setup.as_ref());
        let again = (!gone).then_some(Again {
            cwd,
            session,
            setup,
        });
        let baseline = self.store.baseline(kept).unwrap_or_else(|error| {
            tracing::error!("the chat's plan cannot be read: {error:#}");
            None
        });
        let conversation = Conversation::closed(entries)
            .with_baseline(baseline)
            .with_goal(goal)
            .with_settings(settings)
            .with_keeping(keeps);
        let record = Record::new(Rc::clone(&self.store), kept);
        let agents = self.spawns.agents();
        let chosen = agent_of(&agents, Some(kept), &self.store);
        let starts = self.spawns.starts(&chosen);
        let chat = Chat::new(
            key,
            conversation,
            Some(record),
            again,
            Some(starts),
            Some((agents, chosen)),
            self.internal.clone(),
        );
        if let Some(slot) = self.slot_mut(key) {
            slot.chat = Some(chat);
            slot.state = State::Closed;
        }
    }

    /// Reads the chat `key` again from what is kept of it, as when it
    /// moves out of a sketch, telling the window that watches it all of
    /// it again.
    fn read_again(&mut self, key: u64) {
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        let watched = slot.chat.as_ref().is_some_and(|chat| chat.watched);
        if let Some(mut chat) = slot.chat.take() {
            chat.flush();
        }
        if watched {
            if self.window_watches.contains(&key) {
                self.watch(key);
            }
            let phones: Vec<u64> = self
                .viewers
                .iter()
                .filter(|(_, viewer)| viewer.watches == Some(key))
                .map(|(&id, _)| id)
                .collect();
            for id in phones {
                self.view_chat(id, Some(key));
            }
        }
    }

    /// Has the window watch the chat `key`: tells it all of its
    /// conversation, and the tabs of its folder's stage as they were kept.
    fn watch(&mut self, key: u64) {
        let Some(chat) = self.chat(key) else {
            return;
        };
        let patch = chat.watch();
        let tabs = self
            .slot(key)
            .and_then(|slot| self.stage_in(&self.cwd(slot)));
        self.window_watches.insert(key);
        self.tell(FromDaemon::Tabs { key, tabs });
        self.tell(FromDaemon::Patch { key, patch });
    }

    /// Has the phone of the connection `id` hear on `phone` the projects
    /// and chats, as the window does.
    fn view(&mut self, id: u64, phone: UnboundedSender<FromDaemon>) {
        tracing::info!(phone = id, "a phone came");
        if let Some(listing) = &self.listed {
            let _ = phone.send(FromDaemon::Listing(listing.clone()));
        }
        self.viewers.insert(
            id,
            Viewer {
                tell: phone,
                watches: None,
            },
        );
        if self.listed.is_none() {
            self.tell_listing();
        }
    }

    /// Has the phone of the connection `id` watch the chat `key`, or none:
    /// tells it all of the chat's conversation.
    fn view_chat(&mut self, id: u64, key: Option<u64>) {
        let patch = key.and_then(|key| Some((key, self.chat(key)?.watch())));
        let Some(viewer) = self.viewers.get_mut(&id) else {
            return;
        };
        viewer.watches = key;
        if let Some((key, patch)) = patch {
            let _ = viewer.tell.send(FromDaemon::Patch { key, patch });
        }
    }

    /// Tells how the conversation of the chat `key` changed to the window
    /// and the phones that watch it.
    fn tell_patch(&self, key: u64, patch: Patch) {
        for viewer in self.viewers.values() {
            if viewer.watches == Some(key) {
                let _ = viewer.tell.send(FromDaemon::Patch {
                    key,
                    patch: patch.clone(),
                });
            }
        }
        if self.window_watches.contains(&key) {
            self.tell(FromDaemon::Patch { key, patch });
        }
    }

    /// Pushes the news `message` to the human's phones, unless they are at
    /// the Mac, which tells them itself, or look at the chat on a phone.
    fn push(&self, message: &FromDaemon) {
        let (Some(push), FromDaemon::News { key, name, body }) = (&self.push, message) else {
            return;
        };
        if self.at_the_mac() {
            tracing::debug!(chat = key, "not pushed: the human is at the Mac");
            return;
        }
        if self
            .viewers
            .values()
            .any(|viewer| viewer.watches == Some(*key))
        {
            tracing::debug!(chat = key, "not pushed: the human looks at it on a phone");
            return;
        }
        let _ = push.send(push::Notice {
            key: *key,
            title: name.clone(),
            body: body.clone(),
        });
    }

    /// Whether the human is at the Mac: a window is there, and they used
    /// the Mac lately, where Cox can tell.
    fn at_the_mac(&self) -> bool {
        self.window.is_some() && cox_mac::idle().is_none_or(|idle| idle < AWAY)
    }

    /// Tells the phones `message`, as the window is told it.
    fn tell_phones(&self, message: &FromDaemon) {
        for viewer in self.viewers.values() {
            let _ = viewer.tell.send(message.clone());
        }
    }

    /// The tabs of the stage of `folder`, each of a chat by its key, as
    /// they were kept, or none for a folder with none kept yet.
    fn stage_in(&self, folder: &Path) -> Option<KeptTabs<u64>> {
        let tabs = self
            .store
            .stage(folder)
            .inspect_err(|error| tracing::error!("the stage's tabs cannot be read: {error:#}"))
            .ok()?
            .unwrap_or_default();
        Some(tabs.owned(|owner| {
            self.chats
                .iter()
                .chain(&self.archived)
                .find(|chat| chat.kept == Some(owner))
                .map(|chat| chat.key)
        }))
    }

    /// The project of `folder`, by its place in the projects, made and
    /// kept if there is none yet.
    fn project(&mut self, folder: PathBuf) -> usize {
        if let Some(index) = self
            .projects
            .iter()
            .position(|project| project.folder == folder && !project.deleted)
        {
            // A project put away comes back as its folder is opened again.
            if self.projects[index].archived {
                self.put_away(index, false);
            }
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
        self.projects.push(Project {
            id,
            name,
            folder,
            opened: 0,
            archived: false,
            deleted: false,
        });
        self.projects.len() - 1
    }

    /// Opens a chat in the project of `folder`, or of Cox's own, and has
    /// the window show it.
    fn open(&mut self, folder: Option<PathBuf>) -> u64 {
        let index = folder.map_or(0, |folder| self.project(folder));
        let key = self.open_in(index, None, None);
        self.reveals.push(key);
        key
    }

    /// The folder of a project with nothing set yet, `untitled-` and the
    /// first number no folder there has yet, in the projects of Cox's
    /// home.
    fn untitled(&self) -> PathBuf {
        let projects = self.projects[0].folder.join(crate::settings::PROJECTS);
        (1..=u32::MAX)
            .map(|number| projects.join(format!("untitled-{number}")))
            .find(|folder| {
                !folder.exists()
                    && self
                        .projects
                        .iter()
                        .all(|project| project.deleted || project.folder != *folder)
            })
            .expect("some number is free")
    }

    /// Makes a project with nothing set yet, in a folder of its own (see
    /// [`Self::untitled`]), and answers its folder.
    fn new_project(&mut self) -> Option<PathBuf> {
        let folder = self.untitled();
        if let Err(error) = std::fs::create_dir_all(&folder) {
            tracing::error!("cannot make {}: {error}", folder.display());
            return None;
        }
        Some(folder)
    }

    /// Opens a chat in the project at `index`, under the chat `from` names
    /// by its key, and how it is kept, when an agent opens it, set up as
    /// `setup` says; and answers its key.
    fn open_in(
        &mut self,
        index: usize,
        from: Option<(u64, Option<ChatId>)>,
        setup: Option<Setup>,
    ) -> u64 {
        // A chat opened from a chat that keeps to another folder of the
        // project works there too.
        let folder = from
            .and_then(|(parent, _)| self.slot(parent))
            .and_then(|parent| parent.folder.clone())
            .filter(|_| setup.as_ref().is_none_or(|setup| setup.folder.is_none()));
        let project = &mut self.projects[index];
        if index == 0 {
            // The agent's session fails to open, and says so, if the home
            // cannot be made.
            if let Err(error) = std::fs::create_dir_all(&project.folder) {
                tracing::error!("cannot make {}: {error}", project.folder.display());
            }
        }
        let cwd = folder.clone().unwrap_or_else(|| project.folder.clone());
        let instructions = Instructions::read(&cwd);
        project.opened += 1;
        let name = format!("Chat {}", project.opened);
        let project_id = project.id;
        let kept = match from {
            Some((_, Some(parent))) => self.store.add_chat_under(
                parent,
                project_id,
                &name,
                setup.as_ref().map(Setup::to_json).as_deref(),
            ),
            _ => self.store.add_chat(project_id, &name),
        }
        .inspect_err(|error| tracing::error!("the chat is not kept: {error:#}"))
        .ok();
        if let (Some(kept), Some(folder)) = (kept, &folder)
            && let Err(error) = self.store.keep_folder(kept, Some(folder))
        {
            tracing::error!("the chat's folder is not kept: {error:#}");
        }
        let record = kept.map(|kept| Record::new(Rc::clone(&self.store), kept));
        let key = self.next_key();
        let agents = self.spawns.agents();
        let chosen = agent_of(&agents, kept, &self.store);
        let starts = self.spawns.starts(&chosen);
        let conversation = Chat::opening(&instructions.paths(), setup.as_ref());
        let chat = Chat::new(
            key,
            conversation,
            record,
            None,
            Some(starts),
            Some((agents, chosen)),
            self.internal.clone(),
        );
        self.add(Slot {
            key,
            project: index,
            kept,
            name,
            named: Named::Numbered,
            state: State::Idle,
            unread: false,
            plan_changes: 0,
            asks: false,
            goal: None,
            parent: from.map(|(parent, _)| parent),
            setup: setup.clone(),
            folder,
            ending: None,
            chat: Some(chat),
        });
        // Its triggers run on its opening once its agent has started.
        let in_sketch = self.sketch_of(key).is_some();
        if let Some(chat) = self.chat(key) {
            chat.opened(in_sketch);
        }
        self.start_agent(
            key,
            Start::New {
                instructions: Some(instructions),
                setup,
            },
        );
        key
    }

    /// Starts the agent of the chat `key` as `start` says: at once, or,
    /// for an agent that drives the stage while a window is there, once
    /// the window gives the stage a relay.
    fn start_agent(&mut self, key: u64, start: Start) {
        let Some(slot) = self.slot(key) else {
            return;
        };
        let cwd = start_cwd(&start).unwrap_or_else(|| self.cwd(slot));
        let drives = start
            .setup()
            .is_none_or(|setup| setup.stage == StageUse::Drive);
        let Some(chat) = self.slot_mut(key).and_then(|slot| slot.chat.as_mut()) else {
            return;
        };
        let generation = chat.next_generation();
        if drives && self.window.is_some() {
            let id = self.next_id();
            self.tell(FromDaemon::Prepare {
                key,
                cwd: cwd.clone(),
                id,
            });
            self.preparing.insert(
                id,
                Preparing {
                    key,
                    generation,
                    cwd,
                    start,
                },
            );
        } else {
            self.spawn_agent(key, generation, cwd, start, None);
        }
    }

    /// Starts the agent of `generation` of the chat `key`, working in
    /// `cwd`, as `start` says, its browser tools reaching the stage through
    /// the relay on `relay`, when given.
    fn spawn_agent(
        &mut self,
        key: u64,
        generation: u64,
        cwd: PathBuf,
        start: Start,
        relay: Option<u16>,
    ) {
        let span = tracing::info_span!("chat", id = tracing::field::Empty, cwd = %cwd.display());
        let Some(slot) = self.slot(key) else {
            return;
        };
        if let Some(kept) = slot.kept {
            span.record("id", tracing::field::display(kept));
        }
        let _entered = span.enter();
        let Some(chat) = &slot.chat else {
            return;
        };
        let agent = chat.agent_id().to_owned();
        let Spawned { agent, stage } = self.spawns.chat(cwd, start, &agent, relay);
        let also = stage
            .map(|mut stage| {
                let internal = self.internal.clone();
                vec![spawn(async move {
                    while let Some(request) = stage.recv().await {
                        if internal.send(Internal::Stage { key, request }).is_err() {
                            break;
                        }
                    }
                })]
            })
            .unwrap_or_default();
        if let Some(chat) = self.slot_mut(key).and_then(|slot| slot.chat.as_mut()) {
            chat.attach(agent, generation, also);
        }
    }

    /// Does what the window says the human did.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm for each thing the human does"
    )]
    fn act(&mut self, message: ToDaemon) {
        match message {
            ToDaemon::Open { folder } => {
                self.open(folder);
            }
            ToDaemon::NewProject => {
                if let Some(folder) = self.new_project() {
                    self.open(Some(folder));
                }
            }
            ToDaemon::NewProjectAside => {
                if let Some(folder) = self.new_project() {
                    let index = self.project(folder);
                    self.open_in(index, None, None);
                }
            }
            ToDaemon::NewChat { folder } => {
                // Only in a project the human has, whose folder is there.
                let index = match folder {
                    None => Some(0),
                    Some(folder) => self
                        .projects
                        .iter()
                        .position(|project| project.folder == folder),
                };
                if let Some(index) = index.filter(|&index| self.projects[index].folder.is_dir()) {
                    self.open_in(index, None, None);
                }
            }
            ToDaemon::Shown { key } => {
                self.shown = key;
                if let Some(slot) = key.and_then(|key| self.slot_mut(key)) {
                    slot.unread = false;
                }
            }
            ToDaemon::Watch { key } => self.watch(key),
            ToDaemon::Submit {
                key,
                text,
                attached,
            } => self.submit(key, &text, attached),
            ToDaemon::Choose { key, index } => {
                if let Some(chat) = self.chat(key) {
                    chat.choose(index);
                }
            }
            ToDaemon::Stop { key } => {
                if let Some(chat) = self.chat(key) {
                    chat.stop();
                }
            }
            ToDaemon::SetGoal { key, goal } => {
                if let Some(chat) = self.chat(key) {
                    chat.set_goal(goal);
                }
            }
            ToDaemon::AnswerProposal { key, index, answer } => {
                if let Some(chat) = self.chat(key) {
                    chat.answer_proposal(index, answer);
                }
            }
            ToDaemon::ChangeProposal { key, index, goal } => {
                if let Some(chat) = self.chat(key) {
                    chat.change_proposal(index, goal);
                }
            }
            ToDaemon::AnswerNameProposal { key, index, answer } => {
                self.answer_name_proposal(key, index, answer);
            }
            ToDaemon::AnswerKeep { key, index, keep } => {
                let keeps = self
                    .chat(key)
                    .and_then(|chat| chat.answer_keep(index, keep));
                if let Some(chat) = keeps {
                    self.save_to_wiki_of(key, chat);
                }
            }
            ToDaemon::StartSketch { key, sketch, what } => {
                match self.start_sketch(key, sketch, &what) {
                    Ok(opened) => self.reveals.push(opened.number),
                    Err(why) => {
                        if let Some(chat) = self.chat(key) {
                            chat.note(format!("The sketch cannot start: {why}"));
                        }
                    }
                }
            }
            ToDaemon::Set { key, id, value } => {
                if let Some(chat) = self.chat(key) {
                    chat.set(&id, &value);
                }
            }
            ToDaemon::SwitchAgent { key, agent } => self.switch_agent(key, &agent),
            ToDaemon::Rename { key, name } => {
                let store = Rc::clone(&self.store);
                if let Some(slot) = self.slot_mut(key) {
                    slot.name_as(&name, Named::Human, &store);
                }
            }
            ToDaemon::Archive { key, archived } => self.archive(key, archived),
            ToDaemon::SaveToWiki { key } => {
                if self.save_to_wiki(key) {
                    self.reveals.push(key);
                }
            }
            ToDaemon::Merge { key } => self.merge(key),
            ToDaemon::EndSketch { key, ending } => {
                self.end_sketch_as(key, ending);
            }
            ToDaemon::StageOf { folder } => {
                let tabs = self.stage_in(&folder);
                self.tell(FromDaemon::StageOf { folder, tabs });
            }
            ToDaemon::Tabs { folder, tabs } => {
                let tabs = tabs.owned(|key| self.slot(key).and_then(|slot| slot.kept));
                if let Err(error) = self.store.keep_stage(&folder, &tabs) {
                    tracing::error!("the stage's tabs are not kept: {error:#}");
                }
            }
            ToDaemon::Seen { id, seen } => {
                if let Some(answer) = self.seeing.remove(&id) {
                    answer.send(seen).ok();
                }
            }
            ToDaemon::Ran { id, ran } => {
                if let Some(answer) = self.running.remove(&id) {
                    answer.send(ran).ok();
                }
                // What waits for the answer passes it on, and ends: stopped
                // now, it would drop the answer before it did.
                if let Some(waits) = self.stops.remove(&id) {
                    waits.detach();
                }
            }
            ToDaemon::Prepared { id, port } => {
                if let Some(Preparing {
                    key,
                    generation,
                    cwd,
                    start,
                }) = self.preparing.remove(&id)
                {
                    self.spawn_agent(key, generation, cwd, start, port);
                }
            }
            ToDaemon::Home { home, set } => {
                if set {
                    self.home_given = false;
                    self.projects[0].folder = home;
                } else if !self.home_given {
                    self.projects[0].folder = home;
                }
            }
            ToDaemon::RenameProject { project, name } => self.rename_project(project, &name),
            ToDaemon::ArchiveProject { project, archived } => {
                if let Some(index) = self.index_of(project) {
                    self.put_away(index, archived);
                }
            }
            ToDaemon::DeleteProject { project } => {
                if let Some(index) = self.index_of(project)
                    && let Err(why) = self.delete_project(index)
                {
                    tracing::error!("the project is not deleted: {why}");
                }
            }
            ToDaemon::Sync { id } => {
                // What came before is done, but for what it changed, which
                // is told before this.
                self.settle();
                self.tell(FromDaemon::Settled { id });
            }
        }
    }

    /// Renames the project kept as `project` `name`, and its folder with
    /// it, in the folder it is in, unless the name is no folder's, a
    /// folder there has it already, or one of its chats is at work.
    fn rename_project(&mut self, project: ProjectId, name: &str) {
        let Some(index) = self.index_of(project) else {
            return;
        };
        if let Err(why) = self.rename(index, name) {
            tracing::error!(name, "the project is not renamed: {why}");
        }
    }

    /// The folder the project at `index` would have, renamed `name`, or
    /// why it cannot be renamed so.
    fn renamed(&self, index: usize, name: &str) -> Result<PathBuf, String> {
        if index == 0 {
            return Err("Cox's own chats are in no project.".to_owned());
        }
        let name = name.trim();
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
            return Err("The name is its folder's: no / in it, nor a . first.".to_owned());
        }
        let from = &self.projects[index].folder;
        let to = from.with_file_name(name);
        // A name that differs only in case is the same folder where the
        // disk does not tell cases apart, as on macOS.
        if to != *from && to.exists() && to.canonicalize().ok() != from.canonicalize().ok() {
            return Err("There is a folder of that name there already.".to_owned());
        }
        if self.at_work(index) {
            return Err(
                "A chat of the project is at work: rename it once the turn ends.".to_owned(),
            );
        }
        Ok(to)
    }

    /// Renames the project at `index` `name`, and its folder with it, or
    /// answers why not. Its directory named as it was is renamed with it,
    /// unless it has one of the new name already.
    fn rename(&mut self, index: usize, name: &str) -> Result<(), String> {
        let to = self.renamed(index, name)?;
        let from = self.projects[index].folder.clone();
        if to == from {
            return Ok(());
        }
        let directory = self.rename_directory(&from, &to);
        if let Err(error) = std::fs::rename(&from, &to) {
            if let Some(directory) = directory {
                self.name_back(&from, &directory);
            }
            return Err(format!("{} is not renamed: {error}", from.display()));
        }
        if !self.relocate(index, &to) {
            if let Err(error) = std::fs::rename(&to, &from) {
                tracing::error!("{} is not named back: {error}", to.display());
            } else if let Some(directory) = directory {
                self.name_back(&from, &directory);
            }
            return Err("The project does not move.".to_owned());
        }
        Ok(())
    }

    /// Renames the directory of the project in `from` named as it is,
    /// as the project is renamed, its folder going to `to`, and has what
    /// Cox keeps of the stage there name where it is now; answers the
    /// directory renamed, if one is.
    fn rename_directory(&self, from: &Path, to: &Path) -> Option<Renamed> {
        let directories = from.join(crate::settings::DIRECTORIES);
        let was = directories.join(from.file_name()?);
        let now = directories.join(to.file_name()?);
        // A name that differs only in case is the same folder where the
        // disk does not tell cases apart, as on macOS.
        if !was.is_dir() || now.exists() && now.canonicalize().ok() != was.canonicalize().ok() {
            return None;
        }
        let renamed = Renamed { was, now };
        self.move_directory(from, &renamed.was, &renamed.now)
            .then_some(renamed)
    }

    /// Names the directory of the project in `project` that `renamed`
    /// says was renamed as it was again, as when the project is not
    /// renamed after all.
    fn name_back(&self, project: &Path, renamed: &Renamed) {
        self.move_directory(project, &renamed.now, &renamed.was);
    }

    /// Moves the directory `from` of the project in `project` to `to`,
    /// with what Cox keeps of the stage there; answers whether it moved.
    fn move_directory(&self, project: &Path, from: &Path, to: &Path) -> bool {
        if let Err(error) = std::fs::rename(from, to) {
            tracing::error!("{} is not renamed: {error}", from.display());
            return false;
        }
        let moved = cox_core::moved::Moved { from, to };
        if let Err(error) = self.store.move_in_stage(project, moved) {
            tracing::error!("the stage's tabs do not follow the directory: {error:#}");
        }
        if let Err(error) = storage::moved(moved) {
            tracing::error!("what Cox keeps of the directory does not move: {error}");
        }
        true
    }

    /// Shows `name`, which the agent of the chat `key` proposes for its
    /// project, on a card in the chat's conversation, and answers the
    /// project's name now, or why it cannot be named so.
    fn propose_name(
        &mut self,
        key: u64,
        name: &str,
        why: Option<String>,
    ) -> Result<String, String> {
        let project = self.slot(key).ok_or("the chat has closed")?.project;
        if project == 0 {
            return Err(
                "This chat is in no project: Cox's own chats have no name to propose.".to_owned(),
            );
        }
        let name = name.trim();
        if name.starts_with('.') || name.contains(['/', '\\']) {
            return Err(
                "The name is the project's folder's: no / or \\ in it, nor a . first.".to_owned(),
            );
        }
        let now = self.projects[project]
            .folder
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        self.chat(key)
            .ok_or("the chat has closed")?
            .propose_name(name, why);
        Ok(now)
    }

    /// The human answered the name proposed at `index` in the chat `key`
    /// for its project: renames it so, or lets it be; or, when it cannot
    /// be renamed so, notes why, the card staying for them to answer.
    fn answer_name_proposal(&mut self, key: u64, index: usize, answer: Answer) {
        let Some(project) = self.slot(key).map(|slot| slot.project) else {
            return;
        };
        let Some(name) = self.chat(key).and_then(|chat| chat.proposed_name(index)) else {
            return;
        };
        if answer == Answer::Dismissed {
            if let Some(chat) = self.chat(key) {
                chat.answer_name_proposal(index, answer);
            }
            return;
        }
        if let Err(why) = self.renamed(project, &name) {
            if let Some(chat) = self.chat(key) {
                chat.note(format!("The project is not renamed {name}: {why}"));
            }
            return;
        }
        if let Some(chat) = self.chat(key) {
            chat.answer_name_proposal(index, Answer::Set);
        }
        if let Err(why) = self.rename(project, &name)
            && let Some(chat) = self.chat(key)
        {
            chat.note(format!("The project is not renamed {name}: {why}"));
        }
    }

    /// Whether a chat of the project at `index` is at work.
    fn at_work(&self, index: usize) -> bool {
        self.chats
            .iter()
            .chain(&self.archived)
            .any(|slot| slot.project == index && working(slot.state))
    }

    /// Has the project at `index`, whose folder is now `folder`, work
    /// there, its chats noting it and starting again there, and answers
    /// whether it does, which it does not while one of its chats is at
    /// work.
    fn relocate(&mut self, index: usize, folder: &Path) -> bool {
        let Some(project) = self.projects[index].id else {
            return false;
        };
        let from = self.projects[index].folder.clone();
        if self.at_work(index) {
            tracing::error!("the project does not move while a chat of it is at work");
            return false;
        }
        let name = folder.file_name().map_or_else(
            || folder.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        if let Err(error) = self.store.move_project(project, folder, &name) {
            tracing::error!("the project does not move: {error:#}");
            return false;
        }
        let moved = cox_core::moved::Moved {
            from: &from,
            to: folder,
        };
        if let Err(error) = storage::moved(moved) {
            tracing::error!("what Cox keeps of the folder's triggers does not move: {error}");
        }
        self.scheduled = std::mem::take(&mut self.scheduled)
            .into_iter()
            .map(|(id, key)| (moved.text(&id).unwrap_or(id), key))
            .collect();
        tracing::info!(from = %from.display(), to = %folder.display(), "the project moved");
        folder.clone_into(&mut self.projects[index].folder);
        self.projects[index].name = name;
        let note = format!(
            "The project's folder moved from {} to {}. A line sent starts the agent again there, \
             with the conversation so far.",
            from.display(),
            folder.display()
        );
        let mut moving = Vec::new();
        let mut sketches = Vec::new();
        let store = Rc::clone(&self.store);
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            // A chat that keeps to another folder of the project stays
            // there.
            if slot.project != index || slot.folder.is_some() {
                continue;
            }
            // A chat of a sketch works in the sketch's folder, which
            // stays where it is; a sketch from the project is a worktree
            // of its repository, which Git is told moved.
            if let Some(setup) = &slot.setup
                && let Some(works) = &setup.folder
            {
                if setup.sketch == Some(Sketch::Project) && works.exists() {
                    sketches.push(works.clone());
                }
                continue;
            }
            if let Some(kept) = slot.kept {
                // The agent's session was of the folder it worked in.
                if let Err(error) = store.forget_session(kept) {
                    tracing::error!("the chat's session is not forgotten: {error:#}");
                }
            }
            if let Some(chat) = &mut slot.chat {
                chat.note(note.clone());
            } else if let Some(kept) = slot.kept {
                let entries = store.entries(kept).map(|entries| entries.len());
                if let Ok(number) = entries
                    && let Err(error) = store.keep_entry(kept, number, &Entry::Note(note.clone()))
                {
                    tracing::error!("the chat is not kept as it goes: {error:#}");
                }
            }
            slot.state = State::Closed;
            moving.push(slot.key);
        }
        if !sketches.is_empty() {
            sketch::repair(folder, &sketches);
        }
        // The window hears where the project is before it hears the
        // chats again, with the tabs of its stage there.
        self.tell_listing();
        for key in moving {
            self.read_again(key);
        }
        true
    }

    /// The project kept as `project`, by its place in the projects.
    fn index_of(&self, project: ProjectId) -> Option<usize> {
        self.projects
            .iter()
            .position(|kept| kept.id == Some(project) && !kept.deleted)
    }

    /// Puts the project at `index` away, off the rail, its chats going on
    /// as they were, or brings it back.
    fn put_away(&mut self, index: usize, archived: bool) {
        let Some(id) = self.projects[index].id else {
            return;
        };
        if let Err(error) = self.store.archive_project(id, archived) {
            tracing::error!("whether the project is archived is not kept: {error:#}");
            return;
        }
        self.projects[index].archived = archived;
    }

    /// Deletes the project at `index`, unless one of its chats is at
    /// work: its folder goes to the trash, whole, for the human to take
    /// back from there; its chats stop, and are forgotten with what they
    /// showed, the tabs of its stage and its icon.
    fn delete_project(&mut self, index: usize) -> Result<(), String> {
        let Some(id) = self.projects[index].id else {
            return Err("Cox's own chats are in no project.".to_owned());
        };
        if self.at_work(index) {
            return Err(
                "A chat of the project is at work: delete it once the turn ends.".to_owned(),
            );
        }
        let folder = self.projects[index].folder.clone();
        if folder.exists()
            && let Err(why) = crate::trash::put(&folder)
        {
            // The trash can fail after it took the folder, as for a name
            // it was given beside it; the folder gone is what counts.
            if folder.exists() {
                return Err(why);
            }
            tracing::warn!("the trash took the folder, but said: {why}");
        }
        tracing::info!(folder = %folder.display(), "the project is deleted");
        if let Err(error) = self.store.delete_project(id) {
            tracing::error!("the project's chats are not forgotten: {error:#}");
        }
        if let Err(error) = crate::project_icon::remove(id) {
            tracing::error!("the project's icon is not removed: {error:#}");
        }
        let mut gone = HashSet::new();
        for chats in [&mut self.chats, &mut self.archived] {
            for mut slot in chats.extract_if(.., |slot| slot.project == index) {
                if let Some(chat) = &mut slot.chat {
                    chat.close("The project was deleted.".to_owned());
                }
                gone.insert(slot.key);
            }
        }
        // An agent that waits for one of them waits no more.
        for (_, answer) in self.waiting.extract_if(.., |(key, _)| gone.contains(key)) {
            answer
                .send(Err("the chat's project was deleted".to_owned()))
                .ok();
        }
        let project = &mut self.projects[index];
        project.deleted = true;
        project.archived = false;
        Ok(())
    }

    /// The human sent `text` in the chat `key`: a closed chat's agent
    /// starts again first, if it can.
    fn submit(&mut self, key: u64, text: &str, attached: Vec<Attachment>) {
        if text.trim().is_empty() {
            return;
        }
        let Some(chat) = self.chat(key) else {
            return;
        };
        let earlier = chat.conversation().entries().to_vec();
        if let Some(again) = chat.take_again() {
            let Again {
                cwd,
                session,
                setup,
            } = again;
            let start = Start::again(&cwd, session, &earlier, setup);
            self.start_agent(key, start);
        }
        if let Some(chat) = self.chat(key) {
            chat.submit(text, attached);
        }
    }

    /// Sends `text` to the agent of the chat `key` in the human's place,
    /// starting it again first if it is closed and can be.
    fn go_on(&mut self, key: u64, text: &str) {
        self.start_again(key);
        if let Some(chat) = self.chat(key) {
            chat.told(text);
        }
    }

    /// Starts the agent of the chat `key` again, if it is closed and can
    /// be.
    fn start_again(&mut self, key: u64) {
        let Some(chat) = self.chat(key) else {
            return;
        };
        let earlier = chat.conversation().entries().to_vec();
        if let Some(again) = chat.take_again() {
            let Again {
                cwd,
                session,
                setup,
            } = again;
            self.start_agent(key, Start::again(&cwd, session, &earlier, setup));
        }
    }

    /// Does what the daemon's chats' tasks heard.
    #[expect(clippy::too_many_lines, reason = "one arm for each thing heard")]
    fn hear(&mut self, event: Internal) {
        match event {
            Internal::Agent {
                key,
                generation,
                event,
            } => {
                let in_sketch = matches!(
                    event,
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn) | AgentEvent::Plan(_)
                ) && self.sketch_of(key).is_some();
                if let Some(chat) = self.chat(key) {
                    chat.hear(generation, event, in_sketch);
                }
            }
            Internal::Asked {
                key,
                generation: _,
                request,
            } => self.asked(key, request),
            Internal::Stage { key, request } => self.ask_stage(key, request),
            Internal::Checked {
                key,
                generation,
                ran,
            } => {
                if let Some(chat) = self.chat(key) {
                    chat.heard_check(generation, ran);
                }
            }
            Internal::LoggedIn {
                key,
                generation,
                ran,
            } => {
                if let Some(chat) = self.chat(key) {
                    chat.logged_in(generation, ran);
                }
            }
            Internal::Judge {
                key,
                generation,
                heard,
            } => {
                if let Some(chat) = self.chat(key) {
                    chat.heard_judge(generation, heard);
                }
            }
            Internal::Trigger {
                key,
                generation,
                file,
                heard,
            } => {
                if let Some(chat) = self.chat(key) {
                    chat.heard_trigger(generation, &file, heard);
                }
            }
            Internal::Gathered {
                key,
                generation,
                file,
                gathered,
            } => {
                if let Some(chat) = self.chat(key) {
                    chat.heard_gathered(generation, &file, gathered);
                }
            }
            Internal::GatheredEnough { key, generation } => {
                if let Some(chat) = self.chat(key) {
                    chat.gathered_enough(generation);
                }
            }
            Internal::Cancel { id } => {
                self.stops.remove(&id);
                if self.running.remove(&id).is_some() {
                    self.tell(FromDaemon::Cancel { id });
                }
            }
            Internal::Attach {
                id,
                hello,
                window,
                replaced,
                answer,
            } => {
                let taken = self.attach(id, hello, window, replaced);
                answer.send(taken).ok();
            }
            Internal::Detach { id } => {
                if self.window_id == Some(id) {
                    self.detach();
                }
            }
            Internal::View { id, phone } => self.view(id, phone),
            Internal::ViewChat { id, key } => {
                // The human looks at the chat, on their phone.
                if let Some(slot) = key.and_then(|key| self.slot_mut(key)) {
                    slot.unread = false;
                }
                self.view_chat(id, key);
            }
            Internal::Act { message } => self.act(message),
            Internal::Folder { key, answer } => {
                answer.send(self.slot(key).map(|slot| self.cwd(slot))).ok();
            }
            Internal::Sketching { key, answer } => {
                answer.send(self.sketching(key)).ok();
            }
            Internal::Push { to } => self.push = Some(to),
            Internal::Unview { id } => {
                if self.viewers.remove(&id).is_some() {
                    tracing::info!(phone = id, "a phone went");
                }
            }
            Internal::Quit => self.done = true,
            Internal::Schedules => {
                self.run_schedules(&crate::settings::workflow(), &jiff::Zoned::now());
            }
        }
    }

    /// Runs each of the triggers on a schedule, of the human's `workflow`
    /// and of the projects, that is due `now` and turned on where it
    /// works, unless the chat of its last run is still at work: a chat for
    /// each run, in the project of the folder it works in, or of Cox's
    /// own, on the trigger's text, sent in the human's place, its agent
    /// going on unasked with what the trigger allows (see
    /// [`crate::schedule`]).
    fn run_schedules(&mut self, workflow: &Path, now: &jiff::Zoned) {
        // The first is Cox's own, in the home.
        let home = self.projects[0].folder.clone();
        // A project put away, or deleted, runs none of its own.
        let projects: Vec<PathBuf> = self.projects[1..]
            .iter()
            .filter(|project| !project.archived && !project.deleted)
            .map(|project| project.folder.clone())
            .collect();
        let scheduled = crate::trigger::scheduled(workflow, &home, &projects);
        if scheduled.is_empty() {
            return;
        }
        let arranged = storage::path(storage::Kept::Triggers);
        let mut ran = self
            .ran
            .take()
            .unwrap_or_else(|| crate::schedule::Ran::read(storage::path(storage::Kept::Schedules)));
        for (trigger, works) in scheduled {
            let (Some(cron), By::New { allow, reject, .. }) = (trigger.cron(), &trigger.by) else {
                continue;
            };
            let id = trigger.file.display().to_string();
            let at_work = self
                .scheduled
                .get(&id)
                .and_then(|&key| self.slot(key))
                .is_some_and(|slot| working(slot.state));
            let off = crate::trigger::Arranged::read(arranged.as_deref(), &works)
                .off
                .contains(&trigger.file);
            let last = ran.last(&id);
            if at_work || off || !cron.due(last, now) {
                continue;
            }
            tracing::info!(schedule = id, "a schedule runs");
            ran.ran(&id, now.timestamp());
            if let Err(error) = std::fs::create_dir_all(&works) {
                tracing::error!("cannot make {}: {error}", works.display());
            }
            let project = if works == home {
                0
            } else {
                self.project(works)
            };
            let setup = (!allow.is_empty() || !reject.is_empty()).then(|| Setup {
                policy: Policy {
                    allow: allow.clone(),
                    reject: reject.clone(),
                    ..Policy::default()
                },
                ..Setup::default()
            });
            let key = self.open_in(project, None, setup);
            let store = Rc::clone(&self.store);
            if let Some(slot) = self.slot_mut(key) {
                slot.name_as(
                    &crate::schedule::chat_name(&trigger.name, now),
                    Named::Human,
                    &store,
                );
            }
            let line = crate::schedule::line(&trigger.text, &trigger.name, now, last);
            if let Some(chat) = self.chat(key) {
                chat.told(&line);
            }
            self.scheduled.insert(id, key);
        }
        self.ran = Some(ran);
    }

    /// Takes the window of the connection `id`, which says `hello`, to
    /// tell what it does on `window` from now on, unless another window
    /// is there, or the daemon's own program was `replaced` and no chat
    /// is at work, when the daemon is to stop for the new one. Answers
    /// which of those it is.
    fn attach(
        &mut self,
        id: u64,
        hello: Hello,
        window: UnboundedSender<FromDaemon>,
        replaced: bool,
    ) -> FromDaemon {
        if self.window.is_some() {
            return FromDaemon::Taken;
        }
        let at_work = self
            .chats
            .iter()
            .chain(&self.archived)
            .any(|slot| working(slot.state));
        if replaced && !at_work {
            return FromDaemon::Outdated;
        }
        if replaced {
            tracing::warn!("the daemon goes on as it was, since a chat is at work");
        }
        let Hello { home, command } = hello;
        // A window not given a home leaves Cox's own chats where
        // the settings say now, which may have changed while none was
        // there.
        self.home_given = home.is_some();
        self.projects[0].folder = home.unwrap_or_else(|| {
            crate::settings::home_for_chats(crate::settings::Saved::read().home)
        });
        self.spawns
            .agents()
            .command(command, crate::settings::agent().as_deref());
        self.window = Some(window);
        self.window_id = Some(id);
        self.window_watches.clear();
        self.listed = None;
        self.shown = None;
        FromDaemon::Welcome
    }

    /// Lets the window go: what it was asked to run it does not, and the
    /// agents that waited for its relay start without one.
    fn detach(&mut self) {
        tracing::info!("the window went; the chats go on");
        self.window = None;
        self.window_id = None;
        self.window_watches.clear();
        self.shown = None;
        for (_, ran) in self.running.drain() {
            ran.send(Err(NO_WINDOW.to_owned())).ok();
        }
        for (_, seen) in self.seeing.drain() {
            seen.send(Err(NO_STAGE.to_owned())).ok();
        }
        let preparing: Vec<Preparing> = self.preparing.drain().map(|(_, p)| p).collect();
        for Preparing {
            key,
            generation,
            cwd,
            start,
        } in preparing
        {
            self.spawn_agent(key, generation, cwd, start, None);
        }
    }

    /// Passes on to the window what the agent of the chat `key`, or the
    /// chat, asks of its stage: a command answered once it ends. With no
    /// window, coxd runs the command itself, on a terminal no one sees,
    /// unless it asks the human, which then cannot run.
    fn ask_stage(&mut self, key: u64, request: ToolRequest) {
        if let ToolRequest::ShowMarkdown { path, text } | ToolRequest::ShowFile { path, text, .. } =
            &request
            && let Some(cwd) = self.slot(key).map(|slot| self.cwd(slot))
            && let Some(chat) = self.chat(key)
        {
            let lines = match &request {
                ToolRequest::ShowFile { lines, .. } => lines.map(|lines| (lines.first, lines.last)),
                _ => None,
            };
            let markdown = matches!(request, ToolRequest::ShowMarkdown { .. });
            chat.show_file((path, &cwd), text, markdown, lines);
        }
        let ask = match request {
            ToolRequest::ShowMarkdown { path, text } => StageAsk::ShowMarkdown { path, text },
            ToolRequest::ShowFile { path, text, lines } => StageAsk::ShowFile { path, text, lines },
            ToolRequest::ShowChanges { folder } => StageAsk::ShowChanges { folder },
            ToolRequest::ShowMap { map, at } => StageAsk::ShowMap { map, at },
            ToolRequest::RunCommand {
                command,
                cwd,
                answer,
                asks_human,
            } => {
                if self.window.is_none() {
                    if asks_human {
                        answer.send(Err(NO_WINDOW.to_owned())).ok();
                    } else {
                        tracing::info!(key, "no window is there, so coxd runs `{command}` itself");
                        // It answers, or stops, as the agent waits or not,
                        // so it goes on its own.
                        tokio::task::spawn_local(async move {
                            run_unseen(command, &cwd, answer).await;
                        });
                    }
                    return;
                }
                let id = self.next_id();
                let (ran, heard) = oneshot::channel();
                let internal = self.internal.clone();
                let waits = spawn(async move {
                    let mut answer = answer;
                    tokio::select! {
                        ran = heard => {
                            let ran = ran.unwrap_or_else(|_| Err(NO_WINDOW.to_owned()));
                            answer.send(ran).ok();
                        }
                        () = answer.closed() => {
                            let _ = internal.send(Internal::Cancel { id });
                        }
                    }
                });
                self.running.insert(id, ran);
                self.stops.insert(id, waits);
                StageAsk::RunCommand { id, command, cwd }
            }
            ToolRequest::MacWindow { act, answer } => {
                let Some(id) = self.awaiting_sight(answer) else {
                    return;
                };
                StageAsk::MacWindow { id, act }
            }
            ToolRequest::Android { act, answer } => {
                let Some(id) = self.awaiting_sight(answer) else {
                    return;
                };
                StageAsk::Android { id, act }
            }
        };
        self.tell(FromDaemon::Stage { key, ask });
    }

    /// The number the window answers what an agent sees by, to `answer`:
    /// none, with `answer` answered, while no window is there.
    fn awaiting_sight(&mut self, answer: oneshot::Sender<Result<Seen, String>>) -> Option<u64> {
        if self.window.is_none() {
            answer.send(Err(NO_STAGE.to_owned())).ok();
            return None;
        }
        let id = self.next_id();
        self.seeing.insert(id, answer);
        Some(id)
    }

    /// Answers `request`, which the agent of the chat `key` asks: opens a
    /// sketch beside the chat, shows a goal it proposes, opens a chat under
    /// it, as the agent says, or waits for one.
    fn asked(&mut self, key: u64, request: ChatRequest) {
        match request {
            ChatRequest::StartSketch {
                sketch,
                what,
                show,
                answer,
            } => {
                let opened = self.start_sketch(key, sketch, &what);
                if show && let Ok(opened) = &opened {
                    self.reveals.push(opened.number);
                }
                answer.send(opened).ok();
            }
            ChatRequest::ShowMedia {
                shown,
                from,
                caption,
                answer,
            } => {
                if let Some(chat) = self.chat(key) {
                    chat.show_media(shown, from, caption);
                }
                answer.send(()).ok();
            }
            ChatRequest::ProposeGoal { goal, why, answer } => {
                let now = self
                    .chat(key)
                    .and_then(|chat| chat.propose_goal(&goal, why));
                answer.send(now).ok();
            }
            ChatRequest::ProposeName { name, why, answer } => {
                answer.send(self.propose_name(key, &name, why)).ok();
            }
            ChatRequest::OpenChat {
                text,
                setup,
                goal,
                sketch,
                answer,
            } => {
                let setup = Setup { sketch, ..setup };
                let opened = self.open_chat(key, &text, setup, goal);
                answer.send(opened).ok();
            }
            ChatRequest::WaitChat { chat, answer } => self.wait_chat(chat, answer),
            ChatRequest::SaveToWiki { chat } => self.save_to_wiki_of(key, chat),
            ChatRequest::WikiWritten { page } => {
                let in_sketch = self.sketch_of(key).is_some();
                if let Some(chat) = self.chat(key) {
                    chat.fire(
                        &crate::trigger::On::WikiWritten {
                            pages: Some(vec![page.clone()]),
                        },
                        format!("The chat's agent wrote the page {page} of the wiki."),
                        in_sketch,
                    );
                }
            }
            ChatRequest::Whereabouts { answer } => {
                let whereabouts = self
                    .slot(key)
                    .map(|slot| Whereabouts {
                        project: self.projects[slot.project].name.clone(),
                        // The first is Cox's own, in the home.
                        folder: (slot.project != 0)
                            .then(|| self.projects[slot.project].folder.clone()),
                        chat: slot.name.clone(),
                        kept: slot.kept.map(|kept| kept.to_string()),
                    })
                    .unwrap_or_default();
                answer.send(whereabouts).ok();
            }
        }
    }

    /// Asks the agent of the chat kept as `chat`, or of the chat `key`, to
    /// keep what it learned in the wiki, or notes in the chat `key` why it
    /// cannot.
    fn save_to_wiki_of(&mut self, key: u64, chat: Option<ChatId>) {
        let which = match chat {
            Some(kept) => self
                .chats
                .iter()
                .find(|slot| slot.kept == Some(kept))
                .map(|slot| slot.key),
            None => self
                .chats
                .iter()
                .find(|slot| slot.key == key)
                .map(|slot| slot.key),
        };
        let why = match which {
            None => Some("the chat that learned it is not among the tabs"),
            Some(which) if !self.saves_to_wiki(which) => {
                Some("its agent is at work, or has no wiki_write: ask once it waits")
            }
            Some(which) => {
                self.save_to_wiki(which);
                None
            }
        };
        if let Some(why) = why
            && let Some(chat) = self.chat(key)
        {
            chat.note(format!("Nothing is kept in the wiki: {why}."));
        }
    }

    /// Asks the agent of the chat `key`, in the human's place, to keep what
    /// the chat learned in the human's wiki, as their workflow says to
    /// (see [`workflow::WIKI_SAVE`]), when it can; answers whether it did.
    fn save_to_wiki(&mut self, key: u64) -> bool {
        if !self.saves_to_wiki(key) {
            return false;
        }
        let line = workflow::WIKI_SAVE.say(&[]);
        self.go_on(key, &line);
        true
    }

    /// Asks the agent of the chat `key`, in the human's place, to merge
    /// what it made, as the human's workflow says, unless it is at work,
    /// or is a sketch, which ends its own ways.
    fn merge(&mut self, key: u64) {
        let merges = self
            .chats
            .iter()
            .find(|slot| slot.key == key)
            .is_some_and(|slot| !working(slot.state) && merges(slot.setup.as_ref()));
        if merges {
            let line = workflow::CLOSE_MERGE.say(&[]);
            self.start_again(key);
            if let Some(chat) = self.chat(key) {
                chat.merge(&line);
            }
        }
    }

    /// Whether the human can ask the agent of the chat `key` to keep what
    /// it learned in the wiki: it is not at work, and has the tool.
    fn saves_to_wiki(&self, key: u64) -> bool {
        self.chats
            .iter()
            .find(|slot| slot.key == key)
            .is_some_and(|slot| !working(slot.state) && keeps_in_wiki(slot.setup.as_ref()))
    }

    /// Answers `answer` with how the chat that goes by `key` stands once it
    /// is not at work, or at once when it is not.
    fn wait_chat(&mut self, key: u64, answer: oneshot::Sender<Result<Waited, String>>) {
        if self.slot(key).is_none() {
            answer.send(Err(format!("no chat {key}"))).ok();
            return;
        }
        let Some(chat) = self.chat(key) else {
            answer.send(Ok(Waited::default())).ok();
            return;
        };
        if at_work(chat.conversation().state()) {
            self.waiting.retain(|(_, answer)| !answer.is_closed());
            self.waiting.push((key, answer));
        } else {
            answer.send(Ok(chat.stands())).ok();
        }
    }

    /// Opens a chat under the chat `key`, in its project, set up as
    /// `setup` says, which works until `goal` is met if given, and gets
    /// `text` as its first line, and answers how it goes by. A sketch, as
    /// the setup's `sketch` makes it, works in a folder of its own; any
    /// other chat works where the chat that opened it does.
    fn open_chat(
        &mut self,
        key: u64,
        text: &str,
        mut setup: Setup,
        goal: Option<Goal>,
    ) -> Result<OpenedChat, String> {
        let slot = self.slot(key).ok_or("the chat is gone")?;
        let folder = slot.setup.as_ref().and_then(|setup| setup.folder.clone());
        let project = slot.project;
        setup.folder = match setup.sketch {
            Some(sketch) => {
                let from = folder.unwrap_or_else(|| self.projects[project].folder.clone());
                let made = make_sketch(sketch, &from)?;
                setup.policy = sketch::allow(setup.policy, &made, &workflow::sketch().allow);
                Some(made)
            }
            None => folder,
        };
        self.open_under(key, text, setup, goal)
    }

    /// Opens a chat under the chat `key`, set up as `setup` says, on
    /// `text`, toward `goal`, and answers how it goes by.
    fn open_under(
        &mut self,
        key: u64,
        text: &str,
        setup: Setup,
        goal: Option<Goal>,
    ) -> Result<OpenedChat, String> {
        let slot = self.slot(key).ok_or("the chat is gone")?;
        let (project, kept, opened_by) = (slot.project, slot.kept, slot.name.clone());
        let sketch = setup.sketch;
        let opened = self.open_in(project, Some((key, kept)), Some(setup));
        let slot = self.slot(opened).ok_or("the chat did not open")?;
        let told = OpenedChat {
            number: opened,
            name: slot.name.clone(),
            folder: sketch
                .and(slot.setup.as_ref())
                .and_then(|setup| setup.folder.clone()),
        };
        if let Some(chat) = self.chat(opened) {
            chat.tell(&opened_by, text, goal);
        }
        Ok(told)
    }

    /// Opens a sketch beside the chat `key`, under it in the list, that
    /// starts from `started`, to try out `what`, and answers how it goes
    /// by, or why it cannot open. The sketch's agent is set up as the
    /// chat's is, and told what the chat talked through so far, and what
    /// it tries; the chat goes on in the project, apart from it, and hears
    /// how it ends (see [`Self::finish_sketch`]).
    fn start_sketch(
        &mut self,
        key: u64,
        started: Sketch,
        what: &str,
    ) -> Result<OpenedChat, String> {
        let slot = self
            .chats
            .iter()
            .find(|slot| slot.key == key)
            .ok_or("the chat is not in the list")?;
        let setup = slot.setup.clone().unwrap_or_default();
        if let (Some(_), Some(folder)) = (setup.sketch, &setup.folder) {
            return Err(format!(
                "the chat is a sketch already, working in {}",
                folder.display()
            ));
        }
        let from = setup
            .folder
            .clone()
            .unwrap_or_else(|| self.projects[slot.project].folder.clone());
        let talked = self
            .chat(key)
            .map(|chat| agent::talked_through(chat.conversation().entries()))
            .unwrap_or_default();
        let folder = make_sketch(started, &from)?;
        let context = [setup.context.clone(), talked]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");
        let setup = Setup {
            context: (!context.is_empty()).then_some(context),
            folder: Some(folder.clone()),
            sketch: Some(started),
            policy: sketch::allow(setup.policy, &folder, &workflow::sketch().allow),
            ..setup
        };
        let line = workflow::SKETCH_START.say(&[
            ("folder", &folder.display().to_string()),
            ("from", started.what()),
            ("what", what),
        ]);
        let opened = self.open_under(key, &line, setup, None)?;
        // Named after what it tries, not after the line Cox sends it.
        let name: String = what.chars().take(NAME_LENGTH).collect();
        let store = Rc::clone(&self.store);
        if let Some(slot) = self.slot_mut(opened.number) {
            slot.name_as(&name, Named::FirstLine, &store);
        }
        if let Some(chat) = self.chat(key) {
            chat.note(format!(
                "Sketch: chat {} opens beside this chat, in {}, to try: {what}",
                opened.number,
                folder.display()
            ));
        }
        Ok(OpenedChat { name, ..opened })
    }

    /// Switches the chat `key` to the agent kept as `id`, unless a turn
    /// runs: closes it, its agent stopped, keeps the agent as the chat's,
    /// and starts the agent at once, which goes on from the conversation
    /// so far, told it as a session that cannot be resumed is.
    fn switch_agent(&mut self, key: u64, id: &str) {
        let Some(slot) = self.slot(key) else {
            return;
        };
        let Some(kept) = slot.kept.filter(|_| !working(slot.state)) else {
            return;
        };
        let cwd = self.cwd(slot);
        let setup = slot.setup.clone();
        if self.chat(key).is_none_or(|chat| chat.agent_id() == id) {
            return;
        }
        if let Err(error) = self.store.keep_agent(kept, id) {
            tracing::error!("the chat goes on with the agent it had: {error:#}");
            return;
        }
        let agents = self.spawns.agents();
        let starts = self.spawns.starts(id);
        let to = agents.of(Some(id)).name;
        let Some(chat) = self.chat(key) else {
            return;
        };
        let from = chat
            .agents()
            .map(|agents| agents.of(Some(chat.agent_id())).name)
            .unwrap_or_default();
        tracing::info!(agent = %to, "the chat switches to another agent");
        let again = Again {
            cwd,
            session: None,
            setup,
        };
        chat.switch_agent(&from, &to, (agents, id.to_owned()), starts, again);
        if let Some(slot) = self.slot_mut(key) {
            slot.state = State::Closed;
        }
        // The new agent starts at once, told the conversation so far.
        let Some(chat) = self.chat(key) else {
            return;
        };
        let earlier = chat.conversation().entries().to_vec();
        if let Some(again) = chat.take_again() {
            let Again {
                cwd,
                session,
                setup,
            } = again;
            self.start_agent(key, Start::again(&cwd, session, &earlier, setup));
        }
    }

    /// Takes the chat `key` out of the list, to "Archived" at its foot, or
    /// brings it back. Its agent, if it runs, goes on.
    fn archive(&mut self, key: u64, archived: bool) {
        let slot = if archived {
            let Some(index) = self.chats.iter().position(|slot| slot.key == key) else {
                return;
            };
            self.chats.remove(index)
        } else {
            let Some(index) = self.archived.iter().position(|slot| slot.key == key) else {
                return;
            };
            self.archived.remove(index)
        };
        if let Some(kept) = slot.kept
            && let Err(error) = self.store.archive_chat(kept, archived)
        {
            tracing::error!("whether the chat is archived is not kept: {error:#}");
        }
        if archived {
            self.archived.push(slot);
        } else {
            // Brought back, it is shown.
            self.add(slot);
            self.reveals.push(key);
        }
    }

    /// The sketch the chat `key` is, what it started from and the folder
    /// it works in, while the folder is there.
    fn sketch_of(&self, key: u64) -> Option<(Sketch, PathBuf)> {
        let setup = self.slot(key)?.setup.as_ref()?;
        let folder = setup.folder.clone().filter(|folder| folder.exists())?;
        Some((setup.sketch?, folder))
    }

    /// What the human may do of sketches in the chat `key`, as the window's
    /// menu offers it: start one beside it, unless it is a sketch, or end
    /// it, a sketch that can end.
    fn sketching(&self, key: u64) -> Option<Sketching> {
        let slot = self.slot(key)?;
        match slot.setup.as_ref().and_then(|setup| setup.sketch) {
            None => Some(Sketching::Starts {
                project: self.projects[slot.project].folder.clone(),
            }),
            Some(_) => self
                .sketch_of(key)
                .filter(|_| self.ends_sketch(key))
                .map(|(from, _)| Sketching::Ends { from }),
        }
    }

    /// Whether the sketch `key` can end: its folder is there, and was not
    /// opened as a project, which the human goes on with, and it is not on
    /// its way to an end already.
    fn ends_sketch(&self, key: u64) -> bool {
        self.slot(key).is_some_and(|slot| slot.ending.is_none())
            && self.sketch_of(key).is_some_and(|(_, folder)| {
                !self.projects.iter().any(|project| project.folder == folder)
            })
    }

    /// Ends the sketch `key` as `ending` says: asks its agent to sum it up
    /// in the conversation first, if the ending says to, once its turn
    /// ends if one goes on, and ends it once that is done (see
    /// [`Self::finish_sketch`]).
    fn end_sketch_as(&mut self, key: u64, ending: workflow::Ending) {
        let ends = self.ends_sketch(key)
            && self
                .sketch_of(key)
                .is_some_and(|(from, _)| ending.ends(from));
        if !ends {
            return;
        }
        self.reveals.push(key);
        self.read(key);
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        let waits = ending.sum_up.is_some() && working(slot.state);
        let name = ending.name.clone();
        slot.ending = Some((Ends::Waiting, ending));
        if waits {
            if let Some(chat) = self.chat(key) {
                chat.note(format!(
                    "The sketch ends as \"{name}\" once this turn ends, and its agent has \
                     summed it up."
                ));
            }
        } else {
            self.sum_up(key);
        }
    }

    /// Asks the agent of the sketch `key`, which the human ends, to sum it
    /// up in the conversation, as its ending says, starting it again if it
    /// is closed; the sketch ends once it is done, or at once if the ending
    /// asks for none, or the agent cannot be asked.
    fn sum_up(&mut self, key: u64) {
        let Some((_, ending)) = self.slot(key).and_then(|slot| slot.ending.clone()) else {
            return;
        };
        let Some(text) = ending.sum_up.clone() else {
            self.finish_sketch(key);
            return;
        };
        let Some((_, folder)) = self.sketch_of(key) else {
            if let Some(slot) = self.slot_mut(key) {
                slot.ending = None;
            }
            return;
        };
        let left_out = sketch::left_out(&folder).unwrap_or_else(|error| {
            tracing::warn!("cannot tell what the sketch changed: {error}");
            Vec::new()
        });
        let Some(line) = workflow::say(&text, &[("left_out", &sketch::left_out_lines(&left_out))])
        else {
            if let Some(chat) = self.chat(key) {
                chat.note(format!(
                    "The sketch is not summed up: the workflow has no {text}."
                ));
            }
            self.finish_sketch(key);
            return;
        };
        if let Some(slot) = self.slot_mut(key) {
            slot.ending = Some((Ends::Summing, ending));
        }
        self.go_on(key, &line);
        let asked = self
            .chat(key)
            .is_some_and(|chat| working(chat.conversation().state()));
        if !asked {
            self.finish_sketch(key);
        }
    }

    /// Ends the sketch `key`, whose agent has summed it up if its ending
    /// asked, as the ending says. Its folder moves into a project of its
    /// own, as a directory named as the project is, to go on with what the
    /// sketch made, for an ending that opens it (see
    /// [`Self::open_as_project`]); or else it goes, what the sketch changed kept apart first if
    /// the ending says to (see [`sketch::discard`]), and the sketch ends,
    /// its chat going on in the project toward the ending's goal, if it
    /// has one, its agent told what the ending says then.
    fn finish_sketch(&mut self, key: u64) {
        let Some((_, ending)) = self.slot_mut(key).and_then(|slot| slot.ending.take()) else {
            return;
        };
        let Some((started, folder)) = self.sketch_of(key).filter(|_| self.ends_sketch(key)) else {
            return;
        };
        if ending.open {
            self.open_as_project(key, &folder);
            return;
        }
        let Some(slot) = self.slot(key) else {
            return;
        };
        let title = slot.name.clone();
        let sketch_kept = slot.kept;
        let sketch_keeps = keeps_in_wiki(slot.setup.as_ref());
        let parent = slot.parent;
        // What the sketch's agent said last, as its summing up.
        let summary = self
            .chat(key)
            .and_then(|chat| chat.last_reply())
            .unwrap_or_default();
        let discarded = storage::path(storage::Kept::Sketches)
            .map(|sketches| sketches.join(sketch::DISCARDED))
            .ok_or_else(|| "Cox cannot tell where to keep discarded sketches".to_owned())
            .and_then(|bundles| sketch::discard(started, &folder, &title, &bundles, ending.keep));
        let kept = discarded
            .as_ref()
            .ok()
            .and_then(|discarded| discarded.kept.as_ref())
            .map(ToString::to_string)
            .unwrap_or_default();
        let note = discarded.as_ref().map(sketch::Discarded::note).ok();
        let ended = self.end_sketch(key, &folder, discarded.map(|discarded| discarded.note()));
        if !ended {
            return;
        }
        // The chat the sketch was opened beside goes on with it, in the
        // project; a sketch opened beside none goes on itself.
        let goes_on = parent
            .filter(|parent| self.chats.iter().any(|slot| slot.key == *parent))
            .unwrap_or(key);
        if goes_on != key {
            // The human goes on there.
            self.reveals.push(goes_on);
            if let (Some(chat), Some(note)) = (self.chat(goes_on), note.as_ref()) {
                chat.note(format!("The sketch \"{title}\" (chat {key}) ended: {note}"));
            }
            // Its triggers run where the human goes on, in the project.
            if let Some(chat) = self.chat(goes_on) {
                chat.fire(
                    &crate::trigger::On::SketchEnd,
                    format!(
                        "The sketch \"{title}\" ended{}",
                        note.as_ref()
                            .map_or(".".to_owned(), |note| format!(": {note}"))
                    ),
                    false,
                );
            }
        }
        // What the sketch found, and why it ended, is the sketch's agent's
        // to keep, as the human chooses where they go on.
        let keeps = (goes_on != key).then_some(sketch_kept);
        if sketch_keeps
            && keeps.is_none_or(|kept| kept.is_some())
            && let Some(chat) = self.chat(goes_on)
        {
            chat.ask_to_keep(format!("The sketch \"{title}\" ended."), keeps.flatten());
        }
        if let Some(goal) = &ending.goal
            && let Some(chat) = self.chat(goes_on)
        {
            chat.set_goal(Goal::parse(goal));
        }
        if let Some(text) = &ending.then {
            match workflow::say(text, &[("kept", &kept), ("summary", &summary)]) {
                Some(line) => self.go_on(goes_on, &line),
                None => {
                    if let Some(chat) = self.chat(goes_on) {
                        chat.note(format!(
                            "The agent is told nothing: the workflow has no {text}."
                        ));
                    }
                }
            }
        }
    }

    /// Ends the sketch `key`, which worked in `folder`, as `ended` says, or
    /// says why it could not, and answers whether it ended. Ended, the
    /// sketch's chat goes on in the project, as do the chats it opened,
    /// which worked in its folder too: each is closed, its agent stopped,
    /// and set up to work in the project, asking the human as any chat
    /// does, so that a line sent starts its agent again there, with the
    /// conversation so far.
    fn end_sketch(&mut self, key: u64, folder: &Path, ended: Result<String, String>) -> bool {
        self.reveals.push(key);
        let note = match ended {
            Ok(note) => note,
            Err(error) => {
                if let Some(chat) = self.chat(key) {
                    chat.note(format!("The sketch could not end: {error}"));
                }
                return false;
            }
        };
        let goes_on = "The chat goes on in the project: a line sent starts its agent again \
                       there, with the conversation so far.";
        let mut ended = Vec::new();
        let store = Rc::clone(&self.store);
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            let worked_there = slot.setup.as_ref().is_some_and(|setup| {
                setup
                    .folder
                    .as_ref()
                    .is_some_and(|works| works.starts_with(folder))
            });
            if !worked_there {
                continue;
            }
            let told = if slot.key == key {
                format!("{note} {goes_on}")
            } else {
                format!(
                    "The sketch this chat worked in has ended, and its folder is gone. {goes_on}"
                )
            };
            if let Some(chat) = &mut slot.chat {
                chat.close(told.clone());
            } else {
                // Read once it is shown, it says so.
                if let Some(kept) = slot.kept {
                    let entries = store.entries(kept).map(|entries| entries.len());
                    if let Ok(number) = entries
                        && let Err(error) =
                            store.keep_entry(kept, number, &Entry::Note(told.clone()))
                    {
                        tracing::error!("the chat is not kept as it goes: {error:#}");
                    }
                }
            }
            slot.state = State::Closed;
            slot.setup = slot.setup.take().map(|setup| Setup {
                folder: None,
                sketch: None,
                policy: Policy::default(),
                ..setup
            });
            if let Some(kept) = slot.kept
                && let Err(error) =
                    store.keep_setup(kept, slot.setup.as_ref().map(Setup::to_json).as_deref())
            {
                tracing::error!("the chat's setup is not kept: {error:#}");
            }
            ended.push((slot.key, told));
        }
        // What waits for them hears how they ended.
        for (ended, told) in &ended {
            for (_, answer) in self.waiting.extract_if(.., |(waited, _)| waited == ended) {
                let stands = Waited {
                    asking: None,
                    said: Some(told.clone()),
                };
                answer.send(Ok(stands)).ok();
            }
        }
        // Each is read again, closed, to go on with in the project.
        for (ended, _) in ended {
            self.read_again(ended);
        }
        true
    }

    /// Moves `folder`, of the sketch `key`, into a project with nothing set
    /// yet (see [`Self::untitled`]), as the directory of the project's
    /// name, and opens a chat in the project; the sketch ends, its chats
    /// going on where they were opened.
    fn open_as_project(&mut self, key: u64, folder: &Path) {
        let project = self.untitled();
        let name = project
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let directory = project.join(crate::settings::DIRECTORIES).join(&name);
        let moved = directory
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::rename(folder, &directory))
            .map_err(|error| {
                let _ = std::fs::remove_dir_all(&project);
                format!(
                    "Cox cannot move {} into {}: {error}",
                    folder.display(),
                    directory.display()
                )
            });
        if moved.is_ok() {
            // A repository of the project now, what changed in it is what
            // changed since its last commit.
            if let Err(error) = cox_editor::changes::forget(&directory) {
                tracing::warn!(
                    "{} still counts changes from nothing: {error}",
                    directory.display()
                );
            }
        }
        let ended = moved.map(|()| {
            format!(
                "The sketch was opened as the project {name}, whose directory {name} holds what \
                 it made."
            )
        });
        if self.end_sketch(key, folder, ended) {
            self.open(Some(project));
        }
    }

    /// Keeps and tells what changed: each chat's conversation, how each
    /// chat stands, and the list.
    fn settle(&mut self) {
        // What one chat's change does can change another, as a sketch that
        // ends; each pass looks again, until nothing does.
        for _ in 0..4 {
            if !self.review() {
                break;
            }
        }
        let keys: Vec<u64> = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter(|slot| slot.chat.is_some())
            .map(|slot| slot.key)
            .collect();
        for key in keys {
            let patch = self
                .slot_mut(key)
                .and_then(|slot| slot.chat.as_mut())
                .and_then(Chat::flush);
            if let Some(patch) = patch {
                self.tell_patch(key, patch);
            }
        }
        self.tell_listing();
        for key in std::mem::take(&mut self.reveals) {
            self.tell(FromDaemon::Reveal { key });
        }
    }

    /// How the chat of `slot` stands, as the window lists it: one whose
    /// project's folder is not there does not start again, until the
    /// folder is back.
    fn row(&self, slot: &Slot, agents: &Agents) -> ChatRow {
        let mut row = slot.row(agents, &self.store);
        row.again &= slot.project == 0 || self.cwd(slot).is_dir();
        row
    }

    /// Tells the window the projects and chats, if they changed since it
    /// was last told.
    fn tell_listing(&mut self) {
        let agents = self.spawns.agents();
        let mut listing = Listing {
            projects: self
                .projects
                .iter()
                .map(|project| ProjectRow {
                    id: project.id,
                    name: project.name.clone(),
                    folder: project.folder.clone(),
                    missing: project.id.is_some() && !project.deleted && !project.folder.is_dir(),
                    archived: project.archived,
                    deleted: project.deleted,
                })
                .collect(),
            chats: self
                .chats
                .iter()
                .map(|slot| self.row(slot, &agents))
                .collect(),
            archived: self
                .archived
                .iter()
                .map(|slot| self.row(slot, &agents))
                .collect(),
            agents: std::collections::BTreeMap::new(),
        };
        let ids: HashSet<&str> = listing
            .chats
            .iter()
            .chain(&listing.archived)
            .map(|row| row.agent.as_str())
            .collect();
        let names = agents
            .all()
            .into_iter()
            .filter(|known| ids.contains(known.id.as_str()))
            .map(|known| (known.id, known.name))
            .collect();
        listing.agents = names;
        if self.listed.as_ref() != Some(&listing) {
            let told = FromDaemon::Listing(listing.clone());
            self.tell_phones(&told);
            self.tell(told);
            self.listed = Some(listing);
        }
    }

    /// Follows how each chat stands now: a turn that ends while it is not
    /// shown leaves it unread, and what it is about names it, unless a name
    /// that lasts more has. A chat that comes to wait for the human, ends
    /// a turn, or whose agent changed the plan the human left it to, tells
    /// the window, for it to tell the human unless they look at it. What waits for a chat that is not at work hears how it
    /// stands, and a sketch the human ends goes on to its end. Answers
    /// whether a sketch went on, which may change other chats.
    fn review(&mut self) -> bool {
        let mut news = Vec::new();
        let mut stands = Vec::new();
        let mut ends = Vec::new();
        let store = Rc::clone(&self.store);
        let shown = self.shown;
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            let Some(chat) = &slot.chat else {
                continue;
            };
            let conversation = chat.conversation();
            let state = conversation.state();
            let ended = working(slot.state) && !working(state);
            if ended && shown != Some(slot.key) {
                slot.unread = true;
            }
            // A permission request is news once what its triggers gather
            // came, or the time is up.
            let asks = state == State::Asking && !conversation.gathering();
            let told = if asks && !slot.asks {
                let asked = conversation.asking_for().unwrap_or("permission");
                let mut told = format!("Asks you: {asked}");
                for headline in conversation.headlines() {
                    told.push('\n');
                    told.push_str(headline);
                }
                Some(told)
            } else if ended {
                Some(
                    conversation
                        .last_said()
                        .unwrap_or("The turn ended.")
                        .to_owned(),
                )
            } else {
                None
            };
            // The agent changing the plan the human left it to is news as
            // it goes on.
            let (changes, change) = conversation.plan_changes();
            let told = told.or_else(|| {
                (changes > slot.plan_changes)
                    .then(|| format!("The plan changed:\n{}", change.unwrap_or_default()))
            });
            slot.plan_changes = changes;
            slot.asks = asks;
            slot.state = state;
            slot.goal = conversation.goal().map(|goal| goal.what().clone());
            let about = match (conversation.title(), conversation.first_line()) {
                (Some(title), _) => Some((title.to_owned(), Named::Agent)),
                (None, Some(line)) => Some((line.to_owned(), Named::FirstLine)),
                (None, None) => None,
            };
            if !at_work(state) {
                stands.push((slot.key, chat.stands()));
            }
            if let Some((name, named)) = about {
                let name: String = name.chars().take(NAME_LENGTH).collect();
                // The agent may title its session anew.
                let lasts = named > slot.named || (named == Named::Agent && slot.named == named);
                if lasts && name != slot.name {
                    slot.name_as(&name, named, &store);
                }
            }
            if let Some(body) = told {
                news.push(FromDaemon::News {
                    key: slot.key,
                    name: slot.name.clone(),
                    body,
                });
            }
            if let Some((sketch_ends, _)) = &slot.ending
                && !working(state)
            {
                ends.push((slot.key, *sketch_ends));
            }
        }
        for message in news {
            self.push(&message);
            self.tell_phones(&message);
            self.tell(message);
        }
        for (key, stands) in stands {
            for (_, answer) in self.waiting.extract_if(.., |(waited, _)| *waited == key) {
                answer.send(Ok(stands.clone())).ok();
            }
        }
        // A sketch the human ends is summed up once its turn ends, and
        // ends once that is done.
        let went_on = !ends.is_empty();
        for (key, sketch_ends) in ends {
            match sketch_ends {
                Ends::Waiting => self.sum_up(key),
                Ends::Summing => self.finish_sketch(key),
            }
        }
        went_on
    }
}

/// The folder an agent that starts as `start` says works in, when its
/// setup names one.
fn start_cwd(start: &Start) -> Option<PathBuf> {
    start.setup().and_then(|setup| setup.folder.clone())
}

/// The id of the agent the chat kept as `kept` talks to: the one the
/// human chose, while Cox knows it, or else the one a chat starts with.
fn agent_of(agents: &Agents, kept: Option<ChatId>, store: &Store) -> String {
    let chosen = kept.and_then(|kept| {
        store
            .agent(kept)
            .inspect_err(|error| tracing::error!("the chat's agent cannot be read: {error:#}"))
            .ok()
            .flatten()
    });
    agents.of(chosen.as_deref()).id
}

/// Makes the folder of a sketch that starts from `sketch`, of the folder
/// `from`, and answers it.
fn make_sketch(sketch: Sketch, from: &Path) -> Result<PathBuf, String> {
    let sketches =
        storage::path(storage::Kept::Sketches).ok_or("Cox cannot tell where to keep sketches")?;
    sketch::make(sketch, from, &sketches)
}

/// Whether a chat set up as `setup` merges what it made when the human
/// asks: one that works in the project, and not in a sketch's folder, as
/// a sketch, which ends its own ways, or a chat a sketch opened.
#[must_use]
pub fn merges(setup: Option<&Setup>) -> bool {
    setup.is_none_or(|setup| setup.folder.is_none() && setup.sketch.is_none())
}

/// Whether an agent that stands as `state` is at work.
#[must_use]
pub fn working(state: State) -> bool {
    matches!(
        state,
        State::Busy | State::Asking | State::Stopping | State::Checking
    )
}

/// Whether a chat in `state` is at work, so that an agent waiting for it
/// waits on: not when it waits for the human.
#[must_use]
pub fn at_work(state: State) -> bool {
    matches!(state, State::Busy | State::Stopping | State::Checking)
}

/// A directory of a project renamed with it, from `was` to `now`.
struct Renamed {
    was: PathBuf,
    now: PathBuf,
}
