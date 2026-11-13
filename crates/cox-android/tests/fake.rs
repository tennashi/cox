//! A device on `cox-fake-emulator`, as a tab of the stage has it: started,
//! shown, touched, typed on, and stopped.

#[path = "../src/scratch.rs"]
mod scratch;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use cox_android::device::{Boot, Device, Event, Frame, Input, Status};
use cox_android::running;
use scratch::Scratch;
use tokio::sync::mpsc::UnboundedReceiver;

/// How long a test waits for what the device tells: longer than the three
/// minutes the device gives an emulator to answer, so that the device,
/// not the test, says that one did not start.
const WAIT: Duration = Duration::from_mins(4);

/// The fake's display, in its own pixels, which frames come at while the
/// tab asks for no size, as its longer side is below 1280.
const DISPLAY: (u32, u32) = (540, 1170);

/// The fake's colors, as RGBA.
const GROUND: [u8; 4] = [250, 250, 249, 255];
const STRIPE: [u8; 4] = [241, 245, 244, 255];
const CHOSEN: [u8; 4] = [204, 240, 233, 255];
const INK: [u8; 4] = [28, 32, 33, 255];
const DARK: [u8; 4] = [27, 27, 31, 255];
const TEAL: [u8; 4] = [15, 118, 110, 255];

/// A pixel at the right of the list's first row, and of its second, where
/// no letter is: the first row is ground and the second striped, until
/// the list scrolls.
const FIRST_ROW: (u32, u32) = (520, 250);
const SECOND_ROW: (u32, u32) = (520, 351);

/// A pixel of the toast Back and Home show, left of its letters.
const TOAST: (u32, u32) = (204, 1000);

/// A device of the fake's, opened by a tab.
struct Fake {
    device: Device,
    events: UnboundedReceiver<Event>,
    name: String,
    /// The fake's `HOME`, where it writes its file.
    home: Scratch,
    runtime: tokio::runtime::Runtime,
}

impl Fake {
    /// A device named after `label`, started on the fake with its clock
    /// still, and its file written in a home of its own.
    fn open(label: &str) -> Self {
        let home = Scratch::new(&format!("fake-{label}"));
        let program = home.path().join("emulator");
        write_program(
            &program,
            &format!(
                "#!/bin/sh\nexport HOME='{home}' XDG_RUNTIME_DIR='{home}'\nexec '{fake}' --still \"$@\"\n",
                home = home.path().display(),
                fake = env!("CARGO_BIN_EXE_cox-fake-emulator"),
            ),
        );
        let name = format!("Test_{label}_{}", std::process::id());
        let (device, events) = Device::open(Some(program), &name, Boot::default());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("a runtime");
        Self {
            device,
            events,
            name,
            home,
            runtime,
        }
    }

    fn next(&mut self) -> Event {
        let events = &mut self.events;
        let next = self
            .runtime
            .block_on(async { tokio::time::timeout(WAIT, events.recv()).await });
        next.expect("the device tells within the wait")
            .expect("the device tells more")
    }

    /// Waits for the device to start and show, as Cox starts it.
    fn showing(&mut self) {
        assert_eq!(self.next(), Event::Status(Status::Starting));
        assert_eq!(self.next(), Event::Status(Status::Showing));
    }

    fn frame(&mut self) -> Frame {
        loop {
            if self.next() == Event::Frame
                && let Some(frame) = self.device.take_frame()
            {
                return frame;
            }
        }
    }

    /// The first frame of which `shows` holds.
    fn frame_where(&mut self, shows: impl Fn(&Frame) -> bool) -> Frame {
        loop {
            let frame = self.frame();
            if shows(&frame) {
                return frame;
            }
        }
    }

    fn send(&self, input: Input) {
        self.device.send(input);
    }

    /// A tap at `at`, in the display's pixels.
    fn tap(&self, at: (u32, u32)) {
        self.touch(at, true);
        self.touch(at, false);
    }

    fn touch(&self, (x, y): (u32, u32), down: bool) {
        self.send(Input::Touch {
            x: across(x, DISPLAY.0),
            y: across(y, DISPLAY.1),
            down,
        });
    }

    /// The folder the fake writes its file in, the first of those of its
    /// home.
    fn running_folder(&self) -> PathBuf {
        let home = self.home.path();
        running::folders_from(|name| {
            matches!(name, "HOME" | "XDG_RUNTIME_DIR").then(|| home.as_os_str().to_owned())
        })
        .remove(0)
    }
}

/// Writes `text` as a program at `path`. A shell writes it, not this
/// process: a program another test started meanwhile would hold this
/// process's file open for writing, and Linux would refuse to run it, as
/// "text file busy".
fn write_program(path: &Path, text: &str) {
    let mut shell = Command::new("/bin/sh")
        .args(["-c", r#"cat > "$0" && chmod +x "$0""#])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .expect("a shell");
    shell
        .stdin
        .take()
        .expect("the shell's input")
        .write_all(text.as_bytes())
        .expect("the program written");
    assert!(shell.wait().expect("the shell ends").success());
}

/// `pixel` of `pixels`, from 0 to 1 across, as the device takes a touch.
#[expect(
    clippy::cast_precision_loss,
    reason = "a display's pixels are far below what f32 keeps exactly"
)]
fn across(pixel: u32, pixels: u32) -> f32 {
    pixel as f32 / (pixels - 1) as f32
}

/// The color of the pixel at `x`, `y`, as RGBA.
fn color(frame: &Frame, (x, y): (u32, u32)) -> [u8; 4] {
    let at = ((y * frame.width + x) * 4) as usize;
    let [blue, green, red, alpha] = frame.bgra[at..at + 4] else {
        panic!("a pixel is four bytes");
    };
    [red, green, blue, alpha]
}

