//! When the window's thread stuck: stayed busy [`STUCK_AFTER`] or more, as
//! when it waits on a lock nothing lets go of. [`crate::busy`] says the
//! thread was busy once it is free again, from the thread itself, so a
//! thread that never comes back says nothing. Here a thread of its own
//! watches the beat [`crate::busy`] gives each time it looks, and when the
//! beat stops, has macOS's `sample` write what each of Cox's threads was
//! doing, the window's with them, into a file beside the run's log,
//! `<run>-hang-<time>.txt`; once more if the thread is still stuck after
//! [`AGAIN_AFTER`]. It says nothing in the log while the thread is stuck,
//! as that may be what it is stuck on; once the thread is free, it says how
//! long it stuck and where the samples are. Cox keeps the last [`KEPT`] of
//! the files, and `cox --inspect frames` tells them.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// How long the window's thread is busy before it counts as stuck.
pub const STUCK_AFTER: Duration = Duration::from_secs(3);

/// How long the thread is stuck before it is sampled a second time.
const AGAIN_AFTER: Duration = Duration::from_secs(30);

/// How often the beat is looked at.
const LOOK_EVERY: Duration = Duration::from_millis(250);

/// How many files of samples Cox keeps, of all its runs.
const KEPT: usize = 10;

/// What the name of a file of samples has between the run and the time.
const MARK: &str = "-hang-";

/// The beat of the window's thread, which [`crate::busy`] gives.
pub static HEART: LazyLock<Heart> = LazyLock::new(Heart::new);

/// When a thread last beat.
pub struct Heart {
    born: Instant,
    /// When it last beat, in milliseconds from `born`.
    beat: AtomicU64,
    /// The longest it went without a beat since [`Heart::longest_quiet`]
    /// was last asked, in milliseconds.
    longest: AtomicU64,
}

impl Heart {
    fn new() -> Self {
        Self {
            born: Instant::now(),
            beat: AtomicU64::new(0),
            longest: AtomicU64::new(0),
        }
    }

    /// Says the thread is not stuck now.
    pub fn beat(&self) {
        let now = u64::try_from(self.born.elapsed().as_millis()).unwrap_or(u64::MAX);
        let before = self.beat.swap(now, Ordering::Relaxed);
        self.longest
            .fetch_max(now.saturating_sub(before), Ordering::Relaxed);
    }

    fn last(&self) -> Duration {
        Duration::from_millis(self.beat.load(Ordering::Relaxed))
    }

    /// The longest the thread went without a beat since this was last
    /// asked: the guard is late to see the beat come back while `sample`
    /// runs, so the time from the stuck beat to the one it sees is longer
    /// than the thread stuck.
    fn longest_quiet(&self) -> Duration {
        Duration::from_millis(self.longest.swap(0, Ordering::Relaxed))
    }
}

/// A while the thread stuck, told once it is free.
#[derive(Debug)]
pub struct Stuck {
    pub lasted: Duration,
    /// Each file of samples taken while it was stuck, or why it was not.
    pub samples: Vec<Result<PathBuf, String>>,
}

/// Starts watching the window's thread, keeping the samples of it stuck in
/// `directory`, the run's log's, beside `run`'s log.
pub fn watch(directory: &Path, run: &Path) {
    let Some(run) = run.file_stem().and_then(|stem| stem.to_str()) else {
        return;
    };
    let directory = directory.to_owned();
    let run = run.to_owned();
    HEART.beat();
    guard(
        &HEART,
        STUCK_AFTER,
        AGAIN_AFTER,
        move || {
            let _ = prune(&directory, KEPT - 1);
            sample(&directory, &run)
        },
        |stuck| tell(&stuck),
    );
}

