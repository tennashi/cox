//! The agent side: runs the ACP connection on a thread with a tokio runtime
//! of its own, since GPUI runs its own executor, and talks to the window
//! over channels. Tokio's channels do not need a tokio runtime to wait on,
//! so the window awaits events on GPUI's executor. What goes over the
//! channels is said here too: the [`Command`]s the window sends, and the
//! [`AgentEvent`]s it hears.
//!
//! - [`instructions`] reads what the agent gets told of the project.
//! - [`mcp`] serves Cox's own tools to each chat's agent, and is the
//!   bridge the agent runs to reach them.
//! - [`setup`] says how the agent of a chat opened from a chat is set up:
//!   what it is told, which tools it gets, and which of its permission
//!   requests Cox answers in the human's place.
//! - [`tools`] says which MCP servers the agent gets: the stage's, and
//!   Cox's own.

pub mod instructions;
pub mod mcp;
pub mod setup;
pub mod tools;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cox_acp::{
    Block, Login, LoginMethod, McpServer, News, PermissionAnswer, PermissionKind, PlanEntry,
    PlanState, Session, SlashCommand, Steered, StopReason, ToolCallContent, Transport, Update,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tracing::Instrument as _;

use cox_core::entry::{
    Answer, Attachment, Content, Entry, LoginOption, Step, StepState, TerminalLogin, ToolCallResult,
};
use cox_core::settings::{Choice, Setting};

use crate::agent::instructions::Instructions;
use crate::agent::setup::{Asked, Decision, Policy, Setup};
use crate::chat::mention;
use crate::process::close_on_exec;

/// What the agent side tells the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// The agent opened the session `id`, to resume later, as `how` says,
    /// and `steers` the turn it takes with a line the human sends while
    /// it runs ([`Command::Steer`]), or not.
    Opened {
        id: String,
        how: Opened,
        steers: bool,
    },
    /// The agent gave the session a title, in place of any it gave before.
    Titled(String),
    /// The commands the agent runs when a line starts with `/` and one of
    /// their names, in place of those it told before.
    Commands(Vec<SlashCommand>),
    /// What the agent lets the human choose of the session, such as its
    /// model, each as it is set now: when the session opens, and each time
    /// a setting changes, as the human or the agent changes it.
    Settings(Vec<Setting>),
    /// The agent did not set the setting `name` as the human asked, for
    /// `error`.
    SettingFailed { name: String, error: String },
    /// A piece of the agent's reply.
    ReplyText(String),
    /// A piece of what the agent thinks on its way to the reply.
    ThoughtText(String),
    /// The plan the agent works to, all of it, in place of the plan it
    /// sent before.
    Plan(Vec<Step>),
    /// A tool call the agent made started, was retitled, changed what it
    /// shows, or ended.
    ToolCall {
        /// The same for every event of one call.
        id: String,
        title: Option<String>,
        /// What the call shows from now on, or `None` when that stays as it
        /// was.
        content: Option<Vec<Content>>,
        /// How the call ended, or `None` while it runs.
        result: Option<ToolCallResult>,
    },
    /// The agent asks permission for a tool call; the turn waits for an
    /// [`Command::Answer`], or for a [`Command::Cancel`].
    PermissionRequest {
        /// The id of the tool call, as in its [`AgentEvent::ToolCall`]s.
        id: String,
        title: Option<String>,
        /// What the call is about to do, if the request tells it.
        content: Option<Vec<Content>>,
        options: Vec<String>,
    },
    /// The agent asked permission for a tool call, and Cox answered in the
    /// human's place, as the chat's setup says, choosing `chosen` of
    /// `options`.
    PermissionDecided {
        id: String,
        title: Option<String>,
        content: Option<Vec<Content>>,
        options: Vec<String>,
        chosen: usize,
    },
    /// The agent asks the human to log in, one of the ways of `options`,
    /// before it opens the session or goes on with the turn; the agent
    /// side waits for a [`Command::LogIn`], or for a [`Command::Cancel`],
    /// which gives up. `failed` says why the way tried last failed, when
    /// one did.
    LoginNeeded {
        options: Vec<LoginOption>,
        failed: Option<String>,
    },
    /// Cox tells the human what it does on the way to the agent, as when
    /// it downloads the agent's program.
    Note(String),
    /// The agent side took a line the window steered into the turn
    /// ([`Command::Steer`]): it is the agent's now, in the turn that runs,
    /// or in one that starts for it, which ends with a
    /// [`AgentEvent::TurnEnded`] of its own.
    Steered,
    /// The turn ended.
    TurnEnded(StopReason),
    /// The agent answered the prompt with an error.
    TurnFailed(String),
    /// The connection to the agent closed, with the error that closed it, if
    /// any.
    Closed(Option<String>),
}

/// How the agent's session opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opened {
    /// A new session, for a new chat.
    New,
    /// The session the chat had before, which the agent went on with.
    Resumed,
    /// A new session for a chat that has a conversation already, so the
    /// agent gets told the conversation so far, along with the
    /// instructions in `instructions`, if any, and the context of its
    /// setup, if it was set up with one. The session the chat had is
    /// `lost` when the agent could not go on with it; a chat has none to
    /// go on with when it never got one, or when it moved, as into a
    /// sketch, where its agent works in another folder.
    Retold {
        lost: bool,
        instructions: Vec<PathBuf>,
        context: Option<String>,
    },
}

/// What the window asks of the agent side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Prompt(Prompt),
    /// A line the human sent while a turn runs, which the agent takes into
    /// the turn, for an agent that steers (see [`AgentEvent::Opened`]), or
    /// else once it ends, before the turn counts as ended. With no turn
    /// running, it starts one, as a prompt does. Each is answered with an
    /// [`AgentEvent::Steered`].
    Steer(Prompt),
    Answer(PermissionAnswer),
    /// Asks the agent to stop the turn it is taking.
    Cancel,
    /// Asks the agent to set the setting `id` of its session to `value`,
    /// when the session has the setting, offers the value, and is not set
    /// to it already: one kept from before may be gone since. It is set
    /// whether a turn runs or not.
    Set {
        id: String,
        value: String,
    },
    /// Tells the agent how the human logged in, as it asked.
    LogIn(Login),
}

