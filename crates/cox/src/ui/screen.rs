//! What the screen shows of Cox's window, as a PNG: for an agent that
//! asks Cox how it stands (see [`crate::inspect`]), and for a region of
//! the window the human hands a chat (see [`crate::chat::region`]).

#[cfg(target_os = "macos")]
use std::fs;
#[cfg(not(target_os = "macos"))]
use std::thread;

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
    let file = crate::scratch::folder()
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

/// The screen as it shows `bounds`, in its coordinates, in points, as a
/// PNG, `scale` pixels a point: on X11, through `import`, of
/// `ImageMagick`.
///
/// # Errors
///
/// When the screen cannot be captured.
#[cfg(not(target_os = "macos"))]
pub fn capture(bounds: Bounds<Pixels>, scale: f32) -> Result<Vec<u8>, String> {
    // On a display drawn in memory, as Xvfb's, what the window drew
    // shows only once the pointer has moved: a pixel and back does, and
    // the window has drawn again by the time it is captured.
    for step in ["1", "-1"] {
        let _ = std::process::Command::new("xdotool")
            .args(["mousemove_relative", "--sync", "--", step, "0"])
            .output();
    }
    thread::sleep(std::time::Duration::from_millis(200));
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
