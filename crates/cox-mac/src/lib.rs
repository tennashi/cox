//! The windows of the Mac's other apps, for Cox to show one on its stage:
//! what windows there are, a picture of one as it changes, taken through
//! `ScreenCaptureKit`, and the human's mouse and keys, posted to the window's
//! app through Core Graphics, so that the human uses the app from the
//! stage, as the agent sees it, without it in front.
//!
//! The window itself stays where it is, so the stage shows it a frame
//! behind: to feel how the app answers, the human brings the window itself
//! to the front.
//!
//! macOS asks the human to allow Cox both: to record the screen, for the
//! pictures and the windows' titles, and to control the computer, for the
//! mouse and keys. On other systems there are no windows to show, and a
//! window asked for ends at once, saying so.

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

/// Whether the human allowed Cox to control the computer, which the mouse
/// and keys posted to an app need.
#[must_use]
pub fn may_control() -> bool {
    system::may_control()
}

/// Asks the human to allow Cox to control the computer, as
/// [`ask_to_record`] asks to record.
pub fn ask_to_control() {
    system::ask_to_control();
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

/// Which of the mouse's buttons, as a trackpad's two-finger tap is its
/// right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// What a mouse's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Down,
    /// Moves while held down.
    Drag,
    Up,
}

/// What the human does to the window, as seen on the stage.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The stage draws the window this big, in pixels: the pictures come
    /// as big as fits.
    Size { width: u32, height: u32 },
    /// A button of the mouse, where it is across the window, from 0 to 1
    /// each way.
    Mouse {
        x: f32,
        y: f32,
        button: Button,
        press: Press,
    },
    /// The wheel, in pixels, there.
    Wheel { x: f32, y: f32, dx: i32, dy: i32 },
    /// A key that types no text, by its name, as `Enter` or `ArrowLeft`.
    Key(String),
    /// Text typed.
    Text(String),
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

/// A window shown as it changes, with the human's input posted to its app,
/// until it drops.
pub struct Mirror {
    to: std::sync::mpsc::Sender<Input>,
    shared: Arc<Shared>,
}

impl Mirror {
    /// Starts showing the window `id`.
    #[must_use]
    pub fn open(id: u32) -> (Self, UnboundedReceiver<Event>) {
        let (events, heard) = unbounded_channel();
        let (to, inputs) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name(format!("cox-mac-window-{id}"))
            .spawn(move || system::mirror(id, &worker, &inputs, &events));
        if let Err(error) = spawned {
            tracing::error!("cannot start showing window {id}: {error}");
        }
        (Self { to, shared }, heard)
    }

    /// Passes the human's `input` on.
    pub fn send(&self, input: Input) {
        let _ = self.to.send(input);
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
/// Where `x` and `y`, from 0 to 1 across `frame`, are on the screen.
fn on_screen(frame: Rect, x: f32, y: f32) -> (f64, f64) {
    (
        frame.x + f64::from(x.clamp(0., 1.)) * frame.width,
        frame.y + f64::from(y.clamp(0., 1.)) * frame.height,
    )
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
    fn finds_a_point_of_the_window_on_the_screen() {
        assert_eq!(on_screen(FRAME, 0.5, 0.5), (500., 350.));
        assert_eq!(on_screen(FRAME, 2., -1.), (900., 50.));
    }

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
