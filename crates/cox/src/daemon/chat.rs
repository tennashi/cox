//! One chat as the daemon runs it: its conversation, kept as it goes, and
//! the agent it talks to, with what works beside it: the check of its goal
//! and the goal's judge. What the human does reaches it from the window
//! (see [`crate::daemon::wire::ToDaemon`]), and what changes goes to the
//! window as a [`Patch`] of the conversation.
//!
//! Its tasks run on the daemon's thread, on its tokio `LocalSet`, and
//! tell the daemon what they heard as an [`Internal`] event, with the
//! chat's key and the generation of its agent, so that what an agent
//! stopped since says is let be.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cox_core::entry::Attachment;
use cox_core::goal;
use cox_core::store::{ChatId, Record};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::agent::mcp::{Ended, Ran, ToolRequest};
use crate::agent::setup::Setup;
use crate::agent::{Agent, AgentEvent, AgentThread, Command};
use crate::agents::Agents;
use crate::chat::conversation::{
    self, Answer, Conversation, Does, Due, Entry, Goal, LoginOption, Patch, State, TerminalLogin,
};
use crate::chat::judge::{Heard, Judge, Starts};
use crate::daemon::Internal;
use crate::trigger::{self, Arranged, By, On, Trigger};
use cox_acp::{Login, StopReason};

/// How long the name of the chat that opened a chat grows, in
/// characters, in the note that says so.
const OPENED_BY_LENGTH: usize = 40;

/// A task that stops when this is dropped, unless it was let go on its
/// own (see [`Task::detach`]).
pub struct Task(Option<tokio::task::JoinHandle<()>>);

impl Task {
    /// Lets the task go on until it ends on its own.
    pub fn detach(mut self) {
        self.0.take();
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

/// Spawns `future` on the daemon's `LocalSet`, as a [`Task`].
pub fn spawn(future: impl Future<Output = ()> + 'static) -> Task {
    Task(Some(tokio::task::spawn_local(future)))
}

/// What a closed chat starts its agent again with: where it works, the
/// session it goes on with, and how it is set up.
#[derive(Clone, Debug)]
pub struct Again {
    pub cwd: PathBuf,
    pub session: Option<String>,
    pub setup: Option<Setup>,
}

/// The agent a chat talks to, while it runs.
struct Running {
    commands: UnboundedSender<Command>,
    /// Where the chat asks the same of its stage as the agent's tools do,
    /// as running the goal's command, when the agent has Cox's tools.
    tools: Option<UnboundedSender<ToolRequest>>,
    /// The directory it works in.
    cwd: PathBuf,
    _thread: AgentThread,
    _hears: Vec<Task>,
}

pub struct Chat {
    key: u64,
    conversation: Conversation,
    /// Where the conversation is kept as it changes, if it is.
    record: Option<Record>,
    /// The agent, while it runs. Dropping it stops the agent.
    running: Option<Running>,
    /// What the chat sends its agent while it starts, which goes once it
    /// has.
    queued: Vec<Command>,
    /// Whether its agent is on its way to start.
    starting: bool,
    /// What starts the agent of a closed chat again, until it has.
    again: Option<Again>,
    /// The generation of the agent that runs, or last ran: each agent
    /// started for the chat is the next.
    generation: u64,
    /// The check of the goal that runs, if one does. Dropping it lets go
    /// of the answer, which stops the command.
    check: Option<Task>,
    /// The login in a terminal that runs, if one does.
    login: Option<Task>,
    /// What starts the judge of a goal in words.
    starts: Option<Starts>,
    /// The judge of the goal in words, once it was asked.
    judge: Option<Judge>,
    /// The triggers that were due, as a turn ended or something else
    /// happened, by their files, as the files had them.
    due: HashMap<PathBuf, Trigger>,
    /// Whether the chat runs the triggers of its opening once its agent
    /// has started, in a sketch or not.
    opens: Option<bool>,
    /// Whether the chat worked in a sketch as its last turn ended.
    in_sketch: bool,
    /// The tree of the chat's folder as the last turn ended, when a
    /// trigger due runs on what changed.
    ended_at: Option<String>,
    /// The tree of the chat's folder each trigger last ran at, by its
    /// file, to tell what changed since.
    trees: HashMap<PathBuf, String>,
    /// The agents of the triggers `by: new`, by their files, each with
    /// what it was set up with, once one was asked.
    askers: HashMap<PathBuf, (String, Judge)>,
    /// The trigger whose agent is asked now, if one is.
    asked: Option<PathBuf>,
    /// The agents the chat can talk to, and the id of the one it talks to.
    agents: Option<(Agents, String)>,
    internal: UnboundedSender<Internal>,
    /// Whether a window shows the conversation, so that it is told how it
    /// changes.
    pub watched: bool,
    /// The entries changed since a window was last told, which it is told
    /// next.
    unsent: Vec<usize>,
    /// What the window was last told, but the entries, to tell it only
    /// what changed.
    told: Option<Patch>,
}

impl Chat {
    /// A chat of `conversation`, kept in `record` if given, with no agent
    /// yet. `again` starts its agent again once the human sends to it,
    /// when it can; `starts` starts the judge of a goal in words.
    #[must_use]
    pub fn new(
        key: u64,
        conversation: Conversation,
        record: Option<Record>,
        again: Option<Again>,
        starts: Option<Starts>,
        agents: Option<(Agents, String)>,
        internal: UnboundedSender<Internal>,
    ) -> Self {
        Self {
            key,
            conversation,
            record,
            running: None,
            queued: Vec::new(),
            starting: false,
            again,
            generation: 0,
            check: None,
            login: None,
            starts,
            judge: None,
            due: HashMap::new(),
            opens: None,
            in_sketch: false,
            ended_at: None,
            trees: HashMap::new(),
            askers: HashMap::new(),
            asked: None,
            agents,
            internal,
            watched: false,
            unsent: Vec::new(),
            told: None,
        }
    }

    /// The conversation of a new chat whose agent gets the instructions
    /// of `instructions`, set up as `setup` says, and works in the folders
    /// of `also` too: it notes them.
    #[must_use]
    pub fn opening(
        instructions: &[PathBuf],
        setup: Option<&Setup>,
        also: &[PathBuf],
    ) -> Conversation {
        let notes = instructions
            .iter()
            .map(|path| conversation::instructions_note(path))
            .chain(
                setup
                    .iter()
                    .filter_map(|setup| setup.folder.as_deref())
                    .map(folder_note),
            )
            .chain(
                setup
                    .iter()
                    .filter_map(|setup| setup.context.as_deref())
                    .map(conversation::context_note),
            )
            .chain((!also.is_empty()).then(|| conversation::also_note(also)));
        Conversation::noting_each(notes).with_keeping(crate::agent::setup::keeps_in_wiki(setup))
    }

    #[must_use]
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    /// The record the chat is kept in, if it is.
    #[must_use]
    pub fn record(&self) -> Option<&Record> {
        self.record.as_ref()
    }

    /// The agents the chat can talk to, when it can choose.
    #[must_use]
    pub fn agents(&self) -> Option<&Agents> {
        self.agents.as_ref().map(|(agents, _)| agents)
    }

    /// The id of the agent the chat talks to, or none when it cannot
    /// choose one.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        self.agents
            .as_ref()
            .map_or("", |(_, chosen)| chosen.as_str())
    }

    /// Whether the chat's agent runs, or is on its way to.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.running.is_some() || self.starting
    }

