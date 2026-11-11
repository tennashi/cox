//! Talking to an agent over the Agent Client Protocol (ACP), in Cox's terms.
//!
//! [`connect`] opens a [`Session`] with an agent, handing it the
//! [`McpServer`]s whose tools it may use, or resumes one the agent had
//! opened before, when it can. Each [`Session::send`] starts a
//! [`Turn`], which hands over what the agent sends as [`Update`]s until the
//! turn ends, and which [`Turn::cancel`] asks the agent to end early.
//! What the agent sends while no turn runs, [`Session::next_unprompted`]
//! hands over.
//! The [`Setting`]s the agent lets the human choose of the session, such
//! as its model, come with the session, and [`Session::set`] or
//! [`Turn::set`] chooses one. An agent that asks the human to log in, as
//! ACP's `auth_required` error does, offers [`LoginMethod`]s: one the
//! agent carries out itself, through `authenticate`, or one the human
//! carries out in a terminal, running the agent's own command for it.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthCapabilities, AuthMethod, AuthenticateRequest, AvailableCommand,
    AvailableCommandInput, AvailableCommandsUpdate, CancelNotification, ClientCapabilities,
    ConfigOptionUpdate, Content, ContentBlock, ContentChunk, Diff, EmbeddedResource,
    EmbeddedResourceResource, ImageContent, InitializeRequest, LoadSessionRequest,
    McpServer as AcpMcpServer, McpServerStdio, NewSessionRequest, PermissionOptionKind,
    Plan as AcpPlan, PlanEntry as AcpPlanEntry, PlanEntryStatus, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, ResourceLink,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionConfigId, SessionConfigKind,
    SessionConfigOption, SessionConfigOptionCategory, SessionConfigOptionValue,
    SessionConfigSelectOption, SessionConfigSelectOptions, SessionConfigValueId, SessionId,
    SessionInfoUpdate, SessionNotification, SessionUpdate, SetSessionConfigOptionRequest,
    StopReason as AcpStopReason, TextContent, TextResourceContents, ToolCall as AcpToolCall,
    ToolCallContent as AcpToolCallContent, ToolCallId, ToolCallLocation, ToolCallStatus,
    ToolCallUpdate, ToolKind as AcpToolKind,
};
use agent_client_protocol::schema::{MaybeUndefined, ProtocolVersion};
use agent_client_protocol::{
    AcpAgent, Agent, Client, ConnectTo, ConnectionTo, ErrorCode, LineDirection, Responder,
    UntypedMessage,
};
use anyhow::Context;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
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
/// What the agent tells of the session as a whole, whenever it tells it,
/// between turns too, goes to `heard`: each title it gives the session, and
/// its settings each time they change.
///
/// With `resume`, the id of a session the agent opened before, the agent
/// goes on with that session instead, when it can: [`Session::resumed`]
/// tells whether it did. What the agent tells of the session's past while
/// it resumes it is left out: the session starts from there.
///
/// When the agent asks the human to log in before it opens a session,
/// `login` is asked how, with the ways the agent offers and why the last
/// way tried failed, if one did, and the session is asked for again once
/// the human has logged in, until `login` gives up.
///
/// # Errors
///
/// Returns the error `talk` returns, or an error if the connection fails,
/// the agent refuses to open a session, or `login` gives up.
pub async fn connect<T>(
    agent: impl Transport,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    resume: Option<String>,
    heard: impl Fn(News) + Send + Sync,
    login: impl AsyncFn(Vec<LoginMethod>, Option<String>) -> anyhow::Result<Login>,
    talk: impl AsyncFnOnce(Session) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let (updates, received) = unbounded_channel();
    let mut outcome = None;
    Client
        .builder()
        .name("cox")
        .on_receive_notification(
            async |notification: SessionNotification, _cx| {
                // News of the session comes between turns too.
                match notification.update {
                    SessionUpdate::SessionInfoUpdate(SessionInfoUpdate {
                        title: MaybeUndefined::Value(title),
                        ..
                    }) => heard(News::Titled(title)),
                    SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate {
                        config_options,
                        ..
                    }) => heard(News::Settings(settings(config_options))),
                    SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate {
                        available_commands,
                        ..
                    }) => heard(News::Commands(
                        available_commands
                            .into_iter()
                            .map(SlashCommand::from)
                            .collect(),
                    )),
                    update => {
                        if let Some(update) = translate(update) {
                            deliver(&updates, update);
                        }
                    }
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
            let started = match start(&cx, cwd, mcp_servers, resume, &login).await {
                Ok(started) => started,
                Err(error) => {
                    outcome = Some(Err(error));
                    return Ok(());
                }
            };
            let Started {
                id,
                resumed,
                settings,
                login_methods,
                takes,
            } = started;
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
                settings,
                login_methods,
                takes,
                received,
                tool_calls: HashMap::new(),
            };
            outcome = Some(talk(session).await);
            Ok(())
        })
        .await?;
    outcome.context("the connection closed before the session started")?
}

