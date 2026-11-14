//! What the daemon and a window say to each other: what the human does in
//! the window, [`ToDaemon`], and what the daemon tells the window,
//! [`FromDaemon`]. Both go as JSON, a message a line, once the window
//! talks to a daemon of its own process (see [`crate::daemon`]).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use cox_core::entry::Attachment;
use cox_core::goal::Goal;
use cox_core::store::{ChatId, Named, ProjectId};
use cox_core::tabs::{KeptTabs, Lines};
use serde::{Deserialize, Serialize};

use crate::chat::conversation::{Answer, Patch, State};
use coxd::agent::mcp::{AndroidAct, MacAct, Ran, Seen};
use coxd::agent::setup::Setup;
use coxd::chat::sketch::Sketch;

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
    /// The folder it works in, when it is not its project's, as one that
    /// opened before another folder of the project became the project's.
    #[serde(default)]
    pub folder: Option<PathBuf>,
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
}

/// What an agent's tools ask of its chat's stage, as Cox's
/// [`ToolRequest`](coxd::agent::mcp::ToolRequest) says it, with a number
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

/// Which build of Cox a window or coxd runs, as the file of its program
/// was when it started: a window and a coxd of one build say the same
/// [`ToDaemon`] and [`FromDaemon`], and of two builds they may not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    size: u64,
    modified: SystemTime,
}

impl Build {
    /// The build this process runs, unless the file of its program
    /// cannot be read.
    #[must_use]
    pub fn now() -> Option<Self> {
        let metadata = std::fs::metadata(std::env::current_exe().ok()?).ok()?;
        Some(Self {
            size: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }
}

/// What a window and coxd of this build say to each other, as a hash of
/// the sources that define it (see `build.rs`): a window and a coxd of
/// two builds that speak alike read each other.
pub const SPEAKS: u64 = match u64::from_str_radix(env!("COX_SPEAKS"), 16) {
    Ok(speaks) => speaks,
    Err(_) => panic!("build.rs tells COX_SPEAKS in hex"),
};

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
    /// The window's build, or none from a window of a build that did not
    /// tell it.
    #[serde(default)]
    pub build: Option<Build>,
    /// What the window's build says to coxd (see [`SPEAKS`]), or none
    /// from a window of a build that did not tell it.
    #[serde(default)]
    pub speaks: Option<u64>,
}

impl Hello {
    /// What a window of this build says first, with `home` and `command`
    /// from its command line.
    #[must_use]
    pub fn new(home: Option<PathBuf>, command: Vec<String>) -> Self {
        Self {
            home,
            command,
            build: Build::now(),
            speaks: Some(SPEAKS),
        }
    }

    /// How coxd of `build`, which `speaks` so, answers this hello from a
    /// window it could take, as a chat of its is `at_work` or not: it
    /// takes a window of its own build, and one of another that speaks
    /// as it does while a chat is at work, and stops for any other once
    /// no chat is at work.
    #[must_use]
    pub fn answer(&self, build: Option<Build>, speaks: Option<u64>, at_work: bool) -> FromDaemon {
        if self.build == build || (at_work && self.speaks.is_some() && self.speaks == speaks) {
            FromDaemon::Joined
        } else if at_work {
            FromDaemon::Busy
        } else {
            FromDaemon::Outdated
        }
    }
}

/// What the daemon tells a window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FromDaemon {
    /// The window's [`Hello`] was heard, from a window of the daemon's
    /// build, or of one that speaks as it does: what follows is the
    /// daemon's.
    Joined,
    /// What a coxd of a build that did not tell builds apart answers a
    /// window it takes, of any build: a window takes it as
    /// [`FromDaemon::Busy`], since what follows may not read.
    Welcome,
    /// Another window is there already, which the daemon goes on with.
    Taken,
    /// The window is of another build than the daemon's, as when `cargo
    /// xtask install` replaced its program, and no chat is at work: the
    /// daemon stops, for the window to start one of its own build.
    Outdated,
    /// The window is of another build than the daemon's, which speaks
    /// otherwise, and the daemon goes on as it is, since a chat is at
    /// work: it does not take the window, which comes again later.
    Busy,
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
    /// The human asked the agent of the chat `key` to keep what it
    /// learned in the wiki.
    SaveToWiki { key: u64 },
    /// The human, done looking at what the agent of the chat `key` made,
    /// asked it to merge it.
    Merge { key: u64 },
    /// The human ended the sketch `key` as `ending` says.
    EndSketch {
        key: u64,
        ending: coxd::workflow::Ending,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(size: u64) -> Build {
        Build {
            size,
            modified: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn coxd_takes_a_window_of_its_build_or_one_that_speaks_alike_while_at_work() {
        use FromDaemon::{Busy, Joined, Outdated};
        let ours = Some(built(1));
        let theirs = Some(built(2));
        let alike = Some(7);
        let unlike = Some(8);
        let table = [
            (ours, alike, ours, alike, false, Joined),
            (ours, alike, ours, alike, true, Joined),
            (ours, None, ours, alike, true, Joined),
            (theirs, alike, ours, alike, true, Joined),
            (theirs, alike, ours, alike, false, Outdated),
            (theirs, unlike, ours, alike, true, Busy),
            (theirs, unlike, ours, alike, false, Outdated),
            (theirs, None, ours, alike, true, Busy),
            (theirs, None, ours, alike, false, Outdated),
            (theirs, alike, ours, None, true, Busy),
            (theirs, None, ours, None, true, Busy),
            (None, None, ours, alike, true, Busy),
            (None, None, ours, alike, false, Outdated),
        ];
        for (build, speaks, coxd_build, coxd_speaks, at_work, answer) in table {
            let hello = Hello {
                build,
                speaks,
                ..Hello::default()
            };
            assert_eq!(
                hello.answer(coxd_build, coxd_speaks, at_work),
                answer,
                "{hello:?} to coxd of {coxd_build:?} speaking {coxd_speaks:?}, at work: {at_work}"
            );
        }
    }

    #[test]
    fn a_window_and_a_coxd_of_a_build_that_told_none_read_each_other_greeting() {
        let hello: Hello = serde_json::from_str(r#"{"home":null,"command":[]}"#).unwrap();
        assert_eq!(hello.build, None);
        assert_eq!(hello.speaks, None);
        assert_eq!(
            serde_json::from_str::<FromDaemon>(r#""Welcome""#).unwrap(),
            FromDaemon::Welcome
        );
        let told = Hello::new(None, Vec::new());
        let line = serde_json::to_string(&told).unwrap();
        assert_eq!(serde_json::from_str::<Hello>(&line).unwrap(), told);
        assert!(told.build.is_some(), "{line}");
        assert_eq!(told.speaks, Some(SPEAKS), "{line}");
    }
}
