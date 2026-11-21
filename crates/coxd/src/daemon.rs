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
//! - [`main`] starts coxd, and [`process`] runs it as a process of its
//!   own, which a window reaches on a socket, and [`client`] is how the
//!   window does. [`launcher`] starts each chat's agent for it.
//! - [`tidy`] takes away the worktrees Cox made for chats once they
//!   are done, and makes one again for a chat that goes on in it.
//! - [`phone`] serves the human's phone a page of the chats, which watches
//!   them beside the window, and [`push`] tells the phone of the chats that
//!   need the human while they are away from the Mac.

pub mod chat;
pub mod client;
mod launcher;
pub mod main;
pub mod phone;
pub mod process;
mod project;
mod pulls;
pub mod push;
mod resolve;
mod sketches;
mod tidy;
pub mod wire;

pub use launcher::{hands_off, session_meta};

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use cox_core::entry::{Attachment, OnStep};
use cox_core::goal::Goal;
use cox_core::store::{ChatId, Named, ProjectId, Record, Resolved, ResolvedBy, Store};
use cox_core::tabs::{KeptTab, KeptTabs};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::agent::instructions::Instructions;
use crate::agent::mcp::{
    ChatRequest, OpenedChat, Ran, Seen, Stopped, Subagent, ToolRequest, Waited, Whereabouts, Within,
};
use crate::agent::setup::{Policy, Setup, StageUse, Withheld, keeps_in_wiki};
use crate::agent::{Agent, AgentEvent, Start};
use crate::agents::Agents;
use crate::chat::conversation::{Content, Conversation, Entry, Gathered, Patch, State};
use crate::chat::judge::{Heard, Starts};
use crate::chat::sketch::{self, Sketch};
use crate::chat::spent::Spent;
use crate::settings::Subagents;
use crate::storage;
use crate::terminal::run_unseen;
use crate::trigger::By;
use crate::workflow;
use crate::worktrees;
use chat::{Again, Chat, Task, spawn};
use wire::{AtWork, ChatRow, FromDaemon, Hello, Listing, ProjectRow, StageAsk, ToDaemon};

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

    /// Where the settings say a chat's agent runs its subagents now, which
    /// a chat whose session starts afresh keeps.
    fn subagents(&self) -> Subagents;

    /// Starts the agent kept as `agent`, of a chat working in `cwd`, an
    /// absolute path, as `start` says, its browser tools reaching the
    /// chat's stage through the relay on `relay`, when given, and gives it
    /// Cox's own tools, unless the chat's setup says otherwise, but those
    /// of a project when the chat is in none, as `in_project` says, its
    /// subagents run as `subagents` says. A chat the human opened, not one
    /// an agent or a sketch did, as `top_level` says, hands its work off
    /// (see [`launcher::hands_off`]).
    #[expect(clippy::too_many_arguments, reason = "what a chat's agent starts with")]
    fn chat(
        &self,
        cwd: PathBuf,
        start: Start,
        agent: &str,
        relay: Option<u16>,
        in_project: bool,
        subagents: Subagents,
        top_level: bool,
    ) -> Spawned;

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

/// Where a chat works, and the pages of the stage there, each its address
/// and title.
pub type Staged = (PathBuf, Vec<(String, String)>);

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
    /// [`chat::STOP_WAIT`] passed since the human asked the agent of the
    /// chat to stop.
    StopDue { key: u64, generation: u64 },
    /// [`chat::BACKGROUND_SETTLES`] passed since the last of what the
    /// chat's agent ran in the background ended.
    Settled { key: u64 },
    /// The command `id`, which the window runs, is not waited for any more.
    Cancel { id: u64 },
    /// A window came, as the connection `id`, saying `hello`, to hear
    /// what the daemon says on `window`. `answer` hears how the daemon
    /// took it.
    Attach {
        id: u64,
        hello: Hello,
        window: UnboundedSender<FromDaemon>,
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
    /// The key of the chat kept as `number`, told on `answer`, or nothing
    /// of a chat there is none of.
    Key {
        number: i64,
        answer: oneshot::Sender<Option<u64>>,
    },
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
    /// What the tool call, or the permission request, at `at` in the
    /// conversation of the chat `key` shows, for the phone, which is told
    /// the conversation without it, to show as the human opens it, told
    /// on `answer`, or nothing where there is none.
    Content {
        key: u64,
        at: usize,
        answer: oneshot::Sender<Option<Vec<Content>>>,
    },
    /// Where the chat `key` works, and the pages of the stage there, each
    /// its address and title, for the phone to offer those of the Mac's
    /// own address to try, told on `answer`, or nothing of a chat there is
    /// none of.
    Pages {
        key: u64,
        answer: oneshot::Sender<Option<Staged>>,
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
    /// It is time to look which pull requests of the chats are due to be
    /// read (see [`pulls`]).
    Pulls,
    /// `gh` read how `pull` stands, as `read` says.
    Pulled {
        pull: crate::github::Pull,
        read: Result<crate::github::Status, crate::github::Problem>,
    },
    /// Cox took away the worktrees `removed`, of chats that were done
    /// (see [`tidy`]).
    Tidied {
        removed: Vec<crate::worktrees::Removed>,
    },
}

/// How many of the project's other chats a chat's agent is told of, at
/// most, the last the list shows.
const CHATS_TOLD: usize = 20;

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
    /// What it is, in a sentence or two, as each chat's agent is told, if
    /// the human set it.
    about: Option<String>,
    /// Whether the human deleted it: it stays in the projects, so that
    /// the chats of those after it keep the places they are listed by,
    /// with none of its own.
    deleted: bool,
    /// The chat of it the window showed last, by its key, as kept past a
    /// restart.
    last: Option<u64>,
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

/// Whose a chat is: who reads what it comes to once its turn ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Whose {
    /// The human's: a turn that ends waits for them, until they look.
    Human,
    /// The agent's of the chat `parent`, which opened it and hears how it
    /// stands, as `report` says. The human still hears of what it asks
    /// them.
    Agent { parent: u64, report: Report },
}

impl Whose {
    /// Whose a chat opened from the chat `parent`, if one opened it, set
    /// up as `setup`, is: the agent's that opened it, unless it is a
    /// sketch, which the human ends.
    fn opened(parent: Option<u64>, setup: Option<&Setup>) -> Self {
        match parent {
            Some(parent) if setup.is_none_or(|setup| setup.sketch.is_none()) => Self::Agent {
                parent,
                report: Report::Working,
            },
            _ => Self::Human,
        }
    }
}

/// Whether the agent that opened a chat heard what the chat came to last:
/// its report, once it ended its turn, or what it asks the human.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Report {
    /// It is at work, or has not come to anything yet.
    Working,
    /// It came to something the agent has not heard: Cox tells the agent
    /// in the human's place once the agent's own turn is not running
    /// (see [`Daemon::tell_reports`]).
    Pending,
    /// The agent heard it, from Cox or from `agent_report`.
    Heard,
}

impl Report {
    /// The report of a chat that stood as `was` and stands as `now`: one
    /// that goes back to work has none, and one that stops working has
    /// one pending.
    /// What the agent heard of what the chat runs in the background goes
    /// as `behind` does: back at work, it heard nothing of what it runs
    /// now, and once what it heard of ended, and the background
    /// `settled`, it hears so.
    fn as_it_goes(self, was: State, now: State, behind: Behind, settled: bool) -> (Self, Behind) {
        let report = match (at_work(was), at_work(now)) {
            (_, true) => return (Self::Working, Behind::Untold),
            (true, false) => Self::Pending,
            (false, false) => self,
        };
        if behind == Behind::Told && settled {
            let report = if report == Self::Heard {
                Self::Pending
            } else {
                report
            };
            return (report, Behind::Ended);
        }
        (report, behind)
    }

    /// The report of a chat one of whose pull requests' CI ended: the agent
    /// that heard its last report hears it again, with how CI ended;
    /// otherwise it hears so with the report it has yet to hear.
    fn ci_ended(&mut self) {
        if *self == Self::Heard {
            *self = Self::Pending;
        }
    }
}

/// What the agent that opened a chat heard of what the chat's agent runs
/// in the background, apart from its turns: once it heard its report
/// while something ran, it hears again once all of that ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Behind {
    /// It heard nothing of it.
    Untold,
    /// Its report said the chat works in the background.
    Told,
    /// What ran in the background ended since: its report says so.
    Ended,
}

/// When the agent of the chat that opened subagents can hear their
/// reports.
enum Hears {
    /// Now: it waits for a line, or its closed chat starts again on one.
    Now,
    /// Once its turn ends.
    Later,
    /// Never: its chat is archived, gone, or closed for good, so that the
    /// subagents are the human's.
    Never,
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
    /// What the agent that opened the chat heard of what the chat's agent
    /// runs in the background.
    behind: Behind,
    goal: Option<Goal>,
    parent: Option<u64>,
    /// Whose it is; a line the human sends it makes it theirs.
    whose: Whose,
    setup: Option<Setup>,
    /// The folder it works in, when it is not its project's, as one that
    /// opened before another folder of the project became the project's.
    folder: Option<PathBuf>,
    ending: Option<(Ends, workflow::Ending)>,
    /// The pull requests its agent opened.
    pulls: pulls::Pulls,
    /// What its agent spent, as it was kept, until the chat is read: its
    /// conversation holds it from then.
    spent: Option<Spent>,
    /// When it was marked done, and by whom, while it is (see
    /// [`resolve`]).
    resolved: Option<Resolved>,
    /// When it last moved, in milliseconds since the epoch: when a turn
    /// last started or ended, or, before any did in this run, when the
    /// last entry of its conversation was kept, or it was opened.
    active: i64,
    /// The chat, once it was read or opened in this run.
    chat: Option<Chat>,
}

impl Slot {
    /// How its chat stands, once read, as the agent that waits for it
    /// hears it: with how CI ended on the pull requests it opened, when
    /// that agent is to hear so.
    fn stands(&self) -> Option<Waited> {
        let mut stands = self.chat.as_ref()?.stands();
        if let Whose::Agent { .. } = self.whose {
            stands.ci.clone_from(&self.pulls.told);
        }
        Some(stands)
    }

