//! The cursor Vim's keys move through text the human reads, as the
//! conversation's: a character, a word or a row at a time, from piece to
//! piece, over the rows the last frame laid the pieces out in. Plain
//! Rust, without GPUI.

use std::ops::Range;

use cox_editor::buffer::is_word;

use crate::ui::selection::{Selection, Spot, TextId};
use crate::vim::keymap::Selects;

/// Where the cursor is, and the x it keeps to going up and down, as Vim's
/// `curswant`: none until it goes up or down after moving along a row,
/// and past every row's end after `$`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    pub spot: Spot,
    pub want: Option<f32>,
}

impl Cursor {
    #[must_use]
    pub const fn at(spot: Spot) -> Self {
        Self { spot, want: None }
    }
}

/// Where the cursor goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A character left, within the line (`h`).
    Left,
    /// A character right, within the line (`l`).
    Right,
    /// To the next word's start (`w`).
    WordForward,
    /// To the start of the word, or of the one before (`b`).
    WordBack,
    /// To the end of the word, or of the next (`e`).
    WordEnd,
    /// To the start of the row (`0`).
    RowStart,
    /// To the row's first character that is no space (`^`).
    FirstNonBlank,
    /// To the row's last character (`$`).
    RowEnd,
    /// The row below, at the x the cursor keeps to (`j`).
    Down,
    /// The row above (`k`).
    Up,
}

/// A row of a piece of text as laid out: where it was drawn, from top to
/// bottom and to the x it ends at, and each place on it the cursor can
/// be, a character's start, with the x the character starts at. An empty
/// line has one place, its end.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub top: f32,
    pub bottom: f32,
    pub end: f32,
    pub places: Vec<(usize, f32)>,
}

/// Where a row of the pieces laid out is.
#[derive(Clone, Debug)]
struct RowAt {
    /// Its places, among every place.
    places: Range<usize>,
    top: f32,
    bottom: f32,
    end: f32,
}

impl RowAt {
    fn in_sight(&self, top: f32, bottom: f32) -> bool {
        self.top >= top && self.bottom <= bottom
    }
}

/// Where the cursor is drawn: a block over the character it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub x: f32,
    pub top: f32,
    pub width: f32,
    pub bottom: f32,
}

/// A piece of text as laid out, row by row.
#[derive(Clone, Debug)]
pub struct Piece<'a> {
    pub id: TextId,
    pub text: &'a str,
    pub rows: Vec<Row>,
}

/// What kind of character a place is on, for Vim's words: a word is a
/// run of one kind, other than a space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Space,
    Word,
    Mark,
}

/// A place the cursor can be.
#[derive(Clone, Copy, Debug)]
struct Place {
    spot: Spot,
    x: f32,
    kind: Kind,
    /// The row it is on, among every row.
    row: usize,
    /// Whether a line or a piece starts at it, which a word does not run
    /// across, nor `h` and `l`.
    starts_line: bool,
    /// Whether it is an empty line, where `w` and `b` stop.
    empty: bool,
}

/// The rows of every piece laid out, in order, and every place on them
/// the cursor can be.
#[derive(Debug, Default)]
pub struct Laid {
    places: Vec<Place>,
    rows: Vec<RowAt>,
}

