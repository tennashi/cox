//! Talking to an agent over the Agent Client Protocol (ACP), in Cox's terms.
//!
//! [`connect`] opens a [`Session`] with an agent, handing it the
//! [`McpServer`]s whose tools it may use, or resumes one the agent had
//! opened before, when it can. Each [`Session::send`] starts a
//! [`Turn`], which hands over what the agent sends as [`Update`]s until the
//! turn ends, and which [`Turn::cancel`] asks the agent to end early.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, Content, ContentBlock, ContentChunk, Diff,
    InitializeRequest, LoadSessionRequest, McpServer as AcpMcpServer, McpServerStdio,
    NewSessionRequest, PermissionOptionKind, Plan as AcpPlan, PlanEntry as AcpPlanEntry,
    PlanEntryStatus, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest,
    SelectedPermissionOutcome, SessionId, SessionInfoUpdate, SessionNotification, SessionUpdate,
    StopReason as AcpStopReason, TextContent, ToolCall as AcpToolCall,
    ToolCallContent as AcpToolCallContent, ToolCallId, ToolCallLocation, ToolCallStatus,
    ToolCallUpdate, ToolKind as AcpToolKind,
};
use agent_client_protocol::schema::{MaybeUndefined, ProtocolVersion};
use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectTo, ConnectionTo, LineDirection, Responder,
};
use anyhow::Context;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// A way to reach an agent: a process speaking ACP on its stdio, or, in
/// tests, an in-memory channel.
pub trait Transport: ConnectTo<Client> {}

impl<T: ConnectTo<Client>> Transport for T {}

/// The target under which each line Cox and the agent send each other is
/// said, at `debug`, with `from` saying whose it is: `cox` or `agent`.
pub const WIRE: &str = "cox_acp::wire";

/// The target under which each line the agent writes to its standard error
/// is said, at `info`.
pub const STDERR: &str = "cox_acp::stderr";

/// Starts `command` (the program, then its arguments) as an agent process.
/// What Cox and the agent send each other, and what the agent writes to its
/// standard error, is said a line at a time, under [`WIRE`] and
/// [`STDERR`], in the span `launch` is called in, such as its chat's.
///
/// # Errors
///
/// Returns an error if `command` is empty.
pub fn launch(command: impl IntoIterator<Item = String>) -> anyhow::Result<impl Transport> {
    let span = tracing::Span::current();
    Ok(
        AcpAgent::from_args(command)?.with_debug(move |line, direction| {
            span.in_scope(|| match direction {
                LineDirection::Stdin => tracing::debug!(target: WIRE, from = "cox", "{line}"),
                LineDirection::Stdout => tracing::debug!(target: WIRE, from = "agent", "{line}"),
                LineDirection::Stderr => tracing::info!(target: STDERR, "{line}"),
            });
        }),
    )
}

/// An MCP server the agent starts for the session, to use its tools. The
/// agent runs `command` with `args`, and talks MCP over its stdio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServer {
    /// The name the agent knows the server by, and names its tools after.
    pub name: String,
    /// The program to run, as an absolute path.
    pub command: PathBuf,
    pub args: Vec<String>,
}

impl From<McpServer> for AcpMcpServer {
    fn from(server: McpServer) -> Self {
        Self::Stdio(McpServerStdio::new(server.name, server.command).args(server.args))
    }
}

