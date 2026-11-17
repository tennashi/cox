//! How a window reaches coxd (see [`super::process`]): on its socket,
//! starting it first when none answers there, or again when the one that
//! answers stopped for it, waiting while the one that answers is to stop
//! but at work, as the human sees and may cut short (see [`Waiting`]),
//! and then carrying what the window tells the daemon, and what the
//! daemon tells it, a line each, on threads of its own.
//!
//! A coxd of an older build may not know to stop at once when the
//! window asks it to: the window then takes its socket away, which every
//! coxd stops for, as it does for a signal, and waits for it to have
//! stopped before it starts its own. It needs to know nothing of what
//! the window says, so this holds for any build to come.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::Link;
use super::wire::{Asks, AtWork, FromDaemon, Hello, ToDaemon, Version};

/// How long a window waits for a coxd it started to answer.
const STARTS_WITHIN: Duration = Duration::from_secs(10);

/// How often it looks whether it does.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// How often a window comes again to a coxd that is to stop for it, but
/// was at work.
const AGAIN_EVERY: Duration = Duration::from_secs(2);

/// How long a coxd whose socket the window took away has to stop, its
/// agents with it.
const STOPS_WITHIN: Duration = Duration::from_secs(30);

/// The daemon a window reached: what to tell it, and what it tells, and
/// its version. Once all of `link` is dropped, the daemon lets the window
/// go.
pub struct Connected {
    pub link: Link,
    pub from_daemon: UnboundedReceiver<FromDaemon>,
    pub coxd: Version,
}

/// What the human chose, while the window waited for coxd.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// To have coxd stop now, cutting the chats at work.
    RestartNow,
    /// Not to wait: the window does not open.
    Quit,
}

/// What the human sees while the window waits for coxd to stop, and
/// what they choose there.
pub trait Waiting {
    /// The window waits for coxd, at work on `chats`, or on chats it does
    /// not name, as one of an older build.
    fn waits(&mut self, chats: Option<&[AtWork]>);

    /// What the human chose within `within`, if they chose.
    fn chosen(&mut self, within: Duration) -> Option<Choice>;

    /// coxd is stopping, as the human chose.
    fn stopping(&mut self);
}

/// The window did not reach coxd, as the human chose.
#[derive(Debug)]
pub struct Quit;

impl std::fmt::Display for Quit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the human chose not to wait for coxd")
    }
}

impl std::error::Error for Quit {}

/// Reaches coxd on `socket`, saying `hello`, starting it with `start`
/// when none answers there, or when the one there stopped for the
/// window, which then joins the one it started, whatever `hello` asked.
/// While the one there is to stop but a chat of it is at work, the window
/// comes again until none is, the human seeing it on `waiting`, and has
/// it stop at once when they choose so. Once the daemon goes, the window
/// hears no more, and `gone` is called.
///
/// # Errors
///
/// When coxd does not start, or another window has it, or a coxd asked to
/// stop does not, or the human chose not to wait ([`Quit`]).
pub fn connect(
    socket: &Path,
    hello: &Hello,
    start: &dyn Fn() -> std::io::Result<()>,
    waiting: &mut dyn Waiting,
    gone: impl FnOnce() + Send + 'static,
) -> anyhow::Result<Connected> {
    reach(socket, hello, start, waiting, gone, &alive)
}

