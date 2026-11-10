//! What the window shows of a chat, and what the human's input means,
//! without GPUI: the conversation, as the agent's events change it, and
//! the commands the human's lines turn into (see [`crate::agent`]).
//!
//! A chat can have a [`Goal`]: a command, which the chat works until it
//! passes, or words, which it works until an agent judges them met (see
//! [`crate::chat::judge`]). When a turn ends as the agent ends it, the
//! command runs, or the judge is asked; while the goal is not met, Cox
//! sends the agent what the command printed, or what the judge said, as
//! the next turn, and the agent goes on, until the goal is met, or the
//! chat has taken as many turns on the human's line as one line gets, or
//! the agent stops calling tools, which says it waits for the human.
//!
//! The triggers of the folder the chat works in that are due as the turn
//! ends (see [`crate::trigger`]) run after the goal, the same way, each a
//! [`Due`] that the host hands the conversation ahead of the turn's end:
//! the first that fails stops them, and the agent is told what it found.
//! The conversation shows them, with the goal, on one line of chips (see
//! [`Entry::Checks`]).
//!
//! The triggers that run as something else happens in the chat, as its
//! goal being met, the host hands the conversation as it happens (see
//! [`Conversation::run_event`]), and they run once the chat waits for the
//! human, all of them, on a line of their own (see [`Entry::Event`]):
//! what they find stays there, and the agent is not told it. A trigger
//! `by: this` tells the chat's agent what it says, as the next turn, in
//! the human's place, once those that run with it went.

use std::collections::{BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use cox_acp::{PermissionAnswer, SlashCommand, StopReason};
use cox_core::diff::Line;
pub use cox_core::entry::{
    Answer, Attachment, Check, CheckState, Committed, Content, Entry, LoginOption, Step, StepState,
    TerminalLogin, ToolCallResult,
};
use cox_core::goal;
use cox_core::settings::Setting;
use cox_core::store::ChatId;

use crate::agent::mcp::{Ended, Ran};
use crate::agent::{AgentEvent, Command, Opened, Prompt};
use crate::chat::judge::{self, Verdict};

/// Whose move it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum State {
    /// Waiting for the human to send a line.
    Idle,
    /// A turn is running.
    Busy,
    /// The agent waits for the human to choose one of the options of the
    /// last [`Entry::Permission`].
    Asking,
    /// The human asked the agent to stop the turn, and it has not ended
    /// yet.
    Stopping,
    /// The turn ended, and the command of the chat's goal runs, or its
    /// judge is asked, to see whether the goal is met, or one of the
    /// triggers due runs.
    Checking,
    /// The connection has closed; nothing more can be sent.
    Closed,
}

/// How many turns a chat takes toward its goal on one line of the human's,
/// at most.
pub const GOAL_TURNS: usize = 10;

/// How many turns in a row without a tool call stop a chat working toward
/// its goal: an agent that only talks waits for the human.
const IDLE_TURNS: usize = 2;

/// The most of what the goal's command printed, or its judge said, that
/// the agent is told, in characters, from the end.
const OUTPUT_TAIL: usize = 4000;

/// A trigger due as a turn ends, which runs after the chat's goal (see
/// [`crate::trigger`]): what it goes by, and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    pub name: String,
    /// Its file, which tells it from another of the same name.
    pub file: std::path::PathBuf,
    pub does: Does,
}

/// What a trigger due does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Does {
    /// Runs the command, as the goal's would, passing when it exits 0.
    Run(String),
    /// Asks an agent of its own what the trigger says, which ends its
    /// reply with `PASS`, or `FAIL: ` and why, as a judge's.
    Ask(String),
    /// Tells the chat's agent what the trigger says, as the next turn,
    /// once what runs with it went: once for a line of the human's, as a
    /// turn ends.
    Tell(String),
}

/// What a trigger that gathers what the human needs to answer a
/// permission request came back with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gathered {
    /// What its agent said.
    Said(String),
    /// What its command printed, and how it ended.
    Printed(Ran),
    /// Why it could not run.
    Broken(String),
}

/// What happened in a chat, besides a turn's end, whose triggers run:
/// the event, as a trigger's `on` names it, as `goal.met`, and what the
/// agent of a trigger is told of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Happened {
    pub on: String,
    pub about: String,
}

/// What runs now, as the turn ended.
#[derive(Clone, Debug, PartialEq, Eq)]
enum At {
    Goal,
    Trigger(Due),
}

/// What a chat works until, and how far it has come.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Goal {
    /// The goal, as the human set it.
    what: goal::Goal,
    /// How many turns the agent has taken toward it on the human's last
    /// line.
    turns: usize,
}

impl Goal {
    fn new(what: goal::Goal) -> Self {
        Self { what, turns: 0 }
    }

    /// The goal, as the human set it.
    #[must_use]
    pub fn what(&self) -> &goal::Goal {
        &self.what
    }

    /// The command, for a goal that is one.
    #[must_use]
    pub fn command(&self) -> Option<&str> {
        self.what.command()
    }

    /// The words, for a goal that is words.
    #[must_use]
    pub fn words(&self) -> Option<&str> {
        self.what.words()
    }

    /// How the goal reads, after "Goal:".
    #[must_use]
    pub fn line(&self) -> String {
        line(&self.what)
    }

    /// How many turns the agent has taken toward it on the human's last
    /// line, counting the one that runs.
    #[must_use]
    pub fn turns(&self) -> usize {
        self.turns
    }
}

/// The milestone of a chat whose goal is met, as the card that asks
/// whether to keep what it learned says it.
const GOAL_MET: &str = "The goal is met.";

/// The event of the goal being met, as a trigger's `on` names it.
const GOAL_MET_EVENT: &str = "goal.met";

/// The texts of `steps`.
fn texts(steps: &[Step]) -> Vec<String> {
    steps.iter().map(|step| step.text.clone()).collect()
}

/// The steps `added` to the plan and `removed` from it, a line each.
fn changed_steps(added: &[String], removed: &[String]) -> String {
    let lines: Vec<String> = added
        .iter()
        .map(|step| format!("+ {step}"))
        .chain(removed.iter().map(|step| format!("- {step}")))
        .collect();
    if lines.is_empty() {
        return "The plan is back to the one the line started with.".to_owned();
    }
    lines.join("\n")
}

/// How `goal` reads, after "Goal:".
#[must_use]
pub fn line(goal: &goal::Goal) -> String {
    match goal {
        goal::Goal::Command(command) => format!("until `{command}` passes."),
        goal::Goal::Words(words) => words.clone(),
    }
}

/// How a conversation changed, as the daemon, where it goes on, tells the
/// window that shows it (see [`Conversation::patch`]): the entries that
/// changed, by their places, and all the rest it holds as it is now.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Patch {
    /// How many entries it holds.
    pub len: usize,
    /// The entries that changed, with their places, in order.
    pub entries: Vec<(usize, Entry)>,
    pub state: State,
    pub title: Option<String>,
    pub goal: Option<Goal>,
    pub settings: Vec<Setting>,
    pub commands: Vec<SlashCommand>,
    pub before_login: State,
    pub keeps: bool,
    /// Whether the agent takes a line into the turn it takes: from a
    /// daemon of before it was said, it does not.
    #[serde(default)]
    pub steers: bool,
}

/// How the agent's plan stands against the one the human left it to: the
/// first it sent after the human's last line.
#[derive(Debug, Default)]
struct Drift {
    /// The steps of the plan the agent first sent after the human's last
    /// line, which they left it to, once it sent one: the plan is told
    /// against it as it changes.
    baseline: Option<Vec<String>>,
    /// Whether the next plan the agent sends is the baseline, as the
    /// first after a line of the human's.
    rebase: bool,
    /// Whether the baseline changed since it was last taken, to keep.
    unkept: bool,
    /// Where the entries told against the baseline start: it changed
    /// there.
    from: usize,
    /// What the plan's change since the baseline tells the triggers that
    /// run on it, once the agent changed a step, until they are fired.
    fired: Option<String>,
    /// How many times the agent changed a step of the plan since the
    /// baseline, for the human to be told each.
    changes: usize,
}

/// Which of what the agent told of its session changed since it was last
/// taken, to keep.
#[derive(Debug, Default)]
struct Unkept {
    settings: bool,
    commands: bool,
}

/// The conversation the window shows.
#[derive(Debug)]
pub struct Conversation {
    entries: Vec<Entry>,
    state: State,
    /// The indexes of the entries added or changed since they were last
    /// taken, to keep.
    changed: BTreeSet<usize>,
    /// The title the agent gave its session, if it gave one.
    title: Option<String>,
    /// What the chat works until, if the human set one.
    goal: Option<Goal>,
    /// What the agent lets the human choose of its session, such as its
    /// model, each as it is set, once the agent has said, or as it was
    /// kept.
    settings: Vec<Setting>,
    /// The commands the agent runs when a line starts with `/` and one of
    /// their names, as it last told them, while it runs or since, as they
    /// were kept: a line sent to a closed chat starts it again.
    commands: Vec<SlashCommand>,
    /// Which of the settings and the commands changed since that was last
    /// taken, to keep.
    unkept: Unkept,
    /// Whose move it was when the agent asked the human to log in, which it
    /// is again once they have chosen how.
    before_login: State,
    /// Whether the chat's agent can keep what it learned in the wiki, so
    /// that the chat asks the human whether to at its milestones.
    keeps: bool,
    /// Whether the agent takes a line the human sends while it takes a
    /// turn into that turn, as its session said as it opened.
    steers: bool,
    /// How many lines the human steered into the turn that the agent
    /// side has not taken yet: a turn that ends meanwhile is not the end
    /// of the human's line, as the agent goes on with them.
    steering: usize,
    /// How the agent's plan stands against the one the human left it to.
    drift: Drift,
    /// The triggers due as the turn that runs ends, as the host said.
    due: Vec<Due>,
    /// What runs now, as the turn ended, while something does.
    at: Option<At>,
    /// The triggers due that have not run yet, as the turn ended.
    rest: VecDeque<Due>,
    /// The entry that shows what runs as the turn ended, and the place
    /// in it of what runs now.
    marks: Option<(usize, usize)>,
    /// Whether the goal was met as the turn ended, which asks the human
    /// whether to keep what was learned once the triggers pass too, and
    /// the host runs the triggers of.
    met: Met,
    /// How many turns the chat took on the human's last line.
    turns: usize,
    /// What happened whose triggers run now, while they do; `None` while
    /// those of a turn's end do.
    event: Option<Happened>,
    /// What happened whose triggers wait for the chat to wait for the
    /// human, the first first.
    waiting: VecDeque<(Happened, Vec<Due>)>,
    /// What the triggers `by: this` that went tell the agent once the
    /// rest went, by their files.
    tells: Vec<(PathBuf, String)>,
    /// The triggers `by: this` that told the agent on the last line, by
    /// their files, which a turn's end does not run again until the next.
    told_line: BTreeSet<PathBuf>,
    /// What happened on the last line whose triggers ran, as the goal
    /// being met, which the turns after on the line meet again.
    raised_line: BTreeSet<&'static str>,
}

/// Whether the goal was met as the turn ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Met {
    #[default]
    No,
    /// It was, as what runs as the turn ended runs.
    Now,
    /// It was, once all ran, and the host has not run its triggers yet.
    Untaken,
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

