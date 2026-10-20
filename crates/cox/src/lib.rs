//! Cox: a terminal where a human talks to a coding agent.

use std::io::Write;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest, SessionId, StopReason,
    TextContent,
};
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo, Error, Result};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Talks to `agent`: opens a session, and if `input` has a line, sends it
/// and shows on `output` how the turn ended.
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
    Client
        .builder()
        .name("cox")
        .connect_with(agent, async |cx: ConnectionTo<Agent>| {
            let session = start(&cx).await?;
            let mut lines = input.lines();
            if let Some(line) = lines
                .next_line()
                .await
                .map_err(Error::into_internal_error)?
            {
                take_turn(&cx, &session, line, output).await?;
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
/// ended.
async fn take_turn(
    cx: &ConnectionTo<Agent>,
    session: &SessionId,
    line: String,
    output: &mut impl Write,
) -> Result<()> {
    let prompt = PromptRequest::new(
        session.clone(),
        vec![ContentBlock::Text(TextContent::new(line))],
    );
    let response = cx.send_request(prompt).block_task().await?;
    writeln!(output, "[{}]", stop_reason_name(response.stop_reason))
        .map_err(Error::into_internal_error)
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
