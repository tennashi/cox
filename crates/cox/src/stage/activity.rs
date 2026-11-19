//! What goes on in a folder, as the stage's Activity page shows it, at
//! `cox://api/activity` (see [`crate::stage::plugins`]): the human's
//! inbox, where they look first when they open Cox or have a moment. It
//! tells, in tiers, what is asked of them, the chats of the folder that
//! ask them something, with what they ask, or whose turn ended while they
//! were not looking, or CI ended on a pull request their agent opened;
//! the plans, of the chats whose agents take a turn, or work in the
//! background once it ended, or wait on CI, or whose plan, as the agent
//! last sent it over ACP, is not done, or done while the human did not
//! look yet, or lately, or that were marked done lately, each with what
//! it does now, or runs in the background, its open pull requests, and
//! the steps of its plan, each with the subagents' chats whose names
//! read alike it (see [`linked`]), the other subagents at work under
//! it; what was decided, the questions agents
//! asked there that the human answered, each with what they chose and
//! why, the last first; and what happened without them, the schedules
//! that ran there, the pages of the wiki written from there, and the
//! sources kept there, the last first. The home, where Cox lands, tells
//! of every folder.
//!
//! The window tells this module how its chats stand as they change (see
//! [`tell`]), since the stage's pages are served apart from it; the bar
//! counts what waits on the Activity's button (see [`waiting`]). What a
//! chat does now, and what it asks, come from what the window told the
//! Commands page of its conversation (see [`commands::now`]). What was
//! decided, and the plans, are read from the chats as coxd keeps them
//! (see [`Store::answered`] and [`Store::plans`]), so they hold for chats
//! the window never showed, and last as long as their conversations do:
//! what was decided read again once a chat stops asking, as when the
//! human answers, and once one goes, and the plans each time. A plan the
//! human dismissed from the page stays hidden until the chat's agent
//! sends another (see [`dismiss`]).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use cox_core::entry::{Entry, OnStep, QuestionAnswer, Step, StepsDone, alike};
use cox_core::store::{ChatId, KeptAnswer, KeptPlan, Progress, ResolvedBy, Said, Store};
use jiff::tz::TimeZone;
use serde_json::{Value, json};

use crate::stage::commands;

mod timeline;
use cox_acp::Microdollars;
use coxd::chat::spent::Spent;
use coxd::daemon::wire::{OnStepLine, PullRow};
use coxd::github::CheckState;
pub use timeline::{Of, timeline};

/// How many of what happened the page is told of, the last first.
const HAPPENED: usize = 50;
/// How many of what was decided the page is told of, the last first.
const DECIDED: usize = 20;
/// The windows the Plans tier sums up what moved in, each by its name
/// and its length in milliseconds, the shortest first.
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
/// How many steps done a chat's line in a summary names, the last done
/// first, before it says how many more.
const NAMED: usize = 3;
/// How many times what was decided is read again after a chat stopped
/// asking: coxd may tell the window so before it keeps the answer.
const READ_AGAIN: u8 = 2;

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
    /// How coxd keeps it, which what it decided is read by.
    pub kept: Option<ChatId>,
    /// The pull requests its agent opened, as coxd last read them.
    pub pulls: Vec<PullRow>,
    /// What its agent spent, once it told any of it.
    pub spent: Option<Spent>,
    /// Whether it is marked done: it leaves the page, but for a step of
    /// the plan of the chat that opened it, and what it spent there.
    pub resolved: bool,
    /// Whether the human archived it: it waits for them only while it
    /// asks.
    pub archived: bool,
    /// Whether a line sent now goes into the turn its agent takes, as it
    /// asks the human something too, if it does.
    pub steers: bool,
    /// The lines written on steps of its plan that wait for its turn to
    /// end, the first first.
    pub on_steps: Vec<OnStepLine>,
}

