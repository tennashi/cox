//! What the daemon and a window say to each other: what the human does in
//! the window, [`ToDaemon`], and what the daemon tells the window,
//! [`FromDaemon`]. Both go as JSON, a message a line, once the window
//! talks to a daemon of its own process (see [`crate::daemon`]).

use std::collections::BTreeMap;
use std::path::PathBuf;

use cox_core::entry::{Attachment, OnStep};
use cox_core::goal::Goal;
use cox_core::store::{ChatId, Named, ProjectId, Resolved, TurnEnd};
use cox_core::tabs::{KeptTabs, Lines};
use serde::{Deserialize, Serialize};

use crate::agent::mcp::{AndroidAct, MacAct, Ran, Seen};
use crate::agent::setup::Setup;
use crate::chat::conversation::{Answer, Patch, QuestionAnswer, State};
use crate::chat::sketch::Sketch;
use crate::chat::spent::Spent;
use crate::github::{CheckState, Pull, Status};

/// A project, as the window lists it on its rail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRow {
    /// How it is kept, or `None` for Cox's own.
    pub id: Option<ProjectId>,
    pub name: String,
    /// The folder its chats work in.
    pub folder: PathBuf,
    /// Whether its folder is not there, as when the human moved it.
    #[serde(default)]
    pub missing: bool,
    /// Whether the human put it away, off the rail.
    #[serde(default)]
    pub archived: bool,
    /// Whether the human deleted it, while the daemon runs: it stays in
    /// the list, so that the projects after it keep their places.
    #[serde(default)]
    pub deleted: bool,
    /// What it is, in a sentence or two, as each chat's agent is told, if
    /// the human set it.
    #[serde(default)]
    pub about: Option<String>,
    /// The chat of it the window showed last, by its key, as kept past
    /// a restart, once it showed one that is still there.
    #[serde(default)]
    pub last: Option<u64>,
}

/// A chat, as the window lists it in its tabs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "how a chat stands, each apart"
)]
pub struct ChatRow {
    /// What the chat goes by while the daemon runs, to the window and to
    /// the agents' tools.
    pub key: u64,
    /// Its project, by its place in the projects.
    pub project: usize,
    /// How it is kept, if it is.
    pub kept: Option<ChatId>,
    pub name: String,
    pub named: Named,
    /// Whether its agent runs.
    pub open: bool,
    /// Whether a line sent to it, closed, starts its agent again.
    pub again: bool,
    pub state: State,
    /// Whether a turn of the human's ended that they have not seen; a
    /// turn of a chat an agent opened is that agent's to read.
    pub unread: bool,
    pub goal: Option<Goal>,
    /// The chat whose agent opened it, by its key.
    pub parent: Option<u64>,
    /// How its agent is set up, when the chat that opened it said.
    pub setup: Option<Setup>,
    /// Whether the sketch it is is on its way to an end.
    pub ending: bool,
    /// The id of the agent it talks to.
    pub agent: String,
    /// The folder it works in, when it is not its project's, as one that
    /// opened before another folder of the project became the project's.
    #[serde(default)]
    pub folder: Option<PathBuf>,
    /// What its agent runs apart from the turns Cox asks of it, as it
    /// tells it, in the order it started each: the chat is at work while
    /// anything runs, though its turn ended.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background: Vec<Background>,
    /// The pull requests its agent opened, the last few, the first opened
    /// first, each as it stood when Cox last read it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pulls: Vec<PullRow>,
    /// What its agent spent, once it told any of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spent: Option<Spent>,
    /// When it was marked done, and by whom, while it is: the list folds
    /// it away at its end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
    /// Whether a line the human sends now goes into the turn its agent
    /// takes, as it asks them something too, if it does.
    #[serde(default)]
    pub steers: bool,
    /// The lines the human wrote on steps of its plan that wait for its
    /// turn to end, the first first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_steps: Vec<OnStepLine>,
    /// When it last moved, in milliseconds since the epoch: when a turn
    /// last started or ended, or, before any did in this run, when the
    /// last entry of its conversation was kept, or it was opened.
    #[serde(default)]
    pub active: i64,
    /// How its last turn was cut off, as one that failed, hit a limit, or
    /// whose agent, or coxd, went as it ran, while it waits to go on: not
    /// at work again, nor marked done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_off: Option<TurnEnd>,
}

/// A line the human wrote on a step of a chat's plan, which waits for the
/// turn that runs to end, to go as the next line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnStepLine {
    pub text: String,
    pub step: OnStep,
}