impl Conversation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            state: State::Idle,
            changed: BTreeSet::new(),
            title: None,
            goal: None,
            settings: Vec::new(),
            commands: Vec::new(),
            unkept: Unkept::default(),
            before_login: State::Idle,
            keeps: true,
            steers: false,
            steering: 0,
            drift: Drift::default(),
            due: Vec::new(),
            at: None,
            rest: VecDeque::new(),
            marks: None,
            met: Met::No,
            turns: 0,
            event: None,
            waiting: VecDeque::new(),
            tells: Vec::new(),
            told_line: BTreeSet::new(),
            raised_line: BTreeSet::new(),
        }
    }

    /// A conversation that opens with `note`, a note from Cox.
    #[must_use]
    pub fn noting(note: String) -> Self {
        Self::noting_each([note])
    }

    /// A conversation that starts with `notes` from Cox, in order.
    #[must_use]
    pub fn noting_each(notes: impl IntoIterator<Item = String>) -> Self {
        let mut conversation = Self::new();
        for note in notes {
            conversation.push(Entry::Note(note));
        }
        conversation
    }

    /// The conversation of a chat that has closed, which showed `entries`,
    /// for the human to read.
    #[must_use]
    pub fn closed(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            state: State::Closed,
            changed: BTreeSet::new(),
            title: None,
            goal: None,
            settings: Vec::new(),
            commands: Vec::new(),
            unkept: Unkept::default(),
            before_login: State::Idle,
            keeps: true,
            steers: false,
            steering: 0,
            drift: Drift::default(),
            due: Vec::new(),
            at: None,
            rest: VecDeque::new(),
            marks: None,
            met: Met::No,
            turns: 0,
            event: None,
            waiting: VecDeque::new(),
            tells: Vec::new(),
            told_line: BTreeSet::new(),
            raised_line: BTreeSet::new(),
        }
    }

    /// The conversation, of a chat that works toward `goal`, if the human
    /// set one.
    #[must_use]
    pub fn with_goal(mut self, goal: Option<goal::Goal>) -> Self {
        self.goal = goal.map(Goal::new);
        self
    }

    /// The conversation, of a chat whose agent's plan started the human's
    /// last line as `baseline` says, as it was kept.
    #[must_use]
    pub fn with_baseline(mut self, baseline: Option<Vec<String>>) -> Self {
        self.drift.baseline = baseline;
        self.drift.from = self.entries.len();
        self
    }

    /// The conversation, of a chat whose session was set as `settings`
    /// say, as they were kept.
    #[must_use]
    pub fn with_settings(mut self, settings: Vec<Setting>) -> Self {
        self.settings = settings;
        self
    }

    /// The conversation, of a chat whose agent told `commands` as those it
    /// runs, as they were kept.
    #[must_use]
    pub fn with_commands(mut self, commands: Vec<SlashCommand>) -> Self {
        self.commands = commands;
        self
    }

    /// The conversation, of a chat whose agent can keep what it learned
    /// in the wiki, as `keeps` says, or not, when it has not the tool.
    #[must_use]
    pub fn with_keeping(mut self, keeps: bool) -> Self {
        self.keeps = keeps;
        self
    }

    /// Asks the human, on a card, whether to keep what a chat learned in
    /// the wiki, at the milestone `why` says: this chat's, or, given
    /// `chat`, another's, as a sketch that ended beside it.
    pub fn ask_to_keep(&mut self, why: String, chat: Option<ChatId>) {
        self.push(Entry::Keep {
            why,
            chat,
            kept: None,
        });
    }

    /// Asks the human whether to keep what the chat learned, at the
    /// milestone `why` says, when its agent can: once for a line of the
    /// human's, so that the turn that keeps it, which meets the goal
    /// again, does not ask again.
    fn ask_to_keep_here(&mut self, why: &str) {
        let asked = self
            .entries
            .iter()
            .rev()
            .take_while(|entry| !matches!(entry, Entry::Human(_)))
            .any(|entry| matches!(entry, Entry::Keep { .. }));
        if self.keeps && !asked {
            self.ask_to_keep(why.to_owned(), None);
        }
    }

    /// Takes whether the human chose to keep what a chat learned, as
    /// asked at `index`, and answers which chat is to keep it, `None`
    /// for this one, unless they answered already.
    pub fn answer_keep(&mut self, index: usize, keep: bool) -> Option<Option<ChatId>> {
        let Some(Entry::Keep {
            chat,
            kept: kept @ None,
            ..
        }) = self.entries.get_mut(index)
        else {
            return None;
        };
        *kept = Some(keep);
        self.changed.insert(index);
        Some(*chat)
    }

    /// What the agent lets the human choose of its session, each as it is
    /// set.
    #[must_use]
    pub fn settings(&self) -> &[Setting] {
        &self.settings
    }

    /// The commands the agent runs when a line starts with `/` and one of
    /// their names, as it last told them, while it runs or since.
    #[must_use]
    pub fn commands(&self) -> &[SlashCommand] {
        &self.commands
    }

    /// Whether the settings changed since this was last called, to keep.
    pub fn take_settings_changed(&mut self) -> bool {
        std::mem::take(&mut self.unkept.settings)
    }

    /// Whether the commands changed since this was last called, to keep.
    pub fn take_commands_changed(&mut self) -> bool {
        std::mem::take(&mut self.unkept.commands)
    }

    /// Sets the setting `id` to `value` here, as for a closed chat, whose
    /// agent sets it once it starts again. Answers whether it changed.
    pub fn set(&mut self, id: &str, value: &str) -> bool {
        let Some(setting) = self.settings.iter_mut().find(|setting| setting.id == id) else {
            return false;
        };
        if setting.value == value || !setting.offers(value) {
            return false;
        }
        value.clone_into(&mut setting.value);
        self.unkept.settings = true;
        true
    }

    /// Opens the conversation of a chat that has closed again, for the
    /// human to go on with it once its agent starts again.
    pub fn reopen(&mut self) {
        if self.state == State::Closed {
            self.state = State::Idle;
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// The indexes of the entries added or changed since this was last
    /// called, in order.
    pub fn take_changed(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.changed).into_iter().collect()
    }

    /// What a window that shows the conversation needs to show it as it
    /// is now, given that the entries at `changed` are all that changed
    /// since it was last told: those entries, and the rest of what the
    /// conversation holds, which is small.
    #[must_use]
    pub fn patch(&self, changed: &[usize]) -> Patch {
        Patch {
            len: self.entries.len(),
            entries: changed
                .iter()
                .filter_map(|&index| Some((index, self.entries.get(index)?.clone())))
                .collect(),
            state: self.state,
            title: self.title.clone(),
            goal: self.goal.clone(),
            settings: self.settings.clone(),
            commands: self.commands.clone(),
            before_login: self.before_login,
            keeps: self.keeps,
            steers: self.steers,
        }
    }

    /// What a window that shows the conversation needs to show all of it,
    /// as when it first shows it.
    #[must_use]
    pub fn whole(&self) -> Patch {
        let all: Vec<usize> = (0..self.entries.len()).collect();
        self.patch(&all)
    }

    /// Changes the conversation as `patch` says, for a window that shows
    /// a conversation that goes on elsewhere, in the daemon.
    pub fn apply_patch(&mut self, patch: Patch) {
        let Patch {
            len,
            entries,
            state,
            title,
            goal,
            settings,
            commands,
            before_login,
            keeps,
            steers,
        } = patch;
        self.entries.truncate(len);
        for (index, entry) in entries {
            if index < self.entries.len() {
                self.entries[index] = entry;
            } else if index == self.entries.len() {
                self.entries.push(entry);
            }
        }
        self.state = state;
        self.title = title;
        self.goal = goal;
        self.settings = settings;
        self.commands = commands;
        self.before_login = before_login;
        self.keeps = keeps;
        self.steers = steers;
    }

    fn push(&mut self, entry: Entry) {
        self.changed.insert(self.entries.len());
        self.entries.push(entry);
    }

    /// Adds `note`, a note from Cox.
    pub fn note(&mut self, note: String) {
        self.push(Entry::Note(note));
    }

    /// Shows `commits`, which the agent made in the turn that ends, if it
    /// made any.
    pub fn committed(&mut self, commits: Vec<Committed>) {
        if !commits.is_empty() {
            self.push(Entry::Commits(commits));
        }
    }

    /// The entry that started the last turn, by its place, with the line
    /// it sent: the human's, or Cox's in their place.
    #[must_use]
    pub fn turn_start(&self) -> Option<(usize, &str)> {
        self.entries
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, entry)| match entry {
                Entry::Human(text) | Entry::Cox(text) => Some((index, text.as_str())),
                _ => None,
            })
    }

    /// Closes the conversation, as `note` says why: its agent is gone.
    pub fn close(&mut self, note: String) {
        self.note(note);
        self.state = State::Closed;
    }

    /// Closes the conversation as the human switches it from the agent
    /// named `from` to the one named `to`, which goes on from it. The
    /// settings and the commands were of the agent before.
    pub fn switch(&mut self, from: &str, to: &str) {
        self.push(Entry::Switched {
            from: from.to_owned(),
            to: to.to_owned(),
        });
        self.state = State::Closed;
        self.settings.clear();
        self.commands.clear();
    }

    /// Takes what the human entered, and returns the prompt to send the
    /// agent, when it waits for one.
    pub fn submit(&mut self, input: &str) -> Option<Command> {
        self.submit_with(input, Vec::new())
    }

    /// Takes what the human entered, with what they handed the chat with
    /// it, and returns the prompt to send the agent, when it waits for
    /// one.
    /// one, or, for an agent that [`Conversation::steers`], into the turn
    /// it takes.
    pub fn submit_with(&mut self, input: &str, attached: Vec<Attachment>) -> Option<Command> {
        let command = match self.steer(input) {
            Some(command) => command,
            None => self.start_turn(input, Entry::Human)?,
        };
        // The human leaves the agent to the line from here: the first
        // plan it sends after is the one its plan is told against.
        self.drift.rebase = true;
        if !attached.is_empty() {
            self.push(Entry::Attached(attached.clone()));
        }
        Some(match command {
            Command::Prompt(prompt) => Command::Prompt(Prompt { attached, ..prompt }),
            Command::Steer(prompt) => Command::Steer(Prompt { attached, ..prompt }),
            command => command,
        })
    }

    /// Whether a line the human sends now goes into the turn the agent
    /// takes, as it goes, rather than waiting for it to end: the agent
    /// takes lines so, and a turn runs that the human has not stopped.
    #[must_use]
    pub fn steers(&self) -> bool {
        self.steers && matches!(self.state, State::Busy | State::Asking)
    }

    /// Whether the agent side has yet to take a line the human steered
    /// into the turn: a turn that ends meanwhile does not end their line.
    #[must_use]
    pub fn steering(&self) -> bool {
        self.steering > 0
    }

    /// Steers `text`, a line of the human's, into the turn the agent
    /// takes, when it [`Conversation::steers`]: the line counts as the
    /// human's last, as one that starts a turn does.
    fn steer(&mut self, text: &str) -> Option<Command> {
        if !self.steers() || text.trim().is_empty() {
            return None;
        }
        self.push(Entry::Human(text.to_owned()));
        self.steering += 1;
        self.new_line();
        Some(Command::Steer(text.to_owned().into()))
    }

    /// Takes `text`, a line Cox sends in the human's place, such as the
    /// first line of a chat opened from a chat, and returns the prompt to
    /// send the agent, when it waits for one.
    pub fn told(&mut self, text: &str) -> Option<Command> {
        self.start_turn(text, Entry::Cox)
    }

    /// Starts a turn on `text`, Cox's in the human's place, asking the
    /// agent to merge what it made, when it waits for a line: the work the
    /// human's last line left it closes there.
    pub fn merge(&mut self, text: &str) -> Option<Command> {
        if self.state != State::Idle || text.trim().is_empty() {
            return None;
        }
        self.push(Entry::Merge);
        self.told(text)
    }

    /// Starts a turn on `text`, shown as `entry` makes it, when the agent
    /// waits for a line.
    fn start_turn(&mut self, text: &str, entry: fn(String) -> Entry) -> Option<Command> {
        if self.state != State::Idle || text.trim().is_empty() {
            return None;
        }
        self.push(entry(text.to_owned()));
        self.state = State::Busy;
        self.new_line();
        Some(Command::Prompt(text.to_owned().into()))
    }

    /// Counts what runs from now on toward a new line of the human's.
    fn new_line(&mut self) {
        self.turns = 1;
        self.told_line.clear();
        self.raised_line.clear();
        if let Some(goal) = &mut self.goal {
            goal.turns = 1;
        }
    }

    /// What the chat works until, if the human set a goal.
    #[must_use]
    pub fn goal(&self) -> Option<&Goal> {
        self.goal.as_ref()
    }

    /// Sets the chat to work toward `goal`, or, given `None`, toward no
    /// goal, and notes so. A check of the goal that runs is given up.
    pub fn set_goal(&mut self, goal: Option<goal::Goal>) {
        if goal.as_ref() == self.goal.as_ref().map(Goal::what) {
            return;
        }
        self.goal = goal.map(Goal::new);
        let note = match &self.goal {
            Some(goal) => format!("Goal: {}", goal.line()),
            None => "The goal is cleared.".to_owned(),
        };
        self.push(Entry::Note(note));
        if self.state == State::Checking {
            self.stop_checks(CheckState::Waiting);
        }
    }

    /// Shows `goal`, which the agent proposes for the chat, for the human
    /// to set, change, or let be, with `why` the agent proposes it, if it
    /// said.
    pub fn propose(&mut self, goal: &goal::Goal, why: Option<String>) {
        self.push(Entry::Proposal {
            goal: goal.input(),
            why: why.filter(|why| !why.trim().is_empty()),
            answer: None,
        });
    }

    /// The goal proposed at `index`, while the human has not answered it.
    #[must_use]
    pub fn proposed(&self, index: usize) -> Option<goal::Goal> {
        match self.entries.get(index)? {
            Entry::Proposal {
                goal, answer: None, ..
            } => goal::Goal::parse(goal),
            _ => None,
        }
    }

    /// Takes the human's `answer` to the goal proposed at `index`, and
    /// returns the goal proposed, unless they answered it already. Setting
    /// a goal is the caller's, with [`Conversation::set_goal`].
    pub fn answer_proposal(&mut self, index: usize, answer: Answer) -> Option<goal::Goal> {
        let proposed = self.proposed(index)?;
        if let Some(Entry::Proposal { answer: taken, .. }) = self.entries.get_mut(index) {
            *taken = Some(answer);
            self.changed.insert(index);
        }
        Some(proposed)
    }

    /// Shows `name`, which the agent proposes for the chat's project, for
    /// the human to rename it to or to let be, with `why` the agent
    /// proposes it, if it said.
    pub fn propose_name(&mut self, name: &str, why: Option<String>) {
        self.push(Entry::NameProposal {
            name: name.to_owned(),
            why: why.filter(|why| !why.trim().is_empty()),
            answer: None,
        });
    }

    /// The name proposed at `index`, while the human has not answered it.
    #[must_use]
    pub fn proposed_name(&self, index: usize) -> Option<&str> {
        match self.entries.get(index)? {
            Entry::NameProposal {
                name, answer: None, ..
            } => Some(name),
            _ => None,
        }
    }

    /// Takes the human's `answer` to the name proposed at `index`, unless
    /// they answered it already. Renaming the project is the caller's.
    pub fn answer_name_proposal(&mut self, index: usize, answer: Answer) {
        if self.proposed_name(index).is_none() {
            return;
        }
        if let Some(Entry::NameProposal { answer: taken, .. }) = self.entries.get_mut(index) {
            *taken = Some(answer);
            self.changed.insert(index);
        }
    }

    /// Shows `about`, what the agent proposes the chat's project is, for
    /// the human to set or to let be, with `why` the agent proposes it, if
    /// it said.
    pub fn propose_about(&mut self, about: &str, why: Option<String>) {
        self.push(Entry::AboutProposal {
            about: about.to_owned(),
            why: why.filter(|why| !why.trim().is_empty()),
            answer: None,
        });
    }

    /// What the project is as proposed at `index`, while the human has
    /// not answered it.
    #[must_use]
    pub fn proposed_about(&self, index: usize) -> Option<&str> {
        match self.entries.get(index)? {
            Entry::AboutProposal {
                about,
                answer: None,
                ..
            } => Some(about),
            _ => None,
        }
    }

    /// Takes the human's `answer` to what the project is as proposed at
    /// `index`, unless they answered it already. Setting it is the
    /// caller's.
    pub fn answer_about_proposal(&mut self, index: usize, answer: Answer) {
        if self.proposed_about(index).is_none() {
            return;
        }
        if let Some(Entry::AboutProposal { answer: taken, .. }) = self.entries.get_mut(index) {
            *taken = Some(answer);
            self.changed.insert(index);
        }
    }

    /// Shows the project named `name`, which the agent proposes to make of
    /// the chat, for the human to make or to let be, with `why` the agent
    /// proposes it, if it said, and the sketch whose files go into it,
    /// `sketch`, its name and folder, if it named one.
    pub fn propose_project(
        &mut self,
        name: &str,
        why: Option<String>,
        sketch: Option<(String, PathBuf)>,
    ) {
        let (sketch_name, sketch) = sketch.unzip();
        self.push(Entry::ProjectProposal {
            name: name.to_owned(),
            why: why.filter(|why| !why.trim().is_empty()),
            sketch,
            sketch_name,
            answer: None,
        });
    }

    /// The project proposed at `index`, its name and the folder of the
    /// sketch that goes into it, if any, while the human has not answered
    /// it.
    #[must_use]
    pub fn proposed_project(&self, index: usize) -> Option<(&str, Option<&Path>)> {
        match self.entries.get(index)? {
            Entry::ProjectProposal {
                name,
                sketch,
                answer: None,
                ..
            } => Some((name, sketch.as_deref())),
            _ => None,
        }
    }

    /// Takes the human's `answer` to the project proposed at `index`,
    /// unless they answered it already. Making the project is the
    /// caller's.
    pub fn answer_project_proposal(&mut self, index: usize, answer: Answer) {
        if self.proposed_project(index).is_none() {
            return;
        }
        if let Some(Entry::ProjectProposal { answer: taken, .. }) = self.entries.get_mut(index) {
            *taken = Some(answer);
            self.changed.insert(index);
        }
    }

    /// Hands the conversation the triggers due as the turn that runs
    /// ends, which run after the goal, in order, if it ends as the agent
    /// ends it.
    pub fn set_due(&mut self, due: Vec<Due>) {
        self.due = due;
    }

    /// Starts what runs as the turn ended: the goal, if the chat has one,
    /// then `due`, shown on one line of chips.
    fn start_checks(&mut self, due: Vec<Due>) {
        let goal = self.goal.is_some().then(|| Check {
            name: "goal".to_owned(),
            state: CheckState::Waiting,
            said: None,
        });
        let marks = goal
            .into_iter()
            .chain(due.iter().map(|due| Check {
                name: due.name.clone(),
                state: CheckState::Waiting,
                said: None,
            }))
            .collect();
        self.push(Entry::Checks(marks));
        self.marks = Some((self.entries.len() - 1, 0));
        self.rest = due.into();
        self.met = Met::No;
        self.state = State::Checking;
        self.at = if self.goal.is_some() {
            Some(At::Goal)
        } else {
            self.rest.pop_front().map(At::Trigger)
        };
        self.mark(CheckState::Running);
    }

    /// Has `due`, the triggers that run on what `happened`, run as soon as
    /// the chat waits for the human, which the host starts them with (see
    /// [`Self::run_waiting`]).
    pub fn run_event(&mut self, happened: Happened, due: Vec<Due>) {
        if due.is_empty() {
            return;
        }
        self.waiting.push_back((happened, due));
    }

    /// Starts the triggers of what happened first that wait, if any do
    /// and the chat waits for the human; answers whether it did.
    pub fn run_waiting(&mut self) -> bool {
        if self.state != State::Idle {
            return false;
        }
        let Some((happened, mut due)) = self.waiting.pop_front() else {
            return false;
        };
        // A trigger by: this tells the agent once for a line.
        due.retain(|due| !matches!(due.does, Does::Tell(_)) || !self.told_line.contains(&due.file));
        if due.is_empty() {
            return self.run_waiting();
        }
        let marks = due
            .iter()
            .map(|due| Check {
                name: due.name.clone(),
                state: CheckState::Waiting,
                said: None,
            })
            .collect();
        self.push(Entry::Event {
            on: happened.on.clone(),
            checks: marks,
        });
        self.marks = Some((self.entries.len() - 1, 0));
        self.rest = due.into();
        self.event = Some(happened);
        self.state = State::Checking;
        self.at = self.rest.pop_front().map(At::Trigger);
        self.mark(CheckState::Running);
        true
    }

    /// What happened whose triggers run now, while they do, as against
    /// those of a turn's end.
    #[must_use]
    pub fn event(&self) -> Option<&Happened> {
        self.event
            .as_ref()
            .filter(|_| self.state == State::Checking)
    }

    /// Whether the goal was met since this was last asked, for the host
    /// to run the triggers of it.
    pub fn take_goal_met(&mut self) -> bool {
        let met = self.met == Met::Untaken;
        if met {
            self.met = Met::No;
        }
        met
    }

    /// What the trigger `by: this` that runs now tells the agent, while
    /// one runs.
    #[must_use]
    pub fn telling(&self) -> Option<&str> {
        match self.trigger()?.does {
            Does::Tell(ref text) => Some(text),
            _ => None,
        }
    }

    /// Takes what the trigger `by: this` that runs now tells the agent, to
    /// tell it once the rest went, and returns the next turn to send, if
    /// that was the last.
    pub fn tell(&mut self) -> Option<Command> {
        let due = self.trigger()?.clone();
        let Does::Tell(text) = due.does else {
            return None;
        };
        self.tells.push((due.file, text));
        self.say("Told the chat's agent, once the rest went.");
        self.next(CheckState::Passed)
    }

    /// Marks what runs now as `state`, on the line of chips.
    fn mark(&mut self, state: CheckState) {
        let Some((entry, at)) = self.marks else {
            return;
        };
        if let Some(marks) = self.entries.get_mut(entry).and_then(Entry::checks_mut)
            && let Some(mark) = marks.get_mut(at)
        {
            mark.state = state;
            self.changed.insert(entry);
        }
    }

    /// Keeps what what runs now `said`, on its chip, for the human to read
    /// there.
    fn say(&mut self, said: &str) {
        let Some((entry, at)) = self.marks else {
            return;
        };
        if let Some(marks) = self.entries.get_mut(entry).and_then(Entry::checks_mut)
            && let Some(mark) = marks.get_mut(at)
        {
            mark.said = Some(tail(said, OUTPUT_TAIL)).filter(|said| !said.is_empty());
            self.changed.insert(entry);
        }
    }

    /// What runs now passed: the next runs, or, when none is left, the
    /// chat waits for the human, as [`Self::next`] says.
    fn passed(&mut self) -> Option<Command> {
        self.next(CheckState::Passed)
    }

    /// What runs now went as `state` says, and what runs on: the next, or,
    /// when none is left, the chat waits for the human, asking whether to
    /// keep what was learned if the goal was met, unless the triggers `by:
    /// this` that went tell the agent what they say, which returns the
    /// turn to send.
    fn next(&mut self, state: CheckState) -> Option<Command> {
        self.mark(state);
        if let Some((_, at)) = &mut self.marks {
            *at += 1;
        }
        self.at = self.rest.pop_front().map(At::Trigger);
        if self.at.is_some() {
            self.mark(CheckState::Running);
            return None;
        }
        self.state = State::Idle;
        self.marks = None;
        let event = self.event.take();
        if self.met == Met::Now {
            self.ask_to_keep_here(GOAL_MET);
            // Its triggers run once for a line, as the turns they start
            // meet it again.
            self.met = if self.raised_line.insert(GOAL_MET_EVENT) {
                Met::Untaken
            } else {
                Met::No
            };
        }
        let tells = std::mem::take(&mut self.tells);
        if tells.is_empty() {
            return None;
        }
        if event.is_none() && self.turns >= GOAL_TURNS {
            self.push(Entry::Note(format!(
                "The triggers by: this tell the agent nothing more: the chat took \
                 {GOAL_TURNS} turns, as many as one line gets; send a line to go on."
            )));
            return None;
        }
        let (files, texts): (Vec<PathBuf>, Vec<String>) = tells.into_iter().unzip();
        self.told_line.extend(files);
        let prompt = texts.join("\n\n");
        if event.is_none() {
            self.turns += 1;
        }
        self.push(Entry::Cox(prompt.clone()));
        self.state = State::Busy;
        Some(Command::Prompt(prompt.into()))
    }

    /// Stops what runs as the turn ended, marking what runs now as
    /// `state`; what has not run yet stays waiting. The chat waits for the
    /// human, unless it goes on, as [`Self::go_on`] says.
    fn stop_checks(&mut self, state: CheckState) {
        if self.at.is_some() {
            self.mark(state);
        }
        self.at = None;
        self.rest.clear();
        self.marks = None;
        self.state = State::Idle;
        self.event = None;
        self.tells.clear();
    }

    /// What runs now could not run: as a turn ends, what was to run after
    /// it does not, and the chat waits for the human; as something else
    /// happened, the rest runs.
    fn broken(&mut self) -> Option<Command> {
        if self.event.is_some() {
            return self.next(CheckState::Broken);
        }
        self.stop_checks(CheckState::Broken);
        None
    }

    /// The name of what runs now as the turn ended, `goal` or a
    /// trigger's, as the line of chips shows it, while something runs.
    #[must_use]
    pub fn running_check(&self) -> Option<&str> {
        if self.state != State::Checking {
            return None;
        }
        self.entries.iter().rev().find_map(|entry| {
            entry
                .checks()?
                .iter()
                .find(|check| check.state == CheckState::Running)
                .map(|check| check.name.as_str())
        })
    }

    /// Where the chip of what runs now is, as the turn ended: the entry
    /// of the line of chips, and its place in it, while something runs.
    #[must_use]
    pub fn check_at(&self) -> Option<(usize, usize)> {
        self.marks.filter(|_| self.state == State::Checking)
    }

    /// The trigger that runs now, if one does.
    #[must_use]
    pub fn trigger(&self) -> Option<&Due> {
        match (&self.state, &self.at) {
            (State::Checking, Some(At::Trigger(due))) => Some(due),
            _ => None,
        }
    }

    /// The command that runs to see whether the goal is met, or of the
    /// trigger that runs, while one does.
    #[must_use]
    pub fn checking(&self) -> Option<&str> {
        match (self.state, &self.at) {
            (State::Checking, Some(At::Goal)) => self.goal.as_ref()?.command(),
            (
                State::Checking,
                Some(At::Trigger(Due {
                    does: Does::Run(command),
                    ..
                })),
            ) => Some(command),
            _ => None,
        }
    }

    /// The words of the goal the judge is asked about, while it is.
    #[must_use]
    pub fn judging(&self) -> Option<&str> {
        match (self.state, &self.at) {
            (State::Checking, Some(At::Goal)) => self.goal.as_ref()?.words(),
            _ => None,
        }
    }

    /// The trigger whose agent is asked, while it is.
    #[must_use]
    pub fn asking(&self) -> Option<&Due> {
        self.trigger()
            .filter(|due| matches!(due.does, Does::Ask(_)))
    }

    /// Takes how the command of the goal, or of the trigger that runs,
    /// went, `ran`, and returns the next turn to send the agent, when it
    /// failed and the chat goes on. When it passed, what is due next
    /// runs, as [`Self::checking`] and the others say, or the chat waits
    /// for the human; it waits too when the command could not run, or the
    /// chat has taken as many turns as one line gets, or the agent made
    /// no tool call in its last turns.
    pub fn checked(&mut self, ran: Result<Ran, String>) -> Option<Command> {
        let command = self.checking()?.to_owned();
        let trigger = self.trigger().map(|due| due.name.clone());
        let ran = match ran {
            Ok(ran) => ran,
            Err(error) => {
                self.say(&error);
                self.push(Entry::Note(match &trigger {
                    Some(name) => format!("The trigger `{name}` could not run: {error}"),
                    None => format!("The goal could not be checked: {error}"),
                }));
                return self.broken();
            }
        };
        self.say(&ran.output);
        let not_met = match ran.ended {
            Ended::Exited(0) => {
                if trigger.is_none() {
                    self.push(Entry::Note(format!("Goal met: `{command}` passed.")));
                    self.met = Met::Now;
                }
                return self.passed();
            }
            Ended::Closed => {
                self.push(Entry::Note(match &trigger {
                    Some(name) => format!(
                        "The trigger `{name}` did not run: the terminal of `{command}` was closed."
                    ),
                    None => {
                        format!("The goal was not checked: the terminal of `{command}` was closed.")
                    }
                }));
                return self.broken();
            }
            ended => match &trigger {
                Some(name) => format!("The trigger `{name}` did not pass: `{command}` {ended}."),
                None => format!("Goal not met: `{command}` {ended}."),
            },
        };
        if self.event.is_some() {
            return self.next(CheckState::Failed);
        }
        let output = tail(&ran.output, OUTPUT_TAIL);
        let go_on = if trigger.is_some() {
            "Fix what it found; the triggers run again as your turn ends."
        } else {
            "Go on until it passes."
        };
        let told = if output.is_empty() {
            format!("{go_on} It printed nothing.")
        } else {
            format!("{go_on} It printed, at the end:\n\n{output}")
        };
        self.stop_checks(CheckState::Failed);
        self.go_on(&not_met, &told)
    }

    /// Takes what the judge of the goal in words said, `judged`, or why
    /// it could not judge, and returns the next turn to send the agent,
    /// when the goal is not met and the chat goes on toward it; otherwise
    /// what is due next runs, or the chat waits for the human, as after
    /// [`Conversation::checked`].
    pub fn judged(&mut self, judged: Result<String, String>) -> Option<Command> {
        self.judging()?;
        let said = match judged {
            Ok(said) => said,
            Err(error) => {
                self.say(&error);
                self.push(Entry::Note(format!(
                    "The goal could not be judged: {error}"
                )));
                self.stop_checks(CheckState::Broken);
                return None;
            }
        };
        self.say(&said);
        self.offer_command(&said);
        let why = match Verdict::of(&said) {
            Verdict::Met(said) => {
                self.push(Entry::Note(if said.is_empty() {
                    "Goal met, as judged.".to_owned()
                } else {
                    format!("Goal met, as judged: {said}")
                }));
                self.met = Met::Now;
                return self.passed();
            }
            Verdict::NotMet(why) => why,
            Verdict::Blocked(why) => {
                self.push(Entry::Note(format!(
                    "The goal cannot be judged as written: {why} Change it to what this chat's \
                     work can meet and the judge can check, then send a line to go on."
                )));
                self.stop_checks(CheckState::Broken);
                return None;
            }
        };
        let not_met = format!("Goal not met, as judged: {why}");
        let told = format!(
            "Go on until it is met. The judge said:\n\n{}",
            tail(&said, OUTPUT_TAIL)
        );
        self.stop_checks(CheckState::Failed);
        self.go_on(&not_met, &told)
    }

    /// Takes what the agent of the trigger that runs said, `said`, or why
    /// it could not say, and returns the next turn to send the chat's
    /// agent, when the trigger found something and the chat goes on;
    /// otherwise what is due next runs, or the chat waits for the human,
    /// as after [`Conversation::checked`].
    pub fn found(&mut self, said: Result<String, String>) -> Option<Command> {
        let name = self.asking()?.name.clone();
        let said = match said {
            Ok(said) => said,
            Err(error) => {
                self.say(&error);
                self.push(Entry::Note(format!(
                    "The trigger `{name}` could not run: {error}"
                )));
                return self.broken();
            }
        };
        self.say(&said);
        let why = match Verdict::of(&said) {
            Verdict::Met(_) => return self.passed(),
            // What happened besides a turn's end, the agent is not told.
            Verdict::NotMet(_) | Verdict::Blocked(_) if self.event.is_some() => {
                return self.next(CheckState::Failed);
            }
            Verdict::NotMet(why) | Verdict::Blocked(why) => why,
        };
        let found = format!("The trigger `{name}` found something: {why}");
        let told = format!(
            "Act on what it said; the triggers run again as your turn ends. It said:\n\n{}",
            tail(&said, OUTPUT_TAIL)
        );
        self.stop_checks(CheckState::Failed);
        self.go_on(&found, &told)
    }

    /// Proposes the command the judge `said` decides the goal, if it named
    /// one, for the human to judge the goal by in the judge's place: once
    /// in the chat, as a proposal of the agent's would be.
    fn offer_command(&mut self, said: &str) {
        let Some(command) = judge::command_of(said) else {
            return;
        };
        let goal = goal::Goal::Command(command);
        let input = goal.input();
        let offered = self.entries.iter().any(
            |entry| matches!(entry, Entry::Proposal { goal: proposed, .. } if *proposed == input),
        );
        if !offered {
            self.propose(
                &goal,
                Some(
                    "The judge says this command passes just when the goal is met, so it \
                     can decide the goal in the judge's place."
                        .to_owned(),
                ),
            );
        }
    }

    /// Goes on toward the goal, `not_met` as the check or the judge said:
    /// returns the next turn, telling the agent so and `told`, unless the
    /// chat has taken as many turns as one line gets, or the agent made no
    /// tool call in its last turns, when the chat notes so and waits for
    /// the human.
    fn go_on(&mut self, not_met: &str, told: &str) -> Option<Command> {
        if self.turns >= GOAL_TURNS {
            self.push(Entry::Note(format!(
                "{not_met} The chat took {GOAL_TURNS} turns toward it, as many as one line gets; send a line to go on."
            )));
            return None;
        }
        if self.turns_without_tools() >= IDLE_TURNS {
            self.push(Entry::Note(format!(
                "{not_met} The agent made no tool call in its last {IDLE_TURNS} turns, so it may wait for you."
            )));
            return None;
        }
        let prompt = format!("{not_met} {told}");
        self.turns += 1;
        if let Some(goal) = &mut self.goal {
            goal.turns += 1;
        }
        self.push(Entry::Cox(prompt.clone()));
        self.state = State::Busy;
        Some(Command::Prompt(prompt.into()))
    }

    /// How many of the last turns made no tool call, counting back from
    /// the last turn to the first that made one.
    fn turns_without_tools(&self) -> usize {
        let mut turns = 0;
        let mut called = false;
        for entry in self.entries.iter().rev() {
            match entry {
                Entry::ToolCall { .. } => called = true,
                Entry::Human(_) | Entry::Cox(_) => {
                    if called {
                        break;
                    }
                    turns += 1;
                }
                _ => {}
            }
        }
        turns
    }

    /// The options of the permission request the agent waits on, if it
    /// waits on one.
    #[must_use]
    pub fn asked(&self) -> Option<&[String]> {
        match (self.state, self.entries.last()) {
            (State::Asking, Some(Entry::Permission { options, .. })) => Some(options),
            _ => None,
        }
    }

    /// How many options the human chooses between, by their numbers, while
    /// the agent waits on them: those of a permission request, or the ways
    /// to log in.
    #[must_use]
    pub fn options_asked(&self) -> usize {
        match (self.state, self.entries.last()) {
            (State::Asking, Some(Entry::Permission { options, .. })) => options.len(),
            (State::Asking, Some(Entry::Login { options, .. })) => options.len(),
            _ => 0,
        }
    }

    /// Whether the agent waits for the human to log in.
    #[must_use]
    pub fn logging_in(&self) -> bool {
        matches!(
            (self.state, self.entries.last()),
            (State::Asking, Some(Entry::Login { .. }))
        )
    }

    /// Shows that the agent asks the human to log in, one of the ways of
    /// `options`, and waits for them.
    fn ask_to_log_in(&mut self, options: Vec<LoginOption>, failed: Option<String>) {
        self.push(Entry::Login {
            options,
            failed,
            chosen: None,
        });
        // Asked as the human stopped the turn: the agent side gives up
        // without the human.
        if self.state != State::Stopping {
            self.before_login = match self.state {
                State::Busy | State::Asking => State::Busy,
                _ => State::Idle,
            };
            self.state = State::Asking;
        }
    }

    /// Chooses the way at `index` of logging in, while the agent waits for
    /// the human to log in, and returns it.
    pub fn choose_login(&mut self, index: usize) -> Option<LoginOption> {
        if self.state != State::Asking {
            return None;
        }
        let Some(Entry::Login {
            options, chosen, ..
        }) = self.entries.last_mut()
        else {
            return None;
        };
        let option = options.get(index)?.clone();
        *chosen = Some(index);
        self.changed.insert(self.entries.len() - 1);
        self.state = self.before_login;
        Some(option)
    }

    /// What the agent asks the human's permission for, while it waits on
    /// the human, as the triggers that gather what the human needs to
    /// answer are told: the call, what it is about to do, and the options.
    #[must_use]
    pub fn asked_about(&self) -> Option<String> {
        let (
            State::Asking,
            Some(Entry::Permission {
                title,
                content,
                options,
                chosen: None,
                ..
            }),
        ) = (self.state, self.entries.last())
        else {
            return None;
        };
        let mut about =
            format!("The chat's agent asks the human's permission for a tool call: {title}");
        for content in content {
            match content {
                Content::Text(text) => {
                    let _ = write!(about, "\n\n{}", text.trim());
                }
                Content::Diff { path, lines } => {
                    let _ = write!(about, "\n\nIts change to {path}:\n");
                    for line in lines {
                        let _ = match line {
                            Line::Same(text) => writeln!(about, " {text}"),
                            Line::Removed(text) => writeln!(about, "-{text}"),
                            Line::Added(text) => writeln!(about, "+{text}"),
                            Line::Skipped(count) => writeln!(about, "[{count} lines]"),
                        };
                    }
                }
            }
        }
        let _ = write!(
            about,
            "\n\nThe human chooses one of: {}.",
            options.join(", ")
        );
        Some(about)
    }

    /// Shows the human the picture or video `shown`, kept from the path
    /// `from`, with `caption`, if given.
    pub fn show_media(
        &mut self,
        shown: crate::media::Shown,
        from: String,
        caption: Option<String>,
    ) {
        self.push(Entry::Media {
            file: shown.file,
            kind: shown.kind,
            from,
            caption,
            size: shown.size,
        });
    }

    /// Notes that the agent showed the human the file at `path`, as it
    /// named it, kept as `file`, Markdown if it is `markdown`, or else
    /// code at `lines`, if it named them.
    pub fn show_file(
        &mut self,
        file: String,
        path: String,
        markdown: bool,
        lines: Option<(usize, usize)>,
    ) {
        self.push(Entry::Shown {
            file,
            path,
            markdown,
            lines,
        });
    }

    /// Has the triggers named `names` gather what the human needs to
    /// answer the permission request the agent waits on, which shows that
    /// they do until each has said, or [`Self::stop_gathering`].
    pub fn gather(&mut self, names: Vec<String>) {
        if self.asked().is_none() {
            return;
        }
        if let Some(gathered) = self.gathered_mut() {
            *gathered = names
                .into_iter()
                .map(|name| Check {
                    name,
                    state: CheckState::Running,
                    said: None,
                })
                .collect();
        }
    }

    /// Takes what the trigger at `at` of those that gather said, as
    /// `gathered` says.
    pub fn gathered(&mut self, at: usize, gathered: Gathered) {
        let (state, said) = match gathered {
            Gathered::Said(said) => (CheckState::Passed, head(&said, OUTPUT_TAIL)),
            Gathered::Printed(Ran {
                output,
                ended: Ended::Exited(0),
            }) => (CheckState::Passed, tail(&output, OUTPUT_TAIL)),
            Gathered::Printed(Ran { output, ended }) => {
                let output = tail(&output, OUTPUT_TAIL);
                (CheckState::Failed, format!("{output}\n\n({ended})"))
            }
            Gathered::Broken(why) => (CheckState::Broken, why),
        };
        if let Some(check) = self
            .gathered_mut()
            .and_then(|gathered| gathered.get_mut(at))
            .filter(|check| check.state == CheckState::Running)
        {
            check.state = state;
            check.said = Some(said).filter(|said| !said.trim().is_empty());
        }
    }

    /// Stops what gathers yet, as `why` says, as when the human answered
    /// first, or it took too long.
    pub fn stop_gathering(&mut self, why: &str) {
        let Some(gathered) = self.gathered_mut() else {
            return;
        };
        for check in gathered {
            if check.state == CheckState::Running {
                check.state = CheckState::Broken;
                check.said = Some(why.to_owned());
            }
        }
    }

    /// Whether triggers gather yet what the human needs to answer the
    /// permission request the agent waits on.
    #[must_use]
    pub fn gathering(&self) -> bool {
        match (self.state, self.entries.last()) {
            (State::Asking, Some(Entry::Permission { gathered, .. })) => gathered
                .iter()
                .any(|check| check.state == CheckState::Running),
            _ => false,
        }
    }

    /// The headlines of what the triggers gathered for the permission
    /// request the agent waits on: the first line each said.
    #[must_use]
    pub fn headlines(&self) -> Vec<&str> {
        match (self.state, self.entries.last()) {
            (State::Asking, Some(Entry::Permission { gathered, .. })) => gathered
                .iter()
                .filter(|check| matches!(check.state, CheckState::Passed | CheckState::Failed))
                .filter_map(|check| check.said.as_deref().and_then(headline))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// What the triggers gathered for the last permission request, as it
    /// stands, to change.
    fn gathered_mut(&mut self) -> Option<&mut Vec<Check>> {
        let index = self
            .entries
            .iter()
            .rposition(|entry| matches!(entry, Entry::Permission { .. }))?;
        self.changed.insert(index);
        match &mut self.entries[index] {
            Entry::Permission { gathered, .. } => Some(gathered),
            _ => None,
        }
    }

    /// Chooses the option at `index` of the permission request the agent
    /// waits on, and returns the answer to send.
    pub fn choose(&mut self, index: usize) -> Option<Command> {
        if self.state != State::Asking {
            return None;
        }
        let Some(Entry::Permission {
            options, chosen, ..
        }) = self.entries.last_mut()
        else {
            return None;
        };
        if index >= options.len() {
            return None;
        }
        *chosen = Some(index);
        self.changed.insert(self.entries.len() - 1);
        self.state = State::Busy;
        // The human answered as the plan stands: it is the one the plan
        // is told against from here.
        let planned = self.entries.iter().rev().find_map(|entry| match entry {
            Entry::Plan(steps) => Some(texts(steps)),
            _ => None,
        });
        match planned {
            Some(steps) => self.set_baseline(steps),
            None => self.drift.rebase = true,
        }
        Some(Command::Answer(PermissionAnswer::Select(index)))
    }

    /// Takes `steps` as the baseline, the plan told against from here.
    fn set_baseline(&mut self, steps: Vec<String>) {
        self.drift.rebase = false;
        self.drift.from = self.entries.len();
        if self.drift.baseline.as_ref() != Some(&steps) {
            self.drift.baseline = Some(steps);
            self.drift.unkept = true;
        }
    }

    /// The steps of the plan the agent first sent after the human's last
    /// line, once it sent one.
    #[must_use]
    pub fn baseline(&self) -> Option<&[String]> {
        self.drift.baseline.as_deref()
    }

    /// Whether the baseline changed since this was last called, to keep.
    pub fn take_baseline_changed(&mut self) -> bool {
        std::mem::take(&mut self.drift.unkept)
    }

    /// What the plan's change since the baseline tells the triggers that
    /// run on it, once the agent changed a step of it, if it did since
    /// this was last called.
    pub fn take_plan_changed(&mut self) -> Option<String> {
        self.drift.fired.take()
    }

    /// How many times the agent changed a step of the plan since the
    /// baseline, as the human is told each time, and what the last time
    /// says, as the news of it.
    #[must_use]
    pub fn plan_changes(&self) -> (usize, Option<String>) {
        let last = self.entries.iter().rev().find_map(|entry| match entry {
            Entry::PlanChanged { added, removed } => Some(changed_steps(added, removed)),
            _ => None,
        });
        (self.drift.changes, last)
    }

    /// Tells the steps of the plan the agent sent, `steps`, against the
    /// baseline, or takes them as the baseline, as the first after a
    /// line of the human's: a step added, removed or changed shows how
    /// the plan stands against it, and is fired for the triggers that run
    /// on it. A step that only went further changes nothing.
    fn follow_plan(&mut self, steps: Vec<String>) {
        let Some(baseline) = self.drift.baseline.as_ref().filter(|_| !self.drift.rebase) else {
            self.set_baseline(steps);
            return;
        };
        let added: Vec<String> = steps
            .iter()
            .filter(|step| !baseline.contains(step))
            .cloned()
            .collect();
        let removed: Vec<String> = baseline
            .iter()
            .filter(|step| !steps.contains(step))
            .cloned()
            .collect();
        let told = self.entries[self.drift.from.min(self.entries.len())..]
            .iter()
            .rev()
            .find_map(|entry| match entry {
                Entry::PlanChanged { added, removed } => Some((added.clone(), removed.clone())),
                _ => None,
            })
            .unwrap_or_default();
        if (&added, &removed) == (&told.0, &told.1) {
            return;
        }
        let new_added: Vec<String> = added
            .iter()
            .filter(|step| !told.0.contains(step))
            .cloned()
            .collect();
        let new_removed: Vec<String> = removed
            .iter()
            .filter(|step| !told.1.contains(step))
            .cloned()
            .collect();
        if !new_added.is_empty() || !new_removed.is_empty() {
            self.drift.changes += 1;
            self.drift.fired = Some(format!(
                "The chat's agent changed the plan it started the human's last line with, \
                 which the human left it to.\n\n{}",
                changed_steps(&added, &removed)
            ));
        }
        self.push(Entry::PlanChanged { added, removed });
    }

    /// Asks the agent to stop the turn it is taking, if one is running and
    /// the human has not asked already, and returns the command to send. The
    /// conversation stays: once the turn ends, the human can send the next
    /// line.
    pub fn cancel(&mut self) -> Option<Command> {
        match self.state {
            State::Busy | State::Asking => {
                self.state = State::Stopping;
                Some(Command::Cancel)
            }
            // What runs as the turn ended stops; there is no turn to stop.
            State::Checking => {
                self.push(Entry::Note("Stopped.".to_owned()));
                self.stop_checks(CheckState::Waiting);
                self.waiting.clear();
                None
            }
            State::Idle | State::Stopping | State::Closed => None,
        }
    }

    /// The turn ended as `reason` says: a turn the agent ended as it
    /// does, having done what it was asked, is when the goal is looked
    /// at, and the triggers due run.
    fn turn_ended(&mut self, reason: StopReason) {
        if let Some(note) = ended(reason) {
            self.push(Entry::Note(note.to_owned()));
        }
        // A line steered into the turn, which the agent side has not
        // taken yet, starts a turn of its own, which ends the human's line.
        if self.steering() {
            self.state = State::Busy;
            return;
        }
        // A trigger by: this tells the agent once for a line, so that the
        // turn it starts does not tell it again.
        let mut due = std::mem::take(&mut self.due);
        due.retain(|due| !matches!(due.does, Does::Tell(_)) || !self.told_line.contains(&due.file));
        let check = reason == StopReason::EndTurn
            && self.state == State::Busy
            && (self.goal.is_some() || !due.is_empty());
        if check {
            self.start_checks(due);
        } else {
            self.state = State::Idle;
        }
    }

    /// The index of the entry of tool call `id`, if it has one.
    fn tool_call_entry(&self, id: &str) -> Option<usize> {
        self.entries.iter().rposition(
            |entry| matches!(entry, Entry::ToolCall { id: shown_id, .. } if shown_id == id),
        )
    }

    /// Shows news of a tool call: in place of the call's entry once it has
    /// one, so a call shows as running until it ends. What a permission
    /// request for the call shows already, the call does not show again.
    fn show_tool_call(
        &mut self,
        id: String,
        title: Option<String>,
        content: Option<Vec<Content>>,
        result: Option<ToolCallResult>,
    ) {
        let Some(index) = self.tool_call_entry(&id) else {
            self.push(Entry::ToolCall {
                id,
                title: title.unwrap_or_else(|| "a tool call".to_owned()),
                content: content.unwrap_or_default(),
                result,
            });
            return;
        };
        let content = content.map(|content| {
            let asked = self.entries[index + 1..].iter().any(|entry| {
                matches!(entry, Entry::Permission { id: asked_id, content: asked, .. }
                    if *asked_id == id && *asked == content)
            });
            if asked { Vec::new() } else { content }
        });
        self.changed.insert(index);
        if let Entry::ToolCall {
            title: shown_title,
            content: shown_content,
            result: shown_result,
            ..
        } = &mut self.entries[index]
        {
            if let Some(title) = title {
                *shown_title = title;
            }
            if let Some(content) = content {
                *shown_content = content;
            }
            *shown_result = result;
        }
    }

    /// Shows the plan the agent works to: in place of the plan it sent
    /// before in the turn, if it did, so that the turn shows one plan, as
    /// it stands.
    fn plan(&mut self, steps: Vec<Step>) {
        let texts = texts(&steps);
        let turn = self
            .entries
            .iter()
            .rposition(is_turn_start)
            .map_or(0, |start| start + 1);
        let planned = self.entries[turn..]
            .iter()
            .rposition(|entry| matches!(entry, Entry::Plan(_)));
        match planned {
            Some(index) => {
                self.entries[turn + index] = Entry::Plan(steps);
                self.changed.insert(turn + index);
            }
            None => self.push(Entry::Plan(steps)),
        }
        self.follow_plan(texts);
    }

    /// The title the agent gave its session, if it gave one.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The first line sent to the agent, by the human or by Cox in their
    /// place, if one was.
    #[must_use]
    pub fn first_line(&self) -> Option<&str> {
        self.entries.iter().find_map(|entry| match entry {
            Entry::Human(text) | Entry::Cox(text) => {
                text.lines().map(str::trim).find(|line| !line.is_empty())
            }
            _ => None,
        })
    }

    /// The first line of what came last in the last turn, the agent's
    /// reply or Cox's note of how the turn ended, if anything did.
    #[must_use]
    pub fn last_said(&self) -> Option<&str> {
        self.last_told()?
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
    }

    /// What the agent told in the last turn, whole, as [`Self::last_told`]
    /// says, but for what ran as the turn ended and the notes it added,
    /// which the judge of the goal, or the agent of a trigger, is not told
    /// as the turn's.
    #[must_use]
    pub fn turn_told(&self) -> Option<&str> {
        let turn: Vec<&Entry> = self
            .entries
            .iter()
            .rev()
            .take_while(|entry| !is_turn_start(entry))
            .collect();
        let after = turn
            .iter()
            .position(|entry| entry.checks().is_some())
            .map_or(0, |at| at + 1);
        turn.into_iter().skip(after).find_map(|entry| match entry {
            Entry::Reply(text) | Entry::Note(text) => Some(text.as_str()),
            _ => None,
        })
    }

    /// What came last in the last turn, whole: the agent's reply or Cox's
    /// note of how the turn ended, if anything did.
    #[must_use]
    pub fn last_told(&self) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .take_while(|entry| !is_turn_start(entry))
            .find_map(|entry| match entry {
                Entry::Reply(text) | Entry::Note(text) => Some(text.as_str()),
                _ => None,
            })
    }

    /// What the agent asks permission for, or that it asks the human to log
    /// in, while it waits on the human.
    #[must_use]
    pub fn asking_for(&self) -> Option<&str> {
        match (self.state, self.entries.last()) {
            (State::Asking, Some(Entry::Permission { title, .. })) => Some(title),
            (State::Asking, Some(Entry::Login { .. })) => Some("log in"),
            _ => None,
        }
    }

    /// The plan the agent worked to in the last turn, as it stands, if it
    /// sent one: the turn that runs, or the one that ended, until the
    /// human sends the next line.
    #[must_use]
    pub fn last_plan(&self) -> Option<&[Step]> {
        self.entries
            .iter()
            .rev()
            .take_while(|entry| !is_turn_start(entry))
            .find_map(|entry| match entry {
                Entry::Plan(steps) => Some(steps.as_slice()),
                _ => None,
            })
    }

    /// Shows a permission request with what its tool call is about to do:
    /// what the request tells, or else what the call shows, which then
    /// shows only here. The request waits on the human, unless Cox
    /// `decided` it already, as the chat's setup says.
    fn ask(
        &mut self,
        id: String,
        title: Option<String>,
        content: Option<Vec<Content>>,
        options: Vec<String>,
        decided: Option<usize>,
    ) {
        let call = self.tool_call_entry(&id);
        let shown = call.and_then(|index| match &mut self.entries[index] {
            Entry::ToolCall { content, .. } => Some(content),
            _ => None,
        });
        let content = match shown {
            Some(shown) => {
                let content = content.unwrap_or_else(|| shown.clone());
                if *shown == content {
                    shown.clear();
                    self.changed.extend(call);
                }
                content
            }
            None => content.unwrap_or_default(),
        };
        self.push(Entry::Permission {
            id,
            title: title.unwrap_or_else(|| "a tool call".to_owned()),
            content,
            options,
            chosen: decided,
            by_setup: decided.is_some(),
            gathered: Vec::new(),
        });
    }

    /// Takes `commands` as those the agent runs now.
    fn commands_told(&mut self, commands: Vec<SlashCommand>) {
        if commands != self.commands {
            self.commands = commands;
            self.unkept.commands = true;
        }
    }

    /// Takes `settings` as how the session is set now.
    fn settings_set(&mut self, settings: Vec<Setting>) {
        if settings != self.settings {
            self.settings = settings;
            self.unkept.settings = true;
        }
    }

    /// Notes how the agent's session opened, as `how` says.
    fn opened(&mut self, how: Opened) {
        match how {
            Opened::New => {}
            Opened::Resumed => {
                self.push(Entry::Note(
                    "Resumed: the agent goes on from here.".to_owned(),
                ));
            }
            Opened::Retold {
                lost,
                instructions,
                context,
            } => {
                // A chat with no session to go on with, as one that
                // moved into a sketch, goes on as if it had one.
                if lost {
                    self.push(Entry::Note(
                    "The agent could not resume its session: a new one is told the conversation so far."
                        .to_owned(),
                ));
                }
                for path in instructions {
                    self.push(Entry::Note(instructions_note(&path)));
                }
                if let Some(context) = context {
                    self.push(Entry::Note(context_note(&context)));
                }
            }
        }
    }

    /// Shows what the agent side sent.
    pub fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Titled(title) => self.title = Some(title),
            AgentEvent::Commands(commands) => self.commands_told(commands),
            AgentEvent::Settings(settings) => self.settings_set(settings),
            AgentEvent::SettingFailed { name, error } => {
                self.push(Entry::Note(format!(
                    "The agent did not set {name}: {error}"
                )));
            }
            AgentEvent::Steered => self.steering = self.steering.saturating_sub(1),
            AgentEvent::Opened { how, steers, .. } => {
                self.steers = steers;
                self.opened(how);
            }
            AgentEvent::ReplyText(text) => {
                if let Some(Entry::Reply(reply)) = self.entries.last_mut() {
                    reply.push_str(&text);
                    self.changed.insert(self.entries.len() - 1);
                } else {
                    self.push(Entry::Reply(text));
                }
            }
            AgentEvent::ThoughtText(text) => {
                if let Some(Entry::Thought(thought)) = self.entries.last_mut() {
                    thought.push_str(&text);
                    self.changed.insert(self.entries.len() - 1);
                } else {
                    self.push(Entry::Thought(text));
                }
            }
            AgentEvent::Plan(steps) => self.plan(steps),
            AgentEvent::ToolCall {
                id,
                title,
                content,
                result,
            } => self.show_tool_call(id, title, content, result),
            AgentEvent::PermissionRequest {
                id,
                title,
                content,
                options,
            } => {
                self.ask(id, title, content, options, None);
                // Asked before the agent heard it is to stop: the agent
                // side answers it as cancelled, without the human.
                if self.state != State::Stopping {
                    self.state = State::Asking;
                }
            }
            // Cox answered, as the chat is set up: the human sees what was
            // asked, and what was chosen, and the turn goes on.
            AgentEvent::PermissionDecided {
                id,
                title,
                content,
                options,
                chosen,
            } => self.ask(id, title, content, options, Some(chosen)),
            AgentEvent::TurnEnded(reason) => self.turn_ended(reason),
            AgentEvent::LoginNeeded { options, failed } => self.ask_to_log_in(options, failed),
            AgentEvent::Note(note) => self.push(Entry::Note(note)),
            AgentEvent::TurnFailed(error) => {
                self.push(Entry::Note(format!("The turn failed: {error}")));
                // A line steered into it, which the agent side has not
                // taken yet, starts a turn of its own.
                self.state = if self.steering() {
                    State::Busy
                } else {
                    State::Idle
                };
            }
            AgentEvent::Closed(error) => self.agent_closed(error),
        }
    }

    /// The agent closed, for `error` if it failed.
    fn agent_closed(&mut self, error: Option<String>) {
        let note = match error {
            Some(error) => format!("The agent closed: {error}"),
            None => "The agent closed.".to_owned(),
        };
        self.push(Entry::Note(note));
        self.state = State::Closed;
        self.steering = 0;
        // The commands stay, for the line that starts the agent again.
    }
}

