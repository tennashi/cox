//! An ACP agent for tests, connected to Cox in memory. It replies to each
//! prompt with `echo: <prompt>`, a word at a time. When the prompt mentions
//! "permission", it first announces a tool call to run `touch x.txt`, asks
//! permission to run it, and then reports it done. When the
//! prompt mentions "fail", it answers with an error instead. When the prompt
//! mentions "tool", it first runs `ls` as a tool call that completes. It records
//! what Cox asked of it and answered so tests can check what Cox did.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest,
    PromptResponse, RequestPermissionOutcome, RequestPermissionRequest, SessionId,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields,
};
use agent_client_protocol::{Agent, Channel, Client, ConnectionTo, Error, Result};

#[derive(Default)]
pub struct FakeAgent {
    sessions_opened: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
}

impl FakeAgent {
    /// Starts serving and returns the end of the connection to give Cox.
    pub fn connect(&self) -> Channel {
        let (cox_end, agent_end) = Channel::duplex();
        tokio::spawn(serve(
            agent_end,
            Arc::clone(&self.sessions_opened),
            Arc::clone(&self.prompts),
            Arc::clone(&self.answers),
        ));
        cox_end
    }

    /// How many sessions Cox has opened.
    pub fn sessions_opened(&self) -> usize {
        self.sessions_opened.load(Ordering::SeqCst)
    }

    /// The text of each prompt Cox has sent, in order.
    pub fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }

    /// Cox's answer to each permission request, in order.
    pub fn answers(&self) -> Vec<RequestPermissionOutcome> {
        self.answers.lock().unwrap().clone()
    }
}

async fn serve(
    transport: Channel,
    sessions_opened: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<Vec<RequestPermissionOutcome>>>,
) -> Result<()> {
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
            async move |_request: NewSessionRequest, responder, _cx| {
                sessions_opened.fetch_add(1, Ordering::SeqCst);
                responder.respond(NewSessionResponse::new("session-1"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                let text = text_of(&request.prompt);
                prompts.lock().unwrap().push(text.clone());
                let answers = Arc::clone(&answers);
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
                    if text.contains("permission") {
                        let answer = ask_permission(&turn, &request.session_id).await?;
                        answers.lock().unwrap().push(answer);
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