/// Connects to `agent`, opens a session working in `cwd` with
/// `mcp_servers` for the agent to use, and runs `talk` with it. The
/// connection closes when `talk` returns.
///
/// Each title the agent gives the session, whenever it gives one, goes to
/// `titled`.
///
/// With `resume`, the id of a session the agent opened before, the agent
/// goes on with that session instead, when it can: [`Session::resumed`]
/// tells whether it did. What the agent tells of the session's past while
/// it resumes it is left out: the session starts from there.
///
/// # Errors
///
/// Returns the error `talk` returns, or an error if the connection fails or
/// the agent refuses to open a session.
pub async fn connect<T>(
    agent: impl Transport,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    resume: Option<String>,
    titled: impl Fn(String) + Send + Sync,
    talk: impl AsyncFnOnce(Session) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let (updates, received) = unbounded_channel();
    let mut outcome = None;
    Client
        .builder()
        .name("cox")
        .on_receive_notification(
            async |notification: SessionNotification, _cx| {
                // A title comes between turns too.
                if let SessionUpdate::SessionInfoUpdate(SessionInfoUpdate {
                    title: MaybeUndefined::Value(title),
                    ..
                }) = notification.update
                {
                    titled(title);
                } else if let Some(update) = translate(notification.update) {
                    deliver(&updates, update);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async |request: RequestPermissionRequest, responder, _cx| {
                let request = PermissionRequest::new(request, responder);
                deliver(
                    &updates,
                    Received::Update(Update::PermissionRequest(request)),
                );
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async |cx: ConnectionTo<Agent>| {
            let (id, resumed) = start(&cx, cwd, mcp_servers, resume).await?;
            let mut received = received;
            if resumed {
                // The agent tells what it heard of the session's past before
                // it answers, so all of it has arrived.
                while received.try_recv().is_ok() {}
            }
            let session = Session {
                cx,
                id,
                resumed,
                received,
                tool_calls: HashMap::new(),
            };
            outcome = Some(talk(session).await);
            Ok(())
        })
        .await?;
    outcome.context("the connection closed before the session started")?
}

/// Initializes the connection and opens a session working in `cwd`, or
/// resumes the session `resume` when the agent can, and returns the
/// session's id, with whether it was resumed.
async fn start(
    cx: &ConnectionTo<Agent>,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    resume: Option<String>,
) -> agent_client_protocol::Result<(SessionId, bool)> {
    let initialized = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let mcp_servers: Vec<AcpMcpServer> = mcp_servers.into_iter().map(Into::into).collect();
    if let Some(id) = resume
        && resumed(
            cx,
            &initialized.agent_capabilities,
            SessionId::new(id.clone()),
            &cwd,
            &mcp_servers,
        )
        .await
    {
        return Ok((SessionId::new(id), true));
    }
    let session = cx
        .send_request(NewSessionRequest::new(cwd).mcp_servers(mcp_servers))
        .block_task()
        .await?;
    Ok((session.session_id, false))
}

/// Asks the agent to go on with session `id`, working in `cwd` with
/// `mcp_servers`, and returns whether it does. An agent that can resume a
/// session without telling its past is asked to, since the past is not
/// shown again; one that can only load it, telling its past, is asked to
/// load it. An agent that can do neither, or refuses, does not go on with
/// it.
async fn resumed(
    cx: &ConnectionTo<Agent>,
    capabilities: &AgentCapabilities,
    id: SessionId,
    cwd: &std::path::Path,
    mcp_servers: &[AcpMcpServer],
) -> bool {
    if capabilities.session_capabilities.resume.is_some() {
        let request = ResumeSessionRequest::new(id, cwd).mcp_servers(mcp_servers.to_vec());
        cx.send_request(request).block_task().await.is_ok()
    } else if capabilities.load_session {
        let request = LoadSessionRequest::new(id, cwd).mcp_servers(mcp_servers.to_vec());
        cx.send_request(request).block_task().await.is_ok()
    } else {
        false
    }
}

/// What the connection hands to the turn that is running: an update ready to
/// show, or news of a tool call, which the turn gathers until the call ends.
enum Received {
    Update(Update),
    ToolCall {
        id: ToolCallId,
        title: Option<String>,
        kind: Option<ToolKind>,
        locations: Option<Vec<PathBuf>>,
        content: Option<Vec<ToolCallContent>>,
        status: Option<ToolCallStatus>,
    },
}

/// What the agent has told of a tool call that has not ended, for its
/// permission request, which may leave it out.
#[derive(Clone, Debug, Default)]
struct Known {
    title: Option<String>,
    kind: Option<ToolKind>,
    locations: Vec<PathBuf>,
}

/// Hands `received` to the turn that is running. The receiver lives as long
/// as the session, so sending only fails after `talk` has returned, when
/// there is no one left to show it to.
fn deliver(updates: &UnboundedSender<Received>, received: Received) {
    let _ = updates.send(received);
}

/// The parts of a session update Cox uses, or `None` for the rest.
fn translate(update: SessionUpdate) -> Option<Received> {
    match update {
        SessionUpdate::AgentMessageChunk(ContentChunk {
            content: ContentBlock::Text(text),
            ..
        }) => Some(Received::Update(Update::ReplyText(text.text))),
        SessionUpdate::AgentThoughtChunk(ContentChunk {
            content: ContentBlock::Text(text),
            ..
        }) => Some(Received::Update(Update::ThoughtText(text.text))),
        SessionUpdate::Plan(AcpPlan { entries, .. }) => Some(Received::Update(Update::Plan(
            entries.into_iter().map(PlanEntry::from).collect(),
        ))),
        SessionUpdate::ToolCall(AcpToolCall {
            tool_call_id,
            title,
            kind,
            content,
            locations,
            status,
            ..
        }) => Some(Received::ToolCall {
            id: tool_call_id,
            title: Some(title),
            kind: Some(kind.into()),
            locations: Some(paths(locations)),
            content: Some(translate_content(content)),
            status: Some(status),
        }),
        SessionUpdate::ToolCallUpdate(ToolCallUpdate {
            tool_call_id,
            fields,
            ..
        }) => Some(Received::ToolCall {
            id: tool_call_id,
            title: fields.title,
            kind: fields.kind.map(ToolKind::from),
            locations: fields.locations.map(paths),
            content: fields.content.map(translate_content),
            status: fields.status,
        }),
        _ => None,
    }
}

/// The paths of the files a tool call works on.
fn paths(locations: Vec<ToolCallLocation>) -> Vec<PathBuf> {
    locations
        .into_iter()
        .map(|location| location.path)
        .collect()
}

/// The parts of a tool call's content Cox shows: text and changes to files.
/// Images, resources and terminals are left out.
fn translate_content(content: Vec<AcpToolCallContent>) -> Vec<ToolCallContent> {
    content
        .into_iter()
        .filter_map(|content| match content {
            AcpToolCallContent::Content(Content {
                content: ContentBlock::Text(text),
                ..
            }) => Some(ToolCallContent::Text(text.text)),
            AcpToolCallContent::Diff(Diff {
                path,
                old_text,
                new_text,
                ..
            }) => Some(ToolCallContent::Diff {
                path,
                old_text,
                new_text,
            }),
            _ => None,
        })
        .collect()
}

/// A conversation with an agent.
pub struct Session {
    cx: ConnectionTo<Agent>,
    id: SessionId,
    resumed: bool,
    received: UnboundedReceiver<Received>,
    /// What is known of each tool call of the turn that has not ended yet.
    tool_calls: HashMap<ToolCallId, Known>,
}

impl Session {
    /// The id the agent gave the session, to resume it with later.
    #[must_use]
    pub fn id(&self) -> String {
        self.id.to_string()
    }

    /// Whether the agent went on with a session it had opened before,
    /// rather than opening a new one.
    #[must_use]
    pub fn resumed(&self) -> bool {
        self.resumed
    }

    /// Sends `texts` to the agent as a prompt, each a block of its own,
    /// starting a turn.
    pub fn send(&mut self, texts: impl IntoIterator<Item = String>) -> Turn<'_> {
        let prompt = PromptRequest::new(
            self.id.clone(),
            texts
                .into_iter()
                .map(|text| ContentBlock::Text(TextContent::new(text)))
                .collect(),
        );
        self.tool_calls.clear();
        Turn {
            cx: &self.cx,
            session: &self.id,
            received: &mut self.received,
            tool_calls: &mut self.tool_calls,
            response: Box::pin(self.cx.send_request(prompt).block_task()),
            answer: None,
        }
    }
}

/// One prompt and everything the agent does in answer to it.
pub struct Turn<'s> {
    cx: &'s ConnectionTo<Agent>,
    session: &'s SessionId,
    received: &'s mut UnboundedReceiver<Received>,
    tool_calls: &'s mut HashMap<ToolCallId, Known>,
    response: Pin<Box<dyn Future<Output = agent_client_protocol::Result<PromptResponse>> + Send>>,
    /// The agent's answer to the prompt, once it has arrived.
    answer: Option<agent_client_protocol::Result<PromptResponse>>,
}

impl Turn<'_> {
    /// The next thing the agent sends in this turn, or `None` once the turn
    /// has ended. When the turn ends cancelled, each tool call it left
    /// running ends then, as [`ToolCallResult::Cancelled`].
    pub async fn next(&mut self) -> Option<Update> {
        loop {
            if let Some(answer) = &self.answer {
                let cancelled = matches!(
                    answer,
                    Ok(PromptResponse {
                        stop_reason: AcpStopReason::Cancelled,
                        ..
                    })
                );
                return cancelled
                    .then(|| self.drop_tool_call())
                    .flatten()
                    .map(Update::ToolCall);
            }
            // The agent sends a turn's updates before answering the prompt,
            // and they are delivered in that order, so take every waiting
            // update before looking at the answer.
            let received = tokio::select! {
                biased;
                Some(received) = self.received.recv() => received,
                answer = &mut self.response => {
                    self.answer = Some(answer);
                    continue;
                }
            };
            match received {
                Received::Update(Update::PermissionRequest(mut request)) => {
                    if let Some(known) =
                        self.tool_calls.get(&request.request.tool_call.tool_call_id)
                    {
                        request.fill(known);
                    }
                    return Some(Update::PermissionRequest(request));
                }
                Received::Update(update) => return Some(update),
                Received::ToolCall {
                    id,
                    title,
                    kind,
                    locations,
                    content,
                    status,
                } => {
                    let told = Known {
                        title,
                        kind,
                        locations: locations.unwrap_or_default(),
                    };
                    if let Some(call) = self.follow_tool_call(id, told, content, status) {
                        return Some(Update::ToolCall(call));
                    }
                }
            }
        }
    }

    /// Forgets one of the tool calls that have not ended, if any is left,
    /// and returns it as cancelled.
    fn drop_tool_call(&mut self) -> Option<ToolCall> {
        let id = self.tool_calls.keys().next()?.clone();
        let title = self.tool_calls.remove(&id).and_then(|known| known.title);
        Some(ToolCall {
            id: id.to_string(),
            title,
            content: None,
            result: Some(ToolCallResult::Cancelled),
        })
    }

    /// Records news of a tool call, what it is `told` to be, and returns the
    /// call when there is something new to show: it started, its title or
    /// content changed, or it ended. A kind or locations not told stay as
    /// they were.
    fn follow_tool_call(
        &mut self,
        id: ToolCallId,
        told: Known,
        content: Option<Vec<ToolCallContent>>,
        status: Option<ToolCallStatus>,
    ) -> Option<ToolCall> {
        let title = told.title;
        let result = match status {
            Some(ToolCallStatus::Completed) => Some(ToolCallResult::Completed),
            Some(ToolCallStatus::Failed) => Some(ToolCallResult::Failed),
            _ => None,
        };
        let shown_id = id.to_string();
        if result.is_some() {
            let known = self.tool_calls.remove(&id).and_then(|known| known.title);
            return Some(ToolCall {
                id: shown_id,
                title: title.or(known),
                content,
                result,
            });
        }
        let started = !self.tool_calls.contains_key(&id);
        let known = self.tool_calls.entry(id).or_default();
        if told.kind.is_some() {
            known.kind = told.kind;
        }
        if !told.locations.is_empty() {
            known.locations = told.locations;
        }
        let retitled = title.is_some() && title != known.title;
        if retitled {
            known.title = title;
        }
        (started || retitled || content.is_some()).then(|| ToolCall {
            id: shown_id,
            title: known.title.clone(),
            content,
            result: None,
        })
    }

    /// Asks the agent to end the turn early. The turn goes on until the
    /// agent has stopped, which it tells with [`StopReason::Cancelled`];
    /// until then, updates still arrive, and a permission request waiting
    /// for an answer, or one that arrives after this, is to be answered
    /// with [`PermissionAnswer::Cancel`].
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    pub fn cancel(&self) -> anyhow::Result<()> {
        self.cx
            .send_notification(CancelNotification::new(self.session.clone()))?;
        Ok(())
    }

    /// Waits for the turn to end, and returns why it ended. Updates not taken
    /// with [`Turn::next`] are dropped.
    ///
    /// # Errors
    ///
    /// Returns the error the agent answered the prompt with, or an error if
    /// the connection fails.
    pub async fn end(mut self) -> anyhow::Result<StopReason> {
        loop {
            if let Some(answer) = self.answer.take() {
                return Ok(StopReason::from(answer?.stop_reason));
            }
            self.next().await;
        }
    }
}