/// What the card that asks the human to log in says, with why the way
/// tried last `failed`, if it did.
#[must_use]
pub fn login_title(failed: Option<&str>) -> String {
    match failed {
        Some(failed) => format!("The agent asks you to log in again: {failed}"),
        None => "The agent asks you to log in.".to_owned(),
    }
}

/// What the button of a way to log in says: its name, what it is, and, for
/// a way carried out in a terminal, that it runs there.
#[must_use]
pub fn login_label(option: &LoginOption) -> String {
    let mut label = option.name.clone();
    if let Some(description) = option.description.as_deref().map(str::trim)
        && !description.is_empty()
    {
        label = format!("{label}: {description}");
    }
    if option.terminal.is_some() {
        label.push_str(" (in a terminal)");
    }
    label
}

/// The note that says the human switched the chat from the agent named
/// `from` to the one named `to`.
#[must_use]
pub fn switched_note(from: &str, to: &str) -> String {
    format!("Agent: {to}, in place of {from}. It goes on from the conversation so far.")
}

/// The note that tells the human which instructions the agent gets,
/// those of the file at `path`.
#[must_use]
pub fn instructions_note(path: &Path) -> String {
    format!("Instructions: {}", crate::ui::paths::shortened(path))
}

/// The note that tells the human the context the agent was set up with,
/// by the chat that opened its chat.
#[must_use]
pub fn context_note(context: &str) -> String {
    format!("Context: {}", context.trim())
}

