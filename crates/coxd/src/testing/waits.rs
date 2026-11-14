//! How long a test waits for what it waits for before it fails, saying
//! it never came: one deadline for every wait.

use std::time::{Duration, Instant};

/// How long a wait lasts before it fails. It only catches what hangs: a
/// debug build lays out a long conversation slowly, which can take
/// seconds on a slow machine, as CI's.
pub const GIVEN_UP: Duration = Duration::from_secs(30);

/// A wait that gives up after [`GIVEN_UP`]: what the receiver hears
/// then, unless the sender is dropped first. A thread keeps the time,
/// and ends as soon as the wait does, as a test waits many times, and
/// macOS lets a process have only a few thousand threads.
#[must_use]
pub fn given_up() -> (
    std::sync::mpsc::Sender<()>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (waiting, ended) = std::sync::mpsc::channel();
    let (timed_out, timeout) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        if ended.recv_timeout(GIVEN_UP) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
            let _ = timed_out.send(());
        }
    });
    (waiting, timeout)
}

/// Waits until `done` says so, asking it again every few milliseconds,
/// and fails saying `what` never came after [`GIVEN_UP`].
///
/// # Panics
///
/// When it never came, which fails the test.
pub fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + GIVEN_UP;
    while !done() {
        assert!(Instant::now() < deadline, "waited in vain for: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