    /// Whether a line sent to the closed chat starts its agent again.
    #[must_use]
    pub fn starts_again(&self) -> bool {
        self.again.is_some()
    }

    /// The generation the next agent of the chat is of, which it marks
    /// what it tells with.
    pub fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.starting = true;
        self.generation
    }

    /// Takes what starts the agent of the closed chat again, if it can,
    /// and has the conversation go on: a line sent waits for the agent.
    pub fn take_again(&mut self) -> Option<Again> {
        if self.conversation.state() != State::Closed || self.starting {
            return None;
        }
        let again = self.again.take()?;
        self.conversation.reopen();
        Some(again)
    }

    /// Talks to `agent`, of `generation`, from now on: sends it what the
    /// human asks, hears what it tells, and passes on what its tools ask of
    /// the window. `also` works beside the agent, as long as it runs.
    pub fn attach(&mut self, agent: Agent, generation: u64, also: Vec<Task>) {
        if generation != self.generation {
            return;
        }
        let Agent {
            commands,
            mut events,
            thread,
            cwd,
            tools,
            chat_requests,
            ..
        } = agent;
        let key = self.key;
        let internal = self.internal.clone();
        let hears = spawn(async move {
            while let Some(event) = events.recv().await {
                let heard = Internal::Agent {
                    key,
                    generation,
                    event,
                };
                if internal.send(heard).is_err() {
                    break;
                }
            }
        });
        let mut hearing = also;
        hearing.push(hears);
        if let Some(mut chat_requests) = chat_requests {
            let internal = self.internal.clone();
            hearing.push(spawn(async move {
                while let Some(request) = chat_requests.recv().await {
                    let asked = Internal::Asked {
                        key,
                        generation,
                        request,
                    };
                    if internal.send(asked).is_err() {
                        break;
                    }
                }
            }));
        }
        self.starting = false;
        self.running = Some(Running {
            commands,
            tools,
            cwd,
            _thread: thread,
            _hears: hearing,
        });
        // The session is set as the chat was, or a new chat as the human
        // chose last, once it opens.
        for (id, value) in self.wanted_settings() {
            self.send(Command::Set { id, value });
        }
        for command in std::mem::take(&mut self.queued) {
            self.send(command);
        }
        if let Some(in_sketch) = self.opens.take() {
            self.fire(&On::ChatOpen, "The chat opened.".to_owned(), in_sketch);
        }
    }

    /// Has the chat, which just opened, run the triggers of its opening
    /// once its agent has started, in a sketch or not.
    pub fn opened(&mut self, in_sketch: bool) {
        self.opens = Some(in_sketch);
    }

    /// Runs the triggers of the chat's folder that run on `on`, which
    /// happened as `about` tells their agents, in a sketch or not, as
    /// `in_sketch` says: as soon as the chat waits for the human. A chat
    /// whose agent does not run runs none.
    pub fn fire(&mut self, on: &On, about: String, in_sketch: bool) {
        let Some(cwd) = self.running.as_ref().map(|running| running.cwd.clone()) else {
            return;
        };
        let due = self.due_on(&cwd, on, in_sketch);
        self.conversation.run_event(
            conversation::Happened {
                on: on.name().to_owned(),
                about,
            },
            due,
        );
        self.after();
    }

    /// The triggers of `cwd` that run on `on` there, in a sketch or not,
    /// as `in_sketch` says, on what changed since each last ran, when it
    /// says which files, as the conversation runs them.
    fn due_on(&mut self, cwd: &Path, on: &On, in_sketch: bool) -> Vec<Due> {
        let (triggers, unread) = trigger::of(cwd, &crate::settings::workflow());
        for why in unread {
            tracing::error!("a trigger does not run: {why}");
        }
        let kept = crate::storage::path(crate::storage::Kept::Triggers);
        let triggers: Vec<Trigger> = Arranged::read(kept.as_deref(), cwd)
            .apply(triggers)
            .into_iter()
            .filter(|(trigger, enabled)| *enabled && trigger.runs_on(on, in_sketch))
            .map(|(trigger, _)| trigger)
            .collect();
        let watches = triggers
            .iter()
            .any(|trigger| trigger.when.changed.is_some());
        if watches {
            self.ended_at = trigger::tree(cwd);
        }
        let triggers: Vec<Trigger> = triggers
            .into_iter()
            .filter(|trigger| self.changed_for(cwd, trigger))
            .collect();
        triggers
            .into_iter()
            .map(|trigger| {
                let due = Due {
                    name: trigger.name.clone(),
                    file: trigger.file.clone(),
                    does: match trigger.by {
                        By::Shell => Does::Run(trigger.text.trim().to_owned()),
                        By::This => Does::Tell(trigger.text.trim().to_owned()),
                        By::New { .. } => Does::Ask(trigger.text.trim().to_owned()),
                    },
                };
                self.due.insert(trigger.file.clone(), trigger);
                due
            })
            .collect()
    }

    /// What follows from where the conversation got to: the triggers of
    /// the goal being met, if it was, and those of what happened that
    /// wait, once the chat waits for the human.
    fn after(&mut self) {
        if self.conversation.take_goal_met() {
            let in_sketch = self.in_sketch;
            self.fire(
                &On::GoalMet,
                "The chat's goal is met.".to_owned(),
                in_sketch,
            );
            return;
        }
        if self.conversation.run_waiting() {
            self.check_next();
        }
    }

    /// The agent did not start: the chat says why, and is closed.
    pub fn not_started(&mut self, generation: u64, why: String) {
        if generation != self.generation {
            return;
        }
        self.starting = false;
        self.queued.clear();
        self.conversation.close(why);
    }

    /// What the chat's session is to be set to once its agent starts: each
    /// setting as the chat had it, or, for a chat that had none, as the
    /// human chose last of the model.
    fn wanted_settings(&self) -> Vec<(String, String)> {
        let settings = self.conversation.settings();
        if !settings.is_empty() {
            return settings
                .iter()
                .map(|setting| (setting.id.clone(), setting.value.clone()))
                .collect();
        }
        let Some(record) = &self.record else {
            return Vec::new();
        };
        record.choices(self.agent_id()).unwrap_or_else(|error| {
            tracing::error!("a new chat starts as its agent says, not as chosen: {error:#}");
            Vec::new()
        })
    }

    /// Sets the setting `id` of the chat's session to `value`, as the human
    /// chose: through its agent while it runs, or, for a closed chat, for
    /// its agent to set once it starts again. A choice of the model is
    /// kept for new chats to start with.
    pub fn set(&mut self, id: &str, value: &str) {
        let Some(setting) = self.conversation.settings().iter().find(|s| s.id == id) else {
            return;
        };
        if setting.is_of_model()
            && let Some(record) = &self.record
            && let Err(error) = record.choose(self.agent_id(), id, value)
        {
            tracing::error!("new chats do not start with this choice: {error:#}");
        }
        if self.running.is_some() {
            self.send(Command::Set {
                id: id.to_owned(),
                value: value.to_owned(),
            });
        } else {
            self.conversation.set(id, value);
        }
    }

    /// Starts the chat, opened from the chat named `opened_by`, on `text`
    /// as its first line, sent in the human's place, working until `goal`
    /// is met if given.
    pub fn tell(&mut self, opened_by: &str, text: &str, goal: Option<goal::Goal>) {
        // A chat named after its first line has a long name.
        let opened_by: String = match opened_by.char_indices().nth(OPENED_BY_LENGTH) {
            Some((end, _)) => format!("{}\u{2026}", opened_by[..end].trim_end()),
            None => opened_by.to_owned(),
        };
        self.conversation.note(format!("Opened by: {opened_by}"));
        if goal.is_some() {
            self.set_goal(goal);
        }
        self.told(text);
    }

    /// Sends `text` to the agent in the human's place, as when the chat
    /// moves into a sketch and goes on there. A closed chat's agent is
    /// started again first, by the daemon.
    pub fn told(&mut self, text: &str) {
        if let Some(command) = self.conversation.told(text) {
            self.send(command);
        }
    }

    /// The human sent `text`. Answers whether it went: not while a turn
    /// runs.
    pub fn submit(&mut self, text: &str, attached: Vec<Attachment>) -> bool {
        match self.conversation.submit_with(text, attached) {
            Some(command) => {
                self.send(command);
                true
            }
            None => false,
        }
    }

    /// Shows what the agent of `generation` told. A turn that ends with a
    /// goal to meet starts the goal's check, or asks its judge, and then
    /// runs the triggers due, of the folder the chat works in, in a
    /// sketch or not, as `in_sketch` says.
    pub fn hear(&mut self, generation: u64, event: AgentEvent, in_sketch: bool) {
        if generation != self.generation {
            return;
        }
        if matches!(event, AgentEvent::TurnEnded(StopReason::EndTurn))
            && self.conversation.state() == State::Busy
        {
            self.in_sketch = in_sketch;
            self.find_due(in_sketch);
        }
        if let (AgentEvent::Opened { id, .. }, Some(record)) = (&event, &self.record) {
            if let Err(error) = record.session(id) {
                tracing::error!("the agent's session is not kept, to resume: {error:#}");
            }
            if let Some(again) = &mut self.again {
                again.session = Some(id.clone());
            }
        }
        let closed = matches!(event, AgentEvent::Closed(_));
        let was_checking = self.conversation.state() == State::Checking;
        self.conversation.apply(event);
        if closed {
            self.running = None;
            self.check = None;
            self.judge = None;
            self.askers.clear();
            self.asked = None;
        }
        if !was_checking && self.conversation.state() == State::Checking {
            self.check_next();
        } else {
            self.after();
        }
    }

    /// Hands the conversation the triggers of the chat's folder due as
    /// the turn ends: those turned on there that run on a turn's end, in
    /// a sketch or not, as `in_sketch` says, and on what changed since
    /// each last ran, when it says which files.
    fn find_due(&mut self, in_sketch: bool) {
        let Some(cwd) = self.running.as_ref().map(|running| running.cwd.clone()) else {
            return;
        };
        // For the human to try a trigger on, from its card.
        let said = self.conversation.turn_told().map(str::to_owned);
        let last = crate::storage::path(crate::storage::Kept::LastTurns);
        if let Err(error) = trigger::card::keep_last_turn(last.as_deref(), &cwd, said.as_deref()) {
            tracing::error!("the turn is not kept to try triggers on: {error}");
        }
        let due = self.due_on(&cwd, &On::TurnEnd, in_sketch);
        self.conversation.set_due(due);
    }

    /// Whether what changed in `cwd` since `trigger` last ran, or since
    /// the last commit, before it ran, makes it run. Where Git cannot
    /// tell, it runs.
    fn changed_for(&self, cwd: &Path, trigger: &Trigger) -> bool {
        if trigger.when.changed.is_none() {
            return true;
        }
        let Some(now) = &self.ended_at else {
            return true;
        };
        let since = self
            .trees
            .get(&trigger.file)
            .cloned()
            .or_else(|| trigger::committed(cwd));
        let Some(since) = since else {
            return true;
        };
        trigger::changed_between(cwd, &since, now).is_none_or(|files| trigger.runs_for(&files))
    }

    /// Runs what is due next as the turn ended, as the conversation says:
    /// the goal's command, its judge, or a trigger.
    fn check_next(&mut self) {
        if let (Some(due), Some(tree)) = (self.conversation.trigger(), &self.ended_at) {
            self.trees.insert(due.file.clone(), tree.clone());
        }
        if let Some(command) = self.conversation.checking() {
            self.check(command.to_owned());
        } else if let Some(words) = self.conversation.judging().map(str::to_owned) {
            self.ask_judge(&words);
        } else if let Some(due) = self.conversation.asking().cloned() {
            self.ask_trigger(&due);
        } else if self.conversation.telling().is_some() {
            let running = self.running_trigger();
            let next = self.conversation.tell();
            self.keep_ran(running);
            self.go_on(next);
        }
    }

    /// Sends `next`, the turn the agent goes on with, or, with none, runs
    /// what is due next, if anything is.
    fn go_on(&mut self, next: Option<Command>) {
        match next {
            Some(command) => self.send(command),
            None if self.conversation.state() == State::Checking => self.check_next(),
            None => {}
        }
        self.after();
    }

    /// Asks the agent of the trigger `due`, starting it if the chat has
    /// none of it yet, or one set up otherwise, as when its file changed.
    fn ask_trigger(&mut self, due: &Due) {
        let Does::Ask(text) = &due.does else {
            return;
        };
        let (allow, reject) = match self.due.get(&due.file) {
            Some(Trigger {
                by: By::New { allow, reject, .. },
                ..
            }) => (allow.clone(), reject.clone()),
            _ => (Vec::new(), Vec::new()),
        };
        let set_up = format!("{text}\n{allow:?}\n{reject:?}");
        if self
            .askers
            .get(&due.file)
            .is_none_or(|(was, _)| *was != set_up)
        {
            let (Some(starts), Some(running)) = (&self.starts, &self.running) else {
                let running = self.running_trigger();
                let next = self
                    .conversation
                    .found(Err("the chat has no agent to run it".to_owned()));
                self.keep_ran(running);
                self.go_on(next);
                return;
            };
            let (key, generation) = (self.key, self.generation);
            let internal = self.internal.clone();
            let file = due.file.clone();
            let asker = Judge::start_as(
                starts,
                running.cwd.clone(),
                trigger::setup(text, &allow, &reject),
                move |heard| {
                    let _ = internal.send(Internal::Trigger {
                        key,
                        generation,
                        file: file.clone(),
                        heard,
                    });
                },
            );
            self.askers.insert(due.file.clone(), (set_up, asker));
        }
        let question = match self.conversation.event() {
            Some(happened) => trigger::asked_of(&happened.about),
            None => trigger::question(self.conversation.turn_told()),
        };
        if let Some((_, asker)) = self.askers.get(&due.file) {
            asker.put(question);
            self.asked = Some(due.file.clone());
        }
    }

    /// The agent of the trigger of `file` said `heard`.
    pub fn heard_trigger(&mut self, generation: u64, file: &Path, heard: Heard) {
        if generation != self.generation {
            return;
        }
        let said = match heard {
            Heard::Judged(said) => said,
            Heard::Closed(error) => {
                self.askers.remove(file);
                Err(error)
            }
        };
        if self.asked.as_deref() != Some(file)
            || self
                .conversation
                .asking()
                .is_none_or(|due| due.file != file)
        {
            return;
        }
        self.asked = None;
        let running = self.running_trigger();
        let next = self.conversation.found(said);
        self.keep_ran(running);
        self.go_on(next);
    }

    /// Lets go of what ran as the turn ended, once the chat stopped it,
    /// as when the human did: the command stops, and the judge, or the
    /// agent of the trigger, that is asked is told to stop.
    fn let_checks_go(&mut self, was_checking: bool) {
        if self.conversation.state() == State::Checking {
            return;
        }
        self.check = None;
        if !was_checking {
            return;
        }
        if let Some(judge) = &self.judge {
            judge.cancel();
        }
        if let Some(file) = self.asked.take()
            && let Some((_, asker)) = self.askers.get(&file)
        {
            asker.cancel();
        }
    }

    /// Runs `command`, the goal's, in a terminal on the stage, and hands
    /// the conversation how it went once it ends.
    fn check(&mut self, command: String) {
        let Some((cwd, tools)) = self.tools() else {
            self.checked(Err("the chat has no terminal to run it in".to_owned()));
            return;
        };
        let (answer, ran) = oneshot::channel();
        let request = ToolRequest::RunCommand {
            command,
            cwd,
            answer,
            asks_human: false,
        };
        if tools.send(request).is_err() {
            self.checked(Err("the chat's terminals are gone".to_owned()));
            return;
        }
        let (key, generation) = (self.key, self.generation);
        let internal = self.internal.clone();
        self.check = Some(spawn(async move {
            let ran = match ran.await {
                Ok(ran) => ran,
                Err(_) => Err("its terminal went away".to_owned()),
            };
            let _ = internal.send(Internal::Checked {
                key,
                generation,
                ran,
            });
        }));
    }

    /// Where the chat runs commands as its agent's tools do: the folder it
    /// works in, and the way to its stage.
    fn tools(&self) -> Option<(PathBuf, UnboundedSender<ToolRequest>)> {
        let running = self.running.as_ref()?;
        Some((running.cwd.clone(), running.tools.clone()?))
    }

    /// The goal's command, run while the agent of `generation` ran, ended
    /// as `ran` says.
    pub fn heard_check(&mut self, generation: u64, ran: Result<Ran, String>) {
        if generation == self.generation {
            self.checked(ran);
        }
    }

    /// The goal's command ended as `ran` says: the agent goes on if the
    /// goal is not met yet.
    fn checked(&mut self, ran: Result<Ran, String>) {
        self.check = None;
        if self.conversation.checking().is_none() {
            return;
        }
        let running = self.running_trigger();
        let next = self.conversation.checked(ran);
        self.keep_ran(running);
        self.go_on(next);
    }

    /// The file of the trigger that runs now, and where its chip is, if a
    /// trigger runs.
    fn running_trigger(&self) -> Option<(PathBuf, (usize, usize))> {
        Some((
            self.conversation.trigger()?.file.clone(),
            self.conversation.check_at()?,
        ))
    }

    /// Keeps how the trigger that ran, as [`Self::running_trigger`] said,
    /// went, as its chip shows it, for the Triggers tab to show, once it
    /// went one way or another.
    fn keep_ran(&self, ran: Option<(PathBuf, (usize, usize))>) {
        let Some((file, (entry, at))) = ran else {
            return;
        };
        let Some(cwd) = self.running.as_ref().map(|running| &running.cwd) else {
            return;
        };
        let Some(checks) = self
            .conversation
            .entries()
            .get(entry)
            .and_then(Entry::checks)
        else {
            return;
        };
        let Some(check) = checks.get(at) else {
            return;
        };
        if matches!(
            check.state,
            conversation::CheckState::Waiting | conversation::CheckState::Running
        ) {
            return;
        }
        let ran = trigger::Ran {
            state: check.state,
            at: jiff::Timestamp::now().as_second(),
            said: check.said.clone(),
        };
        let kept = crate::storage::path(crate::storage::Kept::TriggerRuns);
        if let Err(error) = ran.keep(kept.as_deref(), cwd, &file) {
            tracing::error!("how the trigger ran is not kept: {error}");
        }
    }

    /// Asks the judge of the goal, `words`, whether it is met now, starting
    /// the judge if the chat has none yet.
    fn ask_judge(&mut self, words: &str) {
        if self.judge.is_none() {
            let (Some(starts), Some(running)) = (&self.starts, &self.running) else {
                self.judged(Err("the chat has no agent to judge it".to_owned()));
                return;
            };
            let (key, generation) = (self.key, self.generation);
            let internal = self.internal.clone();
            self.judge = Some(Judge::start(
                starts,
                running.cwd.clone(),
                words,
                move |heard| {
                    let _ = internal.send(Internal::Judge {
                        key,
                        generation,
                        heard,
                    });
                },
            ));
        }
        let said = self.conversation.turn_told().map(str::to_owned);
        if let Some(judge) = &self.judge {
            judge.ask(words, said.as_deref());
        }
    }

    /// The judge said `heard` of the goal.
    pub fn heard_judge(&mut self, generation: u64, heard: Heard) {
        if generation != self.generation {
            return;
        }
        match heard {
            Heard::Judged(judged) => self.judged(judged),
            Heard::Closed(error) => {
                self.judge = None;
                self.judged(Err(error));
            }
        }
    }

    /// The judge said `judged` of the goal, or could not judge it: the
    /// agent goes on if the goal is not met yet.
    fn judged(&mut self, judged: Result<String, String>) {
        if self.conversation.judging().is_none() {
            return;
        }
        let next = self.conversation.judged(judged);
        self.go_on(next);
    }

    /// Shows `goal`, which the agent proposes, on a card in the
    /// conversation, with `why`, and answers the goal the chat has now.
    pub fn propose_goal(&mut self, goal: &goal::Goal, why: Option<String>) -> Option<goal::Goal> {
        self.conversation.propose(goal, why);
        self.conversation.goal().map(|goal| goal.what().clone())
    }

    /// The human answered the goal proposed at `index`: set it as it is,
    /// or let it be.
    pub fn answer_proposal(&mut self, index: usize, answer: Answer) {
        let Some(goal) = self.conversation.answer_proposal(index, answer) else {
            return;
        };
        if answer == Answer::Set {
            self.set_goal(Some(goal));
        }
    }

    /// The human changed the goal proposed at `index` into `goal`, and set
    /// it.
    pub fn change_proposal(&mut self, index: usize, goal: goal::Goal) {
        let Some(proposed) = self.conversation.proposed(index) else {
            return;
        };
        let answer = if proposed == goal {
            Answer::Set
        } else {
            Answer::Changed
        };
        self.conversation.answer_proposal(index, answer);
        self.set_goal(Some(goal));
    }

    /// The human chose, on the card at `index`, whether to keep what a
    /// chat learned in the wiki: answers which chat is to keep it, `None`
    /// for this one, when they chose to.
    pub fn answer_keep(&mut self, index: usize, keep: bool) -> Option<Option<ChatId>> {
        let chat = self.conversation.answer_keep(index, keep)?;
        keep.then_some(chat)
    }

    /// Asks the human, on a card, whether to keep what the chat kept as
    /// `chat`, or this one, learned in the wiki, at the milestone `why`
    /// says.
    pub fn ask_to_keep(&mut self, why: String, chat: Option<ChatId>) {
        self.conversation.ask_to_keep(why, chat);
    }

    /// Sets the chat to work toward `goal`, or toward none, and keeps it.
    /// The judge of the goal before, if any, is let go.
    pub fn set_goal(&mut self, goal: Option<goal::Goal>) {
        if goal.as_ref() != self.conversation.goal().map(Goal::what) {
            self.judge = None;
        }
        let was_checking = self.conversation.state() == State::Checking;
        self.conversation.set_goal(goal);
        self.let_checks_go(was_checking);
        if let Some(record) = &self.record
            && let Err(error) = record.goal(self.conversation.goal().map(Goal::what))
        {
            tracing::error!("the chat's goal is not kept: {error:#}");
        }
    }

    /// Adds `note`, a note from Cox, to the conversation.
    pub fn note(&mut self, note: String) {
        self.conversation.note(note);
    }

    /// Closes the chat, as `note` says why, as when the sketch it works in
    /// ends: its agent stops, and the chat cannot start it again.
    pub fn close(&mut self, note: String) {
        self.stop_agent();
        self.again = None;
        self.conversation.close(note);
    }

    /// Closes the chat, its agent stopped, as it switches from the agent
    /// named `from` to the agent named `to`, which `again` starts once the
    /// daemon starts it.
    pub fn switch_agent(
        &mut self,
        from: &str,
        to: &str,
        agents: (Agents, String),
        starts: Starts,
        again: Again,
    ) {
        self.stop_agent();
        self.agents = Some(agents);
        self.starts = Some(starts);
        self.conversation.switch(from, to);
        self.again = Some(again);
    }

    /// Stops the agent, and what works with it.
    fn stop_agent(&mut self) {
        self.running = None;
        self.starting = false;
        self.queued.clear();
        self.check = None;
        self.login = None;
        self.judge = None;
        self.generation += 1;
    }

    /// The human chose the option at `index` of what the agent asks.
    pub fn choose(&mut self, index: usize) {
        if self.conversation.logging_in() {
            if let Some(option) = self.conversation.choose_login(index) {
                self.log_in(option);
            }
        } else if let Some(command) = self.conversation.choose(index) {
            self.send(command);
        }
    }

    /// Logs in as `option` says: through the agent, or in a terminal on the
    /// stage, running the agent's command with what the way adds to it,
    /// after which the agent is told the human has logged in, however the
    /// command ended, and asks again if they have not.
    fn log_in(&mut self, option: LoginOption) {
        let Some(terminal) = option.terminal else {
            self.send(Command::LogIn(Login::Authenticate(option.id)));
            return;
        };
        let command = self
            .agents
            .as_ref()
            .map(|(agents, chosen)| agents.of(Some(chosen)).command);
        let (Some(command), Some((cwd, tools))) = (command, self.tools()) else {
            self.conversation
                .note("Cox cannot run the agent's command here to log in.".to_owned());
            self.send(Command::LogIn(Login::Done));
            return;
        };
        let (answer, ran) = oneshot::channel();
        let request = ToolRequest::RunCommand {
            command: login_command(&command, &terminal),
            cwd,
            answer,
            asks_human: true,
        };
        if tools.send(request).is_err() {
            self.send(Command::LogIn(Login::Done));
            return;
        }
        let (key, generation) = (self.key, self.generation);
        let internal = self.internal.clone();
        self.login = Some(spawn(async move {
            let ran = ran
                .await
                .unwrap_or_else(|_| Err("its terminal went away".to_owned()));
            let _ = internal.send(Internal::LoggedIn {
                key,
                generation,
                ran,
            });
        }));
    }

    /// The command that logs in ended as `ran` says: the agent is told the
    /// human has logged in, and asks again if they have not.
    pub fn logged_in(&mut self, generation: u64, ran: Result<Ran, String>) {
        if generation != self.generation {
            return;
        }
        self.login = None;
        let note = match ran {
            Ok(Ran {
                ended: Ended::Exited(0),
                ..
            }) => None,
            Ok(ran) => Some(format!("The login in the terminal ended {:?}.", ran.ended)),
            Err(error) => Some(format!("The login did not run: {error}")),
        };
        if let Some(note) = note {
            self.conversation.note(note);
        }
        self.send(Command::LogIn(Login::Done));
    }

    /// The human stopped the turn.
    pub fn stop(&mut self) {
        let was_checking = self.conversation.state() == State::Checking;
        if let Some(command) = self.conversation.cancel() {
            self.send(command);
        }
        self.let_checks_go(was_checking);
    }

    fn send(&mut self, command: Command) {
        match &self.running {
            // The agent side is gone once the connection closes, which the
            // conversation shows when the `Closed` event arrives.
            Some(running) => {
                let _ = running.commands.send(command);
            }
            None if self.starting => self.queued.push(command),
            None => {}
        }
    }

    /// Keeps the entries that changed since this was last called, and
    /// answers how the conversation changed for a window that watches it,
    /// if one does and anything did.
    pub fn flush(&mut self) -> Option<Patch> {
        let changed = self.conversation.take_changed();
        let settings_changed = self.conversation.take_settings_changed();
        if let Some(record) = &self.record {
            if settings_changed && let Err(error) = record.settings(self.conversation.settings()) {
                tracing::error!("the chat's settings are not kept: {error:#}");
            }
            for &index in &changed {
                if let Err(error) = record.entry(index, &self.conversation.entries()[index]) {
                    tracing::error!("the chat is not kept as it goes: {error:#}");
                    break;
                }
            }
        }
        if !self.watched {
            self.unsent.extend(changed);
            return None;
        }
        let mut changed = changed;
        if !self.unsent.is_empty() {
            changed.append(&mut self.unsent);
            changed.sort_unstable();
            changed.dedup();
        }
        let patch = self.conversation.patch(&changed);
        let rest = Patch {
            entries: Vec::new(),
            ..patch.clone()
        };
        if changed.is_empty() && self.told.as_ref() == Some(&rest) {
            return None;
        }
        self.told = Some(rest);
        Some(patch)
    }

    /// Has a window watch the conversation from now on, and answers all of
    /// it, for the window to start from.
    pub fn watch(&mut self) -> Patch {
        self.watched = true;
        self.unsent.clear();
        let whole = self.conversation.whole();
        self.told = Some(Patch {
            entries: Vec::new(),
            ..whole.clone()
        });
        whole
    }

    /// What the agent that waits for this chat is told of how it stands.
    #[must_use]
    pub fn stands(&self) -> crate::agent::mcp::Waited {
        crate::agent::mcp::Waited {
            asking: self.conversation.asking_for().map(str::to_owned),
            said: self.conversation.last_told().map(str::to_owned),
        }
    }

    /// What the chat's agent said last, as a sketch's summing up.
    #[must_use]
    pub fn last_reply(&self) -> Option<String> {
        self.conversation
            .entries()
            .iter()
            .rev()
            .find_map(|entry| match entry {
                Entry::Reply(text) => Some(text.clone()),
                _ => None,
            })
    }
}

