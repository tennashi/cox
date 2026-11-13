//! The cursor Vim's keys move in code and in an input: on a line, at a character,
//! and across the text by characters and words, as Vim moves it.

use cox_editor::buffer::{Point as Place, is_word};

use crate::vim::keymap::Across;

/// Where the cursor is: its line, and the character it goes to on each
/// line, which a shorter line holds it before, as Vim's `curswant` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub(crate) line: usize,
    /// The character, from 0, or [`usize::MAX`] for each line's last.
    want: usize,
}

impl Cursor {
    pub(crate) const fn on(line: usize) -> Self {
        Self { line, want: 0 }
    }

    /// The cursor at `place` of `text`, its line.
    pub(crate) fn at(place: Place, text: &str) -> Self {
        Self {
            line: place.line,
            want: text[..place.column.min(text.len())].chars().count(),
        }
    }

    /// The same character, on `line`.
    pub(crate) const fn to_line(self, line: usize) -> Self {
        Self { line, ..self }
    }

    /// Where it is on its line, `text`: at the character it goes to, or
    /// the line's last, as a byte of the line.
    pub(crate) fn place(self, text: &str) -> Place {
        let last = text.chars().count().saturating_sub(1);
        let column = text
            .char_indices()
            .nth(self.want.min(last))
            .map_or(0, |(at, _)| at);
        Place::new(self.line, column)
    }
}

/// What kind of character `c` is, for Vim's words: a word is a run of one
/// kind, other than a space.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Space,
    Word,
    Mark,
}

fn kind(c: Option<char>) -> Kind {
    match c {
        None => Kind::Space,
        Some(c) if c.is_whitespace() => Kind::Space,
        Some(c) if is_word(c) => Kind::Word,
        Some(_) => Kind::Mark,
    }
}

/// The brackets `%` goes between, each opening and closing.
const BRACKETS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// A place in the text, stepped through a character at a time, the end
/// of each line a space between it and the next.
struct Text<'a> {
    lines: &'a [String],
}

impl Text<'_> {
    fn char_at(&self, place: Place) -> Option<char> {
        self.lines.get(place.line)?[place.column..].chars().next()
    }

    fn kind_at(&self, place: Place) -> Kind {
        kind(self.char_at(place))
    }

    /// Whether `place` is an empty line, which Vim's `w` and `b` stop at.
    fn empty(&self, place: Place) -> bool {
        self.lines.get(place.line).is_some_and(String::is_empty)
    }

    fn next(&self, place: Place) -> Option<Place> {
        let line = self.lines.get(place.line)?;
        match line[place.column..].chars().next() {
            Some(c) => Some(Place::new(place.line, place.column + c.len_utf8())),
            None => (place.line + 1 < self.lines.len()).then(|| Place::new(place.line + 1, 0)),
        }
    }

    fn previous(&self, place: Place) -> Option<Place> {
        if place.column > 0 {
            let line = &self.lines[place.line];
            let c = line[..place.column].chars().next_back()?;
            return Some(Place::new(place.line, place.column - c.len_utf8()));
        }
        let line = place.line.checked_sub(1)?;
        Some(Place::new(line, self.lines[line].len()))
    }

    /// The start of the next word after `at` (`w`).
    fn word_forward(&self, at: Place) -> Place {
        let mut place = at;
        let start = self.kind_at(at);
        if start != Kind::Space {
            while self.kind_at(place) == start && self.char_at(place).is_some() {
                let Some(next) = self.next(place) else {
                    return self.last_char(place);
                };
                place = next;
            }
        }
        while self.kind_at(place) == Kind::Space && !(place != at && self.empty(place)) {
            let Some(next) = self.next(place) else {
                return self.last_char(place);
            };
            place = next;
        }
        place
    }

    /// The end of the word at or after the one after `at` (`e`).
    fn word_end(&self, at: Place) -> Place {
        let Some(mut place) = self.next(at) else {
            return at;
        };
        while self.kind_at(place) == Kind::Space {
            let Some(next) = self.next(place) else {
                return self.last_char(place);
            };
            place = next;
        }
        let word = self.kind_at(place);
        while let Some(next) = self
            .next(place)
            .filter(|next| next.line == place.line && self.kind_at(*next) == word)
        {
            place = next;
        }
        place
    }

    /// The start of the word before `at`, or of the one it is in (`b`).
    fn word_back(&self, at: Place) -> Place {
        let Some(mut place) = self.previous(at) else {
            return at;
        };
        while self.kind_at(place) == Kind::Space && !self.empty(place) {
            let Some(previous) = self.previous(place) else {
                return place;
            };
            place = previous;
        }
        let word = self.kind_at(place);
        while let Some(previous) = self
            .previous(place)
            .filter(|previous| previous.line == place.line && self.kind_at(*previous) == word)
        {
            place = previous;
        }
        place
    }

    /// The bracket that matches the one at `at`, or else the next on its
    /// line, if it has a match (`%`).
    fn matching_bracket(&self, at: Place) -> Option<Place> {
        let (column, bracket) = self.lines[at.line][at.column..]
            .char_indices()
            .find(|(_, c)| BRACKETS.iter().any(|(open, close)| c == open || c == close))?;
        let &(open, close) = BRACKETS
            .iter()
            .find(|(open, close)| bracket == *open || bracket == *close)?;
        let forward = bracket == open;
        let other = if forward { close } else { open };
        let mut place = Place::new(at.line, at.column + column);
        let mut depth = 0_usize;
        loop {
            place = if forward {
                self.next(place)?
            } else {
                self.previous(place)?
            };
            match self.char_at(place) {
                Some(c) if c == bracket => depth += 1,
                Some(c) if c == other => match depth.checked_sub(1) {
                    Some(outer) => depth = outer,
                    None => return Some(place),
                },
                _ => {}
            }
        }
    }

    /// The next empty line after the text from `line` on, or the one
    /// before it going back: else the last character, or the first (`}`,
    /// `{`).
    fn paragraph(&self, line: usize, forward: bool) -> Place {
        let empty = |line: usize| self.lines[line].is_empty();
        let mut at = line;
        let mut in_text = !empty(at);
        loop {
            let next = if forward {
                Some(at + 1).filter(|next| *next < self.lines.len())
            } else {
                at.checked_sub(1)
            };
            let Some(next) = next else {
                return if forward {
                    self.last_char(Place::new(at, 0))
                } else {
                    Place::new(0, 0)
                };
            };
            at = next;
            if empty(at) && in_text {
                return Place::new(at, 0);
            }
            in_text |= !empty(at);
        }
    }

    /// The last character of `place`'s line, or its start when empty.
    fn last_char(&self, place: Place) -> Place {
        let line = &self.lines[place.line];
        let column = line.char_indices().next_back().map_or(0, |(at, _)| at);
        Place::new(place.line, column)
    }
}

