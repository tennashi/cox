//! The agent side: runs the ACP connection on a thread with a tokio runtime
//! of its own, since GPUI runs its own executor, and talks to the window
//! over channels. Tokio's channels do not need a tokio runtime to wait on,
//! so the window awaits events on GPUI's executor. What goes over the
//! channels is said here too: the [`Command`]s the window sends, and the
//! [`AgentEvent`]s it hears.
//!
//! - [`instructions`] reads what the agent gets told of the project.
//! - [`mcp`] serves Cox's own tools to each chat's agent, and is the
//!   bridge the agent runs to reach them.
//! - [`tools`] says which MCP servers the agent gets: the stage's, and
//!   Cox's own.

pub mod instructions;
pub mod mcp;
pub mod tools;

use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cox_acp::{
    McpServer, PermissionAnswer, PlanEntry, PlanState, Session, StopReason, ToolCallContent,
    Transport, Update,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use cox_core::entry::{Content, Entry, Step, StepState, ToolCallResult};

use crate::agent::instructions::Instructions;
use crate::process::close_on_exec;

/// What the agent side tells the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// The agent opened the session `id`, to resume later, as `how` says.
    Opened { id: String, how: Opened },
    /// The agent gave the session a title, in place of any it gave before.
    Titled(String),
    /// A piece of the agent's reply.
    ReplyText(String),
    /// A piece of what the agent thinks on its way to the reply.
    ThoughtText(String),
    /// The plan the agent works to, all of it, in place of the plan it
    /// sent before.
    Plan(Vec<Step>),
    /// A tool call the agent made started, was retitled, changed what it
    /// shows, or ended.
    ToolCall {
        /// The same for every event of one call.
        id: String,
        title: Option<String>,
        /// What the call shows from now on, or `None` when that stays as it
        /// was.
        content: Option<Vec<Content>>,
        /// How the call ended, or `None` while it runs.
        result: Option<ToolCallResult>,
    },
    /// The agent asks permission for a tool call; the turn waits for an
    /// [`Command::Answer`], or for a [`Command::Cancel`].
    PermissionRequest {
        /// The id of the tool call, as in its [`AgentEvent::ToolCall`]s.
        id: String,
        title: Option<String>,
        /// What the call is about to do, if the request tells it.
        content: Option<Vec<Content>>,
        options: Vec<String>,
    },
    /// The turn ended.
    TurnEnded(StopReason),
    /// The agent answered the prompt with an error.
    TurnFailed(String),
    /// The connection to the agent closed, with the error that closed it, if
    /// any.
    Closed(Option<String>),
}

/// How the agent's session opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opened {
    /// A new session, for a new chat.
    New,
    /// The session the chat had before, which the agent went on with.
    Resumed,
    /// A new session for a chat that had one before, which the agent
    /// could not go on with, so the agent gets told the conversation so
    /// far, along with the instructions in `instructions`, if any.
    Retold { instructions: Option<PathBuf> },
}

/// What the window asks of the agent side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Prompt(String),
    Answer(PermissionAnswer),
    /// Asks the agent to stop the turn it is taking.
    Cancel,
}

/// The window's ends of the channels to the agent side.
pub struct Agent {
    pub commands: UnboundedSender<Command>,
    pub events: UnboundedReceiver<AgentEvent>,
    /// Where the instructions the agent gets with the first prompt come
    /// from, if it gets any.
    pub instructions: Option<PathBuf>,
    /// Stops the agent when dropped.
    pub thread: AgentThread,
}

/// How a chat's agent starts.
pub enum Start {
    /// For a new chat: the agent gets `instructions`, if any, ahead of the
    /// first prompt.
    New(Option<Instructions>),
    /// For a closed chat the human sends to again: the agent goes on with
    /// the chat's `session`, when the chat had one and the agent can.
    /// Otherwise it opens a new session, which gets `instructions`, if
    /// any, and is told `earlier`, what the chat showed, ahead of the first
    /// prompt.
    Again {
        session: Option<String>,
        instructions: Option<Instructions>,
        earlier: Vec<Entry>,
    },
}

impl Start {
    /// How the agent of a closed chat that worked in `cwd`, and showed
    /// `earlier`, starts again: going on with `session` if it can, or else
    /// told what the chat showed, with the project's instructions as they
    /// are now.
    #[must_use]
    pub fn again(cwd: &Path, session: Option<String>, earlier: &[Entry]) -> Self {
        Self::Again {
            session,
            instructions: Instructions::read(cwd),
            earlier: earlier.to_vec(),
        }
    }
}

