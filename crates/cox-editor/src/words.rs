//! What changed within the lines a change replaced: the words of a line
//! it removed that the line it added in its place does not have, and the
//! other way round, so that the eye goes to them rather than to the
//! whole line, as `delta` and GitHub show them.
//!
//! A run of removed lines followed by a run of added lines is read as the
//! one replaced by the other, each removed line by the first added line
//! after those already taken that has enough in common with it, so that
//! a line put in between does not part the others. Two lines with too
//! little in common are told as replaced whole, as marking most of each
//! would only be noise.

use std::ops::Range;

use crate::changes::Kind;

/// The most of a pair of lines that can have changed, in hundredths of
/// what they hold but spaces, for the words that changed to be told.
const MOST_CHANGED: usize = 60;

/// The most words a line can have for the words that changed in it to be
/// sought, as that takes the product of both lines' words.
const MOST_WORDS: usize = 400;

/// How far down a run of added lines the line a removed line was replaced
/// by is sought, past the last line taken.
const LOOK_AHEAD: usize = 8;

/// Stretches of a line, as ranges of its bytes.
pub type Stretches = Vec<Range<usize>>;

/// The words that changed in each of `lines`, each a line's kind and its
/// text, as bytes of the text: none for a line that is as it was, or that
/// replaced no other, or was replaced by none.
#[must_use]
pub fn within(lines: &[(Kind, &str)]) -> Vec<Stretches> {
    let mut changed_in = vec![Vec::new(); lines.len()];
    let mut at = 0;
    while at < lines.len() {
        let removed = at;
        while at < lines.len() && lines[at].0 == Kind::Removed {
            at += 1;
        }
        let added = at;
        while at < lines.len() && lines[at].0 == Kind::Added {
            at += 1;
        }
        let (removed, added) = (removed..added, added..at);
        if removed.is_empty() && added.is_empty() {
            at += 1;
            continue;
        }
        let mut next = added.start;
        for old in removed {
            let ahead = next..added.end.min(next + LOOK_AHEAD);
            for new in ahead {
                if let Some((gone, came)) = changed(lines[old].1, lines[new].1) {
                    changed_in[old] = gone;
                    changed_in[new] = came;
                    next = new + 1;
                    break;
                }
            }
        }
    }
    changed_in
}

/// What changed between `old`, a line removed, and `new`, the line added
/// in its place: the bytes of each that the other does not have, by
/// words, or `None` when the two have too little in common to tell.
#[must_use]
pub fn changed(old: &str, new: &str) -> Option<(Stretches, Stretches)> {
    let (old_words, new_words) = (words(old), words(new));
    if old_words.len() > MOST_WORDS || new_words.len() > MOST_WORDS {
        return None;
    }
    let (old_kept, new_kept) = common(old, &old_words, new, &new_words);
    let old_changed = joined(old, left(&old_words, &old_kept));
    let new_changed = joined(new, left(&new_words, &new_kept));
    // How much changed, but for spaces, which say little.
    let solid = |text: &str| text.chars().filter(|c| !c.is_whitespace()).count();
    let held = |text: &str, ranges: &[Range<usize>]| -> usize {
        ranges.iter().map(|range| solid(&text[range.clone()])).sum()
    };
    let all = solid(old) + solid(new);
    let gone = held(old, &old_changed) + held(new, &new_changed);
    if all == 0 || gone * 100 > all * MOST_CHANGED {
        return None;
    }
    Some((old_changed, new_changed))
}

/// The words of `text`, as ranges of its bytes: each run of letters,
/// digits and `_`, each run of spaces, and each other character alone.
fn words(text: &str) -> Vec<Range<usize>> {
    let mut words: Vec<Range<usize>> = Vec::new();
    let mut last = None;
    for (at, character) in text.char_indices() {
        let kind = if character.is_alphanumeric() || character == '_' {
            Some(0)
        } else if character.is_whitespace() {
            Some(1)
        } else {
            None
        };
        let end = at + character.len_utf8();
        match words.last_mut() {
            Some(word) if kind.is_some() && kind == last => word.end = end,
            _ => words.push(at..end),
        }
        last = kind;
    }
    words
}

