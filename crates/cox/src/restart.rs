//! Restarting Cox, as the human asks: the window alone, which starts
//! again and joins the coxd that runs, whose chats go on; or Cox and
//! coxd, which stops once none of its chats is at work, or at once,
//! cutting them, for the window to start one of its own build. And
//! telling the human, once a build is installed over the one the window
//! runs, which of those runs it: the window alone while the build speaks
//! as coxd does (see [`Version::restart`]), and whether coxd's own code
//! changed too, which only a restart with coxd runs.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use coxd::daemon::wire::{Asks, Restart, VERSION, Version};
use gpui::{App, Global};

/// What the command line says to print the version of the build, as
/// JSON, for a window of another build to read.
pub const BUILD_VERSION: &str = "--build-version";

/// What the command line says to wait, before reaching coxd, until the
/// process it names, the window this one restarts, has quit.
pub const AFTER: &str = "--after";

/// What the command line says to ask coxd to stop for this window once
/// no chat is at work, or at once.
pub const RESTART_COXD: &str = "--restart-coxd";
pub const RESTART_COXD_NOW: &str = "--restart-coxd-now";

/// How long a window waits for the one it restarts to quit.
const QUITS_WITHIN: Duration = Duration::from_secs(30);

/// How often the window looks whether a build was installed over its own.
const LOOK_EVERY: Duration = Duration::from_secs(3);

/// How the human restarts Cox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// The window alone: the chats go on in coxd.
    Window,
    /// Cox and coxd, once no chat is at work.
    WithCoxd,
    /// Cox and coxd now, cutting the chats at work.
    WithCoxdNow,
}

impl How {
    /// What the window that starts asks of coxd.
    fn asks(self) -> Asks {
        match self {
            Self::Window => Asks::Join,
            Self::WithCoxd => Asks::Restart,
            Self::WithCoxdNow => Asks::RestartNow,
        }
    }
}

/// What waits for a restart, as the window tells the human.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    /// A build was installed since the window started, which a restart
    /// of the window alone runs; `coxd` when coxd's code changed too,
    /// which waits for a restart with coxd.
    Window { coxd: bool },
    /// A build was installed whose window coxd would not read: a restart
    /// runs it with coxd.
    WithCoxd,
    /// No build was installed since, but coxd runs code other than the
    /// window's, which a restart with coxd runs.
    Coxd,
}

impl Pending {
    /// What waits, for a window of `window`'s version with coxd of
    /// `coxd`'s, and the build of `installed` installed since it started,
    /// if one was.
    #[must_use]
    pub fn of(window: Version, coxd: Version, installed: Option<Version>) -> Option<Self> {
        match installed.map(|installed| (installed, installed.restart(coxd))) {
            Some((_, Restart::WithCoxd)) => Some(Self::WithCoxd),
            Some((installed, Restart::WindowOnly)) => Some(Self::Window {
                coxd: installed.daemon != coxd.daemon,
            }),
            None => (window.daemon != coxd.daemon).then_some(Self::Coxd),
        }
    }

    /// What the human is told of it.
    #[must_use]
    pub fn told(self) -> &'static str {
        match self {
            Self::Window { coxd: false } => {
                "A new build is installed: restart the window to run it, the chats go on"
            }
            Self::Window { coxd: true } => {
                "A new build is installed: restart the window to run it, the chats go on. \
                 coxd update pending: its code changed too, which a restart with coxd runs"
            }
            Self::WithCoxd => {
                "A new build is installed that the running coxd does not read: restart Cox \
                 and coxd to run it"
            }
            Self::Coxd => {
                "coxd update pending: coxd runs code older than the window's, which a \
                 restart with coxd runs"
            }
        }
    }

    /// Whether a restart of the window alone runs what waits.
    #[must_use]
    pub fn window_runs_it(self) -> bool {
        matches!(self, Self::Window { .. })
    }
}

/// The file of the window's program, as it was: a build installed over
/// it changes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Program {
    size: u64,
    modified: SystemTime,
}

impl Program {
    fn now() -> Option<Self> {
        let metadata = std::fs::metadata(std::env::current_exe().ok()?).ok()?;
        Some(Self {
            size: metadata.len(),
            modified: metadata.modified().ok()?,
        })
    }
}

/// The builds the window knows of: coxd's version, and the version of
/// the build installed over the window's since it started, if one was.
struct Builds {
    coxd: Version,
    installed: Option<Version>,
}

impl Global for Builds {}

/// Keeps that coxd is of `coxd`'s version, and looks from now on whether
/// a build is installed over the window's.
pub fn watch(coxd: Version, cx: &mut App) {
    let started = Program::now();
    cx.set_global(Builds {
        coxd,
        installed: None,
    });
    cx.spawn(async move |cx| {
        let mut seen = started;
        loop {
            cx.background_executor().timer(LOOK_EVERY).await;
            let now = Program::now();
            if now == seen {
                continue;
            }
            seen = now;
            let installed = if now == started {
                None
            } else {
                cx.background_executor()
                    .spawn(async { installed_version() })
                    .await
            };
            let told = cx.update(|cx| {
                cx.global_mut::<Builds>().installed = installed;
                cx.refresh_windows();
            });
            if told.is_err() {
                break;
            }
        }
    })
    .detach();
}