/// A session the agent opened or went on with.
struct Started {
    id: SessionId,
    /// Whether the agent went on with the session asked for.
    resumed: bool,
    /// What the agent lets the human choose of the session.
    settings: Vec<Setting>,
    /// The ways the agent offers to log in.
    login_methods: Vec<LoginMethod>,
    takes: Takes,
}

/// What the agent takes, as it said as the connection started.
#[derive(Clone, Copy, Debug)]
struct Takes {
    /// Whether it takes a file's text in the prompt, rather than a link
    /// to it (ACP's `embeddedContext`).
    files: bool,
    /// Whether it takes pictures in the prompt (ACP's `image`).
    pictures: bool,
    /// Whether it takes a line into the turn it is taking: see
    /// [`Session::steers`].
    lines: bool,
}

/// Initializes the connection, saying Cox runs the agent's command in a
/// terminal for the human to log in, and opens a session working in
/// `roots`, or resumes the session `resume` when the agent can. When the
/// agent asks the human to log in first, `login` says how, and the session
/// is asked for again once they have.
async fn start(
    cx: &ConnectionTo<Agent>,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    resume: Option<String>,
    login: &impl AsyncFn(Vec<LoginMethod>, Option<String>) -> anyhow::Result<Login>,
) -> anyhow::Result<Started> {
    let capabilities = ClientCapabilities::new().auth(AuthCapabilities::new().terminal(true));
    let initialized = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(capabilities))
        .block_task()
        .await?;
    let login_methods: Vec<LoginMethod> = initialized
        .auth_methods
        .iter()
        .map(LoginMethod::from)
        .collect();
    let prompts = &initialized.agent_capabilities.prompt_capabilities;
    let takes = Takes {
        files: prompts.embedded_context,
        pictures: prompts.image,
        lines: initialized
            .meta
            .as_ref()
            .and_then(|meta| meta.get("steering")?.get("supported")?.as_bool())
            .unwrap_or(false),
    };
    let mcp_servers: Vec<AcpMcpServer> = mcp_servers.into_iter().map(Into::into).collect();
    if let Some(id) = resume
        && let Some(options) = resumed(
            cx,
            &initialized.agent_capabilities,
            SessionId::new(id.clone()),
            &cwd,
            &mcp_servers,
        )
        .await
    {
        return Ok(Started {
            id: SessionId::new(id),
            resumed: true,
            settings: settings(options.unwrap_or_default()),
            login_methods,
            takes,
        });
    }
    let mut failed = None;
    let session = loop {
        let request = NewSessionRequest::new(cwd.clone()).mcp_servers(mcp_servers.clone());
        match cx.send_request(request).block_task().await {
            Ok(session) => break session,
            Err(error) if error.code == ErrorCode::AuthRequired => {
                let how = login(login_methods.clone(), failed.take()).await?;
                failed = log_in(cx, how)
                    .await
                    .err()
                    .map(|error| format!("{error:#}"));
            }
            Err(error) => return Err(error.into()),
        }
    };
    Ok(Started {
        id: session.session_id,
        resumed: false,
        settings: settings(session.config_options.unwrap_or_default()),
        login_methods,
        takes,
    })
}

