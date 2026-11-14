//! An ACP agent for tests, connected to Cox in memory. It replies to each
//! prompt with `echo: <prompt>`, a word at a time. When the prompt mentions
//! "permission", it first announces a tool call to edit `x.txt`, showing the
//! change, asks permission to make it with the change shown again, the way
//! `claude-agent-acp` does, and then reports it done. When the
//! prompt mentions "fail", it answers with an error instead. When the prompt
//! mentions "tool", it first runs `ls` as a tool call that completes, showing
//! what it printed, and when it mentions "twice" too, runs it again. When the
//! prompt mentions "think", it first thinks [`THOUGHT`], and when it mentions
//! "plan", it first sends a plan of two steps, the first under way. When it
//! mentions "title", it first titles its session [`TITLE`], and when it
//! mentions "slow", it waits before anything until the test lets it go on
//! ([`FakeAgent::go_on`]). When the prompt mentions "background", it
//! replies [`BACKGROUND_STARTED`] and ends the turn, and once the test lets
//! it go on, goes on by itself, with no prompt, as `claude-agent-acp` does
//! once a task left running in the background ends: it asks permission to
//! edit `x.txt` as above when the prompt mentions "ask" too, and replies
//! [`BACKGROUND_FINISHED`]. When the prompt mentions "wait", it replies
//! `waiting` and goes on until Cox cancels the turn, or steers a line into
//! it, which it echoes, ending the turn. A turn cancelled, or whose
//! permission request Cox answers as cancelled, ends as cancelled. A
//! prompt mentions a word when one of its words starts with it: "slowly"
//! mentions "slow", and "task" does not mention "ask". Its sessions have
//! two settings, [`MODEL`] and [`MODE`], which Cox sets as it asks, and
//! when a prompt mentions "switch", the agent sets its mode to `plan` on
//! its own. It goes on with a session it opened before as [`Resumes`]
//! says, and asks Cox to log in as [`LogsIn`] says. It records what Cox
//! asked of it and answered so tests can check what Cox did, and what it
//! did itself, as [`Event`]s, for a test to wait for.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthMethod, AuthMethodAgent, AuthMethodTerminal, AuthenticateRequest,
    AuthenticateResponse, AvailableCommand, AvailableCommandsUpdate, CancelNotification,
    ConfigOptionUpdate, ContentBlock, ContentChunk, Diff, EmbeddedResourceResource,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse, McpServer,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, Plan, PlanEntry,
    PlanEntryPriority, PlanEntryStatus, PromptCapabilities, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, ResumeSessionRequest,
    ResumeSessionResponse, SessionAdditionalDirectoriesCapabilities, SessionCapabilities,
    SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption, SessionId,
    SessionInfoUpdate, SessionNotification, SessionResumeCapabilities, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, StopReason, TextContent,
    ToolCall, ToolCallContent, ToolCallLocation, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{
    Agent, Channel, Client, ConnectionTo, Error, Responder, Result, UntypedMessage,
};
use coxd::agent::instructions;
use tokio::sync::{Semaphore, oneshot};

/// How the agent goes on with a session it opened before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Resumes {
    /// It cannot.
    #[default]
    Never,
    /// It loads the session, telling Cox what the session said before.
    Load,
    /// It resumes the session, telling nothing.
    Resume,
    /// It says it can resume a session, and refuses to.
    Refuse,
}

/// When the agent asks Cox to log in, until it has: never, before it
/// opens a session, or as it takes a turn, as `claude-agent-acp` does.
/// It offers to log in with [`KEY`], which works, with [`WRONG_KEY`],
/// which fails, and in a terminal, [`IN_A_TERMINAL`], whose command has
/// the agent logged in once the file it names is there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LogsIn {
    #[default]
    Never,
    AtSession(PathBuf),
    AtPrompt(PathBuf),
}

/// The ids of the ways the agent offers to log in.
pub const KEY: &str = "key";
pub const WRONG_KEY: &str = "wrong-key";
pub const IN_A_TERMINAL: &str = "terminal";

