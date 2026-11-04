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
//! mentions "slow", it waits a moment before anything. When the
//! prompt mentions "wait", it replies `waiting` and goes on until Cox cancels the
//! turn. A turn cancelled, or whose permission request Cox answers as cancelled,
//! ends as cancelled. Its sessions have two settings, [`MODEL`] and
//! [`MODE`], which Cox sets as it asks, and when a prompt mentions
//! "switch", the agent sets its mode to `plan` on its own. It goes on with a session it opened before as
//! [`Resumes`] says. It records what Cox asked of it and answered so tests can
//! check what Cox did.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ConfigOptionUpdate, ContentBlock, ContentChunk, Diff,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse, McpServer,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, Plan, PlanEntry,
    PlanEntryPriority, PlanEntryStatus, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, ResumeSessionRequest, ResumeSessionResponse, SessionCapabilities,
    SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption, SessionId,
    SessionInfoUpdate, SessionNotification, SessionResumeCapabilities, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, StopReason, TextContent,
    ToolCall, ToolCallContent, ToolCallLocation, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{Agent, Channel, Client, ConnectionTo, Error, Responder, Result};
use tokio::sync::Notify;

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

#[derive(Default)]
pub struct FakeAgent {
    resumes: Resumes,
    /// The sessions Cox asked to go on with, in order.
    asked_to_resume: Arc<Mutex<Vec<String>>>,
    sessions_opened: Arc<AtomicUsize>,
    cwd: Arc<Mutex<Option<PathBuf>>>,
    mcp_servers: Arc<Mutex<Vec<McpServer>>>,
    prompts: Arc<Mutex<Vec<Vec<String>>>>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
    cancels: Arc<Cancels>,
    settings: Arc<Mutex<Settings>>,
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

/// The cancels Cox has sent.
#[derive(Default)]
struct Cancels {
    count: AtomicUsize,
    /// Wakes the turn that waits to be cancelled.
    wake: Notify,
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

