//! Cox: a terminal where a human talks to a coding agent.

use std::fmt::Display;
use std::io::Write;

use cox_acp::{
    PermissionAnswer, PermissionKind, PermissionRequest, Session, StopReason, ToolCall,
    ToolCallResult, Transport, Update,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Talks to `agent`: opens a session, then takes one turn for each line of
/// `input` until it ends, skipping blank lines. In each turn, shows on
/// `output` the reply as it arrives, each tool call as it ends, and then how
/// the turn ended, or the error the agent answered with. Rejects every permission request the agent makes,
/// and shows that it did.
///
/// # Errors
///
/// Returns an error if the connection to the agent fails, the agent refuses
/// to open a session, or reading or writing fails.
pub async fn run(
    agent: impl Transport,
    input: impl AsyncBufRead + Unpin + Send,
    output: &mut (impl Write + Send),
) -> anyhow::Result<()> {
    cox_acp::connect(agent, async |mut session| {
        let mut lines = input.lines();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            take_turn(&mut session, line, output).await?;
        }
        Ok(())
    })
    .await
}

/// Sends `line` to the agent and shows what the agent does in the turn: the
/// reply as it arrives, the tool calls it made as they end, the permission
/// requests it rejects, and then why the turn ended, or the error the agent
/// answered with.
async fn take_turn(
    session: &mut Session,
    line: String,
    output: &mut impl Write,
) -> anyhow::Result<()> {
    let mut turn = session.send(line);
    while let Some(update) = turn.next().await {
        match update {
            Update::ReplyText(text) => show(output, text)?,
            Update::PermissionRequest(request) => reject(output, request)?,
            Update::ToolCall(call) => show_tool_call(output, &call)?,
        }
    }
    match turn.end().await {
        Ok(reason) => show(output, format_args!("\n[{}]\n", stop_reason_name(reason))),
        Err(error) => show(output, format_args!("\n[error: {error}]\n")),
    }
}

/// Shows a tool call that has ended, on one line.
fn show_tool_call(output: &mut impl Write, call: &ToolCall) -> anyhow::Result<()> {
    let title = call.title.as_deref().unwrap_or("a tool call");
    let result = match call.result {
        ToolCallResult::Completed => "completed",
        ToolCallResult::Failed => "failed",
    };
    show(output, format_args!("\n[tool: {title} … {result}]\n"))
}

/// Rejects a permission request and shows that it did. Picks the agent's
/// "reject once" option, or cancels the request if there is none.
fn reject(output: &mut impl Write, request: PermissionRequest) -> anyhow::Result<()> {
    let title = request.title().unwrap_or("a tool call");
    show(output, format_args!("\n[rejected: {title}]\n"))?;
    let answer = match request
        .options()
        .iter()
        .position(|option| option.kind == PermissionKind::RejectOnce)
    {
        Some(index) => PermissionAnswer::Select(index),
        None => PermissionAnswer::Cancel,
    };
    request.answer(answer)
}

/// The stop reason as ACP names it on the wire.
fn stop_reason_name(reason: StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        StopReason::Other => "unknown",
    }
}

/// Writes `text` to the screen right away.
fn show(output: &mut impl Write, text: impl Display) -> anyhow::Result<()> {
    write!(output, "{text}")?;
    output.flush()?;
    Ok(())
}