/// Watches `heart` on a thread of its own: once it has not beat for
/// `after`, takes a `sample`, and another once it has not for `again`, and
/// once it beats again, tells `freed` how long it stuck. Nothing else is
/// done while it is stuck.
fn guard(
    heart: &'static Heart,
    after: Duration,
    again: Duration,
    mut sample: impl FnMut() -> Result<PathBuf, String> + Send + 'static,
    mut freed: impl FnMut(Stuck) + Send + 'static,
) {
    let look_every = LOOK_EVERY.min(after / 4);
    let spawned = thread::Builder::new()
        .name("cox-hang-watch".to_owned())
        .spawn(move || {
            let mut stuck: Option<(Duration, Vec<Result<PathBuf, String>>)> = None;
            loop {
                thread::sleep(look_every);
                let last = heart.last();
                let quiet = heart.born.elapsed().saturating_sub(last);
                match &mut stuck {
                    None if quiet >= after => {
                        heart.longest_quiet();
                        stuck = Some((last, vec![sample()]));
                    }
                    Some((since, samples)) if last != *since => {
                        freed(Stuck {
                            lasted: heart.longest_quiet(),
                            samples: std::mem::take(samples),
                        });
                        stuck = None;
                    }
                    Some((_, samples)) if samples.len() == 1 && quiet >= again => {
                        samples.push(sample());
                    }
                    None | Some(_) => {}
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!("the window's thread is not watched for being stuck: {error}");
    }
}

/// Has `sample` write what each of Cox's threads is doing, for two seconds,
/// into a new file in `directory` named for `run` and now.
#[cfg(target_os = "macos")]
fn sample(directory: &Path, run: &str) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt as _;
    use std::process::{Command, Stdio};

    let time = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ");
    let path = directory.join(format!("{run}{MARK}{time}.txt"));
    let status = Command::new("/usr/bin/sample")
        .arg(std::process::id().to_string())
        .arg("2")
        .arg("-file")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("sample did not run: {error}"))?;
    if !status.success() {
        return Err(format!("sample failed: {status}"));
    }
    // What the threads were doing can carry what the agents say.
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    Ok(path)
}

#[cfg(not(target_os = "macos"))]
fn sample(_directory: &Path, _run: &str) -> Result<PathBuf, String> {
    Err("Cox samples its threads on macOS only".to_owned())
}

/// Says in the log that the window's thread was stuck, and where the
/// samples of it are.
fn tell(stuck: &Stuck) {
    let stuck_ms = stuck.lasted.as_millis();
    let mut samples = String::new();
    for sample in &stuck.samples {
        let _ = match sample {
            Ok(path) => write!(samples, "; sampled in {}", path.display()),
            Err(error) => write!(samples, "; not sampled: {error}"),
        };
    }
    tracing::warn!(
        stuck_ms,
        "the window's thread was stuck for {stuck_ms} ms{samples}"
    );
}

/// The files of samples in `directory`, the oldest first. Their names start
/// with the time the run started, then the time of the sample, so they sort
/// by it.
fn kept(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut kept: Vec<PathBuf> = entries
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "txt")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(MARK))
        })
        .collect();
    kept.sort();
    kept
}

/// Removes the files of samples in `directory` but the last `keep`.
fn prune(directory: &Path, keep: usize) -> std::io::Result<()> {
    let kept = kept(directory);
    let past = kept.len().saturating_sub(keep);
    for path in &kept[..past] {
        fs::remove_file(path)?;
    }
    Ok(())
}

/// Tells the files of samples of the window's thread stuck, the newest
/// first, for `cox --inspect frames`.
#[must_use]
pub fn told() -> String {
    let stuck = STUCK_AFTER.as_secs();
    let kept = coxd::logs::directory().map_or_default(kept);
    if kept.is_empty() {
        return format!(
            "No samples of the window's thread stuck for {stuck} s or more are kept.\n"
        );
    }
    let mut told =
        format!("Samples of the window's thread stuck for {stuck} s or more, the newest first:\n");
    for path in kept.iter().rev() {
        let _ = writeln!(told, "  {}", path.display());
    }
    told
}