/// Whether any pixel from `left`, `top`, `width` across and `height`
/// down, has `ink`.
fn inked(
    frame: &Frame,
    (left, top): (u32, u32),
    (width, height): (u32, u32),
    ink: [u8; 4],
) -> bool {
    (top..top + height).any(|y| (left..left + width).any(|x| color(frame, (x, y)) == ink))
}

/// Whether the line shows typed letters, not the faint words that ask
/// for them.
fn line_typed(frame: &Frame) -> bool {
    inked(frame, (36, 150), (72, 24), INK)
}

/// Whether the first row says more than two letters, as `Row 1` does
/// and a line of two put at the top does not.
fn first_row_long(frame: &Frame) -> bool {
    inked(frame, (100, 232), (92, 32), INK)
}

#[test]
fn a_device_cox_starts_shows_its_display() {
    let mut fake = Fake::open("shows");
    fake.showing();
    let frame = fake.frame();
    assert_eq!(
        (frame.width, frame.height),
        DISPLAY,
        "the display's own size"
    );
    assert_eq!(color(&frame, (500, 80)), TEAL, "the fake's app bar");
}

#[test]
fn frames_fit_the_size_the_tab_asked() {
    let mut fake = Fake::open("fits");
    fake.send(Input::Size {
        width: 270,
        height: 600,
    });
    fake.showing();
    let frame = fake.frame();
    assert_eq!(
        (frame.width, frame.height),
        (270, 585),
        "half the display, which keeps its shape within 270 by 600"
    );
}

#[test]
fn a_tap_on_a_row_chooses_it() {
    let mut fake = Fake::open("tap");
    fake.showing();
    assert_eq!(color(&fake.frame(), SECOND_ROW), STRIPE, "the row unchosen");
    fake.tap(SECOND_ROW);
    fake.frame_where(|frame| color(frame, SECOND_ROW) == CHOSEN);
}

#[test]
fn a_stopped_device_leaves_no_running_file() {
    let mut fake = Fake::open("stops");
    fake.showing();
    let folder = fake.running_folder();
    let found = running::in_folders(std::slice::from_ref(&folder));
    assert_eq!(
        found
            .iter()
            .map(|running| &running.device)
            .collect::<Vec<_>>(),
        [&fake.name],
        "its file says it runs"
    );
    fake.device.stop();
    while !matches!(fake.next(), Event::Status(Status::Ended(_))) {}
    let left = std::fs::read_dir(&folder).expect("the folder").count();
    assert_eq!(left, 0, "it shut down, taking its file away");
}

#[test]
fn typed_text_shows_on_the_line() {
    let mut fake = Fake::open("types");
    fake.showing();
    assert!(!line_typed(&fake.frame()), "nothing typed yet");
    fake.send(Input::Text("hi".into()));
    fake.frame_where(line_typed);
}

#[test]
fn enter_puts_the_line_at_the_top_of_the_list() {
    let mut fake = Fake::open("enter");
    fake.showing();
    assert!(first_row_long(&fake.frame()), "the first row is Row 1");
    fake.send(Input::Text("hi".into()));
    fake.frame_where(line_typed);
    fake.send(Input::Key("Enter".into()));
    let frame = fake.frame_where(|frame| !first_row_long(frame));
    assert!(!line_typed(&frame), "the line is emptied");
}

#[test]
fn the_wheel_scrolls_the_list() {
    let mut fake = Fake::open("wheel");
    fake.showing();
    fake.frame();
    fake.send(Input::Wheel { dx: 0, dy: -240 });
    fake.frame_where(|frame| {
        color(frame, FIRST_ROW) == STRIPE && color(frame, SECOND_ROW) == GROUND
    });
}

#[test]
fn a_drag_scrolls_the_list_and_chooses_no_row() {
    let mut fake = Fake::open("drag");
    fake.showing();
    fake.frame();
    fake.touch((520, 700), true);
    fake.touch((520, 604), true);
    fake.touch((520, 604), false);
    let frame = fake.frame_where(|frame| {
        color(frame, FIRST_ROW) == STRIPE && color(frame, SECOND_ROW) == GROUND
    });
    assert!(
        !inked(&frame, (0, 200), (540, 886), CHOSEN),
        "no row is chosen"
    );
}

#[test]
fn back_unchooses_the_row_and_says_so() {
    let mut fake = Fake::open("back");
    fake.showing();
    assert_ne!(color(&fake.frame(), TOAST), DARK, "no toast yet");
    fake.tap(SECOND_ROW);
    fake.frame_where(|frame| color(frame, SECOND_ROW) == CHOSEN);
    fake.send(Input::Key("GoBack".into()));
    fake.frame_where(|frame| color(frame, SECOND_ROW) == STRIPE && color(frame, TOAST) == DARK);
}

#[test]
fn home_scrolls_to_the_top_and_says_so() {
    let mut fake = Fake::open("home");
    fake.showing();
    assert_ne!(color(&fake.frame(), TOAST), DARK, "no toast yet");
    fake.send(Input::Wheel { dx: 0, dy: -240 });
    fake.frame_where(|frame| color(frame, FIRST_ROW) == STRIPE);
    fake.send(Input::Key("GoHome".into()));
    fake.frame_where(|frame| {
        color(frame, FIRST_ROW) == GROUND
            && color(frame, SECOND_ROW) == STRIPE
            && color(frame, TOAST) == DARK
    });
}