/// [`connect`], telling whether a process runs still with `alive`.
fn reach(
    socket: &Path,
    hello: &Hello,
    start: &dyn Fn() -> std::io::Result<()>,
    waiting: &mut dyn Waiting,
    gone: impl FnOnce() + Send + 'static,
    alive: &dyn Fn(libc::pid_t) -> bool,
) -> anyhow::Result<Connected> {
    let mut hello = hello.clone();
    let mut restarted = false;
    let mut started = None;
    loop {
        let stream = match started.take() {
            Some(stream) => stream,
            None => match UnixStream::connect(socket) {
                Ok(stream) => stream,
                Err(_) => start_once(socket, start)?,
            },
        };
        // Before it answers: once it has, it may have closed its end.
        let coxd = peer(&stream);
        match greet(&stream, &hello)? {
            (FromDaemon::Joined(coxd), reader) => return Ok(carry(stream, reader, coxd, gone)),
            (FromDaemon::Taken, _) => bail!("another Cox window has the chats already"),
            (FromDaemon::Outdated, mut reader) => {
                tracing::info!("coxd stops for the window, which starts its own");
                // It closes once it has stopped listening.
                let mut rest = String::new();
                while reader.read_line(&mut rest).is_ok_and(|read| read > 0) {}
                wait_gone(socket);
                if restarted {
                    bail!("coxd stays outdated");
                }
                restarted = true;
                hello.asks = Asks::Join;
            }
            (answer @ (FromDaemon::Busy | FromDaemon::Welcome | FromDaemon::AtWork(_)), _) => {
                if hello.asks == Asks::RestartNow {
                    // Asked to stop now, it would have: it is of a build
                    // that does not know to.
                    tracing::warn!("coxd does not stop now when asked: the window takes it away");
                    if restarted {
                        bail!("coxd stays at work");
                    }
                    waiting.stopping();
                    drop(stream);
                    started = Some(take_away(socket, coxd, start, alive)?);
                    restarted = true;
                    hello.asks = Asks::Join;
                    continue;
                }
                tracing::warn!("coxd is to stop for the window, but at work: the window waits");
                let chats = match answer {
                    FromDaemon::AtWork(chats) => Some(chats),
                    _ => None,
                };
                waiting.waits(chats.as_deref());
                drop(stream);
                match waiting.chosen(AGAIN_EVERY) {
                    Some(Choice::RestartNow) => {
                        tracing::info!("the human has coxd stop now");
                        hello.asks = Asks::RestartNow;
                    }
                    Some(Choice::Quit) => return Err(Quit.into()),
                    None => {}
                }
            }
            (other, _) => bail!("coxd answered {other:?}"),
        }
    }
}

/// Starts coxd with `start`, unless another window did while this one
/// waited its turn, and answers the stream of the one that answers.
fn start_once(
    socket: &Path,
    start: &dyn Fn() -> std::io::Result<()>,
) -> anyhow::Result<UnixStream> {
    let _turn = Turn::take(socket)?;
    if let Ok(stream) = UnixStream::connect(socket) {
        return Ok(stream);
    }
    start().context("coxd does not start")?;
    wait_for(socket)
}

/// Has the coxd of the process `coxd`, which answers on `socket`, stop,
/// by taking its socket away, waits until it has, its agents with it,
/// and starts one in its place with `start`, whose stream it answers. No
/// other window starts one meanwhile, as one waiting too would once the
/// socket is gone.
fn take_away(
    socket: &Path,
    coxd: Option<libc::pid_t>,
    start: &dyn Fn() -> std::io::Result<()>,
    alive: &dyn Fn(libc::pid_t) -> bool,
) -> anyhow::Result<UnixStream> {
    let Some(coxd) = coxd else {
        bail!("which process coxd is cannot be told, to wait for it to stop");
    };
    let _turn = Turn::take(socket)?;
    tracing::info!(
        pid = coxd,
        "the window takes coxd's socket away, for it to stop"
    );
    std::fs::remove_file(socket)
        .with_context(|| format!("cannot take coxd's socket away, {}", socket.display()))?;
    let until = Instant::now() + STOPS_WITHIN;
    while alive(coxd) {
        if Instant::now() > until {
            bail!(
                "coxd (process {coxd}) did not stop within {} s",
                STOPS_WITHIN.as_secs()
            );
        }
        std::thread::sleep(LOOK_EVERY);
    }
    tracing::info!("coxd stopped: the window starts its own");
    start().context("coxd does not start")?;
    wait_for(socket)
}

/// Whether the process `pid` runs still. Signal 0 sends nothing.
fn alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    let answered = unsafe { libc::kill(pid, 0) };
    answered == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The process at the other end of `stream`, as the system tells it.
