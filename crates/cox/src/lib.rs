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
    let mut terminal = Terminal::new(input, output);
    cox_acp::connect(agent, async |mut session| {
        while let Some(line) = terminal.ask("> ").await? {
            if line.trim().is_empty() {
                continue;
            }
            take_turn(&mut session, line, &mut terminal).await?;
        }
        Ok(())
    })
    .await
}

/// Sends `line` to the agent and shows what the agent does in the turn: the
/// reply as it arrives, the tool calls it made as they end, and then why the
/// turn ended, or the error the agent answered with. Asks the human to answer
/// the permission requests the agent makes.
async fn take_turn(
    session: &mut Session,
    line: String,
    terminal: &mut Terminal<impl AsyncBufRead + Unpin, impl Write>,
) -> anyhow::Result<()> {
    let mut turn = session.send(line);
    while let Some(update) = turn.next().await {
        match update {
            Update::ReplyText(text) => terminal.show(&text)?,
            Update::PermissionRequest(request) => ask_permission(request, terminal).await?,
            Update::ToolCall(call) => show_tool_call(terminal, &call)?,
        }
    }
    match turn.end().await {
        Ok(reason) => terminal.show_line(format_args!("[{}]", stop_reason_name(reason))),
        Err(error) => terminal.show_line(format_args!("[error: {error}]")),
    }
}

/// Shows a permission request with its options numbered from 1, and answers
/// it with the option whose number the human enters. Asks again until the
/// answer is one of the numbers, and cancels the request if the input ends.
async fn ask_permission(
    request: PermissionRequest,
    terminal: &mut Terminal<impl AsyncBufRead + Unpin, impl Write>,
) -> anyhow::Result<()> {
    let title = request.title().unwrap_or("a tool call");
    terminal.show_line(format_args!("[permission: {title}]"))?;
    for (number, option) in (1..).zip(request.options()) {
        terminal.show_line(format_args!("  {number}. {}", option.name))?;
    }
    loop {
        let Some(line) = terminal.ask("choose> ").await? else {
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
fn show_tool_call(
    terminal: &mut Terminal<impl AsyncBufRead + Unpin, impl Write>,
    call: &ToolCall,
) -> anyhow::Result<()> {
    let title = call.title.as_deref().unwrap_or("a tool call");
    let result = match call.result {
        ToolCallResult::Completed => "completed",
        ToolCallResult::Failed => "failed",
    };
    terminal.show_line(format_args!("[tool: {title} … {result}]"))
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

/// Where the human and cox meet: cox shows text on the screen and reads the
/// lines the human enters. Remembers whether the screen is at the start of
/// a line, so that what goes on a line of its own starts on a new line
/// without leaving a blank line.
struct Terminal<I, O> {
    lines: Lines<I>,
    output: O,
    at_line_start: bool,
}

impl<I: AsyncBufRead + Unpin, O: Write> Terminal<I, O> {
    fn new(input: I, output: O) -> Self {
        Self {
            lines: input.lines(),
            output,
            at_line_start: true,
        }
    }

    /// Shows `prompt` and reads the line the human enters, or `None` if the
    /// input has ended. Either way, the screen is at the start of a line
    /// afterwards: the human's Enter moves it there, and cox does when the
    /// input ends.
    async fn ask(&mut self, prompt: &str) -> anyhow::Result<Option<String>> {
        self.show(prompt)?;
        let line = self.lines.next_line().await?;
        if line.is_none() {
            self.show("\n")?;
        }
        self.at_line_start = true;
        Ok(line)
    }

    /// Shows `line` on a line of its own.
    fn show_line(&mut self, line: impl Display) -> anyhow::Result<()> {
        if !self.at_line_start {
            self.show("\n")?;
        }
        self.show(&format!("{line}\n"))
    }

    /// Writes `text` to the screen right away.
    fn show(&mut self, text: &str) -> anyhow::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.output.write_all(text.as_bytes())?;
        self.output.flush()?;
        self.at_line_start = text.ends_with('\n');
        Ok(())
    }
}
