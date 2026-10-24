//! Text the human selects in the conversation, which can run across many
//! pieces of text: from a spot in one to a spot in another. Plain Rust,
//! without GPUI.

use std::ops::Range;

/// One piece of text in the conversation: the `part`th piece of the
/// entry at `entry`. Pieces come in the order they show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextId {
    pub entry: usize,
    pub part: usize,
}

/// A place in the conversation's text: before the byte at `offset` of
/// piece `text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub text: TextId,
    pub offset: usize,
}

/// The text between where the human started selecting and where they
/// are now, whichever comes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Spot,
    pub head: Spot,
}

impl Selection {
    #[must_use]
    pub fn at(spot: Spot) -> Self {
        Self {
            anchor: spot,
            head: spot,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    fn ends(&self) -> (Spot, Spot) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The bytes selected of piece `id`, which holds `text`, if any. A spot
    /// that the text no longer has, as when a reply changes while it
    /// arrives, is taken back to where the text has a character.
    #[must_use]
    pub fn range_in(&self, id: TextId, text: &str) -> Option<Range<usize>> {
        let (start, end) = self.ends();
        if id < start.text || id > end.text {
            return None;
        }
        let from = if id == start.text { start.offset } else { 0 };
        let to = if id == end.text {
            end.offset
        } else {
            text.len()
        };
        let range = char_boundary(text, from)..char_boundary(text, to);
        (!range.is_empty()).then_some(range)
    }

    /// The text selected of `pieces`, in order, each joined to the one
    /// before it by what it says.
    #[must_use]
    pub fn copy<'a>(&self, pieces: impl IntoIterator<Item = Piece<'a>>) -> String {
        let mut copied = String::new();
        let mut first = true;
        for piece in pieces {
            let Some(range) = self.range_in(piece.id, piece.text) else {
                continue;
            };
            if !first {
                copied.push_str(piece.joined);
            }
            first = false;
            copied.push_str(&piece.text[range]);
        }
        copied
    }
}

/// The start of the character `offset` is in, or the end of `text`.
fn char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// A piece of text in the conversation, as copying sees it.
#[derive(Clone, Copy, Debug)]
pub struct Piece<'a> {
    pub id: TextId,
    pub text: &'a str,
    /// What comes between this piece and the one before it when both are
    /// copied: a line break, a blank line between paragraphs, a tab
    /// between a table's cells.
    pub joined: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(entry: usize, part: usize, offset: usize) -> Spot {
        Spot {
            text: TextId { entry, part },
            offset,
        }
    }

    fn piece<'a>(entry: usize, part: usize, text: &'a str, joined: &'static str) -> Piece<'a> {
        Piece {
            id: TextId { entry, part },
            text,
            joined,
        }
    }

    #[test]
    fn selects_within_one_piece_either_way() {
        let forward = Selection {
            anchor: spot(0, 0, 1),
            head: spot(0, 0, 3),
        };
        let backward = Selection {
            anchor: spot(0, 0, 3),
            head: spot(0, 0, 1),
        };

        for selection in [forward, backward] {
            assert_eq!(
                selection.range_in(TextId { entry: 0, part: 0 }, "hello"),
                Some(1..3)
            );
            assert_eq!(
                selection.range_in(TextId { entry: 0, part: 1 }, "hello"),
                None
            );
        }
    }

    #[test]
    fn copies_across_pieces_with_what_joins_them() {
        let pieces = [
            piece(0, 0, "hello", ""),
            piece(1, 0, "first", "\n\n"),
            piece(1, 1, "a\tb", "\n\n"),
            piece(1, 2, "c", "\t"),
            piece(2, 0, "after", "\n\n"),
        ];
        let selection = Selection {
            anchor: spot(1, 2, 1),
            head: spot(0, 0, 3),
        };

        assert_eq!(selection.copy(pieces), "lo\n\nfirst\n\na\tb\tc");
    }

    #[test]
    fn a_spot_the_text_no_longer_has_moves_back_to_a_character() {
        let selection = Selection {
            anchor: spot(0, 0, 1),
            head: spot(0, 0, 9),
        };

        assert_eq!(
            selection.range_in(TextId { entry: 0, part: 0 }, "aあ"),
            Some(1..4)
        );
        assert_eq!(
            selection.range_in(TextId { entry: 0, part: 0 }, "aあb"),
            Some(1..5)
        );
        let inside = Selection {
            anchor: spot(0, 0, 0),
            head: spot(0, 0, 2),
        };
        assert_eq!(
            inside.range_in(TextId { entry: 0, part: 0 }, "aあ"),
            Some(0..1)
        );
    }

    #[test]
    fn copies_nothing_from_an_empty_selection() {
        let selection = Selection::at(spot(0, 0, 2));

        assert!(selection.is_empty());
        assert_eq!(selection.copy([piece(0, 0, "hello", "")]), "");
    }
}