/// A line for the agent, with what the human handed the chat with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    /// What goes after the line, in its order: text selected elsewhere
    /// on the screen, as a tab of code.
    pub attached: Vec<Attachment>,
}

impl From<String> for Prompt {
    fn from(text: String) -> Self {
        Self {
            text,
            attached: Vec::new(),
        }
    }
}

/// The window's ends of the channels to the agent side.
pub struct Agent {
    pub commands: UnboundedSender<Command>,
    pub events: UnboundedReceiver<AgentEvent>,
    /// The files the instructions the agent gets with the first prompt
    /// come from.
    pub instructions: Vec<PathBuf>,
    /// How the agent is set up, when the chat that opened its chat said.
    pub setup: Option<Setup>,
    /// The directory the agent works in.
    pub cwd: PathBuf,
    /// Where what the agent's tools ask of its chat goes, for the chat to
    /// ask the same of its stage, such as running the command of its goal
    /// in a terminal there; `None` when the agent got no tools of Cox's
    /// own.
    pub tools: Option<UnboundedSender<mcp::ToolRequest>>,
    /// What the agent's tools ask of the window, such as opening another
    /// chat; `None` when the agent got no tools of Cox's own.
    pub chat_requests: Option<UnboundedReceiver<mcp::ChatRequest>>,
    /// Stops the agent when dropped.
    pub thread: AgentThread,
}

/// How a chat's agent starts. In a chat opened from a chat, the agent is
/// set up as `setup` says: told its context after the instructions, and
/// its permission requests answered as the setup's policy says.
pub enum Start {
    /// For a new chat: the agent gets `instructions`, if any, ahead of the
    /// first prompt.
    New {
        instructions: Option<Instructions>,
        setup: Option<Setup>,
    },
    /// For a closed chat the human sends to again: the agent goes on with
    /// the chat's `session`, when the chat had one and the agent can.
    /// Otherwise it opens a new session, which gets `instructions`, if
    /// any, and is told `earlier`, what the chat showed, ahead of the first
    /// prompt.
    Again {
        session: Option<String>,
        instructions: Option<Instructions>,
        setup: Option<Setup>,
        earlier: Vec<Entry>,
    },
}

impl Start {
    /// How the agent of a closed chat that showed `earlier` starts again:
    /// going on with `session` if it can, or else told what the chat
    /// showed, with `instructions`, as they are now, set up as `setup`
    /// says if the chat was opened from a chat.
    #[must_use]
    pub fn again(
        instructions: Instructions,
        session: Option<String>,
        earlier: &[Entry],
        setup: Option<Setup>,
    ) -> Self {
        Self::Again {
            session,
            instructions: Some(instructions),
            setup,
            earlier: earlier.to_vec(),
        }
    }

    /// How the agent is set up, if the chat that opened its chat said.
    #[must_use]
    pub fn setup(&self) -> Option<&Setup> {
        match self {
            Self::New { setup, .. } | Self::Again { setup, .. } => setup.as_ref(),
        }
    }
}

/// How long the agent side's tasks get to end once the agent is stopped.
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// The thread the agent side runs on. Dropping it stops the agent, along
/// with every process the agent started, and waits for the thread to end.
///
/// The process ending does not do that by itself: the agent runs in a
/// process group of its own, and its processes outlive Cox unless the
/// connection to it is dropped first, which kills the group.
pub struct AgentThread {
    /// Never sent: dropping it tells the thread to stop.
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for AgentThread {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the agent's thread panicked");
        }
    }
}

/// Starts the agent side on a thread of its own. `connect` runs on its
/// runtime, telling the human what it does on the way, as when it
/// downloads the agent's program, with the function it is given, and
/// returns the transport to the agent, which works in `cwd`, an
/// absolute path, gets the tools of `mcp_servers`, and starts as `start`
/// says. What the agent side says is said in the span `spawn` is called
/// in, such as its chat's.
pub fn spawn<T: Transport + 'static>(
    connect: impl FnOnce(&dyn Fn(String)) -> anyhow::Result<T> + Send + 'static,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    start: Start,
) -> Agent {
    let source = match &start {
        Start::New { instructions, .. } => instructions
            .as_ref()
            .map(Instructions::paths)
            .unwrap_or_default(),
        Start::Again { .. } => Vec::new(),
    };
    let setup = start.setup().cloned();
    let policy = setup
        .as_ref()
        .map(|setup| setup.policy.clone())
        .unwrap_or_default();
    let (commands, received) = unbounded_channel();
    let (sent, events) = unbounded_channel();
    let (stop, stopped) = oneshot::channel();
    let works_in = cwd.clone();
    let span = tracing::Span::current();
    let thread = thread::spawn(move || {
        close_on_exec();
        let outcome = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| {
                let outcome = runtime.block_on(
                    async {
                        tokio::select! {
                            outcome = run(connect, cwd, mcp_servers, start, &policy, received, &sent) => outcome,
                            // Dropping the connection kills the agent.
                            _ = stopped => Ok(()),
                        }
                    }
                    .instrument(span),
                );
                // The connection's tasks go with the runtime.
                runtime.shutdown_timeout(STOP_TIMEOUT);
                outcome
            });
        let _ = sent.send(AgentEvent::Closed(outcome.err().map(|e| format!("{e:#}"))));
    });
    Agent {
        commands,
        events,
        instructions: source,
        setup,
        cwd: works_in,
        tools: None,
        chat_requests: None,
        thread: AgentThread {
            stop: Some(stop),
            thread: Some(thread),
        },
    }
}

/// What the human handed, as blocks of the prompt: what it says, with
/// where it is from, then its picture, if it has one.
fn handed_blocks(attachment: Attachment) -> impl Iterator<Item = Block> {
    let picture = attachment.picture.map(Block::Picture);
    [Block::Quote {
        uri: attachment.uri,
        text: attachment.text,
    }]
    .into_iter()
    .chain(picture)
}