impl Laid {
    /// The rows of `pieces`, given in order.
    #[must_use]
    pub fn new<'a>(pieces: impl IntoIterator<Item = Piece<'a>>) -> Self {
        let mut laid = Self::default();
        for piece in pieces {
            let mut first = true;
            for row in piece.rows {
                let start = laid.places.len();
                for (offset, x) in row.places {
                    let here = piece
                        .text
                        .get(offset..)
                        .and_then(|rest| rest.chars().next());
                    let line_start = offset == 0 || piece.text[..offset].ends_with('\n');
                    let empty = matches!(here, None | Some('\n'));
                    laid.places.push(Place {
                        spot: Spot {
                            text: piece.id,
                            offset,
                        },
                        x,
                        kind: match here {
                            Some(c) if !c.is_whitespace() && is_word(c) => Kind::Word,
                            Some(c) if !c.is_whitespace() => Kind::Mark,
                            _ => Kind::Space,
                        },
                        row: laid.rows.len(),
                        starts_line: first || line_start,
                        empty,
                    });
                    first = false;
                }
                if laid.places.len() > start {
                    laid.rows.push(RowAt {
                        places: start..laid.places.len(),
                        top: row.top,
                        bottom: row.bottom,
                        end: row.end,
                    });
                }
            }
        }
        laid
    }

    /// The place `spot` is at, or the last before it, or the first.
    fn index(&self, spot: Spot) -> Option<usize> {
        if self.places.is_empty() {
            return None;
        }
        Some(
            self.places
                .partition_point(|place| place.spot <= spot)
                .saturating_sub(1),
        )
    }

    /// Whether `spot` is among the places laid out, as the start of its
    /// character.
    #[must_use]
    pub fn has(&self, spot: Spot) -> bool {
        self.index(spot)
            .is_some_and(|index| self.places[index].spot == spot)
    }

    /// The cursor `step` moves `cursor` to, `times` over, if any place is
    /// laid out.
    #[must_use]
    pub fn moved(&self, cursor: Cursor, step: Step, times: usize) -> Option<Cursor> {
        let mut at = self.index(cursor.spot)?;
        let row = self.places[at].row;
        let want = cursor.want.unwrap_or(self.places[at].x);
        let last_row = self.rows.len() - 1;
        match step {
            Step::Down | Step::Up => {
                let to = if step == Step::Down {
                    row.saturating_add(times).min(last_row)
                } else {
                    row.saturating_sub(times)
                };
                return Some(Cursor {
                    spot: self.places[self.at_x(to, want)].spot,
                    want: Some(want),
                });
            }
            Step::RowEnd => {
                let to = row.saturating_add(times - 1).min(last_row);
                return Some(Cursor {
                    spot: self.places[self.rows[to].places.end - 1].spot,
                    want: Some(f32::INFINITY),
                });
            }
            Step::RowStart => at = self.rows[row].places.start,
            Step::FirstNonBlank => {
                let places = self.rows[row].places.clone();
                at = places
                    .clone()
                    .find(|&index| self.places[index].kind != Kind::Space)
                    .unwrap_or(places.end - 1);
            }
            _ => {
                for _ in 0..times {
                    at = match step {
                        Step::Left if !self.places[at].starts_line => at - 1,
                        Step::Right
                            if at + 1 < self.places.len() && !self.places[at + 1].starts_line =>
                        {
                            at + 1
                        }
                        Step::WordForward => self.word_forward(at),
                        Step::WordBack => self.word_back(at),
                        Step::WordEnd => self.word_end(at),
                        _ => at,
                    };
                }
            }
        }
        Some(Cursor::at(self.places[at].spot))
    }

    /// The place on row `row` nearest `x`: the last that starts at or
    /// before it, or the row's first.
    fn at_x(&self, row: usize, x: f32) -> usize {
        let places = self.rows[row].places.clone();
        places
            .clone()
            .rfind(|&index| self.places[index].x <= x + 0.5)
            .unwrap_or(places.start)
    }

    /// Whether a word runs on from `at` to the place after it.
    fn joined(&self, at: usize) -> bool {
        let next = &self.places[at + 1];
        !next.starts_line && next.kind == self.places[at].kind
    }

    /// The start of the next word after `at` (`w`), or the last place.
    fn word_forward(&self, at: usize) -> usize {
        let last = self.places.len() - 1;
        let mut index = at;
        if self.places[at].kind != Kind::Space {
            while index < last && self.joined(index) {
                index += 1;
            }
            if index == last {
                return last;
            }
            index += 1;
        }
        while index < last
            && self.places[index].kind == Kind::Space
            && !(index != at && self.places[index].empty)
        {
            index += 1;
        }
        index
    }

    /// The end of the word at or after the one after `at` (`e`).
    fn word_end(&self, at: usize) -> usize {
        let last = self.places.len() - 1;
        if at == last {
            return at;
        }
        let mut index = at + 1;
        while index < last && self.places[index].kind == Kind::Space {
            index += 1;
        }
        while index < last && self.joined(index) {
            index += 1;
        }
        index
    }

    /// The start of the word `at` is in after its start, or of the one
    /// before (`b`).
    fn word_back(&self, at: usize) -> usize {
        if at == 0 {
            return 0;
        }
        let mut index = at - 1;
        while index > 0 && self.places[index].kind == Kind::Space && !self.places[index].empty {
            index -= 1;
        }
        while index > 0 && self.joined(index - 1) {
            index -= 1;
        }
        index
    }

    /// The top and bottom of the row `spot` is on, if it is laid out.
    #[must_use]
    pub fn row_of(&self, spot: Spot) -> Option<(f32, f32)> {
        let index = self.index(spot)?;
        let row = &self.rows[self.places[index].row];
        Some((row.top, row.bottom))
    }

    /// Where the cursor at `spot` is drawn: as wide as its character, or,
    /// where there is none, at the end of a row, `none` wide.
    #[must_use]
    pub fn block(&self, spot: Spot, none: f32) -> Option<Block> {
        let index = self.index(spot)?;
        let place = &self.places[index];
        let row = &self.rows[place.row];
        let next = if index + 1 < row.places.end {
            self.places[index + 1].x
        } else {
            row.end
        };
        let width = if place.empty || next <= place.x {
            none
        } else {
            next - place.x
        };
        Some(Block {
            x: place.x,
            top: row.top,
            width,
            bottom: row.bottom,
        })
    }

    /// Where `cursor` goes to be in sight, from `top` to `bottom`: where it
    /// is, if it is laid out there, or else on the nearest row wholly in
    /// sight, at the x it keeps to.
    #[must_use]
    pub fn into_sight(&self, cursor: Cursor, top: f32, bottom: f32) -> Option<Cursor> {
        if self.has(cursor.spot)
            && self
                .row_of(cursor.spot)
                .is_some_and(|(row_top, row_bottom)| row_top >= top && row_bottom <= bottom)
        {
            return Some(cursor);
        }
        let shown: Vec<usize> = (0..self.rows.len())
            .filter(|&row| self.rows[row].in_sight(top, bottom))
            .collect();
        let (first, last) = (*shown.first()?, *shown.last()?);
        let below = self
            .row_of(cursor.spot)
            .is_some_and(|(_, row_bottom)| row_bottom > bottom);
        let row = if below { last } else { first };
        let want = cursor.want.unwrap_or_else(|| {
            self.index(cursor.spot)
                .map_or(0., |index| self.places[index].x)
        });
        Some(Cursor {
            spot: self.places[self.at_x(row, want)].spot,
            want: cursor.want,
        })
    }

    /// The start of the first row wholly in sight, from `top` to
    /// `bottom`, or the last, as `last` says.
    #[must_use]
    pub fn end_in_sight(&self, top: f32, bottom: f32, last: bool) -> Option<Spot> {
        let mut shown = self.rows.iter().filter(|row| row.in_sight(top, bottom));
        let row = if last {
            shown.next_back()
        } else {
            shown.next()
        }?;
        Some(self.places[row.places.start].spot)
    }
}