/// How long the agent side's tasks get to end once the agent is stopped.
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// The thread the agent side runs on. Dropping it stops the agent, along
/// with every process the agent started, and waits for the thread to end.
///
/// The process ending does not do that by itself: the agent runs in a
/// process group of its own, and its processes outlive Cox unless the
/// connection to it is dropped first, which kills the group.
pub struct AgentThread {
    /// Never sent: dropping it tells the thread to stop.
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for AgentThread {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("the agent's thread panicked");
        }
    }
}

/// Starts the agent side on a thread of its own. `connect` runs on its
/// runtime and returns the transport to the agent, which works in `cwd`, an
/// absolute path, gets the tools of `mcp_servers`, and starts as `start`
/// says.
pub fn spawn<T: Transport + 'static>(
    connect: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    start: Start,
) -> Agent {
    let source = match &start {
        Start::New(instructions) => instructions.as_ref().map(|i| i.path.clone()),
        Start::Again { .. } => None,
    };
    let (commands, received) = unbounded_channel();
    let (sent, events) = unbounded_channel();
    let (stop, stopped) = oneshot::channel();
    let thread = thread::spawn(move || {
        close_on_exec();
        let outcome = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| {
                let outcome = runtime.block_on(async {
                    tokio::select! {
                        outcome = run(connect, cwd, mcp_servers, start, received, &sent) => outcome,
                        // Dropping the connection kills the agent.
                        _ = stopped => Ok(()),
                    }
                });
                // The connection's tasks go with the runtime.
                runtime.shutdown_timeout(STOP_TIMEOUT);
                outcome
            });
        let _ = sent.send(AgentEvent::Closed(outcome.err().map(|e| format!("{e:#}"))));
    });
    Agent {
        commands,
        events,
        instructions: source,
        thread: AgentThread {
            stop: Some(stop),
            thread: Some(thread),
        },
    }
}

async fn run<T: Transport + 'static>(
    connect: impl FnOnce() -> anyhow::Result<T>,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    start: Start,
    mut commands: UnboundedReceiver<Command>,
    events: &UnboundedSender<AgentEvent>,
) -> anyhow::Result<()> {
    let transport = connect()?;
    let resume = match &start {
        Start::New(_) => None,
        Start::Again { session, .. } => session.clone(),
    };
    let told = events.clone();
    let titled = move |title| {
        let _ = told.send(AgentEvent::Titled(title));
    };
    cox_acp::connect(
        transport,
        cwd,
        mcp_servers,
        resume,
        titled,
        async |mut session| {
            let (how, mut ahead) = opened(start, session.resumed());
            let _ = events.send(AgentEvent::Opened {
                id: session.id(),
                how,
            });
            while let Some(command) = commands.recv().await {
                // An answer with no permission request waiting has nothing to
                // answer, and a cancel with no turn running nothing to stop.
                if let Command::Prompt(text) = command {
                    // What goes ahead of the first prompt goes once.
                    let prompt = std::mem::take(&mut ahead).into_iter().chain([text]);
                    take_turn(&mut session, prompt, &mut commands, events).await?;
                }
            }
            Ok(())
        },
    )
    .await
}

/// How the session opened, for an agent that started as `start`, and
/// whether it `resumed` a session, with what goes ahead of the first
/// prompt: the instructions, and what the chat showed, when a session
/// starts anew for a chat that had one.
fn opened(start: Start, resumed: bool) -> (Opened, Vec<String>) {
    match start {
        Start::New(instructions) => (
            Opened::New,
            instructions.map(|i| i.prompt()).into_iter().collect(),
        ),
        Start::Again { .. } if resumed => (Opened::Resumed, Vec::new()),
        Start::Again {
            instructions,
            earlier,
            ..
        } => {
            let how = Opened::Retold {
                instructions: instructions.as_ref().map(|i| i.path.clone()),
            };
            let ahead = instructions
                .map(|i| i.prompt())
                .into_iter()
                .chain([retold(&earlier)])
                .collect();
            (how, ahead)
        }
    }
}