impl Chat {
    /// Whether it waits for the human, as the bell counts it.
    fn waits(&self) -> bool {
        !self.resolved
            && (self.asking || (!self.archived && (self.unread || !self.ci_ended().is_empty())))
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

/// A question an agent asked in a chat, which the human answered: what
/// the Decided tier lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decided {
    pub chat: ChatId,
    /// The name of the chat, as coxd keeps it.
    pub name: String,
    /// The folder of the chat's project, or `None` for Cox's own.
    pub project: Option<PathBuf>,
    /// The folder the chat works in, when it is not its project's.
    pub folder: Option<PathBuf>,
    /// When it was asked, as `2026-11-16 09:30`.
    pub when: String,
    /// When it was answered, or asked where an older Cox kept no time of
    /// the answer, in milliseconds since the epoch.
    pub at: i64,
    /// The question's short label, as "Approach", if the agent gave one.
    pub header: Option<String>,
    pub question: String,
    /// The labels of the options the human chose, or their own words when
    /// they chose none.
    pub choice: String,
    /// What the agent said choosing those options means, which is why
    /// the human chose them.
    pub why: Option<String>,
    /// The human's own words beside the options they chose.
    pub other: Option<String>,
}

impl Decided {
    /// What was decided by `kept`, each of its questions the human
    /// answered, its time as it reads in `zone`.
    fn of(kept: &KeptAnswer, zone: &TimeZone) -> Vec<Self> {
        let Entry::Question {
            message,
            questions,
            answer: Some(QuestionAnswer::Given(picked)),
            ..
        } = &kept.entry
        else {
            return Vec::new();
        };
        let when = jiff::Timestamp::from_millisecond(kept.at).map_or_default(|at| {
            at.to_zoned(zone.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        });
        let message = message.trim();
        questions
            .iter()
            .zip(picked)
            .filter_map(|(question, picked)| {
                let chosen: Vec<_> = picked
                    .chosen
                    .iter()
                    .filter_map(|&index| question.options.get(index))
                    .collect();
                let mut other = picked
                    .other
                    .clone()
                    .filter(|other| !other.trim().is_empty());
                let choice = if chosen.is_empty() {
                    other.take()?
                } else {
                    chosen
                        .iter()
                        .map(|option| option.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let why: Vec<&str> = chosen
                    .iter()
                    .filter_map(|option| option.description.as_deref())
                    .collect();
                let asked = question
                    .text
                    .clone()
                    .or_else(|| {
                        (questions.len() == 1 && !message.is_empty()).then(|| message.to_owned())
                    })
                    .or_else(|| question.header.clone())
                    .unwrap_or_else(|| message.to_owned());
                Some(Self {
                    chat: kept.chat,
                    name: kept.name.clone(),
                    project: kept.project.clone(),
                    folder: kept.folder.clone(),
                    when: when.clone(),
                    at: kept.at,
                    header: question.header.clone(),
                    question: asked,
                    choice,
                    why: (!why.is_empty()).then(|| why.join("; ")),
                    other,
                })
            })
            .collect()
    }
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
}

/// How the chats stood when the window last told.
static TOLD: Mutex<Option<Told>> = Mutex::new(None);

/// What was decided, as last read, and how many times to read it again
/// before that holds.
struct Read {
    decided: Option<Vec<Decided>>,
    again: u8,
}

static READ: Mutex<Read> = Mutex::new(Read {
    decided: None,
    again: 0,
});

/// Keeps how the window's chats stand now, for the pages to read, and
/// has what was decided read again if a chat stopped asking, or went.
pub fn tell(told: Told) {
    let Ok(mut now) = TOLD.lock() else {
        return;
    };
    let changed = now.as_ref().is_some_and(|was| decided_since(was, &told));
    *now = Some(told);
    drop(now);
    if changed && let Ok(mut read) = READ.lock() {
        read.again = READ_AGAIN;
    }
}

/// Whether what was decided may have changed from when the chats stood
/// as `was` to how they stand `now`: a chat stopped asking, as when the
/// human answered it, or went, and with it what it decided.
fn decided_since(was: &Told, now: &Told) -> bool {
    was.chats.iter().any(|chat| {
        let still = now.chats.iter().find(|now| now.key == chat.key);
        still.is_none_or(|still| chat.asking && !still.asking)
    })
}

/// What was decided in every chat coxd keeps, the last asked first, read
/// again once what the window told says it changed.
fn decided() -> Vec<Decided> {
    let Ok(mut read) = READ.lock() else {
        return Vec::new();
    };
    if read.decided.is_none() || read.again > 0 {
        read.again = read.again.saturating_sub(1);
        let zone = TimeZone::system();
        let answered = coxd::storage::path(coxd::storage::Kept::Chats)
            .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
            .and_then(|path| Store::read(&path)?.answered());
        match answered {
            Ok(answered) => {
                read.decided = Some(
                    answered
                        .iter()
                        .flat_map(|kept| Decided::of(kept, &zone))
                        .collect(),
                );
            }
            Err(error) => tracing::warn!("what the chats decided is not read: {error:#}"),
        }
    }
    read.decided.clone().unwrap_or_default()
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

/// Whether `chat` waits for the human and is of the folder `root`, or
/// of any, when `root` is the home of `told`.
fn waits_in(told: &Told, chat: &Chat, root: &Path) -> bool {
    chat.waits() && of_folder(told, chat, root)
}

/// How many chats of the folder `root` wait for the human, for the bar.
#[must_use]
pub fn waiting(root: &Path) -> usize {
    let told = now();
    told.chats
        .iter()
        .filter(|chat| waits_in(&told, chat, root))
        .count()
}

/// What goes on in the folder `root`, as JSON, from how the chats stand
/// now, what they do and ask, as the Commands page was told, what they
/// decided, the human's `wiki`, the folder's own, the sources at `raw`,
/// and `triggers`, as [`coxd::trigger::told`] tells them, of which the
/// schedules ran.
#[must_use]
pub fn told(root: &Path, wiki: &Path, raw: &Path, triggers: &Value) -> Value {
    let at = jiff::Timestamp::now().as_millisecond();
    let kept = plans();
    let opened = opened();
    told_of(
        &now(),
        root,
        &decided(),
        &Plans {
            on: &on_steps(&kept),
            opened: &opened,
            kept: &kept,
            dismissed: &Dismissed::read(),
            moved: &progress(at - DONE_SHOWS),
            at,
        },
        &commands::now,
        &Places {
            wiki,
            raw,
            triggers,
        },
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

/// A line the human wrote on a step of a chat's plan, from the page, for
/// the window to send the chat (see [`listen`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Say {
    /// The chat, by the number the window goes by for it.
    pub key: u64,
    pub text: String,
    pub step: OnStep,
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
        step: OnStep {
            id: asked.open.step,
            text: step.text.clone(),
        },
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

/// The step of `plan` each of `subagents` works on, by its place in the
/// plan: the step its chat's name reads most alike, once alike enough.
fn linked(plan: &[Step], subagents: &[&Chat]) -> Vec<Option<usize>> {
    subagents
        .iter()
        .map(|chat| {
            plan.iter()
                .enumerate()
                .map(|(at, step)| (at, alike(&step.text, &chat.name)))
                .filter(|(_, score)| *score >= LINKED)
                .max_by(|one, other| one.1.total_cmp(&other.1))
                .map(|(at, _)| at)
        })
        .collect()
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

/// Where what happened in a folder is read from: the human's wiki, their
/// sources, and the triggers, as [`coxd::trigger::told`] tells them.
struct Places<'a> {
    wiki: &'a Path,
    raw: &'a Path,
    triggers: &'a Value,
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
                } else if chat.unread {
                    "finished a turn".to_owned()
                } else {
                    chat.ci_ended().join(", ")
                },
                "asks": chat.asking.then(|| now(chat.key).and_then(|now| now.asks)).flatten(),
                "goal": chat.goal,
                "sketch": chat.cwd != chat.project,
                "where": where_of(told, &chat.project),
            })
        })
        .collect()
}

/// The chats of the folder `root`, or of every folder in the home, at
/// work, or with a plan not done, or done not [`DONE_SHOWS`] ago, or
/// marked done not as long ago, or while it asks the human or they did
/// not look, as JSON: each with what
/// it does now, as `now` says, and the steps of its plan, of `plans`,
/// each with the subagents' chats that work on it, and after it its
/// other subagents, all of them under a plan, those at work otherwise.
fn planned(
    told: &Told,
    root: &Path,
    plans: &Plans<'_>,
    now: &dyn Fn(u64) -> Option<commands::Now>,
) -> Vec<Value> {
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
    };
    let listed_chats: Vec<&Chat> = told
        .chats
        .iter()
        .filter(|chat| {
            plans.stays(chat) && of_folder(told, chat, root) && (shown(chat) || chat.waits_on_ci())
        })
        .collect();
    let mut listed = Vec::new();
    let mut under: Vec<(&Chat, usize)> = listed_chats
        .iter()
        .rev()
        .filter(|chat| {
            !chat
                .parent
                .is_some_and(|parent| listed_chats.iter().any(|other| other.key == parent))
        })
        .map(|chat| (*chat, 0))
        .collect();
    while let Some((chat, depth)) = under.pop() {
        let does = chat
            .working
            .then(|| now(chat.key))
            .flatten()
            .unwrap_or_default();
        let plan = plans.of(chat);
        let steps = plan.map_or(&[][..], |plan| plan.steps.as_slice());
        let subagents: Vec<&Chat> = told
            .chats
            .iter()
            .filter(|other| other.parent == Some(chat.key))
            .collect();
        let links = linked(steps, &subagents);
        listed.push(json!({
            "key": chat.key,
            "chat": chat.kept, "takes": takes_a_line(chat), "steers": chat.steers,
            "name": chat.name,
            "goal": chat.goal,
            "depth": depth,
            "working": chat.working,
            "waits": waits_of(chat, depth),
            "sketch": chat.cwd != chat.project,
            "where": where_of(told, &chat.project),
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
            "plan": steps_of(told, steps, plans.on_of(chat), &subagents, &links, now),
            // How far the plan went, which the page tells rather than
            // count again.
            "progress": StepsDone::count(steps),
            "finished": StepsDone::count(steps).finished(),
            // Only a plan that keeps a chat listed on its own is
            // dismissed: one at work shows anyway.
            "dismiss": plan
                .filter(|_| !chat.working && !steps.is_empty())
                .map(|plan| json!({ "chat": plan.chat, "plan": plan.number })),
        }));
        under.extend(
            subagents
                .iter()
                .zip(&links)
                .rev()
                .filter(|(other, link)| {
                    link.is_none()
                        && plans.stays(other)
                        && (!steps.is_empty() || listed_chats.contains(other))
                })
                .map(|(other, _)| (*other, depth + 1)),
        );
    }
    listed
}

/// What `chat`, listed `depth` under the chat at the top of its, waits
/// on, in words, or `None` while it is at work: marked done, it is done.
fn waits_of(chat: &Chat, depth: usize) -> Option<&'static str> {
    if chat.resolved {
        Some("done")
    } else if chat.working {
        None
    } else if chat.asking {
        Some("asks you")
    } else if chat.unread {
        Some("finished a turn")
    } else if chat.waits_on_ci() {
        Some("waits on CI")
    } else if depth > 0 {
        Some("finished")
    } else {
        Some("waits")
    }
}

