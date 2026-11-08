//! The windows of the Mac's other apps, for Cox to show one on its stage:
//! what windows there are, and a picture of one as it changes, taken
//! through `ScreenCaptureKit`, so that the human sees the app the agent
//! tries out beside the chat; and the agent's clicks, keys and text,
//! posted to the window's app through Core Graphics while the window
//! stays behind Cox, where the human sees them land. To use the app
//! themselves, the human brings the window itself to the front: an app
//! behind another takes the mouse and keys posted to it only in part, and
//! only through what macOS keeps private.
//!
//! macOS asks the human to allow Cox to record the screen, for the
//! pictures and the windows' titles, and to control the computer, for
//! what the agent posts. On other systems there are no windows to show,
//! and a window asked for ends at once, saying so.

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

/// Whether the human allowed Cox to control the computer, which what the
/// agent posts to an app needs.
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

/// How long since the human last used the Mac's keyboard, mouse or
/// trackpad, in any app, or nothing off macOS, where Cox cannot tell.
#[must_use]
pub fn idle() -> Option<std::time::Duration> {
    system::idle()
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

/// Which of the mouse's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

/// A key pressed with the modifiers held, as Return, or Command-A.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each modifier is held or not, as the event's flags say"
)]
pub struct Stroke {
    /// The key's virtual key code, as the Mac's keyboards have it.
    pub code: u16,
    pub command: bool,
    pub shift: bool,
    pub option: bool,
    pub control: bool,
}

impl Stroke {
    /// The stroke `said` names: a key's name, after the modifiers held,
    /// each with a `+`, as `Enter`, `cmd+a` or `cmd+shift+z`.
    ///
    /// # Errors
    ///
    /// When a name is no key's or modifier's: says which.
    pub fn parse(said: &str) -> Result<Self, String> {
        let mut stroke = Self::default();
        let mut names: Vec<&str> = said.split('+').map(str::trim).collect();
        let key = names.pop().filter(|key| !key.is_empty());
        for name in names {
            match name.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "meta" | "super" => stroke.command = true,
                "shift" => stroke.shift = true,
                "alt" | "opt" | "option" => stroke.option = true,
                "ctrl" | "control" => stroke.control = true,
                _ => return Err(format!("no modifier {name:?}: cmd, shift, option or ctrl")),
            }
        }
        let key = key.ok_or_else(|| format!("no key in {said:?}"))?;
        stroke.code = key_code(key).ok_or_else(|| format!("no key {key:?}"))?;
        Ok(stroke)
    }
}

/// The keys that type a character, with their virtual key codes, as the
/// Mac's keyboards with the US layout have them.
#[rustfmt::skip]
const CHARACTER_KEYS: [(char, u16); 48] = [
    ('a', 0), ('s', 1), ('d', 2), ('f', 3), ('h', 4), ('g', 5), ('z', 6), ('x', 7),
    ('c', 8), ('v', 9), ('b', 11), ('q', 12), ('w', 13), ('e', 14), ('r', 15), ('y', 16),
    ('t', 17), ('1', 18), ('2', 19), ('3', 20), ('4', 21), ('6', 22), ('5', 23), ('=', 24),
    ('9', 25), ('7', 26), ('-', 27), ('8', 28), ('0', 29), (']', 30), ('o', 31), ('u', 32),
    ('[', 33), ('i', 34), ('p', 35), ('l', 37), ('j', 38), ('\'', 39), ('k', 40), (';', 41),
    ('\\', 42), (',', 43), ('/', 44), ('n', 45), ('m', 46), ('.', 47), ('`', 50), (' ', 49),
];

/// The virtual key code of the key `name`: a letter, a digit, a mark, or
/// a key's name, as `Enter`.
fn key_code(name: &str) -> Option<u16> {
    let lower = name.to_ascii_lowercase();
    let mut characters = lower.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        return CHARACTER_KEYS
            .iter()
            .find_map(|&(key, code)| (key == character).then_some(code));
    }
    Some(match lower.as_str() {
        "enter" | "return" => 36,
        "tab" => 48,
        "space" => 49,
        "backspace" => 51,
        "escape" | "esc" => 53,
        "delete" | "forwarddelete" => 117,
        "home" => 115,
        "end" => 119,
        "pageup" => 116,
        "pagedown" => 121,
        "left" | "arrowleft" => 123,
        "right" | "arrowright" => 124,
        "down" | "arrowdown" => 125,
        "up" | "arrowup" => 126,
        "f1" => 122,
        "f2" => 120,
        "f3" => 99,
        "f4" => 118,
        "f5" => 96,
        "f6" => 97,
        "f7" => 98,
        "f8" => 100,
        "f9" => 101,
        "f10" => 109,
        "f11" => 103,
        "f12" => 111,
        _ => return None,
    })
}

/// What the agent does to the window, or how the stage draws it. Where it
/// falls is in points from the window's top left corner.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The stage draws the window this big, in pixels: the pictures come
    /// as big as fits.
    Size {
        width: u32,
        height: u32,
    },
    /// A click of `button`, `count` times in a row, as a double click.
    Click {
        x: f64,
        y: f64,
        button: Button,
        count: u8,
    },
    /// The wheel, in pixels each way, there: scrolling down and right,
    /// toward the content's end, as a page's `scrollBy` does, as they are
    /// positive.
    Scroll {
        x: f64,
        y: f64,
        dx: i32,
        dy: i32,
    },
    Key(Stroke),
    /// Text typed.
    Text(String),
    /// The agent is done with the window for now, as its turn ended.
    Away,
}

/// What the mirror and its worker share.
#[derive(Default)]
struct Shared {
    /// The newest picture, until it is taken.
    frame: Mutex<Option<Frame>>,
    /// Whether the mirror was told of the picture waiting.
    told: AtomicBool,
    /// Where the window is, as the worker last looked.
    bounds: Mutex<Option<Rect>>,
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

    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "only macOS shows windows")
    )]
    /// Keeps where the window is now.
    fn place(&self, bounds: Rect) {
        *self.bounds.lock().unwrap_or_else(PoisonError::into_inner) = Some(bounds);
    }
}

/// A window shown as it changes, with what the agent does posted to its
/// app, until it drops.
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

    /// Asks for pictures `width` by `height` pixels, or as near as the
    /// window's shape lets them fit.
    pub fn resize(&self, width: u32, height: u32) {
        self.send(Input::Size { width, height });
    }

    /// Posts what the agent does, `input`, to the window's app.
    pub fn send(&self, input: Input) {
        let _ = self.to.send(input);
    }

    /// Where the window is, in points, as last looked, once it is found.
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        *self
            .shared
            .bounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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
    fn reads_a_key_with_its_modifiers() {
        let undo = Stroke::parse("cmd+shift+z").unwrap();
        assert_eq!(
            undo,
            Stroke {
                code: 6,
                command: true,
                shift: true,
                ..Stroke::default()
            }
        );
        assert_eq!(Stroke::parse("Enter").unwrap().code, 36);
        assert_eq!(Stroke::parse("ctrl+A").unwrap().code, 0);
        assert_eq!(Stroke::parse("b").unwrap().code, 11);
        assert_eq!(Stroke::parse("l").unwrap().code, 37);
        assert_eq!(Stroke::parse(".").unwrap().code, 47);
        assert_eq!(Stroke::parse("0").unwrap().code, 29);
        assert_eq!(Stroke::parse("ArrowDown").unwrap().code, 125);
        assert!(Stroke::parse("hyper+a").is_err());
        assert!(Stroke::parse("cmd+").is_err());
        assert!(Stroke::parse("nokey").is_err());
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