/// Logs in as `how` says: through the agent, or as the human has, in a
/// terminal, already.
async fn log_in(cx: &ConnectionTo<Agent>, how: Login) -> anyhow::Result<()> {
    if let Login::Authenticate(id) = how {
        cx.send_request(AuthenticateRequest::new(id))
            .block_task()
            .await?;
    }
    Ok(())
}

/// A way the agent offers the human to log in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// How to run the agent's command in a terminal for the human to log
    /// in, for a way the human carries out there; `None` for one the agent
    /// carries out itself, through `authenticate`.
    pub terminal: Option<TerminalLogin>,
}

/// What to add to the agent's command, run in a terminal, for the human to
/// log in there: it has logged in once the command exits with 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalLogin {
    pub args: Vec<String>,
    /// Over what the command sets of its environment.
    pub env: BTreeMap<String, String>,
}

impl From<&AuthMethod> for LoginMethod {
    fn from(method: &AuthMethod) -> Self {
        let terminal = match method {
            AuthMethod::Terminal(terminal) => Some(TerminalLogin {
                args: terminal.args.clone(),
                env: terminal.env.clone().into_iter().collect(),
            }),
            _ => None,
        };
        Self {
            id: method.id().to_string(),
            name: method.name().to_owned(),
            description: method.description().map(str::to_owned),
            terminal,
        }
    }
}

/// How the human logged in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Login {
    /// Through the agent, the way of this id, which the agent carries out
    /// when asked to.
    Authenticate(String),
    /// In a terminal, which the human has done.
    Done,
}

/// Whether `error` is the agent's asking the human to log in, as when it
/// refuses a prompt so.
#[must_use]
pub fn needs_login(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<agent_client_protocol::Error>()
            .is_some_and(|error| error.code == ErrorCode::AuthRequired)
    })
}

/// Asks the agent to go on with session `id`, working in `cwd`, with
/// `mcp_servers`, and returns, when it does, the settings it says the
/// session has, if it says. An agent that can resume a session without
/// telling its past is asked to, since the past is not shown again; one
/// that can only load it, telling its past, is asked to load it. An agent
/// that can do neither, or refuses, does not go on with it.
async fn resumed(
    cx: &ConnectionTo<Agent>,
    capabilities: &AgentCapabilities,
    id: SessionId,
    cwd: &std::path::Path,
    mcp_servers: &[AcpMcpServer],
) -> Option<Option<Vec<SessionConfigOption>>> {
    if capabilities.session_capabilities.resume.is_some() {
        let request = ResumeSessionRequest::new(id, cwd).mcp_servers(mcp_servers.to_vec());
        let response = cx.send_request(request).block_task().await.ok()?;
        Some(response.config_options)
    } else if capabilities.load_session {
        let request = LoadSessionRequest::new(id, cwd).mcp_servers(mcp_servers.to_vec());
        let response = cx.send_request(request).block_task().await.ok()?;
        Some(response.config_options)
    } else {
        None
    }
}

/// Asks the agent to set the setting `id` of `session` to `value`, and
/// returns all the settings, as they are then.
async fn set(
    cx: &ConnectionTo<Agent>,
    session: &SessionId,
    id: &str,
    value: &str,
) -> anyhow::Result<Vec<Setting>> {
    let request = SetSessionConfigOptionRequest::new(
        session.clone(),
        SessionConfigId::new(id),
        SessionConfigOptionValue::value_id(SessionConfigValueId::new(value)),
    );
    let response = cx.send_request(request).block_task().await?;
    Ok(settings(response.config_options))
}

/// The name of `category`, as ACP writes it.
fn category(category: SessionConfigOptionCategory) -> String {
    match category {
        SessionConfigOptionCategory::Mode => "mode".to_owned(),
        SessionConfigOptionCategory::Model => "model".to_owned(),
        SessionConfigOptionCategory::ModelConfig => "model_config".to_owned(),
        SessionConfigOptionCategory::ThoughtLevel => "thought_level".to_owned(),
        SessionConfigOptionCategory::Other(name) => name,
        // A category newer than this version of Cox knows.
        _ => String::new(),
    }
}