/// The shell command that runs the agent's `command`, its program with
/// what it sets of its environment ahead of it, with what `terminal` adds
/// to it, for the human to log in: each word quoted, so the shell takes it
/// as it is, and the environment the way adds set after the command's,
/// over it.
pub(crate) fn login_command(command: &[String], terminal: &TerminalLogin) -> String {
    let assignment = |name: &str, value: &str| format!("{name}={}", quoted(value));
    let mut words = Vec::new();
    let mut program = false;
    for word in command {
        match word.split_once('=') {
            Some((name, value)) if !program && is_name(name) => {
                words.push(assignment(name, value));
            }
            _ => {
                if !program {
                    program = true;
                    words.extend(
                        terminal
                            .env
                            .iter()
                            .map(|(name, value)| assignment(name, value)),
                    );
                }
                words.push(quoted(word));
            }
        }
    }
    words.extend(terminal.args.iter().map(|arg| quoted(arg)));
    words.join(" ")
}

/// Whether `name` is the name of a variable of the environment.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `word`, quoted for the shell.
fn quoted(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@%+=,".contains(c))
    {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The note that tells the human where a chat set up to work in `folder`
/// works: a sketch's, apart from the project, or another folder.
fn folder_note(folder: &Path) -> String {
    let sketches = crate::storage::path(crate::storage::Kept::Sketches);
    if sketches.is_some_and(|sketches| folder.starts_with(sketches)) {
        conversation::sketch_note(folder)
    } else {
        conversation::folder_note(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|&word| word.to_owned()).collect()
    }

    #[test]
    fn a_login_in_a_terminal_runs_the_agents_command_with_the_ways_args() {
        let terminal = TerminalLogin {
            args: words(&["--cli", "auth login"]),
            env: [("MODE".to_owned(), "it's".to_owned())].into(),
        };
        let command = words(&[
            "CLAUDE_CODE_ENABLE_TODO_TOOLS=1",
            "npx",
            "-y",
            "@scope/agent",
        ]);
        assert_eq!(
            login_command(&command, &terminal),
            "CLAUDE_CODE_ENABLE_TODO_TOOLS=1 MODE='it'\\''s' npx -y @scope/agent --cli 'auth login'"
        );
    }

    #[test]
    fn an_argument_that_looks_like_a_setting_after_the_program_stays_one() {
        let terminal = TerminalLogin {
            args: Vec::new(),
            env: std::collections::BTreeMap::new(),
        };
        let command = words(&["agent", "A=b c"]);
        assert_eq!(login_command(&command, &terminal), "agent 'A=b c'");
    }
}
