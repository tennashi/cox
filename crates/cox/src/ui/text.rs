//! Text drawn so that the human can select and copy it and click its
//! links, as in the conversation: replies as Markdown, the rest as it
//! came.
//!
//! Each piece of text is a [`SelectableText`]. When it is laid out, it
//! notes in [`Painted`] what it says and where it went, which is how the
//! view finds the text under the mouse and what a selection copies. Every
//! frame draws the pieces again, so `Painted` holds those of the last
//! frame, or, for a view that lays out only the text in sight, as the
//! conversation does, those of the last frame with what the frames before
//! laid out of the rest, so that a selection copies what scrolled out of
//! sight too. [`Reading`] turns the human's mouse into what they select
//! of it, and the links they click.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId, FontStyle,
    FontWeight, GlobalElementId, HighlightStyle, Hitbox, HitboxBehavior, Hsla, ImageSource,
    InspectorElementId, IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, ObjectFit, Pixels,
    Point, SharedString, StrikethroughStyle, StyledText, TextLayout, UnderlineStyle, Window, div,
    img, prelude::*, px, rgb,
};
use unicode_segmentation::UnicodeSegmentation;

pub mod cursor;

use crate::ui::diagram;
use crate::ui::markdown::{self, Block, Kind};
use crate::ui::selection::{Piece, Selection, Spot, TextId};
use crate::ui::theme::{
    ACCENT, BORDER, BORDER_STRONG, BUTTON, BUTTON_HOVER, CODE, CODE_FONT, DIM, LINK, RAISED,
    SELECTION,
};
use crate::vim::hints::Hinted as _;
use crate::vim::keymap::Selects;
use crate::vim::search::{self, Direction};

use self::cursor::{Cursor, Laid, Step};

/// How far each list a block is in moves it right.
const INDENT: f32 = 20.;

/// A piece of text as the last frame laid it out.
struct PaintedText {
    id: TextId,
    text: SharedString,
    joined: &'static str,
    layout: TextLayout,
    /// The part of the text that shows: a piece in a box that scrolls on
    /// its own can be partly hidden, and one an earlier frame laid out,
    /// out of sight now, shows none.
    visible: Bounds<Pixels>,
    links: Rc<[(Range<usize>, SharedString)]>,
    /// Whether this frame laid the piece out, or an earlier one did.
    current: bool,
}

impl PaintedText {
    /// Whether the piece shows at `position`.
    fn shows_at(&self, position: Point<Pixels>) -> bool {
        !self.visible.is_empty() && self.visible.contains(&position)
    }
}

/// The pieces of text the last frame laid out, in order, with, once
/// [aged](Painted::age), those earlier frames laid out that it did not,
/// all in the order of their ids.
#[derive(Clone, Default)]
pub struct Painted(Rc<RefCell<Vec<PaintedText>>>, Rc<Cell<bool>>);