/// A pull request a chat's agent opened, as the window shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRow {
    pub pull: Pull,
    /// How it stood when Cox last read it, once it read it.
    pub status: Option<Status>,
    /// How its CI ended, once it ended while the human did not look at
    /// the chat, until they do.
    #[serde(default)]
    pub unseen: Option<CheckState>,
}

impl PullRow {
    /// Whether its CI runs, so that its chat waits on CI.
    #[must_use]
    pub fn waits_on_ci(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| !status.done() && status.checks() == Some(CheckState::Pending))
    }
}

/// Whether a chat that stands as `state`, its agent running `background`,
/// works in the background: its turn ended, and it waits for no one, but
/// something it runs goes on.
#[must_use]
pub fn in_background(state: State, background: &[Background]) -> bool {
    state == State::Idle && !background.is_empty()
}

/// Something a chat's agent runs apart from the turns Cox asks of it, as
/// the agent tells it: a command it left running in the background, a
/// turn it took by itself, as once such a command ended, or a tool call
/// of that turn that has not ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Background {
    pub kind: BackgroundKind,
    /// What it goes by: for a command, what the agent said it does, as
    /// "Wait 120 seconds", or the command; for a tool call, its title.
    pub name: String,
    /// What it does, when the agent says more than its name: for a turn
    /// the agent took by itself, what started it.
    #[serde(default)]
    pub description: Option<String>,
}

/// What kind of thing a chat's agent runs in the background.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundKind {
    /// A command it left running, named by what the agent said it does,
    /// or by the command itself.
    Command,
    /// A turn it took by itself, while no turn Cox asked of it ran.
    Turn,
    /// A tool call it made in a turn it took by itself.
    ToolCall,
    /// Another task, of the kind the agent named, as "monitor".
    Task(String),
}

impl Background {
    /// What it is, in a few words, as a row of the window shows it: the
    /// command, or, for a turn the agent took by itself, what started it.
    #[must_use]
    pub fn what(&self) -> &str {
        match (&self.kind, &self.description) {
            (BackgroundKind::Turn, Some(description)) => description,
            _ => &self.name,
        }
    }

    /// Whether it is a command the agent left running. Its name is most
    /// often what the agent said the command does, not the command, so
    /// the window marks it as a command apart from its name, not as a
    /// line of a shell.
    #[must_use]
    pub fn is_command(&self) -> bool {
        self.kind == BackgroundKind::Command
    }

    /// What it is, with its kind, as a line of text says it where no mark
    /// shows the kind: `command "Wait 120 seconds"`, or, for a turn the
    /// agent took by itself, what started it.
    #[must_use]
    pub fn labeled(&self) -> String {
        let name = &self.name;
        match &self.kind {
            BackgroundKind::Command => format!("command \"{name}\""),
            BackgroundKind::Turn => match &self.description {
                Some(description) => format!("its own turn ({description})"),
                None => "its own turn".to_owned(),
            },
            BackgroundKind::ToolCall => format!("tool call \"{name}\""),
            BackgroundKind::Task(kind) => format!("{kind} \"{name}\""),
        }
    }

    /// What it is, as an agent is told.
    #[must_use]
    pub fn told(&self) -> String {
        let name = &self.name;
        match &self.kind {
            BackgroundKind::Command => format!("the background command \"{name}\""),
            BackgroundKind::Turn => match &self.description {
                Some(description) => format!("a turn it took by itself ({description})"),
                None => "a turn it took by itself".to_owned(),
            },
            BackgroundKind::ToolCall => format!("the tool call \"{name}\""),
            BackgroundKind::Task(kind) => format!("the {kind} \"{name}\""),
        }
    }
}

/// What `background` is, each as an agent is told, in one line.
#[must_use]
pub fn told(background: &[Background]) -> String {
    background
        .iter()
        .map(Background::told)
        .collect::<Vec<_>>()
        .join("; ")
}