/// What the agent says when logging in with [`WRONG_KEY`] fails.
pub const WRONG: &str = "the key is wrong";

impl LogsIn {
    /// The file whose being there has the agent logged in.
    fn file(&self) -> Option<&PathBuf> {
        match self {
            Self::Never => None,
            Self::AtSession(file) | Self::AtPrompt(file) => Some(file),
        }
    }

    /// The ways the agent offers to log in.
    fn methods(&self) -> Vec<AuthMethod> {
        let Some(file) = self.file() else {
            return Vec::new();
        };
        vec![
            AuthMethod::Agent(AuthMethodAgent::new(KEY, "API key").description("Use the key")),
            AuthMethod::Agent(AuthMethodAgent::new(WRONG_KEY, "Old key")),
            AuthMethod::Terminal(
                AuthMethodTerminal::new(IN_A_TERMINAL, "Terminal")
                    .args(vec![file.display().to_string()]),
            ),
        ]
    }

    /// Whether the agent, logged in with `methods` so far, still asks to
    /// log in at `session` or not.
    fn asks(&self, session: bool, methods: &[String]) -> bool {
        let at = match self {
            Self::Never => return false,
            Self::AtSession(_) => session,
            Self::AtPrompt(_) => !session,
        };
        at && !methods.iter().any(|method| method == KEY)
            && !self.file().is_some_and(|file| file.exists())
    }
}

#[derive(Default)]
pub struct FakeAgent {
    resumes: Resumes,
    logs_in: LogsIn,
    /// The ways Cox logged in with, in order.
    logged_in: Arc<Mutex<Vec<String>>>,
    /// The sessions Cox asked to go on with, in order.
    asked_to_resume: Arc<Mutex<Vec<String>>>,
    sessions_opened: Arc<AtomicUsize>,
    cwd: Arc<Mutex<Option<PathBuf>>>,
    mcp_servers: Arc<Mutex<Vec<McpServer>>>,
    prompts: Arc<Mutex<Prompts>>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
    turns: Arc<Turns>,
    settings: Arc<Mutex<Settings>>,
}

/// What the agent did, in order, for a test to wait for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// It took a prompt, saying this.
    Prompted(String),
    /// It waits at a turn for Cox to cancel it, or steer a line into it.
    Waiting,
    /// It waits for the test to let it go on.
    Held,
    /// It asked permission.
    Asked,
    /// It ended a turn, as it said.
    Ended(StopReason),
    /// It failed a turn, with an error.
    Failed,
    /// It went on by itself after a turn and is done.
    WentOn,
}

/// What Cox sent the agent: the text blocks of each prompt, without how
/// Cox tells a new session to work with its tools, which nearly every
/// test leaves be and which is kept apart.
#[derive(Debug, Default)]
struct Prompts {
    sent: Vec<Vec<String>>,
    cox: Vec<String>,
}

/// The settings of the agent's sessions: their values, and each setting
/// Cox set, in order, with what it set it to.
#[derive(Debug)]
struct Settings {
    model: String,
    mode: String,
    set: Vec<(String, String)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            model: "large".to_owned(),
            mode: "ask".to_owned(),
            set: Vec::new(),
        }
    }
}

/// The ids of the agent's settings: the model, `large` or `small`, and the
/// mode, `ask` or `plan`.
pub const MODEL: &str = "model";
/// The name of the one command the agent runs.
pub const COMMAND: &str = "compact";
pub const MODE: &str = "mode";

impl Settings {
    /// Sets what `request` asks, and answers all the settings then.
    fn set(&mut self, request: &SetSessionConfigOptionRequest) -> Result<Vec<SessionConfigOption>> {
        let value = request
            .value
            .as_value_id()
            .map(|value| value.0.to_string())
            .unwrap_or_default();
        match &*request.config_id.0 {
            MODEL => value.clone_into(&mut self.model),
            MODE => value.clone_into(&mut self.mode),
            _ => return Err(Error::new(-32602, "no such setting")),
        }
        self.set.push((request.config_id.0.to_string(), value));
        Ok(self.options())
    }