impl Painted {
    /// Forgets the pieces, before a frame lays them out again.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
        self.1.set(false);
    }

    /// Keeps the pieces of the entries before `entries`, out of sight,
    /// before a frame lays out again those it shows: an entry the frame
    /// lays out replaces what it was.
    pub fn age(&self, entries: usize) {
        let mut texts = self.0.borrow_mut();
        texts.retain(|text| text.id.entry < entries);
        texts.sort_by_key(|text| text.id);
        for text in texts.iter_mut() {
            text.visible = Bounds::default();
            text.current = false;
        }
        self.1.set(true);
    }

    /// Notes `text`, which this frame laid out, in its place among the
    /// pieces.
    fn push(&self, text: PaintedText) {
        let mut texts = self.0.borrow_mut();
        if !self.1.get() {
            texts.push(text);
            return;
        }
        // What an earlier frame laid out of the entry goes, once this one
        // lays it out.
        let entry = text.id.entry;
        let start = texts.partition_point(|kept| kept.id.entry < entry);
        let end = texts.partition_point(|kept| kept.id.entry <= entry);
        let mut index = start;
        for _ in start..end {
            if texts[index].current {
                index += 1;
            } else {
                texts.remove(index);
            }
        }
        let at = texts.partition_point(|kept| kept.id < text.id);
        texts.insert(at, text);
    }

    /// The spot nearest `position` in the text that shows: in the piece
    /// beside it, or at the start of the next piece down when it is
    /// between pieces.
    #[must_use]
    pub fn spot_at(&self, position: Point<Pixels>) -> Option<Spot> {
        let texts = self.0.borrow();
        let shown = || texts.iter().filter(|text| !text.visible.is_empty());
        let beside = shown()
            .filter(|text| text.visible.top() <= position.y && position.y <= text.visible.bottom())
            .min_by(|a, b| {
                horizontal_distance(a.visible, position)
                    .total_cmp(&horizontal_distance(b.visible, position))
            });
        if let Some(text) = beside {
            return Some(Spot {
                text: text.id,
                offset: closest_index(&text.layout, &text.text, position),
            });
        }
        match shown().find(|text| text.visible.top() > position.y) {
            Some(below) => Some(Spot {
                text: below.id,
                offset: 0,
            }),
            None => shown().next_back().map(|last| Spot {
                text: last.id,
                offset: last.text.len(),
            }),
        }
    }

    /// The word, or the run of spaces or punctuation, at `spot`.
    #[must_use]
    pub fn word_at(&self, spot: Spot) -> Selection {
        let texts = self.0.borrow();
        let Some(text) = texts.iter().find(|text| text.id == spot.text) else {
            return Selection::at(spot);
        };
        let word = text
            .text
            .split_word_bound_indices()
            .map(|(start, word)| start..start + word.len())
            .find(|word| word.contains(&spot.offset))
            .unwrap_or(spot.offset..spot.offset);
        Selection {
            anchor: Spot {
                offset: word.start,
                ..spot
            },
            head: Spot {
                offset: word.end,
                ..spot
            },
        }
    }

    /// The whole piece of text `spot` is in.
    #[must_use]
    pub fn piece_at(&self, spot: Spot) -> Selection {
        let texts = self.0.borrow();
        let len = texts
            .iter()
            .find(|text| text.id == spot.text)
            .map_or(spot.offset, |text| text.text.len());
        Selection {
            anchor: Spot { offset: 0, ..spot },
            head: Spot {
                offset: len,
                ..spot
            },
        }
    }

    /// Where the link under `position` goes, if one is.
    #[must_use]
    pub fn link_at(&self, position: Point<Pixels>) -> Option<SharedString> {
        let texts = self.0.borrow();
        let text = texts.iter().find(|text| text.shows_at(position))?;
        link_at(&text.layout, &text.links, position)
    }

    /// The rows of the pieces this frame laid out, as the cursor goes
    /// through them.
    fn laid(&self) -> Laid {
        let texts = self.0.borrow();
        Laid::new(
            texts
                .iter()
                .filter(|text| text.current)
                .map(|text| cursor::Piece {
                    id: text.id,
                    text: &text.text,
                    rows: rows(&text.text, &text.layout),
                }),
        )
    }

    /// What visual mode selects from `anchor` to `head`.
    fn visual(&self, anchor: Spot, head: Spot, selects: Selects) -> Selection {
        let texts = self.0.borrow();
        cursor::visual(anchor, head, selects, |id| {
            texts
                .iter()
                .find(|text| text.id == id)
                .map(|text| text.text.as_ref())
        })
    }

    /// What `selection` selects of the text.
    #[must_use]
    pub fn copy(&self, selection: &Selection) -> String {
        let texts = self.0.borrow();
        selection.copy(texts.iter().map(|text| Piece {
            id: text.id,
            text: &text.text,
            joined: text.joined,
        }))
    }
}

/// A view's text as the human reads it: as the last frame laid it out,
/// what they selected of it, and whether they are selecting still, and
/// where the cursor of Vim's keys is in it.
#[derive(Default)]
pub struct Reading {
    painted: Painted,
    selection: Option<Selection>,
    /// Whether a mouse button went down on the text and is still held.
    selecting: bool,
    /// The cursor of Vim's keys, once they came to the text.
    cursor: Option<Cursor>,
    /// Whether the next frame draws the cursor: while the text has the
    /// keyboard.
    shows_cursor: bool,
    /// Where visual mode started, and what it selects, while it does.
    visual: Option<(Spot, Selects)>,
}

impl Reading {
    /// Forgets how the text was laid out, before a frame lays it out
    /// again.
    pub fn clear(&self) {
        self.painted.clear();
    }

    /// Keeps how the text of the entries before `entries` was laid out,
    /// before a frame lays out again only the text in sight: what scrolled
    /// out of it can still be selected and copied.
    pub fn age(&self, entries: usize) {
        self.painted.age(entries);
    }

    /// The text that shows from the top left of `bounds` to its bottom
    /// right, in the order it reads, as a drag between them selects it.
    #[must_use]
    pub fn text_within(&self, bounds: Bounds<Pixels>) -> String {
        let spot = |position| self.painted.spot_at(position);
        match (spot(bounds.origin), spot(bounds.bottom_right())) {
            (Some(anchor), Some(head)) => self.painted.copy(&Selection { anchor, head }),
            _ => String::new(),
        }
    }

    /// Writes entry `entry` of the text, showing what of it is selected,
    /// and the cursor, while it shows.
    #[must_use]
    pub fn writer(&self, entry: usize) -> Writer {
        let mut writer = Writer::new(&self.painted, self.selection, entry);
        writer.cursor = self
            .cursor
            .filter(|_| self.shows_cursor)
            .map(|cursor| cursor.spot);
        writer
    }

    /// Where the cursor of Vim's keys is, once they came to the text.
    #[must_use]
    pub fn cursor(&self) -> Option<Spot> {
        self.cursor.map(|cursor| cursor.spot)
    }

    /// Whether the frames draw the cursor, from the next.
    pub fn show_cursor(&mut self, shows: bool) {
        self.shows_cursor = shows;
    }