fn peer(stream: &UnixStream) -> Option<libc::pid_t> {
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "macos")]
    {
        let mut pid: libc::pid_t = 0;
        let mut length = libc::socklen_t::try_from(std::mem::size_of::<libc::pid_t>()).ok()?;
        // SAFETY: `pid` and `length` outlive the call, and `length` is
        // the size of `pid`.
        let got = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&raw mut pid).cast(),
                &raw mut length,
            )
        };
        (got == 0 && pid > 0).then_some(pid)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut credentials = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut length = libc::socklen_t::try_from(std::mem::size_of::<libc::ucred>()).ok()?;
        // SAFETY: `credentials` and `length` outlive the call, and
        // `length` is the size of `credentials`.
        let got = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&raw mut credentials).cast(),
                &raw mut length,
            )
        };
        (got == 0 && credentials.pid > 0).then_some(credentials.pid)
    }
}

/// A window's turn to start coxd, which windows of this build take one at
/// a time, on a lock beside the socket, until it is dropped.
struct Turn {
    /// The lock, held while it is open.
    _held: std::fs::File,
}

impl Turn {
    /// Takes the turn to start coxd on `socket`, waiting for another
    /// window's to end, but not for longer than one takes.
    fn take(socket: &Path) -> anyhow::Result<Self> {
        let path = lock_of(socket);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        let until = Instant::now() + STOPS_WITHIN + STARTS_WITHIN;
        loop {
            // SAFETY: the descriptor is the file's, open while it is.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Self { _held: file });
            }
            if Instant::now() > until {
                bail!("another window keeps its turn to start coxd");
            }
            std::thread::sleep(LOOK_EVERY);
        }
    }
}