async fn take_turn(
    session: &mut Session,
    prompt: impl IntoIterator<Item = String>,
    commands: &mut UnboundedReceiver<Command>,
    events: &UnboundedSender<AgentEvent>,
) -> anyhow::Result<()> {
    let send = |event| {
        let _ = events.send(event);
    };
    let mut turn = session.send(prompt);
    // Whether the human has asked to stop the turn.
    let mut cancelled = false;
    loop {
        let update = tokio::select! {
            update = turn.next() => update,
            command = commands.recv() => {
                match command {
                    Some(Command::Cancel) if !cancelled => {
                        cancelled = true;
                        turn.cancel()?;
                    }
                    // The window sends no prompt while a turn runs, no
                    // answer while no permission request waits, and one
                    // cancel at most.
                    Some(_) => {}
                    // The window is gone, and the agent goes with it.
                    None => return Ok(()),
                }
                continue;
            }
        };
        let Some(update) = update else {
            break;
        };
        match update {
            Update::ReplyText(text) => send(AgentEvent::ReplyText(text)),
            Update::ThoughtText(text) => send(AgentEvent::ThoughtText(text)),
            Update::Plan(entries) => {
                send(AgentEvent::Plan(entries.into_iter().map(step).collect()));
            }
            Update::ToolCall(call) => send(AgentEvent::ToolCall {
                id: call.id,
                title: call.title,
                content: call.content.map(shown),
                result: call.result.map(ended),
            }),
            // Once the human has asked to stop, the agent is told no
            // without asking them.
            Update::PermissionRequest(request) if cancelled => {
                request.answer(PermissionAnswer::Cancel)?;
            }
            Update::PermissionRequest(request) => {
                send(AgentEvent::PermissionRequest {
                    id: request.tool_call_id(),
                    title: request.title().map(str::to_owned),
                    content: request.content().map(shown),
                    options: request.options().iter().map(|o| o.name.clone()).collect(),
                });
                let answer = loop {
                    match commands.recv().await {
                        Some(Command::Answer(answer)) => break answer,
                        Some(Command::Cancel) => {
                            cancelled = true;
                            turn.cancel()?;
                            break PermissionAnswer::Cancel;
                        }
                        // The window sends no prompt while a turn runs.
                        Some(Command::Prompt(_)) => {}
                        None => break PermissionAnswer::Cancel,
                    }
                };
                request.answer(answer)?;
            }
        }
    }
    match turn.end().await {
        Ok(reason) => send(AgentEvent::TurnEnded(reason)),
        Err(error) => send(AgentEvent::TurnFailed(format!("{error:#}"))),
    }
    Ok(())
}

/// What a tool call shows, made ready for the window here, on the agent
/// side, where working out a large change does not hold up the window.
/// The most of the conversation so far that a new session is told, in
/// characters: past it, the start is left out.
const RETOLD_LENGTH: usize = 100_000;

/// What a new session of a chat's agent is told of the conversation so
/// far, `entries`, when the agent could not go on with the chat's own: what
/// the human said, what the agent replied, and the tool calls it made, by
/// their titles. Cox's notes are left out.
#[must_use]
pub fn retold(entries: &[Entry]) -> String {
    let told: Vec<String> = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Human(text) => Some(format!("[human]\n{text}")),
            Entry::Reply(text) => Some(format!("[you]\n{text}")),
            Entry::ToolCall { title, result, .. } => {
                let result = result.map_or_else(|| "not ended".to_owned(), |r| format!("{r:?}"));
                Some(format!("[your tool call: {title} … {result}]"))
            }
            Entry::Permission {
                title,
                options,
                chosen,
                ..
            } => {
                let chosen = chosen
                    .and_then(|chosen| options.get(chosen))
                    .map_or("nothing", String::as_str);
                Some(format!(
                    "[you asked permission for: {title}; the human chose: {chosen}]"
                ))
            }
            // What the agent thought and planned is its own, and what came
            // of it is told already.
            Entry::Thought(_) | Entry::Plan(_) | Entry::Note(_) => None,
        })
        .collect();
    let mut told = told.join("\n\n");
    let length = told.chars().count();
    if length > RETOLD_LENGTH {
        let kept: String = told.chars().skip(length - RETOLD_LENGTH).collect();
        told = format!("[… the start is left out]\n{kept}");
    }
    format!(
        "This chat goes on from an earlier session of yours, which could not be resumed. \
         This is the conversation so far, as Cox kept it; go on from its end.\n\n{told}"
    )
}

fn shown(content: Vec<ToolCallContent>) -> Vec<Content> {
    content
        .into_iter()
        .map(|content| match content {
            ToolCallContent::Text(text) => Content::Text(text),
            ToolCallContent::Diff {
                path,
                old_text,
                new_text,
            } => Content::Diff {
                path: path.display().to_string(),
                lines: cox_core::diff::lines(old_text.as_deref(), &new_text),
            },
        })
        .collect()
}

