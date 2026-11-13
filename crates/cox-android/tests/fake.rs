//! A device on `cox-fake-emulator`: started, shown, touched, and stopped,
//! as a tab of the stage does it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cox_android::device::{Boot, Device, Event, Input, Status};
use cox_android::running;

const WAIT: Duration = Duration::from_secs(20);

/// The next event within [`WAIT`].
fn next(events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> Event {
    let waited = Instant::now();
    loop {
        match events.try_recv() {
            Ok(event) => return event,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) if waited.elapsed() < WAIT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("no event: {error}"),
        }
    }
}

fn next_frame(
    device: &Device,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>,
) -> Vec<u8> {
    loop {
        if next(events) == Event::Frame
            && let Some(frame) = device.take_frame()
        {
            assert!(
                frame.width <= 270 && frame.height <= 600,
                "fits the size asked"
            );
            return frame.bgra;
        }
    }
}

#[test]
fn a_device_starts_shows_takes_a_tap_and_stops() {
    let program = PathBuf::from(env!("CARGO_BIN_EXE_cox-fake-emulator"));
    let name = format!("Test_{}", std::process::id());
    let (device, mut events) = Device::open(Some(program), &name, Boot::default());
    device.send(Input::Size {
        width: 270,
        height: 600,
    });
    assert_eq!(next(&mut events), Event::Status(Status::Starting));
    assert_eq!(next(&mut events), Event::Status(Status::Showing));
    assert!(running::of(&name).is_some(), "its file says it runs");

    let before = next_frame(&device, &mut events);
    // A tap on the second row chooses it.
    device.send(Input::Touch {
        x: 0.5,
        y: 0.3,
        down: true,
    });
    device.send(Input::Touch {
        x: 0.5,
        y: 0.3,
        down: false,
    });
    let after = loop {
        let frame = next_frame(&device, &mut events);
        if frame != before {
            break frame;
        }
    };
    assert_ne!(before, after);

    device.stop();
    let waited = Instant::now();
    while running::of(&name).is_some() {
        assert!(waited.elapsed() < WAIT, "the emulator stops");
        std::thread::sleep(Duration::from_millis(50));
    }
}