/// Where the cursor at `at` goes in `lines`, the text, as `across` asks,
/// `times` times.
pub(crate) fn moved(across: Across, at: Cursor, times: usize, lines: &[String]) -> Cursor {
    let Some(line) = lines.get(at.line) else {
        return at;
    };
    let text = Text { lines };
    let place = at.place(line);
    let chars = line.chars().count();
    let char_of = |place: Place| Cursor::at(place, &lines[place.line]);
    match across {
        Across::Left => Cursor {
            line: at.line,
            want: char_of(place).want.saturating_sub(times),
        },
        Across::Right => Cursor {
            line: at.line,
            want: char_of(place)
                .want
                .saturating_add(times)
                .min(chars.saturating_sub(1)),
        },
        Across::LineStart => Cursor::on(at.line),
        Across::FirstNonBlank => Cursor {
            line: at.line,
            want: line
                .chars()
                .take_while(|c| c.is_whitespace())
                .count()
                .min(chars.saturating_sub(1)),
        },
        Across::LineEnd => {
            let line = at.line.saturating_add(times - 1).min(lines.len() - 1);
            Cursor {
                line,
                want: usize::MAX,
            }
        }
        Across::WordForward => char_of((0..times).fold(place, |place, _| text.word_forward(place))),
        Across::WordEnd => char_of((0..times).fold(place, |place, _| text.word_end(place))),
        Across::WordBack => char_of((0..times).fold(place, |place, _| text.word_back(place))),
        Across::Bracket => text.matching_bracket(place).map_or(at, char_of),
        Across::ParagraphForward => {
            char_of((0..times).fold(place, |place, _| text.paragraph(place.line, true)))
        }
        Across::ParagraphBack => {
            char_of((0..times).fold(place, |place, _| text.paragraph(place.line, false)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.split('\n').map(str::to_owned).collect()
    }

    /// Where `across`, `times` times, takes the cursor from `column` of
    /// `line`, as the line and the byte of it.
    fn go(
        text: &str,
        (line, column): (usize, usize),
        across: Across,
        times: usize,
    ) -> (usize, usize) {
        let lines = lines(text);
        let at = Cursor::at(Place::new(line, column), &lines[line]);
        let to = moved(across, at, times, &lines);
        let place = to.place(&lines[to.line]);
        (place.line, place.column)
    }

    #[test]
    fn h_and_l_go_along_the_line_and_stop_at_its_ends() {
        let text = "fn main()";
        assert_eq!(go(text, (0, 3), Across::Left, 1), (0, 2));
        assert_eq!(go(text, (0, 1), Across::Left, 5), (0, 0));
        assert_eq!(go(text, (0, 3), Across::Right, 2), (0, 5));
        assert_eq!(go(text, (0, 7), Across::Right, 9), (0, 8));
        assert_eq!(go("", (0, 0), Across::Right, 1), (0, 0));
    }

    #[test]
    fn they_count_characters_not_bytes() {
        let text = "aあい";
        assert_eq!(go(text, (0, 0), Across::Right, 1), (0, 1));
        assert_eq!(go(text, (0, 1), Across::Right, 1), (0, 4));
        assert_eq!(go(text, (0, 4), Across::Left, 1), (0, 1));
    }

    #[test]
    fn zero_caret_and_dollar_go_to_the_line_s_ends() {
        let text = "    helper();";
        assert_eq!(go(text, (0, 8), Across::LineStart, 1), (0, 0));
        assert_eq!(go(text, (0, 0), Across::FirstNonBlank, 1), (0, 4));
        assert_eq!(go(text, (0, 0), Across::LineEnd, 1), (0, 12));
        assert_eq!(go("a\nbc\ndef", (0, 0), Across::LineEnd, 3), (2, 2));
    }

    #[test]
    fn dollar_holds_each_line_s_end_going_down() {
        let lines = lines("abc\nlonger line\nx");
        let end = moved(Across::LineEnd, Cursor::on(0), 1, &lines);
        assert_eq!(end.to_line(1).place(&lines[1]).column, 10);
        assert_eq!(end.to_line(2).place(&lines[2]).column, 0);
    }

    #[test]
    fn a_shorter_line_holds_the_cursor_before_the_character_it_goes_to() {
        let lines = lines("helper();\nab\nhelper();");
        let at = Cursor::at(Place::new(0, 6), &lines[0]);
        assert_eq!(at.to_line(1).place(&lines[1]).column, 1);
        assert_eq!(at.to_line(2).place(&lines[2]).column, 6);
    }

    #[test]
    fn w_goes_to_the_next_word_or_mark_across_lines() {
        let text = "fn main() {\n    helper();\n\n}";
        assert_eq!(go(text, (0, 0), Across::WordForward, 1), (0, 3));
        assert_eq!(go(text, (0, 3), Across::WordForward, 1), (0, 7));
        assert_eq!(go(text, (0, 7), Across::WordForward, 1), (0, 10));
        assert_eq!(go(text, (0, 10), Across::WordForward, 1), (1, 4));
        assert_eq!(go(text, (1, 4), Across::WordForward, 1), (1, 10));
        assert_eq!(
            go(text, (1, 4), Across::WordForward, 2),
            (2, 0),
            "an empty line"
        );
        assert_eq!(go(text, (3, 0), Across::WordForward, 1), (3, 0), "the end");
    }

    #[test]
    fn b_goes_back_to_the_start_of_a_word() {
        let text = "fn main() {\n    helper();";
        assert_eq!(go(text, (0, 5), Across::WordBack, 1), (0, 3));
        assert_eq!(go(text, (0, 3), Across::WordBack, 1), (0, 0));
        assert_eq!(go(text, (1, 4), Across::WordBack, 1), (0, 10));
        assert_eq!(go(text, (0, 0), Across::WordBack, 1), (0, 0));
    }

    #[test]
    fn percent_goes_to_the_matching_bracket_across_lines() {
        let text = "fn main() {\n    let a = [(1), 2];\n}";
        assert_eq!(go(text, (0, 7), Across::Bracket, 1), (0, 8));
        assert_eq!(go(text, (0, 8), Across::Bracket, 1), (0, 7));
        assert_eq!(go(text, (0, 0), Across::Bracket, 1), (0, 8), "the next");
        assert_eq!(go(text, (0, 10), Across::Bracket, 1), (2, 0));
        assert_eq!(go(text, (2, 0), Across::Bracket, 1), (0, 10));
        assert_eq!(go(text, (1, 12), Across::Bracket, 1), (1, 19), "nested");
        assert_eq!(go(text, (1, 13), Across::Bracket, 1), (1, 15));
        assert_eq!(go(text, (1, 20), Across::Bracket, 1), (1, 20), "none");
        assert_eq!(go("(a", (0, 0), Across::Bracket, 1), (0, 0), "unmatched");
    }

    #[test]
    fn braces_go_to_the_empty_lines_between_paragraphs() {
        let text = "a\nb\n\n\nc\n\nd";
        assert_eq!(go(text, (0, 0), Across::ParagraphForward, 1), (2, 0));
        assert_eq!(go(text, (2, 0), Across::ParagraphForward, 1), (5, 0));
        assert_eq!(go(text, (0, 0), Across::ParagraphForward, 2), (5, 0));
        assert_eq!(
            go(text, (5, 0), Across::ParagraphForward, 1),
            (6, 0),
            "the end"
        );
        assert_eq!(go("a\nbc", (0, 0), Across::ParagraphForward, 1), (1, 1));
        assert_eq!(go(text, (6, 0), Across::ParagraphBack, 1), (5, 0));
        assert_eq!(go(text, (5, 0), Across::ParagraphBack, 1), (3, 0));
        assert_eq!(
            go(text, (6, 0), Across::ParagraphBack, 3),
            (0, 0),
            "the start"
        );
    }

    #[test]
    fn e_goes_to_the_end_of_a_word() {
        let text = "fn main() {\n    helper();";
        assert_eq!(go(text, (0, 0), Across::WordEnd, 1), (0, 1));
        assert_eq!(go(text, (0, 1), Across::WordEnd, 1), (0, 6));
        assert_eq!(go(text, (0, 10), Across::WordEnd, 1), (1, 9));
    }
}