/// Something the agent sends during a turn.
pub enum Update {
    /// A piece of the agent's reply.
    ReplyText(String),
    /// A piece of what the agent thinks on its way to the reply.
    ThoughtText(String),
    /// The plan the agent works to, all of it, in place of the plan it
    /// sent before.
    Plan(Vec<PlanEntry>),
    /// The agent asks to be allowed to run a tool call.
    PermissionRequest(PermissionRequest),
    /// A tool call the agent made started, was retitled, changed what it
    /// shows, or ended.
    ToolCall(ToolCall),
}

/// A tool call the agent made, as it stands.
pub struct ToolCall {
    /// Tells the calls of a session apart: the news of one call carries the
    /// same id from start to end.
    pub id: String,
    /// The title the agent gave the call, if any.
    pub title: Option<String>,
    /// What the call shows from now on, in place of what it showed before,
    /// or `None` when that stays as it was.
    pub content: Option<Vec<ToolCallContent>>,
    /// How the call ended, or `None` while it runs.
    pub result: Option<ToolCallResult>,
}

/// A step of the plan the agent works to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanEntry {
    /// What the step is to do.
    pub text: String,
    pub state: PlanState,
}

/// How far a step of the plan has gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanState {
    Pending,
    InProgress,
    Completed,
}

impl From<AcpPlanEntry> for PlanEntry {
    fn from(entry: AcpPlanEntry) -> Self {
        let state = match entry.status {
            PlanEntryStatus::InProgress => PlanState::InProgress,
            PlanEntryStatus::Completed => PlanState::Completed,
            // A state newer than this version of Cox knows is taken as
            // not started.
            _ => PlanState::Pending,
        };
        Self {
            text: entry.content,
            state,
        }
    }
}

