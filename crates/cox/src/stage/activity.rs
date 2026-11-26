//! What goes on in a folder, as the stage's Activity page shows it, at
//! `cox://api/activity` (see [`crate::stage::plugins`]): the human's
//! inbox, where they look first when they open Cox or have a moment.
//!
//! On top, how it all stands: how many chats wait for the human, how many
//! are at work, and how many steps got done over each of [`WINDOWS`], as
//! the human picks one (see [`summaries`]). Then the chats,
//! one line each: those that ask the human something, with what they ask,
//! or whose turn was cut off, as by a usage limit, with why, which a
//! button has go on (see [`resume`]), or whose turn ended while they were
//! not looking, or CI ended on a pull request their agent opened (see
//! [`waiting_in`]); and those whose
//! agents take a turn, or work in the background once it ended, or wait
//! on CI, or whose plan, as the agent last sent it over ACP, is not done,
//! or was done, or marked done, lately (see [`planned`]). Each tells how
//! it stands, the steps of its plan, how far it went, what it does now,
//! or runs in the background, and its open pull requests; under it, the
//! subagents' chats its agent opened, and theirs, at any depth, each
//! under the step it came from, if it tells (see [`linked`]), with the
//! chats above them though these are not listed on their own (see
//! [`tree_of`]). The home, where Cox lands, tells of every folder. The
//! stage of a folder a chat works in apart, as a sketch or a worktree,
//! tells of the whole project, as the project's does, whichever of its
//! chats the window shows (see [`project_of`]), and the chat the window
//! shows is marked there.
//!
//! Then the pull requests the project holds, whichever of its chats'
//! agents opened them, and those of every project in the home (see
//! [`pulls_in`]): each open one, with how its CI stands, check by check,
//! what its reviews decided, whether it conflicts, how much it changes,
//! and the chat that opened it, and, once its CI failed, a button that
//! has that chat fix it (see [`fix`]); one merged or closed shows as
//! long as a chat done does.
//!
//! A chat's story, what it did when, is read for the chat alone, once the
//! human opens its line (see [`timeline`]), and the thread of a step of
//! its plan once they open the step (see [`thread`]), where a line they
//! write goes to the chat (see [`say`]).
//!
//! The window tells this module how its chats stand as they change (see
//! [`tell`]), since the stage's pages are served apart from it; the bar
//! counts what waits on the Activity's button (see [`waiting`]). What a
//! chat does now, and what it asks, come from what the window told of its
//! conversation (see [`commands::now`]). The plans are read from the chats
//! as coxd keeps them (see [`Store::plans`]), so they hold for chats the
//! window never showed, and last as long as their conversations do, each
//! time the page reads. A plan the human dismissed from the page stays
//! hidden until the chat's agent sends another (see [`dismiss`]).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use cox_core::entry::{OnStep, Step, StepState, StepsDone, alike};
use cox_core::store::{COXD_WENT, ChatId, Ended, KeptPlan, Progress, Said, Store, TurnEnd};
use jiff::tz::TimeZone;
use serde_json::{Value, json};

use crate::stage::commands;

mod timeline;
use cox_acp::Microdollars;
use coxd::chat::spent::Spent;
use coxd::daemon::wire::{OnStepLine, PullRow};
use coxd::github::CheckState;
pub use timeline::{Of, OfCommand, command, timeline};

/// The windows the page counts the steps done in, each by its name and
/// its length in milliseconds, the shortest first.
const WINDOWS: [(&str, i64); 3] = [
    ("15 min", 15 * 60 * 1000),
    ("1 h", 60 * 60 * 1000),
    ("4 h", 4 * 60 * 60 * 1000),
];
/// How long, in milliseconds, a chat that is done shows as done: one
/// whose plan is done, after its agent last did anything, and one marked
/// done, after it was, for the human to see what came of it. As long as
/// the longest of [`WINDOWS`], so that what the page sums up as done
/// there is on the page too.
const DONE_SHOWS: i64 = WINDOWS[WINDOWS.len() - 1].1;

/// A chat as the Activity page is told of it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "how a chat stands, each apart"
)]
pub struct Chat {
    /// The number the window goes by for it this run, which a click on
    /// it, as `cox://chat/<key>`, shows.
    pub key: u64,
    pub name: String,
    /// Where it works.
    pub cwd: PathBuf,
    /// The folder of its project, which a sketch's differs from.
    pub project: PathBuf,
    /// Whether it asks the human something.
    pub asking: bool,
    /// Whether its turn ended while the human was not looking.
    pub unread: bool,
    /// Its goal, as the human set it, if it has one.
    pub goal: Option<String>,
    /// Whether its agent takes a turn, or stops one, or works in the
    /// background once its turn ended.
    pub working: bool,
    /// What its agent runs in the background, once its turn ended.
    pub background: Vec<coxd::daemon::wire::Background>,
    /// The key of the chat whose agent opened it, if one did.
    pub parent: Option<u64>,
    /// How coxd keeps it, which its plan and its story are read by.
    pub kept: Option<ChatId>,
    /// The pull requests its agent opened, as coxd last read them.
    pub pulls: Vec<PullRow>,
    /// What its agent spent, once it told any of it.
    pub spent: Option<Spent>,
    /// Whether it is marked done: it leaves the page, but for a step of
    /// the plan of the chat that opened it, and what it spent there.
    pub resolved: bool,
    /// Whether it is marked done as its report reached the agent that
    /// opened it.
    pub reported: bool,
    /// Whether the human archived it: it waits for them only while it
    /// asks.
    pub archived: bool,
    /// Whether a line sent now goes into the turn its agent takes, as it
    /// asks the human something too, if it does.
    pub steers: bool,
    /// The lines written on steps of its plan that wait for its turn to
    /// end, the first first.
    pub on_steps: Vec<OnStepLine>,
    /// When it last moved, in milliseconds since the epoch.
    pub active: i64,
    /// How its last turn was cut off, while it waits to go on (see
    /// [`Ended::cuts_off`]).
    pub cut_off: Option<TurnEnd>,
}

impl Chat {
    /// Whether it waits for the human, as the bell counts it.
    fn waits(&self) -> bool {
        !self.resolved
            && (self.asking
                || (!self.archived
                    && (self.unread || self.cut().is_some() || !self.ci_ended().is_empty())))
    }

    /// How its last turn was cut off, while it is not at work again, nor
    /// marked done.
    fn cut(&self) -> Option<&TurnEnd> {
        self.cut_off
            .as_ref()
            .filter(|_| !self.working && !self.resolved)
    }

    /// Whether the page may have it go on from where its turn was cut off:
    /// cut off, and not asking the human anything.
    fn resumes(&self) -> bool {
        self.cut().is_some() && !self.asking
    }

    /// Whether CI runs on a pull request its agent opened, which it waits
    /// on, though its turn ended.
    fn waits_on_ci(&self) -> bool {
        self.pulls.iter().any(PullRow::waits_on_ci)
    }

    /// How CI ended on each pull request its agent opened whose CI ended
    /// while the human did not look at it, as "CI passed on #423".
    fn ci_ended(&self) -> Vec<String> {
        self.pulls
            .iter()
            .filter_map(|row| {
                let outcome = match row.unseen? {
                    CheckState::Passed => "passed",
                    CheckState::Failed => "failed",
                    CheckState::Pending | CheckState::Skipped => "ended",
                };
                Some(format!("CI {outcome} on #{}", row.pull.number))
            })
            .collect()
    }
}

/// The pull requests of `chat` the page shows, those still open, as JSON:
/// each with its number, title, how it stands, and how its CI does.
fn pulls_of(chat: &Chat) -> Vec<Value> {
    chat.pulls
        .iter()
        .filter_map(|row| {
            let status = row.status.as_ref().filter(|status| !status.done())?;
            let ci = status.checks().map(|checks| match checks {
                CheckState::Pending => "running",
                CheckState::Passed => "passed",
                CheckState::Failed => "failed",
                CheckState::Skipped => "skipped",
            });
            Some(json!({
                "number": row.pull.number,
                "title": status.title,
                "draft": status.state == coxd::github::PullState::Draft,
                "ci": ci,
            }))
        })
        .collect()
}

/// A pull request a project holds, as the Activity page is told of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pull {
    /// The folder of its project.
    pub project: PathBuf,
    pub pull: coxd::github::Pull,
    /// How it stood when coxd last read it, once it read it.
    pub status: Option<coxd::github::Status>,
    /// The chat whose agent opened it, by the number the window goes by
    /// for it, while the window lists it.
    pub author: Option<u64>,
}

impl Pull {
    /// Whether it is merged or closed.
    fn done(&self) -> bool {
        self.status.as_ref().is_some_and(coxd::github::Status::done)
    }

    /// Whether its CI failed, while it is open.
    fn failed(&self) -> bool {
        !self.done()
            && self.status.as_ref().and_then(coxd::github::Status::checks)
                == Some(CheckState::Failed)
    }

    /// The names of its checks that failed.
    fn failed_checks(&self) -> Vec<&str> {
        self.status
            .iter()
            .flat_map(|status| &status.checks)
            .filter(|check| check.state == CheckState::Failed)
            .map(|check| check.name.as_str())
            .collect()
    }

    /// What its author is told to fix its CI that failed, as the human
    /// would ask it.
    fn fix_line(&self) -> String {
        let checks = self.failed_checks();
        let checks = if checks.is_empty() {
            String::new()
        } else {
            format!(": {}", checks.join(", "))
        };
        format!(
            "CI failed on #{} ({}){checks}. Fix it.",
            self.pull.number,
            self.pull.url()
        )
    }
}

/// The pull requests the page of the folder `root` tells of, as JSON: of
/// its project, or of every project in the home, the first seen first,
/// those open, and those merged or closed within [`DONE_SHOWS`] of `at`,
/// in milliseconds since the epoch. Each with its address, number,
/// title, how it stands, how its CI does, how many of its checks stand
/// each way and the names of those that failed, what its reviews decided,
/// whether it conflicts with its base, how much it changes, when it was
/// opened and merged or closed, the chat that opened it, and whether that
/// chat may be asked to fix it.
fn pulls_in(told: &Told, root: &Path, at: i64) -> Vec<Value> {
    told.pulls
        .iter()
        .filter(|pull| root == told.home || pull.project == root)
        .filter(|pull| {
            let Some(status) = pull.status.as_ref().filter(|status| status.done()) else {
                return true;
            };
            status
                .times
                .merged
                .or(status.times.closed)
                .is_some_and(|ended| at - ended < DONE_SHOWS)
        })
        .map(|pull| {
            let status = pull.status.as_ref();
            let state = status.map(|status| match status.state {
                coxd::github::PullState::Open => "open",
                coxd::github::PullState::Draft => "draft",
                coxd::github::PullState::Merged => "merged",
                coxd::github::PullState::Closed => "closed",
            });
            let ci = status
                .filter(|status| !status.done())
                .and_then(coxd::github::Status::checks)
                .map(|checks| match checks {
                    CheckState::Pending => "running",
                    CheckState::Passed => "passed",
                    CheckState::Failed => "failed",
                    CheckState::Skipped => "skipped",
                });
            let author = pull
                .author
                .and_then(|key| told.chats.iter().find(|chat| chat.key == key));
            json!({
                "repo": pull.pull.repo,
                "number": pull.pull.number,
                "url": pull.pull.url(),
                "title": status.map(|status| status.title.as_str()),
                "state": state,
                "ci": ci,
                "failed": pull.failed_checks(),
                "checks": status.map(checks_of),
                "review": status.and_then(|status| status.review).map(|review| match review {
                    coxd::github::Review::Approved => "approved",
                    coxd::github::Review::ChangesRequested => "changes",
                    coxd::github::Review::Required => "required",
                }),
                "conflicts": status.is_some_and(|status| status.mergeable == coxd::github::Mergeable::Conflicting),
                "size": status.and_then(|status| status.size).map(|size| json!({
                    "added": size.added,
                    "removed": size.removed,
                    "files": size.files,
                })),
                "opened": status.and_then(|status| status.times.opened),
                "ended": status.and_then(|status| status.times.merged.or(status.times.closed)),
                "where": where_of(told, &pull.project),
                "author": author.map(|chat| json!({
                    "key": chat.key,
                    "name": chat.name,
                    "resolved": chat.resolved,
                    "working": chat.working,
                })),
                "fix": author.is_some() && pull.failed(),
            })
        })
        .collect()
}

/// How many of the checks of `status` passed, failed, run or wait, and
/// were skipped, of how many, as JSON.
fn checks_of(status: &coxd::github::Status) -> Value {
    let count = |state| {
        status
            .checks
            .iter()
            .filter(|check| check.state == state)
            .count()
    };
    json!({
        "passed": count(CheckState::Passed),
        "failed": count(CheckState::Failed),
        "running": count(CheckState::Pending),
        "skipped": count(CheckState::Skipped),
        "total": status.checks.len(),
    })
}

/// How the window's chats stand, and its projects, by folder and name,
/// with the home, the folder of Cox's own chats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Told {
    pub chats: Vec<Chat>,
    pub projects: Vec<(PathBuf, String)>,
    pub home: PathBuf,
    /// Why no chat shows its pull requests, as coxd says, if that is so.
    pub github: Option<String>,
    /// The key of the chat the window shows, if it shows one, which the
    /// page marks wherever the chat is on it.
    pub shown: Option<u64>,
    /// The pull requests the projects hold, the first seen first.
    pub pulls: Vec<Pull>,
}

