//! coxd: the daemon as a process of its own, `cox --daemon`, which runs
//! the chats whether a window shows them or not, and goes on after the
//! window closes, until it is stopped (SIGTERM, SIGINT or SIGHUP) or its
//! socket is taken away.
//!
//! A window reaches it on a Unix socket, `run/coxd.sock` in Cox's data
//! folder (see [`crate::storage`]), one message a line, each a JSON
//! object: the window says its [`Hello`] first, and the daemon answers
//! [`FromDaemon::Welcome`], after which each line the window writes is a
//! [`ToDaemon`] and each the daemon writes a [`FromDaemon`] (see
//! [`super::wire`]). One window at a time: another is answered
//! [`FromDaemon::Taken`]. When the program coxd runs was replaced since
//! it started, as `cargo xtask install` replaces it, and no chat is at
//! work, the next window is answered [`FromDaemon::Outdated`], and coxd
//! stops, for the window to start the new one (see [`super::client`]).

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use cox_core::store::Store;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use super::wire::{FromDaemon, Hello, ToDaemon};
use super::{Daemon, Internal, Spawns};

/// How often coxd looks whether it was asked to stop, and whether its
/// socket is still where a window finds it.
const LOOK_EVERY: Duration = Duration::from_millis(500);

/// Runs coxd on `socket` until it is stopped: the chats kept in `store`,
/// those in no project working in `home`, their agents started by
/// `spawns`, all made on this thread.
///
/// # Errors
///
/// When the socket cannot be listened on, or when another coxd answers on
/// it already.
pub fn run(
    socket: &Path,
    home: PathBuf,
    store: impl FnOnce() -> Rc<Store>,
    spawns: impl FnOnce() -> Rc<dyn Spawns>,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("the daemon's runtime does not start")?;
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        let listener = listen(socket)?;
        let bound = std::fs::metadata(socket)?.ino();
        let (to, from_window) = unbounded_channel();
        let (internal, heard) = unbounded_channel();
        let (stop, stopped) = oneshot::channel();
        tokio::task::spawn_local(accept(listener, to, internal.clone()));
        tokio::task::spawn_local(stop_when_asked(socket.to_owned(), bound, stop));
        tracing::info!(socket = %socket.display(), "coxd runs");
        let daemon = Daemon::new(home, false, store(), spawns(), internal);
        daemon.serve(None, from_window, heard, stopped).await;
        // Unless another coxd listens there now, as when the data folder
        // was made again.
        if std::fs::metadata(socket)
            .map(|metadata| metadata.ino())
            .ok()
            == Some(bound)
        {
            let _ = std::fs::remove_file(socket);
        }
        tracing::info!("coxd stops");
        Ok(())
    })
}

/// Listens on `socket`, unless a coxd answers there already: a socket no
/// coxd answers at was left by one that ended.
fn listen(socket: &Path) -> anyhow::Result<UnixListener> {
    if socket.exists() {
        anyhow::ensure!(
            std::os::unix::net::UnixStream::connect(socket).is_err(),
            "another coxd answers on {}",
            socket.display()
        );
        let _ = std::fs::remove_file(socket);
    }
    if let Some(folder) = socket.parent() {
        std::fs::create_dir_all(folder)?;
    }
    UnixListener::bind(socket).with_context(|| format!("cannot listen on {}", socket.display()))
}

/// Stops the daemon once a signal asks it to, or once its socket, which
/// was the file `bound`, is gone, as when its data folder was taken away.
async fn stop_when_asked(socket: PathBuf, bound: u64, stop: oneshot::Sender<()>) {
    if let Err(error) = cox_browser::quit::on_signals() {
        tracing::error!("signals end coxd without stopping the agents: {error}");
    }
    loop {
        tokio::time::sleep(LOOK_EVERY).await;
        if cox_browser::quit::take_asked() {
            tracing::info!("coxd was asked to stop");
            break;
        }
        if std::fs::metadata(&socket)
            .map(|metadata| metadata.ino())
            .ok()
            != Some(bound)
        {
            tracing::info!("coxd's socket is gone");
            break;
        }
    }
    let _ = stop.send(());
}