async fn run<T: Transport + 'static>(
    connect: impl FnOnce(&dyn Fn(String)) -> anyhow::Result<T>,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    start: Start,
    policy: &Policy,
    commands: UnboundedReceiver<Command>,
    events: &UnboundedSender<AgentEvent>,
) -> anyhow::Result<()> {
    let transport = connect(&|note| {
        let _ = events.send(AgentEvent::Note(note));
    })?;
    // Heard by the human's logging in before the session opens, then by
    // the session.
    let commands = tokio::sync::Mutex::new(Commands::new(commands));
    let resume = match &start {
        Start::New { .. } => None,
        Start::Again { session, .. } => session.clone(),
    };
    // The settings as they stand, which the agent changes as it tells, in
    // between turns too.
    let settings = Arc::new(Mutex::new(Vec::new()));
    // The names of the agent's commands, as it last told them.
    let commands_known = Arc::new(Mutex::new(Vec::new()));
    // Where the files a line points to are, from.
    let folder = cwd.clone();
    let told = events.clone();
    let known = Arc::clone(&settings);
    let names = Arc::clone(&commands_known);
    let heard = move |news| match news {
        News::Titled(title) => {
            let _ = told.send(AgentEvent::Titled(title));
        }
        News::Commands(commands) => {
            *names.lock().expect("no one panics holding it") = commands
                .iter()
                .map(|command| command.name.clone())
                .collect();
            let _ = told.send(AgentEvent::Commands(commands));
        }
        News::Settings(changed) => {
            let _ = told.send(AgentEvent::Settings(shown_settings(&changed)));
            *known.lock().expect("no one panics holding it") = changed;
        }
    };
    cox_acp::connect(
        transport,
        cwd,
        mcp_servers,
        resume,
        heard,
        async |methods, failed| {
            let mut commands = commands.lock().await;
            logged_in(&methods, failed, &mut commands, events).await
        },
        async |mut session| {
            let mut commands = commands.lock().await;
            let (how, mut ahead) = opened(start, session.resumed());
            let _ = events.send(AgentEvent::Opened {
                id: session.id(),
                how,
                steers: session.steers(),
            });
            let _ = events.send(AgentEvent::Settings(shown_settings(session.settings())));
            session
                .settings()
                .clone_into(&mut settings.lock().expect("no one panics holding it"));
            while let Some(command) = commands.recv().await {
                // An answer with no permission request waiting has nothing to
                // answer, and a cancel with no turn running nothing to stop.
                match command {
                    Command::Prompt(prompt) => {
                        // What goes ahead of the first prompt goes once,
                        // and not ahead of a command, which the agent
                        // knows by the prompt's first block: it waits for
                        // the next line.
                        let command = runs_command(
                            &prompt.text,
                            &commands_known.lock().expect("no one panics holding it"),
                        );
                        let ahead = if command {
                            Vec::new()
                        } else {
                            std::mem::take(&mut ahead)
                        };
                        let prompt = ahead
                            .into_iter()
                            .map(Block::Text)
                            .chain(line(prompt, &folder));
                        take_turn(
                            &mut session,
                            prompt,
                            policy,
                            &settings,
                            &mut commands,
                            events,
                            &folder,
                        )
                        .await?;
                    }
                    // The turn the line was steered into ended before the
                    // agent side heard it: it starts a turn of its own.
                    Command::Steer(prompt) => {
                        let _ = events.send(AgentEvent::Steered);
                        commands.held.push_front(Command::Prompt(prompt));
                    }
                    Command::Set { id, value } => {
                        let set = async |id: &str, value: &str| session.set(id, value).await;
                        set_setting(set, &settings, &id, &value, events).await;
                    }
                    Command::Answer(_) | Command::Cancel | Command::LogIn(_) => {}
                }
            }
            Ok(())
        },
    )
    .await
}

/// Asks the human to log in, one of the ways of `methods`, telling why
/// the way tried last `failed`, if it did, and answers how they logged in.
///
/// # Errors
///
/// When the human gives up, or the window is gone.
async fn logged_in(
    methods: &[LoginMethod],
    failed: Option<String>,
    commands: &mut Commands,
    events: &UnboundedSender<AgentEvent>,
) -> anyhow::Result<Login> {
    let options = methods
        .iter()
        .map(|method| LoginOption {
            id: method.id.clone(),
            name: method.name.clone(),
            description: method.description.clone(),
            terminal: method.terminal.as_ref().map(|terminal| TerminalLogin {
                args: terminal.args.clone(),
                env: terminal.env.clone(),
            }),
        })
        .collect();
    let _ = events.send(AgentEvent::LoginNeeded { options, failed });
    loop {
        match commands.recv().await {
            Some(Command::LogIn(how)) => return Ok(how),
            Some(Command::Cancel) | None => anyhow::bail!("the human did not log in"),
            // A line sent, or a setting set, before the agent asked waits
            // for the session, which opens once the human has logged in.
            Some(command @ (Command::Prompt(_) | Command::Steer(_) | Command::Set { .. })) => {
                commands.held.push_back(command);
            }
            Some(Command::Answer(_)) => {}
        }
    }
}

/// What the window tells the agent side, with what came while the agent
/// waited for the human to log in, which is heard first.
struct Commands {
    received: UnboundedReceiver<Command>,
    held: VecDeque<Command>,
}

impl Commands {
    fn new(received: UnboundedReceiver<Command>) -> Self {
        Self {
            received,
            held: VecDeque::new(),
        }
    }

    /// The next command, held or new, or `None` once the window is gone.
    /// Cancelled, it loses none.
    async fn recv(&mut self) -> Option<Command> {
        match self.held.pop_front() {
            Some(command) => Some(command),
            None => self.received.recv().await,
        }
    }
}

