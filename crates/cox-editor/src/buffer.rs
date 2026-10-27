//! The text of a file as the editor holds it: a rope, which reads any of
//! its lines without going through the lines before, however long the
//! file.
//!
//! A line is the text between line breaks, without them: only `\n` breaks
//! a line, and a `\r` before it is left out too. A file that ends with a
//! line break has an empty line after it, where the next line would go.

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
    fn slices_across_lines() {
        let buffer = Buffer::new("one\ntwo\nthree\n");
        assert_eq!(
            buffer.slice(Point::new(0, 1)..Point::new(2, 2)),
            "ne\ntwo\nth"
        );
        assert_eq!(buffer.slice(Point::new(1, 0)..Point::new(1, 9)), "two");
        assert_eq!(buffer.slice(Point::new(2, 1)..Point::new(0, 0)), "");
    }
}