    /// Puts the cursor, if it is nowhere yet, on the start of what is
    /// selected, or else of the first row in sight, from `top` to
    /// `bottom`: answers whether it moved.
    pub fn place_cursor(&mut self, top: Pixels, bottom: Pixels) -> bool {
        if self.cursor.is_some() {
            return false;
        }
        let spot = self
            .selection
            .map(|selection| selection.anchor.min(selection.head))
            .or_else(|| {
                self.painted
                    .laid()
                    .end_in_sight(f32::from(top), f32::from(bottom), false)
            });
        self.cursor = spot.map(Cursor::at);
        self.cursor.is_some()
    }

    /// Moves the cursor as `step` says, `times` over, through the text
    /// the last frame laid out, and answers the top and bottom of the row
    /// it is on then.
    pub fn move_cursor(&mut self, step: Step, times: usize) -> Option<(Pixels, Pixels)> {
        let laid = self.painted.laid();
        let cursor = laid.moved(self.cursor?, step, times)?;
        self.put_cursor(cursor);
        laid.row_of(cursor.spot)
            .map(|(top, bottom)| (px(top), px(bottom)))
    }

    /// Puts the cursor at `cursor`, and grows what visual mode selects to
    /// it.
    fn put_cursor(&mut self, cursor: Cursor) {
        self.cursor = Some(cursor);
        if let Some((anchor, selects)) = self.visual {
            self.selection = Some(self.painted.visual(anchor, cursor.spot, selects));
        }
    }

    /// Moves the cursor, where it does not show between `top` and
    /// `bottom`, to the nearest row that does: answers whether it moved.
    pub fn cursor_into_sight(&mut self, top: Pixels, bottom: Pixels) -> bool {
        let Some(cursor) = self.cursor else {
            return false;
        };
        let moved = self
            .painted
            .laid()
            .into_sight(cursor, f32::from(top), f32::from(bottom))
            .filter(|moved| *moved != cursor);
        if let Some(moved) = moved {
            self.put_cursor(moved);
        }
        moved.is_some()
    }

    /// Moves the cursor to the start of the first row that shows between
    /// `top` and `bottom`, or of the last, as `last` says: answers whether
    /// it moved.
    pub fn cursor_to_end_in_sight(&mut self, top: Pixels, bottom: Pixels, last: bool) -> bool {
        let spot = self
            .painted
            .laid()
            .end_in_sight(f32::from(top), f32::from(bottom), last);
        match spot {
            Some(spot) if self.cursor() != Some(spot) => {
                self.put_cursor(Cursor::at(spot));
                true
            }
            _ => false,
        }
    }

    /// Moves the cursor to the start of what is selected, as a search
    /// selected it.
    pub fn cursor_to_selection(&mut self) {
        if let Some(selection) = self.selection {
            self.put_cursor(Cursor::at(selection.anchor.min(selection.head)));
        }
    }

    /// Starts selecting as `selects` says from the cursor, or selects so
    /// from where visual mode started, or stops if it selects so already.
    pub fn visual(&mut self, selects: Selects) {
        let Some(cursor) = self.cursor else {
            return;
        };
        self.visual = match self.visual {
            Some((_, was)) if was == selects => None,
            Some((anchor, _)) => Some((anchor, selects)),
            None => Some((cursor.spot, selects)),
        };
        if self.visual.is_some() {
            self.put_cursor(cursor);
        } else {
            self.selection = None;
        }
    }

    /// Whether visual mode selects.
    #[must_use]
    pub fn in_visual(&self) -> bool {
        self.visual.is_some()
    }

    /// Visual mode ends: what it selected is let go of, or kept, as
    /// `keep` says, and the cursor goes to its start, as after Vim's `y`.
    /// Answers whether it selected.
    pub fn end_visual(&mut self, keep: bool) -> bool {
        if self.visual.take().is_none() {
            return false;
        }
        let selection = if keep {
            self.selection
        } else {
            self.selection.take()
        };
        if let Some(selection) = selection {
            self.cursor = Some(Cursor::at(selection.anchor.min(selection.head)));
        }
        true
    }

    /// A button went down: start selecting there, a word or a whole piece
    /// of text at a time on a double or triple click, or with Shift, move
    /// the end of what is selected there.
    pub fn press(&mut self, event: &MouseDownEvent) {
        self.visual = None;
        let spot = self.painted.spot_at(event.position);
        self.selection = spot.map(|spot| match event.click_count {
            1 => match self.selection {
                Some(selection) if event.modifiers.shift => Selection {
                    head: spot,
                    ..selection
                },
                _ => Selection::at(spot),
            },
            2 => self.painted.word_at(spot),
            _ => self.painted.piece_at(spot),
        });
        self.selecting = self.selection.is_some();
    }

    /// The mouse moved to `position`: answers whether that changed what is
    /// selected.
    pub fn drag(&mut self, position: Point<Pixels>) -> bool {
        if !self.selecting {
            return false;
        }
        if let (Some(selection), Some(spot)) = (&mut self.selection, self.painted.spot_at(position))
            && selection.head != spot
        {
            selection.head = spot;
            return true;
        }
        false
    }

    /// The button came up at `position`: answers where the link there
    /// goes, when the press was a click that selected nothing.
    pub fn release(&mut self, position: Point<Pixels>) -> Option<SharedString> {
        let clicked = std::mem::take(&mut self.selecting)
            && self.selection.is_none_or(|selection| selection.is_empty());
        clicked.then(|| self.painted.link_at(position)).flatten()
    }