/// The projects and chats, as the window shows them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    /// Cox's own first, then the human's, in the order they were
    /// opened.
    pub projects: Vec<ProjectRow>,
    /// The chats in the order of the tabs, project by project, each chat
    /// an agent opened under the chat that opened it.
    pub chats: Vec<ChatRow>,
    /// The chats the human put away.
    pub archived: Vec<ChatRow>,
    /// The names the human knows the agents of the chats by, by their
    /// ids, for a page that knows no agents, as the phone's.
    #[serde(default)]
    pub agents: BTreeMap<String, String>,
    /// Why no chat shows its pull requests, as when `gh` is not logged
    /// in, if that is so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<String>,
    /// The project the human chose last, by its place in the projects,
    /// as kept past a restart, for a window that starts to choose.
    #[serde(default)]
    pub chosen: usize,
    /// The pull requests the projects hold, the first seen first: those
    /// open, and those merged or closed lately.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pulls: Vec<ProjectPull>,
}

/// A pull request a project holds, whichever of its chats' agents opened
/// it, as the window shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPull {
    pub pull: Pull,
    /// Its project, by its place in the projects.
    pub project: usize,
    /// How it stood when Cox last read it, once it read it.
    pub status: Option<Status>,
    /// The chat whose agent opened it, by its key, while it is listed.
    #[serde(default)]
    pub author: Option<u64>,
}

/// What an agent's tools ask of its chat's stage, as Cox's
/// [`ToolRequest`](crate::agent::mcp::ToolRequest) says it, with a number
/// for a command, whose answer the window gives back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageAsk {
    ShowMarkdown {
        path: PathBuf,
        text: String,
    },
    ShowFile {
        path: PathBuf,
        text: String,
        lines: Option<Lines>,
    },
    ShowChanges {
        folder: PathBuf,
    },
    ShowMap {
        map: Option<String>,
        at: Vec<String>,
    },
    /// Run `command` in a terminal, in `cwd`, and answer how it went as
    /// [`ToDaemon::Ran`], with `id`.
    RunCommand {
        id: u64,
        command: String,
        cwd: PathBuf,
    },
    /// Do `act` with the windows of the Mac's other apps, and answer what
    /// the agent sees as [`ToDaemon::Seen`], with `id`.
    MacWindow {
        id: u64,
        act: MacAct,
    },
    /// Do `act` with the Android emulators, and answer what the agent
    /// sees as [`ToDaemon::Seen`], with `id`.
    Android {
        id: u64,
        act: AndroidAct,
    },
}

/// What a build of Cox is, as its window and coxd compare it (see
/// `build.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// What its window and coxd say to each other: a hash of the types
    /// that go between them.
    #[serde(with = "hex")]
    pub protocol: u64,
    /// What its coxd does: a hash of the code coxd runs.
    #[serde(with = "hex")]
    pub daemon: u64,
}

/// The version of this build.
pub const VERSION: Version = Version {
    protocol: hex::of(env!("COX_PROTOCOL")),
    daemon: hex::of(env!("COX_DAEMON")),
};

/// What a window and coxd say to each other besides the types that go
/// between them: one more makes [`Version::protocol`] another, as when a
/// message comes to mean something else.
pub const REVISION: u32 = 1;

/// What it takes to run a build in place of the one that runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    /// The window starts again, and joins the coxd that runs, whose chats
    /// go on.
    WindowOnly,
    /// coxd stops too, once no chat is at work, and the window starts one
    /// of its own build: the build says what coxd would not read.
    WithCoxd,
}

impl Version {
    /// What it takes to run a window of this version with coxd of
    /// `coxd`'s.
    #[must_use]
    pub fn restart(self, coxd: Self) -> Restart {
        if self.protocol == coxd.protocol {
            Restart::WindowOnly
        } else {
            Restart::WithCoxd
        }
    }
}

/// A version's hashes, as 16 hex digits: JSON's numbers do not hold all
/// of a `u64`.
mod hex {
    use serde::{Deserialize as _, Deserializer, Serializer};

    /// `hex`, as `build.rs` writes it.
    pub(super) const fn of(hex: &str) -> u64 {
        match u64::from_str_radix(hex, 16) {
            Ok(value) => value,
            Err(_) => panic!("build.rs tells hex"),
        }
    }

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `with` passes the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(value: &u64, to: S) -> Result<S::Ok, S::Error> {
        to.serialize_str(&format!("{value:016x}"))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(from: D) -> Result<u64, D::Error> {
        let hex = String::deserialize(from)?;
        u64::from_str_radix(&hex, 16).map_err(serde::de::Error::custom)
    }
}

/// What a window asks of the coxd it comes to, as the human started it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Asks {
    /// To join it, if it reads the window; else that it stops for one of
    /// the window's build once no chat is at work.
    #[default]
    Join,
    /// That it stops for one of the window's build once no chat is at
    /// work, as the human restarts Cox and coxd.
    Restart,
    /// That it stops for one of the window's build now, cutting the chats
    /// at work.
    RestartNow,
}

