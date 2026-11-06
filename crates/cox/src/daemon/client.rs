//! How a window reaches coxd (see [`super::process`]): on its socket,
//! starting it first when none answers there, or again when the one that
//! answers is outdated, and then carrying what the window tells the
//! daemon, and what the daemon tells it, a line each, on threads of its
//! own.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::Link;
use super::wire::{FromDaemon, Hello, ToDaemon};

/// How long a window waits for a coxd it started to answer.
const STARTS_WITHIN: Duration = Duration::from_secs(10);

/// How often it looks whether it does.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// The daemon a window reached: what to tell it, and what it tells. Once
/// all of `link` is dropped, the daemon lets the window go.
pub struct Connected {
    pub link: Link,
    pub from_daemon: UnboundedReceiver<FromDaemon>,
}

/// Reaches coxd on `socket`, saying `hello`, starting it with `start`
/// when none answers there, or when the one there is outdated. Once the
/// daemon goes, the window hears no more, and `gone` is called.
///
/// # Errors
///
/// When coxd does not start, or another window has it.
pub fn connect(
    socket: &Path,
    hello: &Hello,
    start: &dyn Fn() -> std::io::Result<()>,
    gone: impl FnOnce() + Send + 'static,
) -> anyhow::Result<Connected> {
    // The outdated coxd stops, and the new one is started, once.
    for _ in 0..2 {
        let stream = if let Ok(stream) = UnixStream::connect(socket) {
            stream
        } else {
            start().context("coxd does not start")?;
            wait_for(socket)?
        };
        match greet(&stream, hello)? {
            (FromDaemon::Welcome, reader) => return Ok(carry(stream, reader, gone)),
            (FromDaemon::Taken, _) => bail!("another Cox window has the chats already"),
            (FromDaemon::Outdated, mut reader) => {
                tracing::info!("coxd was outdated, and stops for the new one");
                // It closes once it has stopped listening.
                let mut rest = String::new();
                while reader.read_line(&mut rest).is_ok_and(|read| read > 0) {}
                wait_gone(socket);
            }
            (other, _) => bail!("coxd answered {other:?}"),
        }
    }
    bail!("coxd stays outdated")
}

/// Waits until coxd answers on `socket`.
fn wait_for(socket: &Path) -> anyhow::Result<UnixStream> {
    let until = Instant::now() + STARTS_WITHIN;
    loop {
        if let Ok(stream) = UnixStream::connect(socket) {
            return Ok(stream);
        }
        if Instant::now() > until {
            bail!("coxd did not answer on {}", socket.display());
        }
        std::thread::sleep(LOOK_EVERY);
    }
}

/// Waits until no coxd answers on `socket`.
fn wait_gone(socket: &Path) {
    let until = Instant::now() + STARTS_WITHIN;
    while UnixStream::connect(socket).is_ok() && Instant::now() < until {
        std::thread::sleep(LOOK_EVERY);
    }
}

/// Says `hello` on `stream`, and answers what the daemon said to it, with
/// what reads the rest.
fn greet(
    stream: &UnixStream,
    hello: &Hello,
) -> anyhow::Result<(FromDaemon, BufReader<UnixStream>)> {
    let mut line = serde_json::to_vec(hello)?;
    line.push(b'\n');
    (&*stream).write_all(&line).context("coxd cannot be told")?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut answer = String::new();
    reader
        .read_line(&mut answer)
        .context("coxd cannot be heard")?;
    let answer = serde_json::from_str(&answer).context("coxd answered what cannot be read")?;
    Ok((answer, reader))
}

/// Carries what the window tells the daemon on `stream`, and what the
/// daemon tells it, read with `reader`, each on a thread of its own,
/// calling `gone` once the daemon has gone.
fn carry(
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    gone: impl FnOnce() + Send + 'static,
) -> Connected {
    let (to, mut told) = unbounded_channel::<ToDaemon>();
    let (heard, from_daemon) = unbounded_channel();
    std::thread::Builder::new()
        .name("to coxd".to_owned())
        .spawn(move || {
            let mut stream = stream;
            while let Some(message) = told.blocking_recv() {
                let Ok(mut line) = serde_json::to_vec(&message) else {
                    continue;
                };
                line.push(b'\n');
                if let Err(error) = stream.write_all(&line) {
                    tracing::error!("coxd cannot be told: {error}");
                    break;
                }
            }
            // The window is done: coxd lets it go, though the thread from
            // coxd still holds the socket.
            let _ = stream.shutdown(std::net::Shutdown::Write);
        })
        .expect("the thread to coxd starts");
    std::thread::Builder::new()
        .name("from coxd".to_owned())
        .spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else { break };
                match serde_json::from_str::<FromDaemon>(&line) {
                    Ok(message) => {
                        if heard.send(message).is_err() {
                            return;
                        }
                    }
                    Err(error) => tracing::error!("coxd said what cannot be read: {error}"),
                }
            }
            tracing::error!("coxd is gone");
            gone();
        })
        .expect("the thread from coxd starts");
    Connected {
        link: Link { to },
        from_daemon,
    }
}