/// How the chats stood when the window last told.
static TOLD: Mutex<Option<Told>> = Mutex::new(None);

/// Keeps how the window's chats stand now, for the pages to read.
pub fn tell(told: Told) {
    if let Ok(mut now) = TOLD.lock() {
        *now = Some(told);
    }
}

/// How the chats stood when the window last told, or none of them.
fn now() -> Told {
    TOLD.lock()
        .ok()
        .and_then(|now| now.clone())
        .unwrap_or_default()
}

/// The window's projects, by folder and name, as it last told.
#[must_use]
pub fn projects() -> Vec<(PathBuf, String)> {
    now().projects
}

/// The home, the folder of Cox's own chats, as the window last told, if it
/// told.
#[must_use]
pub fn home() -> Option<PathBuf> {
    Some(now().home).filter(|home| !home.as_os_str().is_empty())
}

/// Whether `chat` is of the folder `root`, or of any, when `root` is the
/// home of `told`.
fn of_folder(told: &Told, chat: &Chat, root: &Path) -> bool {
    root == told.home || chat.cwd == root || chat.project == root
}

/// The folder the page of the stage of `root` tells of: that of the
/// project whose chats work in `root`, as in a sketch or a worktree of
/// it, so that the page tells the same, of every chat of the project,
/// whichever of them the window shows; `root` itself when it is the
/// folder of a project, or the home, or of none `told` knows.
fn project_of(told: &Told, root: &Path) -> PathBuf {
    let projects = || told.projects.iter().map(|(folder, _)| folder.as_path());
    if root == told.home || projects().any(|folder| folder == root) {
        return root.to_owned();
    }
    told.chats
        .iter()
        .find(|chat| chat.cwd == root)
        .map(|chat| chat.project.clone())
        // A worktree in the project's folder, whose chats are gone.
        .or_else(|| {
            projects()
                .filter(|folder| *folder != told.home && root.starts_with(folder))
                .max_by_key(|folder| folder.components().count())
                .map(Path::to_owned)
        })
        .unwrap_or_else(|| root.to_owned())
}

/// Whether `chat` waits for the human and is of the folder `root`, or
/// of any, when `root` is the home of `told`.
fn waits_in(told: &Told, chat: &Chat, root: &Path) -> bool {
    chat.waits() && of_folder(told, chat, root)
}

/// How many chats wait for the human of those the page of the stage of
/// `root` tells of (see [`project_of`]), for the bar.
#[must_use]
pub fn waiting(root: &Path) -> usize {
    waiting_of(&now(), root)
}

fn waiting_of(told: &Told, root: &Path) -> usize {
    let root = project_of(told, root);
    told.chats
        .iter()
        .filter(|chat| waits_in(told, chat, &root))
        .count()
}

/// What goes on in the project of the folder `root`, as its stage's page
/// tells it (see [`project_of`]), as JSON, from how the chats stand now,
/// and what they do and ask, as the window told of their conversations.
#[must_use]
pub fn told(root: &Path) -> Value {
    let at = jiff::Timestamp::now().as_millisecond();
    let kept = plans();
    let opened = opened();
    told_of(
        &now(),
        root,
        &Plans {
            on: &on_steps(&kept),
            opened: &opened,
            kept: &kept,
            dismissed: &Dismissed::read(),
            moved: &progress(at - DONE_SHOWS),
            at,
        },
        &commands::now,
    )
}

/// The plan each chat's agent sent last, as coxd keeps it, by the chat.
fn plans() -> Vec<KeptPlan> {
    let plans = coxd::storage::path(coxd::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
        .and_then(|path| Store::read(&path)?.plans());
    plans.unwrap_or_else(|error| {
        tracing::warn!("the plans of the chats are not read: {error:#}");
        Vec::new()
    })
}

/// When each chat coxd keeps was opened, in milliseconds since the epoch.
fn opened() -> Vec<(ChatId, i64)> {
    let opened = coxd::storage::path(coxd::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
        .and_then(|path| Store::read(&path)?.opened());
    opened.unwrap_or_else(|error| {
        tracing::warn!("when the chats were opened is not read: {error:#}");
        Vec::new()
    })
}

/// How many lines the human wrote on each step of the plans of the chats
/// of `kept`, by the chat, as coxd keeps them.
fn on_steps(kept: &[KeptPlan]) -> Vec<(ChatId, Vec<(u64, usize)>)> {
    let read = coxd::storage::path(coxd::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
        .and_then(|path| {
            let store = Store::read(&path)?;
            kept.iter()
                .map(|plan| Ok((plan.chat, store.on_steps(plan.chat)?)))
                .collect::<anyhow::Result<Vec<_>>>()
        });
    read.unwrap_or_else(|error| {
        tracing::warn!("the lines on the steps of the plans are not read: {error:#}");
        Vec::new()
    })
}

/// A step of a chat's plan whose thread the page shows: the chat, as
/// coxd keeps it, and the step, by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Open {
    chat: ChatId,
    step: u64,
}

/// The thread of the step `open` names, as JSON, with the step: each
/// line the human wrote on it and what the agent replied, with when, or
/// null when coxd keeps no plan of the chat with that step.
#[must_use]
pub fn thread(open: &Open) -> Value {
    let read = coxd::storage::path(coxd::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
        .and_then(|path| Store::read(&path)?.thread(open.chat, open.step));
    let said = read.unwrap_or_else(|error| {
        tracing::warn!("the thread of a step is not read: {error:#}");
        Vec::new()
    });
    let mut thread = thread_of(open, &said, &TimeZone::system());
    thread["waiting"] = json!(waiting_on(&now(), open));
    thread
}

/// The lines written on the step `open` names that wait for the turn of
/// its chat, of `told`, to end.
fn waiting_on(told: &Told, open: &Open) -> Vec<String> {
    told.chats
        .iter()
        .filter(|chat| chat.kept == Some(open.chat))
        .flat_map(|chat| &chat.on_steps)
        .filter(|line| line.step.id == open.step)
        .map(|line| line.text.clone())
        .collect()
}

/// The thread of the step `open` names, of the lines `said`, as JSON,
/// each with when, by the time of day in `zone`.
fn thread_of(open: &Open, said: &[Said], zone: &TimeZone) -> Value {
    json!({
        "chat": open.chat,
        "step": open.step,
        "said": said
            .iter()
            .map(|said| {
                json!({
                    "by": said.by,
                    "text": said.text,
                    "at": jiff::Timestamp::from_millisecond(said.at)
                        .map_or_default(|at| at.to_zoned(zone.clone()).strftime("%H:%M").to_string()),
                })
            })
            .collect::<Vec<_>>(),
    })
}

/// A line the human wrote on a step of a chat's plan, or asked a chat to
/// fix its CI, or to go on from where its turn was cut off, with, from the
/// page, for the window to send the chat (see [`listen`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Say {
    /// The chat, by the number the window goes by for it.
    pub key: u64,
    pub text: String,
    /// The step it is written on, if it is: a line that has a chat fix
    /// its CI, or go on, is on none.
    pub step: Option<OnStep>,
}

/// Where the lines written on steps go, to the window that listens.
static SAYS: Mutex<Option<tokio::sync::mpsc::UnboundedSender<Say>>> = Mutex::new(None);

/// The lines the human writes on steps of the plans from now on, for the
/// window to send each to its chat.
pub fn listen() -> tokio::sync::mpsc::UnboundedReceiver<Say> {
    let (says, heard) = tokio::sync::mpsc::unbounded_channel();
    if let Ok(mut now) = SAYS.lock() {
        *now = Some(says);
    }
    heard
}

/// What the page asks to say on a step: the line, the chat, by the key
/// the window goes by for it and as coxd keeps it, and the step, by its
/// id.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Asked {
    key: u64,
    #[serde(flatten)]
    open: Open,
    text: String,
}

/// Has the window send the line `asked` on its step, as the JSON of
/// [`Asked`] the page sent says, if the chat takes a line now and its
/// plan, as coxd keeps it, has the step; and answers the step's thread
/// to show, if the page named one.
pub fn say(asked: &str) -> Option<Open> {
    let asked: Asked = serde_json::from_str(asked).ok()?;
    let told = now();
    let says = says_on(&told, &plans(), &asked);
    if let Some(say) = says
        && let Ok(listening) = SAYS.lock()
        && let Some(listening) = listening.as_ref()
    {
        let _ = listening.send(say);
    }
    Some(asked.open)
}

/// The line `asked` to send, on the step of the chat's plan of `kept` it
/// names, as it reads there, when the chat of `told` takes a line on a
/// step (see [`takes_a_line`]).
fn says_on(told: &Told, kept: &[KeptPlan], asked: &Asked) -> Option<Say> {
    let text = asked.text.trim();
    let chat = told
        .chats
        .iter()
        .find(|chat| chat.key == asked.key && chat.kept == Some(asked.open.chat))?;
    if text.is_empty() || !takes_a_line(chat) {
        return None;
    }
    let step = kept
        .iter()
        .find(|plan| plan.chat == asked.open.chat)?
        .steps
        .iter()
        .find(|step| step.id == Some(asked.open.step))?;
    Some(Say {
        key: chat.key,
        text: text.to_owned(),
        step: Some(OnStep {
            id: asked.open.step,
            text: step.text.clone(),
        }),
    })
}

/// What the page asks a chat to fix: the CI that failed on the pull
/// request `number` of `repo`, which its agent opened, the chat by the
/// key the window goes by for it.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Fix {
    key: u64,
    repo: String,
    number: u64,
}

/// Has the window send the chat that opened the pull request `asked`
/// names, as the JSON of [`Fix`] the page sent says, a line that asks it
/// to fix its CI, as the human would: a chat marked done is reopened by
/// it. Nothing is sent unless the pull request's CI failed, as last read,
/// and the chat is the one that opened it.
pub fn fix(asked: &str) {
    let Ok(asked) = serde_json::from_str::<Fix>(asked) else {
        return;
    };
    if let Some(say) = fixes(&now(), &asked)
        && let Ok(listening) = SAYS.lock()
        && let Some(listening) = listening.as_ref()
    {
        let _ = listening.send(say);
    }
}

/// The line that has the chat `asked` names fix the CI of its pull
/// request, of `told`, if it failed.
fn fixes(told: &Told, asked: &Fix) -> Option<Say> {
    let pull = told.pulls.iter().find(|pull| {
        pull.pull.repo == asked.repo
            && pull.pull.number == asked.number
            && pull.author == Some(asked.key)
    })?;
    if !pull.failed() || !told.chats.iter().any(|chat| chat.key == asked.key) {
        return None;
    }
    Some(Say {
        key: asked.key,
        text: pull.fix_line(),
        step: None,
    })
}

/// How `chat`'s last turn was cut off, as JSON, while it waits to go on:
/// how it ended, as coxd keeps it, when, and what the page says of it,
/// after "Cut off: " (see [`cut_off_says`]). Null when it was not.
fn cut_off_of(chat: &Chat) -> Value {
    chat.cut().map_or(Value::Null, |end| {
        json!({
            "how": end.how.word(),
            "at": end.at,
            "says": cut_off_says(end.how, end.why.as_deref()),
        })
    })
}

/// What a turn cut off as `how` says, for `why`, as its agent said it,
/// if it did, is told as, after "Cut off: ": what the agent said of a
/// limit, as when it resets, or why it failed, as it said.
pub(crate) fn cut_off_says(how: Ended, why: Option<&str>) -> String {
    let why = why.map(str::trim).filter(|why| !why.is_empty());
    match (how, why) {
        (Ended::Limited, Some(why)) => why.to_owned(),
        (Ended::Limited, None) => "usage limit reached".to_owned(),
        (Ended::Failed, Some(why)) => format!("the turn failed: {why}"),
        (Ended::Failed, None) => "the turn failed".to_owned(),
        (Ended::Closed, Some(COXD_WENT)) => COXD_WENT.to_owned(),
        (Ended::Closed, Some(why)) => format!("the agent closed: {why}"),
        (Ended::Closed, None) => "the agent closed".to_owned(),
        (Ended::TooLong, _) => "the agent's reply grew too long".to_owned(),
        (Ended::TooManyRequests, _) => {
            "the agent asked the model as often as a turn allows".to_owned()
        }
        (Ended::Done | Ended::Stopped | Ended::Refused | Ended::Other, _) => {
            "the turn ended, for a reason Cox does not know".to_owned()
        }
    }
}

/// The line the page sends a chat whose turn was cut off, for it to go
/// on, as the human would.
pub const RESUME: &str = "Go on from where you were cut off.";

/// What the page asks to have go on from where its turn was cut off: the
/// chat, by the key the window goes by for it.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Resume {
    key: u64,
}

/// Has the window send the chat `asked` names, as the JSON of [`Resume`]
/// the page sent says, [`RESUME`], as the human would: a closed chat's
/// agent starts again on its session for it. Nothing is sent unless the
/// chat's turn was cut off, and it is not at work, nor asks the human
/// anything.
pub fn resume(asked: &str) {
    let Ok(asked) = serde_json::from_str::<Resume>(asked) else {
        return;
    };
    if let Some(say) = resumes(&now(), &asked)
        && let Ok(listening) = SAYS.lock()
        && let Some(listening) = listening.as_ref()
    {
        let _ = listening.send(say);
    }
}