/// What a window says first, as it comes to coxd, of the command line
/// it was started with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Where Cox's own chats work for this window's run, as `--home`
    /// gave it, ahead of the settings.
    pub home: Option<PathBuf>,
    /// The agent a new chat talks to, and its arguments, as the command
    /// line gave it, or none for the one the settings name.
    pub command: Vec<String>,
    /// The window's version, or none from a window of a build that did
    /// not tell it.
    #[serde(default)]
    pub version: Option<Version>,
    #[serde(default)]
    pub asks: Asks,
    /// Whether the window reads which chats are at work, should coxd not
    /// take it while they are: [`FromDaemon::AtWork`] in place of
    /// [`FromDaemon::Busy`]. A coxd of a build that did not know to tell
    /// it answers `Busy` all the same, which a window of any build reads.
    #[serde(default)]
    pub reads_at_work: bool,
}

impl Hello {
    /// What a window of this build says first, with `home` and `command`
    /// from its command line, asking `asks`.
    #[must_use]
    pub fn new(home: Option<PathBuf>, command: Vec<String>, asks: Asks) -> Self {
        Self {
            home,
            command,
            version: Some(VERSION),
            asks,
            reads_at_work: true,
        }
    }

    /// How coxd of `coxd`'s version answers this hello from a window it
    /// could take, as a chat of its is `at_work` or not: it takes a window
    /// that speaks as it does, unless the window asks it to stop, and it
    /// stops for any other once no chat is at work, or at once when the
    /// window asks so.
    #[must_use]
    pub fn answer(&self, coxd: Version, at_work: bool) -> FromDaemon {
        let alike = self
            .version
            .is_some_and(|version| version.restart(coxd) == Restart::WindowOnly);
        match self.asks {
            Asks::Join if alike => FromDaemon::Joined(coxd),
            Asks::Join | Asks::Restart if at_work => FromDaemon::Busy,
            Asks::Join | Asks::Restart | Asks::RestartNow => FromDaemon::Outdated,
        }
    }
}

/// A chat at work, which keeps coxd from stopping for a window of
/// another build, as the window waiting tells the human.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtWork {
    /// Its project's name, or none for Cox's own.
    pub project: Option<String>,
    /// The chat's name.
    pub chat: String,
}

/// What the daemon tells a window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FromDaemon {
    /// The window's [`Hello`] was heard, from a window that speaks as
    /// the daemon of this version does: what follows is the daemon's.
    Joined(Version),
    /// What a coxd of a build that did not tell builds apart answers a
    /// window it takes, of any build: a window takes it as
    /// [`FromDaemon::Busy`], since what follows may not read.
    Welcome,
    /// Another window is there already, which the daemon goes on with.
    Taken,
    /// The window speaks otherwise than the daemon, as when `cargo xtask
    /// install` replaced its program, or asks it to stop, and no chat is
    /// at work, or the window asks it to stop now: the daemon stops, for
    /// the window to start one of its own build.
    Outdated,
    /// The window speaks otherwise than the daemon, or asks it to stop,
    /// and the daemon goes on as it is, since a chat is at work: it does
    /// not take the window, which comes again later.
    Busy,
    /// [`FromDaemon::Busy`], to a window that reads it, with the chats
    /// at work that keep coxd from stopping for it.
    AtWork(Vec<AtWork>),
    /// The projects and chats, as they are now.
    Listing(Listing),
    /// How the conversation of the chat `key` changed, for a window that
    /// watches it (see [`ToDaemon::Watch`]): all of it, the first time.
    Patch { key: u64, patch: Patch },
    /// What the stage of the folder the chat `key` works in had, as it
    /// was kept, each tab of a chat by its key, for a window that starts
    /// to watch the chat.
    Tabs {
        key: u64,
        tabs: Option<KeptTabs<u64>>,
    },
    /// What the stage of `folder` had, as it was kept, each tab of a chat
    /// by its key, for a window that shows the stage with no chat of the
    /// folder (see [`ToDaemon::StageOf`]).
    StageOf {
        folder: PathBuf,
        tabs: Option<KeptTabs<u64>>,
    },
    /// Show the chat `key`, which the human asked for, as when it opens.
    Reveal { key: u64 },
    /// The project kept as `project` has another icon, as the human set
    /// one an agent proposed: the window draws it as it is now.
    Icon { project: ProjectId },
    /// The chat `key`, named `name`, comes to wait for the human, ended
    /// a turn, or its agent changed the plan the human left it to, as
    /// `body` says, for the window to tell the human, unless they look at
    /// it.
    News {
        key: u64,
        name: String,
        body: String,
    },
    /// What the agent of the chat `key` asks of its stage.
    Stage { key: u64, ask: StageAsk },
    /// The command `id` is not waited for any more: it is to stop.
    Cancel { id: u64 },
    /// The agent of the chat `key`, which works in `cwd`, is to start:
    /// the window gives its stage a relay for the agent's browser tools
    /// and answers its port, as [`ToDaemon::Prepared`] with `id`.
    Prepare { key: u64, cwd: PathBuf, id: u64 },
    /// All the window asked before [`ToDaemon::Sync`] with `id` is done.
    Settled { id: u64 },
    /// Every phone is unpaired, as the window asked with
    /// [`ToDaemon::UnpairPhones`], unless `failed` says why not.
    Unpaired { failed: Option<String> },
    /// The map the window asked for with [`ToDaemon::Map`] with `id`, as
    /// the JSON of [`crate::map::Told`].
    Map { id: u64, told: String },
}

