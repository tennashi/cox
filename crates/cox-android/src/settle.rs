//! Waiting for a display to settle after the agent acts on it, so that the
//! picture the agent gets is of where a tap or a key led, not of the
//! animation on its way there: the display is looked at again and again,
//! until it stays the same a while, or until the most the agent waits.

use std::time::Duration;

/// How long the display stays the same to be settled.
pub const STILL: Duration = Duration::from_millis(300);

/// How often the display is looked at as it settles.
pub const LOOK_AGAIN: Duration = Duration::from_millis(100);

/// How a display stood as the wait ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    /// It stayed the same [`STILL`], as of this long after the act.
    Still(Duration),
    /// It was changing still as the most the agent waits ran out.
    Moving(Duration),
}

/// A display settling: what it showed last, and since when.
#[derive(Debug)]
pub struct Settling {
    most: Duration,
    last: Option<(u64, Duration)>,
}

impl Settling {
    /// A wait of `most` at the most.
    #[must_use]
    pub fn new(most: Duration) -> Self {
        Self { most, last: None }
    }

    /// The display, as `frame` sums it up, `at` this long after the act:
    /// how it stands, once the wait ends.
    pub fn see(&mut self, at: Duration, frame: u64) -> Option<Settled> {
        match self.last {
            Some((last, since)) if last == frame => {
                if at.saturating_sub(since) >= STILL {
                    return Some(Settled::Still(at));
                }
            }
            _ => self.last = Some((frame, at)),
        }
        (at >= self.most).then_some(Settled::Moving(at))
    }
}

/// `pixels`, summed up so that two pictures that differ differ in it, all
/// but surely.
#[must_use]
pub fn sum(pixels: &[u8]) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    pixels.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// How a wait of `most` ends as the display shows `frames`, one each
    /// [`LOOK_AGAIN`] from the act on.
    fn ends(most: Duration, frames: &[u64]) -> Option<Settled> {
        let mut settling = Settling::new(most);
        frames.iter().enumerate().find_map(|(looked, frame)| {
            settling.see(LOOK_AGAIN * u32::try_from(looked).unwrap(), *frame)
        })
    }

    #[test]
    fn a_display_that_stays_the_same_a_while_is_settled() {
        assert_eq!(
            ends(ms(2000), &[1, 1, 1, 1, 1]),
            Some(Settled::Still(ms(300)))
        );
    }

    #[test]
    fn a_change_starts_the_wait_for_stillness_again() {
        assert_eq!(
            ends(ms(2000), &[1, 2, 3, 3, 3, 3, 3]),
            Some(Settled::Still(ms(500)))
        );
        assert_eq!(
            ends(ms(2000), &[1, 1, 2, 2]),
            None,
            "not still long enough yet"
        );
    }

    #[test]
    fn a_display_that_keeps_changing_ends_the_wait_at_the_most() {
        let frames: Vec<u64> = (0..30).collect();
        assert_eq!(ends(ms(1000), &frames), Some(Settled::Moving(ms(1000))));
    }

    #[test]
    fn pictures_that_differ_sum_up_differently() {
        assert_eq!(sum(&[1, 2, 3]), sum(&[1, 2, 3]));
        assert_ne!(sum(&[1, 2, 3]), sum(&[1, 2, 4]));
    }
}
