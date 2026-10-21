//! The agent side: runs the ACP connection on a thread with a tokio runtime
//! of its own, since GPUI runs its own executor, and talks to the window
//! over channels. Tokio's channels do not need a tokio runtime to wait on,
//! so the window awaits events on GPUI's executor.

use std::thread;

use cox_acp::{PermissionAnswer, Session, Transport, Update};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::conversation::{AgentEvent, Command};

/// The window's ends of the channels to the agent side.
pub struct Agent {
    pub commands: UnboundedSender<Command>,
    pub events: UnboundedReceiver<AgentEvent>,
}

/// Starts the agent side on a thread of its own. `connect` runs on its
/// runtime and returns the transport to the agent.
pub fn spawn<T: Transport + 'static>(
    connect: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Agent {
    let (commands, received) = unbounded_channel();
    let (sent, events) = unbounded_channel();
    thread::spawn(move || {
        let outcome = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| runtime.block_on(run(connect, received, &sent)));
        let _ = sent.send(AgentEvent::Closed(outcome.err().map(|e| format!("{e:#}"))));
    });
    Agent { commands, events }
}

async fn run<T: Transport + 'static>(
    connect: impl FnOnce() -> anyhow::Result<T>,
    mut commands: UnboundedReceiver<Command>,
    events: &UnboundedSender<AgentEvent>,
) -> anyhow::Result<()> {
    let transport = connect()?;
    cox_acp::connect(transport, async |mut session| {
        while let Some(command) = commands.recv().await {
            // An answer with no permission request waiting has nothing to answer.
            if let Command::Prompt(text) = command {
                take_turn(&mut session, text, &mut commands, events).await?;
            }
        }
        Ok(())
    })
    .await
}

async fn take_turn(
    session: &mut Session,
    text: String,
    commands: &mut UnboundedReceiver<Command>,
    events: &UnboundedSender<AgentEvent>,
) -> anyhow::Result<()> {
    let send = |event| {
        let _ = events.send(event);
    };
    let mut turn = session.send(text);
    while let Some(update) = turn.next().await {
        match update {
            Update::ReplyText(text) => send(AgentEvent::ReplyText(text)),
            Update::ToolCall(call) => send(AgentEvent::ToolCall {
                title: call.title,
                result: call.result,
            }),
            Update::PermissionRequest(request) => {
                send(AgentEvent::PermissionRequest {
                    title: request.title().map(str::to_owned),
                    options: request.options().iter().map(|o| o.name.clone()).collect(),
                });
                let answer = loop {
                    match commands.recv().await {
                        Some(Command::Answer(answer)) => break answer,
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