/// Something a tool call shows: what it is about to do, such as a change to
/// a file, or what it did, such as a command's output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolCallContent {
    Text(String),
    /// A change to the file at `path`, from `old_text` to `new_text`.
    /// Without `old_text`, the file is new.
    Diff {
        path: PathBuf,
        old_text: Option<String>,
        new_text: String,
    },
}

/// What kind of tool a call is, as ACP sorts them: what it does, whatever
/// the agent names its tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    /// Another kind, or a kind newer than this version of Cox knows.
    Other,
}

impl ToolKind {
    /// Every kind, in the order ACP lists them.
    pub const ALL: [Self; 10] = [
        Self::Read,
        Self::Edit,
        Self::Delete,
        Self::Move,
        Self::Search,
        Self::Execute,
        Self::Think,
        Self::Fetch,
        Self::SwitchMode,
        Self::Other,
    ];

    /// The kind's name, as ACP writes it, as `edit` or `switch_mode`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Move => "move",
            Self::Search => "search",
            Self::Execute => "execute",
            Self::Think => "think",
            Self::Fetch => "fetch",
            Self::SwitchMode => "switch_mode",
            Self::Other => "other",
        }
    }

    /// The kind `name` names, as ACP writes it, if any.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Whether a call of the kind changes files: edits, deletes or moves
    /// them.
    #[must_use]
    pub fn changes_files(self) -> bool {
        matches!(self, Self::Edit | Self::Delete | Self::Move)
    }
}