/// The note that tells the human a chat works in `folder`, a sketch's,
/// apart from the project.
#[must_use]
pub fn sketch_note(folder: &Path) -> String {
    format!(
        "Sketch: works in {}, apart from the project.",
        folder.display()
    )
}

/// The note that tells the human a chat works in `folder`, in place of
/// its project's, as the chat of a schedule works where it says.
#[must_use]
pub fn folder_note(folder: &Path) -> String {
    format!("Works in: {}", crate::ui::paths::shortened(folder))
}

/// Whether `entry` starts a turn: a line sent to the agent, by the human
/// or by Cox in their place.
fn is_turn_start(entry: &Entry) -> bool {
    matches!(entry, Entry::Human(_) | Entry::Cox(_))
}

/// The last `length` characters of `text`, at most, less blank lines at
/// its end, with a mark of what is left out.
/// The start of `text`, at most `length` characters of it.
fn head(text: &str, length: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(length) {
        Some((end, _)) => format!("{}[…]", &text[..end]),
        None => text.to_owned(),
    }
}

/// The headline of what a trigger gathered, `said`: its first line, as
/// it reads, without the marks of a Markdown heading or strong text.
#[must_use]
pub fn headline(said: &str) -> Option<&str> {
    let line = said.lines().map(str::trim).find(|line| !line.is_empty())?;
    let line = line.trim_start_matches('#').trim();
    let line = line
        .strip_prefix("**")
        .and_then(|line| line.strip_suffix("**"))
        .unwrap_or(line);
    Some(line)
}