    /// The button came up away from the text.
    pub fn stop(&mut self) {
        self.selecting = false;
    }

    /// Whether `position` is on the text that shows, not beside it.
    #[must_use]
    pub fn is_on_text(&self, position: Point<Pixels>) -> bool {
        self.painted
            .0
            .borrow()
            .iter()
            .any(|text| text.shows_at(position))
    }

    /// Lets go of what is selected: answers whether something was.
    pub fn deselect(&mut self) -> bool {
        self.selection.take().is_some()
    }

    /// Selects the next match of `query` in the text the last frame laid
    /// out, from what is selected, or from the first piece that shows, as
    /// `direction` goes, and answers where the match starts as the frame
    /// drew it.
    pub fn find(&mut self, query: &str, direction: Direction) -> Option<Point<Pixels>> {
        let texts = self.painted.0.borrow();
        let from = match self.selection {
            Some(selection) => {
                let start = selection.anchor.min(selection.head);
                let at = texts.iter().position(|text| text.id == start.text)?;
                (at, start.offset)
            }
            None => (
                texts
                    .iter()
                    .position(|text| !text.visible.is_empty())
                    .unwrap_or(0),
                0,
            ),
        };
        let pieces: Vec<&str> = texts.iter().map(|text| text.text.as_ref()).collect();
        let (at, range) = search::next(&pieces, from, query, false, direction)?;
        let text = &texts[at];
        let spot = |offset| Spot {
            text: text.id,
            offset,
        };
        let start = text.layout.position_for_index(range.start);
        self.selection = Some(Selection {
            anchor: spot(range.start),
            head: spot(range.end),
        });
        start
    }

    /// The entry what is selected starts in, if something is.
    #[must_use]
    pub fn selected_entry(&self) -> Option<usize> {
        self.selection
            .map(|selection| selection.anchor.min(selection.head).text.entry)
    }

    /// Whether the last frame laid out text of entry `entry`.
    #[must_use]
    pub fn laid_out(&self, entry: usize) -> bool {
        self.painted
            .0
            .borrow()
            .iter()
            .any(|text| text.id.entry == entry && text.current)
    }

    /// Selects the next match of `query` in entry `entry`, as the last
    /// frame laid it out, as `direction` goes: from what is selected in
    /// it, when `after` and something is, else from its start or its end.
    /// Answers where the match starts as the frame drew it.
    pub fn find_in(
        &mut self,
        entry: usize,
        query: &str,
        direction: Direction,
        after: bool,
    ) -> Option<Point<Pixels>> {
        let texts = self.painted.0.borrow();
        let pieces: Vec<&PaintedText> = texts
            .iter()
            .filter(|text| text.id.entry == entry && text.current)
            .collect();
        let from = self.selection.filter(|_| after).and_then(|selection| {
            let start = selection.anchor.min(selection.head);
            let at = pieces.iter().position(|text| text.id == start.text)?;
            Some((at, start.offset))
        });
        let mut found = pieces.iter().enumerate().flat_map(|(at, text)| {
            search::matches(query, &text.text)
                .into_iter()
                .map(move |range| (at, range))
        });
        let place = |(at, range): &(usize, Range<usize>)| (*at, range.start);
        let (at, range) = match (direction, from) {
            (Direction::Forward, Some(from)) => found.find(|found| place(found) > from),
            (Direction::Forward, None) => found.next(),
            (Direction::Backward, Some(from)) => found.rev().find(|found| place(found) < from),
            (Direction::Backward, None) => found.next_back(),
        }?;
        let id = pieces[at].id;
        let start = pieces[at].layout.position_for_index(range.start);
        drop(pieces);
        drop(texts);
        let spot = |offset| Spot { text: id, offset };
        self.selection = Some(Selection {
            anchor: spot(range.start),
            head: spot(range.end),
        });
        start
    }

    /// Selects the whole of entry `entry`, as the last frame laid it out,
    /// if it shows text, and answers where it starts as the frame drew it.
    pub fn select_entry(&mut self, entry: usize) -> Option<Point<Pixels>> {
        let texts = self.painted.0.borrow();
        let mut pieces = texts
            .iter()
            .filter(|text| text.id.entry == entry && text.current && !text.text.is_empty());
        let first = pieces.next()?;
        let last = pieces.next_back().unwrap_or(first);
        let selection = Selection {
            anchor: Spot {
                text: first.id,
                offset: 0,
            },
            head: Spot {
                text: last.id,
                offset: last.text.len(),
            },
        };
        let start = first.layout.position_for_index(0);
        drop(texts);
        self.selection = Some(selection);
        start
    }