/// The settings of `options` that Cox shows: those that choose one of a
/// list. A toggle, which an agent offers only to a client that says it
/// shows one, or a kind newer than this version of Cox knows, is left out.
fn settings(options: Vec<SessionConfigOption>) -> Vec<Setting> {
    options
        .into_iter()
        .filter_map(|option| {
            let SessionConfigKind::Select(select) = option.kind else {
                return None;
            };
            let choice = |group: Option<&str>| {
                let group = group.map(str::to_owned);
                move |value: SessionConfigSelectOption| Choice {
                    id: value.value.0.to_string(),
                    name: value.name,
                    description: value.description,
                    group: group.clone(),
                }
            };
            let choices = match select.options {
                SessionConfigSelectOptions::Ungrouped(values) => {
                    values.into_iter().map(choice(None)).collect()
                }
                SessionConfigSelectOptions::Grouped(groups) => groups
                    .into_iter()
                    .flat_map(|group| {
                        let choice = choice(Some(&group.name));
                        group.options.into_iter().map(choice)
                    })
                    .collect(),
                _ => Vec::new(),
            };
            Some(Setting {
                id: option.id.0.to_string(),
                name: option.name,
                description: option.description,
                category: option.category.map(category),
                value: select.current_value.0.to_string(),
                choices,
            })
        })
        .collect()
}

/// What the connection hands to the session: an update ready to show, or
/// news of a tool call, which the session gathers until the call ends.
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

/// Hands `received` to the session: to the turn that is running, or to
/// [`Session::next_unprompted`] while none runs. The receiver lives as long
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

/// What the agent tells of a session as a whole, between turns too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum News {
    /// The agent gave the session a title, in place of any it gave before.
    Titled(String),
    /// The session's settings changed, as the agent changes them on its
    /// own: all of them, as they are now.
    Settings(Vec<Setting>),
    /// The commands the agent runs when a prompt starts with `/` and one
    /// of their names, all of them, in place of those it told before.
    Commands(Vec<SlashCommand>),
}

/// A command the agent runs when the human's line starts with `/` and its
/// name, as `/compact`, with what follows the name as its input: ACP's
/// available command.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SlashCommand {
    /// Its name, without the `/`.
    pub name: String,
    /// What it does, for the human.
    pub description: String,
    /// What to write after the name, for a command that takes input, as
    /// the agent hints it.
    pub hint: Option<String>,
}

impl From<AvailableCommand> for SlashCommand {
    fn from(command: AvailableCommand) -> Self {
        let hint = command.input.and_then(|input| match input {
            AvailableCommandInput::Unstructured(input) => Some(input.hint),
            // A kind of input Cox does not know yet hints nothing.
            _ => None,
        });
        Self {
            // Some agents name their commands with the `/`.
            name: command
                .name
                .strip_prefix('/')
                .unwrap_or(&command.name)
                .to_owned(),
            description: command.description,
            hint,
        }
    }
}

/// Something of a session that the agent lets the human choose, such as
/// the model it uses: ACP's session config option, of the kind that
/// chooses one of a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
    /// The id the agent knows it by, to set it.
    pub id: String,
    /// What the human knows it by, as `Model`.
    pub name: String,
    pub description: Option<String>,
    /// What it is, as ACP names it, for a client that treats some apart,
    /// such as `model`, `mode` or `thought_level`, if the agent says.
    pub category: Option<String>,
    /// The id of the choice it is set to.
    pub value: String,
    /// What it can be set to.
    pub choices: Vec<Choice>,
}

/// One of the values a [`Setting`] can be set to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The id the agent knows it by.
    pub id: String,
    /// What the human knows it by.
    pub name: String,
    pub description: Option<String>,
    /// The name of the group of choices it is listed under, if the agent
    /// groups them.
    pub group: Option<String>,
}

/// A conversation with an agent.
pub struct Session {
    cx: ConnectionTo<Agent>,
    id: SessionId,
    resumed: bool,
    settings: Vec<Setting>,
    login_methods: Vec<LoginMethod>,
    takes: Takes,
    received: UnboundedReceiver<Received>,
    /// What is known of each tool call that has not ended yet.
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

