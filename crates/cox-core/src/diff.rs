//! The lines a change to a file shows in the conversation: what it removes
//! and adds, with a few unchanged lines around each change, and the rest
//! left out.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

/// How many unchanged lines show on each side of a change.
const CONTEXT: usize = 3;

/// How long finding the change may take before settling for a rougher one,
/// so that a large file does not hold up the session.
const TIMEOUT: Duration = Duration::from_millis(500);

/// One line of a change, as shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Line {
    /// An unchanged line near a change.
    Same(String),
    Removed(String),
    Added(String),
    /// This many unchanged lines, left out.
    Skipped(usize),
}

/// The lines that show the change from `old`, or from no file when it is
/// `None`, to `new`.
#[must_use]
pub fn lines(old: Option<&str>, new: &str) -> Vec<Line> {
    let diff = TextDiff::configure()
        .timeout(TIMEOUT)
        .diff_lines(old.unwrap_or_default(), new);
    let mut lines = Vec::new();
    // How many of the old lines are shown or skipped so far.
    let mut done = 0;
    for group in diff.grouped_ops(CONTEXT) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let start = first.old_range().start;
        if start > done {
            lines.push(Line::Skipped(start - done));
        }
        for op in &group {
            for change in diff.iter_changes(op) {
                let text = change.value().trim_end_matches(['\n', '\r']).to_owned();
                lines.push(match change.tag() {
                    ChangeTag::Equal => Line::Same(text),
                    ChangeTag::Delete => Line::Removed(text),
                    ChangeTag::Insert => Line::Added(text),
                });
            }
        }
        done = last.old_range().end;
    }
    if diff.old_len() > done {
        lines.push(Line::Skipped(diff.old_len() - done));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(lines: std::ops::RangeInclusive<usize>) -> String {
        lines.map(|n| n.to_string() + "\n").collect()
    }

    #[test]
    fn shows_a_new_file_as_added() {
        assert_eq!(
            lines(None, "one\ntwo\n"),
            [Line::Added("one".to_owned()), Line::Added("two".to_owned())]
        );
    }

    #[test]
    fn shows_unchanged_lines_near_a_change_and_skips_the_rest() {
        let old = numbered(1..=20);
        let new = old.replace("10\n", "ten\n");

        assert_eq!(
            lines(Some(&old), &new),
            [
                Line::Skipped(6),
                Line::Same("7".to_owned()),
                Line::Same("8".to_owned()),
                Line::Same("9".to_owned()),
                Line::Removed("10".to_owned()),
                Line::Added("ten".to_owned()),
                Line::Same("11".to_owned()),
                Line::Same("12".to_owned()),
                Line::Same("13".to_owned()),
                Line::Skipped(7),
            ]
        );
    }

    #[test]
    fn skips_between_changes_far_apart() {
        let old = numbered(1..=12);
        let new = format!("one\n{}twelve\n", numbered(2..=11));

        assert_eq!(
            lines(Some(&old), &new),
            [
                Line::Removed("1".to_owned()),
                Line::Added("one".to_owned()),
                Line::Same("2".to_owned()),
                Line::Same("3".to_owned()),
                Line::Same("4".to_owned()),
                Line::Skipped(4),
                Line::Same("9".to_owned()),
                Line::Same("10".to_owned()),
                Line::Same("11".to_owned()),
                Line::Removed("12".to_owned()),
                Line::Added("twelve".to_owned()),
            ]
        );
    }

    #[test]
    fn shows_a_file_emptied_as_each_line_removed() {
        assert_eq!(
            lines(Some("one\ntwo\n"), ""),
            [
                Line::Removed("one".to_owned()),
                Line::Removed("two".to_owned())
            ]
        );
    }

    #[test]
    fn shows_an_unchanged_file_as_its_lines_left_out() {
        assert_eq!(lines(Some("one\ntwo\n"), "one\ntwo\n"), [Line::Skipped(2)]);
    }

    #[test]
    fn shows_a_new_empty_file_as_no_lines() {
        assert_eq!(lines(None, ""), []);
    }

    #[test]
    fn shows_lines_without_their_breaks_of_either_kind() {
        assert_eq!(
            lines(Some("one\r\ntwo\r\n"), "one\r\n2\r\n"),
            [
                Line::Same("one".to_owned()),
                Line::Removed("two".to_owned()),
                Line::Added("2".to_owned()),
            ]
        );
    }
}