    /// Selects the whole of the piece of text after what is selected, or
    /// before it, as `direction` goes, or with nothing selected the first
    /// piece that shows, passing those with no text, and answers where it
    /// starts as the last frame drew it.
    pub fn select_piece(&mut self, direction: Direction) -> Option<Point<Pixels>> {
        let texts = self.painted.0.borrow();
        let mut pieces = texts.iter().filter(|text| !text.text.is_empty());
        let piece = match (self.selection, direction) {
            (None, _) => pieces.find(|text| !text.visible.is_empty()),
            (Some(selection), Direction::Forward) => {
                let end = selection.anchor.max(selection.head).text;
                pieces.find(|text| text.id > end)
            }
            (Some(selection), Direction::Backward) => {
                let start = selection.anchor.min(selection.head).text;
                pieces.rfind(|text| text.id < start)
            }
        }?;
        let selection = Selection {
            anchor: Spot {
                text: piece.id,
                offset: 0,
            },
            head: Spot {
                text: piece.id,
                offset: piece.text.len(),
            },
        };
        let start = piece.layout.position_for_index(0);
        drop(texts);
        self.selection = Some(selection);
        start
    }

    /// The text selected.
    #[must_use]
    pub fn selected_text(&self) -> String {
        self.selection
            .map(|selection| self.painted.copy(&selection))
            .unwrap_or_default()
    }
}

/// How far `position` is to the left or the right of `bounds`.
fn horizontal_distance(bounds: Bounds<Pixels>, position: Point<Pixels>) -> f32 {
    if position.x < bounds.left() {
        f32::from(bounds.left() - position.x)
    } else if position.x > bounds.right() {
        f32::from(position.x - bounds.right())
    } else {
        0.
    }
}

/// The offset in `text` of the boundary between characters nearest
/// `position`: past a character once the position is past its middle.
fn closest_index(layout: &TextLayout, text: &str, position: Point<Pixels>) -> usize {
    match layout.index_for_position(position) {
        Ok(index) => {
            let next = text[index..]
                .chars()
                .next()
                .map_or(index, |c| index + c.len_utf8());
            match (
                layout.position_for_index(index),
                layout.position_for_index(next),
            ) {
                (Some(start), Some(end)) if end.y == start.y => {
                    if position.x > start.x + (end.x - start.x) / 2. {
                        next
                    } else {
                        index
                    }
                }
                _ => index,
            }
        }
        Err(index) => index,
    }
}

fn link_at(
    layout: &TextLayout,
    links: &[(Range<usize>, SharedString)],
    position: Point<Pixels>,
) -> Option<SharedString> {
    if links.is_empty() {
        return None;
    }
    let index = layout.index_for_position(position).ok()?;
    links
        .iter()
        .find(|(range, _)| range.contains(&index))
        .map(|(_, url)| url.clone())
}

/// The rows `text` was laid out in by `layout`, as the cursor goes
/// through them, in the window's points.
fn rows(text: &str, layout: &TextLayout) -> Vec<cursor::Row> {
    let left = layout.bounds().left();
    let height = layout.line_height();
    let mut rows = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let (Some(wrapped), Some(origin)) = (
            layout.line_layout_for_index(start),
            layout.position_for_index(start),
        ) else {
            break;
        };
        // Where each character starts along the line unwrapped, by its
        // glyph, or the glyph before for one drawn with it.
        let glyphs: Vec<(usize, Pixels)> = wrapped
            .runs()
            .iter()
            .flat_map(|run| {
                run.glyphs
                    .iter()
                    .map(|glyph| (glyph.index, glyph.position.x))
            })
            .collect();
        let x_of = |index: usize| {
            if index >= line.len() {
                return wrapped.unwrapped_layout.width;
            }
            let at = glyphs.partition_point(|(glyph, _)| *glyph <= index);
            at.checked_sub(1).map_or(px(0.), |at| glyphs[at].1)
        };
        let wraps = wrapped
            .wrap_boundaries()
            .iter()
            .map(|boundary| wrapped.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index);
        let mut row_start = 0;
        for (row, row_end) in wraps.chain([line.len()]).enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "a piece has few rows")]
            let top = origin.y + height * row as f32;
            let from = x_of(row_start);
            let places = if row_start == row_end {
                vec![(start + row_start, f32::from(left))]
            } else {
                line[row_start..row_end]
                    .char_indices()
                    .map(|(at, _)| {
                        let index = row_start + at;
                        (start + index, f32::from(left + x_of(index) - from))
                    })
                    .collect()
            };
            rows.push(cursor::Row {
                top: f32::from(top),
                bottom: f32::from(top + height),
                end: f32::from(left + x_of(row_end) - from),
                places,
            });
            row_start = row_end;
        }
        start += line.len() + 1;
    }
    rows
}

