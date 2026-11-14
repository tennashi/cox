//! A command the agent runs while no window is there to show it, as a
//! chat on a schedule runs one with no human there: coxd runs it on a
//! pseudoterminal of its own, which answers what the program asks of its
//! terminal as a tab's would, and the agent gets what the terminal shows
//! and how the command ended, as from a tab.

use std::path::Path;
use std::time::Duration;

use alacritty_terminal::event::Event;
use alacritty_terminal::event_loop::Msg;
use tokio::sync::oneshot;

use super::colors::color_at;
use super::pty::{Program, Running, START_SIZE};
use super::{START_CELL, ended_of, text_of};
use crate::agent::mcp::Ran;

/// How long a command has to end once it is interrupted, as no one waits
/// for it any more, before its terminal hangs up on it.
const ENDS_WITHIN: Duration = Duration::from_secs(5);

/// Runs `command` in `cwd`, as the agent asks, and answers `answer` once
/// it ends. When the agent stops waiting for it, as when its turn is
/// stopped, the command gets an interrupt, as from Control-C, and then,
/// if it goes on, a hang-up.
pub async fn run_unseen(
    command: String,
    cwd: &Path,
    mut answer: oneshot::Sender<Result<Ran, String>>,
) {
    let Running {
        term,
        pty,
        mut events,
        held: _held,
        ..
    } = match Running::start(Program::Command(command), cwd) {
        Ok(running) => running,
        Err(error) => {
            let _ = answer.send(Err(format!("cannot start a terminal: {error}")));
            return;
        }
    };
    let write = |bytes: Vec<u8>| {
        let _ = pty.send(Msg::Input(bytes.into()));
    };
    let mut status = None;
    let mut heard = |event| match event {
        Event::PtyWrite(text) => write(text.into_bytes()),
        Event::ColorRequest(index, format) => {
            let color = color_at(index, term.lock().colors());
            write(format(color).into_bytes());
        }
        Event::TextAreaSizeRequest(format) => {
            write(format(START_SIZE.window_size(START_CELL.0, START_CELL.1)).into_bytes());
        }
        Event::ChildExit(exited) => status = Some(exited),
        _ => {}
    };
    let waited = loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(Event::Exit) | None => break true,
                Some(event) => heard(event),
            },
            () = answer.closed() => break false,
        }
    };
    if waited {
        let output = text_of(&term.lock());
        let _ = answer.send(Ok(Ran {
            output,
            ended: ended_of(status),
        }));
        return;
    }
    write(b"\x03".to_vec());
    let ended = tokio::time::timeout(ENDS_WITHIN, async {
        while let Some(event) = events.recv().await {
            if matches!(event, Event::Exit) {
                break;
            }
            heard(event);
        }
    })
    .await;
    if ended.is_err() {
        let _ = pty.send(Msg::Shutdown);
    }
}
