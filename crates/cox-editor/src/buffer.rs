//! The text of a file as the editor holds it: a rope, which reads any of
//! its lines without going through the lines before, however long the
//! file.
//!
//! A line is the text between line breaks, without them: only `\n` breaks
//! a line, and a `\r` before it is left out too. A file that ends with a
//! line break has an empty line after it, where the next line would go.
//!
//! The text changes as a whole, as the file does, but the buffer works out
//! the one stretch of it that changed, an [`Edit`], so that what is worked
//! out from the text, such as its syntax, follows only that.

use std::borrow::Cow;
use std::ops::Range;

use ropey::Rope;

/// The text of a file.
#[derive(Clone, Debug, Default)]
pub struct Buffer {
    rope: Rope,
}

/// A place in the text: a line, counted from 0, and a byte of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub line: usize,
    pub column: usize,
}

impl Point {
    #[must_use]
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// A stretch of the text that changed, in bytes of the whole text and in
/// places: it went from `start` to `old_end`, and goes to `new_end` now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
    pub start_point: Point,
    pub old_end_point: Point,
    pub new_end_point: Point,
}

impl Buffer {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
        }
    }

    /// The whole text.
    #[must_use]
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// How long the whole text is, in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rope.len_bytes()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rope.len_bytes() == 0
    }

    /// Makes the text `text`, and says what stretch of it changed, if any.
    pub fn set_text(&mut self, text: &str) -> Option<Edit> {
        let old_len = self.len();
        let same_start = self
            .rope
            .bytes()
            .zip(text.bytes())
            .take_while(|(old, new)| old == new)
            .count();
        let mut start = same_start;
        if start == old_len && start == text.len() {
            return None;
        }
        // The same bytes, so a character starts there in both.
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        let most_end = (old_len - start).min(text.len() - start);
        let same_end = self
            .rope
            .bytes_at(old_len)
            .reversed()
            .zip(text.bytes().rev())
            .take(most_end)
            .take_while(|(old, new)| old == new)
            .count();
        let mut end = same_end;
        while !text.is_char_boundary(text.len() - end) {
            end -= 1;
        }
        let (old_end, new_end) = (old_len - end, text.len() - end);
        let start_point = self.point_of(start);
        let old_end_point = self.point_of(old_end);
        let removed = self.rope.byte_to_char(start)..self.rope.byte_to_char(old_end);
        self.rope.remove(removed.clone());
        self.rope.insert(removed.start, &text[start..new_end]);
        Some(Edit {
            start,
            old_end,
            new_end,
            start_point,
            old_end_point,
            new_end_point: self.point_of(new_end),
        })
    }

    /// The text from `byte` on, as much of it as the rope holds in one
    /// piece there, or nothing at the end of the text.
    #[must_use]
    pub fn chunk_at(&self, byte: usize) -> &str {
        if byte >= self.len() {
            return "";
        }
        let (chunk, start, _, _) = self.rope.chunk_at_byte(byte);
        &chunk[byte - start..]
    }

    /// The text of the bytes in `range`, in the pieces the rope holds it in.
    pub fn chunks(&self, range: Range<usize>) -> impl Iterator<Item = &str> {
        self.rope.byte_slice(range).chunks()
    }

    /// The byte of the whole text that `line` starts at, or the end of the
    /// text past the last line.
    #[must_use]
    pub fn line_start(&self, line: usize) -> usize {
        if line >= self.lines() {
            return self.len();
        }
        self.rope.line_to_byte(line)
    }

    /// The place of the byte `byte` of the whole text.
    fn point_of(&self, byte: usize) -> Point {
        let line = self.rope.byte_to_line(byte);
        Point::new(line, byte - self.rope.line_to_byte(line))
    }

    /// How many lines there are: one more than line breaks.
    #[must_use]
    pub fn lines(&self) -> usize {
        self.rope.len_lines()
    }

    /// The text of `line`, without its line break, or nothing past the
    /// last line.
    #[must_use]
    pub fn line(&self, line: usize) -> Cow<'_, str> {
        if line >= self.lines() {
            return Cow::Borrowed("");
        }
        let slice = self.rope.line(line);
        let text: Cow<'_, str> = slice.into();
        match text {
            Cow::Borrowed(text) => Cow::Borrowed(without_break(text)),
            Cow::Owned(mut text) => {
                let len = without_break(&text).len();
                text.truncate(len);
                Cow::Owned(text)
            }
        }
    }

    /// How many characters the longest line has.
    #[must_use]
    pub fn longest_line(&self) -> usize {
        self.rope
            .lines()
            .map(|line| {
                let mut len = line.len_chars();
                for line_break in ['\n', '\r'] {
                    if len > 0 && line.char(len - 1) == line_break {
                        len -= 1;
                    }
                }
                len
            })
            .max()
            .unwrap_or(0)
    }

    /// The place nearest `point` that is in the text: on the last line if
    /// past it, at the end of its line if past that, and at the start of
    /// the character it falls in.
    #[must_use]
    pub fn clip(&self, point: Point) -> Point {
        let last = self.lines() - 1;
        if point.line > last {
            return Point::new(last, self.line(last).len());
        }
        let line = self.line(point.line);
        let mut column = point.column.min(line.len());
        while !line.is_char_boundary(column) {
            column -= 1;
        }
        Point::new(point.line, column)
    }

    /// The text from one place to another, with the line breaks between.
    #[must_use]
    pub fn slice(&self, range: Range<Point>) -> String {
        let start = self.offset(range.start);
        let end = self.offset(range.end).max(start);
        self.rope.byte_slice(start..end).to_string()
    }

    /// The byte of the whole text at `point`, once clipped.
    fn offset(&self, point: Point) -> usize {
        let point = self.clip(point);
        self.rope.line_to_byte(point.line) + point.column
    }
}