/// A piece of text the human can select, and whose links they can click.
pub struct SelectableText {
    id: TextId,
    text: SharedString,
    joined: &'static str,
    links: Rc<[(Range<usize>, SharedString)]>,
    styled: StyledText,
    painted: Painted,
    /// Where in it the cursor of Vim's keys is drawn, if it is in it.
    cursor: Option<usize>,
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.styled.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.styled
            .prepaint(id, inspector_id, bounds, request_layout, window, cx);
        self.painted.push(PaintedText {
            id: self.id,
            text: self.text.clone(),
            joined: self.joined,
            layout: self.styled.layout().clone(),
            visible: bounds.intersect(&window.content_mask().bounds),
            links: Rc::clone(&self.links),
            current: true,
        });
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(offset) = self.cursor {
            let layout = self.styled.layout();
            let piece = cursor::Piece {
                id: self.id,
                text: &self.text,
                rows: rows(&self.text, layout),
            };
            let spot = Spot {
                text: self.id,
                offset,
            };
            let none = f32::from(layout.line_height()) / 2.;
            if let Some(block) = Laid::new([piece]).block(spot, none) {
                window.paint_quad(gpui::fill(
                    Bounds::from_corners(
                        gpui::point(px(block.x), px(block.top)),
                        gpui::point(px(block.x + block.width), px(block.bottom)),
                    ),
                    gpui::rgba((ACCENT << 8) | 0x88),
                ));
            }
        }
        self.styled.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            &mut (),
            window,
            cx,
        );
        let layout = self.styled.layout().clone();
        let on_link = |window: &Window, hitbox: &Hitbox| {
            hitbox.is_hovered(window)
                && link_at(&layout, &self.links, window.mouse_position()).is_some()
        };
        let was_on_link = on_link(window, hitbox);
        if hitbox.is_hovered(window) {
            let cursor = if was_on_link {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            };
            window.set_cursor_style(cursor, hitbox);
        }
        if self.links.is_empty() {
            return;
        }
        // The cursor changes between text and link within the piece, so
        // the frame is drawn again when the mouse crosses between them.
        let hitbox = hitbox.clone();
        let links = Rc::clone(&self.links);
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let now_on_link =
                hitbox.is_hovered(window) && link_at(&layout, &links, event.position).is_some();
            if now_on_link != was_on_link {
                window.refresh();
            }
        });
    }
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// Makes the pieces of text of one entry of a view's text, such as one of
/// the conversation, numbering them in order.
pub struct Writer {
    painted: Painted,
    selection: Option<Selection>,
    /// Where the cursor of Vim's keys is drawn, if it is.
    cursor: Option<Spot>,
    entry: usize,
    part: usize,
}

impl Writer {
    /// Writes entry `entry`, showing what of it `selection` selects.
    #[must_use]
    pub fn new(painted: &Painted, selection: Option<Selection>, entry: usize) -> Self {
        Self {
            painted: painted.clone(),
            selection,
            cursor: None,
            entry,
            part: 0,
        }
    }

    /// A piece showing `text` in its styles, joined to the piece before it
    /// by `joined` when copied.
    pub fn text(&mut self, text: &markdown::Text, joined: &'static str) -> SelectableText {
        let id = TextId {
            entry: self.entry,
            part: self.part,
        };
        self.part += 1;
        let selected = self
            .selection
            .and_then(|selection| selection.range_in(id, &text.text));
        let links: Rc<[_]> = text
            .runs
            .iter()
            .filter_map(|(range, style)| {
                let url = style.link.as_ref()?;
                Some((range.clone(), SharedString::from(url.clone())))
            })
            .collect();
        let shared = SharedString::from(text.text.clone());
        SelectableText {
            id,
            styled: StyledText::new(shared.clone())
                .with_highlights(highlights(text, selected.as_ref())),
            text: shared,
            joined,
            links,
            painted: self.painted.clone(),
            cursor: self
                .cursor
                .filter(|cursor| cursor.text == id)
                .map(|cursor| cursor.offset),
        }
    }

    /// A piece showing `text` with its web addresses as links.
    pub fn plain(&mut self, text: &str, joined: &'static str) -> SelectableText {
        self.text(&markdown::Text::plain(text), joined)
    }

    /// `text` as a note of Cox's, its commands between backticks drawn as
    /// code (see [`markdown::Text::noted`]).
    pub fn noted(&mut self, text: &str, joined: &'static str) -> SelectableText {
        self.text(&markdown::Text::noted(text), joined)
    }

    /// `markdown`, block by block.
    pub fn markdown(&mut self, markdown: &str) -> gpui::Div {
        self.blocks(&markdown::parse(markdown))
    }

    /// The `blocks` of some Markdown.
    pub fn blocks(&mut self, blocks: &[Block]) -> gpui::Div {
        let blocks: Vec<_> = blocks.iter().map(|block| self.block(block)).collect();
        div().flex().flex_col().gap_2().children(blocks)
    }

    fn block(&mut self, block: &Block) -> gpui::AnyElement {
        let indent = |depth: usize| px(INDENT * depth_f32(depth));
        let shown = match &block.kind {
            Kind::Paragraph(text) => div().pl(indent(block.depth)).child(self.text(text, "\n\n")),
            Kind::Heading(level, text) => div()
                .pl(indent(block.depth))
                .text_size(px(match level {
                    1 => 20.,
                    2 => 17.,
                    _ => 14.,
                }))
                .font_weight(FontWeight::BOLD)
                .child(self.text(text, "\n\n")),
            Kind::Item { marker, text } => div()
                .flex()
                .pl(indent(block.depth.saturating_sub(1)))
                .child(
                    div()
                        .flex_shrink_0()
                        .min_w(px(INDENT))
                        .pr_1()
                        .text_color(rgb(DIM))
                        .child(marker.clone()),
                )
                .child(div().flex_1().min_w_0().child(self.text(text, "\n"))),
            Kind::Code(text) => self.code(text).ml(indent(block.depth)),
            Kind::Rule => div().ml(indent(block.depth)).h(px(1.)).bg(rgb(BORDER)),
            Kind::Table(rows) => self.table(rows).ml(indent(block.depth)),
            Kind::Diagram(source) => self.diagram(source).ml(indent(block.depth)),
        };
        if block.quoted {
            shown
                .border_l_2()
                .border_color(rgb(BORDER_STRONG))
                .pl_2()
                .text_color(rgb(DIM))
                .into_any_element()
        } else {
            shown.into_any_element()
        }
    }