fn tail(text: &str, length: usize) -> String {
    let text = text.trim_end();
    let count = text.chars().count();
    if count <= length {
        return text.to_owned();
    }
    let kept: String = text.chars().skip(count - length).collect();
    format!("[…]{kept}")
}

/// What the conversation notes of a turn that ended for `reason`: nothing
/// when the agent ended it as it does, having done what it was asked.
fn ended(reason: StopReason) -> Option<&'static str> {
    match reason {
        StopReason::EndTurn => None,
        StopReason::Cancelled => Some("Stopped."),
        StopReason::MaxTokens => Some("The turn ended: the agent's reply grew too long."),
        StopReason::MaxTurnRequests => {
            Some("The turn ended: the agent asked the model as often as a turn allows.")
        }
        StopReason::Refusal => Some("The turn ended: the agent refused to go on."),
        StopReason::Other => Some("The turn ended, for a reason Cox does not know."),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::agent::Prompt;

    fn asks_to_log_in() -> AgentEvent {
        AgentEvent::LoginNeeded {
            options: vec![
                LoginOption {
                    id: "key".to_owned(),
                    name: "API key".to_owned(),
                    description: Some("Use the key ".to_owned()),
                    terminal: None,
                },
                LoginOption {
                    id: "terminal".to_owned(),
                    name: "Claude".to_owned(),
                    description: None,
                    terminal: Some(TerminalLogin {
                        args: vec!["login".to_owned()],
                        env: BTreeMap::new(),
                    }),
                },
            ],
            failed: None,
        }
    }

    #[test]
    fn waits_for_the_human_to_log_in_and_goes_back_to_the_turn() {
        let mut conversation = Conversation::new();
        conversation.submit("hello");
        conversation.apply(asks_to_log_in());

        assert_eq!(conversation.state(), State::Asking);
        assert!(conversation.logging_in());
        assert_eq!(conversation.options_asked(), 2);
        assert_eq!(conversation.asking_for(), Some("log in"));
        assert_eq!(conversation.choose_login(2), None);
        let chosen = conversation.choose_login(1).expect("there is a second way");
        assert_eq!(chosen.id, "terminal");
        assert_eq!(conversation.state(), State::Busy);
        assert!(!conversation.logging_in());
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Login {
                options: match asks_to_log_in() {
                    AgentEvent::LoginNeeded { options, .. } => options,
                    _ => unreachable!(),
                },
                failed: None,
                chosen: Some(1),
            })
        );
    }

    #[test]
    fn the_agents_commands_are_kept_and_stay_once_it_closes() {
        let review = SlashCommand {
            name: "review".to_owned(),
            description: "Review the changes".to_owned(),
            hint: None,
        };
        let mut conversation = Conversation::new();
        conversation.apply(AgentEvent::Commands(vec![review.clone()]));
        assert!(conversation.take_commands_changed());
        conversation.apply(AgentEvent::Commands(vec![review.clone()]));
        assert!(!conversation.take_commands_changed());

        conversation.apply(AgentEvent::Closed(None));
        assert_eq!(conversation.commands(), std::slice::from_ref(&review));

        conversation.switch("claude", "codex");
        assert_eq!(conversation.commands(), []);
    }

    #[test]
    fn logging_in_before_a_line_is_sent_leaves_the_chat_idle() {
        let mut conversation = Conversation::new();
        conversation.apply(asks_to_log_in());
        assert_eq!(conversation.submit("hello"), None);
        conversation.choose_login(0);

        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.submit("hello"),
            Some(Command::Prompt("hello".to_owned().into()))
        );
    }

    #[test]
    fn what_the_human_handed_goes_with_the_line_and_shows_after_it() {
        let mut conversation = Conversation::new();
        let attached = vec![Attachment {
            label: "main.rs:2".to_owned(),
            uri: "file:///work/src/main.rs#L2".to_owned(),
            text: "println!(\"hello\");".to_owned(),
            picture: None,
        }];

        assert_eq!(
            conversation.submit_with("why hello?", attached.clone()),
            Some(Command::Prompt(Prompt {
                text: "why hello?".to_owned(),
                attached: attached.clone(),
            }))
        );
        assert_eq!(
            conversation.entries(),
            [
                Entry::Human("why hello?".to_owned()),
                Entry::Attached(attached.clone()),
            ]
        );
        // Not while a turn runs: the human sends it again after.
        assert_eq!(conversation.submit_with("and?", attached), None);
        assert_eq!(conversation.entries().len(), 2);
    }

    #[test]
    fn names_each_way_to_log_in_with_what_it_is_and_where_it_runs() {
        let AgentEvent::LoginNeeded { options, .. } = asks_to_log_in() else {
            unreachable!();
        };
        assert_eq!(login_label(&options[0]), "API key: Use the key");
        assert_eq!(login_label(&options[1]), "Claude (in a terminal)");
        assert_eq!(login_title(None), "The agent asks you to log in.");
        assert_eq!(
            login_title(Some("no key")),
            "The agent asks you to log in again: no key"
        );
    }

    #[test]
    fn a_closed_conversation_reopens_to_go_on() {
        let mut conversation = Conversation::closed(vec![
            Entry::Human("hello".to_owned()),
            Entry::Reply("echo: hello".to_owned()),
        ]);
        assert_eq!(conversation.submit("again"), None);

        conversation.reopen();
        assert_eq!(
            conversation.submit("again"),
            Some(Command::Prompt("again".to_owned().into()))
        );
        conversation.apply(AgentEvent::Opened {
            id: "session-1".to_owned(),
            how: Opened::Resumed,
            steers: false,
        });
        assert_eq!(conversation.take_changed(), [2, 3]);
        assert_eq!(
            conversation.entries()[2..],
            [
                Entry::Human("again".to_owned()),
                Entry::Note("Resumed: the agent goes on from here.".to_owned()),
            ]
        );
    }

    #[test]
    fn sends_a_line_when_idle_and_waits_for_the_turn() {
        let mut conversation = Conversation::new();

        assert_eq!(
            conversation.submit("hello"),
            Some(Command::Prompt("hello".to_owned().into()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.submit("again"), None);
    }

    /// A conversation whose agent steers, as its session said as it
    /// opened.
    fn steering() -> Conversation {
        let mut conversation = Conversation::new();
        conversation.apply(AgentEvent::Opened {
            id: "session-1".to_owned(),
            how: Opened::New,
            steers: true,
        });
        conversation
    }

    #[test]
    fn steers_a_line_into_the_turn_of_an_agent_that_takes_it() {
        let mut conversation = steering();
        assert!(!conversation.steers());
        conversation.submit("hello");
        assert!(conversation.steers());
        conversation.apply(AgentEvent::ReplyText("Working".to_owned()));

        let attached = vec![Attachment {
            label: "main.rs:1".to_owned(),
            uri: "file:///main.rs#L1".to_owned(),
            text: "fn main() {}".to_owned(),
            picture: None,
        }];
        assert_eq!(
            conversation.submit_with("and this too", attached.clone()),
            Some(Command::Steer(Prompt {
                text: "and this too".to_owned(),
                attached: attached.clone(),
            }))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert!(conversation.steering());
        conversation.apply(AgentEvent::Steered);
        assert!(!conversation.steering());
        conversation.apply(AgentEvent::ReplyText("Both done.".to_owned()));
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));

        assert_eq!(
            conversation.entries(),
            [
                Entry::Human("hello".to_owned()),
                Entry::Reply("Working".to_owned()),
                Entry::Human("and this too".to_owned()),
                Entry::Attached(attached),
                Entry::Reply("Both done.".to_owned()),
            ]
        );
        assert_eq!(conversation.state(), State::Idle);
        assert!(!conversation.steers());
    }

    #[test]
    fn holds_a_line_while_an_agent_that_does_not_steer_takes_a_turn() {
        let mut conversation = Conversation::new();
        conversation.submit("hello");

        assert!(!conversation.steers());
        assert_eq!(conversation.submit("and this too"), None);
        assert_eq!(conversation.entries(), [Entry::Human("hello".to_owned())]);
    }

    #[test]
    fn a_turn_that_ends_before_the_agent_side_took_a_steered_line_goes_on() {
        let mut conversation = steering();
        conversation.submit("hello");
        conversation.submit("and this too");

        // The turn ended before the agent side heard the line, which
        // starts a turn of its own.
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));
        assert_eq!(conversation.state(), State::Busy);
        conversation.apply(AgentEvent::Steered);
        conversation.apply(AgentEvent::ReplyText("Done too.".to_owned()));
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));

        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn steers_no_line_into_a_turn_the_human_stopped() {
        let mut conversation = steering();
        conversation.submit("hello");
        conversation.cancel();

        assert!(!conversation.steers());
        assert_eq!(conversation.submit("and this too"), None);
    }

    #[test]
    fn joins_reply_pieces_and_returns_to_idle_when_the_turn_ends_as_it_does() {
        let mut conversation = Conversation::new();
        conversation.submit("hello");

        conversation.apply(AgentEvent::ReplyText("echo: ".to_owned()));
        conversation.apply(AgentEvent::ReplyText("hello".to_owned()));
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));

        assert_eq!(
            conversation.entries(),
            [
                Entry::Human("hello".to_owned()),
                Entry::Reply("echo: hello".to_owned()),
            ]
        );
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn shows_the_commits_of_a_turn_and_names_the_line_it_started_on() {
        let mut conversation = Conversation::new();
        conversation.submit("commit it");
        conversation.apply(AgentEvent::ReplyText("done".to_owned()));
        assert_eq!(conversation.turn_start(), Some((0, "commit it")));
        conversation.committed(Vec::new());
        let commit = Committed {
            folder: "/work".into(),
            repository: None,
            id: "0123456789abcdef".to_owned(),
            subject: "Commit it".to_owned(),
            added: 1,
            removed: 0,
            amended: None,
        };
        conversation.committed(vec![commit.clone()]);
        assert_eq!(commit.short(), "0123456");
        assert_eq!(
            conversation.entries()[1..],
            [
                Entry::Reply("done".to_owned()),
                Entry::Commits(vec![commit])
            ]
        );
    }

    #[test]
    fn joins_thought_pieces_apart_from_the_reply() {
        let mut conversation = Conversation::new();
        conversation.submit("hello");

        conversation.apply(AgentEvent::ThoughtText("Say ".to_owned()));
        conversation.apply(AgentEvent::ThoughtText("it back.".to_owned()));
        conversation.apply(AgentEvent::ReplyText("echo".to_owned()));
        conversation.apply(AgentEvent::ThoughtText("Done.".to_owned()));

        assert_eq!(
            conversation.entries()[1..],
            [
                Entry::Thought("Say it back.".to_owned()),
                Entry::Reply("echo".to_owned()),
                Entry::Thought("Done.".to_owned()),
            ]
        );
    }

    fn step(text: &str, state: StepState) -> Step {
        Step {
            text: text.to_owned(),
            state,
        }
    }

    #[test]
    fn tells_the_steps_the_agent_changed_from_the_plan_the_human_left_it_to() {
        let mut conversation = Conversation::new();
        conversation.submit("fix it");
        let planned = vec![
            step("Read", StepState::InProgress),
            step("Fix", StepState::Pending),
        ];
        conversation.apply(AgentEvent::Plan(planned));
        assert_eq!(
            conversation.baseline(),
            Some(&["Read".to_owned(), "Fix".to_owned()][..])
        );
        assert!(conversation.take_baseline_changed());

        // A step that only goes further changes nothing.
        conversation.apply(AgentEvent::Plan(vec![
            step("Read", StepState::Completed),
            step("Fix", StepState::InProgress),
        ]));
        assert_eq!(conversation.take_plan_changed(), None);
        assert_eq!(conversation.plan_changes().0, 0);

        // A step it adds, and one it leaves out, are told.
        conversation.apply(AgentEvent::Plan(vec![
            step("Read", StepState::Completed),
            step("Skip the tests: they need a network", StepState::InProgress),
        ]));
        let told = conversation.take_plan_changed().unwrap();
        assert!(
            told.contains("+ Skip the tests: they need a network"),
            "{told}"
        );
        assert!(told.contains("- Fix"), "{told}");
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::PlanChanged {
                added: vec!["Skip the tests: they need a network".to_owned()],
                removed: vec!["Fix".to_owned()],
            })
        );
        assert_eq!(
            conversation.plan_changes(),
            (
                1,
                Some("+ Skip the tests: they need a network\n- Fix".to_owned())
            )
        );
        // Told once.
        conversation.apply(AgentEvent::Plan(vec![
            step("Read", StepState::Completed),
            step("Skip the tests: they need a network", StepState::Completed),
        ]));
        assert_eq!(conversation.take_plan_changed(), None);
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));

        // A line Cox sends in the human's place leaves the baseline be.
        conversation.told("go on");
        conversation.apply(AgentEvent::Plan(vec![step("Push", StepState::Pending)]));
        assert!(conversation.take_plan_changed().is_some());
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));

        // The human's next line leaves the plan sent after it as the new
        // baseline.
        conversation.submit("now push");
        conversation.apply(AgentEvent::Plan(vec![step("Push", StepState::Pending)]));
        assert_eq!(conversation.baseline(), Some(&["Push".to_owned()][..]));
        assert_eq!(conversation.take_plan_changed(), None);
    }

    #[test]
    fn asks_to_merge_once_the_turn_ends() {
        let mut conversation = Conversation::new();
        conversation.submit("fix it");
        assert_eq!(conversation.merge("Merge it."), None);
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));

        assert!(conversation.merge("Merge it.").is_some());

        let entries = conversation.entries();
        assert_eq!(
            entries[entries.len() - 2..],
            [Entry::Merge, Entry::Cox("Merge it.".to_owned())]
        );
        assert_eq!(conversation.state(), State::Busy);
    }

    #[test]
    fn answering_the_agent_takes_the_plan_as_it_stands_as_the_baseline() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        conversation.apply(AgentEvent::Plan(vec![step("Edit", StepState::Pending)]));
        conversation.apply(AgentEvent::Plan(vec![
            step("Edit", StepState::Pending),
            step("Also rename it", StepState::Pending),
        ]));
        assert!(conversation.take_plan_changed().is_some());
        conversation.apply(AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("Edit x.txt".to_owned()),
            content: None,
            options: vec!["Allow".to_owned(), "Reject".to_owned()],
        });

        conversation.choose(0);
        assert_eq!(
            conversation.baseline(),
            Some(&["Edit".to_owned(), "Also rename it".to_owned()][..])
        );
        conversation.apply(AgentEvent::Plan(vec![
            step("Edit", StepState::Completed),
            step("Also rename it", StepState::InProgress),
        ]));
        assert_eq!(conversation.take_plan_changed(), None);
    }

    #[test]
    fn shows_one_plan_a_turn_as_it_stands() {
        let mut conversation = Conversation::new();
        conversation.submit("plan");
        let first = vec![
            step("Read", StepState::InProgress),
            step("Fix", StepState::Pending),
        ];
        let then = vec![
            step("Read", StepState::Completed),
            step("Fix", StepState::InProgress),
        ];

        conversation.apply(AgentEvent::Plan(first.clone()));
        assert_eq!(conversation.last_plan(), Some(&first[..]));
        conversation.apply(AgentEvent::ReplyText("reading".to_owned()));
        conversation.take_changed();
        conversation.apply(AgentEvent::Plan(then.clone()));
        assert_eq!(conversation.take_changed(), [1]);
        assert_eq!(
            conversation.entries()[1..],
            [
                Entry::Plan(then.clone()),
                Entry::Reply("reading".to_owned())
            ]
        );
        assert_eq!(conversation.last_plan(), Some(&then[..]));

        // What the turn did stays in sight once it ends.
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));
        assert_eq!(conversation.last_plan(), Some(&then[..]));
        // The next turn plans anew.
        conversation.submit("again");
        assert_eq!(conversation.last_plan(), None);
        conversation.apply(AgentEvent::Plan(first.clone()));
        assert_eq!(conversation.entries().len(), 5);
        assert_eq!(conversation.entries()[1], Entry::Plan(then));
    }

    #[test]
    fn shows_a_tool_call_as_running_until_it_ends() {
        let mut conversation = Conversation::new();
        conversation.submit("tool");
        let tool_call = |title: Option<&str>, result| AgentEvent::ToolCall {
            id: "call-1".to_owned(),
            title: title.map(str::to_owned),
            content: None,
            result,
        };

        conversation.apply(tool_call(Some("ls"), None));
        assert_eq!(
            conversation.entries()[1],
            Entry::ToolCall {
                id: "call-1".to_owned(),
                title: "ls".to_owned(),
                content: Vec::new(),
                result: None,
            }
        );

        conversation.apply(AgentEvent::ReplyText("listing".to_owned()));
        conversation.apply(tool_call(None, Some(ToolCallResult::Completed)));
        assert_eq!(
            conversation.entries()[1..],
            [
                Entry::ToolCall {
                    id: "call-1".to_owned(),
                    title: "ls".to_owned(),
                    content: Vec::new(),
                    result: Some(ToolCallResult::Completed),
                },
                Entry::Reply("listing".to_owned()),
            ]
        );
    }

    /// News of tool call `call-1`, showing `content` when it is given.
    fn tool_call(content: Option<Vec<Content>>, result: Option<ToolCallResult>) -> AgentEvent {
        AgentEvent::ToolCall {
            id: "call-1".to_owned(),
            title: Some("cargo xtask ci".to_owned()),
            content,
            result,
        }
    }

    /// What tool call `call-1` and the permission requests for it show.
    fn shown(conversation: &Conversation) -> Vec<(&'static str, Vec<Content>)> {
        conversation
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::ToolCall { content, .. } => Some(("call", content.clone())),
                Entry::Permission { content, .. } => Some(("permission", content.clone())),
                _ => None,
            })
            .collect()
    }

    fn text(text: &str) -> Content {
        Content::Text(text.to_owned())
    }

    fn permission(content: Option<Vec<Content>>) -> AgentEvent {
        AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("cargo xtask ci".to_owned()),
            content,
            options: vec!["Allow once".to_owned()],
        }
    }

    #[test]
    fn replaces_what_a_tool_call_shows_when_it_changes() {
        let mut conversation = Conversation::new();
        conversation.submit("tool");

        conversation.apply(tool_call(Some(vec![text("Run the checks")]), None));
        conversation.apply(tool_call(None, None));
        assert_eq!(
            shown(&conversation),
            [("call", vec![text("Run the checks")])]
        );

        conversation.apply(tool_call(
            Some(vec![text("error: 1 test failed")]),
            Some(ToolCallResult::Failed),
        ));
        assert_eq!(
            shown(&conversation),
            [("call", vec![text("error: 1 test failed")])]
        );
    }

    #[test]
    fn shows_what_a_permission_request_asks_for_only_there() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        let change = || {
            vec![Content::Diff {
                path: "/work/x.txt".to_owned(),
                lines: cox_core::diff::lines(Some("one\n"), "1\n"),
            }]
        };

        conversation.apply(tool_call(Some(change()), None));
        conversation.apply(permission(Some(change())));
        assert_eq!(
            shown(&conversation),
            [("call", Vec::new()), ("permission", change())]
        );

        conversation.choose(0);
        // Told again, the change still shows only where it was asked for.
        conversation.apply(tool_call(Some(change()), None));
        assert_eq!(
            shown(&conversation),
            [("call", Vec::new()), ("permission", change())]
        );
        conversation.apply(tool_call(
            Some(vec![text("done")]),
            Some(ToolCallResult::Completed),
        ));
        assert_eq!(
            shown(&conversation),
            [("call", vec![text("done")]), ("permission", change())]
        );
    }

    #[test]
    fn a_permission_request_that_tells_nothing_shows_what_its_call_shows() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");

        conversation.apply(tool_call(Some(vec![text("Run the checks")]), None));
        conversation.apply(permission(None));

        assert_eq!(
            shown(&conversation),
            [
                ("call", Vec::new()),
                ("permission", vec![text("Run the checks")])
            ]
        );
    }

    #[test]
    fn shows_what_the_triggers_gathered_for_the_human_to_answer_with() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        conversation.apply(AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("Edit x.txt".to_owned()),
            content: Some(vec![Content::Diff {
                path: "x.txt".to_owned(),
                lines: vec![
                    Line::Removed("old".to_owned()),
                    Line::Added("new".to_owned()),
                ],
            }]),
            options: vec!["Allow".to_owned(), "Reject".to_owned()],
        });
        assert_eq!(
            conversation.asked_about().as_deref(),
            Some(
                "The chat's agent asks the human's permission for a tool call: Edit x.txt\n\n\
                 Its change to x.txt:\n-old\n+new\n\n\nThe human chooses one of: Allow, Reject."
            )
        );
        assert!(!conversation.gathering());

        conversation.gather(vec!["summary".to_owned(), "tests".to_owned()]);
        assert!(conversation.gathering());
        conversation.gathered(
            0,
            Gathered::Said("## Only x.txt changes\n\nNothing else is touched.".to_owned()),
        );
        assert!(conversation.gathering());
        conversation.gathered(
            1,
            Gathered::Printed(Ran {
                output: "2 failed".to_owned(),
                ended: Ended::Exited(1),
            }),
        );
        // Each came, so the human is asked, the headlines first.
        assert!(!conversation.gathering());
        assert_eq!(conversation.headlines(), ["Only x.txt changes", "2 failed"]);
        let Some(Entry::Permission { gathered, .. }) = conversation.entries().last() else {
            panic!("{:?}", conversation.entries());
        };
        assert_eq!(gathered[0].state, CheckState::Passed);
        assert_eq!(gathered[1].state, CheckState::Failed);
        assert_eq!(
            gathered[1].said.as_deref(),
            Some("2 failed\n\n(exit code 1)")
        );
        assert_eq!(conversation.state(), State::Asking);
        assert!(conversation.choose(0).is_some());
    }

    #[test]
    fn asks_without_what_has_not_come_once_gathering_stops() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        conversation.apply(AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("touch x.txt".to_owned()),
            content: None,
            options: vec!["Allow".to_owned()],
        });
        conversation.gather(vec!["slow".to_owned()]);
        conversation.stop_gathering("It took too long.");

        assert!(!conversation.gathering());
        assert_eq!(conversation.headlines(), Vec::<&str>::new());
        let Some(Entry::Permission { gathered, .. }) = conversation.entries().last() else {
            panic!("{:?}", conversation.entries());
        };
        assert_eq!(gathered[0].state, CheckState::Broken);
        assert_eq!(gathered[0].said.as_deref(), Some("It took too long."));
        // What comes after is let be.
        conversation.gathered(0, Gathered::Said("Late".to_owned()));
        assert_eq!(conversation.headlines(), Vec::<&str>::new());
    }

    #[test]
    fn answers_a_permission_request_with_the_option_chosen() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        assert_eq!(conversation.asked(), None);
        conversation.apply(AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("touch x.txt".to_owned()),
            content: None,
            options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
        });

        assert_eq!(
            conversation.asked(),
            Some(&["Allow once".to_owned(), "Reject once".to_owned()][..])
        );
        assert_eq!(conversation.submit("2"), None);
        assert_eq!(conversation.choose(2), None);
        assert_eq!(
            conversation.choose(1),
            Some(Command::Answer(PermissionAnswer::Select(1)))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.asked(), None);
    }

    #[test]
    fn a_permission_request_cox_answered_waits_on_nobody() {
        let mut conversation = Conversation::new();
        conversation.submit("edit");
        conversation.apply(tool_call(Some(vec![text("the change")]), None));
        conversation.apply(AgentEvent::PermissionDecided {
            id: "call-1".to_owned(),
            title: Some("Edit x.txt".to_owned()),
            content: None,
            options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
            chosen: 1,
        });

        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.asked(), None);
        assert_eq!(conversation.asking_for(), None);
        assert_eq!(conversation.choose(0), None);
        // The request shows what the call showed.
        assert_eq!(
            shown(&conversation),
            [("call", vec![]), ("permission", vec![text("the change")])]
        );
        assert!(matches!(
            conversation.entries().last(),
            Some(Entry::Permission {
                chosen: Some(1),
                by_setup: true,
                ..
            })
        ));
    }

    #[test]
    fn stops_a_turn_and_takes_the_next_line_once_it_has_ended() {
        let mut conversation = Conversation::new();
        assert_eq!(conversation.cancel(), None);
        conversation.submit("wait");
        conversation.apply(AgentEvent::ReplyText("waiting".to_owned()));

        assert_eq!(conversation.cancel(), Some(Command::Cancel));
        assert_eq!(conversation.state(), State::Stopping);
        assert_eq!(conversation.cancel(), None);
        assert_eq!(conversation.submit("again"), None);

        conversation.apply(AgentEvent::TurnEnded(StopReason::Cancelled));
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.submit("again"),
            Some(Command::Prompt("again".to_owned().into()))
        );
        assert_eq!(
            conversation.entries(),
            [
                Entry::Human("wait".to_owned()),
                Entry::Reply("waiting".to_owned()),
                Entry::Note("Stopped.".to_owned()),
                Entry::Human("again".to_owned()),
            ]
        );
    }

    #[test]
    fn stops_a_turn_that_waits_for_permission() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        let request = || AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("touch x.txt".to_owned()),
            content: None,
            options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
        };
        conversation.apply(request());

        assert_eq!(conversation.cancel(), Some(Command::Cancel));
        assert_eq!(conversation.choose(0), None);

        // A request the agent made before it heard it is to stop.
        conversation.apply(request());
        assert_eq!(conversation.state(), State::Stopping);
        assert_eq!(conversation.asked(), None);
        assert_eq!(conversation.choose(0), None);
    }

    #[test]
    fn tells_which_entries_changed_to_keep_them() {
        let mut conversation = Conversation::noting("Instructions: ~/cox/COX.md".to_owned());
        assert_eq!(conversation.take_changed(), [0]);
        assert_eq!(conversation.take_changed(), Vec::<usize>::new());

        conversation.submit("hello");
        conversation.apply(AgentEvent::ReplyText("echo".to_owned()));
        assert_eq!(conversation.take_changed(), [1, 2]);
        conversation.apply(AgentEvent::ReplyText(": hello".to_owned()));
        assert_eq!(conversation.take_changed(), [2]);

        conversation.apply(tool_call(Some(vec![text("Run the checks")]), None));
        conversation.apply(AgentEvent::ReplyText("Running".to_owned()));
        assert_eq!(conversation.take_changed(), [3, 4]);
        // The request shows what the call showed, which the call then does
        // not.
        conversation.apply(permission(None));
        assert_eq!(conversation.take_changed(), [3, 5]);
        conversation.choose(0);
        assert_eq!(conversation.take_changed(), [5]);
        conversation.apply(tool_call(None, Some(ToolCallResult::Completed)));
        conversation.apply(AgentEvent::TurnFailed("lost".to_owned()));
        assert_eq!(conversation.take_changed(), [3, 6]);
    }

    /// How the goal's command went: it exited with `code`, printing
    /// `output`.
    #[expect(clippy::unnecessary_wraps, reason = "what `checked` takes")]
    fn ran(code: i32, output: &str) -> Result<Ran, String> {
        Ok(Ran {
            output: output.to_owned(),
            ended: Ended::Exited(code),
        })
    }

    /// The triggers `ci`, which runs `cargo test`, and `simplify`, which
    /// asks its agent, due as the turn ends.
    fn ci_and_simplify() -> Vec<Due> {
        vec![
            Due {
                name: "ci".to_owned(),
                file: "/work/triggers/ci.md".into(),
                does: Does::Run("cargo test".to_owned()),
            },
            Due {
                name: "simplify".to_owned(),
                file: "/home/me/cox/triggers/simplify.md".into(),
                does: Does::Ask("Say what could be simpler.".to_owned()),
            },
        ]
    }

    /// How the line of chips of what ran as the turn ended stands, by
    /// name.
    fn marks(conversation: &Conversation) -> Vec<(String, CheckState)> {
        conversation
            .entries()
            .iter()
            .rev()
            .find_map(|entry| {
                Some(
                    entry
                        .checks()?
                        .iter()
                        .map(|check| (check.name.clone(), check.state))
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_trigger_by_this_tells_the_agent_once_for_a_line() {
        let commit = || Due {
            name: "commit".to_owned(),
            file: "/p/triggers/commit.md".into(),
            does: Does::Tell("Write the commit message.".to_owned()),
        };
        let mut conversation = Conversation::new();
        conversation.submit("fix");
        conversation.set_due(vec![commit(), ci_and_simplify().remove(0)]);
        take_turn(&mut conversation, true);
        assert_eq!(conversation.telling(), Some("Write the commit message."));
        // It tells once the rest went.
        assert_eq!(conversation.tell(), None);
        assert_eq!(conversation.checking(), Some("cargo test"));
        assert_eq!(
            conversation.checked(ran(0, "ok")),
            Some(Command::Prompt(
                "Write the commit message.".to_owned().into()
            ))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Cox("Write the commit message.".to_owned()))
        );
        // The turn it starts does not tell it again.
        conversation.set_due(vec![commit()]);
        take_turn(&mut conversation, true);
        assert_eq!(conversation.state(), State::Idle);
        // The next line does.
        conversation.submit("and the docs");
        conversation.set_due(vec![commit()]);
        take_turn(&mut conversation, true);
        assert_eq!(conversation.telling(), Some("Write the commit message."));
    }

    #[test]
    fn the_goal_met_again_on_a_line_runs_its_triggers_once() {
        let commit = Due {
            name: "commit".to_owned(),
            file: "/p/triggers/commit.md".into(),
            does: Does::Tell("Write the commit message.".to_owned()),
        };
        let happened = Happened {
            on: "goal.met".to_owned(),
            about: "The chat's goal is met.".to_owned(),
        };
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("fix");
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert!(conversation.take_goal_met());
        conversation.run_event(happened.clone(), vec![commit.clone()]);
        assert!(conversation.run_waiting());
        assert_eq!(
            conversation.tell(),
            Some(Command::Prompt(
                "Write the commit message.".to_owned().into()
            ))
        );
        // The turn it starts meets the goal again, which runs nothing.
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert!(!conversation.take_goal_met());
        conversation.run_event(happened, vec![commit]);
        assert!(!conversation.run_waiting());
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn what_happened_runs_its_triggers_once_the_chat_waits_untold() {
        let happened = || Happened {
            on: "goal.met".to_owned(),
            about: "The chat's goal is met.".to_owned(),
        };
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("fix");
        take_turn(&mut conversation, true);
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.state(), State::Idle);
        assert!(conversation.take_goal_met());
        assert!(!conversation.take_goal_met());

        conversation.run_event(happened(), ci_and_simplify());
        assert!(conversation.run_waiting());
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.event(), Some(&happened()));
        assert!(matches!(
            conversation.entries().last(),
            Some(Entry::Event { on, .. }) if on == "goal.met"
        ));
        // What a trigger finds stays on its chip, and the rest still run.
        assert_eq!(conversation.checked(ran(1, "a test failed")), None);
        assert_eq!(
            conversation.asking().map(|due| due.name.as_str()),
            Some("simplify")
        );
        assert_eq!(conversation.found(Ok("FAIL: it repeats".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(conversation.event(), None);
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Failed),
                ("simplify".to_owned(), CheckState::Failed),
            ]
        );

        // While the agent is at work, they wait for its turn to end.
        conversation.submit("more");
        conversation.run_event(happened(), ci_and_simplify());
        assert!(!conversation.run_waiting());
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert_eq!(conversation.state(), State::Idle);
        assert!(conversation.run_waiting());
        assert_eq!(conversation.state(), State::Checking);
    }

    #[test]
    fn the_triggers_run_after_the_goal_until_one_fails() {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), Some("make"));
        assert_eq!(conversation.running_check(), Some("goal"));

        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), Some("cargo test"));
        assert_eq!(
            conversation.trigger().map(|due| due.name.as_str()),
            Some("ci")
        );
        assert_eq!(conversation.running_check(), Some("ci"));
        // The goal is met, but the human is asked to keep what was learned
        // only once the triggers pass too.
        assert!(
            !conversation
                .entries()
                .iter()
                .any(|entry| matches!(entry, Entry::Keep { .. }))
        );

        assert_eq!(conversation.checked(ran(0, "test result: ok")), None);
        assert_eq!(conversation.checking(), None);
        assert_eq!(
            conversation.asking().map(|due| due.name.as_str()),
            Some("simplify")
        );
        // Its agent is told what the turn said, not what ran after.
        assert_eq!(conversation.turn_told(), Some("Done."));

        let said = "Read the diff.\nFAIL: parse repeats read";
        let told = "The trigger `simplify` found something: parse repeats read \
                    Act on what it said; the triggers run again as your turn ends. \
                    It said:\n\nRead the diff.\nFAIL: parse repeats read";
        assert_eq!(
            conversation.found(Ok(said.to_owned())),
            Some(Command::Prompt(told.to_owned().into()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(
            marks(&conversation),
            [
                ("goal".to_owned(), CheckState::Passed),
                ("ci".to_owned(), CheckState::Passed),
                ("simplify".to_owned(), CheckState::Failed),
            ]
        );

        // The next turn runs them all again, and once they pass, the human
        // is asked whether to keep what the chat learned.
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        conversation.checked(ran(0, "ok"));
        assert_eq!(conversation.found(Ok("Fine.\nPASS".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert!(matches!(
            conversation.entries().last(),
            Some(Entry::Keep { .. })
        ));
    }

    #[test]
    fn a_chat_with_no_goal_runs_its_triggers() {
        let mut conversation = Conversation::new();
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);
        assert_eq!(conversation.checking(), Some("cargo test"));
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Running),
                ("simplify".to_owned(), CheckState::Waiting),
            ]
        );

        let told = "The trigger `ci` did not pass: `cargo test` exit code 101. Fix what it \
                    found; the triggers run again as your turn ends. It printed, at the \
                    end:\n\n1 failed";
        assert_eq!(
            conversation.checked(ran(101, "1 failed")),
            Some(Command::Prompt(told.to_owned().into()))
        );
        // The trigger after the one that failed did not run.
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Failed),
                ("simplify".to_owned(), CheckState::Waiting),
            ]
        );

        // A turn with nothing due ends as it did.
        take_turn(&mut conversation, true);
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn a_trigger_that_cannot_run_waits_for_the_human() {
        let mut conversation = Conversation::new();
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, ""));
        assert_eq!(conversation.found(Err("its agent closed".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some("The trigger `simplify` could not run: its agent closed")
        );
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Passed),
                ("simplify".to_owned(), CheckState::Broken),
            ]
        );
    }

    #[test]
    fn stopping_while_a_trigger_runs_gives_the_rest_up() {
        let mut conversation = Conversation::new();
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);

        assert_eq!(conversation.cancel(), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Waiting),
                ("simplify".to_owned(), CheckState::Waiting),
            ]
        );
    }

    /// A conversation working until `make` passes, whose agent takes a
    /// turn on "fix", with a tool call when `calling`, which ends.
    fn toward_make(calling: bool) -> Conversation {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        assert_eq!(
            conversation.entries(),
            [Entry::Note("Goal: until `make` passes.".to_owned())]
        );
        conversation.submit("fix");
        take_turn(&mut conversation, calling);
        conversation
    }

    /// The agent takes a turn, with a tool call when `calling`, and ends
    /// it as it does.
    fn take_turn(conversation: &mut Conversation, calling: bool) {
        if calling {
            let id = format!("call-{}", conversation.entries().len());
            conversation.apply(AgentEvent::ToolCall {
                id,
                title: Some("Edit x.txt".to_owned()),
                content: None,
                result: Some(ToolCallResult::Completed),
            });
        }
        conversation.apply(AgentEvent::ReplyText("Done.".to_owned()));
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));
    }

    #[test]
    fn the_human_is_asked_to_keep_in_the_wiki_once_for_a_line_of_theirs() {
        let mut conversation = toward_make(true);
        conversation.checked(ran(0, "ok"));
        let asked = conversation.entries().len() - 1;
        assert_eq!(conversation.answer_keep(asked, true), Some(None));
        // The turn that keeps it meets the goal again, which asks nothing.
        conversation.told("Keep it.");
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("Goal met: `make` passed.".to_owned()))
        );
        // The human's next line does.
        conversation.submit("more");
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert!(matches!(
            conversation.entries().last(),
            Some(Entry::Keep { kept: None, .. })
        ));
    }

    #[test]
    fn a_chat_whose_agent_cannot_keep_in_the_wiki_is_not_asked_to() {
        let mut conversation = toward_make(true).with_keeping(false);
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("Goal met: `make` passed.".to_owned()))
        );
    }

    #[test]
    fn works_toward_its_goal_until_the_command_passes() {
        let mut conversation = toward_make(true);
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), Some("make"));
        assert_eq!(conversation.submit("again"), None);
        assert_eq!(conversation.goal().map(Goal::turns), Some(1));
        conversation.take_changed();

        let prompt = "Goal not met: `make` exit code 2. Go on until it passes. \
                      It printed, at the end:\n\nerror: x is not defined";
        assert_eq!(
            conversation.checked(ran(2, "error: x is not defined\n\n")),
            Some(Command::Prompt(prompt.to_owned().into()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.checking(), None);
        assert_eq!(conversation.goal().map(Goal::turns), Some(2));
        // The goal's chip shows that it failed, with what it printed.
        assert_eq!(conversation.take_changed(), [4, 5]);
        assert_eq!(
            conversation.entries()[4],
            Entry::Checks(vec![Check {
                name: "goal".to_owned(),
                state: CheckState::Failed,
                said: Some("error: x is not defined".to_owned()),
            }])
        );
        assert_eq!(conversation.entries()[5], Entry::Cox(prompt.to_owned()));

        take_turn(&mut conversation, true);
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.state(), State::Idle);
        // The human is asked whether to keep what the chat learned.
        let entries = conversation.entries();
        assert_eq!(
            entries[entries.len() - 2..],
            [
                Entry::Note("Goal met: `make` passed.".to_owned()),
                Entry::Keep {
                    why: GOAL_MET.to_owned(),
                    chat: None,
                    kept: None,
                },
            ]
        );
        let asked = entries.len() - 1;
        assert_eq!(conversation.answer_keep(asked, false), Some(None));
        assert_eq!(conversation.answer_keep(asked, true), None);
        assert_eq!(
            conversation.entries()[asked],
            Entry::Keep {
                why: GOAL_MET.to_owned(),
                chat: None,
                kept: Some(false),
            }
        );
        // The goal stays, for the human's next line.
        assert_eq!(conversation.goal().and_then(Goal::command), Some("make"));
        conversation.submit("more");
        assert_eq!(conversation.goal().map(Goal::turns), Some(1));
    }

    #[test]
    fn each_turn_toward_the_goal_plans_anew() {
        let mut conversation = toward_make(true);
        let first = vec![step("Fix", StepState::Completed)];
        let then = vec![step("Fix more", StepState::InProgress)];
        conversation.checked(ran(1, ""));
        conversation.apply(AgentEvent::Plan(then.clone()));
        assert_eq!(conversation.last_plan(), Some(&then[..]));

        conversation.apply(AgentEvent::Plan(first.clone()));
        let plans: Vec<_> = conversation
            .entries()
            .iter()
            .filter(|entry| matches!(entry, Entry::Plan(_)))
            .collect();
        assert_eq!(plans, [&Entry::Plan(first)]);
    }

    #[test]
    fn tells_the_agent_the_end_of_a_long_output() {
        let mut conversation = toward_make(true);
        let output = format!("{}{}", "x".repeat(100), "y".repeat(OUTPUT_TAIL));

        let Some(Command::Prompt(Prompt { text: prompt, .. })) =
            conversation.checked(ran(1, &output))
        else {
            panic!("the agent goes on");
        };
        assert!(prompt.ends_with(&format!("[…]{}", "y".repeat(OUTPUT_TAIL))));
        assert!(!prompt.contains("xy"), "{prompt}");
    }

    #[test]
    fn stops_toward_the_goal_when_the_agent_only_talks() {
        let mut conversation = toward_make(false);
        assert!(conversation.checked(ran(1, "")).is_some());

        take_turn(&mut conversation, false);
        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some(
                "Goal not met: `make` exit code 1. The agent made no tool call in its last 2 turns, so it may wait for you."
            )
        );
    }

    #[test]
    fn stops_after_as_many_turns_toward_the_goal_as_one_line_gets() {
        let mut conversation = toward_make(true);
        for turn in 1..GOAL_TURNS {
            assert_eq!(conversation.goal().map(Goal::turns), Some(turn));
            assert!(conversation.checked(ran(1, "")).is_some());
            take_turn(&mut conversation, true);
        }

        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some(
                "Goal not met: `make` exit code 1. The chat took 10 turns toward it, as many as one line gets; send a line to go on."
            )
        );
        // The human's line starts the count again.
        conversation.submit("go on");
        take_turn(&mut conversation, true);
        assert!(conversation.checked(ran(1, "")).is_some());
    }

    #[test]
    fn a_turn_that_is_stopped_or_fails_is_not_checked() {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("wait");
        conversation.cancel();
        conversation.apply(AgentEvent::TurnEnded(StopReason::Cancelled));
        assert_eq!(conversation.state(), State::Idle);

        conversation.submit("fail");
        conversation.apply(AgentEvent::TurnFailed("lost".to_owned()));
        assert_eq!(conversation.state(), State::Idle);

        conversation.submit("long");
        conversation.apply(AgentEvent::TurnEnded(StopReason::MaxTokens));
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn stopping_while_the_goal_is_checked_gives_the_check_up() {
        let mut conversation = toward_make(true);

        assert_eq!(conversation.cancel(), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("Stopped.".to_owned()))
        );
        // What the command answers once it is stopped changes nothing.
        let entries = conversation.entries().len();
        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(conversation.entries().len(), entries);
        assert_eq!(
            conversation.submit("again"),
            Some(Command::Prompt("again".to_owned().into()))
        );
    }

    #[test]
    fn changing_the_goal_while_it_is_checked_gives_the_check_up() {
        let mut conversation = toward_make(true);
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        assert_eq!(conversation.state(), State::Checking);

        conversation.set_goal(None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(conversation.goal(), None);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("The goal is cleared.".to_owned()))
        );
        assert_eq!(conversation.checked(ran(1, "")), None);
        // With no goal, a turn ends as it did.
        conversation.submit("fix");
        take_turn(&mut conversation, true);
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn a_goal_that_cannot_be_checked_waits_for_the_human() {
        let mut conversation = toward_make(true);
        assert_eq!(
            conversation.checked(Err("the chat has no terminal to run it in".to_owned())),
            None
        );
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some("The goal could not be checked: the chat has no terminal to run it in")
        );

        conversation.submit("fix");
        take_turn(&mut conversation, true);
        let closed = Ok(Ran {
            output: String::new(),
            ended: Ended::Closed,
        });
        assert_eq!(conversation.checked(closed), None);
        assert_eq!(
            conversation.last_said(),
            Some("The goal was not checked: the terminal of `make` was closed.")
        );
    }

    /// A conversation working until the greeting names the user, as an
    /// agent judges, whose agent takes a turn on "fix", with a tool call
    /// when `calling`, which ends.
    fn toward_words(calling: bool) -> Conversation {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Words(
            "The greeting names the user.".to_owned(),
        )));
        assert_eq!(
            conversation.entries(),
            [Entry::Note("Goal: The greeting names the user.".to_owned())]
        );
        conversation.submit("fix");
        take_turn(&mut conversation, calling);
        conversation
    }

    #[test]
    fn a_goal_the_judge_cannot_judge_as_written_waits_for_the_human() {
        let mut conversation = toward_words(true);
        conversation.take_changed();

        let said = "It asks for Slack.\nBLOCKED: I cannot see whether Slack got a message.";
        assert_eq!(conversation.judged(Ok(said.to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert!(conversation.entries().iter().any(|entry| matches!(entry,
        Entry::Note(note) if note.starts_with(
            "The goal cannot be judged as written: I cannot see whether Slack got a message."
        ))));
        // The agent is not told to go on, and the goal stays for the human
        // to change.
        assert!(
            !conversation
                .entries()
                .iter()
                .any(|entry| matches!(entry, Entry::Cox(_)))
        );
        assert!(conversation.goal().is_some());
    }

    #[test]
    fn works_toward_a_goal_in_words_until_the_judge_says_it_is_met() {
        let mut conversation = toward_words(true);
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), None);
        assert_eq!(conversation.judging(), Some("The greeting names the user."));
        assert_eq!(conversation.submit("again"), None);
        conversation.take_changed();

        let said = "I read src/main.rs.\nFAIL: the greeting still says hello";
        let prompt = "Goal not met, as judged: the greeting still says hello \
                      Go on until it is met. The judge said:\n\n\
                      I read src/main.rs.\nFAIL: the greeting still says hello";
        assert_eq!(
            conversation.judged(Ok(said.to_owned())),
            Some(Command::Prompt(prompt.to_owned().into()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.judging(), None);
        assert_eq!(conversation.goal().map(Goal::turns), Some(2));
        assert_eq!(conversation.take_changed(), [4, 5]);
        assert_eq!(conversation.entries()[5], Entry::Cox(prompt.to_owned()));

        take_turn(&mut conversation, true);
        assert_eq!(
            conversation.judged(Ok("I read it again.\n\n**PASS**\n".to_owned())),
            None
        );
        assert_eq!(conversation.state(), State::Idle);
        let entries = conversation.entries();
        assert_eq!(
            entries[entries.len() - 2],
            Entry::Note("Goal met, as judged.".to_owned())
        );
        assert!(matches!(entries.last(), Some(Entry::Keep { .. })));
        // The goal stays, for the human's next line.
        assert_eq!(
            conversation.goal().and_then(Goal::words),
            Some("The greeting names the user.")
        );
        conversation.submit("more");
        assert_eq!(conversation.goal().map(Goal::turns), Some(1));
    }

    #[test]
    fn the_command_the_judge_finds_is_offered_once() {
        let mut conversation = toward_words(true);
        let said = "I ran it.\nCOMMAND: cargo test greeting\nFAIL: the test fails";
        assert!(conversation.judged(Ok(said.to_owned())).is_some());
        let offered = conversation
            .entries()
            .iter()
            .position(|entry| matches!(entry, Entry::Proposal { .. }))
            .expect("the command is offered");
        // Offered ahead of the next turn it gives.
        assert!(matches!(conversation.entries()[offered + 1], Entry::Cox(_)));
        assert_eq!(
            conversation.proposed(offered),
            Some(goal::Goal::Command("cargo test greeting".to_owned()))
        );

        take_turn(&mut conversation, true);
        conversation.judged(Ok("COMMAND: cargo test greeting\nPASS".to_owned()));
        let offers = conversation
            .entries()
            .iter()
            .filter(|entry| matches!(entry, Entry::Proposal { .. }))
            .count();
        assert_eq!(offers, 1);

        // Set, it is the goal, which a command checks from then on.
        let command = conversation.answer_proposal(offered, Answer::Set).unwrap();
        conversation.set_goal(Some(command));
        assert_eq!(
            conversation.goal().and_then(Goal::command),
            Some("cargo test greeting")
        );
    }

    #[test]
    fn the_judge_may_say_why_the_goal_is_met() {
        let mut conversation = toward_words(true);
        assert_eq!(
            conversation.judged(Ok("PASS: main.rs greets by name".to_owned())),
            None
        );
        assert_eq!(
            conversation.last_said(),
            Some("Goal met, as judged: main.rs greets by name")
        );
    }

    #[test]
    fn a_judge_that_gives_no_verdict_has_not_met_the_goal() {
        let mut conversation = toward_words(true);
        let prompt = conversation
            .judged(Ok("I am not sure what you mean.".to_owned()))
            .expect("the chat goes on");
        let Command::Prompt(Prompt { text: prompt, .. }) = prompt else {
            panic!("a prompt");
        };
        assert!(
            prompt
                .starts_with("Goal not met, as judged: it gave no verdict Go on until it is met."),
            "{prompt}"
        );
        assert!(prompt.ends_with("I am not sure what you mean."), "{prompt}");
    }

    #[test]
    fn a_goal_that_cannot_be_judged_waits_for_the_human() {
        let mut conversation = toward_words(true);
        assert_eq!(
            conversation.judged(Err("the chat has no agent to judge it".to_owned())),
            None
        );
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some("The goal could not be judged: the chat has no agent to judge it")
        );
        // What a judge answers once the chat waits changes nothing.
        let entries = conversation.entries().len();
        assert_eq!(conversation.judged(Ok("PASS".to_owned())), None);
        assert_eq!(conversation.entries().len(), entries);
    }

    #[test]
    fn a_goal_in_words_stops_when_the_agent_makes_no_tool_call() {
        let mut conversation = toward_words(false);
        assert_eq!(
            conversation.judged(Ok("FAIL: not yet".to_owned())),
            Some(Command::Prompt(
                "Goal not met, as judged: not yet Go on until it is met. The judge said:\n\nFAIL: not yet"
                    .to_owned()
                    .into()
            ))
        );
        take_turn(&mut conversation, false);
        assert_eq!(conversation.judged(Ok("FAIL: not yet".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some(
                "Goal not met, as judged: not yet The agent made no tool call in its last 2 turns, so it may wait for you."
            )
        );
    }

    #[test]
    fn a_goal_proposed_waits_for_the_human_to_answer_it() {
        let mut conversation = Conversation::new();
        conversation.submit("greet the user");
        let make = goal::Goal::Command("make".to_owned());
        conversation.propose(&make, Some("make builds it".to_owned()));
        conversation.propose(
            &goal::Goal::Words("It greets.".to_owned()),
            Some(" ".to_owned()),
        );
        assert_eq!(
            conversation.entries()[1..],
            [
                Entry::Proposal {
                    goal: "$ make".to_owned(),
                    why: Some("make builds it".to_owned()),
                    answer: None,
                },
                Entry::Proposal {
                    goal: "It greets.".to_owned(),
                    why: None,
                    answer: None,
                },
            ]
        );
        // Proposing sets nothing, nor holds up the turn.
        assert_eq!(conversation.goal(), None);
        assert_eq!(conversation.state(), State::Busy);
        conversation.take_changed();

        assert_eq!(conversation.proposed(1), Some(make.clone()));
        assert_eq!(conversation.answer_proposal(1, Answer::Set), Some(make));
        assert_eq!(conversation.take_changed(), [1]);
        // Answered, it is answered for good.
        assert_eq!(conversation.proposed(1), None);
        assert_eq!(conversation.answer_proposal(1, Answer::Dismissed), None);
        assert_eq!(conversation.answer_proposal(0, Answer::Set), None);
        assert!(matches!(
            conversation.entries()[1],
            Entry::Proposal {
                answer: Some(Answer::Set),
                ..
            }
        ));
    }

    #[test]
    fn a_closed_conversation_keeps_the_goal_it_had() {
        let mut conversation = Conversation::closed(vec![Entry::Human("fix".to_owned())])
            .with_goal(Some(goal::Goal::Command("make".to_owned())));
        assert_eq!(conversation.goal().and_then(Goal::command), Some("make"));

        conversation.reopen();
        conversation.submit("again");
        take_turn(&mut conversation, true);
        assert_eq!(conversation.checking(), Some("make"));
    }

    #[test]
    fn a_closed_chats_conversation_is_for_reading() {
        let mut conversation = Conversation::closed(vec![
            Entry::Human("hello".to_owned()),
            Entry::Reply("echo: hello".to_owned()),
        ]);

        assert_eq!(conversation.state(), State::Closed);
        assert_eq!(conversation.submit("again"), None);
        assert_eq!(conversation.cancel(), None);
        assert_eq!(conversation.entries().len(), 2);
        assert_eq!(conversation.take_changed(), Vec::<usize>::new());
    }
}
