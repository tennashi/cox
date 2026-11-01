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

use std::collections::BTreeSet;
use std::path::Path;

use cox_acp::{PermissionAnswer, StopReason};
pub use cox_core::entry::{Answer, Content, Entry, Step, StepState, ToolCallResult};
use cox_core::goal;

use crate::agent::mcp::{Ended, Ran};
use crate::agent::{AgentEvent, Command, Opened};
use crate::chat::judge::{self, Verdict};

/// Whose move it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// judge is asked, to see whether the goal is met.
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

/// What a chat works until, and how far it has come.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// How `goal` reads, after "Goal:".
#[must_use]
pub fn line(goal: &goal::Goal) -> String {
    match goal {
        goal::Goal::Command(command) => format!("until `{command}` passes."),
        goal::Goal::Words(words) => words.clone(),
    }
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
        }
    }

    /// The conversation, of a chat that works toward `goal`, if the human
    /// set one.
    #[must_use]
    pub fn with_goal(mut self, goal: Option<goal::Goal>) -> Self {
        self.goal = goal.map(Goal::new);
        self
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

    fn push(&mut self, entry: Entry) {
        self.changed.insert(self.entries.len());
        self.entries.push(entry);
    }

    /// Adds `note`, a note from Cox.
    pub fn note(&mut self, note: String) {
        self.push(Entry::Note(note));
    }

    /// Takes what the human entered, and returns the prompt to send the
    /// agent, when it waits for one.
    pub fn submit(&mut self, input: &str) -> Option<Command> {
        self.start_turn(input, Entry::Human)
    }

    /// Takes `text`, a line Cox sends in the human's place, such as the
    /// first line of a chat opened from a chat, and returns the prompt to
    /// send the agent, when it waits for one.
    pub fn told(&mut self, text: &str) -> Option<Command> {
        self.start_turn(text, Entry::Cox)
    }

    /// Starts a turn on `text`, shown as `entry` makes it, when the agent
    /// waits for a line.
    fn start_turn(&mut self, text: &str, entry: fn(String) -> Entry) -> Option<Command> {
        if self.state != State::Idle || text.trim().is_empty() {
            return None;
        }
        self.push(entry(text.to_owned()));
        self.state = State::Busy;
        if let Some(goal) = &mut self.goal {
            goal.turns = 1;
        }
        Some(Command::Prompt(text.to_owned()))
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
            self.state = State::Idle;
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

    /// The command that runs to see whether the goal is met, while one
    /// does.
    #[must_use]
    pub fn checking(&self) -> Option<&str> {
        match (self.state, &self.goal) {
            (State::Checking, Some(goal)) => goal.command(),
            _ => None,
        }
    }

    /// The words of the goal the judge is asked about, while it is.
    #[must_use]
    pub fn judging(&self) -> Option<&str> {
        match (self.state, &self.goal) {
            (State::Checking, Some(goal)) => goal.words(),
            _ => None,
        }
    }

    /// Takes how the goal's command went, `ran`, and returns the next
    /// turn to send the agent, when the goal is not met and the chat
    /// goes on toward it. Otherwise the chat notes how it stands and
    /// waits for the human: the goal is met, or the command could not
    /// run, or the chat has taken as many turns as one line gets, or the
    /// agent made no tool call in its last turns.
    pub fn checked(&mut self, ran: Result<Ran, String>) -> Option<Command> {
        let command = self.checking()?.to_owned();
        self.state = State::Idle;
        let ran = match ran {
            Ok(ran) => ran,
            Err(error) => {
                self.push(Entry::Note(format!(
                    "The goal could not be checked: {error}"
                )));
                return None;
            }
        };
        let not_met = match ran.ended {
            Ended::Exited(0) => {
                self.push(Entry::Note(format!("Goal met: `{command}` passed.")));
                return None;
            }
            Ended::Closed => {
                self.push(Entry::Note(format!(
                    "The goal was not checked: the terminal of `{command}` was closed."
                )));
                return None;
            }
            ended => format!("Goal not met: `{command}` {ended}."),
        };
        let output = tail(&ran.output, OUTPUT_TAIL);
        let told = if output.is_empty() {
            "Go on until it passes. It printed nothing.".to_owned()
        } else {
            format!("Go on until it passes. It printed, at the end:\n\n{output}")
        };
        self.go_on(&not_met, &told)
    }

    /// Takes what the judge of the goal in words said, `judged`, or why
    /// it could not judge, and returns the next turn to send the agent,
    /// when the goal is not met and the chat goes on toward it; otherwise
    /// the chat notes how it stands and waits for the human, as after
    /// [`Conversation::checked`].
    pub fn judged(&mut self, judged: Result<String, String>) -> Option<Command> {
        self.judging()?;
        self.state = State::Idle;
        let said = match judged {
            Ok(said) => said,
            Err(error) => {
                self.push(Entry::Note(format!(
                    "The goal could not be judged: {error}"
                )));
                return None;
            }
        };
        self.offer_command(&said);
        let why = match Verdict::of(&said) {
            Verdict::Met(said) if said.is_empty() => {
                self.push(Entry::Note("Goal met, as judged.".to_owned()));
                return None;
            }
            Verdict::Met(said) => {
                self.push(Entry::Note(format!("Goal met, as judged: {said}")));
                return None;
            }
            Verdict::NotMet(why) => why,
        };
        let not_met = format!("Goal not met, as judged: {why}");
        let told = format!(
            "Go on until it is met. The judge said:\n\n{}",
            tail(&said, OUTPUT_TAIL)
        );
        self.go_on(&not_met, &told)
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
        let turns = self.goal.as_ref().map_or(0, Goal::turns);
        if turns >= GOAL_TURNS {
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
        if let Some(goal) = &mut self.goal {
            goal.turns += 1;
        }
        self.push(Entry::Cox(prompt.clone()));
        self.state = State::Busy;
        Some(Command::Prompt(prompt))
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
        Some(Command::Answer(PermissionAnswer::Select(index)))
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
            // The goal's command stops; there is no turn to stop.
            State::Checking => {
                self.push(Entry::Note("Stopped.".to_owned()));
                self.state = State::Idle;
                None
            }
            State::Idle | State::Stopping | State::Closed => None,
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

    /// What the agent asks permission for, while it waits on the human.
    #[must_use]
    pub fn asking_for(&self) -> Option<&str> {
        match (self.state, self.entries.last()) {
            (State::Asking, Some(Entry::Permission { title, .. })) => Some(title),
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
        });
    }

    /// Shows what the agent side sent.
    pub fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Titled(title) => self.title = Some(title),
            AgentEvent::Opened { how, .. } => match how {
                Opened::New => {}
                Opened::Resumed => {
                    self.push(Entry::Note(
                        "Resumed: the agent goes on from here.".to_owned(),
                    ));
                }
                Opened::Retold {
                    instructions,
                    context,
                } => {
                    self.push(Entry::Note(
                        "The agent could not resume its session: a new one is told the conversation so far."
                            .to_owned(),
                    ));
                    if let Some(path) = instructions {
                        self.push(Entry::Note(instructions_note(&path)));
                    }
                    if let Some(context) = context {
                        self.push(Entry::Note(context_note(&context)));
                    }
                }
            },
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
            AgentEvent::TurnEnded(reason) => {
                if let Some(note) = ended(reason) {
                    self.push(Entry::Note(note.to_owned()));
                }
                // A turn the agent ended as it does, having done what it
                // was asked, is when the goal is looked at.
                let check = reason == StopReason::EndTurn
                    && self.state == State::Busy
                    && self.goal.is_some();
                self.state = if check { State::Checking } else { State::Idle };
            }
            AgentEvent::TurnFailed(error) => {
                self.push(Entry::Note(format!("The turn failed: {error}")));
                self.state = State::Idle;
            }
            AgentEvent::Closed(error) => {
                let note = match error {
                    Some(error) => format!("The agent closed: {error}"),
                    None => "The agent closed.".to_owned(),
                };
                self.push(Entry::Note(note));
                self.state = State::Closed;
            }
        }
    }
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

/// The note that tells the human a chat works in `folder`, a trial's,
/// apart from the project.
#[must_use]
pub fn trial_note(folder: &Path) -> String {
    format!(
        "Trial: works in {}, apart from the project.",
        folder.display()
    )
}

/// Whether `entry` starts a turn: a line sent to the agent, by the human
/// or by Cox in their place.
fn is_turn_start(entry: &Entry) -> bool {
    matches!(entry, Entry::Human(_) | Entry::Cox(_))
}

/// The last `length` characters of `text`, at most, less blank lines at
/// its end, with a mark of what is left out.
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
    use super::*;

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
            Some(Command::Prompt("again".to_owned()))
        );
        conversation.apply(AgentEvent::Opened {
            id: "session-1".to_owned(),
            how: Opened::Resumed,
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
            Some(Command::Prompt("hello".to_owned()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.submit("again"), None);
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
    fn shows_one_plan_a_turn_as_it_stands() {
        let mut conversation = Conversation::new();
        conversation.submit("plan");
        let first = vec![step("Read", StepState::InProgress)];
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
            Some(Command::Prompt("again".to_owned()))
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
            Some(Command::Prompt(prompt.to_owned()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.checking(), None);
        assert_eq!(conversation.goal().map(Goal::turns), Some(2));
        assert_eq!(conversation.take_changed(), [4]);
        assert_eq!(conversation.entries()[4], Entry::Cox(prompt.to_owned()));

        take_turn(&mut conversation, true);
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("Goal met: `make` passed.".to_owned()))
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

        let Some(Command::Prompt(prompt)) = conversation.checked(ran(1, &output)) else {
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
            Some(Command::Prompt("again".to_owned()))
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
            Some(Command::Prompt(prompt.to_owned()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.judging(), None);
        assert_eq!(conversation.goal().map(Goal::turns), Some(2));
        assert_eq!(conversation.take_changed(), [4]);
        assert_eq!(conversation.entries()[4], Entry::Cox(prompt.to_owned()));

        take_turn(&mut conversation, true);
        assert_eq!(
            conversation.judged(Ok("I read it again.\n\n**PASS**\n".to_owned())),
            None
        );
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("Goal met, as judged.".to_owned()))
        );
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
        let Command::Prompt(prompt) = prompt else {
            panic!("a prompt");
        };
        assert!(
            prompt.starts_with(
                "Goal not met, as judged: the judge gave no verdict Go on until it is met."
            ),
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
