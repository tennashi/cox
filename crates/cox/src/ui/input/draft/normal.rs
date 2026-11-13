//! What Vim's normal mode does to the text of an input: moves its cursor,
//! which is on a character rather than between two, and takes out,
//! changes, keeps and puts what a motion, whole lines or a word covers.

use std::ops::Range;

use cox_editor::buffer::Point as Place;

use super::{Draft, Kind};
use crate::vim::cursor::{self, Cursor};
use crate::vim::keymap::{Across, Edit, Span};

impl Draft {
    /// Does `edit`, `times` times, and answers what it took out or kept,
    /// to put later: whole lines end in a line break. `kept` is what `p`
    /// puts.
    pub fn edit_normal(&mut self, edit: Edit, times: usize, kept: Option<&str>) -> Option<String> {
        self.marked = None;
        let taken = match edit {
            Edit::Move(across) => {
                let to = self.moved(across, times);
                self.move_to(to);
                None
            }
            Edit::Delete(Span::Line) => Some(self.delete_lines(times)),
            Edit::Delete(span) => {
                let range = self.span(span, times, false);
                Some(self.take(range))
            }
            Edit::Change(span) => {
                let range = self.span(span, times, true);
                return Some(self.take(range));
            }
            Edit::Yank(span) => {
                let range = self.span(span, times, false);
                let kept = self.text[range.clone()].to_owned();
                if span == Span::Line {
                    Some(kept + "\n")
                } else {
                    self.move_to(range.start);
                    Some(kept)
                }
            }
            Edit::Put { before } => {
                if let Some(kept) = kept.filter(|kept| !kept.is_empty()) {
                    self.put(kept, before, times);
                }
                None
            }
            Edit::Undo => {
                (0..times).take_while(|_| self.undo()).for_each(drop);
                None
            }
            Edit::Redo => {
                (0..times).take_while(|_| self.redo()).for_each(drop);
                None
            }
        };
        self.settle();
        taken
    }

    /// Leaves insert mode: the cursor goes onto the character before it,
    /// as Vim's does, unless it is at its line's start.
    pub fn leave_insert(&mut self) {
        self.marked = None;
        let cursor = self.cursor();
        let start = self.line_range(self.place_of(cursor).line).start;
        if cursor > start {
            self.move_to(self.previous_boundary(cursor));
        } else {
            self.move_to(cursor);
        }
        self.settle();
    }

    /// Puts the cursor on a character of its line, its last at most, with
    /// nothing selected, as normal mode has it.
    fn settle(&mut self) {
        let cursor = self.cursor();
        let line = self.line_range(self.place_of(cursor).line);
        let at = if cursor >= line.end && line.end > line.start {
            self.previous_boundary(line.end)
        } else {
            cursor.min(line.end)
        };
        self.move_to(at);
    }

    fn lines(&self) -> Vec<String> {
        self.text.split('\n').map(str::to_owned).collect()
    }

    /// The line and the byte of it that `offset` of the text is at.
    fn place_of(&self, offset: usize) -> Place {
        let before = &self.text[..offset];
        let line = before.matches('\n').count();
        let start = before.rfind('\n').map_or(0, |at| at + 1);
        Place::new(line, offset - start)
    }

    /// The bytes of line `line`, from 0, without its line break.
    fn line_range(&self, line: usize) -> Range<usize> {
        let mut start = 0;
        for (index, text) in self.text.split('\n').enumerate() {
            if index == line {
                return start..start + text.len();
            }
            start += text.len() + 1;
        }
        self.text.len()..self.text.len()
    }

    fn line_count(&self) -> usize {
        self.text.matches('\n').count() + 1
    }

    /// Where `across`, `times` times, takes the cursor.
    fn moved(&self, across: Across, times: usize) -> usize {
        let lines = self.lines();
        let place = self.place_of(self.cursor());
        let at = Cursor::at(place, &lines[place.line]);
        let to = cursor::moved(across, at, times, &lines);
        let to = to.place(&lines[to.line]);
        self.line_range(to.line).start + to.column
    }

    /// What `span`, `times` times, covers from the cursor; to be changed,
    /// a word goes to its end, as Vim's `cw` does, and lines keep their
    /// line breaks.
    fn span(&self, span: Span, times: usize, change: bool) -> Range<usize> {
        let cursor = self.cursor();
        let place = self.place_of(cursor);
        let line = self.line_range(place.line);
        match span {
            Span::Line => {
                let last = (place.line + times - 1).min(self.line_count() - 1);
                line.start..self.line_range(last).end
            }
            Span::InnerWord => self.word_at(cursor),
            Span::AWord => self.a_word(cursor),
            Span::To(Across::Right) => {
                let end = self.text[cursor..line.end]
                    .char_indices()
                    .nth(times)
                    .map_or(line.end, |(at, _)| cursor + at);
                cursor..end
            }
            Span::To(Across::LineEnd) => {
                let last = (place.line + times - 1).min(self.line_count() - 1);
                cursor..self.line_range(last).end
            }
            Span::To(Across::WordForward)
                if change
                    && self.text[cursor..]
                        .chars()
                        .next()
                        .is_some_and(|c| !c.is_whitespace()) =>
            {
                self.through(cursor, self.moved(Across::WordEnd, times))
            }
            Span::To(Across::WordEnd) => self.through(cursor, self.moved(Across::WordEnd, times)),
            Span::To(Across::WordForward) => {
                let to = self.moved(Across::WordForward, times);
                // With no word after it on its line, the word is taken to
                // the line's end, without the break.
                if self.place_of(to).line > place.line
                    || to <= self.moved(Across::WordEnd, times).max(cursor)
                {
                    cursor..line.end
                } else {
                    cursor..to
                }
            }
            Span::To(across) => {
                let to = self.moved(across, times);
                cursor.min(to)..cursor.max(to)
            }
        }
    }