/// `line` without the line break it ends with, if any.
fn without_break(line: &str) -> &str {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line)
}

/// Whether `c` is of a word: a letter, a digit, or `_`.
#[must_use]
pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The bytes of the word around `column` of `line`: letters, digits and
/// `_`, or else the one character there.
#[must_use]
pub fn word_around(line: &str, column: usize) -> Range<usize> {
    let Some(here) = line[column..].chars().next() else {
        return column..column;
    };
    if !is_word(here) {
        return column..column + here.len_utf8();
    }
    let start = line[..column]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(column, |(at, _)| at);
    let end = line[column..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(line.len(), |(at, _)| column + at);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lines_without_their_breaks() {
        let buffer = Buffer::new("fn main() {\r\n}\n");
        assert_eq!(buffer.lines(), 3);
        assert_eq!(buffer.line(0), "fn main() {");
        assert_eq!(buffer.line(1), "}");
        assert_eq!(buffer.line(2), "");
        assert_eq!(buffer.line(3), "");
        assert_eq!(buffer.longest_line(), "fn main() {".len());
    }

    #[test]
    fn an_empty_file_has_one_empty_line() {
        let buffer = Buffer::new("");
        assert_eq!(buffer.lines(), 1);
        assert_eq!(buffer.line(0), "");
    }

    #[test]
    fn reads_a_line_held_in_pieces() {
        // Long enough that the rope holds it in more than one piece.
        let line = "日本語のコメント ".repeat(400);
        let buffer = Buffer::new(&format!("first\n{line}\nlast"));
        assert_eq!(buffer.line(1), line);
        assert_eq!(buffer.line(2), "last");
    }

    #[test]
    fn clips_a_place_into_the_text() {
        let buffer = Buffer::new("ab\n日本\n");
        assert_eq!(buffer.clip(Point::new(0, 9)), Point::new(0, 2));
        // Inside 本, which starts at byte 3.
        assert_eq!(buffer.clip(Point::new(1, 4)), Point::new(1, 3));
        assert_eq!(buffer.clip(Point::new(7, 1)), Point::new(2, 0));
    }

    #[test]
    fn works_out_what_changed() {
        let mut buffer = Buffer::new("fn main() {\n    old();\n}\n");
        assert_eq!(buffer.set_text("fn main() {\n    old();\n}\n"), None);
        let edit = buffer.set_text("fn main() {\n    new();\n    old();\n}\n");
        assert_eq!(buffer.text(), "fn main() {\n    new();\n    old();\n}\n");
        // From `old` on the second line, which `new();\n    ` goes before.
        assert_eq!(
            edit,
            Some(Edit {
                start: 16,
                old_end: 16,
                new_end: 27,
                start_point: Point::new(1, 4),
                old_end_point: Point::new(1, 4),
                new_end_point: Point::new(2, 4),
            })
        );
        let edit = buffer.set_text("").unwrap();
        assert_eq!((edit.start, edit.old_end, edit.new_end), (0, 36, 0));
        assert_eq!(edit.old_end_point, Point::new(4, 0));
    }

    #[test]
    fn changes_whole_characters() {
        // 日, 本 and 文 all start with the same byte.
        let mut buffer = Buffer::new("日本");
        let edit = buffer.set_text("本本").unwrap();
        assert_eq!((edit.start, edit.old_end, edit.new_end), (0, 3, 3));
        let edit = buffer.set_text("本文本").unwrap();
        assert_eq!((edit.start, edit.old_end, edit.new_end), (3, 3, 6));
        assert_eq!(buffer.text(), "本文本");
    }

    #[test]
    fn reads_the_text_in_pieces() {
        let line = "日本語のコメント ".repeat(400);
        let buffer = Buffer::new(&format!("first\n{line}\n"));
        let mut text = String::new();
        let mut at = 0;
        while at < buffer.len() {
            let chunk = buffer.chunk_at(at);
            text.push_str(chunk);
            at += chunk.len();
        }
        assert_eq!(text, buffer.text());
        assert_eq!(
            buffer.chunks(6..buffer.len()).collect::<String>(),
            format!("{line}\n")
        );
        assert_eq!(buffer.line_start(1), 6);
        assert_eq!(buffer.line_start(9), buffer.len());
    }

    #[test]
    fn slices_across_lines() {
        let buffer = Buffer::new("one\ntwo\nthree\n");
        assert_eq!(
            buffer.slice(Point::new(0, 1)..Point::new(2, 2)),
            "ne\ntwo\nth"
        );
        assert_eq!(buffer.slice(Point::new(1, 0)..Point::new(1, 9)), "two");
        assert_eq!(buffer.slice(Point::new(2, 1)..Point::new(0, 0)), "");
    }

    #[test]
    fn finds_the_word_around_a_place() {
        let line = "let 日本_x = a.b;";
        let word = |column| &line[word_around(line, column)];
        assert_eq!(word(0), "let");
        assert_eq!(word("let ".len() + 3), "日本_x");
        assert_eq!(word(line.find('=').unwrap()), "=");
        assert_eq!(word(line.len()), "");
    }
}