/// The chat of `told` at the top of `chat`'s: the one whose agent opened
/// the one that opened it, and so on, as far as the window lists them.
fn top_of<'a>(told: &'a Told, chat: &'a Chat) -> &'a Chat {
    let mut top = chat;
    while let Some(parent) = top
        .parent
        .and_then(|parent| told.chats.iter().find(|other| other.key == parent))
    {
        top = parent;
    }
    top
}

/// What moved in the folder `root`, or in every folder in the home, over
/// each of [`WINDOWS`], as JSON, from what `plans` read moved and the
/// questions answered of `decided`: each chat at the top of those that
/// moved, by what its own agent's plan got done, the subagents' chats
/// under it that reported or were marked done, and those that worked;
/// and, as things stand, the chats that wait on the human, and those
/// that stopped with their plan not done, with when they last moved.
fn summaries(told: &Told, root: &Path, plans: &Plans<'_>, decided: &[Decided]) -> Vec<Value> {
    let zone = TimeZone::system();
    // The time of day, and the day too once it is not today's hour.
    let clock = |at: i64| {
        let format = if plans.at - at < 20 * 60 * 60 * 1000 {
            "%H:%M"
        } else {
            "%b %-d %H:%M"
        };
        jiff::Timestamp::from_millisecond(at)
            .map_or_default(|at| at.to_zoned(zone.clone()).strftime(format).to_string())
    };
    let chats: Vec<&Chat> = told
        .chats
        .iter()
        .filter(|chat| of_folder(told, chat, root))
        .collect();
    let kept = |chat: &Chat, id: ChatId| chat.kept == Some(id);
    let waits: Vec<Value> = chats
        .iter()
        .filter(|chat| !chat.resolved && (chat.asking || (chat.unread && !chat.archived)))
        .map(|chat| {
            json!({
                "key": chat.key,
                "name": chat.name,
                "why": if chat.asking { "asks you" } else { "finished a turn" },
            })
        })
        .collect();
    let stopped: Vec<Value> = chats
        .iter()
        .filter(|chat| !chat.resolved && !chat.working && !chat.asking && !chat.unread)
        .filter_map(|chat| {
            let plan = plans.of(chat)?;
            let done = StepsDone::count(&plan.steps);
            (done.left() > 0).then(|| {
                json!({
                    "key": chat.key,
                    "name": chat.name,
                    "done": done.done,
                    "of": done.of,
                    "since": clock(plan.at),
                })
            })
        })
        .collect();
    WINDOWS
        .iter()
        .map(|&(name, long)| {
            let mut summed = summed(told, &chats, plans.moved, plans.at - long);
            summed["window"] = json!(name);
            summed["answered"] = json!(
                decided
                    .iter()
                    .filter(|decided| {
                        decided.at >= plans.at - long
                            && chats.iter().any(|chat| kept(chat, decided.chat))
                    })
                    .count()
            );
            summed["waits"] = json!(waits);
            summed["stopped"] = json!(stopped);
            summed
        })
        .collect()
}

