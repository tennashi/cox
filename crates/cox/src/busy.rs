//! When the window's thread was too busy to draw: the thread GPUI draws
//! the window on, and on macOS gives CEF its turns on, so that nothing on
//! the window moves while it is busy, the pages of the stage neither. Cox
//! says so in the log as it happens, for an agent that debugs Cox to
//! find what else the log said then, and keeps the last minute of it for
//! `cox --inspect frames` (see [`crate::inspect`]).

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use gpui::App;

/// How often the window's thread is looked at.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// How long the thread is busy before it is said: six frames of a display
/// of 60 Hz.
pub const BUSY_AFTER: Duration = Duration::from_millis(100);

/// How long what was busy is kept.
const KEPT_FOR: Duration = Duration::from_mins(1);

/// What kept the window's thread busy, when, and for how long, the oldest
/// first.
static SEEN: Mutex<VecDeque<Busy>> = Mutex::new(VecDeque::new());

/// A while the window's thread was busy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy {
    /// When it started.
    pub at: Instant,
    /// How long it lasted.
    pub lasted: Duration,
    /// What kept it busy, when Cox knows.
    pub with: Option<&'static str>,
}

/// Looks at the window's thread every [`LOOK_EVERY`] while the app runs:
/// a look that comes [`BUSY_AFTER`] late or more says the thread was busy
/// that long.
pub fn watch(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            let started = Instant::now();
            cx.background_executor().timer(LOOK_EVERY).await;
            let late = started.elapsed().saturating_sub(LOOK_EVERY);
            if late >= BUSY_AFTER {
                note(started + LOOK_EVERY, late, None);
            }
        }
    })
    .detach();
}

/// Times `work`, done on the window's thread, saying it kept the thread
/// busy when it took [`BUSY_AFTER`] or more.
pub fn timed<T>(with: &'static str, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let done = work();
    let lasted = started.elapsed();
    if lasted >= BUSY_AFTER {
        note(started, lasted, Some(with));
    }
    done
}

/// Says in the log that the window's thread was busy for `lasted` from
/// `at`, with `with`, and keeps it.
fn note(at: Instant, lasted: Duration, with: Option<&'static str>) {
    let busy_ms = lasted.as_millis();
    if let Some(with) = with {
        tracing::warn!(
            busy_ms,
            with,
            "the window's thread was busy for {busy_ms} ms with {with}"
        );
    } else {
        tracing::warn!(busy_ms, "the window's thread was busy for {busy_ms} ms");
    }
    let mut seen = SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    while seen
        .front()
        .is_some_and(|busy| now.duration_since(busy.at) > KEPT_FOR)
    {
        seen.pop_front();
    }
    seen.push_back(Busy { at, lasted, with });
}

/// The whiles the window's thread was busy that ended in the last `over`,
/// the oldest first.
#[must_use]
pub fn seen(over: Duration) -> Vec<Busy> {
    let now = Instant::now();
    SEEN.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|busy| now.duration_since(busy.at + busy.lasted) <= over)
        .copied()
        .collect()
}