impl From<AcpToolKind> for ToolKind {
    fn from(kind: AcpToolKind) -> Self {
        match kind {
            AcpToolKind::Read => Self::Read,
            AcpToolKind::Edit => Self::Edit,
            AcpToolKind::Delete => Self::Delete,
            AcpToolKind::Move => Self::Move,
            AcpToolKind::Search => Self::Search,
            AcpToolKind::Execute => Self::Execute,
            AcpToolKind::Think => Self::Think,
            AcpToolKind::Fetch => Self::Fetch,
            AcpToolKind::SwitchMode => Self::SwitchMode,
            _ => Self::Other,
        }
    }
}

/// How a tool call ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolCallResult {
    Completed,
    Failed,
    /// The turn was cancelled before the call ended.
    Cancelled,
}

/// A request from the agent for permission to run a tool call. Answer it with
/// [`PermissionRequest::answer`]; the agent waits until then.
pub struct PermissionRequest {
    // Boxed so that an `Update` stays small whichever variant it holds.
    request: Box<RequestPermissionRequest>,
    responder: Box<Responder<RequestPermissionResponse>>,
    options: Vec<PermissionOption>,
    /// What the request, or else the tool call's news before it, says of
    /// the call.
    known: Known,
}

impl PermissionRequest {
    fn new(
        request: RequestPermissionRequest,
        responder: Responder<RequestPermissionResponse>,
    ) -> Self {
        let options = request
            .options
            .iter()
            .map(|option| PermissionOption {
                name: option.name.clone(),
                kind: PermissionKind::from(option.kind),
            })
            .collect();
        let fields = &request.tool_call.fields;
        let known = Known {
            title: fields.title.clone(),
            kind: fields.kind.map(ToolKind::from),
            locations: fields.locations.clone().map(paths).unwrap_or_default(),
        };
        Self {
            request: Box::new(request),
            responder: Box::new(responder),
            options,
            known,
        }
    }

