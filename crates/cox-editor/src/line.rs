//! A line of the text as it is shown: with its tab characters as spaces,
//! cut when it is very long, and in stretches as its syntax says, each
//! of which the editor draws in its own style.

use std::ops::Range;

use crate::language::Highlight;

/// How many columns apart the stops of a tab character are.
pub const TAB_STOPS: usize = 4;

/// The most of a line that is shown, in bytes, so that a line as long as a
/// minified file's does not hold up every frame.
pub const LINE_LENGTH: usize = 4096;

/// A line as it is shown: with its tab characters as spaces to the next
/// stop, and cut at [`LINE_LENGTH`].
#[derive(Clone, Debug)]
pub struct Shown {
    /// The text shown.
    pub text: String,
    /// Where each character of the line starts, in the text drawn and in
    /// the line, in order.
    starts: Vec<(usize, usize)>,
    /// How long the line is, in bytes.
    pub len: usize,
}

impl Shown {
    #[must_use]
    pub fn of(line: &str) -> Self {
        let mut text = String::with_capacity(line.len().min(LINE_LENGTH));
        let mut starts = Vec::with_capacity(line.len().min(LINE_LENGTH));
        let mut columns = 0;
        for (at, c) in line.char_indices() {
            if text.len() >= LINE_LENGTH {
                break;
            }
            starts.push((text.len(), at));
            if c == '\t' {
                let spaces = TAB_STOPS - columns % TAB_STOPS;
                text.extend(std::iter::repeat_n(' ', spaces));
                columns += spaces;
            } else {
                text.push(c);
                columns += 1;
            }
        }
        Self {
            text,
            starts,
            len: line.len(),
        }
    }

    /// The byte of the line that `index` of the text drawn is at.
    #[must_use]
    pub fn column_of(&self, index: usize) -> usize {
        match self.starts.iter().rposition(|&(shown, _)| shown <= index) {
            Some(at) if self.starts[at].0 == index => self.starts[at].1,
            // Past the last character drawn, or within a tab's spaces,
            // which go to the next character.
            Some(at) => self.starts.get(at + 1).map_or(self.len, |&(_, line)| line),
            None => 0,
        }
    }

    /// The stretches of the text drawn, in order, each with how long it is
    /// and what it is, from `highlights`, what the stretches of the line
    /// are.
    #[must_use]
    pub fn runs(
        &self,
        highlights: &[(Range<usize>, Highlight)],
    ) -> Vec<(usize, Option<Highlight>)> {
        let mut runs: Vec<(usize, Option<Highlight>)> = Vec::new();
        let mut highlights = highlights.iter().peekable();
        for (at, &(shown, column)) in self.starts.iter().enumerate() {
            let end = self
                .starts
                .get(at + 1)
                .map_or(self.text.len(), |&(end, _)| end);
            while highlights
                .next_if(|(range, _)| range.end <= column)
                .is_some()
            {}
            let highlight = highlights
                .peek()
                .filter(|(range, _)| range.start <= column)
                .map(|&&(_, highlight)| highlight);
            match runs.last_mut() {
                Some((len, last)) if *last == highlight => *len += end - shown,
                _ => runs.push((end - shown, highlight)),
            }
        }
        runs
    }

    /// The index of the text drawn that `column` of the line is at.
    #[must_use]
    pub fn index_of(&self, column: usize) -> usize {
        self.starts
            .iter()
            .find(|&&(_, line)| line >= column)
            .map_or(self.text.len(), |&(shown, _)| shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_tab_characters_as_spaces_to_the_next_stop() {
        let shown = Shown::of("\tab\tc");
        assert_eq!(shown.text, "    ab  c");
        // In the spaces of the first tab, and right on `a`.
        assert_eq!(shown.column_of(2), 1);
        assert_eq!(shown.column_of(4), 1);
        assert_eq!(shown.column_of(9), 5);
        assert_eq!(shown.index_of(3), 6);
        assert_eq!(shown.index_of(5), 9);
    }

    #[test]
    fn draws_each_stretch_of_a_line_as_its_syntax_says() {
        // The tab stands for four spaces, and `x` is within no stretch.
        let shown = Shown::of("\tlet x = 1;");
        let highlights = vec![(1..4, Highlight::Keyword), (9..10, Highlight::Number)];
        assert_eq!(
            shown.runs(&highlights),
            [
                (4, None),
                (3, Some(Highlight::Keyword)),
                (5, None),
                (1, Some(Highlight::Number)),
                (1, None),
            ]
        );
        assert_eq!(Shown::of("").runs(&highlights).first(), None);
    }

    #[test]
    fn cuts_a_long_line() {
        let shown = Shown::of(&"x".repeat(LINE_LENGTH * 2));
        assert_eq!(shown.text.len(), LINE_LENGTH);
        assert_eq!(shown.column_of(LINE_LENGTH), LINE_LENGTH * 2);
    }
}