/// Takes each window that comes to `listener`, passing on what it says to
/// the daemon, on `to`, once the daemon took it, as `internal` asks.
async fn accept(
    listener: UnixListener,
    to: UnboundedSender<ToDaemon>,
    internal: UnboundedSender<Internal>,
) {
    let program = Program::now();
    let mut last = 0;
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::error!("a window could not come: {error}");
                continue;
            }
        };
        last += 1;
        let replaced = program.replaced();
        tokio::task::spawn_local(connect(
            last,
            stream,
            replaced,
            to.clone(),
            internal.clone(),
        ));
    }
}

/// Serves the window of the connection `id` on `stream`, telling the
/// daemon it came, and, once the daemon took it, what it says, until it
/// goes.
async fn connect(
    id: u64,
    stream: UnixStream,
    replaced: bool,
    to: UnboundedSender<ToDaemon>,
    internal: UnboundedSender<Internal>,
) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let hello = match lines.next_line().await {
        Ok(Some(line)) => match serde_json::from_str::<Hello>(&line) {
            Ok(hello) => hello,
            Err(error) => {
                tracing::error!("a window said no hello: {error}");
                return;
            }
        },
        _ => return,
    };
    let (window, told) = unbounded_channel();
    let (answer, answered) = oneshot::channel();
    let attach = Internal::Attach {
        id,
        hello,
        window,
        replaced,
        answer,
    };
    if internal.send(attach).is_err() {
        return;
    }
    let Ok(answer) = answered.await else {
        return;
    };
    if write_line(&mut write, &answer).await.is_err() {
        let _ = internal.send(Internal::Detach { id });
        return;
    }
    match answer {
        FromDaemon::Welcome => {}
        FromDaemon::Outdated => {
            tracing::info!("coxd's program was replaced: it stops for the new one");
            let _ = write.shutdown().await;
            let _ = internal.send(Internal::Quit);
            return;
        }
        _ => return,
    }
    tracing::info!("a window came");
    let writes = tokio::task::spawn_local(tell(write, told));
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => match serde_json::from_str::<ToDaemon>(&line) {
                Ok(message) => {
                    if to.send(message).is_err() {
                        break;
                    }
                }
                Err(error) => tracing::error!("the window said what coxd cannot read: {error}"),
            },
            Ok(None) => break,
            Err(error) => {
                tracing::error!("the window could not be heard: {error}");
                break;
            }
        }
    }
    let _ = internal.send(Internal::Detach { id });
    writes.abort();
}

/// Writes to the window each message the daemon tells it, `told`, until
/// the daemon lets it go, or it cannot be written to.
async fn tell(
    mut write: tokio::net::unix::OwnedWriteHalf,
    mut told: UnboundedReceiver<FromDaemon>,
) {
    while let Some(message) = told.recv().await {
        if let Err(error) = write_line(&mut write, &message).await {
            tracing::warn!("the window could not be told: {error}");
            break;
        }
    }
}

/// Writes `message` as a line of JSON.
async fn write_line(
    write: &mut tokio::net::unix::OwnedWriteHalf,
    message: &impl serde::Serialize,
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    write.write_all(&line).await
}

/// The program coxd runs, as it was when coxd started, to tell whether
/// it was replaced since.
struct Program {
    path: Option<PathBuf>,
    modified: Option<SystemTime>,
}

impl Program {
    fn now() -> Self {
        let path = std::env::current_exe().ok();
        let modified = path.as_deref().and_then(modified);
        Self { path, modified }
    }

    /// Whether the program at the path coxd started from is another now.
    fn replaced(&self) -> bool {
        match (&self.path, self.modified) {
            (Some(path), Some(then)) => modified(path) != Some(then),
            _ => false,
        }
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}