    /// The settings as ACP says them.
    fn options(&self) -> Vec<SessionConfigOption> {
        let choice = |id: &str, name: &str| SessionConfigSelectOption::new(id.to_owned(), name);
        vec![
            SessionConfigOption::select(
                MODEL,
                "Model",
                self.model.clone(),
                vec![choice("large", "Large"), choice("small", "Small")],
            )
            .category(SessionConfigOptionCategory::Model),
            SessionConfigOption::select(
                MODE,
                "Mode",
                self.mode.clone(),
                vec![choice("ask", "Ask"), choice("plan", "Plan")],
            )
            .category(SessionConfigOptionCategory::Mode),
        ]
    }
}

/// Sets the mode to `plan` on the agent's own, and tells Cox.
fn switch(
    cx: &ConnectionTo<Client>,
    session: &SessionId,
    settings: &Mutex<Settings>,
) -> Result<()> {
    let options = {
        let mut settings = settings.lock().unwrap();
        "plan".clone_into(&mut settings.mode);
        settings.options()
    };
    cx.send_notification(SessionNotification::new(
        session.clone(),
        SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(options)),
    ))
}

/// How the agent's turns go: what they did, the cancels Cox has sent,
/// the turn that runs, and the times the test let the agent go on.
struct Turns {
    events: Mutex<Vec<Event>>,
    cancels: AtomicUsize,
    running: Mutex<Option<Running>>,
    /// A permit for each time the test let the agent go on.
    gate: Semaphore,
}

/// The turn that runs: whether Cox cancelled it, and what wakes it while
/// it waits to be cancelled or steered.
#[derive(Default)]
struct Running {
    cancelled: bool,
    waiting: Option<oneshot::Sender<Woken>>,
}

impl Default for Turns {
    fn default() -> Self {
        Self {
            events: Mutex::default(),
            cancels: AtomicUsize::default(),
            running: Mutex::default(),
            gate: Semaphore::new(0),
        }
    }
}

/// What ends a turn that waits.
enum Woken {
    Cancelled,
    Steered(String),
}

impl Turns {
    fn note(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }

    /// Waits until the test lets the agent go on.
    async fn held(&self) {
        self.note(Event::Held);
        self.gate
            .acquire()
            .await
            .expect("the gate stays open")
            .forget();
    }

    /// Starts a turn.
    fn start(&self) {
        *self.running.lock().unwrap() = Some(Running::default());
    }

    /// Ends the turn that runs, as `ended` says.
    fn end(&self, ended: Event) {
        *self.running.lock().unwrap() = None;
        self.note(ended);
    }

    /// Waits, as a turn that is cancelled or steered, until Cox does one
    /// of them, once `waits` has told Cox the turn waits: at once when
    /// Cox cancelled the turn already.
    async fn wait(&self, waits: impl FnOnce() -> Result<()>) -> Result<Woken> {
        let (wake, woken) = oneshot::channel();
        {
            let mut running = self.running.lock().unwrap();
            let running = running.get_or_insert_with(Running::default);
            if running.cancelled {
                return Ok(Woken::Cancelled);
            }
            running.waiting = Some(wake);
        }
        waits()?;
        self.note(Event::Waiting);
        Ok(woken.await.unwrap_or(Woken::Cancelled))
    }

    /// Cancels the turn that runs, if one does, waking it if it waits.
    fn cancel(&self) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        if let Some(running) = self.running.lock().unwrap().as_mut() {
            running.cancelled = true;
            if let Some(waiting) = running.waiting.take() {
                let _ = waiting.send(Woken::Cancelled);
            }
        }
    }

    /// Steers `line` into the turn that waits, if one does, answering
    /// whether one did.
    fn steer(&self, line: String) -> bool {
        let waiting = self
            .running
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|running| running.waiting.take());
        waiting.is_some_and(|waiting| waiting.send(Woken::Steered(line)).is_ok())
    }
}

/// Whether `text` mentions `word`: one of its words starts with it.
fn mentions(text: &str, word: &str) -> bool {
    text.split(|letter: char| !letter.is_alphanumeric())
        .any(|said| said.starts_with(word))
}

