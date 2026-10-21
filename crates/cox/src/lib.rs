//! Cox: a terminal where a human talks to a coding agent.

use std::fmt::Display;
use std::io::Write;

use cox_acp::{
    PermissionAnswer, PermissionRequest, Session, StopReason, ToolCall, ToolCallResult, Transport,
    Update,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, Lines};

/// Talks to `agent`: opens a session, then takes one turn for each line of
/// `input` until it ends, skipping blank lines. In each turn, shows on
/// `output` the reply as it arrives, each tool call as it ends, and then how
/// the turn ended, or the error the agent answered with. When the agent asks
/// permission for a tool call, shows the options and reads the human's
/// choice from `input`.
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
            take_turn(&mut session, line, &mut lines, output).await?;
        }
        Ok(())
    })
    .await
}

/// Sends `line` to the agent and shows what the agent does in the turn: the
/// reply as it arrives, the tool calls it made as they end, and then why the
/// turn ended, or the error the agent answered with. Asks the human, through
/// `lines`, to answer the permission requests the agent makes.
async fn take_turn(
    session: &mut Session,
    line: String,
    lines: &mut Lines<impl AsyncBufRead + Unpin>,
    output: &mut impl Write,
) -> anyhow::Result<()> {
    let mut turn = session.send(line);
    while let Some(update) = turn.next().await {
        match update {
            Update::ReplyText(text) => show(output, text)?,
            Update::PermissionRequest(request) => ask_permission(request, lines, output).await?,
            Update::ToolCall(call) => show_tool_call(output, &call)?,
        }
    }
    match turn.end().await {
        Ok(reason) => show(output, format_args!("\n[{}]\n", stop_reason_name(reason))),
        Err(error) => show(output, format_args!("\n[error: {error}]\n")),
    }
}

/// Shows a permission request with its options numbered from 1, and answers
/// it with the option whose number the human enters. Asks again until the
/// answer is one of the numbers, and cancels the request if the input ends.
async fn ask_permission(
    request: PermissionRequest,
    lines: &mut Lines<impl AsyncBufRead + Unpin>,
    output: &mut impl Write,
) -> anyhow::Result<()> {
    let title = request.title().unwrap_or("a tool call");
    show(output, format_args!("\n[permission: {title}]\n"))?;
    for (number, option) in (1..).zip(request.options()) {
        show(output, format_args!("  {number}. {}\n", option.name))?;
    }
    loop {
        show(output, "choose> ")?;
        let Some(line) = lines.next_line().await? else {
            return request.answer(PermissionAnswer::Cancel);
        };
        if let Some(index) = chosen_index(&line, request.options().len()) {
            return request.answer(PermissionAnswer::Select(index));
        }
    }
}

/// The index of the option numbered `line`, if it names one of `count`
/// options numbered from 1.
fn chosen_index(line: &str, count: usize) -> Option<usize> {
    let number: usize = line.trim().parse().ok()?;
    (1..=count).contains(&number).then(|| number - 1)
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
