//! The `frames` tool: how the window's thread and the pages of the stage
//! kept up lately, in words, for an agent that debugs Cox to see where
//! the window, or a page, stuck while the human scrolled it, and when.

use std::fmt::Write as _;
use std::time::Duration;

use cox_browser::{KeptUp, STUCK_AFTER};

use crate::busy::{BUSY_AFTER, Busy};

/// How far back the tool looks unless asked otherwise.
pub const OVER: Duration = Duration::from_secs(10);

/// How many of the waits of a page are told, the last ones.
const TOLD_GAPS: usize = 20;

/// Tells how the window's thread, `busy` with what it was busy with, and
/// the `pages`, by their addresses, kept up over the last `over`.
pub fn tell(over: Duration, busy: &[Busy], pages: &[(String, KeptUp)]) -> String {
    let mut told = format!("Over the last {}:\n", seconds(over));
    if busy.is_empty() {
        let _ = writeln!(
            told,
            "The window's thread was never busy for {} or more.",
            millis(BUSY_AFTER)
        );
    } else {
        let _ = writeln!(
            told,
            "The window's thread was busy for {} or more {} times:",
            millis(BUSY_AFTER),
            busy.len()
        );
        let now = std::time::Instant::now();
        for busy in busy {
            let with = busy
                .with
                .map(|with| format!(", with {with}"))
                .unwrap_or_default();
            let _ = writeln!(
                told,
                "  {} ago, for {}{with}",
                seconds(now.duration_since(busy.at)),
                millis(busy.lasted)
            );
        }
    }
    let _ = writeln!(
        told,
        "Pages, with the waits of {} or more from a turn of the wheel to the next picture \
         (a page at its end paints nothing either):",
        millis(STUCK_AFTER)
    );
    if pages.is_empty() {
        told.push_str("  none\n");
    }
    for (url, kept_up) in pages {
        let _ = write!(
            told,
            "  {url}: the wheel {} times, {} pictures",
            kept_up.wheeled, kept_up.painted
        );
        if kept_up.stuck.is_empty() {
            told.push_str(", no waits\n");
            continue;
        }
        let longest = kept_up
            .stuck
            .iter()
            .map(|stuck| stuck.lasted)
            .max()
            .unwrap_or_default();
        let _ = writeln!(
            told,
            ", {} waits, the longest {}:",
            kept_up.stuck.len(),
            millis(longest)
        );
        let skipped = kept_up.stuck.len().saturating_sub(TOLD_GAPS);
        if skipped > 0 {
            let _ = writeln!(told, "    ({skipped} before these)");
        }
        for stuck in &kept_up.stuck[skipped..] {
            let _ = writeln!(
                told,
                "    {} ago, for {}, the wheel {} times",
                seconds(stuck.ago),
                millis(stuck.lasted),
                stuck.wheeled
            );
        }
    }
    told
}

fn seconds(duration: Duration) -> String {
    format!("{:.1} s", duration.as_secs_f64())
}

fn millis(duration: Duration) -> String {
    format!("{} ms", duration.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cox_browser::Stuck;

    #[test]
    fn a_page_stuck_while_scrolled_is_told_with_when() {
        let pages = [
            (
                "https://example.com/".to_owned(),
                KeptUp {
                    wheeled: 400,
                    painted: 500,
                    stuck: vec![Stuck {
                        ago: Duration::from_millis(3200),
                        lasted: Duration::from_millis(310),
                        wheeled: 40,
                    }],
                },
            ),
            (
                "cox://map/".to_owned(),
                KeptUp {
                    wheeled: 0,
                    painted: 5,
                    stuck: Vec::new(),
                },
            ),
        ];
        let told = tell(OVER, &[], &pages);
        assert_eq!(
            told,
            "Over the last 10.0 s:\n\
             The window's thread was never busy for 100 ms or more.\n\
             Pages, with the waits of 50 ms or more from a turn of the wheel to the next picture \
             (a page at its end paints nothing either):\n  \
             https://example.com/: the wheel 400 times, 500 pictures, 1 waits, the longest 310 ms:\n    \
             3.2 s ago, for 310 ms, the wheel 40 times\n  \
             cox://map/: the wheel 0 times, 5 pictures, no waits\n"
        );
    }
}