/// The line that has the chat `asked` names go on, of `told`, if its turn
/// was cut off and it may (see [`Chat::resumes`]).
fn resumes(told: &Told, asked: &Resume) -> Option<Say> {
    told.chats
        .iter()
        .find(|chat| chat.key == asked.key)
        .filter(|chat| chat.resumes())
        .map(|chat| Say {
            key: chat.key,
            text: RESUME.to_owned(),
            step: None,
        })
}

/// What moved in the chats coxd keeps since `since`, in milliseconds
/// since the epoch, as it keeps it.
fn progress(since: i64) -> Progress {
    let moved = coxd::storage::path(coxd::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
        .and_then(|path| Store::read(&path)?.progress(since));
    moved.unwrap_or_else(|error| {
        tracing::warn!("what moved in the chats is not read: {error:#}");
        Progress::default()
    })
}

/// A plan the human dismissed from the Plans tier, as they saw it: its
/// chat, the number of its entry there, and its steps, since the agent's
/// updates in the same turn keep the number. Hidden until the chat's last
/// plan is another.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Dismissed {
    chat: ChatId,
    number: usize,
    steps: Vec<Step>,
}

/// A plan the page asks to dismiss, by its chat and its entry's number.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
struct Dismiss {
    chat: ChatId,
    plan: usize,
}

impl Dismissed {
    /// Whether `plan` is the one dismissed.
    fn hides(&self, plan: &KeptPlan) -> bool {
        self.chat == plan.chat && self.number == plan.number && self.steps == plan.steps
    }

    /// The plans dismissed, as kept, or none when nothing is kept.
    fn read() -> Vec<Self> {
        coxd::storage::path(coxd::storage::Kept::DismissedPlans)
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Those of `was` that still hide a plan of `kept`, and those of
    /// `kept` that `asked` names, as the page saw them.
    fn now(was: Vec<Self>, asked: &[Dismiss], kept: &[KeptPlan]) -> Vec<Self> {
        let mut now: Vec<Self> = was
            .into_iter()
            .filter(|dismissed| kept.iter().any(|plan| dismissed.hides(plan)))
            .collect();
        for plan in kept {
            if asked
                .iter()
                .any(|asked| asked.chat == plan.chat && asked.plan == plan.number)
                && !now.iter().any(|dismissed| dismissed.hides(plan))
            {
                now.push(Self {
                    chat: plan.chat,
                    number: plan.number,
                    steps: plan.steps.clone(),
                });
            }
        }
        now
    }
}

/// Dismisses the plans `asked`, as the JSON list of [`Dismiss`] the page
/// sent says, for them to be hidden, across restarts too, until each
/// chat's agent sends another.
pub fn dismiss(asked: &str) {
    let Ok(asked) = serde_json::from_str::<Vec<Dismiss>>(asked) else {
        return;
    };
    let Some(path) = coxd::storage::path(coxd::storage::Kept::DismissedPlans) else {
        return;
    };
    let now = Dismissed::now(Dismissed::read(), &asked, &plans());
    if let Err(error) = coxd::storage::write_json(&path, &now) {
        tracing::warn!("the plans dismissed are not kept: {error}");
    }
}

/// How alike a step of a plan must be to a subagent's chat, by its name,
/// as [`alike`] measures it, for the chat to show under the step.
const LINKED: f64 = 0.3;
/// How long after a subagent's chat was opened, in milliseconds, a step
/// of the plan of the chat that opened it may start and still be the
/// step it came from: coxd may keep the plan the agent sent just before
/// it opened the chat after the chat.
const ALONGSIDE: i64 = 2_000;

/// The step of `plan` each of `subagents` came from, by its place in the
/// plan: the step under way when its chat was opened, as `opened` tells
/// when, or started [`ALONGSIDE`] it, the one whose text reads most alike
/// its name, once alike enough, then the one started nearest the time,
/// if more were; or else, as when the agent opened it between steps, the
/// step its name reads most alike, once alike enough.
fn linked(plan: &[Step], subagents: &[&Chat], opened: &[(ChatId, i64)]) -> Vec<Option<usize>> {
    subagents
        .iter()
        .map(|chat| {
            // How alike `step` reads to its name, once alike enough.
            let alike = |step: &Step| {
                Some(alike(&step.text, &chat.name))
                    .filter(|score| *score >= LINKED)
                    .unwrap_or_default()
            };
            let opened = chat
                .kept
                .and_then(|kept| opened.iter().find(|(id, _)| *id == kept))
                .map(|(_, at)| *at);
            opened
                .and_then(|at| {
                    plan.iter()
                        .enumerate()
                        .filter(|(_, step)| under_way(step, at))
                        .max_by(|(_, one), (_, other)| {
                            let apart =
                                |step: &Step| step.started.map(|started| (started - at).abs());
                            alike(one)
                                .total_cmp(&alike(other))
                                .then(apart(other).cmp(&apart(one)))
                        })
                        .map(|(at, _)| at)
                })
                .or_else(|| {
                    plan.iter()
                        .enumerate()
                        .map(|(at, step)| (at, alike(step)))
                        .filter(|(_, score)| *score > 0.0)
                        .max_by(|one, other| one.1.total_cmp(&other.1))
                        .map(|(at, _)| at)
                })
        })
        .collect()
}

/// Whether `step` was under way at `at`, in milliseconds since the
/// epoch: started by then, or [`ALONGSIDE`] it, and under way still, or
/// done since.
fn under_way(step: &Step, at: i64) -> bool {
    step.started
        .is_some_and(|started| started <= at + ALONGSIDE)
        && match step.state {
            StepState::InProgress => true,
            StepState::Completed => step.since.is_some_and(|done| at <= done),
            StepState::Pending => false,
        }
}

/// The plans each chat's agent sent last, those the human dismissed,
/// what moved in the chats over the longest of [`WINDOWS`], and when, in
/// milliseconds since the epoch, they are told.
struct Plans<'a> {
    kept: &'a [KeptPlan],
    /// How many lines the human wrote on each step of each chat's plans.
    on: &'a [(ChatId, Vec<(u64, usize)>)],
    /// When each chat was opened, which tells apart chats of one name.
    opened: &'a [(ChatId, i64)],
    dismissed: &'a [Dismissed],
    moved: &'a Progress,
    at: i64,
}

impl Plans<'_> {
    /// The plan `chat`'s agent sent last, unless the human dismissed it.
    fn of(&self, chat: &Chat) -> Option<&KeptPlan> {
        let plan = self.kept.iter().find(|plan| Some(plan.chat) == chat.kept)?;
        (!self.dismissed.iter().any(|dismissed| dismissed.hides(plan))).then_some(plan)
    }

    /// Whether the human dismissed the plan `chat`'s agent sent last.
    fn dismissed(&self, chat: &Chat) -> bool {
        self.kept
            .iter()
            .find(|plan| Some(plan.chat) == chat.kept)
            .is_some_and(|plan| self.dismissed.iter().any(|dismissed| dismissed.hides(plan)))
    }

    /// Whether `chat` is marked done, and was [`DONE_SHOWS`] ago or
    /// less, so that it shows as done, unless the human dismissed its
    /// plan.
    fn done_lately(&self, chat: &Chat) -> bool {
        chat.resolved
            && !self.dismissed(chat)
            && self
                .moved
                .resolved
                .iter()
                .any(|(id, resolved)| Some(*id) == chat.kept && self.at - resolved.at < DONE_SHOWS)
    }

    /// Whether `chat` shows on the page though it may be marked done: it
    /// is not, or it was lately.
    fn stays(&self, chat: &Chat) -> bool {
        !chat.resolved || self.done_lately(chat)
    }

    /// How many lines the human wrote on each step of `chat`'s plans.
    fn on_of(&self, chat: &Chat) -> &[(u64, usize)] {
        self.on
            .iter()
            .find(|(id, _)| Some(*id) == chat.kept)
            .map_or(&[][..], |(_, on)| on.as_slice())
    }
}

/// Whether a line the human writes on a step of `chat`'s plan goes, as
/// one typed in the chat goes: into the turn its agent takes, if it takes
/// lines so, or else once the turn ends; but not while the agent asks the
/// human something, unless it takes the line into the turn.
fn takes_a_line(chat: &Chat) -> bool {
    !chat.asking || chat.steers
}

/// The project of `folder`, by its name, as `told` says it.
fn name_of(told: &Told, folder: &Path) -> Option<String> {
    told.projects
        .iter()
        .find(|(project, _)| project == folder)
        .map(|(_, name)| name.clone())
}

/// Where `project` is, as the home tells it: by its name, or `~` for
/// the home's own.
fn where_of(told: &Told, project: &Path) -> Option<String> {
    if project == told.home {
        Some("~".to_owned())
    } else {
        name_of(told, project)
    }
}

/// The chats of the folder `root` that wait for the human, or of every
/// folder in the home, each with where it is, and what it asks, as
/// `now` says, as JSON.
fn waiting_in(told: &Told, root: &Path, now: &dyn Fn(u64) -> Option<commands::Now>) -> Vec<Value> {
    told.chats
        .iter()
        .filter(|chat| waits_in(told, chat, root))
        .map(|chat| {
            json!({
                "key": chat.key,
                "chat": chat.kept,
                "name": chat.name,
                "why": if chat.asking {
                    "asks you".to_owned()
                } else if chat.cut().is_some() {
                    "cut off".to_owned()
                } else if chat.unread {
                    "finished a turn".to_owned()
                } else {
                    chat.ci_ended().join(", ")
                },
                "asks": chat.asking.then(|| now(chat.key).and_then(|now| now.asks)).flatten(),
                "cut_off": cut_off_of(chat),
                "resume": chat.resumes(),
                "goal": chat.goal,
                "sketch": chat.cwd != chat.project,
                "where": where_of(told, &chat.project),
                "active": chat.active,
            })
        })
        .collect()
}

/// The chats of the folder `root`, or of every folder in the home, that
/// the page lists, of `told`: those at work, or with a plan, of `plans`,
/// not done, or done not [`DONE_SHOWS`] ago, or done while it asks the
/// human or they did not look, or marked done not as long ago, or
/// waiting on CI, the subagents that wait for the human, and each chat
/// above one of them, as far as the chats that opened them show on the
/// page, are not archived and are of the folder, for each to show under
/// the chat that opened it.
fn tree_of<'a>(told: &'a Told, root: &Path, plans: &Plans<'_>) -> Vec<&'a Chat> {
    let shown = |chat: &Chat| {
        chat.working
            || plans.done_lately(chat)
            || plans.of(chat).is_some_and(|plan| {
                let done = StepsDone::count(&plan.steps);
                done.of > 0
                    && (done.left() > 0
                        || chat.asking
                        || chat.unread
                        || plans.at - plan.at < DONE_SHOWS)
            })
            || chat.waits_on_ci()
            || (chat.parent.is_some()
                && !chat.resolved
                && (chat.asking || (!chat.archived && (chat.unread || chat.cut().is_some()))))
    };
    let mut tree: Vec<&Chat> = told
        .chats
        .iter()
        .filter(|chat| plans.stays(chat) && of_folder(told, chat, root) && shown(chat))
        .collect();
    for at in 0..tree.len() {
        let mut chat = tree[at];
        while let Some(parent) = parent_of(told, chat).filter(|parent| {
            !parent.archived && plans.stays(parent) && of_folder(told, parent, root)
        }) {
            // Those above it are in already, or come in with it.
            if tree.iter().any(|other| other.key == parent.key) {
                break;
            }
            tree.push(parent);
            chat = parent;
        }
    }
    tree
}

/// The chat of `told` whose agent opened `chat`, as far as the window
/// lists it.
fn parent_of<'a>(told: &'a Told, chat: &Chat) -> Option<&'a Chat> {
    let parent = chat.parent?;
    told.chats.iter().find(|other| other.key == parent)
}