    /// Starts serving and returns the end of the connection to give Cox.
    pub fn connect(&self) -> Channel {
        let (cox_end, agent_end) = Channel::duplex();
        tokio::spawn(serve(
            agent_end,
            self.resumes,
            Arc::clone(&self.asked_to_resume),
            Arc::clone(&self.sessions_opened),
            Arc::clone(&self.cwd),
            Arc::clone(&self.mcp_servers),
            Arc::clone(&self.prompts),
            Arc::clone(&self.answers),
            Arc::clone(&self.cancels),
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
            .iter()
            .map(|blocks| blocks.concat())
            .collect()
    }

    /// The text blocks of each prompt Cox has sent, in order.
    pub fn prompt_blocks(&self) -> Vec<Vec<String>> {
        self.prompts.lock().unwrap().clone()
    }

    /// Cox's answer to each permission request, in order.
    pub fn answers(&self) -> Vec<RequestPermissionOutcome> {
        self.answers.lock().unwrap().clone()
    }

    /// How many times Cox has asked to cancel a turn.
    pub fn cancels(&self) -> usize {
        self.cancels.count.load(Ordering::SeqCst)
    }
}

/// What the agent does on its way to the reply, as the prompt asks: it
/// thinks, plans and runs tools.
fn work(cx: &ConnectionTo<Client>, session: &SessionId, text: &str) -> Result<()> {
    if text.contains("title") {
        cx.send_notification(SessionNotification::new(
            session.clone(),
            SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new().title(TITLE)),
        ))?;
    }
    if text.contains("think") {
        think(cx, session)?;
    }
    if text.contains("plan") {
        plan(cx, session)?;
    }
    if text.contains("tool") {
        run_tool(cx, session, "call-2")?;
        if text.contains("twice") {
            run_tool(cx, session, "call-3")?;
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "what the agent records, each once"
)]
async fn serve(
    transport: Channel,
    resumes: Resumes,
    asked_to_resume: Arc<Mutex<Vec<String>>>,
    sessions_opened: Arc<AtomicUsize>,
    cwd: Arc<Mutex<Option<PathBuf>>>,
    mcp_servers: Arc<Mutex<Vec<McpServer>>>,
    prompts: Arc<Mutex<Vec<Vec<String>>>>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
    cancels: Arc<Cancels>,
    settings: Arc<Mutex<Settings>>,
) -> Result<()> {
    let waits = Arc::clone(&cancels);
    let loaded = Arc::clone(&asked_to_resume);
    let (on_resume, on_new, on_set, on_prompt) = (
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
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(capabilities(resumes)),
                )
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
            async move |request: NewSessionRequest, responder, _cx| {
                sessions_opened.fetch_add(1, Ordering::SeqCst);
                *cwd.lock().unwrap() = Some(request.cwd);
                *mcp_servers.lock().unwrap() = request.mcp_servers;
                let options = on_new.lock().unwrap().options();
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
                let blocks = texts_of(&request.prompt);
                let text = blocks.concat();
                prompts.lock().unwrap().push(blocks);
                let verdict = judge(&prompts.lock().unwrap());
                let answers = Arc::clone(&answers);
                let cancels = Arc::clone(&waits);
                let turn = cx.clone();
                if text.contains("switch") {
                    switch(&cx, &request.session_id, &on_prompt)?;
                }
                // Asking permission waits for Cox's answer, which arrives
                // through the loop that runs this handler, so wait elsewhere.
                cx.spawn(take_turn(
                    turn, request, responder, text, verdict, answers, cancels,
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |_: CancelNotification, _cx| {
                cancels.count.fetch_add(1, Ordering::SeqCst);
                cancels.wake.notify_one();
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(transport)
        .await
}

/// Takes the turn that answers `request`, whose text is `text`, as the
/// prompt asks, or says `verdict` when it asks to judge a goal, and
/// answers it with `responder`.
async fn take_turn(
    turn: ConnectionTo<Client>,
    request: PromptRequest,
    responder: Responder<PromptResponse>,
    text: String,
    verdict: Option<&'static str>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
    cancels: Arc<Cancels>,
) -> Result<()> {
    if let Some(verdict) = verdict {
        say(&turn, &request.session_id, verdict)?;
        return responder.respond(PromptResponse::new(StopReason::EndTurn));
    }
    if text.contains("fail") {
        return responder.respond_with_error(Error::new(-32603, "the model is unavailable"));
    }
    if text.contains("slow") {
        tokio::time::sleep(SLOW).await;
    }
    work(&turn, &request.session_id, &text)?;
    if text.contains("wait") {
        say(&turn, &request.session_id, "waiting")?;
        cancels.wake.notified().await;
        return responder.respond(PromptResponse::new(StopReason::Cancelled));
    }
    if text.contains("permission") {
        let answer = ask_permission(&turn, &request.session_id).await?;
        answers.lock().unwrap().push(answer.clone());
        if answer == RequestPermissionOutcome::Cancelled {
            return responder.respond(PromptResponse::new(StopReason::Cancelled));
        }
        finish(&turn, &request.session_id, "call-1")?;
    }
    for word in format!("echo: {text}").split_inclusive(' ') {
        say(&turn, &request.session_id, word)?;
    }
    responder.respond(PromptResponse::new(StopReason::EndTurn))
}

/// What the agent says it can do: go on with a session as `resumes` says.
fn capabilities(resumes: Resumes) -> AgentCapabilities {
    match resumes {
        Resumes::Never => AgentCapabilities::new(),
        Resumes::Load => AgentCapabilities::new().load_session(true),
        Resumes::Resume | Resumes::Refuse => AgentCapabilities::new().session_capabilities(
            SessionCapabilities::new().resume(SessionResumeCapabilities::new()),
        ),
    }
}

/// The text blocks of a prompt.
fn texts_of(prompt: &[ContentBlock]) -> Vec<String> {
    prompt
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

/// Sends a piece of the reply.
/// What the agent thinks when the prompt mentions "think".
/// How long the agent waits before it answers when asked to be slow.
const SLOW: Duration = Duration::from_millis(300);

/// What Cox asks the judge of a goal to end its reply with, which tells
/// the agent it is asked to judge.
const JUDGE_ASKS: &str = "PASS or FAIL";

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
    let response = cx.send_request(request).block_task().await?;
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
