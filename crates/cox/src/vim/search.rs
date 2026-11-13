//! Vim's `/` and `?`, and `n` and `N` after them: the text searched for,
//! which way, and where it is next in a part's text, as each part that
//! searches finds it (see [`Find`]).
//!
//! The text is found as typed, not as a pattern, and as Vim's
//! `smartcase` finds it: whatever its case unless it has a capital. A
//! word `*` or `#` searches for in code is found there only whole.

use std::ops::Range;

use gpui::Action;

/// Which way a search goes through the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    Forward,
    Backward,
}

impl Direction {
    /// This way, or the other when `then` goes back: as `N` goes the
    /// other way from the search it repeats.
    #[must_use]
    pub const fn then(self, then: Self) -> Self {
        match (self, then) {
            (way, Self::Forward) => way,
            (Self::Forward, Self::Backward) => Self::Backward,
            (Self::Backward, Self::Backward) => Self::Forward,
        }
    }
}

/// The last search, which `n` and `N` repeat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Search {
    pub text: String,
    pub direction: Direction,
    /// Whether the text is found only as a whole word, as `*` finds it.
    pub word: bool,
}

/// Finds `text`, as `direction` goes, from where the part with the
/// keyboard is: its next match, which it shows.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct Find {
    pub text: String,
    pub direction: Direction,
    /// Whether the text is found only as a whole word, which only code
    /// heeds: elsewhere it is found anywhere.
    pub word: bool,
}

/// Whether `query` is found in its own case only: when it has a capital.
#[must_use]
pub fn exact(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

/// Where `query` is in `text`, by bytes, none overlapping another.
#[must_use]
pub fn matches(query: &str, text: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let (query, text) = if exact(query) {
        (query.to_owned(), text.to_owned())
    } else {
        (lower(query), lower(text))
    };
    text.match_indices(&query)
        .map(|(at, found)| at..at + found.len())
        .collect()
}

/// Where `query` is in `text`, as [`matches`] finds it, where no letter,
/// digit or `_` is just before or after it: as a whole word.
#[must_use]
pub fn words(query: &str, text: &str) -> Vec<Range<usize>> {
    let word = |c: Option<char>| c.is_some_and(cox_editor::buffer::is_word);
    matches(query, text)
        .into_iter()
        .filter(|range| {
            !word(text[..range.start].chars().next_back())
                && !word(text[range.end..].chars().next())
        })
        .collect()
}

/// `text` in lower case, where that keeps every byte where it was, as it
/// does for ASCII: other letters stay as they are.
fn lower(text: &str) -> String {
    text.chars()
        .map(|letter| {
            let lower = letter.to_lowercase().next().unwrap_or(letter);
            if lower.len_utf8() == letter.len_utf8() {
                lower
            } else {
                letter
            }
        })
        .collect()
}

/// The next match of `query` in `texts`, read in order as one text, from
/// byte `from.1` of text `from.0`, as `direction` goes: after it, or
/// before it, going round at the ends, only as a whole word when `word`.
/// The text it is in, and where.
#[must_use]
pub fn next<S: AsRef<str>>(
    texts: &[S],
    from: (usize, usize),
    query: &str,
    word: bool,
    direction: Direction,
) -> Option<(usize, Range<usize>)> {
    let find = if word { words } else { matches };
    let found: Vec<(usize, Range<usize>)> = texts
        .iter()
        .enumerate()
        .flat_map(|(at, text)| {
            find(query, text.as_ref())
                .into_iter()
                .map(move |range| (at, range))
        })
        .collect();
    let place = |(at, range): &(usize, Range<usize>)| (*at, range.start);
    match direction {
        Direction::Forward => found
            .iter()
            .find(|found| place(found) > from)
            .or_else(|| found.first()),
        Direction::Backward => found
            .iter()
            .rev()
            .find(|found| place(found) < from)
            .or_else(|| found.last()),
    }
    .cloned()
}

#[cfg(test)]
mod tests {
    use super::{Direction, matches, next, words};

    #[test]
    fn a_query_in_lower_case_finds_any_case_and_a_capital_its_own() {
        assert_eq!(matches("ab", "ab AB aB"), [0..2, 3..5, 6..8]);
        assert_eq!(matches("aB", "ab AB aB"), vec![6..8; 1]);
        assert_eq!(matches("", "ab"), Vec::<std::ops::Range<usize>>::new());
        assert_eq!(matches("é", "É é"), [0..2, 3..5]);
    }

    #[test]
    fn the_next_match_goes_on_and_round() {
        let texts = ["one two", "", "two two"];
        let forward = |from| next(&texts, from, "two", false, Direction::Forward);
        assert_eq!(forward((0, 0)), Some((0, 4..7)));
        assert_eq!(forward((0, 4)), Some((2, 0..3)));
        assert_eq!(forward((2, 0)), Some((2, 4..7)));
        assert_eq!(forward((2, 4)), Some((0, 4..7)));
        let backward = |from| next(&texts, from, "two", false, Direction::Backward);
        assert_eq!(backward((2, 4)), Some((2, 0..3)));
        assert_eq!(backward((0, 4)), Some((2, 4..7)));
        assert_eq!(
            next(&texts, (0, 0), "three", false, Direction::Forward),
            None
        );
    }

    #[test]
    fn a_word_is_found_only_whole() {
        assert_eq!(
            words("line", "line lines line_1 (line) é_line"),
            [0..4, 19..23]
        );
        let texts = ["lines", "a line"];
        assert_eq!(
            next(&texts, (0, 0), "line", true, Direction::Forward),
            Some((1, 2..6))
        );
    }

    #[test]
    fn n_goes_the_search_s_way_and_shift_n_the_other() {
        use Direction::{Backward, Forward};
        assert_eq!(Forward.then(Forward), Forward);
        assert_eq!(Forward.then(Backward), Backward);
        assert_eq!(Backward.then(Forward), Backward);
        assert_eq!(Backward.then(Backward), Forward);
    }
}