/// How the session opened, for an agent that started as `start`, and
/// whether it `resumed` a session, with what goes ahead of the first
/// prompt: the instructions, the context of the setup, and what the chat
/// showed, when a session starts anew for a chat that had one.
fn opened(start: Start, resumed: bool) -> (Opened, Vec<String>) {
    match start {
        Start::New {
            instructions,
            setup,
        } => (Opened::New, told(instructions, setup.as_ref())),
        Start::Again { .. } if resumed => (Opened::Resumed, Vec::new()),
        Start::Again {
            session,
            instructions,
            setup,
            earlier,
        } => {
            let lost = session.is_some();
            // A chat where nothing was said yet, as one that switched to
            // another agent before the human sent a line, starts anew.
            let Some(earlier) = retold(&earlier, lost) else {
                return (Opened::New, told(instructions, setup.as_ref()));
            };
            let how = Opened::Retold {
                lost,
                instructions: instructions
                    .as_ref()
                    .map(Instructions::paths)
                    .unwrap_or_default(),
                context: setup.as_ref().and_then(|setup| setup.context.clone()),
            };
            let mut ahead = told(instructions, setup.as_ref());
            ahead.push(earlier);
            (how, ahead)
        }
    }
}

/// Whether `line` runs one of the agent's commands, named `commands`: it
/// starts with `/` and one of their names, then a space or its end.
fn runs_command(line: &str, commands: &[String]) -> bool {
    line.strip_prefix('/')
        .and_then(|rest| rest.split_whitespace().next())
        .is_some_and(|name| commands.iter().any(|command| command == name))
}

/// What a new session is told first: `instructions`, then the context of
/// `setup`, each when there is one.
fn told(instructions: Option<Instructions>, setup: Option<&Setup>) -> Vec<String> {
    instructions
        .map(|i| i.prompts())
        .into_iter()
        .flatten()
        .chain(setup.and_then(Setup::prompt))
        .collect()
}

/// The settings of the session as the window shows them.
fn shown_settings(settings: &[cox_acp::Setting]) -> Vec<Setting> {
    settings
        .iter()
        .map(|setting| Setting {
            id: setting.id.clone(),
            name: setting.name.clone(),
            description: setting.description.clone(),
            category: setting.category.clone(),
            value: setting.value.clone(),
            choices: setting
                .choices
                .iter()
                .map(|choice| Choice {
                    id: choice.id.clone(),
                    name: choice.name.clone(),
                    description: choice.description.clone(),
                    group: choice.group.clone(),
                })
                .collect(),
        })
        .collect()
}

/// Sets the setting `id` of the session to `value` with `set`, unless the
/// session, whose settings stand as `settings` says, has no such setting,
/// does not offer the value, or is set to it already, and tells the window
/// how the settings stand then, or why the agent did not set it.
async fn set_setting(
    set: impl AsyncFnOnce(&str, &str) -> anyhow::Result<Vec<cox_acp::Setting>>,
    settings: &Mutex<Vec<cox_acp::Setting>>,
    id: &str,
    value: &str,
    events: &UnboundedSender<AgentEvent>,
) {
    let name = {
        let settings = settings.lock().expect("no one panics holding it");
        let Some(setting) = settings.iter().find(|setting| setting.id == id) else {
            tracing::debug!("the session has no setting {id} to set");
            return;
        };
        if setting.value == value || !setting.choices.iter().any(|choice| choice.id == value) {
            return;
        }
        setting.name.clone()
    };
    let event = match set(id, value).await {
        Ok(set) => {
            let shown = shown_settings(&set);
            *settings.lock().expect("no one panics holding it") = set;
            AgentEvent::Settings(shown)
        }
        Err(error) => AgentEvent::SettingFailed {
            name,
            error: format!("{error:#}"),
        },
    };
    let _ = events.send(event);
}

/// The blocks of `prompt`, a line for the agent: its text, what the
/// human handed the chat with it, and the files it points to, in
/// `folder`.
fn line(prompt: Prompt, folder: &Path) -> impl Iterator<Item = Block> + use<> {
    let Prompt { text, attached } = prompt;
    let files = mention::files(&text, folder);
    std::iter::once(Block::Text(text))
        .chain(attached.into_iter().flat_map(handed_blocks))
        .chain(files.into_iter().map(Block::File))
}

/// Sends `prompt`, and passes on what the agent sends until the turn
/// ends, with the turns that start for the lines steered into it that
/// the agent did not take, as when it was ending (see [`Command::Steer`]),
/// which the turn ends with.
async fn take_turn(
    session: &mut Session,
    prompt: impl IntoIterator<Item = Block>,
    policy: &Policy,
    settings: &Mutex<Vec<cox_acp::Setting>>,
    commands: &mut Commands,
    events: &UnboundedSender<AgentEvent>,
    folder: &Path,
) -> anyhow::Result<()> {
    let mut prompt: Vec<Block> = prompt.into_iter().collect();
    let mut failed = None;
    loop {
        let Some(Ended { how, later }) = take_one_turn(
            session,
            prompt.clone(),
            policy,
            settings,
            commands,
            events,
            folder,
        )
        .await?
        else {
            return Ok(());
        };
        let login = matches!(&how, Err(error) if cox_acp::needs_login(error));
        let went_on = matches!(how, Ok(reason) if reason != StopReason::Cancelled);
        if !went_on && !login {
            for prompt in &later {
                let _ = events.send(AgentEvent::Note(format!(
                    "The agent did not get this line, as the turn stopped: {}",
                    prompt.text
                )));
            }
        } else if !later.is_empty() {
            let later = later.into_iter().flat_map(|prompt| line(prompt, folder));
            if login {
                // A prompt the agent refused until the human logs in goes
                // again, with the lines steered into it.
                prompt.extend(later);
            } else {
                prompt = later.collect();
                continue;
            }
        }
        match how {
            Err(error) if cox_acp::needs_login(&error) => {
                // Once the human has logged in, the prompt goes again.
                let methods = session.login_methods().to_vec();
                match logged_in(&methods, failed.take(), commands, events).await {
                    Ok(how) => {
                        failed = session.log_in(how).await.err().map(|e| format!("{e:#}"));
                    }
                    Err(error) => {
                        let _ = events.send(AgentEvent::TurnFailed(format!("{error:#}")));
                        return Ok(());
                    }
                }
            }
            Err(error) => {
                let _ = events.send(AgentEvent::TurnFailed(format!("{error:#}")));
                return Ok(());
            }
            Ok(reason) => {
                let _ = events.send(AgentEvent::TurnEnded(reason));
                return Ok(());
            }
        }
    }
}