/// A step of the agent's plan, as the conversation keeps it.
fn step(entry: PlanEntry) -> Step {
    Step {
        text: entry.text,
        state: match entry.state {
            PlanState::Pending => StepState::Pending,
            PlanState::InProgress => StepState::InProgress,
            PlanState::Completed => StepState::Completed,
        },
    }
}

/// How a tool call ended, as the conversation keeps it.
fn ended(result: cox_acp::ToolCallResult) -> ToolCallResult {
    match result {
        cox_acp::ToolCallResult::Completed => ToolCallResult::Completed,
        cox_acp::ToolCallResult::Failed => ToolCallResult::Failed,
        cox_acp::ToolCallResult::Cancelled => ToolCallResult::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn a_new_session_is_told_the_conversation_so_far() {
        let entries = [
            Entry::Note("Instructions: ~/cox/COX.md".to_owned()),
            Entry::Human("edit".to_owned()),
            Entry::ToolCall {
                id: "call-1".to_owned(),
                title: "Edit x.txt".to_owned(),
                content: Vec::new(),
                result: Some(ToolCallResult::Completed),
            },
            Entry::Permission {
                id: "call-1".to_owned(),
                title: "Edit x.txt".to_owned(),
                content: Vec::new(),
                options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
                chosen: Some(0),
            },
            Entry::Reply("Done".to_owned()),
            Entry::Note("Stopped.".to_owned()),
        ];
        let told = retold(&entries);
        assert!(
            told.ends_with(
                "\n\n[human]\nedit\n\n\
                 [your tool call: Edit x.txt … Completed]\n\n\
                 [you asked permission for: Edit x.txt; the human chose: Allow once]\n\n\
                 [you]\nDone"
            ),
            "{told}"
        );
        assert!(!told.contains("EndTurn"), "{told}");
    }

    #[test]
    fn a_long_conversation_is_told_from_its_end() {
        let long = "あ".repeat(RETOLD_LENGTH);
        let told = retold(&[Entry::Human("first".to_owned()), Entry::Reply(long.clone())]);
        assert!(told.ends_with(&format!("[… the start is left out]\n{long}")));
        assert!(!told.contains("first"));
    }

    /// Whether process `pid` is still running: there, and not a zombie.
    fn running(pid: &str) -> bool {
        let output = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .expect("ps runs");
        let state = String::from_utf8_lossy(&output.stdout);
        output.status.success() && !state.trim_start().starts_with('Z')
    }

    /// Waits up to five seconds for `done`.
    fn wait_until(mut done: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            if Instant::now() > deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
        true
    }

    #[test]
    fn dropping_the_agent_ends_every_process_it_started() {
        let file = std::env::temp_dir().join(format!("cox-agent-{}", std::process::id()));
        // Like `npx`, a launcher whose child does the work. This agent never
        // answers, so only stopping it ends it.
        let script = format!("sleep 60 & echo $! > {}; wait", file.display());
        let agent = spawn(
            move || cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script]),
            std::env::temp_dir(),
            Vec::new(),
            Start::New(None),
        );
        let mut pid = String::new();
        assert!(wait_until(|| {
            pid = std::fs::read_to_string(&file).unwrap_or_default();
            pid.ends_with('\n')
        }));
        let pid = pid.trim();
        assert!(running(pid));

        drop(agent);
        let _ = std::fs::remove_file(&file);
        assert!(
            wait_until(|| !running(pid)),
            "the agent's child outlived it"
        );
    }

    #[test]
    fn the_agent_gets_none_of_coxs_files() {
        let file = std::env::temp_dir().join(format!("cox-agent-files-{}", std::process::id()));
        // A descriptor that stays open across `exec`, like CEF's port.
        let open = std::fs::File::open("/dev/null").unwrap();
        // SAFETY: `open` is a valid descriptor; the copy is closed below.
        let inherited = unsafe { libc::dup(std::os::fd::AsRawFd::as_raw_fd(&open)) };
        assert!(inherited > 2);
        let script = format!(
            "if [ -e /dev/fd/{inherited} ]; then echo has; else echo not; fi > {}",
            file.display()
        );
        let agent = spawn(
            move || cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script]),
            std::env::temp_dir(),
            Vec::new(),
            Start::New(None),
        );
        let mut seen = String::new();
        assert!(wait_until(|| {
            seen = std::fs::read_to_string(&file).unwrap_or_default();
            seen.ends_with('\n')
        }));
        drop(agent);
        // SAFETY: `inherited` is open, and nothing else uses it.
        unsafe { libc::close(inherited) };
        let _ = std::fs::remove_file(&file);
        assert_eq!(seen, "not\n");
    }
}
