//! What the screen shows of Cox's window, as a PNG: for an agent that
//! asks Cox how it stands (see [`crate::inspect`]), and for a region of
//! the window the human hands a chat (see [`crate::chat::region`]).

#[cfg(target_os = "macos")]
use std::fs;

use gpui::{App, Bounds, Pixels, Window};

/// Where `window` is on the screen, in points, as [`capture`] takes it:
/// from the top left of the main display. gpui places a window from the
/// top left of the display it is on, which on a display other than the
/// main one is elsewhere.
#[must_use]
#[cfg(target_os = "macos")]
pub fn window_bounds(window: &Window, cx: &App) -> Bounds<Pixels> {
    let mut bounds = window.bounds();
    if let Some(display) = window.display(cx) {
        let placed = objc2_core_graphics::CGDisplayBounds(u32::from(display.id()));
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a display is placed far within what f32 keeps"
        )]
        let offset = gpui::point(
            gpui::px(placed.origin.x as f32),
            gpui::px(placed.origin.y as f32),
        );
        bounds.origin += offset;
    }
    bounds
}

/// Where `window` is on the screen, in points, as [`capture`] takes it.
#[must_use]
#[cfg(not(target_os = "macos"))]
pub fn window_bounds(window: &Window, _cx: &App) -> Bounds<Pixels> {
    window.bounds()
}

/// The screen as it shows `bounds`, in its coordinates, in points, as a
/// PNG: on macOS, through `screencapture`, which has to be allowed to
/// record the screen.
///
/// # Errors
///
/// When the screen cannot be captured.
#[cfg(target_os = "macos")]
pub fn capture(bounds: Bounds<Pixels>, _scale: f32) -> Result<Vec<u8>, String> {
    let area = format!(
        "{},{},{},{}",
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height)
    );
    let file = coxd::scratch::folder()
        .map_err(|error| format!("cannot make the folder of this run: {error}"))?
        .join("screenshot.png");
    let ran = std::process::Command::new("screencapture")
        .args(["-x", "-R", &area])
        .arg(&file)
        .output()
        .map_err(|error| format!("cannot run screencapture: {error}"))?;
    if !ran.status.success() {
        return Err(format!(
            "screencapture failed: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        ));
    }
    let png = fs::read(&file).map_err(|error| format!("cannot read the screenshot: {error}"));
    let _ = fs::remove_file(&file);
    png
}

/// The `NSView` gpui draws `window` in.
#[must_use]
#[cfg(target_os = "macos")]
pub fn ns_view(window: &Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSView>> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: gpui's handle names its window's view, which lives as long as
    // `window` does.
    unsafe { objc2::rc::Retained::retain(handle.ns_view.as_ptr().cast()) }
}

/// Has gpui draw the window of `view` now, and show it at once, as it
/// does when the system asks the view to show its layer: whether other
/// windows cover it or not. Not while gpui is updating the window, which
/// this updates.
#[cfg(target_os = "macos")]
pub fn draw_now(view: &objc2_app_kit::NSView) {
    // SAFETY: gpui's view answers `displayLayer:`, and asks nothing of the
    // layer it is given.
    unsafe {
        let layer: *mut objc2::runtime::AnyObject = objc2::msg_send![view, layer];
        let _: () = objc2::msg_send![view, displayLayer: layer];
    }
}

/// The `NSWindow` behind `window`.
#[must_use]
#[cfg(target_os = "macos")]
pub fn ns_window(window: &Window) -> Option<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    ns_view(window)?.window()
}