/// Blocks the window's thread for as many seconds as
/// `COX_BLOCK_WINDOW_THREAD` says, ten seconds after the window opens, to
/// try the watch out. Only in a build for debugging.
#[cfg(debug_assertions)]
pub fn block_when_asked(cx: &mut gpui::App) {
    let Some(seconds) = std::env::var("COX_BLOCK_WINDOW_THREAD")
        .ok()
        .and_then(|seconds| seconds.parse::<u64>().ok())
    else {
        return;
    };
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_secs(10))
            .await;
        tracing::warn!("blocking the window's thread for {seconds} s, as asked");
        thread::sleep(Duration::from_secs(seconds));
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use coxd::testing::TempDir;

    use super::*;

    /// A heart of a test's own, which beat once, with a guard that samples
    /// into `folder` and says each sample and each time it is freed.
    fn guarded(
        folder: &Path,
        again: Duration,
    ) -> (
        &'static Heart,
        mpsc::Receiver<PathBuf>,
        mpsc::Receiver<Stuck>,
    ) {
        let heart: &'static Heart = Box::leak(Box::new(Heart::new()));
        heart.beat();
        let (says_sample, samples) = mpsc::channel();
        let (says_freed, frees) = mpsc::channel();
        let folder = folder.to_owned();
        let mut taken = 0;
        guard(
            heart,
            Duration::from_millis(100),
            again,
            move || {
                taken += 1;
                let path = folder.join(format!("run{MARK}{taken}.txt"));
                fs::write(&path, "main-thread").map_err(|error| error.to_string())?;
                let _ = says_sample.send(path.clone());
                Ok(path)
            },
            move |stuck| {
                let _ = says_freed.send(stuck);
            },
        );
        (heart, samples, frees)
    }

    const WAIT: Duration = Duration::from_secs(30);

    #[test]
    fn a_stuck_thread_is_sampled_once_and_told_once_free() {
        let folder = TempDir::new();
        let (heart, samples, frees) = guarded(folder.path(), Duration::from_hours(1));
        let sample = samples
            .recv_timeout(WAIT)
            .expect("the stuck thread is sampled");
        assert!(
            frees.try_recv().is_err(),
            "nothing is told while it is stuck"
        );
        heart.beat();
        let stuck = frees.recv_timeout(WAIT).expect("told once free");
        assert_eq!(stuck.samples, [Ok(sample.clone())]);
        assert!(stuck.lasted >= Duration::from_millis(100), "{stuck:?}");
        assert_eq!(kept(folder.path()), [sample]);
        assert!(samples.try_recv().is_err(), "sampled once");
    }

    #[test]
    fn a_thread_stuck_long_is_sampled_again() {
        let folder = TempDir::new();
        let (heart, samples, frees) = guarded(folder.path(), Duration::from_millis(300));
        let first = samples.recv_timeout(WAIT).expect("sampled");
        let second = samples.recv_timeout(WAIT).expect("sampled again");
        heart.beat();
        let stuck = frees.recv_timeout(WAIT).expect("told once free");
        assert_eq!(stuck.samples, [Ok(first), Ok(second)]);
    }

    #[test]
    fn only_the_last_samples_are_kept() {
        let folder = TempDir::new();
        let log = folder.path().join("20261116T165058Z-1.jsonl");
        fs::write(&log, "").unwrap();
        let samples: Vec<PathBuf> = (10..22)
            .map(|second| {
                let path = folder.path().join(format!(
                    "20261116T165058Z-1{MARK}20261116T1700{second}Z.txt"
                ));
                fs::write(&path, "").unwrap();
                path
            })
            .collect();
        prune(folder.path(), KEPT).unwrap();
        assert_eq!(kept(folder.path()), samples[samples.len() - KEPT..]);
        assert!(log.exists(), "the logs are not samples");
    }
}