/// How a turn ended, with the lines steered into it that the agent did
/// not take, to go once it has.
struct Ended {
    how: anyhow::Result<StopReason>,
    later: Vec<Prompt>,
}

/// Sends `prompt` and passes on what the agent sends until the turn ends,
/// and answers how it ended, or `None` when the window is gone. A line
/// steered into it goes to the agent as it runs, for an agent that steers,
/// and else waits for it to end; the files it points to are in `folder`.
async fn take_one_turn(
    session: &mut Session,
    prompt: Vec<Block>,
    policy: &Policy,
    settings: &Mutex<Vec<cox_acp::Setting>>,
    commands: &mut Commands,
    events: &UnboundedSender<AgentEvent>,
    folder: &Path,
) -> anyhow::Result<Option<Ended>> {
    let send = |event| {
        let _ = events.send(event);
    };
    let steers = session.steers();
    let mut turn = session.send(prompt);
    // Whether the human has asked to stop the turn.
    let mut cancelled = false;
    // The lines steered into the turn that wait for it to end.
    let mut later = Vec::new();
    loop {
        let update = tokio::select! {
            update = turn.next() => update,
            command = commands.recv() => {
                match command {
                    Some(Command::Cancel) if !cancelled => {
                        cancelled = true;
                        turn.cancel()?;
                    }
                    Some(Command::Set { id, value }) => {
                        let set = async |id: &str, value: &str| turn.set(id, value).await;
                        set_setting(set, settings, &id, &value, events).await;
                    }
                    Some(Command::Steer(prompt)) => {
                        let now = steers && !cancelled;
                        steer(&turn, prompt, now, folder, &mut later, events).await;
                    }
                    // The window sends no prompt while a turn runs, but
                    // steers it, no answer while no permission request
                    // waits, and one cancel at most.
                    Some(_) => {}
                    // The window is gone, and the agent goes with it.
                    None => return Ok(None),
                }
                continue;
            }
        };
        let Some(update) = update else {
            break;
        };
        match update {
            Update::ReplyText(text) => send(AgentEvent::ReplyText(text)),
            Update::ThoughtText(text) => send(AgentEvent::ThoughtText(text)),
            Update::Plan(entries) => {
                send(AgentEvent::Plan(entries.into_iter().map(step).collect()));
            }
            Update::ToolCall(call) => send(AgentEvent::ToolCall {
                id: call.id,
                title: call.title,
                content: call.content.map(shown),
                result: call.result.map(ended),
            }),
            // Once the human has asked to stop, the agent is told no
            // without asking them.
            Update::PermissionRequest(request) if cancelled => {
                request.answer(PermissionAnswer::Cancel)?;
            }
            // The setup's policy answers in the human's place, when it has
            // an answer and the agent offers it.
            Update::PermissionRequest(request) if decided(policy, &request).is_some() => {
                let chosen = decided(policy, &request).unwrap_or_default();
                send(AgentEvent::PermissionDecided {
                    id: request.tool_call_id(),
                    title: request.title().map(str::to_owned),
                    content: request.content().map(shown),
                    options: request.options().iter().map(|o| o.name.clone()).collect(),
                    chosen,
                });
                request.answer(PermissionAnswer::Select(chosen))?;
            }
            Update::PermissionRequest(request) => {
                send(AgentEvent::PermissionRequest {
                    id: request.tool_call_id(),
                    title: request.title().map(str::to_owned),
                    content: request.content().map(shown),
                    options: request.options().iter().map(|o| o.name.clone()).collect(),
                });
                let answer = loop {
                    match commands.recv().await {
                        Some(Command::Answer(answer)) => break answer,
                        Some(Command::Cancel) => {
                            cancelled = true;
                            turn.cancel()?;
                            break PermissionAnswer::Cancel;
                        }
                        Some(Command::Set { id, value }) => {
                            let set = async |id: &str, value: &str| turn.set(id, value).await;
                            set_setting(set, settings, &id, &value, events).await;
                        }
                        // A line steered into the turn goes to the agent
                        // as one would while it runs: the agent takes it
                        // once the human has answered.
                        Some(Command::Steer(prompt)) => {
                            steer(&turn, prompt, steers, folder, &mut later, events).await;
                        }
                        // The window sends no prompt while a turn runs, and
                        // no login while the agent does not ask for one.
                        Some(Command::Prompt(_) | Command::LogIn(_)) => {}
                        None => break PermissionAnswer::Cancel,
                    }
                };
                request.answer(answer)?;
            }
        }
    }
    Ok(Some(Ended {
        how: turn.end().await,
        later,
    }))
}

/// Takes `prompt`, a line the human steered into `turn`, and steers it
/// into the turn `now`, the files it points to being in `folder`, or
/// else has it go `later`, once the turn has ended, as a line the agent
/// did not take does.
async fn steer(
    turn: &cox_acp::Turn<'_>,
    prompt: Prompt,
    now: bool,
    folder: &Path,
    later: &mut Vec<Prompt>,
    events: &UnboundedSender<AgentEvent>,
) {
    let _ = events.send(AgentEvent::Steered);
    if !now {
        later.push(prompt);
        return;
    }
    match turn.steer(line(prompt.clone(), folder)).await {
        Ok(Steered::Taken) => {}
        Ok(Steered::TooLate) => later.push(prompt),
        Err(error) => {
            tracing::warn!("the agent did not take a line into its turn: {error:#}");
            let _ = events.send(AgentEvent::Note(format!(
                "The agent did not take the line into its turn ({error:#}): it goes once the turn ends."
            )));
            later.push(prompt);
        }
    }
}