    /// A code block, with a button that copies all of it, at its right,
    /// beside the code rather than over its first line.
    fn code(&mut self, text: &markdown::Text) -> gpui::Div {
        let code = text.text.clone();
        let button = ElementId::Name(format!("copy-{}-{}", self.entry, self.part).into());
        div()
            .flex()
            .items_start()
            .gap_2()
            .p_2()
            .rounded_sm()
            .bg(rgb(RAISED))
            .font_family(CODE_FONT)
            .text_size(px(13.))
            .child(div().flex_1().min_w_0().child(self.text(text, "\n\n")))
            .child(
                div()
                    .id(button)
                    .flex_shrink_0()
                    .px_1()
                    .rounded_sm()
                    .text_size(px(11.))
                    .text_color(rgb(DIM))
                    .bg(rgb(BUTTON))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(BUTTON_HOVER)))
                    .child("Copy")
                    .on_hint_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                    }),
            )
    }

    /// A Mermaid diagram, drawn at its size, or less where there is less
    /// room, or its source as a code block when it cannot be drawn.
    fn diagram(&mut self, source: &str) -> gpui::Div {
        let Some(picture) = diagram::picture(source) else {
            return self.code(&markdown::Text::plain(source));
        };
        let size = picture.size(0);
        #[expect(clippy::cast_precision_loss, reason = "a picture's pixels")]
        let width = size.width.0 as f32 / diagram::SCALE;
        div().max_w_full().p_2().rounded_sm().bg(rgb(RAISED)).child(
            img(ImageSource::Render(picture))
                .w(px(width))
                .max_w_full()
                .object_fit(ObjectFit::Contain),
        )
    }

    /// A table, its head in bold, copied as lines of cells between tabs.
    fn table(&mut self, rows: &[Vec<markdown::Text>]) -> gpui::Div {
        let rows: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let cells: Vec<_> = row
                    .iter()
                    .enumerate()
                    .map(|(column, cell)| {
                        let joined = match (index, column) {
                            (0, 0) => "\n\n",
                            (_, 0) => "\n",
                            _ => "\t",
                        };
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_1()
                            .when(column > 0, |cell| {
                                cell.border_l_1().border_color(rgb(BORDER))
                            })
                            .child(self.text(cell, joined))
                    })
                    .collect();
                div()
                    .flex()
                    .when(index > 0, |row| row.border_t_1().border_color(rgb(BORDER)))
                    .when(index == 0, |row| row.font_weight(FontWeight::BOLD))
                    .children(cells)
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(BORDER))
            .children(rows)
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "lists are not nested millions deep"
)]
fn depth_f32(depth: usize) -> f32 {
    depth as f32
}

/// How each part of `text` looks: in its style, and marked where it is
/// selected.
fn highlights(
    text: &markdown::Text,
    selected: Option<&Range<usize>>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut cuts: Vec<usize> = text
        .runs
        .iter()
        .flat_map(|(range, _)| [range.start, range.end])
        .chain(selected.iter().flat_map(|range| [range.start, range.end]))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|cut| {
            let range = cut[0]..cut[1];
            let style = text
                .runs
                .iter()
                .find(|(run, _)| run.contains(&range.start))
                .map(|(_, style)| style)?;
            let mut highlight = HighlightStyle::default();
            if style.strong {
                highlight.font_weight = Some(FontWeight::BOLD);
            }
            if style.emphasis {
                highlight.font_style = Some(FontStyle::Italic);
            }
            if style.strikethrough {
                highlight.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                });
            }
            if style.code {
                highlight.color = Some(color(CODE));
                highlight.background_color = Some(color(RAISED));
            }
            if style.link.is_some() {
                highlight.color = Some(color(LINK));
                highlight.underline = Some(UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                });
            }
            if selected.is_some_and(|selected| selected.contains(&range.start)) {
                highlight.background_color = Some(color(SELECTION));
            }
            (highlight != HighlightStyle::default()).then_some((range, highlight))
        })
        .collect()
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

#[cfg(test)]
mod tests {
    use gpui::{point, size};

    use super::*;

    /// A piece of entry `entry`, laid out with its top at `top`.
    fn piece(entry: usize, part: usize, text: &str, top: f32) -> PaintedText {
        PaintedText {
            id: TextId { entry, part },
            text: SharedString::from(text.to_owned()),
            joined: "\n",
            layout: TextLayout::default(),
            visible: Bounds::new(point(px(0.), px(top)), size(px(10.), px(10.))),
            links: Rc::new([]),
            current: true,
        }
    }