    /// What the agent lets the human choose of the session, as it was when
    /// the session opened.
    #[must_use]
    pub fn settings(&self) -> &[Setting] {
        &self.settings
    }

    /// Whether the agent takes a line the human sends while it takes a
    /// turn into that turn, as it goes ([`Turn::steer`]), rather than
    /// only as a turn of its own once the turn ends: an agent that does
    /// says so as the connection starts (`_meta.steering.supported`, as
    /// `claude-agent-acp` says it).
    #[must_use]
    pub fn steers(&self) -> bool {
        self.takes.lines
    }

    /// The ways the agent offers the human to log in.
    #[must_use]
    pub fn login_methods(&self) -> &[LoginMethod] {
        &self.login_methods
    }

    /// Logs in as `how` says, as when the agent refused a prompt until the
    /// human does.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent fails to log in, or the connection
    /// fails.
    pub async fn log_in(&self, how: Login) -> anyhow::Result<()> {
        log_in(&self.cx, how).await
    }

    /// Asks the agent to set the setting `id` to `value`, and returns all
    /// the settings, as they are then: setting one, such as the model, can
    /// change what others can be.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent refuses, or the connection fails.
    pub async fn set(&mut self, id: &str, value: &str) -> anyhow::Result<Vec<Setting>> {
        let settings = set(&self.cx, &self.id, id, value).await?;
        self.settings.clone_from(&settings);
        Ok(settings)
    }

    /// The next thing the agent sends while no turn runs, as when it goes
    /// on on its own once work it left running ends: `claude-agent-acp`
    /// does, when a task Claude Code ran in the background ends after the
    /// turn that started it. Waits until the agent sends something, and
    /// answers `None` once the connection has closed. Cancelled, it loses
    /// nothing.
    pub async fn next_unprompted(&mut self) -> Option<Update> {
        loop {
            let received = self.received.recv().await?;
            if let Some(update) = follow(&mut self.tool_calls, received) {
                return Some(update);
            }
        }
    }

    /// Asks the agent to stop what it does on its own while no turn runs
    /// (see [`Session::next_unprompted`]), as [`Turn::cancel`] does a turn.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    pub fn cancel(&self) -> anyhow::Result<()> {
        self.cx
            .send_notification(CancelNotification::new(self.id.clone()))?;
        Ok(())
    }

    /// Sends `blocks` to the agent as a prompt, starting a turn. What the
    /// agent sent before, and was not taken with
    /// [`Session::next_unprompted`], comes first in the turn.
    pub fn send(&mut self, blocks: impl IntoIterator<Item = Block>) -> Turn<'_> {
        let prompt = PromptRequest::new(self.id.clone(), content(blocks, self.takes));
        self.tool_calls.clear();
        Turn {
            cx: &self.cx,
            session: &self.id,
            takes: self.takes,
            received: &mut self.received,
            tool_calls: &mut self.tool_calls,
            response: Box::pin(self.cx.send_request(prompt).block_task()),
            answer: None,
        }
    }
}

/// `blocks` as the agent `takes` them.
fn content(blocks: impl IntoIterator<Item = Block>, takes: Takes) -> Vec<ContentBlock> {
    blocks
        .into_iter()
        .map(|block| match block {
            Block::Text(text) => ContentBlock::Text(TextContent::new(text)),
            Block::File(path) => file_block(&path, takes.files),
            Block::Quote { uri, text } => quote_block(uri, text, takes.files),
            Block::Picture(path) => picture_block(&path, takes.pictures),
        })
        .collect()
}

/// The method of the request that steers a turn, as `claude-agent-acp`
/// names it.
const STEER: &str = "_session/steering";

/// How a line steered into a turn went (see [`Turn::steer`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steered {
    /// The agent took it into the turn, which goes on with it.
    Taken,
    /// The turn was ending, so the agent did not take it: it goes as a
    /// prompt of its own once the turn has ended.
    TooLate,
}

