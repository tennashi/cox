//! The windows of the Mac's other apps, for Cox to show one on its stage:
//! what windows there are, and a picture of one as it changes, taken
//! through `ScreenCaptureKit`, so that the human sees the app the agent
//! tries out beside the chat. To use the app, the human brings the window
//! itself to the front: an app behind another takes the mouse and keys
//! posted to it only in part, and only through what macOS keeps private.
//!
//! macOS asks the human to allow Cox to record the screen, for the
//! pictures and the windows' titles. On other systems there are no windows
//! to show, and a window asked for ends at once, saying so.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as system;

#[cfg(not(target_os = "macos"))]
mod other;
#[cfg(not(target_os = "macos"))]
use other as system;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// A window of an app, as it is now.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// The window's number, as the system knows it while it is open.
    pub id: u32,
    /// The process of its app.
    pub pid: i32,
    /// Its app's name, as `Safari`.
    pub app: String,
    /// Its title, if it has one, and Cox may read it.
    pub title: String,
    /// Where it is, in points from the top left of the main display.
    pub frame: Rect,
}

/// A rectangle, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The windows of the other apps that are on screen, by app and title.
///
/// # Errors
///
/// When the system does not say, as before the human allows Cox to record
/// the screen, or off macOS.
pub fn windows() -> anyhow::Result<Vec<Window>> {
    system::windows()
}

/// Whether the human allowed Cox to record the screen, which the pictures
/// of a window need.
#[must_use]
pub fn may_record() -> bool {
    system::may_record()
}

/// Asks the human to allow Cox to record the screen, as System Settings
/// does: macOS shows its own dialog the first time, and later only lists
/// Cox there for the human to turn on.
pub fn ask_to_record() {
    system::ask_to_record();
}

/// Brings the app of the process `pid` to the front, with its windows, for
/// the human to use a window itself.
pub fn bring_to_front(pid: i32) {
    system::bring_to_front(pid);
}

/// A picture of a window, four bytes a pixel, blue, green, red and alpha,
/// row after row from the top.
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// What the mirror of a window tells of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The pictures come.
    Showing,
    /// A new picture waits, for [`Mirror::take_frame`].
    Frame,
    /// The window cannot be shown, or went: why.
    Ended(String),
}

/// What the mirror and its worker share.
#[derive(Default)]
struct Shared {
    /// The newest picture, until it is taken.
    frame: Mutex<Option<Frame>>,
    /// Whether the mirror was told of the picture waiting.
    told: AtomicBool,
}

impl Shared {
    #[cfg_attr(
        all(not(target_os = "macos"), not(test)),
        expect(dead_code, reason = "only macOS shows windows")
    )]
    /// Keeps `frame` as the newest, telling of it unless a picture already
    /// waits.
    fn put(&self, frame: Frame, events: &UnboundedSender<Event>) {
        *self.frame.lock().unwrap_or_else(PoisonError::into_inner) = Some(frame);
        if !self.told.swap(true, Ordering::AcqRel) {
            let _ = events.send(Event::Frame);
        }
    }
}

/// How big the stage draws a window, in pixels: the pictures come as big
/// as fits.
type Size = (u32, u32);

/// A window shown as it changes, until it drops.
pub struct Mirror {
    to: std::sync::mpsc::Sender<Size>,
    shared: Arc<Shared>,
}

impl Mirror {
    /// Starts showing the window `id`.
    #[must_use]
    pub fn open(id: u32) -> (Self, UnboundedReceiver<Event>) {
        let (events, heard) = unbounded_channel();
        let (to, sizes) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name(format!("cox-mac-window-{id}"))
            .spawn(move || system::mirror(id, &worker, &sizes, &events));
        if let Err(error) = spawned {
            tracing::error!("cannot start showing window {id}: {error}");
        }
        (Self { to, shared }, heard)
    }

    /// Asks for pictures `width` by `height` pixels, or as near as the
    /// window's shape lets them fit.
    pub fn resize(&self, width: u32, height: u32) {
        let _ = self.to.send((width, height));
    }

    /// The newest picture, if one came since the last taken.
    #[must_use]
    pub fn take_frame(&self) -> Option<Frame> {
        self.shared.told.store(false, Ordering::Release);
        self.shared
            .frame
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "only macOS shows windows")
)]
/// The biggest size of `frame`'s shape that fits in `width` by `height`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a size on the stage fits in u32"
)]
fn fitting(frame: Rect, width: u32, height: u32) -> (u32, u32) {
    if frame.width <= 0. || frame.height <= 0. {
        return (width.max(1), height.max(1));
    }
    let scale = (f64::from(width) / frame.width).min(f64::from(height) / frame.height);
    (
        ((frame.width * scale).round() as u32).max(1),
        ((frame.height * scale).round() as u32).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Rect = Rect {
        x: 100.,
        y: 50.,
        width: 800.,
        height: 600.,
    };

    #[test]
    fn fits_the_window_in_the_stage() {
        assert_eq!(fitting(FRAME, 1600, 1600), (1600, 1200));
        assert_eq!(fitting(FRAME, 400, 1600), (400, 300));
    }

    #[test]
    fn tells_of_a_waiting_picture_once() {
        let (events, mut heard) = unbounded_channel();
        let shared = Shared::default();
        let frame = Frame {
            width: 1,
            height: 1,
            bgra: vec![0; 4],
        };
        shared.put(frame.clone(), &events);
        shared.put(frame, &events);
        assert_eq!(heard.try_recv(), Ok(Event::Frame));
        assert!(heard.try_recv().is_err());
    }
}
