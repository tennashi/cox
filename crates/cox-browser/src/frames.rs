//! When CEF draws a page. CEF draws a picture of the page only when it is
//! asked to (Chromium's external begin frames), so that the pictures come
//! in step with the window's frames, as on a display of 120 Hz, rather
//! than on a clock of CEF's own that drifts against the window's, which
//! shows some pictures for one frame and others for two.
//!
//! While the page is lively, as while the human scrolls it or something
//! on it moves, the stage asks for a picture at each frame of the window,
//! and has the window draw every frame. Otherwise a slow tick asks, which
//! is enough to see a page start changing on its own, and to draw the
//! pages the window does not show, for `DevTools`' screenshots.

use std::time::{Duration, Instant};

use cef::ImplBrowserHost;

use crate::{BrowsersRef, lock};

/// How often a page is asked for a picture while it is not lively.
pub(crate) const IDLE_INTERVAL: Duration = Duration::from_millis(33);

/// How long a page stays lively after it last changed, or the human last
/// worked on it.
const LIVELY_FOR: Duration = Duration::from_millis(500);

/// When a page was last asked for a picture, and until when it is lively.
#[derive(Debug, Default)]
pub(crate) struct Pacing {
    asked: Option<Instant>,
    lively_until: Option<Instant>,
}

impl Pacing {
    /// The page was asked for a picture at `now`.
    pub(crate) fn asked(&mut self, now: Instant) {
        self.asked = Some(now);
    }

    /// The page changed, or the human worked on it, at `now`.
    pub(crate) fn stirred(&mut self, now: Instant) {
        self.lively_until = Some(now + LIVELY_FOR);
    }

    /// Whether the window should ask for a picture at each of its frames.
    pub(crate) fn is_lively(&self, now: Instant) -> bool {
        self.lively_until.is_some_and(|until| now < until)
    }

    /// Whether the slow tick should ask: nothing asked for a while.
    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.asked
            .is_none_or(|asked| now.duration_since(asked) >= IDLE_INTERVAL)
    }
}

/// Asks each page that nothing asked for a while for a picture, every
/// [`IDLE_INTERVAL`], until CEF shuts down.
pub(crate) fn tick(browsers: &BrowsersRef) {
    loop {
        std::thread::sleep(IDLE_INTERVAL);
        let browsers = lock(browsers);
        if browsers.stopped {
            return;
        }
        let now = Instant::now();
        for shared in browsers.all.iter().filter_map(std::sync::Weak::upgrade) {
            let host = {
                let mut shared = lock(&shared);
                if shared.closed || !shared.pacing.is_due(now) {
                    continue;
                }
                shared.pacing.asked(now);
                shared.host.clone()
            };
            // Held under the lock of the browsers, so that CEF does not
            // shut down meanwhile (see `Cef::shutdown`).
            if let Some(host) = host {
                host.send_external_begin_frame();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_lively_for_a_while_after_it_stirs() {
        let start = Instant::now();
        let mut pacing = Pacing::default();
        assert!(!pacing.is_lively(start));
        pacing.stirred(start);
        assert!(pacing.is_lively(start + LIVELY_FOR / 2));
        assert!(!pacing.is_lively(start + LIVELY_FOR));
    }

    #[test]
    fn the_tick_asks_only_when_nothing_asked_for_a_while() {
        let start = Instant::now();
        let mut pacing = Pacing::default();
        assert!(pacing.is_due(start));
        pacing.asked(start);
        assert!(!pacing.is_due(start + IDLE_INTERVAL / 2));
        assert!(pacing.is_due(start + IDLE_INTERVAL));
    }
}