/// A block of a prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Text(String),
    /// A file the human points the agent to, by its absolute path.
    File(PathBuf),
    /// Text the human hands the agent from elsewhere, with where it is
    /// from, as a URI: the lines of a file, as `file:///src/main.rs#L3-5`,
    /// a page's address, or what in Cox shows it.
    Quote {
        uri: String,
        text: String,
    },
    /// A picture the human hands the agent, as a PNG file, by its
    /// absolute path.
    Picture(PathBuf),
}

impl From<String> for Block {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

/// The largest file whose text goes in the prompt: a larger one goes as
/// a link, for the agent to read as much of it as it wants.
const LARGEST_EMBEDDED: u64 = 256 * 1024;

/// The file at `path` as a block of a prompt: its text, when the agent
/// `embeds` files and it is text, and not too large, or else a link to
/// it.
fn file_block(path: &Path, embeds: bool) -> ContentBlock {
    let uri = file_uri(path);
    let text = embeds
        .then(|| std::fs::metadata(path).ok())
        .flatten()
        .filter(|metadata| metadata.len() <= LARGEST_EMBEDDED)
        .and_then(|_| std::fs::read(path).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok());
    if let Some(text) = text {
        return ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(text, uri)),
        ));
    }
    let name = path
        .file_name()
        .map_or_else(|| uri.clone(), |name| name.to_string_lossy().into_owned());
    ContentBlock::ResourceLink(ResourceLink::new(name, uri))
}

/// `text`, from `uri`, as a block of a prompt: a resource of that URI with
/// that text, when the agent `embeds` resources, or else the text after a
/// line that says where it is from, fenced so that what it says stays
/// apart from the line.
fn quote_block(uri: String, text: String, embeds: bool) -> ContentBlock {
    if embeds {
        return ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(text, uri)),
        ));
    }
    let fence = "`".repeat(longest_run(&text, '`').max(2) + 1);
    ContentBlock::Text(TextContent::new(format!(
        "From {uri}:\n\n{fence}\n{text}\n{fence}"
    )))
}

/// The PNG at `path` as a block of a prompt: the picture, when the agent
/// takes `images`, or else a link to the file, for the agent to read.
fn picture_block(path: &Path, images: bool) -> ContentBlock {
    let uri = file_uri(path);
    if images && let Ok(png) = std::fs::read(path) {
        return ContentBlock::Image(ImageContent::new(STANDARD.encode(png), "image/png").uri(uri));
    }
    let name = path
        .file_name()
        .map_or_else(|| uri.clone(), |name| name.to_string_lossy().into_owned());
    ContentBlock::ResourceLink(ResourceLink::new(name, uri).mime_type("image/png"))
}

/// How many of `letter` the longest run of it in `text` has.
fn longest_run(text: &str, letter: char) -> usize {
    text.split(|other| other != letter)
        .map(str::len)
        .max()
        .unwrap_or(0)
}

/// The `file:` URI of the absolute path `path`.
#[must_use]
pub fn file_uri(path: &Path) -> String {
    url::Url::from_file_path(path)
        .map_or_else(|()| format!("file://{}", path.display()), String::from)
}