    /// Takes what the request leaves out of the call from what its news
    /// before told, `known`.
    fn fill(&mut self, known: &Known) {
        if self.known.title.is_none() {
            self.known.title.clone_from(&known.title);
        }
        if self.known.kind.is_none() {
            self.known.kind = known.kind;
        }
        if self.known.locations.is_empty() {
            self.known.locations.clone_from(&known.locations);
        }
    }

    /// The id of the tool call, the same as in its [`ToolCall`]s.
    #[must_use]
    pub fn tool_call_id(&self) -> String {
        self.request.tool_call.tool_call_id.to_string()
    }

    /// The title of the tool call, if the agent gave one, here or in the
    /// call's news before.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.known.title.as_deref()
    }

    /// What kind of tool the call is, if the agent said, here or in the
    /// call's news before.
    #[must_use]
    pub fn kind(&self) -> Option<ToolKind> {
        self.known.kind
    }

    /// The files the call works on, as the agent said, here or in the
    /// call's news before: none when it did not say.
    #[must_use]
    pub fn locations(&self) -> &[PathBuf] {
        &self.known.locations
    }

    /// What the tool call is about to do, if the agent tells it here; it
    /// may have told it in the call's [`ToolCall`]s instead.
    #[must_use]
    pub fn content(&self) -> Option<Vec<ToolCallContent>> {
        self.request
            .tool_call
            .fields
            .content
            .clone()
            .map(translate_content)
    }

    /// The answers the agent offers.
    #[must_use]
    pub fn options(&self) -> &[PermissionOption] {
        &self.options
    }

    /// Sends `answer` to the agent.
    ///
    /// # Errors
    ///
    /// Returns an error if `answer` selects an option that does not exist,
    /// or if the connection fails.
    pub fn answer(self, answer: PermissionAnswer) -> anyhow::Result<()> {
        let outcome = match answer {
            PermissionAnswer::Select(index) => {
                let option = self
                    .request
                    .options
                    .get(index)
                    .with_context(|| format!("there is no option {index}"))?;
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                    option.option_id.clone(),
                ))
            }
            PermissionAnswer::Cancel => RequestPermissionOutcome::Cancelled,
        };
        self.responder
            .respond(RequestPermissionResponse::new(outcome))?;
        Ok(())
    }
}

/// One of the answers to a permission request.
pub struct PermissionOption {
    pub name: String,
    pub kind: PermissionKind,
}

/// What choosing a permission option means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    /// A kind newer than this version of Cox knows.
    Other,
}

impl From<PermissionOptionKind> for PermissionKind {
    fn from(kind: PermissionOptionKind) -> Self {
        match kind {
            PermissionOptionKind::AllowOnce => Self::AllowOnce,
            PermissionOptionKind::AllowAlways => Self::AllowAlways,
            PermissionOptionKind::RejectOnce => Self::RejectOnce,
            PermissionOptionKind::RejectAlways => Self::RejectAlways,
            _ => Self::Other,
        }
    }
}

/// An answer to a permission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionAnswer {
    /// Choose the option at this index in [`PermissionRequest::options`].
    Select(usize),
    /// Decline to choose; the agent treats the tool call as not allowed.
    Cancel,
}

/// Why a turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    /// A reason newer than this version of Cox knows.
    Other,
}

impl From<AcpStopReason> for StopReason {
    fn from(reason: AcpStopReason) -> Self {
        match reason {
            AcpStopReason::EndTurn => Self::EndTurn,
            AcpStopReason::MaxTokens => Self::MaxTokens,
            AcpStopReason::MaxTurnRequests => Self::MaxTurnRequests,
            AcpStopReason::Refusal => Self::Refusal,
            AcpStopReason::Cancelled => Self::Cancelled,
            _ => Self::Other,
        }
    }
}
