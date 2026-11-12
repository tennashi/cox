//! How a window reaches coxd (see [`super::process`]): on its socket,
//! starting it first when none answers there, or again when the one that
//! answers is outdated, waiting while the one that answers is of another
//! build and at work, and then carrying what the window tells the
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

/// How often a window comes again to a coxd of another build that was at
/// work.
const AGAIN_EVERY: Duration = Duration::from_secs(2);

/// The daemon a window reached: what to tell it, and what it tells. Once
/// all of `link` is dropped, the daemon lets the window go.
pub struct Connected {
    pub link: Link,
    pub from_daemon: UnboundedReceiver<FromDaemon>,
}

/// Reaches coxd on `socket`, saying `hello`, starting it with `start`
/// when none answers there, or when the one there is outdated. While the
/// one there is of another build and a chat of it is at work, the window
/// comes again until none is, having called `waiting` once. Once the
/// daemon goes, the window hears no more, and `gone` is called.
///
/// # Errors
///
/// When coxd does not start, or another window has it.
pub fn connect(
    socket: &Path,
    hello: &Hello,
    start: &dyn Fn() -> std::io::Result<()>,
    waiting: &dyn Fn(),
    gone: impl FnOnce() + Send + 'static,
) -> anyhow::Result<Connected> {
    let mut restarted = false;
    let mut waited = false;
    loop {
        let stream = if let Ok(stream) = UnixStream::connect(socket) {
            stream
        } else {
            start().context("coxd does not start")?;
            wait_for(socket)?
        };
        match greet(&stream, hello)? {
            (FromDaemon::Joined, reader) => return Ok(carry(stream, reader, gone)),
            (FromDaemon::Taken, _) => bail!("another Cox window has the chats already"),
            (FromDaemon::Outdated, mut reader) => {
                tracing::info!("coxd was outdated, and stops for the new one");
                // It closes once it has stopped listening.
                let mut rest = String::new();
                while reader.read_line(&mut rest).is_ok_and(|read| read > 0) {}
                wait_gone(socket);
                if restarted {
                    bail!("coxd stays outdated");
                }
                restarted = true;
            }
            (FromDaemon::Busy | FromDaemon::Welcome, _) => {
                tracing::warn!("coxd is of another build, at work: the window waits");
                if !waited {
                    waiting();
                    waited = true;
                }
                drop(stream);
                std::thread::sleep(AGAIN_EVERY);
            }
            (other, _) => bail!("coxd answered {other:?}"),
        }
    }
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

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::testing::TempDir;

    /// A coxd on `socket` that answers each window that comes, in turn,
    /// the line of `answers`, counting in `heard` the hellos it heard.
    fn coxd(socket: &Path, answers: &'static [&'static str], heard: &Arc<AtomicUsize>) {
        let _ = std::fs::remove_file(socket);
        let listener = UnixListener::bind(socket).unwrap();
        let heard = Arc::clone(heard);
        std::thread::spawn(move || {
            let mut windows = Vec::new();
            for answer in answers {
                let (stream, _) = listener.accept().unwrap();
                let mut hello = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut hello)
                    .unwrap();
                heard.fetch_add(1, Ordering::SeqCst);
                (&stream)
                    .write_all(format!("{answer}\n").as_bytes())
                    .unwrap();
                windows.push(stream);
            }
            std::thread::park();
        });
    }

    #[test]
    fn a_window_waits_out_a_coxd_that_does_not_say_it_is_of_its_build() {
        let folder = TempDir::new();
        let socket = folder.join("greet.sock");
        let heard = Arc::new(AtomicUsize::new(0));
        coxd(
            &socket,
            &[r#""Welcome""#, r#""Busy""#, r#""Joined""#],
            &heard,
        );
        let waited = AtomicUsize::new(0);

        let connected = connect(
            &socket,
            &Hello::default(),
            &|| Err(std::io::Error::other("coxd runs already")),
            &|| {
                waited.fetch_add(1, Ordering::SeqCst);
            },
            || {},
        );

        assert!(connected.is_ok(), "{:?}", connected.err());
        assert_eq!(
            heard.load(Ordering::SeqCst),
            3,
            "the window comes again, past the coxd that welcomed it as any window, and the busy one"
        );
        assert_eq!(waited.load(Ordering::SeqCst), 1, "the human is told once");
    }

    #[test]
    fn a_window_joins_a_coxd_of_another_build_at_work_that_speaks_as_it_does() {
        let socket = std::env::temp_dir().join(format!("cox-speaks-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let heard = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let hello: Hello = serde_json::from_str(&line).unwrap();
            let answer = hello.answer(None, Some(crate::daemon::wire::SPEAKS), true);
            (&stream)
                .write_all(format!("{}\n", serde_json::to_string(&answer).unwrap()).as_bytes())
                .unwrap();
            (answer, stream)
        });

        let connected = connect(
            &socket,
            &Hello::new(None, Vec::new()),
            &|| Err(std::io::Error::other("coxd runs already")),
            &|| panic!("the window does not wait for a coxd that speaks as it does"),
            || {},
        );

        assert!(connected.is_ok(), "{:?}", connected.err());
        let (answer, _stream) = heard.join().unwrap();
        assert_eq!(answer, FromDaemon::Joined);
        let _ = std::fs::remove_file(&socket);
    }
}
