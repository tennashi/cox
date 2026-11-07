//! What the daemon and a window say to each other: what the human does in
//! the window, [`ToDaemon`], and what the daemon tells the window,
//! [`FromDaemon`]. Both go as JSON, a message a line, once the window
//! talks to a daemon of its own process (see [`crate::daemon`]).

use std::path::PathBuf;

use cox_core::entry::Attachment;
use cox_core::goal::Goal;
use cox_core::store::{ChatId, Named, ProjectId};
use cox_core::tabs::{KeptTabs, Lines};
use serde::{Deserialize, Serialize};

use crate::agent::mcp::{AndroidAct, MacAct, Ran, Seen};
use crate::agent::setup::Setup;
use crate::chat::conversation::{Answer, Patch, State};
use crate::chat::sketch::Sketch;

/// A project, as the window lists it on its rail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRow {
    /// How it is kept, or `None` for Cox's own.
    pub id: Option<ProjectId>,
    pub name: String,
    /// The folder its chats work in.
    pub folder: PathBuf,
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
    /// Whether a turn ended that the human has not seen.
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
}

/// What the daemon tells a window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FromDaemon {
    /// The window's [`Hello`] was heard: what follows is the daemon's.
    Welcome,
    /// Another window is there already, which the daemon goes on with.
    Taken,
    /// The daemon's own program was replaced, as by `cargo xtask
    /// install`, and no chat is at work: it stops, for the window to
    /// start the new one.
    Outdated,
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
    /// The chat `key`, named `name`, comes to wait for the human, or ended
    /// a turn, as `body` says, for the window to tell the human, unless
    /// they look at it.
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
}

/// What a window tells the daemon: what the human did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToDaemon {
    /// Open a chat in the project of `folder`, or of Cox's own, and show
    /// it.
    Open { folder: Option<PathBuf> },
    /// The window shows the chat `key`, or none.
    Shown { key: Option<u64> },
    /// The window shows the conversation of the chat `key` from now on:
    /// tell it how it changes.
    Watch { key: u64 },
    /// The human sent `text` in the chat `key`, with what they handed it
    /// from elsewhere on the screen.
    Submit {
        key: u64,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attached: Vec<Attachment>,
    },
    /// The human chose the option at `index` of what the agent of the
    /// chat `key` asks: of a permission request, or of the ways to log in.
    Choose { key: u64, index: usize },
    /// The human stopped the chat `key`'s turn.
    Stop { key: u64 },
    /// The human set the goal of the chat `key`, or cleared it.
    SetGoal { key: u64, goal: Option<Goal> },
    /// The human answered the goal proposed at `index` in the chat `key`.
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
    /// The human asked the agent of the chat `key` to keep what it
    /// learned in the wiki.
    SaveToWiki { key: u64 },
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
    /// Answer [`FromDaemon::Settled`] with `id` once all asked before is
    /// done.
    Sync { id: u64 },
}
