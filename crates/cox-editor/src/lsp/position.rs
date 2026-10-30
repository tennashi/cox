//! A place in a file as the server counts it: a line, and how far into
//! it in what the server counts a line in, which the editor turns into
//! and from its own places in the file's text.

use crate::buffer::{Buffer, Point};

/// A place in a file as the server counts it: a line, from 0, and how far
/// into it, in the server's [`Encoding`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// What the server counts a line in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    /// Bytes, as the editor does.
    Utf8,
    /// UTF-16 code units, which every server knows.
    #[default]
    Utf16,
    /// Characters.
    Utf32,
}

impl Encoding {
    /// The encoding the server names, UTF-16 unless it says otherwise.
    pub(super) fn named(name: &str) -> Self {
        match name {
            "utf-8" => Self::Utf8,
            "utf-32" => Self::Utf32,
            _ => Self::Utf16,
        }
    }

    /// `point` of `buffer`, as the server counts it.
    #[must_use]
    pub fn position(self, buffer: &Buffer, point: Point) -> Position {
        let point = buffer.clip(point);
        let before = &buffer.line(point.line)[..point.column];
        let character = match self {
            Self::Utf8 => before.len(),
            Self::Utf16 => before.encode_utf16().count(),
            Self::Utf32 => before.chars().count(),
        };
        Position {
            line: count(point.line),
            character: count(character),
        }
    }

    /// The place in `buffer` that `position` is at, as the server counts
    /// it: at the end of its line if past it.
    #[must_use]
    pub fn point(self, buffer: &Buffer, position: Position) -> Point {
        let line = position.line as usize;
        let text = buffer.line(line);
        let character = position.character as usize;
        let column = match self {
            Self::Utf8 => character,
            Self::Utf16 | Self::Utf32 => {
                let mut counted = 0;
                text.char_indices()
                    .find(|&(_, c)| {
                        let past = counted >= character;
                        counted += if self == Self::Utf16 {
                            c.len_utf16()
                        } else {
                            1
                        };
                        past
                    })
                    .map_or(text.len(), |(at, _)| at)
            }
        };
        buffer.clip(Point::new(line, column))
    }
}

/// `n` as the protocol's numbers, which no line goes past.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{Encoding, Position};
    use crate::buffer::{Buffer, Point};

    #[test]
    fn counts_a_line_as_the_server_does() {
        let buffer = Buffer::new("let 日本 = \"😀\";\n");
        let after_emoji = Point::new(0, "let 日本 = \"😀".len());
        for (encoding, character) in [
            (Encoding::Utf8, 18),
            (Encoding::Utf16, 12),
            (Encoding::Utf32, 11),
        ] {
            let position = encoding.position(&buffer, after_emoji);
            assert_eq!(position, Position { line: 0, character });
            assert_eq!(encoding.point(&buffer, position), after_emoji);
        }
        // Past the end of the line, and within a character.
        let end = Point::new(0, buffer.line(0).len());
        let past = Position {
            line: 0,
            character: 99,
        };
        assert_eq!(Encoding::Utf16.point(&buffer, past), end);
        assert_eq!(
            Encoding::Utf8.point(
                &buffer,
                Position {
                    line: 0,
                    character: 5
                }
            ),
            Point::new(0, 4)
        );
    }
}