/// Clicks `button` `count` times at `at`, a point of the window gpui draws
/// in `view` as [`capture_window`] shows it, by handing the view the events the system would. Nothing goes
/// through the screen: its pointer does not move, and no other app hears
/// of it. Not while gpui is updating the window, which this updates.
///
/// # Errors
///
/// When the system makes no event.
#[cfg(target_os = "macos")]
pub fn click(
    view: &objc2_app_kit::NSView,
    at: gpui::Point<Pixels>,
    button: gpui::MouseButton,
    count: usize,
) -> Result<(), String> {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSProcessInfo};
    let window = view.window().ok_or("the view is in no window")?;
    let number = window.windowNumber();
    // `at` is a point of the screenshot, which shows the whole window, its
    // title bar too, from the top; the window counts from its bottom.
    let location = NSPoint::new(
        f64::from(f32::from(at.x)),
        window.frame().size.height - f64::from(f32::from(at.y)),
    );
    let (down, up) = match button {
        gpui::MouseButton::Right => (NSEventType::RightMouseDown, NSEventType::RightMouseUp),
        gpui::MouseButton::Middle => (NSEventType::OtherMouseDown, NSEventType::OtherMouseUp),
        _ => (NSEventType::LeftMouseDown, NSEventType::LeftMouseUp),
    };
    for clicks in 1..=count {
        for kind in [down, up] {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                location,
                NSEventModifierFlags::empty(),
                NSProcessInfo::processInfo().systemUptime(),
                number,
                None,
                0,
                isize::try_from(clicks).unwrap_or(1),
                if kind == down { 1.0 } else { 0.0 },
            )
            .ok_or("the system made no mouse event")?;
            match kind {
                NSEventType::LeftMouseDown => view.mouseDown(&event),
                NSEventType::LeftMouseUp => view.mouseUp(&event),
                NSEventType::RightMouseDown => view.rightMouseDown(&event),
                NSEventType::RightMouseUp => view.rightMouseUp(&event),
                NSEventType::OtherMouseDown => view.otherMouseDown(&event),
                _ => view.otherMouseUp(&event),
            }
        }
    }
    Ok(())
}

/// What the window server numbers `window` by, as `screencapture -l`
/// takes it.
#[must_use]
#[cfg(target_os = "macos")]
pub fn window_number(window: &Window) -> Option<isize> {
    ns_window(window).map(|window| window.windowNumber())
}

/// The window the window server numbers `number`, as a PNG, through
/// `screencapture`: what the window draws, whether other windows cover it
/// or not.
///
/// # Errors
///
/// When the window cannot be captured.
#[cfg(target_os = "macos")]
pub fn capture_window(number: isize) -> Result<Vec<u8>, String> {
    let file = coxd::scratch::folder()
        .map_err(|error| format!("cannot make the folder of this run: {error}"))?
        .join("window.png");
    let ran = std::process::Command::new("screencapture")
        .args(["-x", "-o", "-l", &number.to_string()])
        .arg(&file)
        .output()
        .map_err(|error| format!("cannot run screencapture: {error}"))?;
    if !ran.status.success() {
        return Err(format!(
            "screencapture failed: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        ));
    }
    let png = fs::read(&file).map_err(|error| format!("cannot read the screenshot: {error}"));
    let _ = fs::remove_file(&file);
    png
}

/// Moves the pointer a pixel and back, through `xdotool`. On Xvfb, gpui
/// draws no frame at all until input comes, and draws on from then.
///
/// # Errors
///
/// When `xdotool` cannot move the pointer.
#[cfg(not(target_os = "macos"))]
pub fn wake() -> Result<(), String> {
    for step in ["1", "-1"] {
        let ran = std::process::Command::new("xdotool")
            .args(["mousemove_relative", "--sync", "--", step, "0"])
            .output()
            .map_err(|error| format!("cannot run xdotool: {error}"))?;
        if !ran.status.success() {
            return Err(format!(
                "xdotool failed: {}",
                String::from_utf8_lossy(&ran.stderr).trim()
            ));
        }
    }
    Ok(())
}

/// The screen as it shows `bounds`, in its coordinates, in points, as a
/// PNG, `scale` pixels a point: on X11, through `import`, of
/// `ImageMagick`.
///
/// # Errors
///
/// When the screen cannot be captured.
#[cfg(not(target_os = "macos"))]
pub fn capture(bounds: Bounds<Pixels>, scale: f32) -> Result<Vec<u8>, String> {
    let pixels = |points: Pixels| (f32::from(points) * scale).round();
    let area = format!(
        "{}x{}+{}+{}",
        pixels(bounds.size.width),
        pixels(bounds.size.height),
        pixels(bounds.origin.x),
        pixels(bounds.origin.y)
    );
    let ran = std::process::Command::new("import")
        .args(["-window", "root", "-crop", &area, "+repage", "png:-"])
        .output()
        .map_err(|error| format!("cannot run import, of ImageMagick: {error}"))?;
    if !ran.status.success() {
        return Err(format!(
            "import failed: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        ));
    }
    Ok(ran.stdout)
}
