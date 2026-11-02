//! Fitting the page to the stage: the size the stage has, in logical
//! pixels at the window's scale, and the size CEF paints at, which
//! follows it one resize at a time, since CEF holds its pictures back
//! after a resize until one of the size it was told arrives.

use std::time::{Duration, Instant};

use cef::ImplBrowserHost as _;
use cef::sys::cef_paint_element_type_t;

use super::Browser;

/// The stage's size in logical pixels, and the window's scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Viewport {
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) scale: f32,
}

impl Viewport {
    /// The size of the picture CEF paints for this viewport, in device
    /// pixels: it rounds up, as Chromium does.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "pixel counts are small and positive; rounding is the point"
    )]
    pub(super) fn device_size(self) -> (u32, u32) {
        let device = |logical: i32| (logical.max(1) as f32 * self.scale).ceil() as u32;
        (device(self.width), device(self.height))
    }

    /// The viewport, at this scale, that CEF paints a picture of `device`
    /// pixels for: the one to report while waiting for it.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "pixel counts are small and positive; rounding is the point"
    )]
    fn painted_as(device: (u32, u32), scale: f32) -> Self {
        // Rounding up may overshoot by one when the scale is fractional;
        // one logical pixel less then lands exactly.
        let logical = |pixels: u32| {
            let guess = (pixels as f32 / scale).ceil() as i32;
            let fits = |logical: i32| (logical as f32 * scale).ceil() as u32 == pixels;
            (guess - 1..=guess)
                .find(|&logical| logical >= 1 && fits(logical))
                .unwrap_or(guess)
        };
        Self {
            width: logical(device.0),
            height: logical(device.1),
            scale,
        }
    }
}

/// How long CEF gets to paint at a new size on its own before it is asked
/// for a picture, and asked again.
const RESIZE_NUDGE: Duration = Duration::from_millis(200);

/// How long CEF gets to paint at a new size before it is taken to be
/// stuck at another.
const RESIZE_TIMEOUT: Duration = Duration::from_secs(1);

/// A size CEF was told of, while no picture of that size has come yet.
#[derive(Clone, Copy, Debug)]
pub(super) struct Resizing {
    since: Instant,
    /// When CEF was last asked for a picture meanwhile.
    nudged: Option<Instant>,
}

/// What to ask CEF for in [`Browser::settle_resize`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    /// Lay the page out at the size `view_rect` reports now.
    Resize,
    /// Paint a picture of the page as it is.
    Picture,
}

impl Browser {
    /// Tells CEF the stage's size in logical pixels and the window's scale
    /// factor. CEF hears of one size at a time: the next waits until a
    /// picture of this one came (see [`Browser::settle_resize`]).
    pub fn resize(&self, width: i32, height: i32, scale: f32) {
        self.shared().wanted = Viewport {
            width,
            height,
            scale,
        };
        self.settle_resize();
    }

    /// Moves a resize along: tells CEF of the stage's size once it painted
    /// the last size it was told. CEF does not always hand over the picture
    /// of a new size by itself, so after [`RESIZE_NUDGE`] it is asked for
    /// one, and when none of that size came within [`RESIZE_TIMEOUT`], the
    /// page is taken to be stuck at the size of its last picture: that size
    /// is reported and a picture asked for, so that the picture CEF waits
    /// for comes and the next size can be told. Call it now and then as
    /// well as on every resize.
    pub fn settle_resize(&self) {
        let mut shared = self.shared();
        let Some(host) = shared.host.clone() else {
            // Nothing has been told yet: the browser is created at this size.
            shared.reported = shared.wanted;
            return;
        };
        let now = Instant::now();
        let was = shared.reported;
        let ask = match shared.resizing {
            None if shared.wanted != shared.reported => {
                shared.reported = shared.wanted;
                shared.resizing = Some(Resizing {
                    since: now,
                    nudged: None,
                });
                Ask::Resize
            }
            Some(resizing) if now.duration_since(resizing.since) >= RESIZE_TIMEOUT => {
                let painted = shared.painted;
                tracing::debug!("no picture at {was:?} yet, painted {painted:?}");
                let Some(painted) = painted else {
                    return;
                };
                shared.reported = Viewport::painted_as(painted, was.scale);
                shared.resizing = Some(Resizing {
                    since: now,
                    nudged: None,
                });
                Ask::Picture
            }
            Some(resizing)
                if now.duration_since(resizing.nudged.unwrap_or(resizing.since))
                    >= RESIZE_NUDGE =>
            {
                shared.resizing = Some(Resizing {
                    nudged: Some(now),
                    ..resizing
                });
                Ask::Picture
            }
            _ => return,
        };
        let reported = shared.reported;
        // CEF answers these by asking the render handler for the screen
        // info, which locks `shared` too: on macOS that happens right here,
        // on this thread, so the lock must be gone by then.
        drop(shared);
        tracing::debug!("asking CEF: {ask:?} at {reported:?}");
        #[expect(clippy::float_cmp, reason = "the same value, or not")]
        if reported.scale != was.scale {
            host.notify_screen_info_changed();
        }
        match ask {
            Ask::Resize => host.was_resized(),
            Ask::Picture => host.invalidate(cef::PaintElementType::from(
                cef_paint_element_type_t::PET_VIEW,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Viewport;

    #[test]
    fn device_size_rounds_up() {
        let viewport = Viewport {
            width: 635,
            height: 400,
            scale: 1.5,
        };
        assert_eq!(viewport.device_size(), (953, 600));
    }

    #[test]
    fn painted_as_finds_the_viewport_behind_a_picture() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for logical in [1, 2, 3, 635, 636, 888, 1113] {
                let viewport = Viewport {
                    width: logical,
                    height: logical,
                    scale,
                };
                let back = Viewport::painted_as(viewport.device_size(), scale);
                assert_eq!(back.device_size(), viewport.device_size(), "{viewport:?}");
            }
        }
    }
}