/// What a tool call shows, made ready for the window here, on the agent
/// side, where working out a large change does not hold up the window.
/// The option of `request` that `policy` chooses, if it has an answer to
/// such a request, and the agent offers an option that means it: to allow
/// once, or to reject once. The policy never answers for always: it is
/// the setup's to keep, not the agent's.
fn decided(policy: &Policy, request: &cox_acp::PermissionRequest) -> Option<usize> {
    let asked = Asked {
        title: request.title().unwrap_or_default(),
        kind: request.kind(),
        locations: request.locations(),
    };
    let kind = match policy.decide(asked)? {
        Decision::Allow => PermissionKind::AllowOnce,
        Decision::Reject => PermissionKind::RejectOnce,
    };
    request
        .options()
        .iter()
        .position(|option| option.kind == kind)
}

/// The most of the conversation so far that a new session is told, in
/// characters: past it, the start is left out.
const RETOLD_LENGTH: usize = 100_000;

/// What the agent is told of a goal, a name, what the project is or a
/// project it proposed, and what the human did with it.
fn told_of_proposal(entry: &Entry) -> Option<String> {
    match entry {
        Entry::Proposal { goal, answer, .. } => {
            let answer = match answer {
                Some(Answer::Set) => "the human set it",
                Some(Answer::Changed) => "the human changed it, and set that",
                Some(Answer::Dismissed) => "the human let it be",
                None => "the human has not answered",
            };
            Some(format!("[you proposed the goal: {goal}; {answer}]"))
        }
        Entry::NameProposal { name, answer, .. } => {
            let answer = match answer {
                Some(Answer::Set | Answer::Changed) => "the human renamed the project so",
                Some(Answer::Dismissed) => "the human let it be",
                None => "the human has not answered",
            };
            Some(format!(
                "[you proposed the project's name: {name}; {answer}]"
            ))
        }
        Entry::AboutProposal { about, answer, .. } => {
            let answer = match answer {
                Some(Answer::Set | Answer::Changed) => "the human set it",
                Some(Answer::Dismissed) => "the human let it be",
                None => "the human has not answered",
            };
            Some(format!(
                "[you proposed what the project is: {about}; {answer}]"
            ))
        }
        Entry::ProjectProposal { name, answer, .. } => {
            let answer = match answer {
                Some(Answer::Set | Answer::Changed) => {
                    "the human made it, and the chat moved into it"
                }
                Some(Answer::Dismissed) => "the human let it be",
                None => "the human has not answered",
            };
            Some(format!(
                "[you proposed making the chat the project {name}; {answer}]"
            ))
        }
        _ => None,
    }
}

/// The conversation so far, `entries`, as an agent is told it: what
/// the human said, and Cox in their place, what the agent replied, and the
/// tool calls it made, by their titles, and where the human switched the
/// chat from one agent to another, which the answer says it holds. Cox's
/// notes are left out. A chat where nothing was said has nothing to tell.
/// Past [`RETOLD_LENGTH`], the start is left out.
fn told_so_far(entries: &[Entry]) -> Option<(String, bool)> {
    let (mut told, switched) = said(entries)?;
    let length = told.chars().count();
    if length > RETOLD_LENGTH {
        let kept: String = told.chars().skip(length - RETOLD_LENGTH).collect();
        told = format!("[… the start is left out]\n{kept}");
    }
    Some((told, switched))
}

