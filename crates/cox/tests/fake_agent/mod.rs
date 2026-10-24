//! An ACP agent for tests, connected to Cox in memory. It replies to each
//! prompt with `echo: <prompt>`, a word at a time. When the prompt mentions
//! "permission", it first announces a tool call to run `touch x.txt`, asks
//! permission to run it, and then reports it done. When the
//! prompt mentions "fail", it answers with an error instead. When the prompt
//! mentions "tool", it first runs `ls` as a tool call that completes. When the
//! prompt mentions "wait", it replies `waiting` and goes on until Cox cancels the
//! turn. A turn cancelled, or whose permission request Cox answers as cancelled,
//! ends as cancelled. It records what Cox asked of it and answered so tests can
//! check what Cox did.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, InitializeRequest,
    InitializeResponse, McpServer, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionId, SessionNotification, SessionUpdate, StopReason,
    TextContent, ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};
use agent_client_protocol::{Agent, Channel, Client, ConnectionTo, Error, Result};
use tokio::sync::Notify;

#[derive(Default)]
pub struct FakeAgent {
    sessions_opened: Arc<AtomicUsize>,
    cwd: Arc<Mutex<Option<PathBuf>>>,
    mcp_servers: Arc<Mutex<Vec<McpServer>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
    cancels: Arc<Cancels>,
}

/// The cancels Cox has sent.
#[derive(Default)]
struct Cancels {
    count: AtomicUsize,
    /// Wakes the turn that waits to be cancelled.
    wake: Notify,
}

impl FakeAgent {
    /// Starts serving and returns the end of the connection to give Cox.
    pub fn connect(&self) -> Channel {
        let (cox_end, agent_end) = Channel::duplex();
        tokio::spawn(serve(
            agent_end,
            Arc::clone(&self.sessions_opened),
            Arc::clone(&self.cwd),
            Arc::clone(&self.mcp_servers),
            Arc::clone(&self.prompts),
            Arc::clone(&self.answers),
            Arc::clone(&self.cancels),
        ));
        cox_end
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

async fn serve(
    transport: Channel,
    sessions_opened: Arc<AtomicUsize>,
    cwd: Arc<Mutex<Option<PathBuf>>>,
    mcp_servers: Arc<Mutex<Vec<McpServer>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
    cancels: Arc<Cancels>,
) -> Result<()> {
    let waits = Arc::clone(&cancels);
    Agent
        .builder()
        .name("fake-agent")
        .on_receive_request(
            async |request: InitializeRequest, responder, _cx| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _cx| {
                sessions_opened.fetch_add(1, Ordering::SeqCst);
                *cwd.lock().unwrap() = Some(request.cwd);
                *mcp_servers.lock().unwrap() = request.mcp_servers;
                responder.respond(NewSessionResponse::new("session-1"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                let text = text_of(&request.prompt);
                prompts.lock().unwrap().push(text.clone());
                let answers = Arc::clone(&answers);
                let cancels = Arc::clone(&waits);
                let turn = cx.clone();
                // Asking permission waits for Cox's answer, which arrives
                // through the loop that runs this handler, so wait elsewhere.
                cx.spawn(async move {
                    if text.contains("fail") {
                        return responder
                            .respond_with_error(Error::new(-32603, "the model is unavailable"));
                    }
                    if text.contains("tool") {
                        run_tool(&turn, &request.session_id)?;
                    }
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
                })
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

/// The text blocks of a prompt, joined.
fn text_of(prompt: &[ContentBlock]) -> String {
    prompt
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

/// Sends a piece of the reply.
fn say(cx: &ConnectionTo<Client>, session: &SessionId, text: &str) -> Result<()> {
    cx.send_notification(SessionNotification::new(
        session.clone(),
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            text,
        )))),
    ))
}

/// Announces a tool call to run `touch x.txt` and asks permission to run
/// it, offering to allow or reject it once.
async fn ask_permission(
    cx: &ConnectionTo<Client>,
    session: &SessionId,
) -> Result<RequestPermissionOutcome> {
    cx.send_notification(SessionNotification::new(
        session.clone(),
        SessionUpdate::ToolCall(ToolCall::new("call-1", "touch x.txt")),
    ))?;
    let request = RequestPermissionRequest::new(
        session.clone(),
        ToolCallUpdate::new("call-1", ToolCallUpdateFields::new().title("touch x.txt")),
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

/// Runs `ls` as a tool call: announces it, then reports it running and done.
fn run_tool(cx: &ConnectionTo<Client>, session: &SessionId) -> Result<()> {
    let update = |update| cx.send_notification(SessionNotification::new(session.clone(), update));
    update(SessionUpdate::ToolCall(ToolCall::new("call-2", "ls")))?;
    for status in [ToolCallStatus::InProgress, ToolCallStatus::Completed] {
        update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
            "call-2",
            ToolCallUpdateFields::new().status(status),
        )))?;
    }
    Ok(())
}