/// What waits for a restart now, if anything does.
#[must_use]
pub fn pending(cx: &App) -> Option<Pending> {
    let builds = cx.try_global::<Builds>()?;
    Pending::of(VERSION, builds.coxd, builds.installed)
}

/// The version of the build installed where the window's program is, as
/// it prints it, unless it does not.
fn installed_version() -> Option<Version> {
    let output = Command::new(std::env::current_exe().ok()?)
        .arg(BUILD_VERSION)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let version = serde_json::from_slice(&output.stdout);
    if let Err(error) = &version {
        tracing::warn!("the build installed told no version: {error}");
    }
    version.ok()
}

/// Prints the version of this build, as [`installed_version`] reads it.
pub fn print_version() {
    match serde_json::to_string(&VERSION) {
        Ok(line) => println!("{line}"),
        Err(error) => eprintln!("cox: {error}"),
    }
}

/// Starts the window again as `how` says, from the program installed
/// where this one's is, with the command line this one had, once this
/// one has quit, which it does.
///
/// # Errors
///
/// When the program cannot be started: the window goes on then.
pub fn restart(how: How, cx: &mut App) -> anyhow::Result<()> {
    let program: PathBuf = std::env::current_exe()?;
    let args = successor_args(std::env::args().skip(1), how, std::process::id());
    tracing::info!(
        "restarts as {how:?}: {} {}",
        program.display(),
        args.join(" ")
    );
    {
        use std::os::unix::process::CommandExt as _;
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
    }
    cx.quit();
    Ok(())
}

/// The command line of the window that restarts this one, of process
/// `pid`, which had `args`, as `how` says.
fn successor_args(args: impl IntoIterator<Item = String>, how: How, pid: u32) -> Vec<String> {
    let mut successor = vec![AFTER.to_owned(), pid.to_string()];
    match how.asks() {
        Asks::Join => {}
        Asks::Restart => successor.push(RESTART_COXD.to_owned()),
        Asks::RestartNow => successor.push(RESTART_COXD_NOW.to_owned()),
    }
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            AFTER => {
                args.next();
            }
            RESTART_COXD | RESTART_COXD_NOW => {}
            _ => successor.push(arg),
        }
    }
    successor
}

/// Waits until the process `pid`, the window this one restarts, has
/// quit, and so let coxd go, or for [`QUITS_WITHIN`].
pub fn wait_for(pid: u32) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    let until = Instant::now() + QUITS_WITHIN;
    // SAFETY: signal 0 only asks whether the process is there.
    while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COXD: Version = Version {
        protocol: 7,
        daemon: 1,
    };

    #[test]
    fn a_build_installed_that_speaks_as_coxd_runs_with_the_window_alone() {
        let newer_window = Version { ..COXD };
        let newer_coxd = Version { daemon: 2, ..COXD };
        let unlike = Version {
            protocol: 8,
            daemon: 2,
        };
        assert_eq!(Pending::of(COXD, COXD, None), None);
        assert_eq!(
            Pending::of(COXD, COXD, Some(newer_window)),
            Some(Pending::Window { coxd: false })
        );
        assert_eq!(
            Pending::of(COXD, COXD, Some(newer_coxd)),
            Some(Pending::Window { coxd: true })
        );
        assert_eq!(
            Pending::of(COXD, COXD, Some(unlike)),
            Some(Pending::WithCoxd)
        );
        assert_eq!(
            Pending::of(newer_coxd, COXD, None),
            Some(Pending::Coxd),
            "a window joined a coxd of older code"
        );
        assert!(Pending::Window { coxd: true }.window_runs_it());
        assert!(!Pending::WithCoxd.window_runs_it());
    }

    #[test]
    fn the_window_that_restarts_this_one_waits_for_it_and_asks_coxd_as_the_human_did() {
        let args = || {
            [
                "--after",
                "41",
                "--restart-coxd",
                "--data",
                "/tmp/d",
                "--behind",
                "node",
            ]
            .map(str::to_owned)
        };
        assert_eq!(
            successor_args(args(), How::Window, 42),
            ["--after", "42", "--data", "/tmp/d", "--behind", "node"]
        );
        assert_eq!(
            successor_args(args(), How::WithCoxd, 42),
            [
                "--after",
                "42",
                "--restart-coxd",
                "--data",
                "/tmp/d",
                "--behind",
                "node"
            ]
        );
        assert_eq!(
            successor_args(args(), How::WithCoxdNow, 42),
            [
                "--after",
                "42",
                "--restart-coxd-now",
                "--data",
                "/tmp/d",
                "--behind",
                "node"
            ]
        );
    }
}