/// The chats the page lists of the folder `root` (see [`tree_of`]), as
/// JSON, as a tree, each under the chat whose agent opened it, at any
/// depth: those under no step of its plan, of `plans`, right under it,
/// then those under each step of it they came from (see [`linked`]), in
/// the plan's order, and, under a plan, its subagents done too. Each tells
/// what it does now, as `now` says, how it stands, the steps of its plan
/// and how far it went, how deep it is, the chat it is under and the
/// step, by its place in the plan.
fn planned(
    told: &Told,
    root: &Path,
    plans: &Plans<'_>,
    now: &dyn Fn(u64) -> Option<commands::Now>,
) -> Vec<Value> {
    let tree = tree_of(told, root, plans);
    let in_tree = |chat: &Chat| tree.iter().any(|other| other.key == chat.key);
    // Each chat to list, how deep, and the chat and the step it is under.
    let mut under: Vec<(&Chat, usize, Option<u64>, Option<usize>)> = told
        .chats
        .iter()
        .rev()
        .filter(|chat| in_tree(chat) && !parent_of(told, chat).is_some_and(in_tree))
        .map(|chat| (chat, 0, None, None))
        .collect();
    let mut listed: Vec<Value> = Vec::new();
    let mut seen: Vec<u64> = Vec::new();
    while let Some((chat, depth, parent, step)) = under.pop() {
        if seen.contains(&chat.key) {
            continue;
        }
        seen.push(chat.key);
        let steps = plans.of(chat).map_or(&[][..], |plan| plan.steps.as_slice());
        let subagents: Vec<&Chat> = told
            .chats
            .iter()
            .filter(|other| other.parent == Some(chat.key))
            .collect();
        let links = linked(steps, &subagents, plans.opened);
        let mut row = row_of(told, chat, depth, plans, now);
        row["parent"] = json!(parent);
        row["step"] = json!(step);
        listed.push(row);
        let mut children: Vec<(&Chat, Option<usize>)> = subagents
            .into_iter()
            .zip(links)
            .filter(|(other, link)| {
                link.is_some() || (plans.stays(other) && (!steps.is_empty() || in_tree(other)))
            })
            .collect();
        // Those under no step first, then those under each, in order.
        children.sort_by_key(|(_, link)| *link);
        under.extend(
            children
                .into_iter()
                .rev()
                .map(|(other, link)| (other, depth + 1, Some(chat.key), link)),
        );
    }
    listed
}

/// What the page tells of `chat`, listed `depth` under the chat at the
/// top of its, as JSON: what it does now, as `now` says, how it stands,
/// and its plan, of `plans`, with how far it went.
fn row_of(
    told: &Told,
    chat: &Chat,
    depth: usize,
    plans: &Plans<'_>,
    now: &dyn Fn(u64) -> Option<commands::Now>,
) -> Value {
    let does = chat
        .working
        .then(|| now(chat.key))
        .flatten()
        .unwrap_or_default();
    let plan = plans.of(chat);
    let steps = plan.map_or(&[][..], |plan| plan.steps.as_slice());
    json!({
        "key": chat.key,
        "chat": chat.kept, "takes": takes_a_line(chat), "steers": chat.steers,
        "name": chat.name,
        "goal": chat.goal,
        "depth": depth,
        "working": chat.working,
        "waits": waits_of(told, chat, depth),
        "cut_off": cut_off_of(chat),
        "resume": chat.resumes(),
        "reported": chat.reported,
        "sketch": chat.cwd != chat.project,
        "where": where_of(told, &chat.project),
        "active": chat.active,
        "said": does.said,
        "running": does.running.map(|(called, line)| json!({"called": called, "line": line})),
        "pulls": pulls_of(chat),
        "spent": spent_of(told, chat),
        "background": chat
            .background
            .iter()
            .map(|background| {
                json!({
                    "what": background.what(),
                    "command": background.is_command(),
                })
            })
            .collect::<Vec<_>>(),
        "plan": steps_of(steps, plans.on_of(chat)),
        // How far the plan went, which the page tells rather than count
        // again.
        "progress": StepsDone::count(steps),
        "finished": StepsDone::count(steps).finished(),
        // Only a plan that keeps a chat listed on its own is dismissed:
        // one at work shows anyway.
        "dismiss": plan
            .filter(|_| !chat.working && !steps.is_empty())
            .map(|plan| json!({ "chat": plan.chat, "plan": plan.number })),
    })
}

/// What `chat`, listed `depth` under the chat at the top of its, waits
/// on, in words, or `None` while it is at work: marked done, it is done;
/// with subagents under it at work, of `told`, at any depth, it waits on
/// them.
fn waits_of(told: &Told, chat: &Chat, depth: usize) -> Option<&'static str> {
    if chat.resolved {
        Some("done")
    } else if chat.working {
        None
    } else if chat.asking {
        Some("asks you")
    } else if chat.cut().is_some() {
        Some("cut off")
    } else if chat.unread {
        Some("finished a turn")
    } else if chat.waits_on_ci() {
        Some("waits on CI")
    } else if works_under(told, chat) {
        Some("waits on subagents")
    } else if depth > 0 {
        Some("finished")
    } else {
        Some("waits")
    }
}

/// Whether a subagent under `chat`, of `told`, at any depth, is at work.
fn works_under(told: &Told, chat: &Chat) -> bool {
    told.chats
        .iter()
        .filter(|other| other.working)
        .any(|other| {
            let mut above = parent_of(told, other);
            // As deep as the window lists chats, for a loop not to go round.
            for _ in 0..told.chats.len() {
                match above {
                    Some(parent) if parent.key == chat.key => return true,
                    Some(parent) => above = parent_of(told, parent),
                    None => return false,
                }
            }
            false
        })
}

/// How many steps got done in the folder `root`, or in every folder in
/// the home, over each of [`WINDOWS`], as JSON, as `plans` read them
/// moved: of the plans of its chats' agents, the subagents' too, each
/// step once, however many of the window's chats stand for its chat.
fn summaries(told: &Told, root: &Path, plans: &Plans<'_>) -> Vec<Value> {
    // The chats, as coxd keeps them, each once.
    let mut kept: Vec<ChatId> = Vec::new();
    for chat in told.chats.iter().filter(|chat| of_folder(told, chat, root)) {
        if let Some(id) = chat.kept.filter(|id| !kept.contains(id)) {
            kept.push(id);
        }
    }
    WINDOWS
        .iter()
        .map(|&(name, long)| {
            let start = plans.at - long;
            let steps = plans
                .moved
                .done
                .iter()
                .filter(|(chat, _, at)| *at >= start && kept.contains(chat))
                .count();
            json!({ "window": name, "steps": steps })
        })
        .collect()
}

/// What `chat`'s agent spent, with what the agents of the subagents'
/// chats under it spent added, as JSON: the cost, as the human reads it,
/// once one of the agents told one, how full the chat's context is, in
/// hundredths, if its agent told, and what each part cost, for the page
/// to tell when asked. Null when none of their agents told anything.
fn spent_of(told: &Told, chat: &Chat) -> Value {
    let mut subagents = None;
    let mut under = vec![chat.key];
    while let Some(key) = under.pop() {
        for other in told.chats.iter().filter(|other| other.parent == Some(key)) {
            under.push(other.key);
            if let Some(spent) = &other.spent {
                subagents = Some(subagents.unwrap_or_default() + spent.cost());
            }
        }
    }
    if chat.spent.is_none() && subagents.is_none() {
        return Value::Null;
    }
    let own = chat
        .spent
        .as_ref()
        .map_or_else(Microdollars::default, Spent::cost);
    let context = chat
        .spent
        .as_ref()
        .and_then(|spent| spent.context)
        .map(coxd::chat::spent::Context::percent);
    let mut title = format!("this chat {own}");
    if let Some(subagents) = subagents {
        write!(title, ", its subagents {subagents}").unwrap();
    }
    if let Some(context) = context {
        write!(title, "; its context {context}% full").unwrap();
    }
    let cost = own + subagents.unwrap_or_default();
    json!({
        "cost": (cost > Microdollars::default()).then(|| cost.to_string()),
        "context": context,
        "title": title,
    })
}

/// The steps of a plan, as JSON, each with its id and how many lines the
/// human wrote on it, as `on` counts them: the subagents that came from
/// it are listed under it on their own (see [`planned`]).
fn steps_of(steps: &[Step], on: &[(u64, usize)]) -> Vec<Value> {
    steps
        .iter()
        .map(|step| {
            let written = step
                .id
                .and_then(|id| on.iter().find(|(on, _)| *on == id))
                .map_or(0, |(_, count)| *count);
            json!({
                "text": step.text,
                "state": step.state,
                "id": step.id,
                "on": written,
            })
        })
        .collect()
}

/// What goes on in the project of the folder `root`, as [`told`] tells
/// it, the chats standing as `told` says, doing and asking as `now`
/// says.
fn told_of(
    told: &Told,
    root: &Path,
    plans: &Plans<'_>,
    now: &dyn Fn(u64) -> Option<commands::Now>,
) -> Value {
    let root = &project_of(told, root);
    let busy = told
        .chats
        .iter()
        .filter(|chat| chat.working && of_folder(told, chat, root))
        .count();
    let mut told_json = json!({
        "folder": root,
        "home": *root == told.home,
        "github": told.github,
        "shown": told.shown,
        "waiting": waiting_in(told, root, now),
        "plans": planned(told, root, plans, now),
        "summaries": summaries(told, root, plans),
        "busy": busy,
        "pulls": pulls_in(told, root, plans.at),
    });
    hint(&mut told_json, &hints(told, root, plans));
    told_json
}

/// A way to tell a chat apart from others of its name, if it tells.
type Hinting<'a> = dyn Fn(&Chat) -> Option<String> + 'a;