    /// Whether the chat is at work at `now`: in a turn, or with its agent
    /// at work by itself apart from one (see [`Chat::goes_on`]). Stopping
    /// coxd, closing its agent, or sending it another line then would cut
    /// it off.
    fn at_work(&self, now: Instant) -> bool {
        working(self.state) || self.chat.as_ref().is_some_and(|chat| chat.goes_on(now))
    }

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
                        .is_none_or(|folder| folder.exists() || worktrees::in_worktrees(folder))
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
            background: self.chat.as_ref().map_or_default(Chat::background),
            pulls: Vec::new(),
            spent: match &self.chat {
                Some(chat) => chat.conversation().spent().cloned(),
                None => self.spent.clone(),
            },
            resolved: self.resolved,
            steers: self
                .chat
                .as_ref()
                .is_some_and(|chat| chat.conversation().steers()),
            on_steps: self.chat.as_ref().map_or_default(Chat::waiting_on_steps),
            active: self.active,
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
    /// The phones coxd serves the page to, for the human to unpair them
    /// all, unless it serves none.
    phones: Option<Rc<phone::Phones>>,
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
    /// The project the human chose last, by its place in the projects, as
    /// kept past a restart.
    chosen: usize,
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
    /// What coxd knows of the pull requests the chats' agents opened.
    watches: pulls::Watches,
    /// Why no chat shows its pull requests, as when `gh` is not logged
    /// in, if that is so.
    github: Option<String>,
    /// Whether coxd looks which worktrees it may take away (see
    /// [`tidy`]).
    tidying: tidy::Tidying,
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
/// how to tell it what the human does, and to stop. Its thread keeps
/// what lasts, and trashes, where this thread was told to (see
/// [`storage::keep_on_this_thread`]).
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
    let data = storage::kept_on_this_thread();
    let trash = crate::trash::kept_on_this_thread();
    let thread = std::thread::Builder::new()
        .name("daemon".to_owned())
        .spawn(move || {
            storage::keep_on_this_thread(data);
            crate::trash::keep_on_this_thread(trash);
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
                about: None,
                deleted: false,
                last: None,
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
            phones: None,
            listed: None,
            running: HashMap::new(),
            seeing: HashMap::new(),
            stops: HashMap::new(),
            preparing: HashMap::new(),
            last_id: 0,
            reveals: Vec::new(),
            chosen: 0,
            window_id: None,
            done: false,
            ran: None,
            scheduled: HashMap::new(),
            watches: pulls::Watches::new(),
            github: None,
            tidying: tidy::Tidying::default(),
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
                about: project.about,
                deleted: false,
                last: None,
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
                behind: Behind::Untold,
                goal: chat.goal,
                parent,
                whose: Whose::opened(parent, setup.as_ref()),
                setup,
                folder: chat.folder,
                ending: None,
                pulls: pulls::Pulls::default(),
                spent: chat.spent.as_deref().and_then(|spent| {
                    serde_json::from_str(spent)
                        .inspect_err(|error| {
                            tracing::error!(
                                "what the chat's agent spent cannot be read: {error:#}"
                            );
                        })
                        .ok()
                }),
                resolved: chat.resolved,
                active: chat.active,
                chat: None,
            };
            if chat.archived {
                self.archived.push(slot);
            } else {
                self.add(slot);
            }
        }
        self.read_shown();
    }

    /// Reads the chat the window showed last of each project, and the
    /// project the human chose last, as kept.
    fn read_shown(&mut self) {
        let shown = match self.store.shown() {
            Ok(shown) => shown,
            Err(error) => {
                tracing::error!("the chats shown last cannot be read: {error:#}");
                return;
            }
        };
        // The project chosen last comes last.
        for kept in shown {
            let Some(index) = self
                .projects
                .iter()
                .position(|project| project.id == kept.project)
            else {
                continue;
            };
            let key = kept.chat.and_then(|chat| {
                self.chats
                    .iter()
                    .chain(&self.archived)
                    .find(|slot| slot.kept == Some(chat))
                    .map(|slot| slot.key)
            });
            self.projects[index].last = key;
            self.chosen = index;
        }
    }

    /// Keeps that the window shows the chat `key`, or none, of the project
    /// at `project`, the one the human chose, for a window that starts to
    /// go back to.
    fn keep_shown(&mut self, key: Option<u64>, project: usize) {
        let slot = key.and_then(|key| self.slot(key));
        let project = slot.map_or(project, |slot| slot.project);
        let kept = slot.and_then(|slot| slot.kept);
        let Some(chosen) = self
            .projects
            .get_mut(project)
            .filter(|project| !project.deleted)
        else {
            return;
        };
        if key.is_some() {
            chosen.last = key;
        }
        let id = chosen.id;
        self.chosen = project;
        if let Err(error) = self.store.keep_shown(id, kept) {
            tracing::error!("the chat shown is not kept: {error:#}");
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

    /// How the chat `key` stands, as the agent that waits for it hears it
    /// (see [`Slot::stands`]).
    fn stands(&mut self, key: u64) -> Option<Waited> {
        self.chat(key)?;
        self.slot(key)?.stands()
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
        let spent = slot.spent.clone();
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
        let commands = self.store.commands(kept).unwrap_or_else(|error| {
            tracing::error!("the chat's commands cannot be read: {error:#}");
            Vec::new()
        });
        // A chat of a sketch whose folder is gone from under it does not
        // start again; one of a worktree Cox took away has it made again
        // as it does (see [`Self::remake_worktree`]).
        let gone = setup.as_ref().is_some_and(|setup| setup.folder.is_some())
            && !cwd.exists()
            && !worktrees::in_worktrees(&cwd);
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
            .with_commands(commands)
            .with_spent(spent)
            .with_keeping(keeps);
        let chat = self.make_chat(key, Some(kept), conversation, again);
        if let Some(slot) = self.slot_mut(key) {
            slot.chat = Some(chat);
            slot.state = State::Closed;
        }
    }

    /// The chat `key`, kept as `kept` if it is, showing `conversation`,
    /// with the agent chosen for it and that agent's judge; `again` says
    /// how a closed chat starts again.
    fn make_chat(
        &self,
        key: u64,
        kept: Option<ChatId>,
        conversation: Conversation,
        again: Option<Again>,
    ) -> Chat {
        let record = kept.map(|kept| Record::new(Rc::clone(&self.store), kept));
        let agents = self.spawns.agents();
        let chosen = agent_of(&agents, kept, &self.store);
        let starts = self.spawns.starts(&chosen);
        Chat::new(
            key,
            conversation,
            record,
            again,
            Some(starts),
            Some((agents, chosen)),
            self.internal.clone(),
        )
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

    /// Unpairs every phone, as the human asked in the settings: makes the
    /// secret a phone pairs with again, lets in only the phones paired
    /// with it from now on, pushes to none paired before, and ends what
    /// each phone there follows the chats on. Tells the window whether it did.
    fn unpair_phones(&mut self) {
        let failed = match phone::new_token() {
            Ok(secret) => {
                if let Some(phones) = &self.phones {
                    phones.unpaired(secret);
                }
                self.viewers.clear();
                tracing::info!("every phone is unpaired");
                None
            }
            Err(error) => {
                tracing::error!("the phones cannot be unpaired: {error:#}");
                Some(format!("{error:#}"))
            }
        };
        self.tell(FromDaemon::Unpaired { failed });
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
            kept: self.slot(*key).and_then(|slot| slot.kept),
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

    /// The project at `folder`, by its place in the projects: not one the
    /// human deleted, which is at no folder (see [`cox_core::folder`]).
    fn project_at(&self, folder: &Path) -> Option<usize> {
        self.projects
            .iter()
            .position(|project| !project.deleted && project.folder == folder)
    }

    /// The project of `folder`, by its place in the projects, made and
    /// kept if there is none yet.
    fn project(&mut self, folder: PathBuf) -> usize {
        if let Some(index) = self.project_at(&folder) {
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
            about: None,
            deleted: false,
            last: None,
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
            .find(|folder| !folder.exists() && self.project_at(folder).is_none())
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
        project.opened += 1;
        let name = format!("Chat {}", project.opened);
        let project_id = project.id;
        let instructions = self.instructions(index, &cwd, None);
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
        let key = self.next_key();
        let conversation = Chat::opening(&instructions.paths(), setup.as_ref());
        // Its agent, once it closed, as when it ended or did not stop,
        // starts again with the next line, going on with its session.
        let again = Again {
            cwd: setup
                .as_ref()
                .and_then(|setup| setup.folder.clone())
                .unwrap_or(cwd),
            session: None,
            setup: setup.clone(),
        };
        let chat = self.make_chat(key, kept, conversation, Some(again));
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
            behind: Behind::Untold,
            goal: None,
            parent: from.map(|(parent, _)| parent),
            whose: Whose::opened(from.map(|(parent, _)| parent), setup.as_ref()),
            setup: setup.clone(),
            folder,
            ending: None,
            pulls: pulls::Pulls::default(),
            spent: None,
            resolved: None,
            active: cox_core::store::now(),
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
        // The first project is Cox's own chats', in none.
        let in_project = slot.project != 0;
        let top_level = slot.parent.is_none();
        let subagents = self.subagents_of(slot.kept, &start);
        let Spawned { agent, stage } = self
            .spawns
            .chat(cwd, start, &agent, relay, in_project, subagents, top_level);
        let also = stage.map_or_default(|mut stage| {
            let internal = self.internal.clone();
            vec![spawn(async move {
                while let Some(request) = stage.recv().await {
                    if internal.send(Internal::Stage { key, request }).is_err() {
                        break;
                    }
                }
            })]
        });
        if let Some(chat) = self.slot_mut(key).and_then(|slot| slot.chat.as_mut()) {
            chat.attach(agent, generation, also);
        }
    }

    /// Where the agent of the chat kept as `kept`, starting as `start`
    /// says, runs its subagents: as its session did, when it goes on with
    /// one, so that a change to the settings leaves its tools as they
    /// were, or else as the settings say now, which the chat keeps for
    /// the session it starts.
    fn subagents_of(&self, kept: Option<ChatId>, start: &Start) -> Subagents {
        let resumes = matches!(
            start,
            Start::Again {
                session: Some(_),
                ..
            }
        );
        if let Some(kept) = kept.filter(|_| resumes) {
            return match self.store.subagents(kept) {
                Ok(subagents) => subagents
                    .as_deref()
                    .and_then(Subagents::named)
                    .unwrap_or_default(),
                Err(error) => {
                    tracing::error!("the chat's subagents cannot be read: {error:#}");
                    Subagents::default()
                }
            };
        }
        let subagents = self.spawns.subagents();
        if let Some(kept) = kept
            && let Err(error) = self.store.keep_subagents(kept, subagents.name())
        {
            tracing::error!("the chat's subagents are not kept: {error:#}");
        }
        subagents
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
                    Some(folder) => self.project_at(&folder),
                };
                if let Some(index) = index.filter(|&index| self.projects[index].folder.is_dir()) {
                    self.open_in(index, None, None);
                }
            }
            ToDaemon::Shown { key, project } => {
                self.shown = key;
                self.keep_shown(key, project);
                if let Some(slot) = key.and_then(|key| self.slot_mut(key)) {
                    slot.unread = false;
                    slot.pulls.seen();
                }
            }
            ToDaemon::Watch { key } => self.watch(key),
            ToDaemon::Submit {
                key,
                text,
                attached,
                step,
            } => self.submit(key, &text, attached, step),
            ToDaemon::Choose { key, index } => {
                if let Some(chat) = self.chat(key) {
                    chat.choose(index);
                }
            }
            ToDaemon::AnswerQuestion { key, answer } => {
                if let Some(chat) = self.chat(key) {
                    chat.answer_question(answer);
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
                self.answer_proposal(key, index, answer);
            }
            ToDaemon::ChangeProposal { key, index, goal } => {
                if let Some(chat) = self.chat(key) {
                    chat.change_proposal(index, goal);
                }
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
                match self.start_sketch(key, sketch, None, &what) {
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
            ToDaemon::Resolve { key, resolved } => {
                if resolved {
                    self.resolve(key, ResolvedBy::Human);
                } else {
                    self.reopen(key);
                }
            }
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
            ToDaemon::UnpairPhones => self.unpair_phones(),
            ToDaemon::RenameProject { project, name } => self.rename_project(project, &name),
            ToDaemon::SetAbout { project, about } => {
                if let Some(index) = self.index_of(project) {
                    self.set_about(index, &about);
                }
            }
            ToDaemon::CompareWith {
                folder,
                directory,
                base,
            } => self.compare_with(&folder, &directory, base.as_deref()),
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

    /// What the agent of a chat of the project at `index` that works in
    /// `cwd` is told first: what the project is, if it says, the
    /// instructions, the index of the project's own wiki, if it has pages,
    /// then the project's chats but the chat `except`.
    fn instructions(&self, index: usize, cwd: &Path, except: Option<u64>) -> Instructions {
        let project = &self.projects[index];
        let instructions = Instructions::read(cwd);
        if index == 0 {
            return instructions;
        }
        let wiki = project.folder.join(crate::wiki::PROJECT_WIKI);
        let pages = crate::wiki::read(&wiki, crate::wiki::INDEX).ok();
        instructions
            .about(&project.name, project.about.as_deref())
            .wiki(&wiki, pages.as_deref())
            .chats(&self.chats_told(index, except))
    }

    /// What the agent of the chat `key`, which works in `cwd`, is told
    /// first (see [`Self::instructions`]).
    fn instructions_of(&self, key: u64, cwd: &Path) -> Instructions {
        let index = self.slot(key).map_or(0, |slot| slot.project);
        self.instructions(index, cwd, Some(key))
    }

    /// The chats of the project at `index` but the chat `except`, as its
    /// chats' agents are told them, a line each: the last
    /// [`CHATS_TOLD`] the list shows, by the numbers they are kept by,
    /// which `read_chat` takes.
    fn chats_told(&self, index: usize, except: Option<u64>) -> Vec<String> {
        let kept = |key: u64| self.slot(key).and_then(|slot| slot.kept);
        let lines: Vec<String> = self
            .chats
            .iter()
            .filter(|slot| slot.project == index && Some(slot.key) != except)
            .filter_map(|slot| {
                let number = slot.kept?;
                let state = match slot.state {
                    _ if slot.resolved.is_some() => "marked done",
                    State::Asking => "waits for the human to answer it",
                    state if working(state) => "at work",
                    State::Closed => "closed",
                    _ => "idle",
                };
                let mut line = format!("- chat {number}: \"{}\", {state}", slot.name);
                if let Some(goal) = &slot.goal {
                    line.push_str("; works until: ");
                    line.push_str(&goal.input());
                }
                if let Some(parent) = slot.parent.and_then(kept) {
                    let _ = write!(line, "; opened from chat {parent}");
                }
                if slot
                    .setup
                    .as_ref()
                    .is_some_and(|setup| setup.folder.is_some())
                {
                    line.push_str("; a sketch, apart from the project");
                }
                Some(line)
            })
            .collect();
        let from = lines.len().saturating_sub(CHATS_TOLD);
        lines[from..].to_vec()
    }

    /// The chats the agent of the chat `key` may ask about, open or not:
    /// those of its project; none while it is gone.
    fn seen_by(&self, key: u64) -> Option<impl Iterator<Item = &Slot>> {
        let project = self.slot(key)?.project;
        Some(
            self.chats
                .iter()
                .chain(&self.archived)
                .filter(move |slot| slot.project == project),
        )
    }

    /// What the chat kept as `number`, of the project of the chat `key`,
    /// talked through, as its agent reads it, or why it cannot.
    fn read_chat(&self, key: u64, number: i64) -> Result<String, String> {
        let slot = self
            .seen_by(key)
            .ok_or("the chat has closed")?
            .find(|slot| slot.kept.is_some_and(|kept| kept.number() == number))
            .ok_or_else(|| format!("This project has no chat {number}."))?;
        let entries = match &slot.chat {
            Some(chat) => chat.conversation().entries().to_vec(),
            None => slot
                .kept
                .map(|kept| self.store.entries(kept))
                .transpose()
                .map_err(|error| format!("Chat {number} cannot be read: {error:#}"))?
                .unwrap_or_default(),
        };
        Ok(crate::agent::read_out(number, &slot.name, &entries)
            .unwrap_or_else(|| format!("Nothing was said in chat {number} yet.")))
    }

    /// The human sent `text` in the chat `key`, on the step of its plan
    /// `step` names, if it does, which makes it theirs: a closed chat's
    /// agent starts again first, if it can.
    fn submit(&mut self, key: u64, text: &str, attached: Vec<Attachment>, step: Option<OnStep>) {
        if text.trim().is_empty() {
            return;
        }
        self.reopen(key);
        if let Some(slot) = self.slot_mut(key) {
            slot.whose = Whose::Human;
        }
        // Cox tells the chat why its agent does not start, if it does not.
        let _ = self.start_again(key);
        if let Some(chat) = self.chat(key) {
            chat.submit(text, attached, step);
        }
    }

    /// Sends `text` to the agent of the chat `key` in the human's place,
    /// starting it again first if it is closed and can be.
    fn go_on(&mut self, key: u64, text: &str) {
        self.reopen(key);
        let _ = self.start_again(key);
        if let Some(chat) = self.chat(key) {
            chat.told(text);
        }
    }

    /// Starts the agent of the chat `key` again, if it is closed and can
    /// be, making the worktree it works in again first, if Cox took it
    /// away (see [`Self::remake_worktree`]).
    ///
    /// # Errors
    ///
    /// When Cox cannot make that worktree again, with why, as the chat is
    /// told: its agent does not start.
    fn start_again(&mut self, key: u64) -> Result<(), String> {
        self.remake_worktree(key)?;
        let Some(chat) = self.chat(key) else {
            return Ok(());
        };
        let earlier = chat.conversation().entries().to_vec();
        if let Some(again) = chat.take_again() {
            let Again {
                cwd,
                session,
                setup,
            } = again;
            self.start_agent(
                key,
                Start::again(self.instructions_of(key, &cwd), session, &earlier, setup),
            );
        }
        Ok(())
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
            Internal::StopDue { key, generation } => {
                if let Some(chat) = self.chat(key) {
                    chat.stop_due(generation);
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
                answer,
            } => {
                let taken = self.attach(id, hello, window, Instant::now());
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
                    slot.pulls.seen();
                }
                self.view_chat(id, key);
            }
            Internal::Act { message } => self.act(message),
            Internal::Key { number, answer } => {
                let key = self
                    .chats
                    .iter()
                    .chain(&self.archived)
                    .find(|slot| slot.kept.is_some_and(|kept| kept.number() == number))
                    .map(|slot| slot.key);
                answer.send(key).ok();
            }
            Internal::Folder { key, answer } => {
                answer.send(self.slot(key).map(|slot| self.cwd(slot))).ok();
            }
            Internal::Content { key, at, answer } => {
                let content = self.chat(key).and_then(|chat| {
                    match chat.conversation().entries().get(at)? {
                        Entry::ToolCall { content, .. } | Entry::Permission { content, .. } => {
                            Some(content.clone())
                        }
                        _ => None,
                    }
                });
                answer.send(content).ok();
            }
            Internal::Pages { key, answer } => {
                let pages = self.slot(key).map(|slot| self.cwd(slot)).map(|folder| {
                    let pages = self
                        .stage_in(&folder)
                        .unwrap_or_default()
                        .tabs
                        .into_iter()
                        .filter_map(|tab| match tab {
                            KeptTab::Page { url, title } => Some((url, title)),
                            _ => None,
                        })
                        .collect();
                    (folder, pages)
                });
                answer.send(pages).ok();
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
            Internal::Settled { key } => {
                if let Some(chat) = self.chat(key) {
                    chat.background_settles();
                }
            }
            Internal::Schedules => {
                self.run_schedules(&crate::settings::workflow(), &jiff::Zoned::now());
            }
            Internal::Pulls => {
                let now = Instant::now();
                self.look_at_pulls(now);
                self.tidy_worktrees(now, false);
            }
            Internal::Pulled { pull, read } => self.pulled(&pull, read, Instant::now()),
            Internal::Tidied { removed } => self.tidied(removed),
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
        let arranged = storage::path(storage::Kept::Triggers);
        let scheduled = crate::trigger::scheduled(workflow, &home, &projects, arranged.as_deref());
        if scheduled.is_empty() {
            return;
        }
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
                .is_some_and(|slot| slot.at_work(Instant::now()));
            let last = ran.last(&id);
            if at_work || !cron.due(last, now) {
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
    /// is there, or the window speaks otherwise than the daemon or asks
    /// it to stop, as a chat is at work at `now` or not (see
    /// [`Hello::answer`] and [`Slot::at_work`]). Answers which of those
    /// it is.
    fn attach(
        &mut self,
        id: u64,
        hello: Hello,
        window: UnboundedSender<FromDaemon>,
        now: Instant,
    ) -> FromDaemon {
        if self.window.is_some() {
            return FromDaemon::Taken;
        }
        let at_work = self
            .chats
            .iter()
            .chain(&self.archived)
            .any(|slot| slot.at_work(now));
        let answer = hello.answer(wire::VERSION, at_work);
        match answer {
            FromDaemon::Joined(_) => {}
            FromDaemon::Busy => {
                tracing::warn!("a window that coxd is to stop for waits, since a chat is at work");
                if hello.reads_at_work {
                    return FromDaemon::AtWork(self.chats_at_work(now));
                }
                return answer;
            }
            _ => return answer,
        }
        if hello.version != Some(wire::VERSION) {
            tracing::info!("a window of another build that speaks alike joins");
        }
        let Hello { home, command, .. } = hello;
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
        answer
    }

    /// The chats at work at `now`, as a window waiting for coxd to stop
    /// tells the human.
    fn chats_at_work(&self, now: Instant) -> Vec<AtWork> {
        self.chats
            .iter()
            .chain(&self.archived)
            .filter(|slot| slot.at_work(now))
            .map(|slot| AtWork {
                project: self
                    .projects
                    .get(slot.project)
                    .filter(|project| project.id.is_some())
                    .map(|project| project.name.clone()),
                chat: slot.name.clone(),
            })
            .collect()
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
    #[expect(clippy::too_many_lines, reason = "one arm for each request")]
    fn asked(&mut self, key: u64, request: ChatRequest) {
        match request {
            ChatRequest::StartSketch {
                sketch,
                directory,
                what,
                show,
                answer,
            } => {
                let opened = self.start_sketch(key, sketch, directory.as_deref(), &what);
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
            ChatRequest::ReadChat { chat, answer } => {
                answer.send(self.read_chat(key, chat)).ok();
            }
            ChatRequest::ResolveChat { chat, answer } => {
                answer.send(self.resolve_chat(key, chat)).ok();
            }
            ChatRequest::ProposeAbout { about, why, answer } => {
                answer.send(self.propose_about(key, &about, why)).ok();
            }
            ChatRequest::ProposeIcon {
                file,
                from,
                why,
                answer,
            } => {
                answer.send(self.propose_icon(key, &file, &from, why)).ok();
            }
            ChatRequest::ProposeProject {
                name,
                why,
                sketch,
                answer,
            } => {
                answer
                    .send(self.propose_project(key, &name, why, sketch))
                    .ok();
            }
            ChatRequest::OpenChat {
                text,
                title,
                setup,
                model,
                goal,
                sketch,
                within,
                answer,
            } => {
                let setup = Setup { sketch, ..setup };
                let opened =
                    self.open_chat(key, &text, title, setup, model.as_deref(), goal, *within);
                answer.send(opened).ok();
            }
            ChatRequest::WaitChat { chat, answer } => self.wait_chat(key, chat, answer),
            ChatRequest::FindSubagent { subagent, answer } => {
                answer.send(self.find_subagent(key, &subagent)).ok();
            }
            ChatRequest::ContinueChat {
                subagent,
                text,
                answer,
            } => {
                answer.send(self.continue_chat(key, &subagent, &text)).ok();
            }
            ChatRequest::StopChat {
                subagent,
                reason,
                answer,
            } => {
                answer
                    .send(self.stop_subagent(key, &subagent, reason.as_deref()))
                    .ok();
            }
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
                let whereabouts = self.slot(key).map_or_default(|slot| Whereabouts {
                    project: self.projects[slot.project].name.clone(),
                    // The first is Cox's own, in the home.
                    folder: (slot.project != 0).then(|| self.projects[slot.project].folder.clone()),
                    chat: slot.name.clone(),
                    kept: slot.kept.map(|kept| kept.to_string()),
                });
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
            Some(which) if !self.save_to_wiki(which) => {
                Some("its agent is at work, or has no wiki_write: ask once it waits")
            }
            Some(_) => None,
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
        let saves = self
            .chats
            .iter()
            .find(|slot| slot.key == key)
            .is_some_and(|slot| saves_to_wiki(slot.state, slot.setup.as_ref()));
        if !saves {
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
            .is_some_and(|slot| !slot.at_work(Instant::now()) && merges(slot.setup.as_ref()));
        if merges {
            let line = workflow::CLOSE_MERGE.say(&[]);
            self.reopen(key);
            let _ = self.start_again(key);
            if let Some(chat) = self.chat(key) {
                chat.merge(&line);
            }
        }
    }

    /// Answers `answer` with how the chat that goes by `key`, of the
    /// project of the chat `asking`, stands once it is not at work, or at
    /// once when it is not.
    fn wait_chat(
        &mut self,
        asking: u64,
        key: u64,
        answer: oneshot::Sender<Result<Waited, String>>,
    ) {
        let seen = self
            .seen_by(asking)
            .is_some_and(|mut chats| chats.any(|slot| slot.key == key));
        if !seen {
            answer.send(Err(format!("no chat {key}"))).ok();
            return;
        }
        // Asked again, it is not done.
        self.reopen(key);
        let Some(chat) = self.chat(key) else {
            answer.send(Ok(Waited::default())).ok();
            return;
        };
        if at_work(chat.conversation().state()) {
            self.waiting.retain(|(_, answer)| !answer.is_closed());
            self.waiting.push((key, answer));
        } else {
            answer.send(Ok(self.stands(key).unwrap_or_default())).ok();
            self.heard(key);
        }
    }

    /// Sends `text`, in the place of the chat `asking`, to the subagent it
    /// names as `subagent`, one it opened, as its next line: brought back
    /// to the list if archived, and its agent started again on its session
    /// first if it is closed, as one cut off by a stop, or by coxd going,
    /// is. The chat `asking` hears its report once it ends, as it heard
    /// the first, even if the human took the chat meanwhile, unless it is
    /// a sketch, which stays the human's. Answers its
    /// number and title, or why it cannot go on: at work still, or waiting
    /// for the human, it is not cut off, and the agent waits for its
    /// report first.
    fn continue_chat(
        &mut self,
        asking: u64,
        subagent: &Subagent,
        text: &str,
    ) -> Result<(u64, String), String> {
        if text.trim().is_empty() {
            return Err("no prompt: say what the subagent is to do next as `prompt`".to_owned());
        }
        let (child, name) = self
            .find_subagent(asking, subagent)
            .map_err(|why| why.replace("agent_report", "agent_continue"))?;
        let slot = self.slot(child).ok_or_else(|| format!("no chat {child}"))?;
        if slot.parent != Some(asking) {
            return Err(format!(
                "The chat \"{name}\" is not a subagent this chat ran with agent: only those go \
                 on with agent_continue."
            ));
        }
        if slot.state == State::Asking {
            return Err(format!(
                "The subagent \"{name}\" waits for the human to answer: it goes on once they \
                 do. Get its report with agent_report, giving its title, then continue it."
            ));
        }
        if slot.at_work(Instant::now()) {
            return Err(format!(
                "The subagent \"{name}\" is at work still, so nothing is sent to it. Wait for \
                 its report, as Cox tells it, or with agent_report, giving its title, then \
                 continue it."
            ));
        }
        if self.archived.iter().any(|slot| slot.key == child) {
            self.archive(child, false);
            // Back under the chat that opened it, as a subagent opened anew
            // is, without taking the human's eye.
            self.reveals.retain(|key| *key != child);
        }
        self.reopen(child);
        self.start_again(child).map_err(|why| {
            format!(
                "The subagent \"{name}\" cannot start again: the worktree it works in is gone, \
                 and Cox cannot make it again: {why}."
            )
        })?;
        let chat = self.chat(child).ok_or_else(|| format!("no chat {child}"))?;
        if chat.conversation().state() != State::Idle {
            return Err(format!(
                "The subagent \"{name}\" is closed and cannot start again, as when the folder \
                 of its sketch is gone: run the work anew with agent."
            ));
        }
        chat.told(text);
        // Whose it is goes back to what it was opened as: a sketch stays
        // the human's to end.
        if let Some(slot) = self.slot_mut(child) {
            slot.whose = Whose::opened(Some(asking), slot.setup.as_ref());
        }
        Ok((child, name))
    }

    /// Stops, in the place of the chat `asking`, the subagent it names as
    /// `subagent`, one it opened, as the human stops a chat: its turn, as
    /// it waits for the human too, whose question goes; or, once its turn
    /// ended, what its agent runs in the background, by ending the agent,
    /// which the next line starts again on its session. Its chat stays
    /// open, and shows that the agent of `asking` stopped it, for `reason`
    /// if given. Answers its number and title, and what was stopped, or why
    /// it cannot be stopped: one the human took over is theirs.
    fn stop_subagent(
        &mut self,
        asking: u64,
        subagent: &Subagent,
        reason: Option<&str>,
    ) -> Result<(u64, String, Stopped), String> {
        let (child, name) = self
            .find_subagent(asking, subagent)
            .map_err(|why| why.replace("agent_report", "agent_stop"))?;
        let slot = self.slot(child).ok_or_else(|| format!("no chat {child}"))?;
        if slot.parent != Some(asking) {
            return Err(format!(
                "The chat \"{name}\" is not a subagent this chat ran with agent: only those are \
                 stopped with agent_stop. Ask the human to stop it."
            ));
        }
        let sketch = slot
            .setup
            .as_ref()
            .is_some_and(|setup| setup.sketch.is_some());
        if slot.whose == Whose::Human && !sketch {
            return Err(format!(
                "The human took the subagent \"{name}\" over, so it is theirs to stop: ask \
                 them to."
            ));
        }
        let background = slot.at_work(Instant::now());
        let opener = self.slot(asking).map_or_default(|slot| slot.name.clone());
        let Some(chat) = self.chat(child) else {
            return Ok((child, name, Stopped::Nothing));
        };
        let stopped = if working(chat.conversation().state()) {
            Stopped::Turn
        } else if background {
            Stopped::Background
        } else {
            return Ok((child, name, Stopped::Nothing));
        };
        chat.note(
            match reason.map(str::trim).filter(|reason| !reason.is_empty()) {
                Some(reason) => format!("The agent of \"{opener}\" stopped this chat: {reason}"),
                None => format!("The agent of \"{opener}\" stopped this chat."),
            },
        );
        match stopped {
            Stopped::Turn => chat.stop(),
            Stopped::Background | Stopped::Nothing => chat.end_background(),
        }
        Ok((child, name, stopped))
    }

    /// The number and the title of the chat the agent of the chat
    /// `asking` names as `subagent`: by its number, among the chats it may
    /// see, or by its title, among those it opened; or why there is none,
    /// or, when several have the title, which they are.
    fn find_subagent(&self, asking: u64, subagent: &Subagent) -> Result<(u64, String), String> {
        let title = match subagent {
            Subagent::Number(number) => {
                return self
                    .seen_by(asking)
                    .and_then(|mut chats| chats.find(|slot| slot.key == *number))
                    .map(|slot| (slot.key, slot.name.clone()))
                    .ok_or_else(|| format!("no chat {number}"));
            }
            Subagent::Title(title) => title,
        };
        let own: Vec<&Slot> = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter(|slot| slot.parent == Some(asking))
            .collect();
        let titled: Vec<&Slot> = own
            .iter()
            .copied()
            .filter(|slot| slot.name == *title)
            .collect();
        match titled[..] {
            [slot] => Ok((slot.key, slot.name.clone())),
            [] if own.is_empty() => Err(format!(
                "No subagent of this chat is titled \"{title}\": this chat opened none."
            )),
            // A subagent's agent may title its chat anew: the human sees
            // it by that title now.
            [] => Err(format!(
                "No subagent of this chat is titled \"{title}\". Its subagents go by {} now: \
                 give agent_report one of those, and name it so to the human.",
                own.iter()
                    .map(|slot| format!("\"{}\"", slot.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            _ => Err(format!(
                "Several subagents of this chat are titled \"{title}\", with the numbers {}: \
                 give agent_report the number of the one you mean as `chat`.",
                titled
                    .iter()
                    .map(|slot| slot.key.to_string())
                    .collect::<Vec<_>>()
                    .join(" and ")
            )),
        }
    }

    /// Marks the report of the chat `key` heard by the agent that opened
    /// it, so that Cox does not tell it again, and the chat done, once
    /// its report reached that agent and nothing of it runs: a report of
    /// CI that ended leaves it open, as CI ending reopened it.
    /// What it heard of what the chat runs in the background, it hears
    /// again once that ended (see [`Behind`]).
    fn heard(&mut self, key: u64) {
        let ci = self
            .slot(key)
            .is_some_and(|slot| !slot.pulls.told.is_empty());
        let behind = if self
            .chat(key)
            .is_some_and(|chat| !chat.background().is_empty())
        {
            Behind::Told
        } else {
            Behind::Untold
        };
        let mut done = false;
        if let Some(Slot {
            whose: Whose::Agent { report, .. },
            behind: heard,
            ..
        }) = self.slot_mut(key)
        {
            if *report == Report::Pending {
                *report = Report::Heard;
                done = behind == Behind::Untold && !ci;
            }
            *heard = behind;
        }
        if done
            && self
                .slot(key)
                .is_some_and(|slot| !slot.at_work(Instant::now()))
        {
            self.resolve(key, ResolvedBy::Agent);
        }
        if let Some(
            slot @ Slot {
                whose: Whose::Agent { .. },
                ..
            },
        ) = self.slot_mut(key)
        {
            slot.pulls.told.clear();
        }
    }

    /// Tells the agent of each chat that opened subagents, in the human's
    /// place, the reports it has not heard, in one line once its turn is
    /// not running, and gives the human the subagents of a chat whose
    /// agent never hears them. Answers whether it told any agent.
    fn tell_reports(&mut self) -> bool {
        let mut pending: Vec<(u64, Vec<u64>)> = Vec::new();
        for slot in self.chats.iter().chain(&self.archived) {
            if let Whose::Agent {
                parent,
                report: Report::Pending,
            } = slot.whose
            {
                match pending.iter_mut().find(|(of, _)| *of == parent) {
                    Some((_, children)) => children.push(slot.key),
                    None => pending.push((parent, vec![slot.key])),
                }
            }
        }
        let mut told = false;
        for (parent, children) in pending {
            match self.hears(parent) {
                Hears::Now => {
                    let reports: Vec<String> = children
                        .iter()
                        .filter_map(|&child| {
                            let slot = self.slot(child)?;
                            let (name, ended) = (slot.name.clone(), slot.behind == Behind::Ended);
                            let stands = self.stands(child)?;
                            Some(workflow::subagent_report(child, &name, &stands, ended))
                        })
                        .collect();
                    let line =
                        workflow::SUBAGENT_REPORTS.say(&[("reports", &reports.join("\n\n"))]);
                    self.go_on(parent, &line);
                    for child in children {
                        self.heard(child);
                    }
                    told = true;
                }
                Hears::Later => {}
                Hears::Never => self.orphan(parent),
            }
        }
        told
    }

    /// When the agent of the chat `key` can hear the reports of the
    /// subagents it opened.
    fn hears(&mut self, key: u64) -> Hears {
        if !self.chats.iter().any(|slot| slot.key == key) {
            return Hears::Never;
        }
        let Some(chat) = self.chat(key) else {
            return Hears::Never;
        };
        match chat.conversation().state() {
            State::Idle => Hears::Now,
            State::Closed if chat.starts_again() => Hears::Now,
            State::Closed => Hears::Never,
            State::Busy | State::Asking | State::Stopping | State::Checking => Hears::Later,
        }
    }

    /// Gives the human the chats the agent of the chat `parent` opened,
    /// as no agent is left to hear of them: one whose report it had not
    /// heard waits for the human, as a turn of theirs that ended would.
    fn orphan(&mut self, parent: u64) {
        let shown = self.shown;
        let mut news = Vec::new();
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            let Whose::Agent { parent: of, report } = slot.whose else {
                continue;
            };
            if of != parent {
                continue;
            }
            slot.whose = Whose::Human;
            if report != Report::Pending || slot.resolved.is_some() {
                continue;
            }
            if shown != Some(slot.key) {
                slot.unread = true;
            }
            let said = slot
                .chat
                .as_ref()
                .and_then(|chat| chat.conversation().last_said())
                .unwrap_or("The turn ended.");
            news.push(FromDaemon::News {
                key: slot.key,
                name: slot.name.clone(),
                body: said.to_owned(),
            });
        }
        for message in news {
            self.tell_news(message);
        }
    }

    /// Tells the window, the phones, and, while the human is away, their
    /// phones' notifications, of what a chat came to.
    fn tell_news(&mut self, message: FromDaemon) {
        self.push(&message);
        self.tell_phones(&message);
        self.tell(message);
    }

    /// Opens a chat under the chat `key`, in its project, set up as
    /// `setup` says, kept to what the chat `key` may do (see
    /// [`Setup::opened_by`]), which works until `goal` is met if given,
    /// and gets `text` as its first line, and answers how it goes by. A
    /// sketch, as the setup's `sketch` makes it, works in a folder of its
    /// own; any other chat works where the chat that opened it does. Its
    /// agent runs with `model` as far as its session offers it, and it
    /// goes by `title`, if given, as it would by its first line, until its
    /// agent titles it. Where it works, `within` says: a sketch of the
    /// project of one of its directories, or a worktree Cox makes for it,
    /// marked as its own.
    #[expect(clippy::too_many_arguments, reason = "what a chat opens with")]
    fn open_chat(
        &mut self,
        key: u64,
        text: &str,
        title: Option<String>,
        setup: Setup,
        model: Option<&str>,
        goal: Option<Goal>,
        within: Within,
    ) -> Result<OpenedChat, String> {
        let slot = self.slot(key).ok_or("the chat is gone")?;
        let (mut setup, withheld) = setup.opened_by(slot.setup.as_ref());
        let folder = slot.setup.as_ref().and_then(|setup| setup.folder.clone());
        let project = slot.project;
        let project_folder = self.projects[project].folder.clone();
        let mut worktree = None;
        setup.folder = match (setup.sketch, within.worktree) {
            (Some(_), Some(_)) => {
                return Err("a subagent works in a sketch or in a worktree, not both".to_owned());
            }
            (Some(sketch), None) => {
                let from = folder.unwrap_or_else(|| project_folder.clone());
                let made = make_sketch(sketch, &from, within.directory.as_deref())?;
                setup.policy = sketch::allow(setup.policy, &made, &workflow::sketch().allow);
                Some(made)
            }
            (None, Some(asked)) => {
                if project == 0 {
                    return Err(
                        "this chat is in no project, so it has no directories to make a \
                         worktree of"
                            .to_owned(),
                    );
                }
                let made = worktrees::make(&project_folder, &asked, None)?;
                worktree = Some(made.clone());
                Some(made)
            }
            (None, None) => folder,
        };
        let mut opened = self.open_under(key, text, setup, model, goal)?;
        if let Some(made) = &worktree
            && let Some(kept) = self.slot(opened.number).and_then(|slot| slot.kept)
            && let Err(error) = worktrees::claim(made, kept.number())
        {
            tracing::error!("the worktree is not marked as the chat's: {error}");
        }
        let store = Rc::clone(&self.store);
        if let Some(title) = title
            && let Some(slot) = self.slot_mut(opened.number)
        {
            let title: String = title.chars().take(NAME_LENGTH).collect();
            slot.name_as(&title, Named::FirstLine, &store);
            opened.name = title;
        }
        Ok(OpenedChat { withheld, ..opened })
    }

    /// Opens a chat under the chat `key`, set up as `setup` says, on
    /// `text`, toward `goal`, its agent running with `model` if given,
    /// and answers how it goes by.
    fn open_under(
        &mut self,
        key: u64,
        text: &str,
        setup: Setup,
        model: Option<&str>,
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
            withheld: Withheld::default(),
        };
        if let Some(chat) = self.chat(opened) {
            if let Some(model) = model {
                chat.run_with(model);
            }
            chat.tell(&opened_by, text, goal);
        }
        Ok(told)
    }

    /// Switches the chat `key` to the agent kept as `id`, unless the chat
    /// is at work (see [`Slot::at_work`]): closes it, its agent stopped, keeps the agent as the chat's,
    /// and starts the agent at once, which goes on from the conversation
    /// so far, told it as a session that cannot be resumed is.
    fn switch_agent(&mut self, key: u64, id: &str) {
        let Some(slot) = self.slot(key) else {
            return;
        };
        let Some(kept) = slot.kept.filter(|_| !slot.at_work(Instant::now())) else {
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
            .map_or_default(|agents| agents.of(Some(chat.agent_id())).name);
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
        let _ = self.start_again(key);
    }

    /// Takes the chat `key` out of the list, to "Archived" at its foot, or
    /// brings it back. Its agent, if it runs, goes on, but the chats it
    /// opened are the human's from then on (see [`Self::tell_reports`]).
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

    /// Keeps and tells what changed: each chat's conversation, how each
    /// chat stands, and the list.
    fn settle(&mut self) {
        // A line written on a step while a turn ran goes once it ended.
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            if let Some(chat) = &mut slot.chat {
                chat.send_on_steps();
            }
        }
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
        row.pulls = self.pull_rows(&slot.pulls);
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
                    about: project.about.clone(),
                    last: project.last.filter(|&key| self.slot(key).is_some()),
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
            github: self.github.clone(),
            chosen: self.chosen,
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

    /// Follows how each chat stands now: a turn that ends in a chat of the
    /// human's while it is not shown leaves it unread, and what it is
    /// about names it, unless a name that lasts more has. A chat that comes
    /// to wait for the human, ends a turn of theirs, or whose agent changed
    /// the plan the human left it to, tells the window, for it to tell the
    /// human unless they look at it. What waits for a chat that is not at
    /// work hears how it stands, an agent hears the reports of the chats it
    /// opened that it did not wait for (see [`Self::tell_reports`]), and a
    /// sketch the human ends goes on to its end. Answers whether an agent
    /// was told or a sketch went on, which may change other chats.
    fn review(&mut self) -> bool {
        let mut news = Vec::new();
        let mut stands = Vec::new();
        let (mut ends, mut turns) = (Vec::new(), Vec::new());
        let store = Rc::clone(&self.store);
        let shown = self.shown;
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            let Some(chat) = &slot.chat else {
                continue;
            };
            let conversation = chat.conversation();
            let state = conversation.state();
            let turned = resolve::Turned::of(slot, conversation);
            let quiet = slot.resolved.is_some() || turned == Some(resolve::Turned::Merged);
            turns.extend(turned.map(|turned| (slot.key, turned)));
            let ended = working(slot.state) && !working(state) && slot.whose == Whose::Human;
            if let Whose::Agent { report, .. } = &mut slot.whose {
                let settled = chat.background_settled();
                (*report, slot.behind) = report.as_it_goes(slot.state, state, slot.behind, settled);
            }
            if ended && !quiet && shown != Some(slot.key) {
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
            (slot.state, slot.active) = (state, moved(slot.state, state, slot.active));
            slot.goal = conversation.goal().map(|goal| goal.what().clone());
            let about = match (conversation.title(), conversation.first_line()) {
                (Some(title), _) => Some((title.to_owned(), Named::Agent)),
                (None, Some(line)) => Some((line.to_owned(), Named::FirstLine)),
                (None, None) => None,
            };
            if !at_work(state) {
                stands.extend(slot.stands().map(|stands| (slot.key, stands)));
            }
            if let Some((name, named)) = about {
                let name: String = name.chars().take(NAME_LENGTH).collect();
                // The agent may title its session anew.
                let lasts = named > slot.named || (named == Named::Agent && slot.named == named);
                if lasts && name != slot.name {
                    slot.name_as(&name, named, &store);
                }
            }
            if let Some(body) = told.filter(|_| !quiet) {
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
            self.tell_news(message);
        }
        self.turned(turns);
        for (key, stands) in stands {
            let mut heard = false;
            for (_, answer) in self.waiting.extract_if(.., |(waited, _)| *waited == key) {
                heard |= answer.send(Ok(stands.clone())).is_ok();
            }
            if heard {
                self.heard(key);
            }
        }
        let told = self.tell_reports() | self.tell_ci();
        // A sketch the human ends is summed up once its turn ends, and
        // ends once that is done.
        let went_on = told || !ends.is_empty();
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
/// `from`, or of its project's directory `directory`, and answers it.
fn make_sketch(sketch: Sketch, from: &Path, directory: Option<&str>) -> Result<PathBuf, String> {
    let sketches =
        storage::path(storage::Kept::Sketches).ok_or("Cox cannot tell where to keep sketches")?;
    let from = match sketch {
        Sketch::Project => sketch::source(from, directory)?,
        Sketch::Empty => from.to_owned(),
    };
    sketch::make(sketch, &from, &sketches)
}

/// Whether a chat set up as `setup` merges what it made when the human
/// asks: one that works in the project, and not in a sketch's folder, as
/// a sketch, which ends its own ways, or a chat a sketch opened.
#[must_use]
pub fn merges(setup: Option<&Setup>) -> bool {
    setup.is_none_or(|setup| setup.folder.is_none() && setup.sketch.is_none())
}

/// Whether the human can ask the agent of a chat that stands as `state`,
/// set up as `setup`, to keep what it learned in the wiki: it is not at
/// work, and has the tool.
#[must_use]
pub fn saves_to_wiki(state: State, setup: Option<&Setup>) -> bool {
    !working(state) && keeps_in_wiki(setup)
}

/// The sketch a chat set up as `setup` is, what it started from and the
/// folder it works in, while the folder is there.
#[must_use]
pub fn sketch_of(setup: Option<&Setup>) -> Option<(Sketch, PathBuf)> {
    let setup = setup?;
    let folder = setup.folder.clone().filter(|folder| folder.exists())?;
    Some((setup.sketch?, folder))
}

/// Whether a chat set up as `setup` is a sketch that can end: its folder
/// is there, and was not opened as a project, as `is_project` says, which
/// the human goes on with, and it is not `ending` already.
#[must_use]
pub fn ends_sketch(
    setup: Option<&Setup>,
    ending: bool,
    is_project: impl FnOnce(&Path) -> bool,
) -> bool {
    !ending && sketch_of(setup).is_some_and(|(_, folder)| !is_project(&folder))
}

/// Whether an agent that stands as `state` is at work.
#[must_use]
pub fn working(state: State) -> bool {
    matches!(
        state,
        State::Busy | State::Asking | State::Stopping | State::Checking
    )
}

/// When a chat that stood as `was`, and last moved at `active`, last
/// moved, standing as `now`: a turn that starts or ends moves it now.
fn moved(was: State, now: State, active: i64) -> i64 {
    if working(was) == working(now) {
        active
    } else {
        cox_core::store::now()
    }
}

/// Whether a chat in `state` is at work, so that an agent waiting for it
/// waits on: not when it waits for the human.
#[must_use]
pub fn at_work(state: State) -> bool {
    matches!(state, State::Busy | State::Stopping | State::Checking)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cox_core::entry::ToolCallResult;

    use super::chat::GOES_ON_QUIET;
    use super::wire::Version;
    use super::*;
    use crate::testing::TempDir;

    /// No agents, for a daemon whose chats stay closed.
    struct NoAgents;

    impl Spawns for NoAgents {
        fn agents(&self) -> Agents {
            Agents::new(Vec::new(), Vec::new(), Vec::new())
        }

        fn subagents(&self) -> Subagents {
            Subagents::Agent
        }

        fn chat(
            &self,
            _: PathBuf,
            _: Start,
            _: &str,
            _: Option<u16>,
            _: bool,
            _: Subagents,
            _: bool,
        ) -> Spawned {
            unreachable!("no chat's agent starts")
        }

        fn starts(&self, _: &str) -> Starts {
            Rc::new(|_, _| unreachable!("no judge starts"))
        }
    }

    #[test]
    fn the_phone_reaches_the_chat_it_named_after_coxd_starts_again() {
        let store = Rc::new(Store::in_memory().unwrap());
        let gone = store.project(Path::new("/tmp/cox-gone"), "Gone").unwrap();
        store.add_chat(Some(gone), "Before").unwrap();
        let here = store.project(Path::new("/tmp/cox-here"), "Here").unwrap();
        let named = store.add_chat(Some(here), "Named").unwrap();
        let next = store.add_chat(Some(here), "Next").unwrap();
        let home = TempDir::new();
        let start = || {
            Daemon::new(
                home.to_path_buf(),
                false,
                Rc::clone(&store),
                Rc::new(NoAgents),
                unbounded_channel().0,
            )
        };
        let key_of = |daemon: &mut Daemon, kept: ChatId| {
            let (answer, key) = oneshot::channel();
            daemon.hear(Internal::Key {
                number: kept.number(),
                answer,
            });
            key.blocking_recv().unwrap()
        };

        let mut before = start();
        let held = key_of(&mut before, named).unwrap();
        drop(before);
        store.delete_project(gone).unwrap();
        let mut after = start();
        assert_eq!(
            key_of(&mut after, next),
            Some(held),
            "the key the phone held names another chat now"
        );

        let key = key_of(&mut after, named).unwrap();
        after.act(ToDaemon::SetGoal {
            key,
            goal: Some(Goal::Words("Meant for Named".to_owned())),
        });
        let goals: Vec<_> = store
            .chats()
            .unwrap()
            .into_iter()
            .map(|chat| (chat.name, chat.goal))
            .collect();
        assert_eq!(
            goals,
            [
                (
                    "Named".to_owned(),
                    Some(Goal::Words("Meant for Named".to_owned()))
                ),
                ("Next".to_owned(), None),
            ]
        );
        let none = store.add_chat(Some(here), "Not listed").unwrap();
        assert_eq!(key_of(&mut after, none), None);
    }

    /// A daemon of `store`, whose home is `home` and whose chats stay
    /// closed, and what it tells the window.
    fn daemon_of(store: &Rc<Store>, home: &Path) -> (Daemon, UnboundedReceiver<FromDaemon>) {
        let mut daemon = Daemon::new(
            home.to_path_buf(),
            false,
            Rc::clone(store),
            Rc::new(NoAgents),
            unbounded_channel().0,
        );
        let (window, told) = unbounded_channel();
        daemon.window = Some(window);
        (daemon, told)
    }

    /// The key the daemon lists the chat kept as `kept` by.
    fn key_of(daemon: &mut Daemon, kept: ChatId) -> u64 {
        let (answer, key) = oneshot::channel();
        daemon.hear(Internal::Key {
            number: kept.number(),
            answer,
        });
        key.blocking_recv().unwrap().unwrap()
    }

    /// A store with a project in `folder` and one chat of it, `Chat`.
    fn one_chat(folder: &Path) -> (Rc<Store>, ChatId) {
        let store = Rc::new(Store::in_memory().unwrap());
        let project = store.project(folder, "Project").unwrap();
        let chat = store.add_chat(Some(project), "Chat").unwrap();
        (store, chat)
    }

    fn kept_chat(store: &Store, chat: ChatId) -> cox_core::store::KeptChat {
        store
            .chats()
            .unwrap()
            .into_iter()
            .find(|kept| kept.id == chat)
            .unwrap()
    }

    #[test]
    fn a_chat_the_human_renames_is_kept_by_that_name() {
        let home = TempDir::new();
        let (store, chat) = one_chat(&home);
        let (mut daemon, _told) = daemon_of(&store, &home);
        let key = key_of(&mut daemon, chat);

        daemon.act(ToDaemon::Rename {
            key,
            name: "Renamed".to_owned(),
        });

        assert_eq!(kept_chat(&store, chat).name, "Renamed");
    }

    #[test]
    fn a_chat_put_away_is_kept_archived() {
        let home = TempDir::new();
        let (store, chat) = one_chat(&home);
        let (mut daemon, _told) = daemon_of(&store, &home);
        let key = key_of(&mut daemon, chat);

        daemon.act(ToDaemon::Archive {
            key,
            archived: true,
        });

        assert!(kept_chat(&store, chat).archived);
    }

    #[test]
    fn a_chat_brought_back_from_the_archive_is_kept_listed() {
        let home = TempDir::new();
        let (store, chat) = one_chat(&home);
        let (mut daemon, _told) = daemon_of(&store, &home);
        let key = key_of(&mut daemon, chat);
        daemon.act(ToDaemon::Archive {
            key,
            archived: true,
        });

        daemon.act(ToDaemon::Archive {
            key,
            archived: false,
        });

        assert!(!kept_chat(&store, chat).archived);
    }

    #[test]
    fn no_chat_opens_in_a_project_whose_folder_is_gone() {
        let home = TempDir::new();
        let gone = home.join("gone");
        let (store, _) = one_chat(&gone);
        let (mut daemon, _told) = daemon_of(&store, &home);

        daemon.act(ToDaemon::NewChat { folder: Some(gone) });

        assert_eq!(store.chats().unwrap().len(), 1, "no chat is kept");
    }

    /// The projects kept, by id, each with how many chats of it are kept.
    fn kept_projects(store: &Store) -> Vec<(ProjectId, usize)> {
        let chats = store.chats().unwrap();
        store
            .projects()
            .unwrap()
            .into_iter()
            .map(|project| {
                let of = |chat: &&cox_core::store::KeptChat| chat.project == Some(project.id);
                (project.id, chats.iter().filter(of).count())
            })
            .collect()
    }

    #[test]
    fn a_folder_opened_with_a_slash_after_it_is_of_its_project() {
        let home = TempDir::new();
        storage::keep_on_this_thread(Some(home.join("data")));
        let folder = home.join("shop");
        std::fs::create_dir(&folder).unwrap();
        let (store, _) = one_chat(&folder);
        let (mut daemon, _told) = daemon_of(&store, &home);
        let shop = store.projects().unwrap()[0].id;

        daemon.act(ToDaemon::Open {
            folder: Some(home.join("shop/")),
        });
        daemon.act(ToDaemon::NewChat {
            folder: Some(home.join("shop/")),
        });

        assert_eq!(kept_projects(&store), [(shop, 3)]);
    }

    #[test]
    fn a_chat_opens_in_a_folder_again_in_a_new_project_once_its_project_was_deleted() {
        let home = TempDir::new();
        storage::keep_on_this_thread(Some(home.join("data")));
        crate::trash::keep_on_this_thread(Some(home.join("Trash")));
        let folder = home.join("shop");
        std::fs::create_dir(&folder).unwrap();
        let (store, _) = one_chat(&folder);
        let (mut daemon, mut told) = daemon_of(&store, &home);
        let deleted = store.projects().unwrap()[0].id;
        daemon.act(ToDaemon::DeleteProject { project: deleted });
        std::fs::create_dir(&folder).unwrap();

        daemon.act(ToDaemon::Open {
            folder: Some(folder.clone()),
        });
        daemon.act(ToDaemon::NewChat {
            folder: Some(folder),
        });
        daemon.settle();

        let mut listing = None;
        while let Ok(message) = told.try_recv() {
            if let FromDaemon::Listing(now) = message {
                listing = Some(now);
            }
        }
        let listing = listing.expect("the window is told the list");
        let projects: Vec<usize> = listing.chats.iter().map(|chat| chat.project).collect();
        assert_eq!(projects.len(), 2, "two chats are listed");
        for project in projects {
            assert!(!listing.projects[project].deleted, "{:?}", listing.projects);
        }
    }

    #[test]
    fn the_window_is_told_the_tabs_a_folders_stage_had_as_they_were_kept() {
        let home = TempDir::new();
        let (store, chat) = one_chat(&home);
        let (mut daemon, mut told) = daemon_of(&store, &home);
        let key = key_of(&mut daemon, chat);
        let tabs = KeptTabs {
            tabs: vec![KeptTab::Terminal {
                title: "zsh".to_owned(),
            }],
            shown: 0,
            owners: vec![Some(key)],
        };
        daemon.act(ToDaemon::Tabs {
            folder: home.to_path_buf(),
            tabs: tabs.clone(),
        });

        daemon.act(ToDaemon::StageOf {
            folder: home.to_path_buf(),
        });

        match told.try_recv() {
            Ok(FromDaemon::StageOf {
                folder,
                tabs: Some(kept),
            }) => {
                assert_eq!(folder, home.path());
                assert_eq!(kept, tabs);
            }
            _ => panic!("the window is not told the stage's tabs"),
        }
    }

    #[test]
    fn only_a_chat_that_works_in_its_project_merges_what_it_made() {
        let in_folder = Setup {
            folder: Some(PathBuf::from("/elsewhere")),
            ..Setup::default()
        };
        let in_sketch = Setup {
            sketch: Some(Sketch::Empty),
            ..Setup::default()
        };

        assert!(merges(None));
        assert!(merges(Some(&Setup::default())));
        assert!(!merges(Some(&in_folder)));
        assert!(!merges(Some(&in_sketch)));
    }

    #[test]
    fn a_sketch_ends_while_its_folder_is_there_and_no_projects() {
        let home = TempDir::new();
        let sketch = Setup {
            folder: Some(home.to_path_buf()),
            sketch: Some(Sketch::Empty),
            ..Setup::default()
        };
        let gone = Setup {
            folder: Some(home.join("gone")),
            ..sketch.clone()
        };
        let opened_by_a_sketch = Setup {
            sketch: None,
            ..sketch.clone()
        };
        let no_project = |_: &Path| false;

        assert!(ends_sketch(Some(&sketch), false, no_project));
        assert!(!ends_sketch(Some(&sketch), true, no_project));
        assert!(!ends_sketch(Some(&sketch), false, |folder| folder == &*home));
        assert!(!ends_sketch(Some(&gone), false, no_project));
        assert!(!ends_sketch(Some(&opened_by_a_sketch), false, no_project));
        assert!(!ends_sketch(None, false, no_project));
    }

    #[test]
    fn a_chat_saves_to_the_wiki_while_it_waits_with_the_tool() {
        let without = Setup {
            tools: Some(vec!["show_file".to_owned()]),
            ..Setup::default()
        };

        assert!(saves_to_wiki(State::Idle, None));
        assert!(!saves_to_wiki(State::Busy, None));
        assert!(!saves_to_wiki(State::Idle, Some(&without)));
    }

    #[test]
    fn a_chat_works_while_a_turn_runs_asks_stops_or_checks() {
        let working: Vec<_> = [
            State::Idle,
            State::Busy,
            State::Asking,
            State::Stopping,
            State::Checking,
            State::Closed,
        ]
        .into_iter()
        .filter(|&state| working(state))
        .collect();

        assert_eq!(
            working,
            [State::Busy, State::Asking, State::Stopping, State::Checking]
        );
    }

    #[test]
    fn a_chat_asking_the_human_is_not_at_work() {
        let at_work: Vec<_> = [
            State::Idle,
            State::Busy,
            State::Asking,
            State::Stopping,
            State::Checking,
            State::Closed,
        ]
        .into_iter()
        .filter(|&state| at_work(state))
        .collect();

        assert_eq!(at_work, [State::Busy, State::Stopping, State::Checking]);
    }

    /// The ends of a chat's agent the test plays: what it is sent, kept
    /// for the chat to send to, and where it tells what it does.
    struct Played {
        commands: UnboundedReceiver<crate::agent::Command>,
        events: UnboundedSender<AgentEvent>,
        /// The session it was started to go on with, if any.
        resumes: Option<String>,
        /// Where it was started to work.
        cwd: PathBuf,
    }

    /// Agents the test plays, each kept as it starts.
    #[derive(Default)]
    struct PlayedAgents(std::cell::RefCell<Vec<Played>>);

    impl Spawns for PlayedAgents {
        fn agents(&self) -> Agents {
            Agents::new(vec!["played".to_owned()], Vec::new(), Vec::new())
        }

        fn subagents(&self) -> Subagents {
            Subagents::Agent
        }

        fn chat(
            &self,
            cwd: PathBuf,
            start: Start,
            _: &str,
            _: Option<u16>,
            _: bool,
            _: Subagents,
            _: bool,
        ) -> Spawned {
            let (commands, commands_got) = unbounded_channel();
            let (events_sent, events) = unbounded_channel();
            let resumes = match start {
                Start::Again { session, .. } => session,
                Start::New { .. } => None,
            };
            self.0.borrow_mut().push(Played {
                commands: commands_got,
                events: events_sent,
                resumes,
                cwd: cwd.clone(),
            });
            Spawned {
                agent: Agent {
                    commands,
                    events,
                    instructions: Vec::new(),
                    setup: None,
                    cwd,
                    tools: None,
                    chat_requests: None,
                    thread: crate::agent::AgentThread::none(),
                },
                stage: None,
            }
        }

        fn starts(&self, _: &str) -> Starts {
            Rc::new(|_, _| unreachable!("no judge starts"))
        }
    }

    /// A daemon whose chats' agents `agents` plays, run as coxd runs it,
    /// with no window yet, and what its chats' tasks tell it.
    fn played_daemon(
        home: &Path,
        agents: &Rc<PlayedAgents>,
    ) -> (Daemon, UnboundedReceiver<Internal>) {
        let (internal, heard) = unbounded_channel();
        let spawns: Rc<dyn Spawns> = agents.clone();
        let daemon = Daemon::new(
            home.to_path_buf(),
            false,
            Rc::new(Store::in_memory().unwrap()),
            spawns,
            internal,
        );
        (daemon, heard)
    }

    /// Has the daemon hear what its chats' tasks told it by now, and
    /// settle, as its loop does.
    async fn pump(daemon: &mut Daemon, heard: &mut UnboundedReceiver<Internal>) {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        while let Ok(event) = heard.try_recv() {
            daemon.hear(event);
        }
        daemon.settle();
    }

    /// A window of `version` comes as the connection `id`, `later` from
    /// now, and is answered.
    fn window_comes(daemon: &mut Daemon, id: u64, version: Version, later: Duration) -> FromDaemon {
        let hello = Hello {
            version: Some(version),
            ..Hello::default()
        };
        let (window, _told) = unbounded_channel();
        daemon.attach(id, hello, window, Instant::now() + later)
    }

    fn on_daemon(test: impl Future<Output = ()>) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, test);
    }

    /// A chat whose agent works, as `works` has it do, in a daemon whose
    /// window then went, and the answers a window of another build that
    /// speaks otherwise, and one that speaks alike, get, `later` from then.
    fn answers_once_the_window_went(
        later: Duration,
        works: impl FnOnce(&mut Daemon, u64, &Played),
    ) -> [FromDaemon; 2] {
        let mut answers = None;
        on_daemon(async {
            let home = TempDir::new();
            storage::keep_on_this_thread(Some(home.join("data")));
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, mut heard) = played_daemon(&home, &agents);
            daemon.act(ToDaemon::NewChat { folder: None });
            pump(&mut daemon, &mut heard).await;
            let key = daemon.chats.last().unwrap().key;
            assert_eq!(agents.0.borrow().len(), 1, "the chat's agent started");
            assert_eq!(
                window_comes(&mut daemon, 1, wire::VERSION, Duration::ZERO),
                FromDaemon::Joined(wire::VERSION)
            );

            works(&mut daemon, key, &agents.0.borrow()[0]);
            pump(&mut daemon, &mut heard).await;
            daemon.hear(Internal::Detach { id: 1 });
            pump(&mut daemon, &mut heard).await;

            let unlike = Version {
                protocol: wire::VERSION.protocol + 1,
                ..wire::VERSION
            };
            let alike = Version {
                daemon: wire::VERSION.daemon + 1,
                ..wire::VERSION
            };
            let unlike = window_comes(&mut daemon, 2, unlike, later);
            let alike = window_comes(&mut daemon, 3, alike, later);
            answers = Some([unlike, alike]);
        });
        answers.unwrap()
    }

    #[test]
    fn a_window_of_another_build_waits_for_a_turn_that_runs_after_the_window_went() {
        let answers = answers_once_the_window_went(Duration::ZERO, |daemon, key, _| {
            daemon.act(ToDaemon::Submit {
                key,
                text: "work".to_owned(),
                attached: Vec::new(),
                step: None,
            });
        });

        assert_eq!(
            answers,
            [FromDaemon::Busy, FromDaemon::Joined(wire::VERSION)]
        );
    }

    #[test]
    fn a_window_that_reads_it_hears_which_chats_keep_coxd_at_work() {
        on_daemon(async {
            let home = TempDir::new();
            storage::keep_on_this_thread(Some(home.join("data")));
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, mut heard) = played_daemon(&home, &agents);
            daemon.act(ToDaemon::NewChat { folder: None });
            daemon.act(ToDaemon::NewChat { folder: None });
            pump(&mut daemon, &mut heard).await;
            let key = daemon.chats.last().unwrap().key;
            submit(&mut daemon, key);
            pump(&mut daemon, &mut heard).await;
            let name = daemon.slot(key).unwrap().name.clone();

            let unlike = Version {
                protocol: wire::VERSION.protocol + 1,
                ..wire::VERSION
            };
            let hello = Hello {
                version: Some(unlike),
                reads_at_work: true,
                ..Hello::default()
            };
            let (window, _told) = unbounded_channel();
            let answer = daemon.attach(1, hello, window, Instant::now());

            assert_eq!(
                answer,
                FromDaemon::AtWork(vec![AtWork {
                    project: None,
                    chat: name,
                }]),
                "the chat at work, of Cox's own, and not the one idle"
            );
        });
    }

    #[test]
    fn a_window_of_another_build_waits_for_an_agent_that_goes_on_by_itself() {
        let answers = answers_once_the_window_went(Duration::ZERO, |daemon, key, agent| {
            daemon.act(ToDaemon::Submit {
                key,
                text: "work".to_owned(),
                attached: Vec::new(),
                step: None,
            });
            agent
                .events
                .send(AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn))
                .unwrap();
            agent.events.send(AgentEvent::WentOn).unwrap();
            agent
                .events
                .send(AgentEvent::ReplyText("The tests passed.".to_owned()))
                .unwrap();
        });

        assert_eq!(
            answers,
            [FromDaemon::Busy, FromDaemon::Joined(wire::VERSION)]
        );
    }

    /// Has the agent `agent` tell `events`, as it sends them.
    fn tells(agent: &Played, events: impl IntoIterator<Item = AgentEvent>) {
        for event in events {
            agent.events.send(event).unwrap();
        }
    }

    /// Has the human send the chat `key` a line, which starts a turn.
    fn submit(daemon: &mut Daemon, key: u64) {
        daemon.act(ToDaemon::Submit {
            key,
            text: "work".to_owned(),
            attached: Vec::new(),
            step: None,
        });
    }

    /// A tool call the agent made, `id`, started, or ended as `result`.
    fn tool_call(id: &str, result: Option<ToolCallResult>) -> AgentEvent {
        AgentEvent::ToolCall {
            id: id.to_owned(),
            title: Some("gh pr checks --watch".to_owned()),
            command: false,
            content: None,
            result,
        }
    }

    /// Longer than an agent going on by itself may be quiet.
    const QUIET: Duration = GOES_ON_QUIET.saturating_add(Duration::from_secs(1));

    #[test]
    fn a_window_of_another_build_waits_for_a_tool_call_an_agent_goes_on_with_however_quiet() {
        let answers = answers_once_the_window_went(QUIET, |daemon, key, agent| {
            submit(daemon, key);
            tells(
                agent,
                [
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
                    AgentEvent::WentOn,
                    tool_call("watch", None),
                ],
            );
        });

        assert_eq!(
            answers,
            [FromDaemon::Busy, FromDaemon::Joined(wire::VERSION)]
        );
    }

    #[test]
    fn a_window_of_another_build_waits_for_a_task_that_runs_after_the_turn_ended() {
        let answers = answers_once_the_window_went(QUIET, |daemon, key, agent| {
            submit(daemon, key);
            tells(
                agent,
                [
                    AgentEvent::Task(cox_acp::Task {
                        id: "ci".to_owned(),
                        running: true,
                        started: true,
                        ..cox_acp::Task::default()
                    }),
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
                ],
            );
        });

        assert_eq!(
            answers,
            [FromDaemon::Busy, FromDaemon::Joined(wire::VERSION)]
        );
    }

    #[test]
    fn a_window_of_another_build_moves_coxd_on_once_the_tool_call_and_the_task_ended() {
        let answers = answers_once_the_window_went(QUIET, |daemon, key, agent| {
            submit(daemon, key);
            let task = |running| {
                AgentEvent::Task(cox_acp::Task {
                    id: "ci".to_owned(),
                    running,
                    started: running,
                    ..cox_acp::Task::default()
                })
            };
            tells(
                agent,
                [
                    task(true),
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
                    AgentEvent::WentOn,
                    tool_call("watch", None),
                    tool_call("watch", Some(ToolCallResult::Completed)),
                    task(false),
                ],
            );
        });

        assert_eq!(
            answers,
            [FromDaemon::Outdated, FromDaemon::Joined(wire::VERSION)],
            "a window that speaks alike joins coxd, at work or not"
        );
    }

    /// A chat the human works in, with no turn running, whose agent ran
    /// the subagents `titles` in the background, each at work on its first
    /// line, in a daemon whose agents the test plays: the chat's agent
    /// first, then the subagents'.
    struct Opener {
        daemon: Daemon,
        heard: UnboundedReceiver<Internal>,
        agents: Rc<PlayedAgents>,
        key: u64,
        subagents: Vec<u64>,
        _home: TempDir,
    }

    impl Opener {
        async fn new(titles: &[&str]) -> Self {
            let home = TempDir::new();
            storage::keep_on_this_thread(Some(home.join("data")));
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, mut heard) = played_daemon(&home, &agents);
            daemon.act(ToDaemon::NewChat { folder: None });
            pump(&mut daemon, &mut heard).await;
            let key = daemon.chats.last().unwrap().key;
            let mut subagents = Vec::new();
            for title in titles {
                let (answer, mut opened) = oneshot::channel();
                daemon.asked(
                    key,
                    ChatRequest::OpenChat {
                        text: "look".to_owned(),
                        title: Some((*title).to_owned()),
                        setup: Setup::default(),
                        model: None,
                        goal: None,
                        sketch: None,
                        within: Box::default(),
                        answer,
                    },
                );
                subagents.push(opened.try_recv().unwrap().unwrap().number);
            }
            let mut opener = Self {
                daemon,
                heard,
                agents,
                key,
                subagents,
                _home: home,
            };
            opener.pump().await;
            opener
        }

        async fn pump(&mut self) {
            pump(&mut self.daemon, &mut self.heard).await;
        }

        /// The agent of the chat, or of its subagent `index`.
        fn agent(&self, index: Option<usize>) -> std::cell::Ref<'_, Played> {
            std::cell::Ref::map(self.agents.0.borrow(), |agents| {
                &agents[index.map_or(0, |index| index + 1)]
            })
        }

        /// Has the subagent `index` end its turn, saying `said`.
        fn ends(&self, index: usize, said: &str) {
            tells(
                &self.agent(Some(index)),
                [
                    AgentEvent::ReplyText(said.to_owned()),
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
                ],
            );
        }

        /// The lines the chat's agent was sent since last asked.
        fn prompts(&self) -> Vec<String> {
            let mut agents = self.agents.0.borrow_mut();
            let mut prompts = Vec::new();
            while let Ok(command) = agents[0].commands.try_recv() {
                if let crate::agent::Command::Prompt(prompt) = command {
                    prompts.push(prompt.text);
                }
            }
            prompts
        }

        /// Asks how the subagent `index` stands, as `agent_report` does.
        fn wait(&mut self, index: usize) -> oneshot::Receiver<Result<Waited, String>> {
            let (answer, waited) = oneshot::channel();
            self.daemon
                .wait_chat(self.key, self.subagents[index], answer);
            waited
        }
    }

    /// What the chat's agent is told of a subagent, "Look", that ended its
    /// turn saying "Found it.", per the requirement.
    const FOUND_IT: &str = "The subagent \"Look\" ended its turn. It last said:\n\nFound it.";

    /// A daemon whose agents the test plays, with a chat of the project in
    /// `home/shop`, whose directory `app` is a repository with a commit,
    /// and the chat's key.
    async fn shop_with_app(
        home: &TempDir,
        agents: &Rc<PlayedAgents>,
    ) -> (Daemon, UnboundedReceiver<Internal>, u64) {
        storage::keep_on_this_thread(Some(home.join("data")));
        let shop = home.join("shop");
        let app = shop.join(crate::settings::DIRECTORIES).join("app");
        std::fs::create_dir_all(&app).unwrap();
        crate::testing::run_git(&app, &["init", "--quiet", "-b", "main"]).unwrap();
        crate::testing::run_git(&app, &["commit", "--quiet", "--allow-empty", "-m", "first"])
            .unwrap();
        let store = Rc::new(Store::in_memory().unwrap());
        store.project(&shop, "shop").unwrap();
        let (internal, mut heard) = unbounded_channel();
        let spawns: Rc<dyn Spawns> = agents.clone();
        let mut daemon = Daemon::new(home.to_path_buf(), false, store, spawns, internal);
        daemon.act(ToDaemon::NewChat { folder: Some(shop) });
        pump(&mut daemon, &mut heard).await;
        let key = daemon.chats.last().unwrap().key;
        (daemon, heard, key)
    }

    #[test]
    fn a_subagent_given_a_worktree_works_in_one_cox_makes_marked_as_its_own() {
        on_daemon(async {
            let home = TempDir::new();
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, _heard, key) = shop_with_app(&home, &agents).await;
            let (answer, mut opened) = oneshot::channel();
            daemon.asked(
                key,
                ChatRequest::OpenChat {
                    text: "fix it".to_owned(),
                    title: Some("Fix".to_owned()),
                    setup: Setup::default(),
                    model: None,
                    goal: None,
                    sketch: None,
                    within: Box::new(Within {
                        directory: None,
                        worktree: Some(worktrees::Asked {
                            directory: "app".to_owned(),
                            branch: Some("fix".to_owned()),
                            from: None,
                        }),
                    }),
                    answer,
                },
            );
            let number = opened.try_recv().unwrap().unwrap().number;

            let shop = home.join("shop");
            let made = shop.join(worktrees::WORKTREES).join("app").join("fix");
            let slot = daemon.slot(number).unwrap();
            assert_eq!(
                slot.setup.as_ref().and_then(|setup| setup.folder.clone()),
                Some(made.clone())
            );
            let kept = slot.kept.unwrap().number();
            let listed = worktrees::of(&shop.join(crate::settings::DIRECTORIES).join("app"), &shop);
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].made_for, Some(kept));
        });
    }

    #[test]
    fn the_worktree_made_for_a_subagent_goes_once_it_is_done_and_it_is_told() {
        on_daemon(async {
            let home = TempDir::new();
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, mut heard, key) = shop_with_app(&home, &agents).await;
            let (answer, mut opened) = oneshot::channel();
            daemon.asked(
                key,
                ChatRequest::OpenChat {
                    text: "look".to_owned(),
                    title: Some("Look".to_owned()),
                    setup: Setup::default(),
                    model: None,
                    goal: None,
                    sketch: None,
                    within: Box::new(Within {
                        directory: None,
                        worktree: Some(worktrees::Asked {
                            directory: "app".to_owned(),
                            branch: Some("look".to_owned()),
                            from: None,
                        }),
                    }),
                    answer,
                },
            );
            let number = opened.try_recv().unwrap().unwrap().number;
            let made = home
                .join("shop")
                .join(worktrees::WORKTREES)
                .join("app")
                .join("look");
            assert!(made.exists());

            daemon.resolve(number, ResolvedBy::Human);
            // Cox looks on a thread of its own, and says what it took away.
            let tidied = tokio::time::timeout(crate::testing::waits::GIVEN_UP, async {
                loop {
                    match heard.recv().await {
                        Some(event @ Internal::Tidied { .. }) => break event,
                        Some(event) => daemon.hear(event),
                        None => panic!("the daemon's events ended"),
                    }
                }
            })
            .await
            .expect("Cox looked which worktrees it may take away");
            daemon.hear(tidied);
            assert!(!daemon.tidying_now());

            assert!(!made.exists(), "the worktree is taken away");
            let notes: Vec<String> = daemon
                .chat(number)
                .unwrap()
                .conversation()
                .entries()
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Note(note) => Some(note.clone()),
                    _ => None,
                })
                .collect();
            assert!(
                notes
                    .iter()
                    .any(|note| note.starts_with("Took away the worktree at ")
                        && note.ends_with("held nothing that is not elsewhere.")),
                "{notes:?}"
            );
        });
    }

    /// Opens, in the chat `key`, a subagent "Look" that works in a
    /// worktree of `app` on the branch `look`, has its turn end, and has
    /// it marked done, so that Cox takes its worktree away; answers its
    /// key and the worktree's folder, gone.
    async fn looked_in_a_worktree_taken_away(
        daemon: &mut Daemon,
        heard: &mut UnboundedReceiver<Internal>,
        agents: &PlayedAgents,
        home: &Path,
        key: u64,
    ) -> (u64, PathBuf) {
        let (answer, mut opened) = oneshot::channel();
        daemon.asked(
            key,
            ChatRequest::OpenChat {
                text: "look".to_owned(),
                title: Some("Look".to_owned()),
                setup: Setup::default(),
                model: None,
                goal: None,
                sketch: None,
                within: Box::new(Within {
                    directory: None,
                    worktree: Some(worktrees::Asked {
                        directory: "app".to_owned(),
                        branch: Some("look".to_owned()),
                        from: None,
                    }),
                }),
                answer,
            },
        );
        let number = opened.try_recv().unwrap().unwrap().number;
        pump(daemon, heard).await;
        let made = home
            .join("shop")
            .join(worktrees::WORKTREES)
            .join("app")
            .join("look");
        tells(
            agents.0.borrow().last().unwrap(),
            [
                AgentEvent::ReplyText("Found it.".to_owned()),
                AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
            ],
        );
        pump(daemon, heard).await;
        daemon.resolve(number, ResolvedBy::Human);
        tokio::time::timeout(crate::testing::waits::GIVEN_UP, async {
            while made.exists() || daemon.tidying_now() {
                match heard.recv().await {
                    Some(event) => daemon.hear(event),
                    None => panic!("the daemon's events ended"),
                }
            }
        })
        .await
        .expect("Cox took the worktree away");
        (number, made)
    }

    /// The notes the chat `key` shows.
    fn notes_of(daemon: &mut Daemon, key: u64) -> Vec<String> {
        daemon
            .chat(key)
            .unwrap()
            .conversation()
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::Note(note) => Some(note.clone()),
                _ => None,
            })
            .collect()
    }

    /// Has the agent of the chat `key` continue its subagent "Look" with
    /// `text`, as `agent_continue` does.
    fn continue_look(daemon: &mut Daemon, key: u64, text: &str) -> Result<(u64, String), String> {
        let (answer, mut went_on) = oneshot::channel();
        daemon.asked(
            key,
            ChatRequest::ContinueChat {
                subagent: Subagent::Title("Look".to_owned()),
                text: text.to_owned(),
                answer,
            },
        );
        went_on.try_recv().unwrap()
    }

    #[test]
    fn a_subagent_continued_once_its_worktree_was_taken_away_has_it_made_again_first() {
        on_daemon(async {
            let home = TempDir::new();
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, mut heard, key) = shop_with_app(&home, &agents).await;
            let (number, made) =
                looked_in_a_worktree_taken_away(&mut daemon, &mut heard, &agents, &home, key).await;
            let kept = daemon.slot(number).unwrap().kept.unwrap().number();

            assert_eq!(
                continue_look(&mut daemon, key, "Go on."),
                Ok((number, "Look".to_owned()))
            );

            assert!(made.exists(), "the worktree is made again");
            let app = home
                .join("shop")
                .join(crate::settings::DIRECTORIES)
                .join("app");
            let listed = worktrees::of(&app, &home.join("shop"));
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].branch.as_deref(), Some("look"));
            assert_eq!(listed[0].made_for, Some(kept), "Cox's again");
            assert_eq!(agents.0.borrow().last().unwrap().cwd, made);
            let notes = notes_of(&mut daemon, number);
            assert!(
                notes.iter().any(|note| note.starts_with(&format!(
                    "Cox made the worktree this chat works in again, at {}, on its branch look, \
                     made again from its last commit ",
                    made.display()
                ))),
                "{notes:?}"
            );
        });
    }

    #[test]
    fn a_chat_whose_worktree_was_taken_away_starts_again_there_once_coxd_did() {
        on_daemon(async {
            let home = TempDir::new();
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, mut heard, key) = shop_with_app(&home, &agents).await;
            let (number, made) =
                looked_in_a_worktree_taken_away(&mut daemon, &mut heard, &agents, &home, key).await;
            let kept = daemon.slot(number).unwrap().kept;
            let store = Rc::clone(&daemon.store);
            drop(daemon);

            let (internal, _heard) = unbounded_channel();
            let spawns: Rc<dyn Spawns> = agents.clone();
            let mut after = Daemon::new(
                home.to_path_buf(),
                false,
                Rc::clone(&store),
                spawns,
                internal,
            );
            let look = after
                .chats
                .iter()
                .find(|slot| slot.kept == kept)
                .unwrap()
                .key;
            let row = after
                .slot(look)
                .unwrap()
                .row(&after.spawns.agents(), &store);
            assert!(row.again, "it starts again, though its folder is gone");
            submit(&mut after, look);

            assert!(made.exists(), "the worktree is made again");
            assert_eq!(agents.0.borrow().last().unwrap().cwd, made);
            assert_eq!(
                after.chat(look).unwrap().conversation().state(),
                State::Busy
            );
        });
    }

    #[test]
    fn a_subagent_whose_worktree_cannot_be_made_again_does_not_start_and_says_why() {
        on_daemon(async {
            let home = TempDir::new();
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, mut heard, key) = shop_with_app(&home, &agents).await;
            let (number, made) =
                looked_in_a_worktree_taken_away(&mut daemon, &mut heard, &agents, &home, key).await;
            let app = home
                .join("shop")
                .join(crate::settings::DIRECTORIES)
                .join("app");
            std::fs::remove_dir_all(&app).unwrap();
            let started = agents.0.borrow().len();

            let refused = continue_look(&mut daemon, key, "Go on.").unwrap_err();

            assert_eq!(
                refused,
                "The subagent \"Look\" cannot start again: the worktree it works in is gone, and \
                 Cox cannot make it again: the project has no repository app in its directories."
            );
            assert!(!made.exists());
            assert_eq!(agents.0.borrow().len(), started, "no agent started");
            let notes = notes_of(&mut daemon, number);
            assert!(
                notes.iter().any(|note| note
                    == &format!(
                        "The worktree this chat works in, at {}, is gone, and Cox cannot make it \
                         again, so its agent does not start: the project has no repository app \
                         in its directories.",
                        made.display()
                    )),
                "{notes:?}"
            );
        });
    }

    #[test]
    fn a_sketch_of_a_project_whose_folder_is_no_repository_is_of_its_one_repository() {
        on_daemon(async {
            let home = TempDir::new();
            let agents = Rc::new(PlayedAgents::default());
            let (mut daemon, _heard, key) = shop_with_app(&home, &agents).await;

            let opened = daemon.start_sketch(key, Sketch::Project, None, "try");

            let folder = opened.unwrap().folder.unwrap();
            assert!(
                crate::testing::run_git(&folder, &["rev-parse", "HEAD"]).is_ok(),
                "{}",
                folder.display()
            );
        });
    }

    #[test]
    fn an_agent_finds_a_subagent_by_its_title_among_its_own_or_by_its_number() {
        on_daemon(async {
            let opener = Opener::new(&["Look", "Fix", "Fix"]).await;
            let [look, fix, other_fix] = opener.subagents[..] else {
                panic!("{:?}", opener.subagents);
            };
            let find = |asking, subagent| opener.daemon.find_subagent(asking, &subagent);
            let title = |title: &str| Subagent::Title(title.to_owned());

            assert_eq!(
                find(opener.key, title("Look")),
                Ok((look, "Look".to_owned()))
            );
            assert_eq!(
                find(opener.key, Subagent::Number(fix)),
                Ok((fix, "Fix".to_owned())),
                "the number still names it"
            );
            assert_eq!(
                find(opener.key, title("Fix")),
                Err(format!(
                    "Several subagents of this chat are titled \"Fix\", with the numbers {fix} \
                     and {other_fix}: give agent_report the number of the one you mean as `chat`."
                ))
            );
            assert_eq!(
                find(opener.key, title("Build")),
                Err(
                    "No subagent of this chat is titled \"Build\". Its subagents go by \"Look\", \
                     \"Fix\", \"Fix\" now: give agent_report one of those, and name it so to the \
                     human."
                        .to_owned()
                )
            );
            assert_eq!(
                find(look, title("Fix")),
                Err(
                    "No subagent of this chat is titled \"Fix\": this chat opened none.".to_owned()
                ),
                "a title names only a chat's own subagents"
            );
        });
    }

    #[test]
    fn a_subagent_that_ends_while_its_opener_waits_starts_a_turn_of_the_opener_on_its_report() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            opener.prompts();

            opener.ends(0, "Found it.");
            opener.pump().await;

            let prompts = opener.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(prompts[0].starts_with(FOUND_IT), "{prompts:?}");
            assert!(prompts[0].contains("agent_report"), "{prompts:?}");
            assert_eq!(
                opener
                    .daemon
                    .chat(opener.key)
                    .unwrap()
                    .conversation()
                    .state(),
                State::Busy
            );
            // agent_report still answers it, and nothing is told twice.
            let waited = opener.wait(0).await.unwrap().unwrap();
            assert_eq!(waited.said.as_deref(), Some("Found it."));
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;
            assert_eq!(opener.prompts(), Vec::<String>::new());
        });
    }

    #[test]
    fn an_opener_at_work_hears_a_subagent_s_report_once_its_turn_ends() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            submit(&mut opener.daemon, opener.key);
            opener.pump().await;
            assert_eq!(opener.prompts(), ["work"]);

            opener.ends(0, "Found it.");
            opener.pump().await;
            assert_eq!(opener.prompts(), Vec::<String>::new(), "not into its turn");

            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;
            let prompts = opener.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(prompts[0].starts_with(FOUND_IT), "{prompts:?}");
        });
    }

    #[test]
    fn subagents_that_end_together_are_told_in_one_line() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look", "Fix"]).await;
            opener.prompts();

            opener.ends(0, "Found it.");
            opener.ends(1, "Fixed it.");
            opener.pump().await;

            let prompts = opener.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            let fixed = "The subagent \"Fix\" ended its turn. It last said:\n\nFixed it.";
            assert!(
                prompts[0].starts_with(&format!("{FOUND_IT}\n\n{fixed}")),
                "{prompts:?}"
            );
        });
    }

    #[test]
    fn a_subagent_s_report_heard_with_agent_report_is_not_told() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            opener.prompts();
            let waited = opener.wait(0);

            opener.ends(0, "Found it.");
            opener.pump().await;

            assert_eq!(
                waited.await.unwrap().unwrap().said.as_deref(),
                Some("Found it.")
            );
            assert_eq!(opener.prompts(), Vec::<String>::new());
            // Asked again, it answers again.
            let again = opener.wait(0).await.unwrap().unwrap();
            assert_eq!(again.said.as_deref(), Some("Found it."));
        });
    }

    impl Opener {
        /// Has the chat's agent continue its subagent `subagent` with
        /// `text`, as `agent_continue` does.
        fn continues(&mut self, subagent: Subagent, text: &str) -> Result<(u64, String), String> {
            let (answer, mut went_on) = oneshot::channel();
            self.daemon.asked(
                self.key,
                ChatRequest::ContinueChat {
                    subagent,
                    text: text.to_owned(),
                    answer,
                },
            );
            went_on.try_recv().unwrap()
        }

        /// Has the chat's agent stop its subagent `subagent`, for
        /// `reason`, as `agent_stop` does.
        fn stops(
            &mut self,
            subagent: Subagent,
            reason: Option<&str>,
        ) -> Result<(u64, String, Stopped), String> {
            let (answer, mut stopped) = oneshot::channel();
            self.daemon.asked(
                self.key,
                ChatRequest::StopChat {
                    subagent,
                    reason: reason.map(str::to_owned),
                    answer,
                },
            );
            stopped.try_recv().unwrap()
        }

        /// Whether the agent `index` of all those played was asked to
        /// cancel its turn, among what it was sent since last asked.
        fn cancelled(&self, index: usize) -> bool {
            let mut agents = self.agents.0.borrow_mut();
            let mut cancelled = false;
            while let Ok(command) = agents[index].commands.try_recv() {
                cancelled |= matches!(command, crate::agent::Command::Cancel);
            }
            cancelled
        }

        /// The notes the chat `key` shows of who stopped it.
        fn notes(&mut self, key: u64) -> Vec<String> {
            self.daemon
                .chat(key)
                .unwrap()
                .conversation()
                .entries()
                .iter()
                .filter_map(|entry| match entry {
                    cox_core::entry::Entry::Note(note) if note.contains("stopped this chat") => {
                        Some(note.clone())
                    }
                    _ => None,
                })
                .collect()
        }

        /// The lines the agent `index` of all those played was sent.
        fn prompts_of(&self, index: usize) -> Vec<String> {
            let mut agents = self.agents.0.borrow_mut();
            let mut prompts = Vec::new();
            while let Ok(command) = agents[index].commands.try_recv() {
                if let crate::agent::Command::Prompt(prompt) = command {
                    prompts.push(prompt.text);
                }
            }
            prompts
        }
    }

    #[test]
    fn a_subagent_cut_off_goes_on_in_its_own_chat_on_its_session_and_reports_to_its_opener() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            let look = opener.subagents[0];
            tells(
                &opener.agent(Some(0)),
                [
                    AgentEvent::Opened {
                        id: "look-1".to_owned(),
                        how: crate::agent::Opened::New,
                        steers: false,
                    },
                    AgentEvent::ReplyText("Half of it.".to_owned()),
                    AgentEvent::Closed(Some("coxd went".to_owned())),
                ],
            );
            opener.pump().await;
            // The opener heard it was cut off, and its turn on that ended.
            opener.prompts();
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;
            assert_eq!(opener.agents.0.borrow().len(), 2);

            assert_eq!(
                opener.continues(Subagent::Title("Look".to_owned()), "Go on."),
                Ok((look, "Look".to_owned()))
            );
            opener.pump().await;

            assert_eq!(
                opener.agents.0.borrow().len(),
                3,
                "the subagent's agent started again, in no new chat"
            );
            assert_eq!(opener.daemon.chats.len(), 2, "no chat opened");
            assert_eq!(
                opener.agent(Some(1)).resumes.as_deref(),
                Some("look-1"),
                "on its own session"
            );
            assert_eq!(opener.prompts_of(2), ["Go on."]);
            opener.ends(1, "All of it.");
            opener.pump().await;
            let prompts = opener.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(
                prompts[0].starts_with(
                    "The subagent \"Look\" ended its turn. It last said:\n\nAll of it."
                ),
                "{prompts:?}"
            );
        });
    }

    #[test]
    fn a_subagent_at_work_or_not_the_openers_own_is_not_continued() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            let look = opener.subagents[0];

            assert_eq!(
                opener.continues(Subagent::Number(look), "Go on."),
                Err(
                    "The subagent \"Look\" is at work still, so nothing is sent to it. Wait \
                     for its report, as Cox tells it, or with agent_report, giving its title, \
                     then continue it."
                        .to_owned()
                )
            );
            assert_eq!(opener.prompts_of(1), ["look"], "nothing more was sent");

            let (answer, mut went_on) = oneshot::channel();
            opener.daemon.asked(
                look,
                ChatRequest::ContinueChat {
                    subagent: Subagent::Number(opener.key),
                    text: "Go on.".to_owned(),
                    answer,
                },
            );
            assert!(
                went_on.try_recv().unwrap().is_err(),
                "a subagent does not continue the chat that opened it"
            );
            assert_eq!(
                opener.continues(Subagent::Title("Fix".to_owned()), "Go on."),
                Err(
                    "No subagent of this chat is titled \"Fix\". Its subagents go by \"Look\" \
                     now: give agent_continue one of those, and name it so to the human."
                        .to_owned()
                )
            );
        });
    }

    #[test]
    fn a_subagent_at_work_is_stopped_by_its_opener_and_goes_on_in_its_chat() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            let look = opener.subagents[0];
            opener.prompts();
            tells(
                &opener.agent(Some(0)),
                [AgentEvent::ReplyText("Half of it.".to_owned())],
            );
            opener.pump().await;

            assert_eq!(
                opener.stops(Subagent::Title("Look".to_owned()), Some("Wrong folder.")),
                Ok((look, "Look".to_owned(), Stopped::Turn))
            );
            assert!(opener.cancelled(1), "its turn is cancelled");
            let opener_name = opener.daemon.slot(opener.key).unwrap().name.clone();
            assert_eq!(
                opener.notes(look),
                [format!(
                    "The agent of \"{opener_name}\" stopped this chat: Wrong folder."
                )],
                "its chat shows the human who stopped it, and why"
            );
            let waited = opener.wait(0);
            tells(
                &opener.agent(Some(0)),
                [AgentEvent::TurnEnded(cox_acp::StopReason::Cancelled)],
            );
            opener.pump().await;

            let waited = waited.await.unwrap().unwrap();
            assert_eq!(waited.reply.as_deref(), Some("Half of it."));
            assert_eq!(waited.said.as_deref(), Some("Stopped."));
            assert_eq!(
                opener.prompts(),
                Vec::<String>::new(),
                "heard as it stopped, so not told again"
            );
            assert!(opener.daemon.slot(look).is_some(), "its chat stays");
            assert_eq!(
                opener.continues(Subagent::Title("Look".to_owned()), "Go on."),
                Ok((look, "Look".to_owned()))
            );
            opener.pump().await;
            // Its report heard, it was done, its agent closed: it starts
            // again, in its own chat.
            assert_eq!(opener.daemon.chats.len(), 2, "no chat opened");
            let last = opener.agents.0.borrow().len() - 1;
            assert_eq!(opener.prompts_of(last), ["Go on."]);
        });
    }

    #[test]
    fn a_subagent_waiting_for_the_human_is_stopped_and_its_question_goes() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            let look = opener.subagents[0];
            tells(
                &opener.agent(Some(0)),
                [AgentEvent::PermissionRequest {
                    id: "rm".to_owned(),
                    title: Some("rm -rf build".to_owned()),
                    content: None,
                    options: vec!["Allow".to_owned(), "Reject".to_owned()],
                }],
            );
            opener.pump().await;
            assert_eq!(opener.daemon.slot(look).unwrap().state, State::Asking);

            assert_eq!(
                opener.stops(Subagent::Number(look), None),
                Ok((look, "Look".to_owned(), Stopped::Turn))
            );
            assert!(opener.cancelled(1), "which answers its question too");
            let opener_name = opener.daemon.slot(opener.key).unwrap().name.clone();
            assert_eq!(
                opener.notes(look),
                [format!("The agent of \"{opener_name}\" stopped this chat.")]
            );
            assert_eq!(
                opener.stops(Subagent::Number(look), None),
                Ok((look, "Look".to_owned(), Stopped::Turn)),
                "stopping, it is the turn that stops still"
            );
        });
    }

    #[test]
    fn a_subagent_not_at_work_or_not_the_openers_own_or_the_humans_is_not_stopped() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            let look = opener.subagents[0];

            let (answer, mut stopped) = oneshot::channel();
            opener.daemon.asked(
                look,
                ChatRequest::StopChat {
                    subagent: Subagent::Number(opener.key),
                    reason: None,
                    answer,
                },
            );
            assert!(
                stopped.try_recv().unwrap().is_err(),
                "a subagent does not stop the chat that opened it"
            );
            assert_eq!(
                opener.stops(Subagent::Title("Fix".to_owned()), None),
                Err(
                    "No subagent of this chat is titled \"Fix\". Its subagents go by \"Look\" \
                     now: give agent_stop one of those, and name it so to the human."
                        .to_owned()
                )
            );

            opener.daemon.slot_mut(look).unwrap().whose = Whose::Human;
            assert_eq!(
                opener.stops(Subagent::Title("Look".to_owned()), None),
                Err(
                    "The human took the subagent \"Look\" over, so it is theirs to stop: ask \
                     them to."
                        .to_owned()
                )
            );
            assert!(!opener.cancelled(1), "nothing was stopped");

            opener.ends(0, "Found it.");
            opener.pump().await;
            opener.daemon.slot_mut(look).unwrap().whose = Whose::opened(Some(opener.key), None);
            assert_eq!(
                opener.stops(Subagent::Title("Look".to_owned()), Some("Enough.")),
                Ok((look, "Look".to_owned(), Stopped::Nothing))
            );
            assert!(!opener.cancelled(1));
            assert_eq!(opener.notes(look), Vec::<String>::new(), "nothing to show");
        });
    }

    #[test]
    fn what_a_subagent_runs_in_the_background_is_stopped_with_its_agent_to_start_again() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            let look = opener.subagents[0];
            tells(
                &opener.agent(Some(0)),
                [AgentEvent::Task(cox_acp::Task {
                    id: "sleep".to_owned(),
                    running: true,
                    started: true,
                    name: Some("sleep 120".to_owned()),
                    description: Some("sleep 120".to_owned()),
                    kind: Some("shell".to_owned()),
                })],
            );
            opener.ends(0, "Started it.");
            opener.pump().await;
            assert!(runs(&mut opener.daemon, look));

            assert_eq!(
                opener.stops(Subagent::Title("Look".to_owned()), None),
                Ok((look, "Look".to_owned(), Stopped::Background))
            );
            assert!(!runs(&mut opener.daemon, look), "its agent is ended");
            opener.pump().await;
            let waited = opener.wait(0).await.unwrap().unwrap();
            assert_eq!(waited.background, [], "nothing runs");
            assert_eq!(waited.reply.as_deref(), Some("Started it."));

            assert_eq!(
                opener.continues(Subagent::Title("Look".to_owned()), "Go on."),
                Ok((look, "Look".to_owned()))
            );
            opener.pump().await;
            assert_eq!(opener.agents.0.borrow().len(), 3, "started again");
            assert_eq!(opener.prompts_of(2), ["Go on."]);
        });
    }

    #[test]
    fn an_archived_subagent_comes_back_under_its_opener_to_go_on() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            let look = opener.subagents[0];
            opener.ends(0, "Found it.");
            opener.pump().await;
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;
            opener.daemon.archive(look, true);
            opener.pump().await;
            assert!(opener.daemon.chats.iter().all(|slot| slot.key != look));

            assert_eq!(
                opener.continues(Subagent::Title("Look".to_owned()), "Look again."),
                Ok((look, "Look".to_owned()))
            );

            assert!(opener.daemon.chats.iter().any(|slot| slot.key == look));
            assert!(
                !opener.daemon.reveals.contains(&look),
                "it does not take the human's eye"
            );
            opener.pump().await;
            let last = opener.agents.0.borrow().len() - 1;
            assert_eq!(last, 2, "done, its agent closed, and starts again");
            assert_eq!(opener.prompts_of(last), ["Look again."]);
        });
    }

    #[test]
    fn a_subagent_at_work_in_the_background_is_told_so_and_again_once_that_settled() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            opener.prompts();
            let number = opener.subagents[0];
            let task = |running, name: Option<&str>| {
                AgentEvent::Task(cox_acp::Task {
                    id: "sleep".to_owned(),
                    running,
                    started: running,
                    name: name.map(str::to_owned),
                    description: name.map(str::to_owned),
                    kind: name.map(|_| "shell".to_owned()),
                })
            };
            tells(&opener.agent(Some(0)), [task(true, Some("sleep 120"))]);
            opener.ends(0, "Started it.");
            opener.pump().await;

            let prompts = opener.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(
                prompts[0].starts_with(
                    "The subagent \"Look\" ended its turn, but still works in the background: \
                     the background command \"sleep 120\". Cox tells you once that ends; to stop \
                     it, call agent_stop, giving its title. It last said:\n\nStarted it."
                ),
                "{prompts:?}"
            );
            let listed = opener.daemon.listed.clone().unwrap();
            let row = listed.chats.iter().find(|row| row.key == number).unwrap();
            assert_eq!(row.state, State::Idle);
            assert_eq!(
                row.background,
                [wire::Background {
                    kind: wire::BackgroundKind::Command,
                    name: "sleep 120".to_owned(),
                    description: None,
                }]
            );
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;

            tells(&opener.agent(Some(0)), [task(false, None)]);
            opener.pump().await;
            assert_eq!(
                opener.prompts(),
                Vec::<String>::new(),
                "not until it settled"
            );
            let listed = opener.daemon.listed.clone().unwrap();
            let row = listed.chats.iter().find(|row| row.key == number).unwrap();
            assert_eq!(row.background, []);

            opener.daemon.hear(Internal::Settled { key: number });
            opener.pump().await;
            let prompts = opener.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(
                prompts[0].starts_with(
                    "The subagent \"Look\" is done with what it ran in the background. It last \
                     said:\n\nStarted it."
                ),
                "{prompts:?}"
            );
            // It is told once.
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.daemon.hear(Internal::Settled { key: number });
            opener.pump().await;
            assert_eq!(opener.prompts(), Vec::<String>::new());
        });
    }

    #[test]
    fn a_subagent_that_asks_the_human_is_told_as_asking() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            opener.prompts();

            tells(
                &opener.agent(Some(0)),
                [AgentEvent::PermissionRequest {
                    id: "rm".to_owned(),
                    title: Some("rm -rf build".to_owned()),
                    content: None,
                    options: vec!["Allow".to_owned(), "Reject".to_owned()],
                }],
            );
            opener.pump().await;

            let prompts = opener.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(
                prompts[0].starts_with(
                    "The subagent \"Look\" waits for the human to answer: rm -rf build."
                ),
                "{prompts:?}"
            );
        });
    }

    #[test]
    fn archiving_the_opener_leaves_a_subagent_s_unheard_report_unread_for_the_human() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look", "Fix"]).await;
            submit(&mut opener.daemon, opener.key);
            opener.pump().await;
            opener.ends(0, "Found it.");
            opener.pump().await;
            let unread = |opener: &Opener| -> Vec<bool> {
                opener
                    .subagents
                    .iter()
                    .map(|&key| opener.daemon.slot(key).unwrap().unread)
                    .collect()
            };
            assert_eq!(unread(&opener), [false, false], "the agent's to hear");

            opener.daemon.act(ToDaemon::Archive {
                key: opener.key,
                archived: true,
            });
            opener.pump().await;
            assert_eq!(unread(&opener), [true, false], "the human's now");

            // The one at work waits for the human once it ends, and the
            // archived chat's agent hears of neither.
            opener.ends(1, "Fixed it.");
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;
            assert_eq!(unread(&opener), [true, true]);
            assert_eq!(opener.prompts(), ["work"]);
        });
    }

    #[test]
    fn ci_that_ends_on_a_subagent_s_pull_request_is_told_once_to_the_human_and_the_opener() {
        use crate::github::{Check, CheckState, Pull, PullState, Status};
        on_daemon(async {
            let mut opener = Opener::new(&["Ship"]).await;
            let (window, mut told) = unbounded_channel();
            opener.daemon.window = Some(window);
            submit(&mut opener.daemon, opener.key);
            opener.pump().await;
            tells(
                &opener.agent(Some(0)),
                [AgentEvent::ToolCall {
                    id: "create".to_owned(),
                    title: Some("git push -u origin b && gh pr create --draft --fill".to_owned()),
                    command: true,
                    content: Some(vec![cox_core::entry::Content::Text(
                        "https://github.com/tennashi/cox/pull/423\n".to_owned(),
                    )]),
                    result: Some(ToolCallResult::Completed),
                }],
            );
            opener.ends(0, "Opened the pull request.");
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;
            assert_eq!(opener.prompts().len(), 2, "the line, then the report");
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;

            let pull = Pull {
                repo: "tennashi/cox".to_owned(),
                number: 423,
            };
            let now = Instant::now();
            assert_eq!(opener.daemon.due_pulls(now), std::slice::from_ref(&pull));
            assert_eq!(opener.daemon.due_pulls(now), [], "read once at a time");
            let status = |state| Status {
                title: "Ship it".to_owned(),
                state: PullState::Draft,
                head: "a".to_owned(),
                checks: vec![Check {
                    name: "ci (macos-latest)".to_owned(),
                    state,
                    link: None,
                    ended: None,
                }],
                times: crate::github::Times::default(),
            };
            for state in [CheckState::Pending, CheckState::Failed, CheckState::Failed] {
                opener.daemon.pulled(&pull, Ok(status(state)), now);
                opener.pump().await;
            }

            let mut news = Vec::new();
            while let Ok(message) = told.try_recv() {
                if let FromDaemon::News { key, body, .. } = message {
                    news.push((key, body));
                }
            }
            let ship = opener.subagents[0];
            let ci: Vec<_> = news
                .iter()
                .filter(|(_, body)| body.starts_with("CI"))
                .collect();
            assert_eq!(ci, [&(ship, "CI failed on #423 Ship it".to_owned())]);
            let row = |opener: &Opener| {
                let slot = opener.daemon.slot(ship).unwrap();
                opener.daemon.pull_rows(&slot.pulls)
            };
            assert_eq!(row(&opener)[0].unseen, Some(CheckState::Failed));
            assert!(!row(&opener)[0].waits_on_ci());
            let prompts = opener.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(
                prompts[0].contains(
                    "CI failed on the pull request it opened, \
                     https://github.com/tennashi/cox/pull/423 (\"Ship it\"). Failed: ci (macos-latest)."
                ),
                "{}",
                prompts[0]
            );

            opener.daemon.act(ToDaemon::Shown {
                key: Some(ship),
                project: 0,
            });
            assert_eq!(row(&opener)[0].unseen, None, "seen");
        });
    }

    /// What the agent tells as it opens the pull request #423 with
    /// `gh pr create`.
    fn opens_423() -> AgentEvent {
        AgentEvent::ToolCall {
            id: "create".to_owned(),
            title: Some("git push -u origin b && gh pr create --draft --fill".to_owned()),
            command: true,
            content: Some(vec![cox_core::entry::Content::Text(
                "https://github.com/tennashi/cox/pull/423\n".to_owned(),
            )]),
            result: Some(ToolCallResult::Completed),
        }
    }

    /// The pull request #423, and how it stands with its one check as
    /// `state`.
    fn pull_423() -> (
        crate::github::Pull,
        impl Fn(crate::github::CheckState) -> crate::github::Status,
    ) {
        use crate::github::{Check, Pull, PullState, Status};
        let pull = Pull {
            repo: "tennashi/cox".to_owned(),
            number: 423,
        };
        let status = |state| Status {
            title: "Ship it".to_owned(),
            state: PullState::Draft,
            head: "a".to_owned(),
            checks: vec![Check {
                name: "ci (macos-latest)".to_owned(),
                state,
                link: Some("https://github.com/tennashi/cox/actions/runs/1/job/2".to_owned()),
                ended: None,
            }],
            times: crate::github::Times::default(),
        };
        (pull, status)
    }

    /// What the agent of a chat of the human's is told once CI failed on
    /// #423.
    const FAILED_ON_423: &str = "CI failed on the pull request this chat opened, #423 \
        \"Ship it\", https://github.com/tennashi/cox/pull/423. Failed: ci (macos-latest), \
        https://github.com/tennashi/cox/actions/runs/1/job/2.\n\nCox tells you this unasked, \
        once CI ends. Go on with what it leaves you to do: if the human asked you to merge it \
        once CI passes, merge it; if a check failed, find why and fix it. Tell the human what \
        you did.";

    #[test]
    fn the_agent_of_a_chat_of_the_human_s_is_told_once_each_time_ci_ends_and_goes_on() {
        use crate::github::CheckState;
        on_daemon(async {
            let mut chat = Opener::new(&[]).await;
            submit(&mut chat.daemon, chat.key);
            chat.pump().await;
            tells(
                &chat.agent(None),
                [
                    opens_423(),
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
                ],
            );
            chat.pump().await;
            assert_eq!(chat.prompts(), ["work"]);

            let (pull, status) = pull_423();
            let now = Instant::now();
            assert_eq!(chat.daemon.due_pulls(now), std::slice::from_ref(&pull));
            for state in [CheckState::Pending, CheckState::Failed, CheckState::Failed] {
                chat.daemon.pulled(&pull, Ok(status(state)), now);
                chat.pump().await;
            }
            assert_eq!(
                chat.prompts(),
                [FAILED_ON_423],
                "one line, which starts a turn"
            );
            assert_eq!(
                chat.daemon.chat(chat.key).unwrap().conversation().state(),
                State::Busy
            );

            tells(
                &chat.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            chat.pump().await;
            assert_eq!(chat.prompts(), Vec::<String>::new(), "told once");

            for state in [CheckState::Pending, CheckState::Passed, CheckState::Passed] {
                chat.daemon.pulled(&pull, Ok(status(state)), now);
                chat.pump().await;
            }
            let prompts = chat.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(
                prompts[0].starts_with(
                    "CI passed on the pull request this chat opened, #423 \"Ship it\", \
                     https://github.com/tennashi/cox/pull/423.\n\n"
                ),
                "{}",
                prompts[0]
            );
        });
    }

    #[test]
    fn the_agent_of_a_chat_of_the_human_s_at_work_is_told_ci_ended_once_its_turn_ends() {
        use crate::github::CheckState;
        on_daemon(async {
            let mut chat = Opener::new(&[]).await;
            submit(&mut chat.daemon, chat.key);
            chat.pump().await;
            tells(&chat.agent(None), [opens_423()]);
            chat.pump().await;
            assert_eq!(chat.prompts(), ["work"]);

            let (pull, status) = pull_423();
            let now = Instant::now();
            chat.daemon.due_pulls(now);
            chat.daemon
                .pulled(&pull, Ok(status(CheckState::Pending)), now);
            chat.daemon
                .pulled(&pull, Ok(status(CheckState::Failed)), now);
            chat.pump().await;
            assert_eq!(
                chat.prompts(),
                Vec::<String>::new(),
                "not while its turn runs"
            );

            tells(
                &chat.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            chat.pump().await;
            assert_eq!(chat.prompts(), [FAILED_ON_423]);
        });
    }

    #[test]
    fn an_archived_chat_of_the_human_s_is_not_told_ci_ended() {
        use crate::github::CheckState;
        on_daemon(async {
            let mut chat = Opener::new(&[]).await;
            submit(&mut chat.daemon, chat.key);
            chat.pump().await;
            tells(
                &chat.agent(None),
                [
                    opens_423(),
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
                ],
            );
            chat.pump().await;
            chat.prompts();
            let (pull, status) = pull_423();
            let now = Instant::now();
            chat.daemon.due_pulls(now);
            chat.daemon.act(ToDaemon::Archive {
                key: chat.key,
                archived: true,
            });

            chat.daemon
                .pulled(&pull, Ok(status(CheckState::Pending)), now);
            chat.daemon
                .pulled(&pull, Ok(status(CheckState::Failed)), now);
            chat.pump().await;
            assert_eq!(chat.prompts(), Vec::<String>::new());
        });
    }

    #[test]
    fn an_agent_that_waits_for_a_subagent_s_report_hears_how_ci_ended_with_it() {
        use crate::github::CheckState;
        on_daemon(async {
            let mut opener = Opener::new(&["Ship"]).await;
            submit(&mut opener.daemon, opener.key);
            opener.pump().await;
            tells(&opener.agent(Some(0)), [opens_423()]);
            opener.ends(0, "Opened the pull request.");
            opener.pump().await;
            let mut waited = opener.wait(0);
            assert_eq!(waited.try_recv().unwrap().unwrap().ci, Vec::<String>::new());

            let (pull, status) = pull_423();
            let now = Instant::now();
            opener.daemon.due_pulls(now);
            opener
                .daemon
                .pulled(&pull, Ok(status(CheckState::Pending)), now);
            opener
                .daemon
                .pulled(&pull, Ok(status(CheckState::Failed)), now);
            opener.pump().await;
            let mut waited = opener.wait(0);
            assert_eq!(
                waited.try_recv().unwrap().unwrap().ci,
                ["CI failed on the pull request it opened, \
                     https://github.com/tennashi/cox/pull/423 (\"Ship it\"). Failed: ci \
                     (macos-latest), https://github.com/tennashi/cox/actions/runs/1/job/2."]
            );

            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            opener.pump().await;
            assert_eq!(
                opener.prompts(),
                ["work"],
                "heard already, so not told again"
            );
        });
    }

    /// When the chat `key` was marked done, and by whom, while it is.
    fn resolved(daemon: &Daemon, key: u64) -> Option<ResolvedBy> {
        daemon
            .slot(key)
            .unwrap()
            .resolved
            .map(|resolved| resolved.by)
    }

    /// Whether the agent of the chat `key` runs.
    fn runs(daemon: &mut Daemon, key: u64) -> bool {
        daemon.chat(key).unwrap().is_open()
    }

    /// The lines the last agent started was sent.
    fn last_prompts(agents: &PlayedAgents) -> Vec<String> {
        let mut agents = agents.0.borrow_mut();
        let last = agents.last_mut().unwrap();
        let mut prompts = Vec::new();
        while let Ok(command) = last.commands.try_recv() {
            if let crate::agent::Command::Prompt(prompt) = command {
                prompts.push(prompt.text);
            }
        }
        prompts
    }

    #[test]
    fn the_chat_shown_last_of_each_project_and_the_project_chosen_last_are_listed_past_a_restart() {
        let home = TempDir::new();
        let (store, older) = one_chat(&home);
        let project = kept_chat(&store, older).project;
        store.add_chat(project, "Newer").unwrap();
        let own = store.add_chat(None, "Own").unwrap();
        let (mut daemon, _told) = daemon_of(&store, &home);
        let (older_key, own_key) = (key_of(&mut daemon, older), key_of(&mut daemon, own));
        daemon.act(ToDaemon::Shown {
            key: Some(older_key),
            project: 1,
        });
        daemon.act(ToDaemon::Shown {
            key: Some(own_key),
            project: 0,
        });

        let listed = |daemon: &mut Daemon| {
            daemon.listed = None;
            daemon.tell_listing();
            let listing = daemon.listed.clone().unwrap();
            let lasts: Vec<_> = listing
                .projects
                .iter()
                .map(|project| project.last)
                .collect();
            (lasts, listing.chosen)
        };
        let (mut again, _told) = daemon_of(&store, &home);
        let (older_again, own_again) = (key_of(&mut again, older), key_of(&mut again, own));
        assert_eq!(
            listed(&mut again),
            (vec![Some(own_again), Some(older_again)], 0),
            "Cox's own chosen last, each project with the chat of it shown last"
        );

        // The project chosen with no chat shown keeps the chat of it shown
        // before.
        again.act(ToDaemon::Shown {
            key: None,
            project: 1,
        });
        let (mut third, _told) = daemon_of(&store, &home);
        let (older_third, own_third) = (key_of(&mut third, older), key_of(&mut third, own));
        assert_eq!(
            listed(&mut third),
            (vec![Some(own_third), Some(older_third)], 1)
        );
    }

    #[test]
    fn a_chat_listed_tells_when_it_last_moved() {
        let home = TempDir::new();
        let (store, chat) = one_chat(&home);
        let (mut daemon, _told) = daemon_of(&store, &home);
        let key = key_of(&mut daemon, chat);
        let kept = kept_chat(&store, chat).active;
        daemon.listed = None;
        daemon.tell_listing();
        let listing = daemon.listed.clone().unwrap();
        assert_eq!(listing.chats[0].key, key);
        assert_eq!(
            listing.chats[0].active, kept,
            "as kept, before a turn moved it"
        );
    }

    #[test]
    fn the_chats_told_say_which_are_marked_done() {
        let home = TempDir::new();
        let (store, chat) = one_chat(&home);
        let (mut daemon, _told) = daemon_of(&store, &home);
        let key = key_of(&mut daemon, chat);
        let project = daemon.slot(key).unwrap().project;

        daemon.act(ToDaemon::Resolve {
            key,
            resolved: true,
        });
        assert_eq!(
            daemon.chats_told(project, None),
            [format!("- chat {}: \"Chat\", marked done", chat.number())]
        );

        daemon.act(ToDaemon::Resolve {
            key,
            resolved: false,
        });
        let told = daemon.chats_told(project, None);
        assert_eq!(told.len(), 1);
        assert!(!told[0].contains("done"), "{told:?}");
    }

    #[test]
    fn a_chat_marked_done_is_kept_so_once_and_reopened_once() {
        let home = TempDir::new();
        let (store, chat) = one_chat(&home);
        let (mut daemon, _told) = daemon_of(&store, &home);
        let key = key_of(&mut daemon, chat);
        daemon.slot_mut(key).unwrap().unread = true;

        daemon.act(ToDaemon::Resolve {
            key,
            resolved: true,
        });
        let first = kept_chat(&store, chat).resolved.unwrap();
        daemon.act(ToDaemon::Resolve {
            key,
            resolved: true,
        });

        assert_eq!(first.by, ResolvedBy::Human);
        assert_eq!(kept_chat(&store, chat).resolved, Some(first), "marked once");
        assert!(!daemon.slot(key).unwrap().unread);
        let (mut again, _told) = daemon_of(&store, &home);
        let key_again = key_of(&mut again, chat);
        assert_eq!(
            again.slot(key_again).unwrap().resolved,
            Some(first),
            "done past coxd starting again"
        );

        for _ in 0..2 {
            daemon.act(ToDaemon::Resolve {
                key,
                resolved: false,
            });
            assert_eq!(kept_chat(&store, chat).resolved, None);
            assert_eq!(resolved(&daemon, key), None);
        }
    }

    #[test]
    fn a_chat_marked_done_at_work_stops_closes_and_goes_on_with_the_next_line() {
        on_daemon(async {
            let mut chat = Opener::new(&[]).await;
            submit(&mut chat.daemon, chat.key);
            chat.pump().await;
            assert_eq!(chat.prompts(), ["work"]);

            chat.daemon.act(ToDaemon::Resolve {
                key: chat.key,
                resolved: true,
            });
            chat.pump().await;
            let cancel =
                std::iter::from_fn(|| chat.agents.0.borrow_mut()[0].commands.try_recv().ok())
                    .any(|command| matches!(command, crate::agent::Command::Cancel));
            assert!(cancel, "asked to stop");
            assert_eq!(
                chat.daemon.chat(chat.key).unwrap().conversation().state(),
                State::Stopping
            );

            tells(
                &chat.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::Cancelled)],
            );
            chat.pump().await;
            assert!(!runs(&mut chat.daemon, chat.key), "closed once stopped");
            assert!(!chat.daemon.slot(chat.key).unwrap().unread);

            submit(&mut chat.daemon, chat.key);
            chat.pump().await;
            assert_eq!(resolved(&chat.daemon, chat.key), None, "a line reopens it");
            assert_eq!(chat.agents.0.borrow().len(), 2, "its agent started again");
            assert_eq!(last_prompts(&chat.agents), ["work"]);
        });
    }

    #[test]
    fn ci_that_ends_in_a_chat_marked_done_reopens_it() {
        use crate::github::CheckState;
        on_daemon(async {
            let mut chat = Opener::new(&[]).await;
            submit(&mut chat.daemon, chat.key);
            chat.pump().await;
            tells(
                &chat.agent(None),
                [
                    opens_423(),
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
                ],
            );
            chat.pump().await;
            chat.daemon.act(ToDaemon::Resolve {
                key: chat.key,
                resolved: true,
            });
            chat.pump().await;
            assert!(!runs(&mut chat.daemon, chat.key));

            let (pull, status) = pull_423();
            let now = Instant::now();
            assert_eq!(chat.daemon.due_pulls(now), std::slice::from_ref(&pull));
            for state in [CheckState::Pending, CheckState::Failed, CheckState::Failed] {
                chat.daemon.pulled(&pull, Ok(status(state)), now);
                chat.pump().await;
            }

            assert_eq!(resolved(&chat.daemon, chat.key), None);
            assert_eq!(last_prompts(&chat.agents), [FAILED_ON_423]);
        });
    }

    #[test]
    fn a_subagent_is_done_once_its_report_reached_its_opener_and_reopens_asked_again() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            opener.prompts();
            let look = opener.subagents[0];

            opener.ends(0, "Found it.");
            opener.pump().await;

            assert!(opener.prompts()[0].starts_with(FOUND_IT));
            assert_eq!(resolved(&opener.daemon, look), Some(ResolvedBy::Agent));
            assert!(!runs(&mut opener.daemon, look), "its agent closed");
            assert_eq!(resolved(&opener.daemon, opener.key), None);

            let again = opener.wait(0).await.unwrap().unwrap();
            assert_eq!(again.said.as_deref(), Some("Found it."));
            assert_eq!(resolved(&opener.daemon, look), None, "asked again");
        });
    }

    #[test]
    fn a_subagent_at_work_in_the_background_is_done_only_once_that_ended_was_told() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            opener.prompts();
            let look = opener.subagents[0];
            let task = |running| {
                AgentEvent::Task(cox_acp::Task {
                    id: "serve".to_owned(),
                    running,
                    started: running,
                    name: Some("npm run dev".to_owned()),
                    description: None,
                    kind: Some("shell".to_owned()),
                })
            };
            tells(&opener.agent(Some(0)), [task(true)]);
            opener.ends(0, "Serving.");
            opener.pump().await;
            assert!(opener.prompts()[0].contains("still works in the background"));
            assert_eq!(resolved(&opener.daemon, look), None);
            tells(
                &opener.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );

            tells(&opener.agent(Some(0)), [task(false)]);
            opener.pump().await;
            opener.daemon.hear(Internal::Settled { key: look });
            opener.pump().await;
            assert_eq!(
                resolved(&opener.daemon, look),
                Some(ResolvedBy::Agent),
                "once what it ran ended was told"
            );
        });
    }

    #[test]
    fn a_chat_whose_merge_ended_is_done() {
        on_daemon(async {
            let mut chat = Opener::new(&[]).await;
            submit(&mut chat.daemon, chat.key);
            chat.pump().await;
            tells(
                &chat.agent(None),
                [AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn)],
            );
            chat.pump().await;
            assert_eq!(
                resolved(&chat.daemon, chat.key),
                None,
                "a turn alone is not"
            );

            chat.daemon.act(ToDaemon::Merge { key: chat.key });
            chat.pump().await;
            tells(
                &chat.agent(None),
                [
                    AgentEvent::ReplyText("Merged.".to_owned()),
                    AgentEvent::TurnEnded(cox_acp::StopReason::EndTurn),
                ],
            );
            chat.pump().await;

            assert_eq!(resolved(&chat.daemon, chat.key), Some(ResolvedBy::Agent));
            assert!(!chat.daemon.slot(chat.key).unwrap().unread);
        });
    }

    #[test]
    fn a_parent_marked_done_marks_its_subagents_done_but_those_at_work_or_asking() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look", "Fix", "Ask"]).await;
            submit(&mut opener.daemon, opener.key);
            opener.pump().await;
            opener.ends(0, "Found it.");
            tells(
                &opener.agent(Some(2)),
                [AgentEvent::PermissionRequest {
                    id: "rm".to_owned(),
                    title: Some("rm -rf build".to_owned()),
                    content: None,
                    options: vec!["Allow".to_owned(), "Reject".to_owned()],
                }],
            );
            opener.pump().await;
            assert_eq!(
                resolved(&opener.daemon, opener.subagents[0]),
                None,
                "unheard"
            );

            opener.daemon.act(ToDaemon::Resolve {
                key: opener.key,
                resolved: true,
            });
            opener.pump().await;

            let done: Vec<Option<ResolvedBy>> = opener
                .subagents
                .iter()
                .map(|&key| resolved(&opener.daemon, key))
                .collect();
            assert_eq!(done, [Some(ResolvedBy::Human), None, None]);
        });
    }

    #[test]
    fn an_agent_marks_another_chat_done_but_its_own_one_at_work_or_one_asking() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look", "Fix", "Ask"]).await;
            submit(&mut opener.daemon, opener.key);
            opener.pump().await;
            opener.ends(0, "Found it.");
            tells(
                &opener.agent(Some(2)),
                [AgentEvent::PermissionRequest {
                    id: "rm".to_owned(),
                    title: Some("rm -rf build".to_owned()),
                    content: None,
                    options: vec!["Allow".to_owned(), "Reject".to_owned()],
                }],
            );
            opener.pump().await;
            let number =
                |daemon: &Daemon, key: u64| daemon.slot(key).unwrap().kept.unwrap().number();
            let [look, fix, ask] = opener.subagents[..] else {
                panic!("three subagents");
            };
            // Fix, at work, asks to mark each chat done.
            let resolve = |daemon: &mut Daemon, chat: i64| {
                let (answer, mut resolved) = oneshot::channel();
                daemon.asked(fix, ChatRequest::ResolveChat { chat, answer });
                resolved.try_recv().unwrap()
            };
            let key = opener.key;
            let daemon = &mut opener.daemon;
            let [own, opener_number, ask_number, look_number] =
                [fix, key, ask, look].map(|key| number(daemon, key));

            let own = resolve(daemon, own).unwrap_err();
            assert!(
                own.ends_with(
                    "is this chat: the human marks it done, or Cox does once its work ends."
                ),
                "{own}"
            );
            let at_work = resolve(daemon, opener_number).unwrap_err();
            assert!(at_work.contains("is at work"), "{at_work}");
            let asking = resolve(daemon, ask_number).unwrap_err();
            assert!(asking.contains("waits for the human"), "{asking}");
            let none = resolve(daemon, 999).unwrap_err();
            assert_eq!(none, "This project has no chat 999.");
            assert_eq!(resolved(daemon, fix), None);
            assert_eq!(resolved(daemon, key), None);
            assert_eq!(resolved(daemon, ask), None);

            let done = resolve(daemon, look_number).unwrap();
            assert_eq!(
                done,
                format!(
                    "Marked chat {look_number}, \"Look\", done. Anything that arrives in it \
                     reopens it."
                )
            );
            assert_eq!(resolved(daemon, look), Some(ResolvedBy::Agent));
            let again = resolve(daemon, look_number).unwrap();
            assert!(again.ends_with("is marked done already."), "{again}");
            opener.pump().await;
            assert!(!runs(&mut opener.daemon, look), "its agent closed");
        });
    }

    #[test]
    fn a_report_that_arrives_in_a_parent_marked_done_reopens_it() {
        on_daemon(async {
            let mut opener = Opener::new(&["Look"]).await;
            opener.prompts();
            opener.daemon.act(ToDaemon::Resolve {
                key: opener.key,
                resolved: true,
            });
            opener.pump().await;
            assert!(!runs(&mut opener.daemon, opener.key));
            assert_eq!(
                resolved(&opener.daemon, opener.subagents[0]),
                None,
                "at work"
            );

            opener.ends(0, "Found it.");
            opener.pump().await;

            assert_eq!(resolved(&opener.daemon, opener.key), None);
            assert!(last_prompts(&opener.agents)[0].starts_with(FOUND_IT));
        });
    }
}