/// What a window tells the daemon: what the human did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToDaemon {
    /// Open a chat in the project of `folder`, or of Cox's own, and show
    /// it.
    Open { folder: Option<PathBuf> },
    /// Make a project, with nothing set yet, in a folder of its own in
    /// the projects of Cox's home, and open a chat in it, and show it.
    NewProject,
    /// Make a project as [`ToDaemon::NewProject`] does, and open a chat
    /// in it, as from the phone: the window lists them, but goes on
    /// showing what it shows.
    NewProjectAside,
    /// Open a chat in the project of `folder`, one the human has, or of
    /// Cox's own, as from the phone: the window lists it, but goes on
    /// showing what it shows.
    NewChat { folder: Option<PathBuf> },
    /// The window shows the chat `key`, or none, of the project chosen,
    /// by its place in the projects.
    Shown { key: Option<u64>, project: usize },
    /// The window shows the conversation of the chat `key` from now on:
    /// tell it how it changes.
    Watch { key: u64 },
    /// The human sent `text` in the chat `key`, with what they handed it
    /// from elsewhere on the screen, and on the step of its agent's plan
    /// they wrote it on, from the Activity page, if they did.
    Submit {
        key: u64,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attached: Vec<Attachment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step: Option<OnStep>,
    },
    /// The human chose the option at `index` of what the agent of the
    /// chat `key` asks: of a permission request, or of the ways to log in.
    Choose { key: u64, index: usize },
    /// The human answered the questions the agent of the chat `key` asks.
    AnswerQuestion { key: u64, answer: QuestionAnswer },
    /// The human stopped the chat `key`'s turn.
    Stop { key: u64 },
    /// The human set the goal of the chat `key`, or cleared it.
    SetGoal { key: u64, goal: Option<Goal> },
    /// The human answered what was proposed at `index` in the chat `key`.
    AnswerProposal {
        key: u64,
        index: usize,
        answer: Answer,
    },
    /// The human changed the goal proposed at `index` in the chat `key`
    /// into `goal`, and set it.
    ChangeProposal { key: u64, index: usize, goal: Goal },
    /// The human chose, on the card at `index` of the chat `key`, whether
    /// to keep what a chat learned in the wiki.
    AnswerKeep { key: u64, index: usize, keep: bool },
    /// The human opened a sketch beside the chat `key`, from `sketch`, to
    /// try `what`.
    StartSketch {
        key: u64,
        sketch: Sketch,
        what: String,
    },
    /// The human set the setting `id` of the chat `key`'s session to
    /// `value`.
    Set { key: u64, id: String, value: String },
    /// The human switched the chat `key` to the agent kept as `agent`.
    SwitchAgent { key: u64, agent: String },
    /// The human named the chat `key`.
    Rename { key: u64, name: String },
    /// The human put the chat `key` away, or brought it back.
    Archive { key: u64, archived: bool },
    /// The human marked the chat `key` done, which closes its agent, or
    /// reopened it.
    Resolve { key: u64, resolved: bool },
    /// The human asked the agent of the chat `key` to keep what it
    /// learned in the wiki.
    SaveToWiki { key: u64 },
    /// The human, done looking at what the agent of the chat `key` made,
    /// asked it to merge it.
    Merge { key: u64 },
    /// The human ended the sketch `key` as `ending` says.
    EndSketch {
        key: u64,
        ending: crate::workflow::Ending,
    },
    /// The window shows the stage of `folder` with no chat of it: tell it
    /// the tabs the stage had (see [`FromDaemon::StageOf`]).
    StageOf { folder: PathBuf },
    /// The stage of `folder`, where its chats work, has these tabs now,
    /// each of a chat by its key.
    Tabs {
        folder: PathBuf,
        tabs: KeptTabs<u64>,
    },
    /// The command `id` ran as `ran` says.
    Ran { id: u64, ran: Result<Ran, String> },
    /// What the agent sees of the Mac's windows, or of the Android
    /// devices, once the act `id` is done.
    Seen { id: u64, seen: Result<Seen, String> },
    /// The relay of the stage asked for as `id` listens on `port`, or did
    /// not start.
    Prepared { id: u64, port: Option<u16> },
    /// Cox's own chats opened from now on work in `home`, as the
    /// human set it, or as the settings say now when `set` is false.
    Home { home: PathBuf, set: bool },
    /// Unpair every phone: make the secret a phone pairs with again, so
    /// that each pairs again, with the code the settings show then.
    UnpairPhones,
    /// Rename the project kept as `project` `name`, and its folder with
    /// it, unless one of its chats is at work.
    RenameProject { project: ProjectId, name: String },
    /// The human set what `project` is, or, blank, that it says nothing
    /// of what it is.
    SetAbout { project: ProjectId, about: String },
    /// The human chose to compare the directory `directory` of the project
    /// in `folder`, '' for the project's folder, with the branch `base`, in
    /// what changed, or, with none, with the one Cox finds.
    CompareWith {
        folder: PathBuf,
        directory: String,
        base: Option<String>,
    },
    /// Put the project kept as `project` away, off the rail, its chats
    /// going on as they were, or bring it back.
    ArchiveProject { project: ProjectId, archived: bool },
    /// Delete the project kept as `project`: its folder goes to the
    /// trash, and its chats stop and are forgotten, unless one of them is
    /// at work.
    DeleteProject { project: ProjectId },
    /// Answer [`FromDaemon::Settled`] with `id` once all asked before is
    /// done.
    Sync { id: u64 },
    /// Answer [`FromDaemon::Map`] with `id`: the `part` a page asks for
    /// of the map of the project in `folder` named `name`, or its first,
    /// making the indexes of its code that are not made yet, and again
    /// those that failed, when `again` (see [`crate::map::Maps::told`]).
    Map {
        id: u64,
        folder: PathBuf,
        name: Option<String>,
        again: bool,
        #[serde(default)]
        part: crate::map::Part,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coxd_takes_a_window_that_speaks_alike_and_stops_for_another_or_when_asked() {
        use Asks::{Join, Restart, RestartNow};
        use FromDaemon::{Busy, Joined, Outdated};
        let coxd = Version {
            protocol: 7,
            daemon: 1,
        };
        let newer = Some(Version {
            protocol: 7,
            daemon: 2,
        });
        let unlike = Some(Version {
            protocol: 8,
            daemon: 1,
        });
        let table = [
            (Some(coxd), Join, false, Joined(coxd)),
            (newer, Join, false, Joined(coxd)),
            (newer, Join, true, Joined(coxd)),
            (unlike, Join, true, Busy),
            (unlike, Join, false, Outdated),
            (None, Join, true, Busy),
            (None, Join, false, Outdated),
            (newer, Restart, true, Busy),
            (newer, Restart, false, Outdated),
            (unlike, Restart, true, Busy),
            (newer, RestartNow, true, Outdated),
            (unlike, RestartNow, true, Outdated),
        ];
        for (version, asks, at_work, answer) in table {
            let hello = Hello {
                version,
                asks,
                ..Hello::default()
            };
            assert_eq!(
                hello.answer(coxd, at_work),
                answer,
                "{hello:?} to coxd of {coxd:?}, at work: {at_work}"
            );
        }
    }

    #[test]
    fn a_build_runs_in_place_of_another_with_the_window_alone_while_it_speaks_alike() {
        let running = Version {
            protocol: 7,
            daemon: 1,
        };
        let coxd_changed = Version {
            protocol: 7,
            daemon: 2,
        };
        let protocol_changed = Version {
            protocol: 8,
            daemon: 2,
        };
        assert_eq!(running.restart(running), Restart::WindowOnly);
        assert_eq!(coxd_changed.restart(running), Restart::WindowOnly);
        assert_eq!(protocol_changed.restart(running), Restart::WithCoxd);
    }

    #[test]
    fn what_runs_in_the_background_is_told_by_its_kind() {
        let background = |kind, name: &str, description: Option<&str>| Background {
            kind,
            name: name.to_owned(),
            description: description.map(str::to_owned),
        };
        let all = [
            background(BackgroundKind::Command, "sleep 120", None),
            background(
                BackgroundKind::Turn,
                "Claude is working",
                Some("The command ended"),
            ),
            background(BackgroundKind::ToolCall, "Read log", None),
            background(BackgroundKind::Task("monitor".to_owned()), "CI", None),
        ];
        let what: Vec<&str> = all.iter().map(Background::what).collect();
        assert_eq!(what, ["sleep 120", "The command ended", "Read log", "CI"]);
        let labeled: Vec<String> = all.iter().map(Background::labeled).collect();
        assert_eq!(
            labeled,
            [
                "command \"sleep 120\"",
                "its own turn (The command ended)",
                "tool call \"Read log\"",
                "monitor \"CI\"",
            ]
        );
        assert!(all[0].is_command() && !all[1].is_command());
        assert_eq!(
            told(&all),
            "the background command \"sleep 120\"; a turn it took by itself (The command ended); the tool \
             call \"Read log\"; the monitor \"CI\""
        );
        assert!(in_background(State::Idle, &all));
        assert!(
            !in_background(State::Busy, &all),
            "a turn is at work as it is"
        );
        assert!(!in_background(State::Idle, &[]));
    }

    #[test]
    fn a_chat_row_of_a_build_that_told_no_background_reads() {
        let row = ChatRow {
            key: 1,
            project: 0,
            kept: None,
            name: "chat".to_owned(),
            named: Named::FirstLine,
            open: true,
            again: false,
            state: State::Idle,
            unread: false,
            goal: None,
            parent: None,
            setup: None,
            ending: false,
            agent: "claude".to_owned(),
            folder: None,
            background: Vec::new(),
            pulls: Vec::new(),
            spent: None,
            resolved: None,
            steers: false,
            on_steps: Vec::new(),
            active: 0,
            cut_off: None,
        };
        let line = serde_json::to_string(&row).unwrap();
        assert!(!line.contains("background"), "{line}");
        assert!(!line.contains("pulls"), "{line}");
        assert!(!line.contains("resolved"), "{line}");
        assert!(!line.contains("on_steps"), "{line}");
        assert!(!line.contains("cut_off"), "{line}");
        assert_eq!(serde_json::from_str::<ChatRow>(&line).unwrap(), row);
    }

    #[test]
    fn a_window_and_a_coxd_of_a_build_that_told_none_read_each_other_greeting() {
        let hello: Hello = serde_json::from_str(r#"{"home":null,"command":[]}"#).unwrap();
        assert_eq!(hello.version, None);
        assert_eq!(hello.asks, Asks::Join);
        assert!(
            !hello.reads_at_work,
            "a window of a build that does not read which chats are at work hears Busy"
        );
        let older: Hello = serde_json::from_str(
            r#"{"home":null,"command":[],"build":{"size":1,"modified":{"secs_since_epoch":0,"nanos_since_epoch":0}},"speaks":7}"#,
        )
        .unwrap();
        assert_eq!(
            older.version, None,
            "a window of a build that told its build and speaks"
        );
        for (line, answer) in [
            (r#""Welcome""#, FromDaemon::Welcome),
            (r#""Busy""#, FromDaemon::Busy),
            (r#""Outdated""#, FromDaemon::Outdated),
            (r#""Taken""#, FromDaemon::Taken),
        ] {
            assert_eq!(serde_json::from_str::<FromDaemon>(line).unwrap(), answer);
        }
        let told = Hello::new(None, Vec::new(), Asks::Restart);
        let line = serde_json::to_string(&told).unwrap();
        assert_eq!(serde_json::from_str::<Hello>(&line).unwrap(), told);
        assert_eq!(told.version, Some(VERSION), "{line}");
        assert!(told.reads_at_work, "{line}");
        assert!(
            line.contains(&format!("{:016x}", VERSION.protocol)),
            "{line}"
        );
    }
}