impl FakeAgent {
    /// An agent that goes on with a session it opened before as `resumes`
    /// says.
    pub fn resuming(resumes: Resumes) -> Self {
        Self {
            resumes,
            ..Self::default()
        }
    }

    /// An agent that asks Cox to log in as `logs_in` says.
    pub fn logging_in(logs_in: LogsIn) -> Self {
        Self {
            logs_in,
            ..Self::default()
        }
    }

    /// The ways Cox logged in with, in order.
    pub fn logged_in(&self) -> Vec<String> {
        self.logged_in.lock().unwrap().clone()
    }

    /// Starts serving and returns the end of the connection to give Cox.
    pub fn connect(&self) -> Channel {
        let (cox_end, agent_end) = Channel::duplex();
        let login = Login {
            logs_in: self.logs_in.clone(),
            logged_in: Arc::clone(&self.logged_in),
        };
        tokio::spawn(serve(
            agent_end,
            login,
            self.resumes,
            Arc::clone(&self.asked_to_resume),
            Arc::clone(&self.sessions_opened),
            Arc::clone(&self.cwd),
            Arc::clone(&self.mcp_servers),
            Arc::clone(&self.prompts),
            Arc::clone(&self.answers),
            Arc::clone(&self.turns),
            Arc::clone(&self.settings),
        ));
        cox_end
    }

    /// Each setting Cox set, in order, with what it set it to.
    pub fn settings_set(&self) -> Vec<(String, String)> {
        self.settings.lock().unwrap().set.clone()
    }

    /// The sessions Cox asked to go on with, in order.
    pub fn resumed(&self) -> Vec<String> {
        self.asked_to_resume.lock().unwrap().clone()
    }

    /// How many sessions Cox has opened.
    pub fn sessions_opened(&self) -> usize {
        self.sessions_opened.load(Ordering::SeqCst)
    }

    /// The directory Cox opened the session in.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.cwd.lock().unwrap().clone()
    }

    /// The MCP servers Cox handed over when it opened the session.
    pub fn mcp_servers(&self) -> Vec<McpServer> {
        self.mcp_servers.lock().unwrap().clone()
    }

    /// The text of each prompt Cox has sent, in order.
    pub fn prompts(&self) -> Vec<String> {
        self.prompts
            .lock()
            .unwrap()
            .sent
            .iter()
            .map(|blocks| blocks.concat())
            .collect()
    }

    /// The text blocks of each prompt Cox has sent, in order.
    pub fn prompt_blocks(&self) -> Vec<Vec<String>> {
        self.prompts.lock().unwrap().sent.clone()
    }

    /// How Cox told each new session to work with its tools, which
    /// [`Self::prompts`] leaves out, in order.
    pub fn told_by_cox(&self) -> Vec<String> {
        self.prompts.lock().unwrap().cox.clone()
    }

    /// Cox's answer to each permission request, in order.
    pub fn answers(&self) -> Vec<RequestPermissionOutcome> {
        self.answers.lock().unwrap().clone()
    }

    /// How many times Cox has asked to cancel a turn.
    pub fn cancels(&self) -> usize {
        self.turns.cancels.load(Ordering::SeqCst)
    }

    /// What the agent did, in order.
    pub fn events(&self) -> Vec<Event> {
        self.turns.events.lock().unwrap().clone()
    }

    /// Whether the agent did `event`.
    pub fn did(&self, event: &Event) -> bool {
        self.turns.events.lock().unwrap().contains(event)
    }

    /// Lets the agent go on, once, from where it waits for the test to,
    /// now or the next time it does: a turn whose prompt mentions "slow",
    /// before anything, or the work it left running in the background.
    pub fn go_on(&self) {
        self.turns.gate.add_permits(1);
    }
}

/// How the agent asks Cox to log in, and the ways Cox logged in with.
#[derive(Clone)]
struct Login {
    logs_in: LogsIn,
    logged_in: Arc<Mutex<Vec<String>>>,
}

