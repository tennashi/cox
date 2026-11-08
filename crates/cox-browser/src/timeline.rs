//! How a page's pictures kept up with the wheel lately, for an agent that
//! debugs Cox to ask (see [`crate::Cef::kept_up`]): when the wheel
//! scrolled the page, and when CEF painted it. A page the wheel scrolls
//! that paints nothing for a while after looks stuck to the human, as
//! when what carries the wheel and the asks for pictures to the page is
//! busy elsewhere; a page the wheel cannot scroll further, at its end,
//! paints nothing either. A page that does not change paints nothing
//! while the wheel rests, so it is the wait from a turn of the wheel to
//! the next picture that tells.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long what happened is kept.
const KEPT_FOR: Duration = Duration::from_mins(1);

/// A wait from a turn of the wheel to the next picture that the human
/// sees: three frames of a display of 60 Hz.
pub const STUCK_AFTER: Duration = Duration::from_millis(50);

/// When the wheel scrolled a page, and when CEF painted it, over the last
/// [`KEPT_FOR`].
#[derive(Debug, Default)]
pub(crate) struct Timeline {
    wheeled: VecDeque<Instant>,
    painted: VecDeque<Instant>,
}

impl Timeline {
    /// The wheel scrolled the page at `now`.
    pub(crate) fn wheeled(&mut self, now: Instant) {
        keep(&mut self.wheeled, now);
    }

    /// CEF painted the page at `now`.
    pub(crate) fn painted(&mut self, now: Instant) {
        keep(&mut self.painted, now);
    }

    /// How the page kept up over the `over` until `now`.
    pub(crate) fn kept_up(&self, now: Instant, over: Duration) -> KeptUp {
        let since = now.checked_sub(over).unwrap_or(now);
        let within = |times: &VecDeque<Instant>| times.iter().filter(|&&at| at >= since).count();
        // The gaps between pictures, from `since` to `now`, and in each,
        // the wait from the first turn of the wheel in it to the picture
        // that ends it.
        let mut edges = vec![since];
        edges.extend(self.painted.iter().copied().filter(|&at| at >= since));
        edges.push(now);
        let stuck = edges
            .windows(2)
            .filter_map(|gap| {
                let (from, to) = (gap[0], gap[1]);
                let mut wheeled = self.wheeled.iter().filter(|&&at| at >= from && at < to);
                let first = *wheeled.next()?;
                let lasted = to.duration_since(first);
                (lasted >= STUCK_AFTER).then(|| Stuck {
                    ago: now.duration_since(first),
                    lasted,
                    wheeled: 1 + wheeled.count(),
                })
            })
            .collect();
        KeptUp {
            wheeled: within(&self.wheeled),
            painted: within(&self.painted),
            stuck,
        }
    }
}

/// Keeps `now` in `times`, forgetting what is older than [`KEPT_FOR`].
fn keep(times: &mut VecDeque<Instant>, now: Instant) {
    while times
        .front()
        .is_some_and(|&first| now.duration_since(first) > KEPT_FOR)
    {
        times.pop_front();
    }
    times.push_back(now);
}

/// How a page's pictures kept up with the wheel over a while.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptUp {
    /// How many times the wheel scrolled the page.
    pub wheeled: usize,
    /// How many pictures CEF painted of it.
    pub painted: usize,
    /// The waits of [`STUCK_AFTER`] or more from a turn of the wheel to
    /// the next picture, the oldest first.
    pub stuck: Vec<Stuck>,
}

/// A wait from a turn of the wheel to the next picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stuck {
    /// How long ago the wheel turned.
    pub ago: Duration,
    /// How long it lasted, until a picture came, or until now.
    pub lasted: Duration,
    /// How many times the wheel turned meanwhile, that one with them.
    pub wheeled: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn a_gap_in_the_pictures_while_the_wheel_scrolls_is_stuck() {
        let start = Instant::now();
        let mut timeline = Timeline::default();
        // Every 16 ms, a turn of the wheel and a picture, but for 300 ms
        // from 100 ms on, when the wheel turns and no picture comes.
        for at in (0..1000).step_by(16) {
            let at = ms(at);
            timeline.wheeled(start + at);
            if at < ms(100) || at > ms(400) {
                timeline.painted(start + at + ms(1));
            }
        }
        let now = start + ms(1000);
        let kept_up = timeline.kept_up(now, ms(1000));
        assert_eq!(kept_up.wheeled, 63);
        assert_eq!(kept_up.stuck.len(), 1, "{kept_up:?}");
        let stuck = kept_up.stuck[0];
        assert_eq!(stuck.ago, ms(1000 - 112));
        assert_eq!(stuck.lasted, ms(417 - 112));
        assert_eq!(stuck.wheeled, 20);
    }

    #[test]
    fn a_gap_while_the_wheel_rests_is_not() {
        let start = Instant::now();
        let mut timeline = Timeline::default();
        timeline.wheeled(start);
        timeline.painted(start + ms(1));
        timeline.painted(start + ms(900));
        let kept_up = timeline.kept_up(start + ms(1000), ms(1000));
        assert_eq!((kept_up.wheeled, kept_up.painted), (1, 2));
        assert!(kept_up.stuck.is_empty(), "{kept_up:?}");
    }

    #[test]
    fn a_page_that_paints_nothing_since_the_wheel_turned_is_stuck_until_now() {
        let start = Instant::now();
        let mut timeline = Timeline::default();
        timeline.painted(start);
        timeline.wheeled(start + ms(500));
        let kept_up = timeline.kept_up(start + ms(1000), ms(800));
        assert_eq!(
            kept_up.stuck,
            [Stuck {
                ago: ms(500),
                lasted: ms(500),
                wheeled: 1
            }]
        );
    }

    #[test]
    fn a_page_that_rested_long_and_paints_at_once_as_the_wheel_turns_is_not() {
        let start = Instant::now();
        let mut timeline = Timeline::default();
        timeline.painted(start);
        timeline.wheeled(start + ms(5000));
        timeline.painted(start + ms(5010));
        let kept_up = timeline.kept_up(start + ms(6000), ms(6000));
        assert!(kept_up.stuck.is_empty(), "{kept_up:?}");
    }

    #[test]
    fn a_turn_of_the_wheel_before_the_while_asked_about_is_left_out() {
        let start = Instant::now();
        let mut timeline = Timeline::default();
        timeline.painted(start);
        timeline.wheeled(start + ms(100));
        timeline.painted(start + ms(900));
        let kept_up = timeline.kept_up(start + ms(1000), ms(500));
        assert!(kept_up.stuck.is_empty(), "{kept_up:?}");
    }

    #[test]
    fn what_is_older_than_a_while_is_forgotten() {
        let start = Instant::now();
        let mut timeline = Timeline::default();
        timeline.wheeled(start);
        timeline.wheeled(start + KEPT_FOR + ms(1));
        assert_eq!(timeline.wheeled.len(), 1);
    }
}