/// A chat at the top of those that moved in a window, as [`summed`] sums
/// it up: the steps its agent's plan got done, the last first, and the
/// subagents' chats under it that `ended`, as they reported or were
/// marked done, and that `worked`.
struct Line<'a> {
    chat: &'a Chat,
    done: Vec<(&'a str, i64)>,
    ended: Vec<&'a Chat>,
    worked: Vec<&'a Chat>,
}

/// What moved among `chats` of `told` since `start`, as `moved` read it,
/// as JSON: how many steps the plans of the chats at the top were done,
/// which the lines name, how many subagents reported, and
/// each chat at the top of those that moved, as a [`Line`].
fn summed(told: &Told, chats: &[&Chat], moved: &Progress, start: i64) -> Value {
    let mut lines: Vec<Line<'_>> = Vec::new();
    let (mut steps, mut reports) = (0, 0);
    for chat in chats {
        let kept = |id: ChatId| chat.kept == Some(id);
        let done: Vec<(&str, i64)> = moved
            .done
            .iter()
            .filter(|(id, _, at)| kept(*id) && *at >= start)
            .map(|(_, text, at)| (text.as_str(), *at))
            .collect();
        let resolved = moved
            .resolved
            .iter()
            .find(|(id, resolved)| kept(*id) && resolved.at >= start)
            .map(|(_, resolved)| resolved);
        let active = moved
            .active
            .iter()
            .any(|(id, at)| kept(*id) && *at >= start);
        if done.is_empty() && resolved.is_none() && !active {
            continue;
        }
        let top = top_of(told, chat);
        let at = lines
            .iter()
            .position(|line| line.chat.key == top.key)
            .unwrap_or_else(|| {
                lines.push(Line {
                    chat: top,
                    done: Vec::new(),
                    ended: Vec::new(),
                    worked: Vec::new(),
                });
                lines.len() - 1
            });
        let line = &mut lines[at];
        if top.key == chat.key {
            steps += done.len();
            line.done.extend(done);
        } else if let Some(resolved) = resolved {
            if resolved.by == ResolvedBy::Agent {
                reports += 1;
            }
            line.ended.push(chat);
        } else {
            line.worked.push(chat);
        }
    }
    let named = |chats: &[&Chat]| -> Vec<Value> {
        chats
            .iter()
            .map(|chat| json!({ "key": chat.key, "name": chat.name, "working": chat.working }))
            .collect()
    };
    json!({
        "steps": steps,
        "reports": reports,
        "moved": lines
            .iter_mut()
            .map(|line| {
                line.done.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
                json!({
                    "key": line.chat.key,
                    "name": line.chat.name,
                    "done": line.done.iter().take(NAMED).map(|(text, _)| text).collect::<Vec<_>>(),
                    "more": line.done.len().saturating_sub(NAMED),
                    "ended": named(&line.ended),
                    "worked": named(&line.worked),
                })
            })
            .collect::<Vec<_>>(),
    })
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

/// The steps of a plan, as JSON, each with its id, how many lines the
/// human wrote on it, as `on` counts them, those of `subagents` that
/// `links` link to it, what they do now, as `now` says, and what they
/// spent, with the chats under them, of `told`.
fn steps_of(
    told: &Told,
    steps: &[Step],
    on: &[(u64, usize)],
    subagents: &[&Chat],
    links: &[Option<usize>],
    now: &dyn Fn(u64) -> Option<commands::Now>,
) -> Vec<Value> {
    steps
        .iter()
        .enumerate()
        .map(|(at, step)| {
            let chats: Vec<Value> = subagents
                .iter()
                .zip(links)
                .filter(|(_, link)| **link == Some(at))
                .map(|(chat, _)| {
                    json!({
                        "key": chat.key,
                        "name": chat.name,
                        "working": chat.working,
                        "pulls": pulls_of(chat),
                        "spent": spent_of(told, chat),
                        "said": chat
                            .working
                            .then(|| now(chat.key).and_then(|now| now.said))
                            .flatten(),
                    })
                })
                .collect();
            let written = step
                .id
                .and_then(|id| on.iter().find(|(on, _)| *on == id))
                .map_or(0, |(_, count)| *count);
            json!({
                "text": step.text,
                "state": step.state,
                "id": step.id,
                "on": written,
                "chats": chats,
            })
        })
        .collect()
}

/// What was decided in the folder `root`, or in any in the home, the
/// last [`DECIDED`] of `decided`, as JSON: each chat by its key when the
/// window lists it, and by its name there.
fn decided_in(told: &Told, root: &Path, decided: &[Decided]) -> Vec<Value> {
    decided
        .iter()
        .filter(|decided| {
            let project = decided.project.as_deref().unwrap_or(&told.home);
            root == told.home || project == root || decided.folder.as_deref() == Some(root)
        })
        .take(DECIDED)
        .map(|decided| {
            let listed = told
                .chats
                .iter()
                .find(|chat| chat.kept == Some(decided.chat));
            json!({
                "when": decided.when,
                "header": decided.header,
                "question": decided.question,
                "choice": decided.choice,
                "why": decided.why,
                "other": decided.other,
                "chat": listed.map_or(decided.name.as_str(), |chat| chat.name.as_str()),
                "key": listed.map(|chat| chat.key),
                "where": where_of(told, decided.project.as_deref().unwrap_or(&told.home)),
            })
        })
        .collect()
}

/// What goes on in the folder `root`, as [`told`] tells it, the chats
/// standing as `told` says, doing and asking as `now` says, having
/// `decided` what they did, and what happened read from `places`.
fn told_of(
    told: &Told,
    root: &Path,
    decided: &[Decided],
    plans: &Plans<'_>,
    now: &dyn Fn(u64) -> Option<commands::Now>,
    places: &Places<'_>,
) -> Value {
    let Places {
        wiki,
        raw,
        triggers,
    } = *places;
    let home = root == told.home;
    let waiting = waiting_in(told, root, now);
    let planned = planned(told, root, plans, now);
    let summaries = summaries(told, root, plans, decided);
    let busy = told
        .chats
        .iter()
        .filter(|chat| chat.working && of_folder(told, chat, root))
        .count();
    let decided = decided_in(told, root, decided);
    let name = name_of(told, root);
    let mut happened = Vec::new();
    for folder in triggers["folders"].as_array().into_iter().flatten() {
        let place = folder["name"].as_str().unwrap_or("~");
        for trigger in folder["triggers"].as_array().into_iter().flatten() {
            let schedule = &trigger["schedule"];
            let Some(last) = schedule["last"].as_str() else {
                continue;
            };
            let every = schedule["every"].as_str().unwrap_or_default();
            happened.push(json!({
                "when": last,
                "kind": "schedule",
                "title": trigger["name"],
                "says": if home {
                    format!("ran in {place}, {every}")
                } else {
                    format!("ran, {every}")
                },
                "href": "cox://triggers/",
            }));
        }
    }
    let written =
        |root: &Path, shelf: &str, of: &dyn Fn(&str) -> bool, happened: &mut Vec<Value>| {
            let told = coxd::wiki::told::told(root);
            for page in told["pages"].as_array().into_iter().flatten() {
                let name = page["name"].as_str().unwrap_or_default();
                for used in page["used"].as_array().into_iter().flatten() {
                    let what = used["what"].as_str().unwrap_or_default();
                    if of(what) {
                        happened.push(json!({
                            "when": used["day"],
                            "kind": "wiki",
                            "title": page["title"],
                            "says": what,
                            "href": format!("cox://wiki/#{shelf}{name}"),
                        }));
                    }
                }
            }
        };
    if home {
        written(wiki, "", &|_: &str| true, &mut happened);
    } else if let Some(name) = &name {
        let ours = format!("in {name}, chat ");
        written(
            wiki,
            "",
            &|what: &str| what.starts_with(&ours),
            &mut happened,
        );
    }
    if !home {
        written(
            &root.join(coxd::wiki::PROJECT_WIKI),
            "project:",
            &|_: &str| true,
            &mut happened,
        );
    }
    kept_sources(root, raw, home, name.as_deref(), &mut happened);
    happened.sort_by(|a, b| {
        let when = |event: &Value| event["when"].as_str().unwrap_or_default().to_owned();
        when(b).cmp(&when(a))
    });
    happened.truncate(HAPPENED);
    let mut told_json = json!({
        "folder": root,
        "home": home,
        "github": told.github,
        "waiting": waiting,
        "plans": planned,
        "summaries": summaries,
        "busy": busy,
        "decided": decided,
        "happened": happened,
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

/// Pushes onto `happened` the sources kept of the folder `root`, the
/// project of `name`, if it is one: those of the human's sources at
/// `raw` the project kept, or all of them in the home, and those of the
/// project's own.
fn kept_sources(
    root: &Path,
    raw: &Path,
    home: bool,
    name: Option<&str>,
    happened: &mut Vec<Value>,
) {
    let mut sources: Vec<(Value, &str)> = coxd::raw::told(raw)["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|source| (source.clone(), "raw:"))
        .collect();
    if !home {
        sources.extend(
            coxd::raw::told(&root.join(coxd::raw::PROJECT_RAW))["sources"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|source| (source.clone(), "project-raw:")),
        );
    }
    for (source, shelf) in &sources {
        let project = source["project"].as_str();
        let kept_here = home || *shelf == "project-raw:" || (project.is_some() && name == project);
        if kept_here {
            happened.push(json!({
                "when": source["kept"],
                "kind": "source",
                "title": source["title"],
                "says": format!(
                    "kept in {}{}{}",
                    source["topic"].as_str().unwrap_or_default(),
                    source["chat"].as_str().map_or_default(|chat| format!(" by {chat}")),
                    project
                        .filter(|_| home)
                        .map_or_default(|project| format!(" in {project}"))
                ),
                "href": format!("cox://wiki/#{shelf}{}", source["name"].as_str().unwrap_or_default()),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cox_core::entry::StepState;
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
            archived: false,
            steers: false,
            on_steps: Vec::new(),
        }
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
            github: None,
        };
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &[], &NO_PLANS, &|_| None, &places);

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

    /// What goes on in the folder `root`, nothing decided, doing or
    /// asking anything, and what happened read from `wiki`, `raw` and
    /// `triggers`.
    fn told_at(told: &Told, root: &Path, wiki: &Path, raw: &Path, triggers: &Value) -> Value {
        let places = Places {
            wiki,
            raw,
            triggers,
        };
        told_of(told, root, &[], &NO_PLANS, &|_| None, &places)
    }

    /// What goes on in the folder `root`, where nothing happened.
    fn told_in(told: &Told, root: &Path, at: &Path) -> Value {
        told_at(told, root, &at.join("wiki"), &at.join("raw"), &json!({}))
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
            github: None,
        };
        let json = told_in(&told, &root, &root);
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
        let json = told_in(&told, &home, &home);
        let keys: Vec<_> = json["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| chat["key"].as_u64().unwrap())
            .collect();
        assert_eq!(keys, [1, 3, 4]);
    }

    #[test]
    fn a_question_answered_is_decided_with_why_and_a_declined_one_is_not() {
        use cox_core::entry::{Picked, Question, QuestionOption};

        let option = |label: &str, description: Option<&str>| QuestionOption {
            label: label.to_owned(),
            description: description.map(str::to_owned),
        };
        let question = Question {
            header: Some("Approach".to_owned()),
            text: None,
            options: vec![
                option("Derive", Some("One source of truth")),
                option("Record", Some("A table of its own")),
            ],
            multiple: false,
            other: true,
        };
        let words = Question {
            header: Some("Name".to_owned()),
            text: Some("What is it called?".to_owned()),
            options: Vec::new(),
            multiple: false,
            other: true,
        };
        let kept = |answer| KeptAnswer {
            chat: serde_json::from_value(json!(7)).unwrap(),
            name: "plan".to_owned(),
            project: Some(PathBuf::from("/src/app")),
            folder: None,
            at: 1_763_280_000_000,
            entry: Entry::Question {
                message: "Which way do we go?".to_owned(),
                questions: vec![question.clone(), words.clone()],
                answer: Some(answer),
                answered_at: None,
            },
        };
        let given = QuestionAnswer::Given(vec![
            Picked {
                chosen: vec![0],
                other: Some("but keep it cheap".to_owned()),
            },
            Picked {
                chosen: Vec::new(),
                other: Some("inbox".to_owned()),
            },
        ]);

        let decided = Decided::of(&kept(given), &TimeZone::UTC);

        assert_eq!(decided.len(), 2);
        assert_eq!(decided[0].when, "2025-11-16 08:00");
        assert_eq!(decided[0].header.as_deref(), Some("Approach"));
        // Of several questions, one without words of its own is asked by
        // its label, the message being for them all.
        assert_eq!(decided[0].question, "Approach");
        assert_eq!(decided[0].choice, "Derive");
        assert_eq!(decided[0].why.as_deref(), Some("One source of truth"));
        assert_eq!(decided[0].other.as_deref(), Some("but keep it cheap"));
        assert_eq!(decided[1].question, "What is it called?");
        assert_eq!(decided[1].choice, "inbox");
        assert_eq!(decided[1].why, None);
        assert_eq!(decided[1].other, None);
        assert_eq!(
            Decided::of(&kept(QuestionAnswer::Declined), &TimeZone::UTC),
            []
        );
        let skipped = QuestionAnswer::Given(vec![Picked::default(), Picked::default()]);
        assert_eq!(Decided::of(&kept(skipped), &TimeZone::UTC), []);
    }

    #[test]
    fn the_inbox_tells_what_is_asked_at_work_and_decided_in_the_folder() {
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
            github: None,
        };
        let decided = |chat: u64, project: &Path, question: &str| Decided {
            chat: kept(chat),
            name: format!("kept {chat}"),
            project: Some(project.to_owned()),
            folder: None,
            when: "2026-11-16 09:00".to_owned(),
            at: 0,
            header: None,
            question: question.to_owned(),
            choice: "Derive".to_owned(),
            why: Some("One source of truth".to_owned()),
            other: None,
        };
        let decisions = [
            decided(1, &root, "Which way?"),
            decided(8, &other, "Which name?"),
            decided(6, &root, "Which store?"),
        ];
        let now = |key: u64| {
            Some(commands::Now {
                said: Some(format!("step of {key}")),
                running: (key == 3).then(|| ("Wait".to_owned(), "sleep 30".to_owned())),
                asks: (key == 1).then(|| "Which way?".to_owned()),
            })
        };
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &decisions, &NO_PLANS, &now, &places);

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
        let decided = json["decided"].as_array().unwrap();
        assert_eq!(decided.len(), 2);
        // A chat the window lists is linked, by the name it has there.
        assert_eq!(decided[0]["key"], 1);
        assert_eq!(decided[0]["chat"], "chat 1");
        assert_eq!(decided[0]["why"], "One source of truth");
        assert_eq!(decided[1]["key"], Value::Null);
        assert_eq!(decided[1]["chat"], "kept 6");

        // The home tells of every folder.
        let json = told_of(&told, &home, &decisions, &NO_PLANS, &now, &places);
        assert_eq!(json["plans"].as_array().unwrap().len(), 4);
        assert_eq!(json["busy"], 4);
        let questions: Vec<&str> = json["decided"]
            .as_array()
            .unwrap()
            .iter()
            .map(|decided| decided["question"].as_str().unwrap())
            .collect();
        assert_eq!(questions, ["Which way?", "Which name?", "Which store?"]);
        assert_eq!(json["decided"][1]["where"], "other");
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
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &[], &plans, &|_| None, &places);

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
                step: OnStep {
                    id: 1,
                    text: "Rename the setting".to_owned(),
                },
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

        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };
        let json = told_of(&told, &root, &[], &plans, &|_| None, &places);
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
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &[], &plans, &now, &places);

        let rows: Vec<(u64, u64)> = json["plans"]
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
        // A subagent whose name reads as a step shows under the step, not
        // again under the chat; the others show under the chat, done or
        // not, while it has a plan.
        assert_eq!(
            rows,
            [(2, 0), (4, 1), (11, 1), (5, 0), (6, 0), (8, 0), (10, 0)]
        );
        let plan = &json["plans"][0]["plan"];
        assert_eq!(plan[0]["state"], "in_progress");
        assert_eq!(
            plan[0]["chats"],
            json!([{ "key": 3, "name": "Add the --shout flag", "working": true, "pulls": [], "spent": null, "said": "step of 3" }])
        );
        assert_eq!(
            plan[1]["chats"],
            json!([{ "key": 9, "name": "README docs update", "working": false, "pulls": [], "spent": null, "said": null }])
        );
        assert_eq!(plan[2]["chats"], json!([]));
        assert_eq!(json["plans"][2]["waits"], "finished");
        assert_eq!(json["plans"][3]["waits"], "waits");
        // Only the chats at work count as at work.
        assert_eq!(json["busy"], 3);
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
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &[], &plans, &|_| None, &places);

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
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &[], &plans, &|_| None, &places);

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
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &[], &plans, &|_| None, &places);

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
            github: None,
        };
        // What the page is told after a restart: the plans and those
        // dismissed read again from where they are kept.
        let page = || {
            told_of(
                &told,
                &root,
                &[],
                &Plans {
                    kept: &plans(),
                    on: &[],
                    opened: &[],
                    dismissed: &Dismissed::read(),
                    moved: &NO_MOVED,
                    at: 0,
                },
                &|_| None,
                &Places {
                    wiki: &root.join("wiki"),
                    raw: &root.join("raw"),
                    triggers: &json!({}),
                },
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
    fn sums_up_what_moved_in_each_window_and_what_stands_still() {
        use cox_core::store::Resolved;
        let folder = TempDir::new();
        let root = folder.path().join("app");
        let minute = 60 * 1000;
        let at = 100 * 24 * 60 * minute;
        let mut chats: Vec<Chat> = (1..=6)
            .map(|key| {
                let mut chat = chat(key, &root, &root, false, false);
                chat.kept = Some(kept(key));
                chat
            })
            .collect();
        (chats[0].working, chats[1].resolved) = (true, true);
        (chats[1].parent, chats[2].parent) = (Some(1), Some(1));
        chats[4].asking = true;
        // Not of the folder: it moves, but elsewhere.
        let elsewhere = &mut chats[5];
        (elsewhere.cwd, elsewhere.project) = (folder.path().join("x"), folder.path().join("x"));
        let told = Told {
            chats,
            projects: vec![(root.clone(), "app".to_owned())],
            home: folder.path().to_owned(),
            github: None,
        };
        let moved = Progress {
            active: vec![
                (kept(1), at - minute),
                (kept(2), at - 20 * minute),
                (kept(3), at - 2 * 60 * minute),
                (kept(6), at - minute),
            ],
            done: vec![
                (kept(1), "Fix".to_owned(), at - 30 * minute),
                (kept(1), "Read".to_owned(), at - 5 * minute),
                (kept(6), "Elsewhere".to_owned(), at - minute),
                // A subagent's own: its chat stands for it.
                (kept(3), "Its own".to_owned(), at - 3 * 60 * minute),
            ],
            resolved: vec![(
                kept(2),
                Resolved {
                    at: at - 20 * minute,
                    by: ResolvedBy::Agent,
                },
            )],
        };
        let stopped = [KeptPlan {
            chat: kept(4),
            number: 3,
            steps: vec![
                step("Plan", StepState::Completed),
                step("Build", StepState::InProgress),
            ],
            at: at - 3 * 60 * minute,
        }];
        let plans = Plans {
            kept: &stopped,
            moved: &moved,
            at,
            ..NO_PLANS
        };
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &[], &plans, &|_| None, &places);

        let summaries = json["summaries"].as_array().unwrap();
        let line = |window: usize| &summaries[window]["moved"][0];
        assert_eq!(
            summaries
                .iter()
                .map(|summary| (
                    summary["window"].as_str().unwrap(),
                    summary["steps"].as_u64().unwrap(),
                    summary["reports"].as_u64().unwrap(),
                    summary["moved"].as_array().unwrap().len(),
                ))
                .collect::<Vec<_>>(),
            [("15 min", 1, 0, 1), ("1 h", 2, 1, 1), ("4 h", 2, 1, 1)]
        );
        assert_eq!(line(0)["done"], json!(["Read"]));
        assert_eq!(line(1)["done"], json!(["Read", "Fix"]));
        let keys = |chats: &Value| -> Vec<u64> {
            let chats = chats.as_array().unwrap().iter();
            chats.map(|chat| chat["key"].as_u64().unwrap()).collect()
        };
        assert_eq!(keys(&line(1)["ended"]), [2]);
        assert_eq!(keys(&line(1)["worked"]), [0; 0]);
        assert_eq!(keys(&line(2)["worked"]), [3]);
        for summary in summaries {
            assert_eq!(
                summary["waits"],
                json!([{ "key": 5, "name": "chat 5", "why": "asks you" }])
            );
            let stopped = &summary["stopped"];
            assert_eq!(
                (&stopped[0]["key"], &stopped[0]["done"], &stopped[0]["of"]),
                (&json!(4), &json!(1), &json!(2))
            );
            assert_eq!(stopped.as_array().unwrap().len(), 1);
        }
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
        assert_eq!(linked(&plan, &chats), [Some(1), Some(2), Some(0), None]);
    }

    #[test]
    fn what_was_decided_is_read_again_once_a_chat_stops_asking_or_goes() {
        let root = PathBuf::from("/p");
        let told = |chats: Vec<Chat>| Told {
            chats,
            projects: Vec::new(),
            home: PathBuf::from("/home"),
            github: None,
        };
        let asking = told(vec![
            chat(1, &root, &root, true, false),
            chat(2, &root, &root, false, false),
        ]);
        let answered = told(vec![
            chat(1, &root, &root, false, true),
            chat(2, &root, &root, false, false),
        ]);
        let gone = told(vec![chat(1, &root, &root, false, true)]);

        assert!(decided_since(&asking, &answered));
        assert!(!decided_since(&answered, &asking));
        assert!(!decided_since(&answered, &answered));
        assert!(decided_since(&answered, &gone));
    }

    #[test]
    fn what_happened_there_comes_the_last_first() {
        let root = TempDir::new();
        // The human's wiki, apart from the project's own, in `root`.
        let wiki = root.join("human/wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        std::fs::write(
            wiki.join("errors.md"),
            "# Errors\n\nUse anyhow.\n\n## Used in\n\n\
             - 2026-11-05 in app, chat \"one\"\n\
             - 2026-11-06 in other, chat \"two\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("wiki")).unwrap();
        std::fs::write(
            root.join("wiki/people.md"),
            "# People\n\nAsk Ann.\n\n## Used in\n\n- 2026-11-04 in app, chat \"three\"\n",
        )
        .unwrap();
        // The human's sources, apart from the project's own, in `root`.
        let raw = root.join("human/raw");
        let source = coxd::raw::Source {
            from: "https://example.com/",
            title: "News",
            text: "What changed.",
        };
        let kept = |project| coxd::raw::Kept {
            day: "2026-11-07",
            chat: "news",
            chat_id: None,
            project,
        };
        coxd::raw::keep(&raw, "news", source, kept(Some("app"))).unwrap();
        coxd::raw::keep(&raw, "elsewhere", source, kept(Some("other"))).unwrap();
        let schedules = json!({
            "folders": [{"folder": root.path(), "name": null, "triggers": [
                {"name": "triage", "schedule": {"every": "Every 6 hours", "last": "2026-11-07 09:00"}},
                {"name": "never", "schedule": {"every": "Daily 09:00", "last": null}},
                {"name": "ci"},
            ]}],
        });
        let home = root.join("home");
        let told = Told {
            chats: Vec::new(),
            projects: vec![
                (home.clone(), "~".to_owned()),
                (root.to_path_buf(), "app".to_owned()),
            ],
            home: home.clone(),
            github: None,
        };
        let json = told_at(&told, &root, &wiki, &raw, &schedules);
        let kinds: Vec<(&str, &str)> = json["happened"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["kind"].as_str().unwrap(),
                    event["when"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("schedule", "2026-11-07 09:00"),
                ("source", "2026-11-07"),
                ("wiki", "2026-11-05"),
                ("wiki", "2026-11-04"),
            ]
        );
        assert_eq!(json["happened"][2]["href"], "cox://wiki/#errors");
        assert_eq!(json["happened"][3]["href"], "cox://wiki/#project:people");

        // The home tells of what happened in every folder: the wiki's
        // pages written from any, and the sources kept in any, but not
        // the project's own wiki, which only the project shows.
        let schedules = json!({
            "folders": [
                {"folder": home, "name": null, "triggers": []},
                {"folder": root.path(), "name": "app", "triggers": [
                    {"name": "triage", "schedule": {"every": "Every 6 hours", "last": "2026-11-07 09:00"}},
                ]},
            ],
        });
        let json = told_at(&told, &home, &wiki, &raw, &schedules);
        let said: Vec<(&str, &str)> = json["happened"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["kind"].as_str().unwrap(),
                    event["says"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                ("schedule", "ran in app, Every 6 hours"),
                ("source", "kept in elsewhere by news in other"),
                ("source", "kept in news by news in app"),
                ("wiki", "in other, chat \"two\""),
                ("wiki", "in app, chat \"one\""),
            ]
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
            }),
            unseen,
        }
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
        };
        let json = told_in(&told, &root, &root);
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
