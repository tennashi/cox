//! Cox: a terminal where a human talks to a coding agent.

use std::fmt::Display;
use std::io::Write;
use std::sync::Mutex;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, InitializeRequest, NewSessionRequest, PermissionOptionKind,
    PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionId, SessionNotification, SessionUpdate, StopReason,
    TextContent,
};
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo, Error, Result};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Talks to `agent`: opens a session, then takes one turn for each line of
/// `input` until it ends. In each turn, shows on `output` the reply as it
/// arrives and then how the turn ended. Rejects every permission request
/// the agent makes, and shows that it did.
///
/// # Errors
///
/// Returns an error if the connection to the agent fails, the agent
/// answers a request with an error, or reading or writing fails.
pub async fn run(
    agent: impl ConnectTo<Client>,
    input: impl AsyncBufRead + Unpin + Send,
    output: &mut (impl Write + Send),
) -> Result<()> {
    let screen = Mutex::new(output);
    Client
        .builder()
        .name("cox")
        .on_receive_notification(
            async |notification: SessionNotification, _cx| {
                show_update(&screen, &notification.update)
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async |request: RequestPermissionRequest, responder, _cx| {
                responder.respond(reject(&screen, &request)?)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async |cx: ConnectionTo<Agent>| {
            let session = start(&cx).await?;
            let mut lines = input.lines();
            while let Some(line) = lines
                .next_line()
                .await
                .map_err(Error::into_internal_error)?
            {
                take_turn(&cx, &session, line, &screen).await?;
            }
            Ok(())
        })
        .await
}

/// Initializes the connection and opens a session in the current directory.
async fn start(cx: &ConnectionTo<Agent>) -> Result<SessionId> {
    cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let cwd = std::env::current_dir().map_err(Error::into_internal_error)?;
    let session = cx
        .send_request(NewSessionRequest::new(cwd))
        .block_task()
        .await?;
    Ok(session.session_id)
}

/// Sends `line` to the agent, waits for the turn to end, and shows why it
/// ended. The reply is shown by `show_update` while the turn goes on.
async fn take_turn(
    cx: &ConnectionTo<Agent>,
    session: &SessionId,
    line: String,
    screen: &Mutex<impl Write>,
) -> Result<()> {
    let prompt = PromptRequest::new(
        session.clone(),
        vec![ContentBlock::Text(TextContent::new(line))],
    );
    let response = cx.send_request(prompt).block_task().await?;
    show(
        screen,
        format_args!("\n[{}]\n", stop_reason_name(response.stop_reason)),
    )
}

/// Shows the part of an update the human reads: the text of the reply.
fn show_update(screen: &Mutex<impl Write>, update: &SessionUpdate) -> Result<()> {
    match update {
        SessionUpdate::AgentMessageChunk(ContentChunk {
            content: ContentBlock::Text(text),
            ..
        }) => show(screen, &text.text),
        _ => Ok(()),
    }
}

/// Rejects a permission request and shows that it did. Picks the agent's
/// "reject once" option, or cancels the request if there is none.
fn reject(
    screen: &Mutex<impl Write>,
    request: &RequestPermissionRequest,
) -> Result<RequestPermissionResponse> {
    let title = request
        .tool_call
        .fields
        .title
        .as_deref()
        .unwrap_or("a tool call");
    show(screen, format_args!("\n[rejected: {title}]\n"))?;
    let outcome = match request
        .options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::RejectOnce)
    {
        Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
            option.option_id.clone(),
        )),
        None => RequestPermissionOutcome::Cancelled,
    };
    Ok(RequestPermissionResponse::new(outcome))
}

/// The stop reason as ACP names it on the wire.
fn stop_reason_name(reason: StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        _ => "unknown",
    }
}

/// Writes `text` to the screen right away.
fn show(screen: &Mutex<impl Write>, text: impl Display) -> Result<()> {
    let mut screen = screen.lock().expect("no writer to the screen panicked");
    write!(screen, "{text}").map_err(Error::into_internal_error)?;
    screen.flush().map_err(Error::into_internal_error)
}