/// The lock of the turns to start coxd on `socket`.
fn lock_of(socket: &Path) -> PathBuf {
    socket.with_extension("lock")
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

/// Carries what the window tells the daemon of `coxd`'s version on
/// `stream`, and what the daemon tells it, read with `reader`, each on a
/// thread of its own, calling `gone` once the daemon has gone.
fn carry(
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    coxd: Version,
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
        coxd,
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::daemon::wire::VERSION;
    use crate::testing::TempDir;

    /// What the human saw while the window waited, choosing in turn each
    /// of `choices`, then nothing.
    #[derive(Default)]
    struct Seen {
        waits: Vec<Option<Vec<AtWork>>>,
        stopping: usize,
        choices: Vec<Choice>,
    }

    impl Waiting for Seen {
        fn waits(&mut self, chats: Option<&[AtWork]>) {
            self.waits.push(chats.map(<[AtWork]>::to_vec));
        }

        fn chosen(&mut self, _within: Duration) -> Option<Choice> {
            (!self.choices.is_empty()).then(|| self.choices.remove(0))
        }

        fn stopping(&mut self) {
            self.stopping += 1;
        }
    }

    /// A window that is not to wait.
    struct NoWaiting;

    impl Waiting for NoWaiting {
        fn waits(&mut self, _chats: Option<&[AtWork]>) {
            panic!("the window does not wait");
        }

        fn chosen(&mut self, _within: Duration) -> Option<Choice> {
            None
        }

        fn stopping(&mut self) {
            panic!("the window does not stop coxd");
        }
    }

    /// A coxd on `socket` that answers each window that comes, in turn,
    /// the line of `answers`, counting in `heard` the hellos it heard.
    fn coxd(socket: &Path, answers: Vec<String>, heard: &Arc<AtomicUsize>) {
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
            vec![
                r#""Welcome""#.to_owned(),
                r#""Busy""#.to_owned(),
                serde_json::to_string(&FromDaemon::Joined(VERSION)).unwrap(),
            ],
            &heard,
        );
        let mut seen = Seen::default();

        let connected = connect(
            &socket,
            &Hello::default(),
            &|| Err(std::io::Error::other("coxd runs already")),
            &mut seen,
            || {},
        );

        assert!(connected.is_ok(), "{:?}", connected.err());
        assert_eq!(
            heard.load(Ordering::SeqCst),
            3,
            "the window comes again, past the coxd that welcomed it as any window, and the busy one"
        );
        assert_eq!(
            seen.waits,
            [None, None],
            "the human sees the window wait, on chats neither coxd names"
        );
    }

    #[test]
    fn a_window_joins_a_coxd_of_another_build_that_speaks_as_it_does() {
        let folder = TempDir::new();
        let socket = folder.join("speaks.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let heard = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let hello: Hello = serde_json::from_str(&line).unwrap();
            let coxd = Version {
                daemon: VERSION.daemon + 1,
                ..VERSION
            };
            let answer = hello.answer(coxd, true);
            (&stream)
                .write_all(format!("{}\n", serde_json::to_string(&answer).unwrap()).as_bytes())
                .unwrap();
            (answer, stream)
        });

        let connected = connect(
            &socket,
            &Hello::new(None, Vec::new(), Asks::Join),
            &|| Err(std::io::Error::other("coxd runs already")),
            &mut NoWaiting,
            || {},
        );

        assert!(connected.is_ok(), "{:?}", connected.err());
        let (answer, _stream) = heard.join().unwrap();
        let coxd = Version {
            daemon: VERSION.daemon + 1,
            ..VERSION
        };
        assert_eq!(answer, FromDaemon::Joined(coxd));
        assert_eq!(
            connected.unwrap().coxd,
            coxd,
            "the window knows coxd's version"
        );
    }

    /// Serves one window on `socket`, as coxd of this build with a
    /// chat at work, and stops if it answers anything but Joined.
    fn serve(socket: &Path, asked: &Arc<std::sync::Mutex<Vec<Asks>>>) {
        let listener = UnixListener::bind(socket).unwrap();
        let asked = Arc::clone(asked);
        let socket = socket.to_owned();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let hello: Hello = serde_json::from_str(&line).unwrap();
            asked.lock().unwrap().push(hello.asks);
            let answer = hello.answer(VERSION, true);
            (&stream)
                .write_all(format!("{}\n", serde_json::to_string(&answer).unwrap()).as_bytes())
                .unwrap();
            if matches!(answer, FromDaemon::Joined(_)) {
                std::thread::park();
            }
            drop(listener);
            std::fs::remove_file(&socket).unwrap();
        });
    }

    #[test]
    fn a_window_that_asked_coxd_to_stop_joins_the_coxd_it_starts() {
        let folder = TempDir::new();
        let socket = folder.join("restart.sock");
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        serve(&socket, &asked);

        let connected = connect(
            &socket,
            &Hello::new(None, Vec::new(), Asks::RestartNow),
            &|| {
                serve(&socket, &asked);
                Ok(())
            },
            &mut NoWaiting,
            || {},
        );

        assert!(connected.is_ok(), "{:?}", connected.err());
        assert_eq!(
            *asked.lock().unwrap(),
            [Asks::RestartNow, Asks::Join],
            "the coxd the window started is not asked to stop too"
        );
    }

    /// A coxd on `socket` that answers each hello it hears, kept in
    /// `heard`, with what `answers` says, and stops listening once it
    /// answered `Outdated`, as coxd does.
    fn coxd_that(
        socket: &Path,
        heard: &Arc<std::sync::Mutex<Vec<Hello>>>,
        answers: impl Fn(&Hello) -> FromDaemon + Send + 'static,
    ) {
        let listener = UnixListener::bind(socket).unwrap();
        let heard = Arc::clone(heard);
        let socket = socket.to_owned();
        std::thread::spawn(move || {
            let mut windows = Vec::new();
            loop {
                let (stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let hello: Hello = serde_json::from_str(&line).unwrap();
                let answer = answers(&hello);
                heard.lock().unwrap().push(hello);
                (&stream)
                    .write_all(format!("{}\n", serde_json::to_string(&answer).unwrap()).as_bytes())
                    .unwrap();
                if answer == FromDaemon::Outdated {
                    drop(listener);
                    std::fs::remove_file(&socket).unwrap();
                    return;
                }
                // It holds only a window it took, as coxd does.
                if matches!(answer, FromDaemon::Joined(_)) {
                    windows.push(stream);
                }
            }
        });
    }

    fn asked(heard: &Arc<std::sync::Mutex<Vec<Hello>>>) -> Vec<Asks> {
        heard
            .lock()
            .unwrap()
            .iter()
            .map(|hello| hello.asks)
            .collect()
    }

    #[test]
    fn a_window_shows_the_chats_at_work_and_has_coxd_stop_now_once_the_human_chooses() {
        let folder = TempDir::new();
        let socket = folder.join("chosen.sock");
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let at_work = vec![AtWork {
            project: Some("cox".to_owned()),
            chat: "Fix the build".to_owned(),
        }];
        let unlike = Version {
            protocol: VERSION.protocol + 1,
            ..VERSION
        };
        let told = at_work.clone();
        coxd_that(&socket, &heard, move |hello| match hello.asks {
            Asks::RestartNow => FromDaemon::Outdated,
            _ if hello.reads_at_work => FromDaemon::AtWork(told.clone()),
            _ => FromDaemon::Busy,
        });
        let started = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut seen = Seen {
            choices: vec![Choice::RestartNow],
            ..Seen::default()
        };

        let connected = connect(
            &socket,
            &Hello::new(None, Vec::new(), Asks::Join),
            &|| {
                coxd_that(&socket, &started, move |_| FromDaemon::Joined(unlike));
                Ok(())
            },
            &mut seen,
            || {},
        );

        assert!(connected.is_ok(), "{:?}", connected.err());
        assert_eq!(seen.waits, [Some(at_work)], "the human sees which chats");
        assert_eq!(asked(&heard), [Asks::Join, Asks::RestartNow]);
        assert_eq!(asked(&started), [Asks::Join]);
        assert_eq!(seen.stopping, 0, "coxd stopped by itself, as asked");
    }

    #[test]
    fn a_window_takes_away_an_older_coxd_that_does_not_stop_now_when_asked() {
        let folder = TempDir::new();
        let socket = folder.join("older.sock");
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        // As a coxd of a build before `Asks::RestartNow`, whatever it is
        // asked.
        coxd_that(&socket, &heard, |_| FromDaemon::Busy);
        let started = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut seen = Seen {
            choices: vec![Choice::RestartNow],
            ..Seen::default()
        };
        let waited_for = std::sync::Mutex::new(Vec::new());

        let connected = reach(
            &socket,
            &Hello::new(None, Vec::new(), Asks::Join),
            &|| {
                coxd_that(&socket, &started, |_| FromDaemon::Joined(VERSION));
                Ok(())
            },
            &mut seen,
            || {},
            &|pid| {
                waited_for.lock().unwrap().push(pid);
                // It stops once its socket is gone, as every coxd does.
                socket.exists()
            },
        );

        assert!(connected.is_ok(), "{:?}", connected.err());
        assert_eq!(asked(&heard), [Asks::Join, Asks::RestartNow]);
        assert_eq!(
            asked(&started),
            [Asks::Join],
            "the coxd the window started is not asked to stop too"
        );
        assert_eq!(seen.stopping, 1, "the human sees coxd stopping");
        assert_eq!(
            waited_for.lock().unwrap().first().copied(),
            Some(libc::pid_t::try_from(std::process::id()).unwrap()),
            "the window waits for the process at the other end of the socket"
        );
    }

    #[test]
    fn a_window_does_not_wait_when_the_human_quits() {
        let folder = TempDir::new();
        let socket = folder.join("quit.sock");
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        coxd_that(&socket, &heard, |_| FromDaemon::Busy);
        let mut seen = Seen {
            choices: vec![Choice::Quit],
            ..Seen::default()
        };

        let connected = connect(
            &socket,
            &Hello::default(),
            &|| panic!("coxd runs"),
            &mut seen,
            || {},
        );

        assert!(connected.err().unwrap().downcast_ref::<Quit>().is_some());
        assert!(socket.exists(), "coxd goes on");
    }

    #[test]
    fn a_window_joins_the_coxd_another_started_while_it_waited_its_turn() {
        let folder = TempDir::new();
        let socket = folder.join("turns.sock");
        let turn = Turn::take(&socket).unwrap();
        let waiter = {
            let socket = socket.clone();
            std::thread::spawn(move || {
                start_once(&socket, &|| panic!("the window starts no coxd of its own"))
                    .map(|_| ())
                    .map_err(|error| format!("{error:#}"))
            })
        };
        // Long enough for the other window to wait for its turn, which
        // nothing tells.
        std::thread::sleep(Duration::from_millis(200));
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        coxd_that(&socket, &heard, |_| FromDaemon::Joined(VERSION));
        drop(turn);

        assert_eq!(waiter.join().unwrap(), Ok(()));
    }
}