impl Login {
    /// Whether the agent asks to log in before it opens a session, at
    /// `session`, or before it takes a turn.
    fn asks(&self, session: bool) -> bool {
        self.logs_in.asks(session, &self.logged_in.lock().unwrap())
    }
}

/// What the agent does on its way to the reply, as the prompt asks: it
/// thinks, plans and runs tools.
fn work(cx: &ConnectionTo<Client>, session: &SessionId, text: &str) -> Result<()> {
    if mentions(text, "title") {
        cx.send_notification(SessionNotification::new(
            session.clone(),
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(TITLE)),
        ))?;
    }
    if mentions(text, "think") {
        think(cx, session)?;
    }
    if mentions(text, "plan") {
        plan(cx, session)?;
    }
    if mentions(text, "tool") {
        run_tool(cx, session, "call-2")?;
        if mentions(text, "twice") {
            run_tool(cx, session, "call-3")?;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "what the agent records, each once"
)]
#[expect(
    clippy::too_many_lines,
    reason = "a handler for each request the agent takes"
)]
async fn serve(
    transport: Channel,
    login: Login,
    resumes: Resumes,
    asked_to_resume: Arc<Mutex<Vec<String>>>,
    sessions_opened: Arc<AtomicUsize>,
    cwd: Arc<Mutex<Option<PathBuf>>>,
    mcp_servers: Arc<Mutex<Vec<McpServer>>>,
    prompts: Arc<Mutex<Prompts>>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
    turns: Arc<Turns>,
    settings: Arc<Mutex<Settings>>,
) -> Result<()> {
    let prompted = Arc::clone(&turns);
    let steers = Arc::clone(&turns);
    let loaded = Arc::clone(&asked_to_resume);
    let (on_initialize, on_authenticate, on_new, on_prompt_login) =
        (login.clone(), login.clone(), login.clone(), login);
    let (on_resume, on_new_settings, on_set, on_prompt) = (
        Arc::clone(&settings),
        Arc::clone(&settings),
        Arc::clone(&settings),
        Arc::clone(&settings),
    );
    Agent
        .builder()
        .name("fake-agent")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _cx| {
                let mut meta = serde_json::Map::new();
                meta.insert(
                    "steering".to_owned(),
                    serde_json::json!({ "supported": true }),
                );
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(capabilities(resumes))
                        .auth_methods(on_initialize.logs_in.methods())
                        .meta(meta),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: AuthenticateRequest, responder, _cx| {
                let method = request.method_id.0.to_string();
                if method == WRONG_KEY {
                    return responder.respond_with_error(Error::new(-32603, WRONG));
                }
                on_authenticate.logged_in.lock().unwrap().push(method);
                responder.respond(AuthenticateResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ResumeSessionRequest, responder, _cx| {
                asked_to_resume
                    .lock()
                    .unwrap()
                    .push(request.session_id.to_string());
                if resumes == Resumes::Refuse {
                    return responder.respond_with_error(Error::new(-32002, "no such session"));
                }
                let options = on_resume.lock().unwrap().options();
                responder.respond(ResumeSessionResponse::new().config_options(options))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest, responder, cx: ConnectionTo<Client>| {
                loaded.lock().unwrap().push(request.session_id.to_string());
                // What the session said before, which Cox shows already.
                say(&cx, &request.session_id, "said before")?;
                let options = settings.lock().unwrap().options();
                responder.respond(LoadSessionResponse::new().config_options(options))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, cx: ConnectionTo<Client>| {
                if on_new.asks(true) {
                    return responder.respond_with_error(Error::auth_required());
                }
                sessions_opened.fetch_add(1, Ordering::SeqCst);
                *cwd.lock().unwrap() = Some(request.cwd);
                *mcp_servers.lock().unwrap() = request.mcp_servers;
                // The agent's commands, told ahead of the session's
                // answer, so that Cox knows them before any line is sent.
                cx.send_notification(SessionNotification::new(
                    SessionId::new("session-1"),
                    SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(vec![
                        AvailableCommand::new(COMMAND, "Sum up the conversation so far"),
                    ])),
                ))?;
                let options = on_new_settings.lock().unwrap().options();
                responder.respond(NewSessionResponse::new("session-1").config_options(options))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionConfigOptionRequest, responder, _cx| match on_set
                .lock()
                .unwrap()
                .set(&request)
            {
                Ok(options) => responder.respond(SetSessionConfigOptionResponse::new(options)),
                Err(error) => responder.respond_with_error(error),
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                if on_prompt_login.asks(false) {
                    return responder.respond_with_error(Error::auth_required());
                }
                let (cox, blocks): (Vec<_>, Vec<_>) = texts_of(&request.prompt)
                    .into_iter()
                    .partition(|block| block.starts_with(instructions::COX_START));
                let text = blocks.concat();
                prompted.note(Event::Prompted(text.clone()));
                let verdict = {
                    let mut prompts = prompts.lock().unwrap();
                    prompts.cox.extend(cox);
                    prompts.sent.push(blocks);
                    judge(&prompts.sent)
                };
                let answers = Arc::clone(&answers);
                let turns = Arc::clone(&prompted);
                turns.start();
                let turn = cx.clone();
                if mentions(&text, "switch") {
                    switch(&cx, &request.session_id, &on_prompt)?;
                }
                // Asking permission waits for Cox's answer, which arrives
                // through the loop that runs this handler, so wait elsewhere.
                cx.spawn(async move {
                    let ended = take_turn(&turn, &request, &text, verdict, &answers, &turns).await;
                    match ended {
                        Ok(Ended::Turn(reason)) => {
                            turns.end(Event::Ended(reason));
                            responder.respond(PromptResponse::new(reason))
                        }
                        Ok(Ended::Failed(error)) | Err(error) => {
                            turns.end(Event::Failed);
                            responder.respond_with_error(error)
                        }
                        Ok(Ended::Background) => {
                            turns.end(Event::Ended(StopReason::EndTurn));
                            responder.respond(PromptResponse::new(StopReason::EndTurn))?;
                            go_on(&turn, &request.session_id, &text, &answers, &turns).await
                        }
                    }
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        // It takes a line into a turn that waits, as `claude-agent-acp`
        // takes one (`_session/steering`).
        .on_receive_request(
            async move |request: UntypedMessage, responder: Responder<serde_json::Value>, _cx| {
                if request.method != "_session/steering" {
                    return responder.respond_with_error(Error::method_not_found());
                }
                let prompt: Vec<ContentBlock> =
                    serde_json::from_value(request.params["prompt"].clone()).unwrap_or_default();
                if !steers.steer(texts_of(&prompt).concat()) {
                    return responder.respond(
                        serde_json::json!({ "outcome": "promptRequired", "reason": "noRunningTurn" }),
                    );
                }
                responder.respond(serde_json::json!({ "outcome": "injected" }))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |_: CancelNotification, _cx| {
                turns.cancel();
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(transport)
        .await
}

/// How a turn ends: as it says, with an error, or once the agent left
/// work running in the background, to go on with after.
enum Ended {
    Turn(StopReason),
    Failed(Error),
    Background,
}

/// Takes the turn that answers `request`, whose text is `text`, as the
/// prompt asks, or says `verdict` when it asks to judge a goal.
async fn take_turn(
    turn: &ConnectionTo<Client>,
    request: &PromptRequest,
    text: &str,
    verdict: Option<&'static str>,
    answers: &Mutex<Vec<RequestPermissionOutcome>>,
    turns: &Turns,
) -> Result<Ended> {
    let session = &request.session_id;
    if let Some(verdict) = verdict {
        say(turn, session, verdict)?;
        return Ok(Ended::Turn(StopReason::EndTurn));
    }
    if mentions(text, "fail") {
        return Ok(Ended::Failed(Error::new(
            -32603,
            "the model is unavailable",
        )));
    }
    if mentions(text, "slow") {
        turns.held().await;
    }
    work(turn, session, text)?;
    if mentions(text, "background") {
        say(turn, session, BACKGROUND_STARTED)?;
        return Ok(Ended::Background);
    }
    if mentions(text, "wait") {
        let woken = turns.wait(|| say(turn, session, "waiting")).await?;
        let Woken::Steered(steered) = woken else {
            return Ok(Ended::Turn(StopReason::Cancelled));
        };
        say(turn, session, &format!("echo: {steered}"))?;
        return Ok(Ended::Turn(StopReason::EndTurn));
    }
    if mentions(text, "permission") {
        let answer = ask_permission(turn, session, turns).await?;
        answers.lock().unwrap().push(answer.clone());
        if answer == RequestPermissionOutcome::Cancelled {
            return Ok(Ended::Turn(StopReason::Cancelled));
        }
        finish(turn, session, "call-1")?;
    }
    for word in format!("echo: {text}").split_inclusive(' ') {
        say(turn, session, word)?;
    }
    Ok(Ended::Turn(StopReason::EndTurn))
}

/// What the agent says it can do: go on with a session as `resumes` says.
fn capabilities(resumes: Resumes) -> AgentCapabilities {
    // It takes the files a prompt points to whole.
    let prompts = PromptCapabilities::new().embedded_context(true);
    // It works in other folders beside its own.
    let sessions = SessionCapabilities::new()
        .additional_directories(SessionAdditionalDirectoriesCapabilities::new());
    let capabilities = match resumes {
        Resumes::Never => AgentCapabilities::new().session_capabilities(sessions),
        Resumes::Load => AgentCapabilities::new()
            .load_session(true)
            .session_capabilities(sessions),
        Resumes::Resume | Resumes::Refuse => AgentCapabilities::new()
            .session_capabilities(sessions.resume(SessionResumeCapabilities::new())),
    };
    capabilities.prompt_capabilities(prompts)
}

/// The blocks of a prompt, as text: a file given whole as `file <uri>:`
/// and its text, and a link to one as `link <uri>`.
fn texts_of(prompt: &[ContentBlock]) -> Vec<String> {
    prompt
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            ContentBlock::Resource(resource) => match &resource.resource {
                EmbeddedResourceResource::TextResourceContents(file) => {
                    Some(format!("file {}: {}", file.uri, file.text))
                }
                _ => None,
            },
            ContentBlock::ResourceLink(link) => Some(format!("link {}", link.uri)),
            _ => None,
        })
        .collect()
}

/// What Cox asks the judge of a goal to end its reply with, which tells
/// the agent it is asked to judge.
const JUDGE_ASKS: &str = "PASS or FAIL";

/// What the agent says as it ends the turn that leaves work running in
/// the background, and once it went on by itself after it.
pub const BACKGROUND_STARTED: &str = "started in the background";
pub const BACKGROUND_FINISHED: &str = "finished in the background";

/// Goes on by itself after the turn that answered `text` ended, with no
/// prompt, once the test lets it, as `claude-agent-acp` does once a task
/// Claude Code left running in the background ends: asks permission
/// first when `text` mentions "ask", and says it finished.
async fn go_on(
    cx: &ConnectionTo<Client>,
    session: &SessionId,
    text: &str,
    answers: &Mutex<Vec<RequestPermissionOutcome>>,
    turns: &Turns,
) -> Result<()> {
    turns.held().await;
    if mentions(text, "ask") {
        let answer = ask_permission(cx, session, turns).await?;
        answers.lock().unwrap().push(answer.clone());
        if answer == RequestPermissionOutcome::Cancelled {
            turns.note(Event::WentOn);
            return Ok(());
        }
        finish(cx, session, "call-1")?;
    }
    say(cx, session, BACKGROUND_FINISHED)?;
    turns.note(Event::WentOn);
    Ok(())
}

/// What the agent says when the last of `prompts` asks it to judge a
/// goal: it finds the goal not met the first time it is asked, and met
/// from then on. `None` when it is not asked to judge.
fn judge(prompts: &[Vec<String>]) -> Option<&'static str> {
    let asks = |blocks: &Vec<String>| blocks.concat().contains(JUDGE_ASKS);
    if !prompts.last().is_some_and(asks) {
        return None;
    }
    let judged = prompts.iter().filter(|blocks| asks(blocks)).count();
    Some(if judged <= 1 {
        "I read src/main.rs.\nFAIL: the greeting still says hello"
    } else {
        "I read it again.\nPASS"
    })
}

/// The title the agent gives its session when asked to.
pub const TITLE: &str = "Fix the tests";

pub const THOUGHT: &str = "Look at the code first.\nThen answer.";

fn think(cx: &ConnectionTo<Client>, session: &SessionId) -> Result<()> {
    cx.send_notification(SessionNotification::new(
        session.clone(),
        SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            THOUGHT,
        )))),
    ))
}

fn plan(cx: &ConnectionTo<Client>, session: &SessionId) -> Result<()> {
    let step = |text: &str, status| PlanEntry::new(text, PlanEntryPriority::Medium, status);
    cx.send_notification(SessionNotification::new(
        session.clone(),
        SessionUpdate::Plan(Plan::new(vec![
            step("Read the code", PlanEntryStatus::InProgress),
            step("Fix it", PlanEntryStatus::Pending),
        ])),
    ))
}

fn say(cx: &ConnectionTo<Client>, session: &SessionId, text: &str) -> Result<()> {
    cx.send_notification(SessionNotification::new(
        session.clone(),
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            text,
        )))),
    ))
}