/// What visual mode selects from `anchor` to `head`: the characters
/// between, both in, or the whole pieces of text between, as `selects`
/// says, the text of each piece as `text_of` tells it.
#[must_use]
pub fn visual<'a>(
    anchor: Spot,
    head: Spot,
    selects: Selects,
    text_of: impl Fn(TextId) -> Option<&'a str>,
) -> Selection {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    let end_of_piece = |spot: Spot| Spot {
        offset: text_of(spot.text).map_or(usize::MAX, str::len),
        ..spot
    };
    let (from, to) = match selects {
        Selects::Characters => {
            let width = text_of(end.text)
                .and_then(|text| text.get(end.offset..))
                .and_then(|rest| rest.chars().next())
                .map_or(1, char::len_utf8);
            (
                start,
                Spot {
                    offset: end.offset + width,
                    ..end
                },
            )
        }
        Selects::Lines => (Spot { offset: 0, ..start }, end_of_piece(end)),
    };
    // The anchor stays where visual mode started, so that `o` could
    // swap the ends as Vim's does.
    if anchor <= head {
        Selection {
            anchor: from,
            head: to,
        }
    } else {
        Selection {
            anchor: to,
            head: from,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: f32 = 20.;
    const WIDTH: f32 = 10.;

    /// A piece laid out in rows of at most `columns` characters, each
    /// `WIDTH` wide, from `top`, breaking at its lines too.
    fn piece(entry: usize, part: usize, text: &str, columns: usize, top: f32) -> Piece<'_> {
        let mut rows = Vec::new();
        let mut offset = 0;
        for line in text.split('\n') {
            let chars: Vec<(usize, char)> = line.char_indices().collect();
            if chars.is_empty() {
                rows.push(vec![(offset, 0.)]);
            }
            for chunk in chars.chunks(columns) {
                #[expect(clippy::cast_precision_loss, reason = "a test's few columns")]
                let places = chunk
                    .iter()
                    .enumerate()
                    .map(|(column, (at, _))| (offset + at, column as f32 * WIDTH))
                    .collect();
                rows.push(places);
            }
            offset += line.len() + 1;
        }
        #[expect(clippy::cast_precision_loss, reason = "a test's few rows")]
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(row, places): (usize, Vec<(usize, f32)>)| Row {
                top: top + row as f32 * LINE,
                bottom: top + (row + 1) as f32 * LINE,
                end: places.len() as f32 * WIDTH,
                places,
            })
            .collect();
        Piece {
            id: TextId { entry, part },
            text,
            rows,
        }
    }

    fn spot(entry: usize, part: usize, offset: usize) -> Spot {
        Spot {
            text: TextId { entry, part },
            offset,
        }
    }

    /// A reply of two pieces, the first wrapped in rows of ten: "the
    /// quick " / "brown fox" / "jumps" and a code block of two lines.
    fn laid() -> Laid {
        Laid::new([
            piece(0, 0, "the quick brown fox\njumps", 10, 0.),
            piece(0, 1, "let a = 1;\n\nb", 40, 60.),
        ])
    }

    fn walk(laid: &Laid, from: Spot, steps: &[(Step, usize)]) -> Vec<Spot> {
        let mut cursor = Cursor::at(from);
        steps
            .iter()
            .map(|&(step, times)| {
                cursor = laid.moved(cursor, step, times).expect("text is laid out");
                cursor.spot
            })
            .collect()
    }

    #[test]
    fn h_and_l_go_a_character_along_the_line_and_stop_at_its_ends() {
        let laid = laid();
        assert_eq!(
            walk(
                &laid,
                spot(0, 0, 0),
                &[(Step::Left, 1), (Step::Right, 3), (Step::Right, 9)]
            ),
            [spot(0, 0, 0), spot(0, 0, 3), spot(0, 0, 12)],
            "l goes on from a wrapped row to the next"
        );
        assert_eq!(
            walk(&laid, spot(0, 0, 18), &[(Step::Right, 1), (Step::Left, 20)]),
            [spot(0, 0, 18), spot(0, 0, 0)],
            "l stops at the line's end"
        );
        assert_eq!(
            walk(&laid, spot(0, 0, 20), &[(Step::Left, 1)]),
            [spot(0, 0, 20)],
            "h stops at the line's start"
        );
    }

    #[test]
    fn w_b_and_e_go_by_words_from_piece_to_piece() {
        let laid = laid();
        assert_eq!(
            walk(
                &laid,
                spot(0, 0, 0),
                &[
                    (Step::WordForward, 1),
                    (Step::WordForward, 3),
                    (Step::WordForward, 1),
                    (Step::WordForward, 2),
                ]
            ),
            [spot(0, 0, 4), spot(0, 0, 20), spot(0, 1, 0), spot(0, 1, 6)],
            "w goes on into the next line and the next piece"
        );
        assert_eq!(
            walk(&laid, spot(0, 1, 9), &[(Step::WordForward, 1)]),
            [spot(0, 1, 11)],
            "w stops at an empty line"
        );
        assert_eq!(
            walk(
                &laid,
                spot(0, 1, 4),
                &[
                    (Step::WordBack, 1),
                    (Step::WordBack, 1),
                    (Step::WordBack, 2)
                ]
            ),
            [spot(0, 1, 0), spot(0, 0, 20), spot(0, 0, 10)],
        );
        assert_eq!(
            walk(
                &laid,
                spot(0, 0, 0),
                &[(Step::WordEnd, 1), (Step::WordEnd, 1), (Step::WordEnd, 4)]
            ),
            [spot(0, 0, 2), spot(0, 0, 8), spot(0, 1, 2)],
        );
        assert_eq!(
            walk(&laid, spot(0, 1, 12), &[(Step::WordForward, 1)]),
            [spot(0, 1, 12)],
            "w stays at the last place"
        );
    }

    #[test]
    fn zero_caret_and_dollar_go_along_the_row_the_cursor_is_on() {
        let indented = Laid::new([piece(0, 0, "  indented line", 40, 0.)]);
        assert_eq!(
            walk(
                &indented,
                spot(0, 0, 9),
                &[
                    (Step::RowStart, 1),
                    (Step::FirstNonBlank, 1),
                    (Step::RowEnd, 1)
                ]
            ),
            [spot(0, 0, 0), spot(0, 0, 2), spot(0, 0, 14)],
        );
        let laid = laid();
        assert_eq!(
            walk(
                &laid,
                spot(0, 0, 12),
                &[(Step::RowStart, 1), (Step::RowEnd, 1)]
            ),
            [spot(0, 0, 10), spot(0, 0, 18)],
            "the row, not the line, of wrapped text"
        );
    }

    #[test]
    fn j_and_k_go_by_rows_at_the_x_the_cursor_keeps_to() {
        let laid = laid();
        assert_eq!(
            walk(
                &laid,
                spot(0, 0, 7),
                &[
                    (Step::Down, 1),
                    (Step::Down, 1),
                    (Step::Down, 1),
                    (Step::Up, 2)
                ]
            ),
            [
                spot(0, 0, 17),
                spot(0, 0, 24),
                spot(0, 1, 7),
                spot(0, 0, 17)
            ],
            "a shorter row holds the cursor at its end, and the next goes back to the x"
        );
        assert_eq!(
            walk(&laid, spot(0, 0, 0), &[(Step::RowEnd, 1), (Step::Down, 3)]),
            [spot(0, 0, 9), spot(0, 1, 9)],
            "after $ each row's end"
        );
        assert_eq!(
            walk(&laid, spot(0, 1, 12), &[(Step::Down, 5), (Step::Up, 99)]),
            [spot(0, 1, 12), spot(0, 0, 0)],
            "counts stop at the ends"
        );
    }

    #[test]
    fn a_cursor_out_of_sight_goes_to_the_nearest_row_in_sight() {
        let laid = laid();
        let cursor = Cursor::at(spot(0, 0, 3));
        assert_eq!(laid.into_sight(cursor, 0., 100.), Some(cursor));
        assert_eq!(
            laid.into_sight(cursor, 20., 100.).map(|cursor| cursor.spot),
            Some(spot(0, 0, 13)),
            "scrolled down past it: the first row in sight"
        );
        let below = Cursor::at(spot(0, 1, 12));
        assert_eq!(
            laid.into_sight(below, 0., 60.).map(|cursor| cursor.spot),
            Some(spot(0, 0, 20)),
            "scrolled up past it: the last row in sight"
        );
        let gone = Cursor::at(spot(7, 0, 0));
        assert_eq!(
            laid.into_sight(gone, 0., 40.).map(|cursor| cursor.spot),
            Some(spot(0, 0, 10)),
            "after all that is laid out: the last row in sight"
        );
        assert_eq!(laid.end_in_sight(20., 100., false), Some(spot(0, 0, 10)));
        assert_eq!(laid.end_in_sight(20., 100., true), Some(spot(0, 1, 11)));
    }

    #[test]
    fn the_cursor_is_drawn_over_its_character() {
        let laid = laid();
        assert_eq!(
            laid.block(spot(0, 0, 4), 5.),
            Some(Block {
                x: 40.,
                top: 0.,
                width: WIDTH,
                bottom: LINE
            })
        );
        assert_eq!(
            laid.block(spot(0, 1, 11), 5.)
                .map(|block| (block.top, block.width)),
            Some((80., 5.)),
            "an empty line"
        );
    }

    #[test]
    fn visual_mode_selects_characters_both_ends_in_or_whole_pieces() {
        let text = |id: TextId| match id.part {
            0 => Some("héllo"),
            _ => Some("world"),
        };
        let selection = visual(spot(0, 1, 2), spot(0, 0, 1), Selects::Characters, text);
        assert_eq!(
            selection.range_in(TextId { entry: 0, part: 0 }, "héllo"),
            Some(1..6)
        );
        assert_eq!(
            selection.range_in(TextId { entry: 0, part: 1 }, "world"),
            Some(0..3)
        );
        let one = visual(spot(0, 0, 1), spot(0, 0, 1), Selects::Characters, text);
        assert_eq!(
            one.range_in(TextId { entry: 0, part: 0 }, "héllo"),
            Some(1..3),
            "the character under the cursor, whole"
        );
        let pieces = visual(spot(0, 0, 3), spot(0, 1, 1), Selects::Lines, text);
        assert_eq!(
            pieces.range_in(TextId { entry: 0, part: 0 }, "héllo"),
            Some(0..6)
        );
        assert_eq!(
            pieces.range_in(TextId { entry: 0, part: 1 }, "world"),
            Some(0..5)
        );
    }
}