    /// From `from` through the character at `to`.
    fn through(&self, from: usize, to: usize) -> Range<usize> {
        let end = self.text[to..]
            .chars()
            .next()
            .map_or(to, |c| to + c.len_utf8());
        from..end.max(from)
    }

    /// The word at `offset` with the spaces after it, or if none, those
    /// before it.
    fn a_word(&self, offset: usize) -> Range<usize> {
        let word = self.word_at(offset);
        let line = self.line_range(self.place_of(offset).line);
        let after = self.text[word.end..line.end]
            .find(|c: char| !matches!(c, ' ' | '\t'))
            .map_or(line.end, |at| word.end + at);
        if after > word.end {
            return word.start..after;
        }
        let before = self.text[line.start..word.start]
            .rfind(|c: char| !matches!(c, ' ' | '\t'))
            .map_or(line.start, |at| line.start + at + 1);
        before..word.end
    }

    /// Takes out `range`, leaving the cursor where it was, and answers
    /// what it took.
    fn take(&mut self, range: Range<usize>) -> String {
        let taken = self.text[range.clone()].to_owned();
        if !range.is_empty() {
            self.note(Kind::Other, &range);
            self.replace(range.clone(), "");
            self.history.last = None;
        }
        self.move_to(range.start);
        taken
    }

    /// Takes out `times` whole lines from the cursor's, with a line break,
    /// and answers them, ending in one.
    fn delete_lines(&mut self, times: usize) -> String {
        let line = self.place_of(self.cursor()).line;
        let count = self.line_count();
        let last = (line + times - 1).min(count - 1);
        let lines = self.line_range(line).start..self.line_range(last).end;
        let taken = self.text[lines.clone()].to_owned() + "\n";
        let range = if last + 1 < count {
            lines.start..lines.end + 1
        } else if line > 0 {
            lines.start - 1..lines.end
        } else {
            lines
        };
        self.take(range);
        let line = line.min(self.line_count() - 1);
        self.move_to(self.line_range(line).start);
        taken
    }

    /// Puts `kept`, `times` over: whole lines, ending in a line break,
    /// under the cursor's or over it; else after the cursor's character
    /// or before it, with the cursor on the last put.
    fn put(&mut self, kept: &str, before: bool, times: usize) {
        let cursor = self.cursor();
        let place = self.place_of(cursor);
        let line = self.line_range(place.line);
        if let Some(lines) = kept.strip_suffix('\n') {
            let lines = vec![lines; times].join("\n");
            let (at, text, start) = if before {
                (line.start, format!("{lines}\n"), line.start)
            } else {
                (line.end, format!("\n{lines}"), line.end + 1)
            };
            self.insert(at, &text);
            self.move_to(start);
        } else {
            let text = kept.repeat(times);
            let at = if before {
                cursor
            } else {
                self.text[cursor..line.end]
                    .chars()
                    .next()
                    .map_or(cursor, |c| cursor + c.len_utf8())
            };
            self.insert(at, &text);
            let end = at + text.len();
            self.move_to(self.previous_boundary(end).max(at));
        }
    }