/// The change the tool call that asks permission makes: the second of the
/// three lines of `x.txt`.
pub const OLD_TEXT: &str = "one\ntwo\nthree\n";
pub const NEW_TEXT: &str = "one\n2\nthree\n";

/// What `ls` prints.
pub const LS_OUTPUT: &str = "x.txt\ny.txt";

/// Announces a tool call to edit `x.txt`, of ACP's kind `edit`, showing
/// the change, and asks permission to make it, offering to allow or
/// reject it once. The request leaves the kind and the file out, as
/// `claude-agent-acp`'s do: they were told with the call.
async fn ask_permission(
    cx: &ConnectionTo<Client>,
    session: &SessionId,
    turns: &Turns,
) -> Result<RequestPermissionOutcome> {
    let change = || {
        vec![ToolCallContent::Diff(
            Diff::new("/work/x.txt", NEW_TEXT).old_text(OLD_TEXT),
        )]
    };
    cx.send_notification(SessionNotification::new(
        session.clone(),
        SessionUpdate::ToolCall(
            ToolCall::new("call-1", "Edit x.txt")
                .kind(ToolKind::Edit)
                .locations(vec![ToolCallLocation::new("/work/x.txt")])
                .content(change()),
        ),
    ))?;
    let request = RequestPermissionRequest::new(
        session.clone(),
        ToolCallUpdate::new(
            "call-1",
            ToolCallUpdateFields::new()
                .title("Edit x.txt")
                .content(change()),
        ),
        vec![
            PermissionOption::new("allow", "Allow once", PermissionOptionKind::AllowOnce),
            PermissionOption::new("reject", "Reject once", PermissionOptionKind::RejectOnce),
        ],
    );
    let response = cx.send_request(request).block_task();
    turns.note(Event::Asked);
    let response = response.await?;
    Ok(response.outcome)
}

/// Reports the tool call `id` done.
fn finish(cx: &ConnectionTo<Client>, session: &SessionId, id: &str) -> Result<()> {
    cx.send_notification(SessionNotification::new(
        session.clone(),
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            id.to_owned(),
            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
        )),
    ))
}

/// Runs `ls` as a tool call: announces it, then reports it running, and
/// done with what it printed.
fn run_tool(cx: &ConnectionTo<Client>, session: &SessionId, id: &str) -> Result<()> {
    let update = |update| cx.send_notification(SessionNotification::new(session.clone(), update));
    update(SessionUpdate::ToolCall(ToolCall::new(id.to_owned(), "ls")))?;
    update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        id.to_owned(),
        ToolCallUpdateFields::new().status(ToolCallStatus::InProgress),
    )))?;
    update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        id.to_owned(),
        ToolCallUpdateFields::new()
            .status(ToolCallStatus::Completed)
            .content(vec![LS_OUTPUT.into()]),
    )))
}