/// What tells each chat of the folder `root`, or of every folder in the
/// home, apart from the others of its name, for those whose name another
/// has, by its key: the first of where it works, the chat that opened
/// it, and when it was opened, of `plans`, to the minute or else to the
/// second, that no other of its name shares.
fn hints(told: &Told, root: &Path, plans: &Plans<'_>) -> Vec<(u64, String)> {
    let chats: Vec<&Chat> = told
        .chats
        .iter()
        .filter(|chat| of_folder(told, chat, root))
        .collect();
    let zone = TimeZone::system();
    let folder = |chat: &Chat| {
        if chat.cwd == chat.project {
            where_of(told, &chat.project)
        } else {
            chat.cwd
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        }
        .map(|folder| format!("in {folder}"))
    };
    let parent = |chat: &Chat| {
        let parent = chat.parent?;
        let parent = told.chats.iter().find(|other| other.key == parent)?;
        Some(format!("under {}", parent.name))
    };
    let opened = |chat: &Chat, seconds: bool| {
        let at = plans
            .opened
            .iter()
            .find(|(id, _)| Some(*id) == chat.kept)?
            .1;
        let day = if plans.at - at < 20 * 60 * 60 * 1000 {
            ""
        } else {
            "%b %-d "
        };
        let format = format!("opened {day}%H:%M{}", if seconds { ":%S" } else { "" });
        let at = jiff::Timestamp::from_millisecond(at).ok()?;
        Some(at.to_zoned(zone.clone()).strftime(&format).to_string())
    };
    let (by_minute, by_second) = (
        |chat: &Chat| opened(chat, false),
        |chat: &Chat| opened(chat, true),
    );
    // To the second, for chats opened in the same minute.
    let ways: [&Hinting<'_>; 4] = [&folder, &parent, &by_minute, &by_second];
    let mut hints = Vec::new();
    for chat in &chats {
        let alike: Vec<&&Chat> = chats
            .iter()
            .filter(|other| other.key != chat.key && other.name == chat.name)
            .collect();
        if alike.is_empty() {
            continue;
        }
        let told_apart = ways.iter().find_map(|way| {
            let hint = way(chat)?;
            alike
                .iter()
                .all(|other| way(other).as_ref() != Some(&hint))
                .then_some(hint)
        });
        if let Some(hint) = told_apart {
            hints.push((chat.key, hint));
        }
    }
    hints
}

/// Puts each of `hints` on each chat `told` tells of, by its key and its
/// name, as `hint`, in every part of it.
fn hint(told: &mut Value, hints: &[(u64, String)]) {
    match told {
        Value::Object(object) => {
            let named =
                object.contains_key("name") || object.get("chat").is_some_and(Value::is_string);
            if named
                && let Some(key) = object.get("key").and_then(Value::as_u64)
                && let Some((_, hint)) = hints.iter().find(|(hinted, _)| *hinted == key)
            {
                object.insert("hint".to_owned(), json!(hint));
            }
            for value in object.values_mut() {
                hint(value, hints);
            }
        }
        Value::Array(values) => {
            for value in values {
                hint(value, hints);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cox_core::entry::Entry;
    use cox_core::store::ResolvedBy;
    use coxd::testing::TempDir;

    const NO_MOVED: Progress = Progress {
        active: Vec::new(),
        done: Vec::new(),
        resolved: Vec::new(),
    };

    const NO_PLANS: Plans<'static> = Plans {
        kept: &[],
        on: &[],
        opened: &[],
        dismissed: &[],
        moved: &NO_MOVED,
        at: 0,
    };

    /// The chat coxd keeps by `number`.
    fn kept(number: u64) -> ChatId {
        serde_json::from_value(json!(number)).unwrap()
    }

    fn step(text: &str, state: StepState) -> Step {
        Step {
            text: text.to_owned(),
            state,
            since: None,
            started: None,
            id: None,
        }
    }

    fn chat(key: u64, cwd: &Path, project: &Path, asking: bool, unread: bool) -> Chat {
        Chat {
            key,
            name: format!("chat {key}"),
            cwd: cwd.to_owned(),
            project: project.to_owned(),
            asking,
            unread,
            goal: None,
            working: false,
            background: Vec::new(),
            parent: None,
            kept: None,
            pulls: Vec::new(),
            spent: None,
            resolved: false,
            reported: false,
            archived: false,
            steers: false,
            on_steps: Vec::new(),
            active: 0,
            cut_off: None,
        }
    }

    /// The chats `json` lists, a line each, stepped in as deep as each
    /// is, under the line of the chat above it, which it tells it is
    /// under, with the step of that chat's plan it came from, if it tells,
    /// how it waits and whether it reported.
    fn tree(json: &Value) -> Vec<String> {
        let mut above: Vec<Value> = Vec::new();
        json["plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| {
                let depth = usize::try_from(chat["depth"].as_u64().unwrap()).unwrap();
                above.truncate(depth);
                let parent = above.last().cloned().unwrap_or(Value::Null);
                assert_eq!(
                    chat["parent"], parent,
                    "{} is under the line above",
                    chat["key"]
                );
                above.push(chat["key"].clone());
                let step = chat["step"]
                    .as_u64()
                    .map_or_else(String::new, |step| format!(" on {step}"));
                let reported = if chat["reported"] == true {
                    ", reported"
                } else {
                    ""
                };
                format!(
                    "{}{}{step}: {}{reported}",
                    "  ".repeat(depth),
                    chat["key"],
                    chat["waits"]
                )
            })
            .collect()
    }

    #[test]
    fn a_chat_at_work_tells_what_it_and_the_subagents_under_it_spent() {
        use coxd::chat::spent::{Context, Spent};
        let folder = TempDir::new();
        let root = folder.path().join("app");
        let spent = |micros, context: Option<u64>| {
            Some(Spent {
                session: cox_acp::Microdollars(micros),
                context: context.map(|used| Context {
                    used,
                    size: 1_000_000,
                }),
                ..Spent::default()
            })
        };
        let mut main = chat(1, &root, &root, false, false);
        main.working = true;
        main.spent = spent(420_000, Some(612_000));
        let mut subagent = chat(2, &root, &root, false, false);
        subagent.parent = Some(1);
        subagent.working = true;
        subagent.spent = spent(100_000, Some(30_000));
        let mut under_it = chat(3, &root, &root, false, false);
        under_it.parent = Some(2);
        under_it.spent = spent(5_000, None);
        let mut silent = chat(4, &root, &root, false, false);
        silent.working = true;
        let told = Told {
            chats: vec![main, subagent, under_it, silent],
            projects: vec![(root.clone(), "app".to_owned())],
            home: folder.path().to_owned(),
            ..Told::default()
        };

        let json = told_of(&told, &root, &NO_PLANS, &|_| None);

        let spent: Vec<(u64, &Value)> = json["plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| (chat["key"].as_u64().unwrap(), &chat["spent"]))
            .collect();
        assert_eq!(
            spent,
            [
                (
                    1,
                    &json!({
                        "cost": "$0.53",
                        "context": 61,
                        "title": "this chat $0.42, its subagents $0.11; its context 61% full",
                    })
                ),
                (
                    2,
                    &json!({
                        "cost": "$0.11",
                        "context": 3,
                        "title": "this chat $0.10, its subagents $0.01; its context 3% full",
                    })
                ),
                (4, &Value::Null),
            ]
        );
    }

    /// What goes on in the folder `root`, no chat doing or asking
    /// anything.
    fn told_in(told: &Told, root: &Path) -> Value {
        told_of(told, root, &NO_PLANS, &|_| None)
    }

    #[test]
    fn what_waits_is_of_the_folder_and_its_sketches() {
        let root = TempDir::new();
        let other = root.join("other");
        let sketch = root.join("sketch");
        let told = Told {
            chats: vec![
                chat(1, &root, &root, true, false),
                chat(2, &root, &root, false, false),
                chat(3, &sketch, &root, false, true),
                chat(4, &other, &other, true, false),
            ],
            projects: vec![(root.to_path_buf(), "app".to_owned())],
            home: root.join("home"),
            ..Told::default()
        };
        let json = told_in(&told, &root);
        let waiting = json["waiting"].as_array().unwrap();
        assert_eq!(waiting.len(), 2);
        assert_eq!(waiting[0]["key"], 1);
        assert_eq!(waiting[0]["why"], "asks you");
        assert_eq!(waiting[1]["key"], 3);
        assert_eq!(waiting[1]["why"], "finished a turn");
        assert_eq!(waiting[1]["sketch"], true);
        assert_eq!(waiting[1]["where"], "app");

        // The home tells of what waits in every folder.
        let home = told.home.clone();
        let json = told_in(&told, &home);
        let keys: Vec<_> = json["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| chat["key"].as_u64().unwrap())
            .collect();
        assert_eq!(keys, [1, 3, 4]);
    }

    #[test]
    fn the_inbox_tells_what_is_asked_and_at_work_in_the_folder() {
        let root = TempDir::new();
        let other = root.join("other");
        let home = root.join("home");
        let mut asking = chat(1, &root, &root, true, false);
        asking.kept = Some(kept(1));
        let mut parent = chat(2, &root, &root, false, false);
        parent.working = true;
        parent.goal = Some("ship it".to_owned());
        let mut subagent = chat(3, &root, &root, false, false);
        subagent.working = true;
        subagent.parent = Some(2);
        let mut elsewhere = chat(4, &other, &other, false, false);
        elsewhere.working = true;
        let mut orphan = chat(5, &root, &root, false, false);
        orphan.working = true;
        orphan.parent = Some(9);
        orphan.background = vec![coxd::daemon::wire::Background {
            kind: coxd::daemon::wire::BackgroundKind::Command,
            name: "sleep 120".to_owned(),
            description: None,
        }];
        let told = Told {
            chats: vec![asking, subagent, parent, elsewhere, orphan],
            projects: vec![
                (root.to_path_buf(), "app".to_owned()),
                (other.clone(), "other".to_owned()),
            ],
            home: home.clone(),
            ..Told::default()
        };
        let now = |key: u64| {
            Some(commands::Now {
                said: Some(format!("step of {key}")),
                running: (key == 3).then(|| ("Wait".to_owned(), "sleep 30".to_owned())),
                asks: (key == 1).then(|| "Which way?".to_owned()),
            })
        };

        let json = told_of(&told, &root, &NO_PLANS, &now);

        assert_eq!(json["waiting"][0]["asks"], "Which way?");
        let working: Vec<(u64, u64)> = json["plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| {
                (
                    chat["key"].as_u64().unwrap(),
                    chat["depth"].as_u64().unwrap(),
                )
            })
            .collect();
        // A subagent comes under the chat that opened it, and one whose
        // opener is not at work comes on its own.
        assert_eq!(working, [(2, 0), (3, 1), (5, 0)]);
        assert_eq!(json["plans"][0]["goal"], "ship it");
        assert_eq!(json["plans"][0]["said"], "step of 2");
        let running = json!({"called": "Wait", "line": "sleep 30"});
        assert_eq!(json["plans"][1]["running"], running);
        // A chat at work in the background tells what runs there.
        assert_eq!(
            json["plans"][2]["background"],
            json!([{"what": "sleep 120", "command": true}])
        );
        // Only what the page draws.
        let mut keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "busy",
                "folder",
                "github",
                "home",
                "plans",
                "pulls",
                "shown",
                "summaries",
                "waiting"
            ]
        );

        // The home tells of every folder.
        let json = told_of(&told, &home, &NO_PLANS, &now);
        assert_eq!(json["plans"].as_array().unwrap().len(), 4);
        assert_eq!(json["busy"], 4);
    }

    #[test]
    fn a_step_takes_a_line_on_it_as_the_chat_takes_one() {
        let root = TempDir::new();
        let of = |key: u64, asking: bool, working: bool, steers: bool| {
            let mut of = chat(key, &root, &root, asking, !asking && !working);
            of.kept = Some(kept(key));
            of.working = working;
            of.steers = steers;
            of
        };
        let told = Told {
            // Waiting, asking, at work and asking with a turn it steers.
            chats: vec![
                of(1, false, false, false),
                of(2, true, false, false),
                of(3, false, true, false),
                of(4, true, true, true),
            ],
            home: root.join("home"),
            ..Told::default()
        };
        let known = |text: &str, id| Step {
            id: Some(id),
            ..step(text, StepState::Pending)
        };
        let kept_plans = [1, 2, 3, 4].map(|chat| KeptPlan {
            chat: kept(chat),
            number: 1,
            steps: vec![
                known("Rename the setting", 1),
                step("Older", StepState::Pending),
            ],
            at: 0,
        });
        let on = [(kept(1), vec![(1, 2)])];
        let plans = Plans {
            kept: &kept_plans,
            on: &on,
            ..NO_PLANS
        };

        let json = told_of(&told, &root, &plans, &|_| None);

        let shown = &json["plans"][0];
        assert_eq!((&shown["chat"], &shown["takes"]), (&json!(1), &json!(true)));
        let takes: Vec<(u64, bool)> = json["plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| {
                (
                    chat["key"].as_u64().unwrap(),
                    chat["takes"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(takes, [(1, true), (2, false), (3, true), (4, true)]);
        let steps = &shown["plan"];
        assert_eq!((&steps[0]["id"], &steps[0]["on"]), (&json!(1), &json!(2)));
        assert_eq!(
            (&steps[1]["id"], &steps[1]["on"]),
            (&Value::Null, &json!(0))
        );

        // A line goes on a step the chat's plan has, as the step reads
        // there, to a chat that takes one.
        let asked = |key: u64, chat: u64, step: u64, text: &str| Asked {
            key,
            open: Open {
                chat: kept(chat),
                step,
            },
            text: text.to_owned(),
        };
        assert_eq!(
            says_on(&told, &kept_plans, &asked(1, 1, 1, " keep the old name ")),
            Some(Say {
                key: 1,
                text: "keep the old name".to_owned(),
                step: Some(OnStep {
                    id: 1,
                    text: "Rename the setting".to_owned(),
                }),
            })
        );
        for (key, chat, step, text) in [
            (2, 2, 1, "asking"),
            (1, 1, 9, "no such step"),
            (1, 2, 1, "another chat's key"),
            (1, 1, 1, "  "),
        ] {
            assert_eq!(
                says_on(&told, &kept_plans, &asked(key, chat, step, text)),
                None,
                "{text}"
            );
        }
        // At work, it takes the line, into the turn or once it ends.
        assert!(says_on(&told, &kept_plans, &asked(3, 3, 1, "busy")).is_some());
        assert!(says_on(&told, &kept_plans, &asked(4, 4, 1, "steered")).is_some());
    }

    #[test]
    fn a_thread_tells_the_lines_on_its_step_that_wait_for_the_turn_to_end() {
        let root = TempDir::new();
        let mut waits = chat(1, &root, &root, false, false);
        waits.kept = Some(kept(1));
        let waiting = |text: &str, id| OnStepLine {
            text: text.to_owned(),
            step: OnStep {
                id,
                text: String::new(),
            },
        };
        waits.on_steps = vec![waiting("later", 1), waiting("elsewhere", 9)];
        let told = Told {
            chats: vec![waits],
            ..Told::default()
        };
        let open = Open {
            chat: kept(1),
            step: 1,
        };

        assert_eq!(waiting_on(&told, &open), ["later"]);
    }

    #[test]
    fn a_thread_tells_who_said_each_line_and_when() {
        use cox_core::store::Sayer;
        let thread = thread_of(
            &Open {
                chat: kept(1),
                step: 1,
            },
            &[
                Said {
                    by: Sayer::Human,
                    text: "keep the old name".to_owned(),
                    at: 0,
                    number: 4,
                },
                Said {
                    by: Sayer::Agent,
                    text: "Kept.".to_owned(),
                    at: 60_000,
                    number: 6,
                },
            ],
            &TimeZone::UTC,
        );
        assert_eq!(
            thread,
            json!({
                "chat": 1,
                "step": 1,
                "said": [
                    { "by": "human", "text": "keep the old name", "at": "00:00" },
                    { "by": "agent", "text": "Kept.", "at": "00:01" },
                ],
            })
        );
    }

    #[test]
    fn chats_of_one_name_are_told_apart_by_folder_then_parent_then_when_opened() {
        let root = TempDir::new();
        let named = |key: u64, name: &str, cwd: &Path, parent: Option<u64>| {
            let mut named = chat(key, cwd, &root, false, true);
            named.name = name.to_owned();
            named.parent = parent;
            named.kept = Some(kept(key));
            named
        };
        let told = Told {
            chats: vec![
                named(1, "Fix CI", &root.join("sketch-a"), None),
                named(2, "Fix CI", &root.join("sketch-b"), None),
                named(3, "Lead", &root, None),
                named(4, "Lead", &root, None),
                named(5, "Review", &root, Some(3)),
                named(6, "Review", &root, Some(1)),
                named(7, "Only one", &root, None),
                named(8, "Look", &root, None),
                named(9, "Look", &root, None),
            ],
            home: root.join("home"),
            ..Told::default()
        };
        let hour = 60 * 60 * 1000;
        let opened = [
            (kept(3), 9 * hour),
            (kept(4), 9 * hour + 60_000),
            (kept(8), 9 * hour + 1_000),
            (kept(9), 9 * hour + 2_000),
        ];
        let plans = Plans {
            opened: &opened,
            at: 10 * hour,
            ..NO_PLANS
        };

        let mut hints = hints(&told, &root, &plans);
        hints.sort();
        let at = |at: i64, format: &str| {
            jiff::Timestamp::from_millisecond(9 * hour + at)
                .unwrap()
                .to_zoned(TimeZone::system())
                .strftime(format)
                .to_string()
        };
        assert_eq!(
            hints,
            [
                (1, "in sketch-a".to_owned()),
                (2, "in sketch-b".to_owned()),
                (3, at(0, "opened %H:%M")),
                (4, at(60_000, "opened %H:%M")),
                (5, "under Lead".to_owned()),
                (6, "under Fix CI".to_owned()),
                (8, at(1_000, "opened %H:%M:%S")),
                (9, at(2_000, "opened %H:%M:%S")),
            ]
        );

        let json = told_of(&told, &root, &plans, &|_| None);
        let waiting: Vec<(&str, &Value)> = json["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| (chat["name"].as_str().unwrap(), &chat["hint"]))
            .collect();
        assert_eq!(waiting[0], ("Fix CI", &json!("in sketch-a")));
        assert_eq!(waiting[6], ("Only one", &Value::Null));
    }

    #[test]
    fn a_chat_shows_its_plan_while_at_work_not_done_or_done_lately() {
        let root = TempDir::new();
        let listed = |key: u64, name: &str, working: bool, parent: Option<u64>| {
            let mut listed = chat(key, &root, &root, false, false);
            listed.name = name.to_owned();
            listed.working = working;
            listed.parent = parent;
            listed.kept = Some(kept(key));
            listed
        };
        let mut unseen = listed(6, "unseen", false, None);
        unseen.unread = true;
        let mut asking = listed(8, "asking", false, None);
        asking.asking = true;
        let told = Told {
            chats: vec![
                listed(2, "main", true, None),
                listed(3, "Add the --shout flag", true, Some(2)),
                listed(4, "Look around", true, Some(2)),
                listed(9, "README docs update", false, Some(2)),
                listed(11, "Check CI", false, Some(2)),
                listed(5, "waits", false, None),
                unseen,
                listed(7, "done long ago", false, None),
                asking,
                listed(10, "done lately", false, None),
            ],
            home: root.join("home"),
            ..Told::default()
        };
        let at = 100 * DONE_SHOWS;
        let plan = |chat: u64, steps: Vec<Step>, ago: i64| KeptPlan {
            chat: kept(chat),
            number: 1,
            steps,
            at: at - ago,
        };
        let done = || vec![step("Ship it", StepState::Completed)];
        let kept_plans = [
            plan(
                2,
                vec![
                    step("Add a --shout flag with tests", StepState::InProgress),
                    step("Update the README", StepState::Completed),
                    step("Commit", StepState::Pending),
                ],
                0,
            ),
            plan(5, vec![step("Later", StepState::Pending)], DONE_SHOWS * 2),
            plan(6, done(), DONE_SHOWS * 2),
            plan(7, done(), DONE_SHOWS * 2),
            plan(8, done(), DONE_SHOWS * 2),
            plan(10, done(), DONE_SHOWS / 2),
        ];
        let plans = Plans {
            kept: &kept_plans,
            at,
            ..NO_PLANS
        };
        let now = |key: u64| {
            Some(commands::Now {
                said: Some(format!("step of {key}")),
                running: None,
                asks: None,
            })
        };

        let json = told_of(&told, &root, &plans, &now);

        // Under the chat, its subagents that came from no step of its
        // plan first, done or not, while it has a plan, then those whose
        // names read as a step, under the step, in the plan's order.
        assert_eq!(
            tree(&json),
            [
                "2: null",
                "  4: null",
                r#"  11: "finished""#,
                "  3 on 0: null",
                r#"  9 on 1: "finished""#,
                r#"5: "waits""#,
                r#"6: "finished a turn""#,
                r#"8: "asks you""#,
                r#"10: "waits""#,
            ]
        );
        assert_eq!(json["plans"][0]["plan"][0]["state"], "in_progress");
        assert_eq!(json["plans"][3]["said"], "step of 3");
        // Only the chats at work count as at work.
        assert_eq!(json["busy"], 3);
    }

    #[test]
    fn subagents_show_under_the_chats_and_the_steps_they_came_from_at_any_depth() {
        let root = TempDir::new();
        let listed = |key: u64, name: &str, working: bool, parent: Option<u64>| {
            let mut listed = chat(key, &root, &root, false, false);
            listed.name = name.to_owned();
            listed.working = working;
            listed.parent = parent;
            listed.kept = Some(kept(key));
            listed
        };
        let mut reported = listed(6, "Update the docs", false, Some(1));
        (reported.resolved, reported.reported) = (true, true);
        let mut asking = listed(7, "Pick a name", false, Some(1));
        asking.asking = true;
        // 1, with no plan, opened 2, 6 and 7; 2 opened 3 while it tested,
        // and 5 before it had a plan; 3 opened 4 while it ran on Linux.
        let told = Told {
            chats: vec![
                listed(1, "orchestrate the release", false, None),
                listed(2, "Add the --shout flag", true, Some(1)),
                listed(3, "Test the --shout flag", true, Some(2)),
                listed(4, "Run them on Linux", true, Some(3)),
                listed(5, "Look up the conventions", false, Some(2)),
                reported,
                asking,
            ],
            home: root.join("home"),
            ..Told::default()
        };
        let at = 10 * DONE_SHOWS;
        // So many seconds ago.
        let ago = |seconds: i64| at - seconds * 1000;
        let step = |text: &str, state: StepState, started: Option<i64>, since: i64| Step {
            text: text.to_owned(),
            state,
            since: Some(ago(since)),
            started: started.map(ago),
            id: None,
        };
        let plan = |chat: u64, steps: Vec<Step>| KeptPlan {
            chat: kept(chat),
            number: 1,
            steps,
            at,
        };
        let kept_plans = [
            plan(
                2,
                vec![
                    step("Read the code", StepState::Completed, Some(100), 90),
                    step("Add the flag", StepState::Completed, Some(90), 60),
                    step("Test it", StepState::InProgress, Some(60), 60),
                    step("Commit", StepState::Pending, None, 100),
                ],
            ),
            plan(
                3,
                vec![
                    step("Write the tests", StepState::Completed, Some(50), 30),
                    step("Run them on Linux", StepState::InProgress, Some(30), 30),
                    step("Run them on macOS", StepState::Pending, None, 50),
                ],
            ),
            plan(4, vec![step("Build", StepState::InProgress, Some(20), 20)]),
        ];
        let opened = [(kept(3), ago(50)), (kept(4), ago(20)), (kept(5), ago(200))];
        let moved = Progress {
            resolved: vec![(
                kept(6),
                cox_core::store::Resolved {
                    at: at - 10,
                    by: ResolvedBy::Agent,
                },
            )],
            ..NO_MOVED
        };
        let plans = Plans {
            kept: &kept_plans,
            opened: &opened,
            moved: &moved,
            at,
            ..NO_PLANS
        };

        let json = told_of(&told, &root, &plans, &|_| None);

        // The chat with no plan shows above the subagents it waits on.
        assert_eq!(
            tree(&json),
            [
                r#"1: "waits on subagents""#,
                "  2: null",
                r#"    5: "finished""#,
                "    3 on 2: null",
                "      4 on 1: null",
                r#"  6: "done", reported"#,
                r#"  7: "asks you""#,
            ]
        );
        // Only the chats at work count as at work.
        assert_eq!(json["busy"], 3);
    }

    #[test]
    fn the_chat_above_a_subagent_at_work_shows_unless_archived() {
        let root = TempDir::new();
        let listed = |key: u64, parent: Option<u64>, working: bool, archived: bool| {
            let mut listed = chat(key, &root, &root, false, false);
            (listed.parent, listed.working, listed.archived) = (parent, working, archived);
            listed
        };
        // 1 and 3, with no plan and not at work, opened 2 and 4, at work;
        // the human archived 3.
        let told = Told {
            chats: vec![
                listed(1, None, false, false),
                listed(2, Some(1), true, false),
                listed(3, None, false, true),
                listed(4, Some(3), true, false),
            ],
            home: root.join("home"),
            ..Told::default()
        };

        let json = told_of(&told, &root, &NO_PLANS, &|_| None);

        assert_eq!(
            tree(&json),
            [r#"1: "waits on subagents""#, "  2: null", "4: null"]
        );
        assert_eq!(json["busy"], 2);
    }

    #[test]
    fn the_stage_of_a_sketch_or_a_worktree_tells_of_the_whole_project_as_its_own_does() {
        let folder = TempDir::new();
        let project = folder.join("app");
        let other = folder.join("other");
        let sketch = folder.join("sketches").join("scratch-1");
        let worktree = project.join("worktrees").join("app").join("parser");
        let minute = 60 * 1000;
        let at = 100 * 24 * 60 * minute;
        let listed = |key: u64, cwd: &Path, parent: Option<u64>| {
            let mut listed = chat(key, cwd, &project, false, false);
            (listed.kept, listed.parent, listed.working) = (Some(kept(key)), parent, true);
            listed
        };
        let mut asking = listed(4, &project, Some(1));
        (asking.working, asking.asking) = (false, true);
        let mut elsewhere = listed(7, &other, None);
        elsewhere.project.clone_from(&other);
        // Two chats of the project the human opened, each with subagents
        // of its own: 1's in a sketch, with one more there, and one beside
        // it, which asks the human something; 5's in a worktree.
        let told = Told {
            chats: vec![
                listed(1, &project, None),
                listed(2, &sketch, Some(1)),
                listed(3, &sketch, Some(2)),
                asking,
                listed(5, &project, None),
                listed(6, &worktree, Some(5)),
                elsewhere,
            ],
            projects: vec![
                (folder.join("home"), "~".to_owned()),
                (project.clone(), "app".to_owned()),
                (other.clone(), "other".to_owned()),
            ],
            home: folder.join("home"),
            github: None,
            shown: Some(3),
            pulls: Vec::new(),
        };
        let moved = Progress {
            active: [1, 2, 3, 5, 6, 7]
                .into_iter()
                .map(|chat| (kept(chat), at - minute))
                .collect(),
            done: vec![
                (kept(3), "Seed the corpus".to_owned(), at - 5 * minute),
                (kept(7), "Its own".to_owned(), at - 10 * minute),
                (kept(6), "Build on Linux".to_owned(), at - 30 * minute),
            ],
            resolved: Vec::new(),
        };
        let plans = Plans {
            moved: &moved,
            at,
            ..NO_PLANS
        };
        let page = |root: &Path| told_of(&told, root, &plans, &|_| None);

        let json = page(&project);

        assert_eq!(
            tree(&json),
            [
                "1: null",
                "  2: null",
                "    3: null",
                r#"  4: "asks you""#,
                "5: null",
                "  6: null"
            ]
        );
        assert_eq!(json["folder"], json!(project));
        assert_eq!(json["shown"], 3);
        assert_eq!(json["busy"], 5);
        let steps: Vec<u64> = json["summaries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|summary| summary["steps"].as_u64().unwrap())
            .collect();
        assert_eq!(steps, [1, 2, 2]);
        assert_eq!(waiting_of(&told, &project), 1);
        // The stage of the sketch, of the worktree, and of a worktree in
        // the project's folder whose chats are gone, tell the same.
        let gone = project.join("worktrees").join("app").join("gone");
        for root in [&sketch, &worktree, &gone] {
            assert_eq!(page(root), json, "{}", root.display());
            assert_eq!(waiting_of(&told, root), 1, "{}", root.display());
        }
        // Another project's tells of its own.
        assert_eq!(page(&other)["busy"], 1);
        assert_eq!(waiting_of(&told, &other), 0);
    }

    #[test]
    fn how_far_each_plan_went_is_told_for_the_page_not_to_count_it() {
        let root = TempDir::new();
        let listed = |key: u64, parent: Option<u64>| {
            let mut listed = chat(key, &root, &root, false, false);
            listed.working = true;
            listed.parent = parent;
            listed.kept = Some(kept(key));
            listed
        };
        let told = Told {
            chats: vec![listed(1, None), listed(2, Some(1)), listed(3, None)],
            ..Told::default()
        };
        let plan = |chat: u64, steps: Vec<Step>| KeptPlan {
            chat: kept(chat),
            number: 1,
            steps,
            at: 0,
        };
        let kept_plans = [
            plan(
                1,
                vec![
                    step("Read", StepState::Completed),
                    step("Fix", StepState::InProgress),
                    step("Test", StepState::Pending),
                ],
            ),
            plan(3, vec![step("Ship it", StepState::Completed)]),
        ];
        let plans = Plans {
            kept: &kept_plans,
            ..NO_PLANS
        };

        let json = told_of(&told, &root, &plans, &|_| None);

        let told: Vec<(u64, &Value, &Value)> = json["plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| {
                (
                    chat["key"].as_u64().unwrap(),
                    &chat["progress"],
                    &chat["finished"],
                )
            })
            .collect();
        // None, for a subagent with no plan of its own.
        assert_eq!(
            told,
            [
                (1, &json!({ "done": 1, "of": 3 }), &json!(false)),
                (2, &json!({ "done": 0, "of": 0 }), &json!(false)),
                (3, &json!({ "done": 1, "of": 1 }), &json!(true)),
            ]
        );
    }

    #[test]
    fn a_chat_marked_done_long_ago_leaves_the_plans_and_what_waits_and_one_archived_waits_only_asking()
     {
        let root = TempDir::new();
        let listed = |key: u64, parent: Option<u64>| {
            let mut listed = chat(key, &root, &root, false, false);
            listed.parent = parent;
            listed.kept = Some(kept(key));
            listed
        };
        let mut main = listed(1, None);
        main.working = true;
        let mut done = listed(2, None);
        done.resolved = true;
        done.unread = true;
        let mut done_under = listed(3, Some(1));
        done_under.resolved = true;
        let mut archived_unread = listed(4, None);
        archived_unread.archived = true;
        archived_unread.unread = true;
        let mut archived_asking = listed(5, None);
        archived_asking.archived = true;
        archived_asking.asking = true;
        let told = Told {
            chats: vec![main, done, done_under, archived_unread, archived_asking],
            home: root.join("home"),
            ..Told::default()
        };
        let not_done = || vec![step("Later", StepState::Pending)];
        let kept_plans = [
            KeptPlan {
                chat: kept(1),
                number: 1,
                steps: not_done(),
                at: 0,
            },
            KeptPlan {
                chat: kept(2),
                number: 1,
                steps: not_done(),
                at: 0,
            },
        ];
        let plans = Plans {
            kept: &kept_plans,
            ..NO_PLANS
        };

        let json = told_of(&told, &root, &plans, &|_| None);

        let keys = |tier: &str| -> Vec<u64> {
            json[tier]
                .as_array()
                .unwrap()
                .iter()
                .map(|chat| chat["key"].as_u64().unwrap())
                .collect()
        };
        assert_eq!(keys("plans"), [1], "neither 2 nor 3 under 1");
        assert_eq!(keys("waiting"), [5]);
    }

    #[test]
    fn a_chat_marked_done_shows_as_done_with_its_subagents_as_long_as_the_longest_window() {
        let root = TempDir::new();
        let listed = |key: u64, parent: Option<u64>| {
            let mut listed = chat(key, &root, &root, false, false);
            listed.parent = parent;
            listed.kept = Some(kept(key));
            listed.resolved = true;
            listed
        };
        let at = 10 * DONE_SHOWS;
        let resolved = |key: u64, ago: i64| {
            (
                kept(key),
                cox_core::store::Resolved {
                    at: at - ago,
                    by: ResolvedBy::Human,
                },
            )
        };
        let told = Told {
            // 1 has no plan; 2, its subagent, reported; 3 was marked done
            // long ago; 4's plan the human dismissed.
            chats: vec![
                listed(1, None),
                listed(2, Some(1)),
                listed(3, None),
                listed(4, None),
            ],
            home: root.join("home"),
            ..Told::default()
        };
        let kept_plans = [KeptPlan {
            chat: kept(4),
            number: 1,
            steps: vec![step("Later", StepState::Pending)],
            at: at - DONE_SHOWS / 2,
        }];
        let moved = Progress {
            resolved: vec![
                resolved(1, DONE_SHOWS - 1),
                resolved(2, DONE_SHOWS / 2),
                resolved(3, DONE_SHOWS),
                resolved(4, 0),
            ],
            ..NO_MOVED
        };
        let dismissed = [Dismissed {
            chat: kept(4),
            number: 1,
            steps: kept_plans[0].steps.clone(),
        }];
        let plans = Plans {
            kept: &kept_plans,
            dismissed: &dismissed,
            moved: &moved,
            at,
            ..NO_PLANS
        };

        let json = told_of(&told, &root, &plans, &|_| None);

        let rows: Vec<(u64, u64, &str)> = json["plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| {
                (
                    chat["key"].as_u64().unwrap(),
                    chat["depth"].as_u64().unwrap(),
                    chat["waits"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(rows, [(1, 0, "done"), (2, 1, "done")]);
        assert_eq!(json["waiting"], json!([]), "none of them waits");
    }

    #[test]
    fn a_dismissed_plan_stays_hidden_across_restarts_until_another_comes() {
        let root = TempDir::new();
        coxd::storage::keep_on_this_thread(Some(root.join("data")));
        let store = Store::open(&coxd::storage::path(coxd::storage::Kept::Chats).unwrap()).unwrap();
        let half_done = vec![
            step("Read", StepState::Completed),
            step("Fix", StepState::InProgress),
        ];
        let stale = store.add_chat(None, "stale").unwrap();
        let other = store.add_chat(None, "other").unwrap();
        let busy = store.add_chat(None, "busy").unwrap();
        for chat in [stale, other, busy] {
            store
                .keep_entry(chat, 0, &Entry::Human("plan it".to_owned()))
                .unwrap();
            store
                .keep_entry(chat, 1, &Entry::Plan(half_done.clone()))
                .unwrap();
        }
        let listed = |key: u64, kept: ChatId, working: bool| {
            let mut listed = chat(key, &root, &root, false, false);
            listed.kept = Some(kept);
            listed.working = working;
            listed
        };
        let told = Told {
            chats: vec![
                listed(1, stale, false),
                listed(2, other, false),
                listed(3, busy, true),
            ],
            projects: Vec::new(),
            home: root.join("home"),
            ..Told::default()
        };
        // What the page is told after a restart: the plans and those
        // dismissed read again from where they are kept.
        let page = || {
            told_of(
                &told,
                &root,
                &Plans {
                    kept: &plans(),
                    on: &[],
                    opened: &[],
                    dismissed: &Dismissed::read(),
                    moved: &NO_MOVED,
                    at: 0,
                },
                &|_| None,
            )
        };
        let rows = || -> Vec<u64> {
            page()["plans"]
                .as_array()
                .unwrap()
                .iter()
                .map(|chat| chat["key"].as_u64().unwrap())
                .collect()
        };
        // As the page's button asks, with what it was told of the first.
        let dismiss_first = || dismiss(&json!([page()["plans"][0]["dismiss"]]).to_string());
        assert_eq!(rows(), [1, 2, 3]);
        assert_eq!(page()["plans"][2]["dismiss"], Value::Null, "at work");

        dismiss_first();
        assert_eq!(rows(), [2, 3], "dismissed, read again as after a restart");
        assert_eq!(page()["busy"], 1);

        // The agent's update to the plan in the same turn shows it again.
        let updated = vec![
            step("Read", StepState::Completed),
            step("Fix", StepState::Completed),
            step("Test", StepState::Pending),
        ];
        store
            .keep_entry(stale, 1, &Entry::Plan(updated.clone()))
            .unwrap();
        assert_eq!(rows(), [1, 2, 3], "an update shows it again");
        dismiss_first();
        assert_eq!(rows(), [2, 3]);
        // So does the same plan sent again in a later turn.
        store
            .keep_entry(stale, 2, &Entry::Human("again".to_owned()))
            .unwrap();
        store.keep_entry(stale, 3, &Entry::Plan(updated)).unwrap();
        assert_eq!(rows(), [1, 2, 3], "a new plan shows again");
        coxd::storage::keep_on_this_thread(None);
    }

    #[test]
    fn counts_the_steps_done_in_each_window_of_the_folder() {
        let folder = TempDir::new();
        let root = folder.path().join("app");
        let minute = 60 * 1000;
        let at = 100 * 24 * 60 * minute;
        let mut chats: Vec<Chat> = (1..=3)
            .map(|key| {
                let mut chat = chat(key, &root, &root, false, false);
                chat.kept = Some(kept(key));
                chat
            })
            .collect();
        chats[1].parent = Some(1);
        // Not of the folder: it moves, but elsewhere.
        let elsewhere = &mut chats[2];
        (elsewhere.cwd, elsewhere.project) = (folder.path().join("x"), folder.path().join("x"));
        let told = Told {
            chats,
            projects: vec![(root.clone(), "app".to_owned())],
            home: folder.path().to_owned(),
            ..Told::default()
        };
        let moved = Progress {
            active: vec![
                (kept(1), at - minute),
                (kept(2), at - 2 * 60 * minute),
                (kept(3), at - minute),
            ],
            done: vec![
                (kept(1), "Fix".to_owned(), at - 30 * minute),
                (kept(1), "Read".to_owned(), at - 5 * minute),
                (kept(3), "Elsewhere".to_owned(), at - minute),
                // A subagent's own.
                (kept(2), "Its own".to_owned(), at - 3 * 60 * minute),
            ],
            resolved: Vec::new(),
        };
        let plans = Plans {
            moved: &moved,
            at,
            ..NO_PLANS
        };

        let json = told_of(&told, &root, &plans, &|_| None);

        assert_eq!(
            json["summaries"],
            json!([
                { "window": "15 min", "steps": 1 },
                { "window": "1 h", "steps": 2 },
                { "window": "4 h", "steps": 3 },
            ])
        );
    }

    #[test]
    fn counts_the_steps_the_subagents_under_a_chat_got_done_at_any_depth_once() {
        let folder = TempDir::new();
        let root = folder.path().join("app");
        let minute = 60 * 1000;
        let at = 100 * 24 * 60 * minute;
        let listed = |key: u64, kept_as: u64, parent: Option<u64>| {
            let mut listed = chat(key, &root, &root, false, false);
            listed.kept = Some(kept(kept_as));
            listed.parent = parent;
            listed
        };
        let told = Told {
            chats: vec![
                // The chat at the top hands its work out, with no plan of
                // its own; its subagent opens one more.
                listed(1, 1, None),
                listed(2, 2, Some(1)),
                listed(3, 3, Some(2)),
                // Another of the window's chats that stands for the
                // subagent's, as coxd keeps it.
                listed(4, 2, Some(1)),
                listed(5, 5, None),
            ],
            projects: vec![(root.clone(), "app".to_owned())],
            home: folder.path().to_owned(),
            ..Told::default()
        };
        let moved = Progress {
            active: [1, 2, 3, 5]
                .into_iter()
                .map(|chat| (kept(chat), at - minute))
                .collect(),
            done: vec![
                (kept(2), "Add the flag".to_owned(), at - 5 * minute),
                (kept(2), "Test it".to_owned(), at - 10 * minute),
                (kept(3), "Write the docs".to_owned(), at - 40 * minute),
                (kept(3), "Push".to_owned(), at - 2 * 60 * minute),
                (kept(5), "Its own".to_owned(), at - 3 * 60 * minute),
            ],
            resolved: Vec::new(),
        };
        let plans = Plans {
            moved: &moved,
            at,
            ..NO_PLANS
        };

        let json = told_of(&told, &root, &plans, &|_| None);

        let summaries = json["summaries"].as_array().unwrap();
        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary["steps"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [2, 3, 5]
        );
    }

    #[test]
    fn a_subagent_links_to_the_step_its_name_reads_most_alike() {
        let step = |text: &str| Step {
            text: text.to_owned(),
            state: StepState::Pending,
            since: None,
            started: None,
            id: None,
        };
        let plan = [
            step("--shout フラグを追加しテストを書く"),
            step("README を更新する"),
            step("GitHub Actions のワークフローを追加"),
        ];
        let root = Path::new("/p");
        let named = |name: &str| {
            let mut named = chat(1, root, root, false, false);
            name.clone_into(&mut named.name);
            named
        };
        let chats = [
            named("README の更新"),
            named("CI ワークフロー追加"),
            named("Shout flag"),
            named("全体のレビュー"),
        ];
        let chats: Vec<&Chat> = chats.iter().collect();
        assert_eq!(
            linked(&plan, &chats, &[]),
            [Some(1), Some(2), Some(0), None]
        );
    }

    #[test]
    fn a_subagent_links_to_the_step_under_way_when_it_was_opened() {
        // So many seconds after the epoch.
        let second = |seconds: i64| seconds * 1000;
        let step = |text: &str, state: StepState, started: Option<i64>, since: i64| Step {
            text: text.to_owned(),
            state,
            since: Some(second(since)),
            started: started.map(second),
            id: None,
        };
        let plan = [
            step("Add the flag", StepState::Completed, Some(10), 20),
            step("Test it", StepState::Completed, Some(20), 40),
            step("Write the docs", StepState::InProgress, Some(30), 30),
            step("Push", StepState::InProgress, Some(35), 35),
            step("Tag it", StepState::Pending, None, 0),
        ];
        let root = Path::new("/p");
        let opened = |key: u64, name: &str| {
            let mut opened = chat(key, root, root, false, false);
            name.clone_into(&mut opened.name);
            opened.kept = Some(kept(key));
            opened
        };
        let chats = [
            // While the flag was added, though it reads as the tests.
            opened(1, "Test the flag"),
            // While the tests and the docs were under way: the docs, as
            // it reads so.
            opened(2, "Docs for the flag"),
            // While three were: the one started nearest, as none reads so.
            opened(3, "Look around"),
            // Before any step: the step it reads as.
            opened(4, "Tag it"),
            // Not kept by coxd, so not known when: as it reads.
            chat(5, root, root, false, false),
            // As the flag was done and the tests started, kept just after
            // the chat was opened: the tests.
            opened(6, "Look again"),
            // Just before the first step started, kept after the chat was
            // opened: that step, not one started later.
            opened(7, "Look first"),
        ];
        let at = [
            (kept(1), second(15)),
            (kept(2), second(32)),
            (kept(3), second(38)),
            (kept(4), second(5)),
            (kept(6), second(20) - 1),
            (kept(7), second(10) - 6),
        ];
        let chats: Vec<&Chat> = chats.iter().collect();
        assert_eq!(
            linked(&plan, &chats, &at),
            [Some(0), Some(2), Some(3), Some(4), None, Some(1), Some(0)]
        );
    }

    fn pull(number: u64, checks: CheckState, unseen: Option<CheckState>) -> PullRow {
        use coxd::github::{Check, Pull, PullState, Status};
        PullRow {
            pull: Pull {
                repo: "tennashi/cox".to_owned(),
                number,
            },
            status: Some(Status {
                title: format!("pull {number}"),
                state: PullState::Draft,
                head: "a".to_owned(),
                checks: vec![Check {
                    name: "ci".to_owned(),
                    state: checks,
                    link: None,
                    ended: None,
                }],
                times: coxd::github::Times::default(),
                review: None,
                mergeable: coxd::github::Mergeable::Unknown,
                size: None,
            }),
            unseen,
        }
    }

    /// The pull request `number` a project in `project` holds, opened by
    /// the chat `author`, its CI standing as `checks`, and itself as
    /// `state`, merged or closed at `ended`, if it was.
    fn held(
        project: &Path,
        number: u64,
        author: Option<u64>,
        checks: CheckState,
        state: coxd::github::PullState,
        ended: Option<i64>,
    ) -> Pull {
        let mut row = pull(number, checks, None);
        let status = row.status.as_mut().unwrap();
        status.state = state;
        status.checks[0].name = "ci (macos-latest)".to_owned();
        status.times.merged = ended;
        Pull {
            project: project.to_owned(),
            pull: row.pull,
            status: row.status,
            author,
        }
    }

    /// When the page is read in the tests of the pull requests.
    const PULLS_AT: i64 = 10 * DONE_SHOWS;

    /// The projects `app` and `shop` in `home`, and the pull requests they
    /// hold: of `app`, #7, whose CI runs, opened by the chat 1; #8, whose
    /// CI failed, by the chat 2, marked done; #9, whose CI failed, by a
    /// chat gone; #10, merged lately, and #11, merged long ago, by the
    /// chat 1; and of `shop`, #12, opened outside Cox, by no chat.
    fn holding(home: &Path) -> Told {
        use coxd::github::PullState;
        let root = home.join("app");
        let other = home.join("shop");
        let mut done = chat(2, &root, &root, false, false);
        done.resolved = true;
        let at = PULLS_AT;
        Told {
            chats: vec![chat(1, &root, &root, false, false), done],
            projects: vec![
                (root.clone(), "app".to_owned()),
                (other.clone(), "shop".to_owned()),
            ],
            home: home.to_path_buf(),
            pulls: vec![
                held(
                    &root,
                    7,
                    Some(1),
                    CheckState::Pending,
                    PullState::Open,
                    None,
                ),
                held(&root, 8, Some(2), CheckState::Failed, PullState::Open, None),
                held(&root, 9, None, CheckState::Failed, PullState::Draft, None),
                held(
                    &root,
                    10,
                    Some(1),
                    CheckState::Passed,
                    PullState::Merged,
                    Some(at - 1000),
                ),
                held(
                    &root,
                    11,
                    Some(1),
                    CheckState::Passed,
                    PullState::Merged,
                    Some(at - DONE_SHOWS),
                ),
                held(&other, 12, None, CheckState::Passed, PullState::Open, None),
            ],
            ..Told::default()
        }
    }

    #[test]
    fn the_pull_requests_of_the_project_show_with_their_ci_and_the_chat_that_opened_each() {
        let home = TempDir::new();
        let told = holding(&home);
        let root = home.join("app");
        let at = PULLS_AT;
        let plans = Plans { at, ..NO_PLANS };
        let json = told_of(&told, &root, &plans, &|_| None);
        let shown: Vec<(u64, &Value, &Value, &Value, &Value)> = json["pulls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pull| {
                (
                    pull["number"].as_u64().unwrap(),
                    &pull["state"],
                    &pull["ci"],
                    &pull["author"]["key"],
                    &pull["fix"],
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                (
                    7,
                    &json!("open"),
                    &json!("running"),
                    &json!(1),
                    &json!(false)
                ),
                // The chat that opened it is done: it may be asked to fix it.
                (8, &json!("open"), &json!("failed"), &json!(2), &json!(true)),
                // No chat is left to ask.
                (
                    9,
                    &json!("draft"),
                    &json!("failed"),
                    &Value::Null,
                    &json!(false)
                ),
                // Merged lately; one merged long ago is gone.
                (10, &json!("merged"), &Value::Null, &json!(1), &json!(false)),
            ]
        );
        let failed = &json["pulls"][1];
        assert_eq!(failed["url"], "https://github.com/tennashi/cox/pull/8");
        assert_eq!(failed["title"], "pull 8");
        assert_eq!(failed["failed"], json!(["ci (macos-latest)"]));
        assert_eq!(failed["author"]["name"], "chat 2");
        assert_eq!(failed["author"]["resolved"], true);
        // The home tells of every project's, with where each is.
        let json = told_of(&told, &home, &plans, &|_| None);
        let pulls = json["pulls"].as_array().unwrap();
        assert_eq!(pulls.len(), 5);
        assert_eq!(pulls[4]["where"], "shop");
        // One opened outside Cox shows with no chat, and none to ask.
        assert_eq!(pulls[4]["number"], 12);
        assert_eq!(pulls[4]["state"], "open");
        assert_eq!(pulls[4]["author"], Value::Null);
        assert_eq!(pulls[4]["fix"], false);
    }

    #[test]
    fn a_pull_request_tells_its_checks_review_conflicts_size_and_times() {
        let home = TempDir::new();
        let mut told = holding(&home);
        let root = home.join("app");
        {
            let status = told.pulls[1].status.as_mut().unwrap();
            let check = |name: &str, state| coxd::github::Check {
                name: name.to_owned(),
                state,
                link: None,
                ended: None,
            };
            status.checks.extend([
                check("clippy", CheckState::Passed),
                check("fmt", CheckState::Passed),
                check("docs", CheckState::Pending),
                check("deploy", CheckState::Skipped),
            ]);
            status.review = Some(coxd::github::Review::ChangesRequested);
            status.mergeable = coxd::github::Mergeable::Conflicting;
            status.size = Some(coxd::github::Size {
                added: 412,
                removed: 96,
                files: 14,
            });
            status.times.opened = Some(PULLS_AT - 60_000);
        }
        let plans = Plans {
            at: PULLS_AT,
            ..NO_PLANS
        };
        let json = told_of(&told, &root, &plans, &|_| None);
        let pull = &json["pulls"][1];
        assert_eq!(
            pull["checks"],
            json!({"passed": 2, "failed": 1, "running": 1, "skipped": 1, "total": 5})
        );
        assert_eq!(pull["review"], "changes");
        assert_eq!(pull["conflicts"], true);
        assert_eq!(
            pull["size"],
            json!({"added": 412, "removed": 96, "files": 14})
        );
        assert_eq!(pull["opened"], PULLS_AT - 60_000);
        assert_eq!(pull["ended"], Value::Null);
        // One read before Cox read reviews: none asked for, no conflicts
        // known, its size unknown; merged, it tells when.
        let merged = &json["pulls"][3];
        assert_eq!(merged["review"], Value::Null);
        assert_eq!(merged["conflicts"], false);
        assert_eq!(merged["size"], Value::Null);
        assert_eq!(merged["ended"], PULLS_AT - 1000);
    }

    #[test]
    fn the_chat_that_opened_a_pull_request_whose_ci_failed_is_asked_to_fix_it() {
        let home = TempDir::new();
        let told = holding(&home);
        // Asked to fix one, the chat that opened it is sent a line that
        // says what failed.
        let fix = |key, number| Fix {
            key,
            repo: "tennashi/cox".to_owned(),
            number,
        };
        assert_eq!(
            fixes(&told, &fix(2, 8)),
            Some(Say {
                key: 2,
                text: "CI failed on #8 (https://github.com/tennashi/cox/pull/8): \
                       ci (macos-latest). Fix it."
                    .to_owned(),
                step: None,
            })
        );
        // Not one whose CI did not fail, nor by another chat.
        assert_eq!(fixes(&told, &fix(1, 7)), None);
        assert_eq!(fixes(&told, &fix(1, 8)), None);
        assert_eq!(fixes(&told, &fix(1, 9)), None);
    }

    #[test]
    fn a_chat_cut_off_waits_for_the_human_with_why_and_goes_on_when_asked() {
        let root = TempDir::new();
        let limit = |why: &str| TurnEnd {
            at: 5,
            how: Ended::Limited,
            why: Some(why.to_owned()),
        };
        // Cut off, and unread too: cut off says more.
        let mut cut = chat(1, &root, &root, false, true);
        cut.cut_off = Some(limit("You've hit your limit · resets 3pm"));
        // A subagent cut off shows under its opener, at work.
        let mut opener = chat(2, &root, &root, false, false);
        opener.working = true;
        let mut sub = chat(3, &root, &root, false, false);
        sub.parent = Some(2);
        sub.cut_off = Some(TurnEnd {
            at: 6,
            how: Ended::Closed,
            why: Some(COXD_WENT.to_owned()),
        });
        // Cut off, but at work again, marked done, or archived: it does
        // not wait.
        let mut again = chat(4, &root, &root, false, false);
        again.cut_off = Some(limit("limit"));
        again.working = true;
        let mut done = chat(5, &root, &root, false, false);
        done.cut_off = Some(limit("limit"));
        done.resolved = true;
        let mut archived = chat(6, &root, &root, false, false);
        archived.cut_off = Some(limit("limit"));
        archived.archived = true;
        // Cut off and asking: it waits, but goes on only once answered.
        let mut asking = chat(7, &root, &root, true, false);
        asking.cut_off = Some(limit("limit"));
        let told = Told {
            chats: vec![cut, opener, sub, again, done, archived, asking],
            projects: vec![(root.to_path_buf(), "app".to_owned())],
            home: root.join("home"),
            ..Told::default()
        };

        let json = told_in(&told, &root);
        let waiting: Vec<(u64, &str)> = json["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| (chat["key"].as_u64().unwrap(), chat["why"].as_str().unwrap()))
            .collect();
        assert_eq!(waiting, [(1, "cut off"), (3, "cut off"), (7, "asks you")]);
        assert_eq!(waiting_of(&told, &root), 3);
        assert_eq!(
            json["waiting"][0]["cut_off"],
            json!({
                "how": "limited",
                "at": 5,
                "says": "You've hit your limit · resets 3pm",
            })
        );
        assert_eq!(json["waiting"][0]["resume"], true);
        assert_eq!(json["waiting"][2]["resume"], false, "asking");
        let sub = json["plans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|chat| chat["key"] == 3)
            .expect("the subagent cut off is listed under its opener");
        assert_eq!(sub["waits"], "cut off");
        assert_eq!(sub["cut_off"]["says"], COXD_WENT);
        assert_eq!(sub["resume"], true);

        // Asked to go on, the chat cut off is sent the line, and no other.
        let resume = |key| resumes(&told, &Resume { key });
        assert_eq!(
            resume(1),
            Some(Say {
                key: 1,
                text: RESUME.to_owned(),
                step: None,
            })
        );
        assert_eq!(resume(3).map(|say| say.key), Some(3));
        for key in [2, 4, 5, 7, 99] {
            assert_eq!(resume(key), None, "chat {key}");
        }
    }

    #[test]
    fn a_turn_cut_off_says_why_as_the_agent_said() {
        let says = |how, why: Option<&str>| cut_off_says(how, why);
        assert_eq!(says(Ended::Limited, None), "usage limit reached");
        assert_eq!(says(Ended::Limited, Some(" resets 3pm ")), "resets 3pm");
        assert_eq!(says(Ended::Failed, Some("lost")), "the turn failed: lost");
        assert_eq!(says(Ended::Closed, None), "the agent closed");
        assert_eq!(
            says(Ended::Closed, Some("crashed")),
            "the agent closed: crashed"
        );
        assert_eq!(says(Ended::Closed, Some(COXD_WENT)), COXD_WENT);
        assert_eq!(
            says(Ended::TooLong, None),
            "the agent's reply grew too long"
        );
    }

    #[test]
    fn a_chat_waiting_on_ci_is_listed_as_such_and_one_whose_ci_ended_unseen_waits() {
        let root = TempDir::new();
        let mut running = chat(1, &root, &root, false, false);
        running.pulls = vec![pull(423, CheckState::Pending, None)];
        let mut ended = chat(2, &root, &root, false, false);
        ended.pulls = vec![pull(421, CheckState::Failed, Some(CheckState::Failed))];
        let told = Told {
            chats: vec![running, ended],
            projects: vec![(root.to_path_buf(), "app".to_owned())],
            home: root.join("home"),
            github: Some("gh is not logged in".to_owned()),
            ..Told::default()
        };
        let json = told_in(&told, &root);
        let plans = json["plans"].as_array().unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0]["key"], 1);
        assert_eq!(plans[0]["waits"], "waits on CI");
        assert_eq!(
            plans[0]["pulls"],
            json!([{"number": 423, "title": "pull 423", "draft": true, "ci": "running"}])
        );
        let waiting = json["waiting"].as_array().unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0]["key"], 2);
        assert_eq!(waiting[0]["why"], "CI failed on #421");
        assert_eq!(json["github"], "gh is not logged in");
    }
}