    fn insert(&mut self, at: usize, text: &str) {
        let range = at..at;
        self.note(Kind::Other, &range);
        self.replace(range, text);
        self.history.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A draft of `text`, the cursor on its byte `at`, in normal mode.
    fn at(text: &str, at: usize) -> Draft {
        let mut draft = Draft::new();
        draft.set_text(text);
        draft.move_to(at);
        draft
    }

    fn after(draft: &Draft) -> (&str, usize) {
        (draft.text(), draft.cursor())
    }

    #[test]
    fn escape_puts_the_cursor_on_the_character_before_it() {
        let mut draft = at("hello", 5);
        draft.leave_insert();
        assert_eq!(after(&draft), ("hello", 4));
        let mut draft = at("ab\ncd", 3);
        draft.leave_insert();
        assert_eq!(after(&draft), ("ab\ncd", 3), "not past its line's start");
    }

    #[test]
    fn motions_move_the_cursor_and_stay_on_a_character() {
        let mut draft = at("say hello world", 0);
        draft.edit_normal(Edit::Move(Across::WordForward), 2, None);
        assert_eq!(draft.cursor(), 10);
        draft.edit_normal(Edit::Move(Across::LineEnd), 1, None);
        assert_eq!(draft.cursor(), 14);
        draft.edit_normal(Edit::Move(Across::Right), 1, None);
        assert_eq!(draft.cursor(), 14, "the last character at most");
        draft.edit_normal(Edit::Move(Across::LineStart), 1, None);
        assert_eq!(draft.cursor(), 0);
    }

    #[test]
    fn x_and_d_take_out_what_a_motion_covers() {
        let mut draft = at("hello world", 0);
        let taken = draft.edit_normal(Edit::Delete(Span::To(Across::Right)), 2, None);
        assert_eq!(
            (after(&draft), taken.as_deref()),
            (("llo world", 0), Some("he"))
        );

        let mut draft = at("say hello world", 4);
        draft.edit_normal(Edit::Delete(Span::To(Across::WordForward)), 1, None);
        assert_eq!(after(&draft), ("say world", 4));

        let mut draft = at("say hello", 4);
        draft.edit_normal(Edit::Delete(Span::To(Across::WordForward)), 1, None);
        assert_eq!(
            after(&draft),
            ("say ", 3),
            "the last word, and the cursor on a character"
        );

        let mut draft = at("say hello world", 6);
        draft.edit_normal(Edit::Delete(Span::To(Across::LineEnd)), 1, None);
        assert_eq!(after(&draft), ("say he", 5));

        let mut draft = at("say hello", 6);
        draft.edit_normal(Edit::Delete(Span::To(Across::WordBack)), 1, None);
        assert_eq!(after(&draft), ("say llo", 4));

        let mut draft = at("say hello", 7);
        draft.edit_normal(Edit::Delete(Span::InnerWord), 1, None);
        assert_eq!(after(&draft), ("say ", 3));

        let mut draft = at("say hello world", 6);
        draft.edit_normal(Edit::Delete(Span::AWord), 1, None);
        assert_eq!(after(&draft), ("say world", 4));
    }

    #[test]
    fn dd_takes_out_whole_lines_and_p_puts_them_back_under() {
        let mut draft = at("one\ntwo\nthree", 5);
        let taken = draft.edit_normal(Edit::Delete(Span::Line), 1, None);
        assert_eq!(taken.as_deref(), Some("two\n"));
        assert_eq!(after(&draft), ("one\nthree", 4));
        draft.edit_normal(Edit::Put { before: false }, 1, taken.as_deref());
        assert_eq!(after(&draft), ("one\nthree\ntwo", 10));
        draft.edit_normal(Edit::Put { before: true }, 1, Some("zero\n"));
        assert_eq!(after(&draft), ("one\nthree\nzero\ntwo", 10));

        let mut draft = at("one\ntwo", 5);
        draft.edit_normal(Edit::Delete(Span::Line), 1, None);
        assert_eq!(
            after(&draft),
            ("one", 0),
            "the last line takes the break before it"
        );
        let mut draft = at("only", 2);
        draft.edit_normal(Edit::Delete(Span::Line), 1, None);
        assert_eq!(after(&draft), ("", 0));
    }

    #[test]
    fn c_takes_out_and_leaves_the_cursor_to_type_there() {
        let mut draft = at("say hello world", 4);
        let taken = draft.edit_normal(Edit::Change(Span::To(Across::WordForward)), 1, None);
        assert_eq!(taken.as_deref(), Some("hello"));
        assert_eq!(
            after(&draft),
            ("say  world", 4),
            "cw changes the word, not the space after"
        );

        let mut draft = at("say hello", 2);
        draft.edit_normal(Edit::Change(Span::Line), 1, None);
        assert_eq!(after(&draft), ("", 0));

        let mut draft = at("say hello", 9);
        draft.edit_normal(Edit::Change(Span::To(Across::LineEnd)), 1, None);
        assert_eq!(
            after(&draft),
            ("say hello", 9),
            "at the end, nothing to change"
        );
    }

    #[test]
    fn y_keeps_and_p_puts_after_the_character() {
        let mut draft = at("ab", 0);
        let kept = draft.edit_normal(Edit::Yank(Span::InnerWord), 1, None);
        assert_eq!(kept.as_deref(), Some("ab"));
        assert_eq!(draft.text(), "ab");
        draft.edit_normal(Edit::Put { before: false }, 2, kept.as_deref());
        assert_eq!(after(&draft), ("aababb", 4));
        draft.edit_normal(Edit::Move(Across::LineStart), 1, None);
        draft.edit_normal(Edit::Put { before: true }, 1, Some("x"));
        assert_eq!(after(&draft), ("xaababb", 0));

        let mut draft = at("one", 1);
        let kept = draft.edit_normal(Edit::Yank(Span::Line), 1, None);
        assert_eq!(kept.as_deref(), Some("one\n"));
        assert_eq!(draft.cursor(), 1, "yy leaves the cursor");
    }

    #[test]
    fn u_undoes_an_edit_and_ctrl_r_does_it_again() {
        let mut draft = at("say hello", 4);
        draft.edit_normal(Edit::Delete(Span::InnerWord), 1, None);
        draft.edit_normal(Edit::Undo, 1, None);
        assert_eq!(draft.text(), "say hello");
        draft.edit_normal(Edit::Redo, 1, None);
        assert_eq!(draft.text(), "say ");
    }
}
