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

use cox_core::entry::{Attachment, Committed};
use cox_core::goal;
use cox_core::store::{ChatId, Record};
use cox_git::commits::{self, Turn};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::chat::conversation::{
    self, Answer, Came, Conversation, Does, Due, Entry, Gathered, Goal, LoginOption, Patch,
    Proposed, State, TerminalLogin,
};
use crate::daemon::Internal;
use crate::trigger::{self, Arranged, By, On, Trigger};
use cox_acp::{Login, StopReason};
use coxd::agent::mcp::{Ended, Ran, ToolRequest};
use coxd::agent::setup::Setup;
use coxd::agent::{Agent, AgentEvent, AgentThread, Command};
use coxd::agents::Agents;
use coxd::chat::judge::{Heard, Judge, Starts};

/// How long the name of the chat that opened a chat grows, in
/// characters, in the note that says so.
const OPENED_BY_LENGTH: usize = 40;

/// How long the human waits on the triggers that gather what they need
/// to answer a permission request, before they are asked without what
/// has not come; and how long a trigger tried from its card runs.
pub(crate) const GATHER_FOR: std::time::Duration = std::time::Duration::from_mins(1);

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

/// The repositories of `cwd`, where a chat works, each with its last
/// commit now.
fn heads(cwd: &Path) -> Vec<Head> {
    coxd::repositories::of(cwd)
        .into_iter()
        .map(|(name, folder)| Head {
            commit: commits::head(&folder),
            name,
            folder,
        })
        .collect()
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

/// Where a chat finds the human's triggers and keeps how they are
/// arranged and ran: this Cox's places, or, for a test, a folder of its
/// own.
enum Places {
    Cox,
    #[cfg(test)]
    In(PathBuf),
}

impl Places {
    /// The folder of the human's workflow, with its triggers.
    fn workflow(&self) -> PathBuf {
        match self {
            Self::Cox => coxd::settings::workflow(),
            #[cfg(test)]
            Self::In(root) => root.join("workflow"),
        }
    }

    /// Where `kept` is kept, if Cox can tell.
    fn kept(&self, kept: coxd::storage::Kept) -> Option<PathBuf> {
        match self {
            Self::Cox => coxd::storage::path(kept),
            #[cfg(test)]
            Self::In(root) => Some(root.join(kept.place())),
        }
    }
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
    /// The triggers that gather what the human needs to answer the
    /// permission request the agent waits on, by their files, in the
    /// order of their chips, each with whether it is waited on, and its
    /// command while it runs. Dropping one lets go of its command.
    gathering: Vec<(PathBuf, bool, Option<Task>)>,
    /// The agents of the triggers `by: new` that gather, by their files,
    /// each with what it was set up with, once one was asked.
    gatherers: HashMap<PathBuf, (String, Judge)>,
    /// What asks the human without what has not come, once the time is
    /// up, while the triggers gather.
    gather_until: Option<Task>,
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
    /// Where the chat finds the triggers and keeps how they ran.
    places: Places,
    /// Where the turn that runs started, while one runs: the folder the
    /// chat works in, and the last commit of each of its repositories then,
    /// to tell the commits the agent makes in the turn.
    turn_heads: Option<(PathBuf, Vec<Head>)>,
}

/// A repository of the folder a chat works in, as a turn started.
struct Head {
    /// The project's directory it is, by name, or none for the repository
    /// the folder is in (see [`coxd::repositories::of`]).
    name: Option<String>,
    folder: PathBuf,
    /// Its last commit then, by its full name, if it had one.
    commit: Option<String>,
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
            gathering: Vec::new(),
            gatherers: HashMap::new(),
            gather_until: None,
            agents,
            internal,
            watched: false,
            unsent: Vec::new(),
            told: None,
            turn_heads: None,
            places: Places::Cox,
        }
    }

    /// The chat, finding the human's triggers and keeping how they ran in
    /// `root`, not in this Cox's places.
    #[cfg(test)]
    fn keeping_in(mut self, root: &Path) -> Self {
        self.places = Places::In(root.to_owned());
        self
    }

    /// The conversation of a new chat whose agent gets the instructions
    /// of `instructions`, set up as `setup` says: it notes them.
    #[must_use]
    pub fn opening(instructions: &[PathBuf], setup: Option<&Setup>) -> Conversation {
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
            );
        Conversation::noting_each(notes).with_keeping(coxd::agent::setup::keeps_in_wiki(setup))
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
        let (triggers, unread) = trigger::of(cwd, &self.places.workflow());
        for why in unread {
            tracing::error!("a trigger does not run: {why}");
        }
        let kept = self.places.kept(coxd::storage::Kept::Triggers);
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

    /// Asks the agent, in the human's place, to merge what it made, with
    /// `text`, closing the work the human's last line left it.
    pub fn merge(&mut self, text: &str) {
        if let Some(command) = self.conversation.merge(text) {
            self.send(command);
        }
    }

    /// The human sent `text`. Answers whether it went: not while a turn
    /// runs, unless the agent takes it into the turn.
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
            && !self.conversation.steering()
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
        // The commits of the turn show as it ends, ahead of what runs then;
        // a turn a steered line goes on with ends later.
        if (closed || matches!(event, AgentEvent::TurnEnded(_))) && !self.conversation.steering() {
            self.show_commits();
        }
        let asks = matches!(event, AgentEvent::PermissionRequest { .. });
        let was_checking = self.conversation.state() == State::Checking;
        self.conversation.apply(event);
        if let Some(about) = self.conversation.take_plan_changed() {
            self.fire(&On::PlanChanged, about, in_sketch);
        }
        if closed {
            self.running = None;
            self.check = None;
            self.judge = None;
            self.askers.clear();
            self.asked = None;
            self.gatherers.clear();
        }
        self.settle_gathering();
        if asks {
            self.gather(in_sketch);
        }
        if !was_checking && self.conversation.state() == State::Checking {
            self.check_next();
        } else {
            self.after();
        }
    }

    /// Shows the commits the agent made in the turn that ends, in the
    /// repositories of the folder it worked in, marking each in Git with
    /// the turn, when the chat is kept, for the human to find the turn from
    /// the commit (see [`cox_git::commits`]).
    fn show_commits(&mut self) {
        let Some((cwd, heads)) = self.turn_heads.take() else {
            return;
        };
        let turn = match (&self.record, self.conversation.turn_start()) {
            (Some(record), Some((entry, line))) => Some(Turn {
                chat: record.chat().number(),
                entry,
                line: line.to_owned(),
            }),
            _ => None,
        };
        let mut committed = Vec::new();
        for head in heads {
            for commit in commits::made(&head.folder, head.commit.as_deref()) {
                if let Some(turn) = &turn
                    && let Err(error) = commits::mark(&head.folder, &commit.id, turn)
                {
                    tracing::error!("a commit is not marked with its turn: {error}");
                }
                committed.push(Committed {
                    folder: cwd.clone(),
                    repository: head.name.clone(),
                    amended: commit.earlier.last().map(|earlier| earlier.id.clone()),
                    id: commit.id,
                    subject: commit.subject,
                    added: commit.added,
                    removed: commit.removed,
                });
            }
        }
        self.conversation.committed(committed);
    }

    /// Has the triggers of the chat's folder that run as the agent asks
    /// the human's permission, in a sketch or not, as `in_sketch` says,
    /// gather what the human needs to answer, all at once: their
    /// commands run, and their agents are asked, until each said, or the
    /// time is up.
    fn gather(&mut self, in_sketch: bool) {
        let (Some(about), Some(cwd)) = (
            self.conversation.asked_about(),
            self.running.as_ref().map(|running| running.cwd.clone()),
        ) else {
            return;
        };
        let due = self.due_on(&cwd, &On::PermissionAsked, in_sketch);
        if due.is_empty() {
            return;
        }
        self.conversation
            .gather(due.iter().map(|due| due.name.clone()).collect());
        self.gathering = due
            .iter()
            .map(|due| (due.file.clone(), true, None))
            .collect();
        for (at, due) in due.iter().enumerate() {
            match &due.does {
                Does::Run(command) => self.gather_by_running(at, command.clone()),
                Does::Ask(text) => self.gather_by_asking(at, due, text, &cwd, &about),
                Does::Tell(_) => self.conversation.gathered(
                    at,
                    Gathered::Broken(
                        "A trigger by: this does not run as the agent asks: the agent waits \
                         on the human."
                            .to_owned(),
                    ),
                ),
            }
        }
        let (key, generation) = (self.key, self.generation);
        let internal = self.internal.clone();
        self.gather_until = Some(spawn(async move {
            tokio::time::sleep(GATHER_FOR).await;
            let _ = internal.send(Internal::GatheredEnough { key, generation });
        }));
        self.settle_gathering();
    }

    /// Runs `command`, of the trigger at `at` of those that gather, in a
    /// terminal on the stage, and takes what it printed once it ends.
    fn gather_by_running(&mut self, at: usize, command: String) {
        let Some((cwd, tools)) = self.tools() else {
            self.conversation.gathered(
                at,
                Gathered::Broken("the chat has no terminal to run it in".to_owned()),
            );
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
            self.conversation.gathered(
                at,
                Gathered::Broken("the chat's terminals are gone".to_owned()),
            );
            return;
        }
        let (key, generation) = (self.key, self.generation);
        let internal = self.internal.clone();
        let file = self.gathering[at].0.clone();
        self.gathering[at].2 = Some(spawn(async move {
            let gathered = match ran.await {
                Ok(Ok(ran)) => Gathered::Printed(ran),
                Ok(Err(error)) => Gathered::Broken(error),
                Err(_) => Gathered::Broken("its terminal went away".to_owned()),
            };
            let _ = internal.send(Internal::Gathered {
                key,
                generation,
                file,
                gathered,
            });
        }));
    }

    /// Asks the agent of the trigger `due`, at `at` of those that gather,
    /// which gathers as `text` says,
    /// of what the chat's agent asks, as `about` tells it, starting the
    /// agent if the chat has none of it yet, or one set up otherwise.
    fn gather_by_asking(&mut self, at: usize, due: &Due, text: &str, cwd: &Path, about: &str) {
        let (allow, reject) = match self.due.get(&due.file) {
            Some(Trigger {
                by: By::New { allow, reject, .. },
                ..
            }) => (allow.clone(), reject.clone()),
            _ => (Vec::new(), Vec::new()),
        };
        let set_up = format!("{text}\n{allow:?}\n{reject:?}");
        if self
            .gatherers
            .get(&due.file)
            .is_none_or(|(was, _)| *was != set_up)
        {
            let Some(starts) = &self.starts else {
                self.conversation.gathered(
                    at,
                    Gathered::Broken("the chat has no agent to run it".to_owned()),
                );
                return;
            };
            let (key, generation) = (self.key, self.generation);
            let internal = self.internal.clone();
            let file = due.file.clone();
            let gatherer = Judge::start_as(
                starts,
                cwd.to_owned(),
                trigger::gathers(text, &allow, &reject),
                move |heard| {
                    let gathered = match heard {
                        Heard::Judged(Ok(said)) => Gathered::Said(said),
                        Heard::Judged(Err(error)) | Heard::Closed(error) => Gathered::Broken(error),
                    };
                    let _ = internal.send(Internal::Gathered {
                        key,
                        generation,
                        file: file.clone(),
                        gathered,
                    });
                },
            );
            self.gatherers.insert(due.file.clone(), (set_up, gatherer));
        }
        if let Some((_, gatherer)) = self.gatherers.get(&due.file) {
            gatherer.put(trigger::asked_to_gather(about));
        }
    }

    /// The trigger of `file` that gathers, for the agent of `generation`,
    /// came back as `gathered` says.
    pub fn heard_gathered(&mut self, generation: u64, file: &Path, gathered: Gathered) {
        if generation != self.generation {
            return;
        }
        if matches!(gathered, Gathered::Broken(_)) {
            self.gatherers.remove(file);
        }
        let Some(at) = self
            .gathering
            .iter()
            .position(|(gathers, waited, _)| *waited && gathers == file)
        else {
            return;
        };
        self.gathering[at].1 = false;
        self.gathering[at].2 = None;
        self.conversation.gathered(at, gathered);
        self.settle_gathering();
    }

    /// The time the human waits on the triggers that gather, for the
    /// agent of `generation`, is up: they are asked without what has not
    /// come.
    pub fn gathered_enough(&mut self, generation: u64) {
        if generation != self.generation || self.gathering.is_empty() {
            return;
        }
        self.conversation.stop_gathering(&format!(
            "It took longer than {} seconds: the human was asked without it.",
            GATHER_FOR.as_secs()
        ));
        self.settle_gathering();
    }

    /// Lets go of what gathers, once the conversation no longer waits on
    /// it: each said, the time is up, or the request is no longer asked,
    /// as when the human answered first.
    fn settle_gathering(&mut self) {
        if self.gathering.is_empty() || self.conversation.gathering() {
            return;
        }
        self.conversation
            .stop_gathering("Not waited for: the request was no longer asked.");
        for (file, waited, _) in std::mem::take(&mut self.gathering) {
            if waited && let Some((_, gatherer)) = self.gatherers.get(&file) {
                gatherer.cancel();
            }
        }
        self.gather_until = None;
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
        let last = self.places.kept(coxd::storage::Kept::LastTurns);
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
        self.came(Came::Found(said));
    }

    /// The agent of the trigger that runs said `said`: the agent goes on if
    /// the trigger found something.
    fn found(&mut self, said: Result<String, String>) {
        self.asked = None;
        let running = self.running_trigger();
        let next = self.conversation.found(said);
        self.keep_ran(running);
        self.go_on(next);
    }

    /// Takes `came`, what came back of what runs as the turn ended, or,
    /// while the agent asks the human in the middle of it, has the
    /// conversation keep it until they answered.
    fn came(&mut self, came: Came) {
        if self.conversation.state() == State::Asking {
            self.conversation.hold(came);
            return;
        }
        match came {
            Came::Ran(ran) => self.checked(ran),
            Came::Judged(judged) => self.judged(judged),
            Came::Found(said) => self.found(said),
        }
    }

    /// Lets go of what ran as the turn ended, once the chat stopped it,
    /// as when the human did: the command stops, and the judge, or the
    /// agent of the trigger, that is asked is told to stop.
    fn let_checks_go(&mut self, was_checking: bool) {
        if self.conversation.runs_checks() {
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

    /// Shows the human `shown`, a picture or a video kept from the path
    /// `from`, with `caption`, if given, in the conversation.
    pub fn show_media(&mut self, shown: coxd::media::Shown, from: String, caption: Option<String>) {
        self.conversation.show_media(shown, from, caption);
    }

    /// Notes in the conversation that the agent showed the human `text`,
    /// the file at `path`, from `cwd` where the chat works, Markdown if it
    /// is `markdown`, or else code at `lines`, keeping a copy of it as it
    /// is now, for the human to read on the phone too, where there is no
    /// stage.
    pub fn show_file(
        &mut self,
        (path, cwd): (&Path, &Path),
        text: &str,
        markdown: bool,
        lines: Option<(usize, usize)>,
    ) {
        let file = match coxd::media::keep_text(text, markdown, None) {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!("{} is not kept to read: {error}", path.display());
                return;
            }
        };
        let real = std::fs::canonicalize(cwd).ok();
        let shown = path
            .strip_prefix(cwd)
            .ok()
            .or_else(|| path.strip_prefix(real.as_ref()?).ok())
            .unwrap_or(path);
        self.conversation
            .show_file(file, shown.display().to_string(), markdown, lines);
    }

    /// Shows the pictures and videos a trigger's command printed as
    /// Markdown does, `![caption](path)`, its paths from the chat's
    /// folder, noting those it cannot.
    fn show_printed(&mut self, output: &str) {
        let Some(cwd) = self.running.as_ref().map(|running| running.cwd.clone()) else {
            return;
        };
        for (caption, path) in coxd::media::printed(output) {
            match coxd::media::keep(&cwd.join(&path), None) {
                Ok(shown) => self.conversation.show_media(shown, path, caption),
                Err(why) => self.conversation.note(format!("Not shown: {why}")),
            }
        }
    }

    /// The goal's command, run while the agent of `generation` ran, ended
    /// as `ran` says.
    pub fn heard_check(&mut self, generation: u64, ran: Result<Ran, String>) {
        if generation == self.generation {
            self.came(Came::Ran(ran));
        }
    }

    /// The goal's command ended as `ran` says: the agent goes on if the
    /// goal is not met yet.
    fn checked(&mut self, ran: Result<Ran, String>) {
        self.check = None;
        if self.conversation.checking().is_none() {
            return;
        }
        if let (Some(_), Ok(ran)) = (self.conversation.trigger(), &ran) {
            self.show_printed(&ran.output);
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
        let kept = self.places.kept(coxd::storage::Kept::TriggerRuns);
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
            Heard::Judged(judged) => self.came(Came::Judged(judged)),
            Heard::Closed(error) => {
                self.judge = None;
                self.came(Came::Judged(Err(error)));
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
        self.propose(Proposed::Goal(goal.input()), why);
        self.conversation.goal().map(|goal| goal.what().clone())
    }

    /// Shows what the agent proposes, `proposed`, on a card in the
    /// conversation, with `why`.
    pub fn propose(&mut self, proposed: Proposed, why: Option<String>) {
        self.conversation.propose(proposed, why);
    }

    /// What was proposed at `index`, while the human has not answered it.
    #[must_use]
    pub fn proposed(&self, index: usize) -> Option<Proposed> {
        self.conversation.proposed(index).cloned()
    }

    /// The human answered what was proposed at `index`, as `answer` says
    /// they did; carrying it out is the caller's.
    pub fn answer_proposal(&mut self, index: usize, answer: Answer) {
        self.conversation.answer_proposal(index, answer);
    }

    /// The human changed the goal proposed at `index` into `goal`, and set
    /// it.
    pub fn change_proposal(&mut self, index: usize, goal: goal::Goal) {
        let Some(proposed) = self.proposed(index).as_ref().and_then(Proposed::goal) else {
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
        let was_checking = self.conversation.runs_checks();
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
        self.settle_gathering();
        if let Some(came) = self.conversation.take_held() {
            self.came(came);
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
        let was_checking = self.conversation.runs_checks();
        if let Some(command) = self.conversation.cancel() {
            self.send(command);
        }
        self.let_checks_go(was_checking);
        self.settle_gathering();
    }

    fn send(&mut self, command: Command) {
        match &self.running {
            // The agent side is gone once the connection closes, which the
            // conversation shows when the `Closed` event arrives.
            Some(running) => {
                if matches!(command, Command::Prompt(_)) {
                    let cwd = running.cwd.clone();
                    self.turn_heads = Some((cwd.clone(), heads(&cwd)));
                }
                let Some(running) = &self.running else {
                    return;
                };
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
        let commands_changed = self.conversation.take_commands_changed();
        let baseline_changed = self.conversation.take_baseline_changed();
        if let Some(record) = &self.record {
            if settings_changed && let Err(error) = record.settings(self.conversation.settings()) {
                tracing::error!("the chat's settings are not kept: {error:#}");
            }
            if commands_changed && let Err(error) = record.commands(self.conversation.commands()) {
                tracing::error!("the chat's commands are not kept: {error:#}");
            }
            if baseline_changed && let Err(error) = record.baseline(self.conversation.baseline()) {
                tracing::error!("the chat's plan is not kept: {error:#}");
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
    pub fn stands(&self) -> coxd::agent::mcp::Waited {
        coxd::agent::mcp::Waited {
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
    let sketches = coxd::storage::path(coxd::storage::Kept::Sketches);
    if sketches.is_some_and(|sketches| folder.starts_with(sketches)) {
        conversation::sketch_note(folder)
    } else {
        conversation::folder_note(folder)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::*;
    use crate::chat::conversation::CheckState;
    use coxd::agent::AgentThread;
    use coxd::testing::TempDir;

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

    /// Runs `test` as the daemon runs its chats, on a `LocalSet`.
    fn on_daemon(test: impl Future<Output = ()>) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, test);
    }

    /// An agent the test plays: what it is sent, what it tells, and what
    /// its tools ask of the stage.
    struct Played {
        commands: UnboundedReceiver<Command>,
        events: UnboundedSender<AgentEvent>,
        stage: UnboundedReceiver<ToolRequest>,
    }

    impl Played {
        /// The texts of the prompts the agent was sent since last asked.
        fn prompts(&mut self) -> Vec<String> {
            std::iter::from_fn(|| self.commands.try_recv().ok())
                .filter_map(|command| match command {
                    Command::Prompt(prompt) => Some(prompt.text),
                    _ => None,
                })
                .collect()
        }

        /// The command the stage was asked to run in a terminal, and where
        /// to answer how it ran, if it was asked.
        fn ran(&mut self) -> Option<(String, oneshot::Sender<Result<Ran, String>>)> {
            match self.stage.try_recv().ok()? {
                ToolRequest::RunCommand {
                    command, answer, ..
                } => Some((command, answer)),
                _ => None,
            }
        }
    }

    /// An agent working in `cwd` that the test plays.
    fn played(cwd: &Path) -> (Agent, Played) {
        let (commands, commands_got) = unbounded_channel();
        let (events_sent, events) = unbounded_channel();
        let (tools, stage) = unbounded_channel();
        let agent = Agent {
            commands,
            events,
            instructions: Vec::new(),
            setup: None,
            cwd: cwd.to_owned(),
            tools: Some(tools),
            chat_requests: None,
            thread: AgentThread::none(),
        };
        let played = Played {
            commands: commands_got,
            events: events_sent,
            stage,
        };
        (agent, played)
    }

    /// A chat as the daemon holds it, kept, whose agent and whose
    /// triggers' agents the test plays, working in a folder of its own,
    /// with the human's workflow and Cox's state in another.
    struct Talk {
        chat: Chat,
        generation: u64,
        agent: Played,
        /// The agents of the triggers, in the order they were started.
        started: Rc<RefCell<Vec<Played>>>,
        internal: UnboundedReceiver<Internal>,
        places: TempDir,
        cwd: TempDir,
    }

    impl Talk {
        /// A chat whose agent has started; it goes on with a session as
        /// `again` says, once closed.
        fn new(again: Option<Again>) -> Self {
            let mut talk = Self::starting(again);
            let (agent, played) = played(&talk.cwd);
            talk.chat.attach(agent, talk.generation, Vec::new());
            talk.agent = played;
            talk
        }

        /// A chat whose agent is on its way to start.
        fn starting(again: Option<Again>) -> Self {
            let places = TempDir::new();
            let cwd = TempDir::new();
            std::fs::create_dir_all(places.join("workflow").join(trigger::TRIGGERS)).unwrap();
            let (internal_sent, internal) = unbounded_channel();
            let started: Rc<RefCell<Vec<Played>>> = Rc::default();
            let starts: Starts = {
                let started = started.clone();
                Rc::new(move |cwd: PathBuf, _| {
                    let (agent, played) = played(&cwd);
                    started.borrow_mut().push(played);
                    agent
                })
            };
            let store = cox_core::store::Store::open(&places.join("cox.db")).unwrap();
            let id = store.add_chat(None, "Talk").unwrap();
            let record = Record::new(Rc::new(store), id);
            let mut chat = Chat::new(
                1,
                Conversation::new(),
                Some(record),
                again,
                Some(starts),
                None,
                internal_sent,
            )
            .keeping_in(&places);
            let generation = chat.next_generation();
            let (_, agent) = played(&cwd);
            Self {
                chat,
                generation,
                agent,
                started,
                internal,
                places,
                cwd,
            }
        }

        /// Gives the human a trigger named `name` in their workflow, its
        /// file saying `text`.
        fn trigger(&self, name: &str, text: &str) -> PathBuf {
            let file = self
                .places
                .join("workflow")
                .join(trigger::TRIGGERS)
                .join(format!("{name}.md"));
            std::fs::write(&file, text).unwrap();
            file
        }

        fn hear(&mut self, event: AgentEvent) {
            self.chat.hear(self.generation, event, false);
        }

        /// The human sends `line`, and the agent ends its turn.
        fn take_a_turn(&mut self, line: &str) {
            assert!(self.chat.submit(line, Vec::new()), "the line goes");
            self.hear(AgentEvent::TurnEnded(StopReason::EndTurn));
        }

        /// What a task of the chat told the daemon next.
        async fn told(&mut self) -> Internal {
            self.internal.recv().await.unwrap()
        }

        fn state(&self) -> State {
            self.chat.conversation().state()
        }
    }

    const SHELL_AT_TURN_END: &str = "---\non: turn.end\nby: shell\n---\ncargo test\n";

    fn exited(code: i32, output: &str) -> Ran {
        Ran {
            output: output.to_owned(),
            ended: Ended::Exited(code),
        }
    }

    #[test]
    fn a_line_the_human_sends_goes_to_the_agent() {
        on_daemon(async {
            let mut talk = Talk::new(None);

            assert!(talk.chat.submit("hello", Vec::new()));

            assert_eq!(talk.agent.prompts(), ["hello"]);
            assert_eq!(talk.state(), State::Busy);
        });
    }

    #[test]
    fn a_line_sent_while_a_turn_runs_does_not_go() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.chat.submit("hello", Vec::new());

            assert!(!talk.chat.submit("again", Vec::new()));

            assert_eq!(talk.agent.prompts(), ["hello"]);
        });
    }

    #[test]
    fn a_line_sent_while_the_agent_starts_goes_once_it_has() {
        on_daemon(async {
            let mut talk = Talk::starting(None);
            assert!(talk.chat.submit("hello", Vec::new()));
            let (agent, mut played) = played(&talk.cwd);

            talk.chat.attach(agent, talk.generation, Vec::new());

            assert_eq!(played.prompts(), ["hello"]);
        });
    }

    #[test]
    fn an_agent_started_for_a_generation_since_passed_is_not_talked_to() {
        on_daemon(async {
            let mut talk = Talk::starting(None);
            let stale = talk.generation;
            talk.generation = talk.chat.next_generation();
            let (agent, mut played) = played(&talk.cwd);

            talk.chat.attach(agent, stale, Vec::new());
            talk.chat.submit("hello", Vec::new());

            assert_eq!(played.prompts(), Vec::<String>::new());
            assert!(talk.chat.is_open(), "the chat waits on its new agent");
        });
    }

    #[test]
    fn a_turn_ended_lets_the_human_send_again() {
        on_daemon(async {
            let mut talk = Talk::new(None);

            talk.take_a_turn("hello");

            assert_eq!(talk.state(), State::Idle);
            assert!(talk.chat.submit("and then", Vec::new()));
        });
    }

    #[test]
    fn what_an_agent_stopped_since_tells_is_let_be() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.chat.submit("hello", Vec::new());

            talk.chat.hear(
                talk.generation - 1,
                AgentEvent::TurnEnded(StopReason::EndTurn),
                false,
            );

            assert_eq!(talk.state(), State::Busy);
        });
    }

    #[test]
    fn the_agent_closing_closes_the_chat() {
        on_daemon(async {
            let mut talk = Talk::new(None);

            talk.hear(AgentEvent::Closed(Some("gone".to_owned())));

            assert!(!talk.chat.is_open());
            assert_eq!(talk.state(), State::Closed);
        });
    }

    /// What starts a chat working in `cwd` again, with no session yet.
    fn again(cwd: &Path) -> Again {
        Again {
            cwd: cwd.to_owned(),
            session: None,
            setup: None,
        }
    }

    #[test]
    fn an_open_chat_is_not_started_again() {
        on_daemon(async {
            let cwd = TempDir::new();
            let mut talk = Talk::new(Some(again(&cwd)));

            assert!(talk.chat.take_again().is_none());
        });
    }

    #[test]
    fn a_closed_chat_is_started_again_and_goes_on() {
        on_daemon(async {
            let cwd = TempDir::new();
            let mut talk = Talk::new(Some(again(&cwd)));
            talk.hear(AgentEvent::Closed(None));

            let taken = talk.chat.take_again();

            assert_eq!(taken.map(|again| again.cwd), Some(cwd.to_path_buf()));
            assert_eq!(talk.state(), State::Idle, "a line sent waits for the agent");
        });
    }

    #[test]
    fn a_closed_chat_whose_agent_is_on_its_way_is_not_started_again() {
        on_daemon(async {
            let cwd = TempDir::new();
            let mut talk = Talk::new(Some(again(&cwd)));
            talk.hear(AgentEvent::Closed(None));

            talk.chat.next_generation();

            assert!(talk.chat.take_again().is_none());
        });
    }

    #[test]
    fn a_closed_chat_starts_again_with_the_session_its_agent_opened() {
        on_daemon(async {
            let cwd = TempDir::new();
            let mut talk = Talk::new(Some(again(&cwd)));
            talk.hear(AgentEvent::Opened {
                id: "session-1".to_owned(),
                how: coxd::agent::Opened::New,
                steers: false,
            });
            talk.hear(AgentEvent::Closed(None));

            let taken = talk.chat.take_again().unwrap();

            assert_eq!(taken.session.as_deref(), Some("session-1"));
        });
    }

    #[test]
    fn a_chat_closed_for_good_is_not_started_again() {
        on_daemon(async {
            let cwd = TempDir::new();
            let mut talk = Talk::new(Some(again(&cwd)));

            talk.chat.close("The sketch ended.".to_owned());

            assert_eq!(talk.state(), State::Closed);
            assert!(talk.chat.take_again().is_none());
        });
    }

    #[test]
    fn a_turn_ended_runs_the_triggers_due_in_a_terminal_where_the_chat_works() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("test", SHELL_AT_TURN_END);

            talk.take_a_turn("hello");

            let (command, _) = talk.agent.ran().expect("the trigger runs");
            assert_eq!(command, "cargo test");
            assert_eq!(talk.state(), State::Checking);
        });
    }

    #[test]
    fn a_trigger_turned_off_where_the_chat_works_does_not_run() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            let file = talk.trigger("test", SHELL_AT_TURN_END);
            trigger::Change::Switch {
                folder: talk.cwd.to_path_buf(),
                file,
                on: false,
                sum: None,
            }
            .make(Some(
                &talk.places.join(coxd::storage::Kept::Triggers.place()),
            ))
            .unwrap();

            talk.take_a_turn("hello");

            assert!(talk.agent.ran().is_none());
            assert_eq!(talk.state(), State::Idle);
        });
    }

    #[test]
    fn a_trigger_that_fails_has_the_agent_act_on_what_it_printed() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("test", SHELL_AT_TURN_END);
            talk.take_a_turn("hello");
            talk.agent.prompts();
            let (_, answer) = talk.agent.ran().unwrap();

            answer.send(Ok(exited(1, "1 test FAILED"))).unwrap();
            let Internal::Checked {
                generation, ran, ..
            } = talk.told().await
            else {
                panic!("the daemon is not told the trigger ran");
            };
            talk.chat.heard_check(generation, ran);

            let prompts = talk.agent.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(prompts[0].contains("1 test FAILED"), "{prompts:?}");
            assert_eq!(talk.state(), State::Busy);
        });
    }

    #[test]
    fn a_trigger_that_passes_leaves_the_chat_to_the_human() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("test", SHELL_AT_TURN_END);
            talk.take_a_turn("hello");
            talk.agent.prompts();
            let (_, answer) = talk.agent.ran().unwrap();

            answer.send(Ok(exited(0, "ok"))).unwrap();
            let Internal::Checked {
                generation, ran, ..
            } = talk.told().await
            else {
                panic!("the daemon is not told the trigger ran");
            };
            talk.chat.heard_check(generation, ran);

            assert_eq!(talk.agent.prompts(), Vec::<String>::new());
            assert_eq!(talk.state(), State::Idle);
        });
    }

    #[test]
    fn how_a_trigger_ran_is_kept_for_the_triggers_tab() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            let file = talk.trigger("test", SHELL_AT_TURN_END);
            talk.take_a_turn("hello");
            let (_, answer) = talk.agent.ran().unwrap();

            answer.send(Ok(exited(0, "ok"))).unwrap();
            let Internal::Checked {
                generation, ran, ..
            } = talk.told().await
            else {
                panic!("the daemon is not told the trigger ran");
            };
            talk.chat.heard_check(generation, ran);

            let runs = talk.places.join(coxd::storage::Kept::TriggerRuns.place());
            let kept = trigger::Ran::read(Some(&runs), &talk.cwd);
            assert_eq!(
                kept.get(&file).map(|ran| ran.state),
                Some(CheckState::Passed)
            );
        });
    }

    #[test]
    fn a_turn_ended_with_a_goal_runs_the_goals_command() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.chat
                .set_goal(Some(goal::Goal::Command("make check".to_owned())));

            talk.take_a_turn("hello");

            let (command, _) = talk.agent.ran().expect("the goal's command runs");
            assert_eq!(command, "make check");
        });
    }

    #[test]
    fn a_goal_not_met_has_the_agent_go_on() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.chat
                .set_goal(Some(goal::Goal::Command("make check".to_owned())));
            talk.take_a_turn("hello");
            talk.agent.prompts();
            let (_, answer) = talk.agent.ran().unwrap();

            answer.send(Ok(exited(2, "check failed"))).unwrap();
            let Internal::Checked {
                generation, ran, ..
            } = talk.told().await
            else {
                panic!("the daemon is not told the goal's command ran");
            };
            talk.chat.heard_check(generation, ran);

            assert_eq!(talk.agent.prompts().len(), 1, "the agent goes on");
            assert_eq!(talk.state(), State::Busy);
        });
    }

    #[test]
    fn a_goal_met_leaves_the_chat_to_the_human() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.chat
                .set_goal(Some(goal::Goal::Command("make check".to_owned())));
            talk.take_a_turn("hello");
            talk.agent.prompts();
            let (_, answer) = talk.agent.ran().unwrap();

            answer.send(Ok(exited(0, ""))).unwrap();
            let Internal::Checked {
                generation, ran, ..
            } = talk.told().await
            else {
                panic!("the daemon is not told the goal's command ran");
            };
            talk.chat.heard_check(generation, ran);

            assert_eq!(talk.agent.prompts(), Vec::<String>::new());
            assert_eq!(talk.state(), State::Idle);
        });
    }

    #[test]
    fn a_chat_runs_the_triggers_of_its_opening_once_its_agent_has_started() {
        on_daemon(async {
            let mut talk = Talk::starting(None);
            talk.trigger("hello", "---\non: chat.open\nby: shell\n---\necho hi\n");
            talk.chat.opened(false);
            let (agent, mut played) = played(&talk.cwd);
            assert!(played.ran().is_none());

            talk.chat.attach(agent, talk.generation, Vec::new());

            let (command, _) = played.ran().expect("the opening's trigger runs");
            assert_eq!(command, "echo hi");
        });
    }

    #[test]
    fn a_chat_whose_agent_does_not_run_runs_no_triggers() {
        on_daemon(async {
            let mut talk = Talk::starting(None);
            talk.trigger("hello", "---\non: chat.open\nby: shell\n---\necho hi\n");
            let before = talk.chat.conversation().entries().len();

            talk.chat
                .fire(&On::ChatOpen, "The chat opened.".to_owned(), false);

            assert_eq!(talk.chat.conversation().entries().len(), before);
        });
    }

    const NEW_AT_TURN_END: &str = "---\non: turn.end\nby: new\n---\nReview the change.\n";

    #[test]
    fn a_trigger_by_a_new_agent_is_asked_of_the_turn() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("review", NEW_AT_TURN_END);

            talk.take_a_turn("hello");

            let mut started = talk.started.borrow_mut();
            assert_eq!(started.len(), 1, "the trigger's agent starts");
            assert_eq!(started[0].prompts().len(), 1, "it is asked once");
        });
    }

    /// Plays the agent of the trigger started first saying `reply`, and
    /// hands the chat what the daemon is told of it.
    async fn trigger_says(talk: &mut Talk, reply: &str) {
        {
            let started = talk.started.borrow();
            let events = &started[0].events;
            events
                .send(AgentEvent::ReplyText(reply.to_owned()))
                .unwrap();
            events
                .send(AgentEvent::TurnEnded(StopReason::EndTurn))
                .unwrap();
        }
        let Internal::Trigger {
            generation,
            file,
            heard,
            ..
        } = talk.told().await
        else {
            panic!("the daemon is not told what the trigger's agent said");
        };
        talk.chat.heard_trigger(generation, &file, heard);
    }

    #[test]
    fn a_trigger_whose_agent_finds_something_has_the_chats_agent_act_on_it() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("review", NEW_AT_TURN_END);
            talk.take_a_turn("hello");
            talk.agent.prompts();

            trigger_says(&mut talk, "FAIL: the greeting is rude").await;

            let prompts = talk.agent.prompts();
            assert_eq!(prompts.len(), 1, "{prompts:?}");
            assert!(prompts[0].contains("the greeting is rude"), "{prompts:?}");
        });
    }

    #[test]
    fn a_trigger_whose_agent_finds_nothing_leaves_the_chat_to_the_human() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("review", NEW_AT_TURN_END);
            talk.take_a_turn("hello");
            talk.agent.prompts();

            trigger_says(&mut talk, "PASS").await;

            assert_eq!(talk.agent.prompts(), Vec::<String>::new());
            assert_eq!(talk.state(), State::Idle);
        });
    }

    #[test]
    fn what_the_agent_of_a_trigger_says_for_an_agent_stopped_since_is_let_be() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            let file = talk.trigger("review", NEW_AT_TURN_END);
            talk.take_a_turn("hello");
            talk.agent.prompts();

            talk.chat.heard_trigger(
                talk.generation - 1,
                &file,
                Heard::Judged(Ok("FAIL: rude".to_owned())),
            );

            assert_eq!(talk.agent.prompts(), Vec::<String>::new());
            assert_eq!(talk.state(), State::Checking, "the trigger still runs");
        });
    }

    const GATHERS: &str = "---\non: permission.asked\nby: shell\n---\ngit status\n";

    fn asks_permission() -> AgentEvent {
        AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("rm -rf target".to_owned()),
            content: None,
            options: vec!["Allow".to_owned(), "Reject".to_owned()],
        }
    }

    /// The states of what gathers for the permission request the agent
    /// waits on, as the human sees them.
    fn gathered(talk: &Talk) -> Vec<(CheckState, Option<String>)> {
        talk.chat
            .conversation()
            .entries()
            .iter()
            .rev()
            .find_map(|entry| match entry {
                Entry::Permission { gathered, .. } => Some(
                    gathered
                        .iter()
                        .map(|check| (check.state, check.said.clone()))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_permission_request_has_the_triggers_that_gather_run() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("status", GATHERS);
            talk.chat.submit("clean up", Vec::new());

            talk.hear(asks_permission());

            let (command, _) = talk.agent.ran().expect("the trigger that gathers runs");
            assert_eq!(command, "git status");
            assert!(talk.chat.conversation().gathering());
        });
    }

    #[test]
    fn what_a_trigger_that_gathers_printed_is_shown_with_the_request() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("status", GATHERS);
            talk.chat.submit("clean up", Vec::new());
            talk.hear(asks_permission());
            let (_, answer) = talk.agent.ran().unwrap();

            answer.send(Ok(exited(0, "nothing to commit"))).unwrap();
            let Internal::Gathered {
                generation,
                file,
                gathered: what,
                ..
            } = talk.told().await
            else {
                panic!("the daemon is not told what was gathered");
            };
            talk.chat.heard_gathered(generation, &file, what);

            assert!(!talk.chat.conversation().gathering());
            let shown = gathered(&talk);
            assert_eq!(shown.len(), 1);
            assert!(
                shown[0]
                    .1
                    .as_deref()
                    .is_some_and(|said| said.contains("nothing to commit")),
                "{shown:?}"
            );
        });
    }

    #[test]
    fn the_human_is_asked_without_what_has_not_come_once_the_time_is_up() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("status", GATHERS);
            talk.chat.submit("clean up", Vec::new());
            talk.hear(asks_permission());

            talk.chat.gathered_enough(talk.generation);

            assert!(!talk.chat.conversation().gathering());
            assert_eq!(talk.state(), State::Asking, "the human is asked");
        });
    }

    #[test]
    fn the_time_up_for_an_agent_stopped_since_is_let_be() {
        on_daemon(async {
            let mut talk = Talk::new(None);
            talk.trigger("status", GATHERS);
            talk.chat.submit("clean up", Vec::new());
            talk.hear(asks_permission());

            talk.chat.gathered_enough(talk.generation - 1);

            assert!(talk.chat.conversation().gathering());
        });
    }
}