/// One prompt and everything the agent does in answer to it.
pub struct Turn<'s> {
    cx: &'s ConnectionTo<Agent>,
    session: &'s SessionId,
    takes: Takes,
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
            if let Some(update) = follow(self.tool_calls, received) {
                return Some(update);
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

    /// Hands the agent `blocks`, a line the human sent while the turn
    /// runs, to take into it, for an agent that [`Session::steers`]: the
    /// agent goes on with the line in the turn, which ends once it has
    /// answered it too. It does not start a turn of its own when the turn
    /// is ending: the answer says so, for the line to go as a prompt.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent refuses, or the connection fails.
    pub async fn steer(&self, blocks: impl IntoIterator<Item = Block>) -> anyhow::Result<Steered> {
        let params = serde_json::json!({
            "sessionId": self.session,
            "prompt": content(blocks, self.takes),
            "_meta": { "steering": { "idleBehavior": "promptRequired" } },
        });
        let answer = self
            .cx
            .send_request(UntypedMessage::new(STEER, params)?)
            .block_task()
            .await?;
        steered(&answer)
    }

    /// Asks the agent to set the setting `id` to `value` while the turn
    /// runs, as [`Session::set`] does.
    ///
    /// # Errors
    ///
    /// Returns an error if the agent refuses, or the connection fails.
    pub async fn set(&self, id: &str, value: &str) -> anyhow::Result<Vec<Setting>> {
        set(self.cx, self.session, id, value).await
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

/// Takes `received`, what the agent sent, given what is known of the
/// tool calls that have not ended, and returns what there is to show of
/// it, if anything.
fn follow(tool_calls: &mut HashMap<ToolCallId, Known>, received: Received) -> Option<Update> {
    match received {
        Received::Update(Update::PermissionRequest(mut request)) => {
            if let Some(known) = tool_calls.get(&request.request.tool_call.tool_call_id) {
                request.fill(known);
            }
            Some(Update::PermissionRequest(request))
        }
        Received::Update(update) => Some(update),
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
            follow_tool_call(tool_calls, id, told, content, status).map(Update::ToolCall)
        }
    }
}

/// Records news of a tool call, what it is `told` to be, and returns the
/// call when there is something new to show: it started, its title or
/// content changed, or it ended. A kind or locations not told stay as
/// they were.
fn follow_tool_call(
    tool_calls: &mut HashMap<ToolCallId, Known>,
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
        let known = tool_calls.remove(&id).and_then(|known| known.title);
        return Some(ToolCall {
            id: shown_id,
            title: title.or(known),
            content,
            result,
        });
    }
    let started = !tool_calls.contains_key(&id);
    let known = tool_calls.entry(id).or_default();
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

/// How the agent `answer`ed a line steered into a turn.
fn steered(answer: &serde_json::Value) -> anyhow::Result<Steered> {
    match answer["outcome"].as_str() {
        Some("injected") => Ok(Steered::Taken),
        Some("promptRequired") => Ok(Steered::TooLate),
        _ => anyhow::bail!("the agent answered the steered line with {answer}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_goes_as_a_resource_to_an_agent_that_embeds() {
        let block = quote_block("file:///a.rs#L2-3".to_owned(), "b\nc".to_owned(), true);
        let ContentBlock::Resource(resource) = block else {
            panic!("a resource: {block:?}");
        };
        let EmbeddedResourceResource::TextResourceContents(text) = resource.resource else {
            panic!("text");
        };
        assert_eq!(text.uri, "file:///a.rs#L2-3");
        assert_eq!(text.text, "b\nc");
    }

    #[test]
    fn a_quote_goes_as_fenced_text_to_an_agent_that_does_not_embed() {
        let block = quote_block("cox://terminal".to_owned(), "a ```b```".to_owned(), false);
        let ContentBlock::Text(text) = block else {
            panic!("text: {block:?}");
        };
        assert_eq!(text.text, "From cox://terminal:\n\n````\na ```b```\n````");
    }

    #[test]
    fn a_picture_goes_whole_to_an_agent_that_takes_images() {
        let path = std::env::temp_dir().join(format!("cox-acp-picture-{}.png", std::process::id()));
        std::fs::write(&path, b"png").unwrap();
        let block = picture_block(&path, true);
        std::fs::remove_file(&path).unwrap();
        let ContentBlock::Image(image) = block else {
            panic!("an image: {block:?}");
        };
        assert_eq!(image.data, STANDARD.encode(b"png"));
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.uri, Some(file_uri(&path)));
    }

    #[test]
    fn a_picture_goes_as_a_link_to_an_agent_that_does_not_take_images() {
        let block = picture_block(Path::new("/kept/1.png"), false);
        let ContentBlock::ResourceLink(link) = block else {
            panic!("a link: {block:?}");
        };
        assert_eq!(link.name, "1.png");
        assert_eq!(link.uri, "file:///kept/1.png");
        assert_eq!(link.mime_type.as_deref(), Some("image/png"));
    }
}