/// What was said in `entries`, whole, as [`told_so_far`] tells it, and
/// whether the human switched the chat from one agent to another.
fn said(entries: &[Entry]) -> Option<(String, bool)> {
    let switched = entries
        .iter()
        .any(|entry| matches!(entry, Entry::Switched { .. }));
    let said = entries.iter().any(|entry| {
        !matches!(
            entry,
            Entry::Note(_)
                | Entry::Thought(_)
                | Entry::Plan(_)
                | Entry::PlanChanged { .. }
                | Entry::Switched { .. }
                | Entry::Login { .. }
                | Entry::Keep { .. }
                | Entry::Commits(_)
                | Entry::Checks(_)
                | Entry::Event { .. }
        )
    });
    if !said {
        return None;
    }
    let told: Vec<String> = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Human(text) => Some(format!("[human]\n{text}")),
            Entry::Attached(attached) => Some(
                attached
                    .iter()
                    .map(|attachment| {
                        format!(
                            "[the human handed you, from {}]\n{}",
                            attachment.uri, attachment.text
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            Entry::Cox(text) => Some(format!("[cox]\n{text}")),
            Entry::Merge => Some("[the human asked you to merge what you made]".to_owned()),
            Entry::Media { from, caption, .. } => Some(match caption {
                Some(caption) => format!("[shown to the human: {from}, {caption}]"),
                None => format!("[shown to the human: {from}]"),
            }),
            Entry::Shown { path, .. } => Some(format!("[you showed the human {path}]")),
            Entry::Commits(commits) => Some(format!(
                "[you committed: {}]",
                commits
                    .iter()
                    .map(|commit| format!("{} {}", commit.short(), commit.subject))
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            Entry::Reply(text) => Some(format!("[you]\n{text}")),
            Entry::ToolCall { title, result, .. } => {
                let result = result.map_or_else(|| "not ended".to_owned(), |r| format!("{r:?}"));
                Some(format!("[your tool call: {title} … {result}]"))
            }
            Entry::Permission {
                title,
                options,
                chosen,
                by_setup,
                ..
            } => {
                let chosen = chosen
                    .and_then(|chosen| options.get(chosen))
                    .map_or("nothing", String::as_str);
                let who = if *by_setup {
                    "Cox, as the chat is set up,"
                } else {
                    "the human"
                };
                Some(format!(
                    "[you asked permission for: {title}; {who} chose: {chosen}]"
                ))
            }
            Entry::Proposal { .. }
            | Entry::NameProposal { .. }
            | Entry::AboutProposal { .. }
            | Entry::ProjectProposal { .. } => told_of_proposal(entry),
            Entry::Switched { from, to } => Some(format!(
                "[the human switched the chat from the agent {from} to the agent {to}]"
            )),
            // What the agent thought and planned is its own, and what came
            // of it is told already; how the human logged in is theirs,
            // and so is whether to keep what the chat learned, which the
            // agent is asked in a line of its own when they choose to.
            Entry::Thought(_)
            | Entry::Plan(_)
            | Entry::PlanChanged { .. }
            | Entry::Note(_)
            | Entry::Login { .. }
            | Entry::Keep { .. }
            | Entry::Checks(_)
            | Entry::Event { .. } => None,
        })
        .collect();
    Some((told.join("\n\n"), switched))
}

/// What was said in a sketch, `entries`, whole, as it is kept once the
/// sketch ends (see [`crate::raw`]), where `[you]` stands for the
/// sketch's agent; or nothing, when nothing was said in it.
#[must_use]
pub fn said_in_sketch(entries: &[Entry]) -> Option<String> {
    said(entries).map(|(said, _)| said)
}

/// The most of another chat's conversation an agent reads with
/// `read_chat`, in characters: past it, the start is left out.
const READ_LENGTH: usize = 40_000;

/// What an agent reading another chat of its project is told of what the
/// chat `name`, kept as `number`, showed, `entries`, from its end; or
/// nothing, when nothing was said in it.
#[must_use]
pub fn read_out(number: i64, name: &str, entries: &[Entry]) -> Option<String> {
    let (mut told, _) = told_so_far(entries)?;
    let length = told.chars().count();
    if length > READ_LENGTH {
        let kept: String = told.chars().skip(length - READ_LENGTH).collect();
        told = format!("[… the start is left out]\n{kept}");
    }
    Some(format!(
        "Chat {number}, \"{name}\", as Cox kept it, where [you] stands for that chat's agent, \
         not you:\n\n{told}"
    ))
}

/// What the agent of a sketch opened beside a chat is told of what the
/// chat talked through so far, `entries`, ahead of what it tries; or
/// nothing, when nothing was said in the chat.
#[must_use]
pub fn talked_through(entries: &[Entry]) -> Option<String> {
    let (told, _) = told_so_far(entries)?;
    Some(format!(
        "This sketch was opened beside a chat, which goes on in the project. This is what \
         that chat talked through so far, as Cox kept it, where [you] stands for that chat's \
         agent:\n\n{told}"
    ))
}

/// What a new session of a chat's agent is told of the conversation so
/// far, `entries`, when the agent does not go on with a session of the
/// chat's own, which is `lost` if the agent could not go on with it; see
/// [`told_so_far`].
#[must_use]
pub fn retold(entries: &[Entry], lost: bool) -> Option<String> {
    let (told, switched) = told_so_far(entries)?;
    let from = if switched {
        "This chat goes on from an earlier session, of yours or of another agent: where the \
         human switched the chat from one agent to another, it says so, and [you] stands for \
         the agent the chat had then."
    } else if lost {
        "This chat goes on from an earlier session of yours, which could not be resumed."
    } else {
        "This chat goes on from an earlier session of yours."
    };
    Some(format!(
        "{from} This is the conversation so far, as Cox kept it; go on from its end.\n\n{told}"
    ))
}

fn shown(content: Vec<ToolCallContent>) -> Vec<Content> {
    content
        .into_iter()
        .map(|content| match content {
            ToolCallContent::Text(text) => Content::Text(text),
            ToolCallContent::Diff {
                path,
                old_text,
                new_text,
            } => Content::Diff {
                path: path.display().to_string(),
                lines: cox_core::diff::lines(old_text.as_deref(), &new_text),
            },
        })
        .collect()
}

/// A step of the agent's plan, as the conversation keeps it.
fn step(entry: PlanEntry) -> Step {
    Step {
        text: entry.text,
        state: match entry.state {
            PlanState::Pending => StepState::Pending,
            PlanState::InProgress => StepState::InProgress,
            PlanState::Completed => StepState::Completed,
        },
    }
}

/// How a tool call ended, as the conversation keeps it.
fn ended(result: cox_acp::ToolCallResult) -> ToolCallResult {
    match result {
        cox_acp::ToolCallResult::Completed => ToolCallResult::Completed,
        cox_acp::ToolCallResult::Failed => ToolCallResult::Failed,
        cox_acp::ToolCallResult::Cancelled => ToolCallResult::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn a_line_runs_a_command_the_agent_named() {
        let commands = ["compact".to_owned(), "review".to_owned()];
        assert!(runs_command("/compact", &commands));
        assert!(runs_command("/review the last commit", &commands));
        assert!(!runs_command("/compacting", &commands));
        assert!(!runs_command("/usr/bin is gone", &commands));
        assert!(!runs_command(" /compact", &commands));
        assert!(!runs_command("compact", &commands));
    }

    #[test]
    fn a_new_session_is_told_the_conversation_so_far() {
        let entries = [
            Entry::Note("Instructions: ~/cox/COX.md".to_owned()),
            Entry::Human("edit".to_owned()),
            Entry::ToolCall {
                id: "call-1".to_owned(),
                title: "Edit x.txt".to_owned(),
                content: Vec::new(),
                result: Some(ToolCallResult::Completed),
            },
            Entry::Permission {
                id: "call-1".to_owned(),
                title: "Edit x.txt".to_owned(),
                content: Vec::new(),
                options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
                chosen: Some(0),
                by_setup: false,
                gathered: Vec::new(),
            },
            Entry::Permission {
                id: "call-2".to_owned(),
                title: "Write y.txt".to_owned(),
                content: Vec::new(),
                options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
                chosen: Some(1),
                by_setup: true,
                gathered: Vec::new(),
            },
            Entry::Reply("Done".to_owned()),
            Entry::Proposal {
                goal: "$ make".to_owned(),
                why: Some("make builds it".to_owned()),
                answer: Some(Answer::Changed),
            },
            Entry::Note("Stopped.".to_owned()),
        ];
        let told = retold(&entries, true).unwrap();
        assert!(
            told.starts_with("This chat goes on from an earlier session of yours, which could not")
        );
        assert!(
            told.ends_with(
                "\n\n[human]\nedit\n\n\
                 [your tool call: Edit x.txt … Completed]\n\n\
                 [you asked permission for: Edit x.txt; the human chose: Allow once]\n\n\
                 [you asked permission for: Write y.txt; Cox, as the chat is set up, chose: Reject once]\n\n\
                 [you]\nDone\n\n\
                 [you proposed the goal: $ make; the human changed it, and set that]"
            ),
            "{told}"
        );
        assert!(!told.contains("EndTurn"), "{told}");
    }

    #[test]
    fn another_chat_is_read_out_from_its_end() {
        assert_eq!(
            read_out(3, "Chat 3", &[Entry::Note("Opened".to_owned())]),
            None
        );
        let read = read_out(
            3,
            "Cart",
            &[
                Entry::Human("Add a cart.".to_owned()),
                Entry::Reply("Added it.".to_owned()),
            ],
        )
        .unwrap();
        assert!(
            read.starts_with("Chat 3, \"Cart\", as Cox kept it"),
            "{read}"
        );
        assert!(
            read.ends_with("[human]\nAdd a cart.\n\n[you]\nAdded it."),
            "{read}"
        );
        let long = "あ".repeat(READ_LENGTH);
        let read = read_out(
            3,
            "Cart",
            &[Entry::Human("first".to_owned()), Entry::Reply(long.clone())],
        )
        .unwrap();
        assert!(read.ends_with(&format!("[… the start is left out]\n{long}")));
        assert!(!read.contains("first"));
    }

    #[test]
    fn a_long_conversation_is_told_from_its_end() {
        let long = "あ".repeat(RETOLD_LENGTH);
        let told = retold(
            &[Entry::Human("first".to_owned()), Entry::Reply(long.clone())],
            false,
        )
        .unwrap();
        assert!(told.starts_with("This chat goes on from an earlier session of yours. This"));
        assert!(told.ends_with(&format!("[… the start is left out]\n{long}")));
        assert!(!told.contains("first"));
    }

    #[test]
    fn a_chat_switched_to_another_agent_tells_it_so() {
        let entries = [
            Entry::Human("greet".to_owned()),
            Entry::Reply("Hello".to_owned()),
            Entry::Switched {
                from: "Claude Agent".to_owned(),
                to: "Codex".to_owned(),
            },
        ];
        let told = retold(&entries, false).unwrap();
        assert!(
            told.starts_with("This chat goes on from an earlier session, of yours or of another"),
            "{told}"
        );
        assert!(
            told.ends_with(
                "[you]\nHello\n\n\
                 [the human switched the chat from the agent Claude Agent to the agent Codex]"
            ),
            "{told}"
        );
    }

    #[test]
    fn a_chat_where_nothing_was_said_starts_anew() {
        let entries = [
            Entry::Note("Instructions: COX.md".to_owned()),
            Entry::Switched {
                from: "Claude Agent".to_owned(),
                to: "Codex".to_owned(),
            },
        ];
        assert_eq!(retold(&entries, false), None);
        let start = Start::Again {
            session: None,
            instructions: None,
            setup: None,
            earlier: entries.to_vec(),
        };
        assert_eq!(opened(start, false), (Opened::New, Vec::new()));
    }

    /// Whether process `pid` is still running: there, and not a zombie.
    fn running(pid: &str) -> bool {
        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .expect("ps runs");
        let state = String::from_utf8_lossy(&output.stdout);
        output.status.success() && !state.trim_start().starts_with('Z')
    }

    /// Waits up to five seconds for `done`.
    fn wait_until(mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            if Instant::now() > deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
        true
    }

    #[test]
    fn dropping_the_agent_ends_every_process_it_started() {
        let file = std::env::temp_dir().join(format!("cox-agent-{}", std::process::id()));
        // Like `npx`, a launcher whose child does the work. This agent never
        // answers, so only stopping it ends it.
        let script = format!("sleep 60 & echo $! > {}; wait", file.display());
        let agent = spawn(
            move |_| cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script]),
            std::env::temp_dir(),
            Vec::new(),
            Start::New {
                instructions: None,
                setup: None,
            },
        );
        let mut pid = String::new();
        assert!(wait_until(|| {
            pid = std::fs::read_to_string(&file).unwrap_or_default();
            pid.ends_with('\n')
        }));
        let pid = pid.trim();
        assert!(running(pid));

        drop(agent);
        let _ = std::fs::remove_file(&file);
        assert!(
            wait_until(|| !running(pid)),
            "the agent's child outlived it"
        );
    }

    #[test]
    fn the_agent_gets_none_of_coxs_files() {
        let file = std::env::temp_dir().join(format!("cox-agent-files-{}", std::process::id()));
        // A descriptor that stays open across `exec`, like CEF's port.
        let open = std::fs::File::open("/dev/null").unwrap();
        // SAFETY: `open` is a valid descriptor; the copy is closed below.
        let inherited = unsafe { libc::dup(std::os::fd::AsRawFd::as_raw_fd(&open)) };
        assert!(inherited > 2);
        // Looked for before the output is redirected: `dash` keeps a copy
        // of the descriptor a redirect replaces at 10 or above, where the
        // one looked for is too once other tests hold many open.
        let script = format!(
            "seen=not; [ -e /dev/fd/{inherited} ] && seen=has; echo $seen > {}",
            file.display()
        );
        let agent = spawn(
            move |_| cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script]),
            std::env::temp_dir(),
            Vec::new(),
            Start::New {
                instructions: None,
                setup: None,
            },
        );
        let mut seen = String::new();
        assert!(wait_until(|| {
            seen = std::fs::read_to_string(&file).unwrap_or_default();
            seen.ends_with('\n')
        }));
        drop(agent);
        // SAFETY: `inherited` is open, and nothing else uses it.
        unsafe { libc::close(inherited) };
        let _ = std::fs::remove_file(&file);
        assert_eq!(seen, "not\n");
    }
}