/// Which words of `old` and of `new` the two have in common, in order: the
/// longest run of them both have, as Myers's diff would find it.
fn common(
    old: &str,
    old_words: &[Range<usize>],
    new: &str,
    new_words: &[Range<usize>],
) -> (Vec<bool>, Vec<bool>) {
    let same = |i: usize, j: usize| old[old_words[i].clone()] == new[new_words[j].clone()];
    let (n, m) = (old_words.len(), new_words.len());
    // longest[i][j]: the most words old's from i and new's from j share.
    let mut longest = vec![vec![0_u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            longest[i][j] = if same(i, j) {
                longest[i + 1][j + 1] + 1
            } else {
                longest[i + 1][j].max(longest[i][j + 1])
            };
        }
    }
    let (mut old_kept, mut new_kept) = (vec![false; n], vec![false; m]);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if same(i, j) {
            old_kept[i] = true;
            new_kept[j] = true;
            i += 1;
            j += 1;
        } else if longest[i + 1][j] >= longest[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (old_kept, new_kept)
}

/// The words not `kept`, as ranges, those next to each other made one.
fn left(words: &[Range<usize>], kept: &[bool]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (word, _) in words.iter().zip(kept).filter(|(_, kept)| !**kept) {
        match ranges.last_mut() {
            Some(range) if range.end == word.start => range.end = word.end,
            _ => ranges.push(word.clone()),
        }
    }
    ranges
}

/// `ranges` of `text`, those with only spaces between them made one, so
/// that the words a change put in a row read as one.
fn joined(text: &str, ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut joined: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match joined.last_mut() {
            Some(last) if text[last.end..range.start].chars().all(char::is_whitespace) => {
                last.end = range.end;
            }
            _ => joined.push(range),
        }
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_each_removed_line_with_an_added_line_like_it() {
        use Kind::{Added, Removed, Same};
        let lines = [
            (Same, "fn main() {"),
            (Removed, "    println!(\"Hello!\");"),
            (Added, "    let name = user();"),
            (Added, "    println!(\"Hello, {name}!\");"),
            (Same, "}"),
            (Removed, "let b = 2;"),
            (Same, "x"),
            (Added, "let b = 3;"),
        ];
        let within = within(&lines);
        let said = |line: usize| -> Vec<&str> {
            within[line]
                .iter()
                .map(|range| &lines[line].1[range.clone()])
                .collect()
        };
        // The line put in between is left out, and the one like the line
        // removed shows what it added.
        assert!(within[1].is_empty() && within[2].is_empty());
        assert_eq!(said(3), [", {name}"]);
        // Lines apart, with one as it was between them, replace nothing.
        assert!(within[5].is_empty() && within[7].is_empty());
    }

    /// What changed between `old` and `new`, as the words of each.
    fn told(old: &str, new: &str) -> (Vec<String>, Vec<String>) {
        let (gone, came) = changed(old, new).unwrap();
        let said = |text: &str, ranges: Stretches| -> Vec<String> {
            ranges
                .into_iter()
                .map(|range| text[range].to_owned())
                .collect()
        };
        (said(old, gone), said(new, came))
    }

    #[test]
    fn tells_the_words_that_changed() {
        assert_eq!(
            told(r#"version = "0.1.0""#, r#"version = "0.2.0""#),
            (vec!["1".to_owned()], vec!["2".to_owned()])
        );
        assert_eq!(
            told("    let b = 2;", "    let total = 3;"),
            (
                vec!["b".to_owned(), "2".to_owned()],
                vec!["total".to_owned(), "3".to_owned()]
            )
        );
        // Words in a row, with spaces between them, as one.
        assert_eq!(
            told(
                "/// Greets whoever runs it.",
                "/// Greets the user by name."
            ),
            (
                vec!["whoever runs it".to_owned()],
                vec!["the user by name".to_owned()]
            )
        );
    }

    #[test]
    fn tells_nothing_of_lines_with_too_little_in_common() {
        assert_eq!(changed("println!(\"Hello!\");", "let name = user();"), None);
        assert_eq!(changed("", ""), None);
    }

    #[test]
    fn counts_letters_of_any_script_as_a_word() {
        assert_eq!(
            told("名前 = 1", "名称 = 1"),
            (vec!["名前".to_owned()], vec!["名称".to_owned()])
        );
    }
}
