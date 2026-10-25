//! The agent side: runs the ACP connection on a thread with a tokio runtime
//! of its own, since GPUI runs its own executor, and talks to the window
//! over channels. Tokio's channels do not need a tokio runtime to wait on,
//! so the window awaits events on GPUI's executor.

use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cox_acp::{McpServer, PermissionAnswer, Session, ToolCallContent, Transport, Update};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::conversation::{AgentEvent, Command, Content};
use crate::instructions::Instructions;

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
/// absolute path, gets the tools of `mcp_servers`, and gets `instructions`
/// ahead of the first prompt.
pub fn spawn<T: Transport + 'static>(
    connect: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    instructions: Option<Instructions>,
) -> Agent {
    let (source, prompt) = instructions
        .map(|instructions| {
            let prompt = instructions.prompt();
            (instructions.path, prompt)
        })
        .unzip();
    let (commands, received) = unbounded_channel();
    let (sent, events) = unbounded_channel();
    let (stop, stopped) = oneshot::channel();
    let thread = thread::spawn(move || {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        close_on_exec();
        let outcome = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| {
                let outcome = runtime.block_on(async {
                    tokio::select! {
                        outcome = run(connect, cwd, mcp_servers, prompt, received, &sent) => outcome,
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

/// Marks every file Cox has open to close when a program starts, so the
/// program started next, the agent or a terminal's, gets none of them.
/// Rust opens its files that way already, but CEF opens the stage's
/// `DevTools` port without: an agent that outlived Cox would keep the port
/// from the next Cox.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn close_on_exec() {
    let Ok(entries) = std::fs::read_dir("/dev/fd") else {
        return;
    };
    let fds: Vec<libc::c_int> = entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    // Past standard input, output and error, which the agent gets its own of.
    for fd in fds.into_iter().filter(|&fd| fd > 2) {
        // SAFETY: this only sets a flag of the descriptor, and fails
        // harmlessly on one no longer open, like the listing's own.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

async fn run<T: Transport + 'static>(
    connect: impl FnOnce() -> anyhow::Result<T>,
    cwd: PathBuf,
    mcp_servers: Vec<McpServer>,
    mut instructions: Option<String>,
    mut commands: UnboundedReceiver<Command>,
    events: &UnboundedSender<AgentEvent>,
) -> anyhow::Result<()> {
    let transport = connect()?;
    cox_acp::connect(transport, cwd, mcp_servers, async |mut session| {
        while let Some(command) = commands.recv().await {
            // An answer with no permission request waiting has nothing to
            // answer, and a cancel with no turn running nothing to stop.
            if let Command::Prompt(text) = command {
                // The instructions go once, ahead of the first prompt.
                let prompt = instructions.take().into_iter().chain([text]);
                take_turn(&mut session, prompt, &mut commands, events).await?;
            }
        }
        Ok(())
    })
    .await
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
            Update::ToolCall(call) => send(AgentEvent::ToolCall {
                id: call.id,
                title: call.title,
                content: call.content.map(shown),
                result: call.result,
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
fn shown(content: Vec<ToolCallContent>) -> Vec<Content> {
    content.into_iter().map(Content::from).collect()
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use std::process::Command;
    use std::time::Instant;

    use super::*;

    /// Whether process `pid` is still running: there, and not a zombie.
    fn running(pid: &str) -> bool {
        let output = Command::new("ps")
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
            None,
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
            None,
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