    fn id(entry: usize, part: usize) -> TextId {
        TextId { entry, part }
    }

    /// What is copied from the start of `from` to the end of `to`.
    fn copied(reading: &Reading, from: TextId, to: TextId) -> String {
        reading.painted.copy(&Selection {
            anchor: Spot {
                text: from,
                offset: 0,
            },
            head: Spot {
                text: to,
                offset: usize::MAX,
            },
        })
    }

    #[test]
    fn aged_pieces_stay_until_their_entry_is_laid_out_again() {
        let reading = Reading::default();
        for (entry, text) in [(0, "a"), (1, "b"), (2, "c")] {
            reading.painted.push(piece(entry, 0, text, 0.));
        }
        reading.age(3);
        // The frame lays out entry 1 alone, now in two pieces.
        reading.painted.push(piece(1, 0, "b1", 20.));
        reading.painted.push(piece(1, 1, "b2", 30.));

        assert_eq!(copied(&reading, id(0, 0), id(2, 0)), "a\nb1\nb2\nc");
        assert!(reading.laid_out(1), "the frame laid out entry 1");
        assert!(!reading.laid_out(0) && !reading.laid_out(2));
    }

    #[test]
    fn aged_pieces_show_nowhere() {
        let reading = Reading::default();
        reading.painted.push(piece(0, 0, "a", 0.));
        reading.painted.push(piece(1, 0, "b", 20.));
        reading.age(2);
        reading.painted.push(piece(1, 0, "b", 20.));

        assert!(!reading.is_on_text(point(px(5.), px(5.))), "where a was");
        assert!(reading.is_on_text(point(px(5.), px(25.))), "where b is");
    }

    #[test]
    fn aging_forgets_the_entries_gone() {
        let reading = Reading::default();
        reading.painted.push(piece(0, 0, "a", 0.));
        reading.painted.push(piece(1, 0, "b", 20.));
        reading.age(1);
        assert_eq!(copied(&reading, id(0, 0), id(1, 0)), "a");
    }

    #[test]
    fn clearing_forgets_every_piece() {
        let reading = Reading::default();
        reading.painted.push(piece(0, 0, "a", 0.));
        reading.age(1);
        reading.clear();
        reading.painted.push(piece(1, 0, "b", 0.));
        reading.painted.push(piece(0, 0, "a", 20.));
        // In the order laid out, as a view that does not age its text
        // has them.
        assert_eq!(copied(&reading, id(0, 0), id(1, 0)), "b\na");
    }

    /// A reading of the pieces `pieces`, each of its entry and part, its
    /// text, and the top it shows at, 20 pixels apart and 10 tall.
    fn reading(pieces: &[(usize, usize, &str)]) -> Reading {
        let reading = Reading::default();
        for (at, &(entry, part, text)) in pieces.iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "a few pieces")]
            let top = at as f32 * 20.;
            reading.painted.push(piece(entry, part, text, top));
        }
        reading
    }

    fn spot(entry: usize, part: usize, offset: usize) -> Spot {
        Spot {
            text: id(entry, part),
            offset,
        }
    }

    #[test]
    fn a_double_click_selects_the_word_it_is_on() {
        let reading = reading(&[(0, 0, "hello wide world")]);

        let word = reading.painted.word_at(spot(0, 0, 8));

        assert_eq!(reading.painted.copy(&word), "wide");
    }

    #[test]
    fn a_double_click_on_spaces_selects_the_spaces() {
        let reading = reading(&[(0, 0, "a   b")]);

        let word = reading.painted.word_at(spot(0, 0, 2));

        assert_eq!(reading.painted.copy(&word), "   ");
    }

    #[test]
    fn a_word_in_no_piece_shown_is_nothing() {
        let reading = reading(&[(0, 0, "hello")]);

        let word = reading.painted.word_at(spot(5, 0, 2));

        assert_eq!(word, Selection::at(spot(5, 0, 2)));
    }

    #[test]
    fn a_spot_between_pieces_is_the_start_of_the_piece_below() {
        let reading = reading(&[(0, 0, "first"), (1, 0, "second")]);

        let found = reading.painted.spot_at(point(px(5.), px(15.)));

        assert_eq!(found, Some(spot(1, 0, 0)));
    }

    #[test]
    fn a_spot_below_every_piece_is_the_end_of_the_last() {
        let reading = reading(&[(0, 0, "first"), (1, 0, "second")]);

        let found = reading.painted.spot_at(point(px(5.), px(500.)));

        assert_eq!(found, Some(spot(1, 0, "second".len())));
    }

    #[test]
    fn no_spot_is_in_text_that_shows_nowhere() {
        let reading = Reading::default();

        assert_eq!(reading.painted.spot_at(point(px(5.), px(5.))), None);
    }

    #[test]
    fn an_entry_with_no_text_laid_out_is_not_selected() {
        let mut reading = reading(&[(0, 0, "only")]);

        reading.select_entry(3);

        assert_eq!(reading.selected_entry(), None);
    }

    #[test]
    fn nothing_selected_copies_nothing() {
        let reading = reading(&[(0, 0, "text")]);

        assert_eq!(reading.selected_text(), "");
    }
}
