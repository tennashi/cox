//! The text in an input as it is typed, without drawing it: what it says,
//! what is selected, and what the input method is still composing.
//!
//! Offsets are in bytes of the text, but for those the input method gives
//! and gets, which count UTF-16 units, as the platforms do.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// The text in an input, with its selection and composition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    text: String,
    /// Byte range of the selection; empty when it is just the cursor.
    selected: Range<usize>,
    /// Whether the cursor is at the start of the selection rather than its
    /// end: the selection grew to the left.
    reversed: bool,
    /// Byte range the input method is composing.
    marked: Option<Range<usize>>,
    history: History,
}

/// The text and its selection as they were before an edit, to go back to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Was {
    text: String,
    selected: Range<usize>,
    reversed: bool,
}

/// The edits that can be undone and redone, each as what the text was
/// before it, and the last edit, which one that goes on from it joins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct History {
    undone: Vec<Was>,
    redone: Vec<Was>,
    last: Option<Last>,
}

/// An edit that the next joins, if it is of the same kind and starts
/// where it left the cursor: letters typed one after another, or taken
/// out, are undone together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Last {
    kind: Kind,
    cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Typing,
    Deleting,
    Other,
}

/// How many edits can be undone.
const UNDOABLE: usize = 200;

impl Draft {
    /// An empty text, with the cursor at its start.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Byte range of the selection; empty when it is just the cursor.
    #[must_use]
    pub fn selected(&self) -> &Range<usize> {
        &self.selected
    }

    #[must_use]
    pub fn selected_text(&self) -> &str {
        &self.text[self.selected.clone()]
    }

    /// Whether the cursor is at the start of the selection rather than its
    /// end.
    #[must_use]
    pub fn is_reversed(&self) -> bool {
        self.reversed
    }

    /// Where the cursor is: the end of the selection it moves.
    #[must_use]
    pub fn cursor(&self) -> usize {
        if self.reversed {
            self.selected.start
        } else {
            self.selected.end
        }
    }

    /// Byte range the input method is composing, if it is.
    #[must_use]
    pub fn marked(&self) -> Option<&Range<usize>> {
        self.marked.as_ref()
    }

    /// Whether an input method is composing text.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    /// Puts `text` in place of everything, with the cursor at its end,
    /// and nothing to undo.
    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.move_to(self.text.len());
        self.marked = None;
        self.history = History::default();
    }

    /// Goes back to the text before the last edit, if there is one.
    pub fn undo(&mut self) -> bool {
        let Some(was) = self.history.undone.pop() else {
            return false;
        };
        let now = self.was();
        self.history.redone.push(now);
        self.go_back_to(was);
        true
    }

    /// Does again the last edit undone, if there is one.
    pub fn redo(&mut self) -> bool {
        let Some(was) = self.history.redone.pop() else {
            return false;
        };
        let now = self.was();
        self.history.undone.push(now);
        self.go_back_to(was);
        true
    }

    fn was(&self) -> Was {
        Was {
            text: self.text.clone(),
            selected: self.selected.clone(),
            reversed: self.reversed,
        }
    }

    fn go_back_to(&mut self, was: Was) {
        self.text = was.text;
        self.selected = was.selected;
        self.reversed = was.reversed;
        self.marked = None;
        self.history.last = None;
    }

    /// Keeps the text as it is before an edit of `kind`, unless the edit
    /// goes on from the last, or from a composition, which is one edit
    /// from its start.
    fn note(&mut self, kind: Kind, range: &Range<usize>) {
        if self.marked.is_some() {
            return;
        }
        let goes_on = kind != Kind::Other
            && self.history.last.is_some_and(|last| {
                last.kind == kind && (range.start == last.cursor || range.end == last.cursor)
            });
        if !goes_on {
            let was = self.was();
            self.history.undone.push(was);
            if self.history.undone.len() > UNDOABLE {
                self.history.undone.remove(0);
            }
        }
        self.history.redone.clear();
    }

    /// Puts the cursor at `offset`, with nothing selected.
    pub fn move_to(&mut self, offset: usize) {
        self.selected = offset..offset;
        self.reversed = false;
    }

    /// Moves the cursor to `offset`, keeping the other end of the selection
    /// where it is.
    pub fn select_to(&mut self, offset: usize) {
        let anchor = if self.reversed {
            self.selected.end
        } else {
            self.selected.start
        };
        self.reversed = offset < anchor;
        self.selected = anchor.min(offset)..anchor.max(offset);
    }

    /// Selects the whole text, so that typing replaces it.
    pub fn select_all(&mut self) {
        self.move_to(0);
        self.select_to(self.text.len());
    }

    /// Leaves the cursor where it is, with nothing selected.
    pub fn deselect(&mut self) {
        self.move_to(self.cursor());
    }

    /// Left: to the start of the selection, or a character back.
    pub fn move_left(&mut self) {
        let offset = if self.selected.is_empty() {
            self.previous_boundary(self.selected.start)
        } else {
            self.selected.start
        };
        self.move_to(offset);
    }

    /// Right: to the end of the selection, or a character on.
    pub fn move_right(&mut self) {
        let offset = if self.selected.is_empty() {
            self.next_boundary(self.selected.end)
        } else {
            self.selected.end
        };
        self.move_to(offset);
    }

    /// Shift+Left: the selection grows, or shrinks, a character to the left.
    pub fn select_left(&mut self) {
        self.select_to(self.previous_boundary(self.cursor()));
    }

    /// Shift+Right: the selection grows, or shrinks, a character to the
    /// right.
    pub fn select_right(&mut self) {
        self.select_to(self.next_boundary(self.cursor()));
    }

    /// Option+Left (Control+Left off macOS): to the start of the word
    /// before the selection, or before the cursor.
    pub fn move_word_left(&mut self) {
        self.move_to(self.previous_word_start(self.selected.start));
    }

    /// Option+Right (Control+Right off macOS): to the end of the word after
    /// the selection, or after the cursor.
    pub fn move_word_right(&mut self) {
        self.move_to(self.next_word_end(self.selected.end));
    }

    /// The selection grows, or shrinks, to the start of the word before
    /// the cursor.
    pub fn select_word_left(&mut self) {
        self.select_to(self.previous_word_start(self.cursor()));
    }

    /// The selection grows, or shrinks, to the end of the word after the
    /// cursor.
    pub fn select_word_right(&mut self) {
        self.select_to(self.next_word_end(self.cursor()));
    }

    /// Selects from where `origin` starts or ends to where `reached` does,
    /// whichever takes in both: a drag that started by selecting `origin`,
    /// a word or a line, and is now over `reached`, another.
    pub fn select_spanning(&mut self, origin: &Range<usize>, reached: &Range<usize>) {
        self.reversed = reached.start < origin.start;
        self.selected = if self.reversed {
            reached.start..origin.end
        } else {
            origin.start..reached.end.max(origin.end)
        };
    }

    /// Backspace: takes out the selection, or the character before the
    /// cursor.
    pub fn delete_backward(&mut self) {
        if self.selected.is_empty() {
            let start = self.previous_boundary(self.selected.start);
            self.selected = start..self.selected.end;
        }
        self.commit(None, "");
    }

    /// Delete: takes out the selection, or the character after the cursor.
    pub fn delete_forward(&mut self) {
        if self.selected.is_empty() {
            let end = self.next_boundary(self.selected.end);
            self.selected = self.selected.start..end;
        }
        self.commit(None, "");
    }

    /// Option+Backspace (Control+Backspace off macOS): takes out the
    /// selection, or the word before the cursor.
    pub fn delete_word_backward(&mut self) {
        self.delete_to(self.previous_word_start(self.selected.start));
    }

    /// Option+Delete (Control+Delete off macOS): takes out the selection,
    /// or the word after the cursor.
    pub fn delete_word_forward(&mut self) {
        self.delete_to(self.next_word_end(self.selected.end));
    }

    /// Takes out the selection, or what is between the cursor and
    /// `offset`.
    pub fn delete_to(&mut self, offset: usize) {
        if self.selected.is_empty() {
            let cursor = self.selected.start;
            self.selected = cursor.min(offset)..cursor.max(offset);
        }
        self.commit(None, "");
    }

    /// What a double click at `offset` selects: the word there, or the run
    /// of spaces or of punctuation, or at the end of a line, the one
    /// before it.
    #[must_use]
    pub fn word_at(&self, offset: usize) -> Range<usize> {
        let mut before = offset..offset;
        for (start, piece) in self.text.split_word_bound_indices() {
            if piece.starts_with(['\n', '\r']) {
                continue;
            }
            let piece = start..start + piece.len();
            if piece.contains(&offset) {
                return piece;
            }
            if piece.end == offset {
                before = piece;
            }
        }
        before
    }

    /// What a triple click at `offset` selects: the line there, between
    /// line breaks.
    #[must_use]
    pub fn line_at(&self, offset: usize) -> Range<usize> {
        let start = self.text[..offset].rfind('\n').map_or(0, |at| at + 1);
        let end = self.text[offset..]
            .find(['\r', '\n'])
            .map_or(self.text.len(), |at| offset + at);
        start..end
    }

    /// Final text, typed or committed by the input method: `text` in place
    /// of `range_utf16`, else the text being composed, else the selection,
    /// with the cursor after it.
    pub fn commit(&mut self, range_utf16: Option<&Range<usize>>, text: &str) {
        let range = self.target_range(range_utf16);
        let kind = if text.is_empty() {
            Kind::Deleting
        } else if range.is_empty() && text.chars().count() == 1 && text != "\n" {
            Kind::Typing
        } else {
            Kind::Other
        };
        let changes = !(range.is_empty() && text.is_empty());
        if changes {
            self.note(kind, &range);
        }
        let placed = self.replace(range, text);
        self.move_to(placed.end);
        self.marked = None;
        if changes {
            self.history.last = Some(Last {
                kind,
                cursor: placed.end,
            });
        }
    }

    /// Text the input method is still composing: `text` in place of
    /// `range_utf16`, else the text being composed, else the selection,
    /// which it marks, with `selected_utf16` of it selected, counted from
    /// its start, or else the cursor after it.
    pub fn compose(
        &mut self,
        range_utf16: Option<&Range<usize>>,
        text: &str,
        selected_utf16: Option<&Range<usize>>,
    ) {
        let range = self.target_range(range_utf16);
        if !(range.is_empty() && text.is_empty()) {
            self.note(Kind::Other, &range);
        }
        let placed = self.replace(range, text);
        self.marked = (!text.is_empty()).then(|| placed.clone());
        self.selected = match selected_utf16 {
            Some(selected) => {
                let offset = self.offset_to_utf16(placed.start);
                self.range_from_utf16(&(selected.start + offset..selected.end + offset))
            }
            None => placed.end..placed.end,
        };
        self.reversed = false;
    }

    /// The input method stopped composing, leaving the text as it is.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// The text in `range_utf16`, and that range as it fits the text's
    /// characters.
    #[must_use]
    pub fn text_in(&self, range_utf16: &Range<usize>) -> (String, Range<usize>) {
        let range = self.range_from_utf16(range_utf16);
        let adjusted = self.range_to_utf16(&range);
        (self.text[range].to_owned(), adjusted)
    }

    /// The byte offset of the character at `offset` UTF-16 units in.
    #[must_use]
    pub fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut bytes = 0;
        for ch in self.text.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            bytes += ch.len_utf8();
        }
        bytes
    }

    /// How many UTF-16 units in byte `offset` is.
    #[must_use]
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text[..offset].encode_utf16().count()
    }

    #[must_use]
    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    #[must_use]
    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    /// The start of the character before `offset`, or the start of the
    /// text.
    fn previous_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .rev()
            .map(|(index, _)| index)
            .find(|&index| index < offset)
            .unwrap_or(0)
    }

    /// The start of the character after the one at `offset`, or the end of
    /// the text.
    fn next_boundary(&self, offset: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|&index| index > offset)
            .unwrap_or(self.text.len())
    }

    /// The words of the text: the pieces Unicode tells words apart into
    /// that hold a letter or a digit, rather than spaces or punctuation.
    fn words(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.text
            .split_word_bound_indices()
            .filter(|(_, piece)| piece.chars().any(char::is_alphanumeric))
            .map(|(start, piece)| start..start + piece.len())
    }

    /// The start of the word before `offset`, or in it, or the start of
    /// the text.
    fn previous_word_start(&self, offset: usize) -> usize {
        self.words()
            .map(|word| word.start)
            .take_while(|&start| start < offset)
            .last()
            .unwrap_or(0)
    }

    /// The end of the word after `offset`, or in it, or the end of the
    /// text.
    fn next_word_end(&self, offset: usize) -> usize {
        self.words()
            .map(|word| word.end)
            .find(|&end| end > offset)
            .unwrap_or(self.text.len())
    }

    /// The range a replacement applies to: the one given, else the text
    /// being composed, else the selection.
    fn target_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selected.clone())
    }

    /// Puts `text` in place of `range`, and returns where it ended up.
    fn replace(&mut self, range: Range<usize>, text: &str) -> Range<usize> {
        self.text.replace_range(range.clone(), text);
        range.start..range.start + text.len()
    }
}

/// A pasted text as one line: each line break, of any kind, becomes a
/// space.
#[must_use]
pub fn one_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// A pasted text with each line break, of any kind, as `\n`.
#[must_use]
pub fn unix_line_breaks(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::{Draft, one_line, unix_line_breaks};

    /// A draft saying `text`, with the cursor at its end.
    fn draft(text: &str) -> Draft {
        let mut draft = Draft::new();
        draft.set_text(text);
        draft
    }

    #[test]
    fn typing_goes_in_at_the_cursor_and_in_place_of_the_selection() {
        let mut draft = draft("ab");
        draft.commit(None, "c");
        assert_eq!(draft.text(), "abc");
        assert_eq!(draft.cursor(), 3);
        draft.move_to(1);
        draft.commit(None, "X");
        assert_eq!(draft.text(), "aXbc");
        draft.move_to(0);
        draft.select_to(2);
        draft.commit(None, "Y");
        assert_eq!(draft.text(), "Ybc");
        assert_eq!(draft.cursor(), 1);
    }

    #[test]
    fn the_cursor_moves_by_characters_not_bytes() {
        // A flag, of two code points, and a family, joined of four.
        let mut draft = draft("a🇯🇵👨‍👩‍👧‍👦");
        assert_eq!(draft.cursor(), draft.text().len());
        draft.move_left();
        assert_eq!(draft.cursor(), "a🇯🇵".len());
        draft.move_left();
        assert_eq!(draft.cursor(), 1);
        draft.move_left();
        draft.move_left();
        assert_eq!(draft.cursor(), 0);
        draft.move_right();
        draft.move_right();
        assert_eq!(draft.cursor(), "a🇯🇵".len());
    }

    #[test]
    fn backspace_and_delete_take_out_a_character_or_the_selection() {
        let mut draft = draft("aé𝄞b");
        draft.move_left();
        draft.delete_backward();
        assert_eq!(draft.text(), "aéb");
        draft.move_to(0);
        draft.delete_forward();
        assert_eq!(draft.text(), "éb");
        draft.select_all();
        draft.delete_backward();
        assert_eq!(draft.text(), "");
        draft.delete_backward();
        draft.delete_forward();
        assert_eq!(draft.text(), "");
    }

    #[test]
    fn a_selection_keeps_its_anchor_and_can_turn_around_it() {
        let mut draft = draft("abcd");
        draft.move_left();
        draft.move_left();
        draft.select_left();
        assert_eq!(draft.selected(), &(1..2));
        assert!(draft.is_reversed());
        assert_eq!(draft.cursor(), 1);
        draft.select_right();
        draft.select_right();
        draft.select_right();
        assert_eq!(draft.selected(), &(2..4));
        assert!(!draft.is_reversed());
        assert_eq!(draft.selected_text(), "cd");
        draft.move_left();
        assert_eq!(draft.cursor(), 2);
        assert!(draft.selected().is_empty());
    }

    #[test]
    fn deselecting_leaves_the_cursor_where_it_is() {
        let mut draft = draft("abc");
        draft.move_to(0);
        draft.select_to(2);
        draft.deselect();
        assert_eq!(draft.selected(), &(2..2));
        draft.select_to(0);
        draft.deselect();
        assert_eq!(draft.selected(), &(0..0));
    }

    #[test]
    fn a_composition_is_marked_until_it_is_committed() {
        let mut draft = draft("a");
        draft.compose(None, "k", None);
        assert_eq!(draft.text(), "ak");
        assert_eq!(draft.marked(), Some(&(1..2)));
        assert!(draft.is_composing());
        // The next piece replaces what is composed, not the selection.
        draft.move_to(0);
        draft.compose(None, "か", None);
        assert_eq!(draft.text(), "aか");
        assert_eq!(draft.marked(), Some(&(1..4)));
        assert_eq!(draft.cursor(), 4);
        draft.commit(None, "漢");
        assert_eq!(draft.text(), "a漢");
        assert!(!draft.is_composing());
        assert_eq!(draft.cursor(), 4);
    }

    #[test]
    fn a_composition_selects_within_itself_as_the_input_method_says() {
        let mut draft = draft("𝄞");
        draft.compose(None, "abc", Some(&(1..2)));
        // From the composition's start, in UTF-16 units: after the clef's
        // two.
        assert_eq!(draft.selected(), &(5..6));
        assert_eq!(draft.selected_text(), "b");
        draft.compose(None, "", None);
        assert_eq!(draft.text(), "𝄞");
        assert!(!draft.is_composing());
    }

    #[test]
    fn unmarking_keeps_the_text_as_composed() {
        let mut draft = draft("");
        draft.compose(None, "ab", None);
        draft.unmark();
        assert_eq!(draft.text(), "ab");
        assert!(!draft.is_composing());
        draft.commit(None, "c");
        assert_eq!(draft.text(), "abc");
    }

    #[test]
    fn the_input_method_counts_utf16_units() {
        let draft = draft("a𝄞b");
        assert_eq!(draft.offset_to_utf16(5), 3);
        assert_eq!(draft.offset_from_utf16(3), 5);
        assert_eq!(draft.range_to_utf16(&(1..5)), 1..3);
        assert_eq!(draft.range_from_utf16(&(1..3)), 1..5);
        // Past the end is the end.
        assert_eq!(draft.offset_from_utf16(9), 6);
        // Within the clef, the range widens to it.
        assert_eq!(draft.text_in(&(1..2)), ("𝄞".to_owned(), 1..3));
        assert_eq!(draft.text_in(&(0..4)), ("a𝄞b".to_owned(), 0..4));
    }

    #[test]
    fn a_replacement_the_input_method_places_goes_where_it_says() {
        let mut draft = draft("abcd");
        draft.commit(Some(&(1..3)), "X");
        assert_eq!(draft.text(), "aXd");
        assert_eq!(draft.cursor(), 2);
    }

    #[test]
    fn setting_the_text_puts_the_cursor_at_its_end_and_stops_composing() {
        let mut draft = draft("");
        draft.compose(None, "k", None);
        draft.set_text("hello");
        assert_eq!(draft.text(), "hello");
        assert_eq!(draft.cursor(), 5);
        assert!(!draft.is_composing());
    }

    #[test]
    fn the_cursor_moves_by_words_past_spaces_and_punctuation() {
        let mut draft = draft("one, two  three");
        draft.move_word_left();
        assert_eq!(draft.cursor(), 10);
        draft.move_word_left();
        assert_eq!(draft.cursor(), 5);
        draft.move_word_left();
        draft.move_word_left();
        assert_eq!(draft.cursor(), 0);
        draft.move_word_right();
        assert_eq!(draft.cursor(), 3);
        draft.move_word_right();
        assert_eq!(draft.cursor(), 8);
        // From the middle of a word, to its end.
        draft.move_to(11);
        draft.move_word_right();
        assert_eq!(draft.cursor(), 15);
        draft.move_word_right();
        assert_eq!(draft.cursor(), 15);
    }

    #[test]
    fn a_selection_grows_by_words() {
        let mut draft = draft("one two three");
        draft.move_to(7);
        draft.select_word_left();
        assert_eq!(draft.selected_text(), "two");
        draft.select_word_left();
        assert_eq!(draft.selected_text(), "one two");
        // The cursor goes to the end of the word it was at the start of.
        draft.select_word_right();
        assert_eq!(draft.selected_text(), " two");
        // A move with a selection starts from its end on that side.
        draft.move_word_right();
        assert_eq!(draft.cursor(), 13);
    }

    #[test]
    fn whole_words_are_taken_out_with_what_is_between() {
        let mut draft = draft("one two  three");
        draft.move_to(7);
        draft.delete_word_forward();
        assert_eq!(draft.text(), "one two");
        draft.delete_word_backward();
        assert_eq!(draft.text(), "one ");
        draft.delete_word_backward();
        assert_eq!(draft.text(), "");
        draft.set_text("abc");
        draft.delete_to(1);
        assert_eq!(draft.text(), "a");
    }

    #[test]
    fn a_double_click_selects_the_word_or_what_is_between_words() {
        let draft = draft("one, two\nthree");
        assert_eq!(draft.word_at(1), 0..3);
        assert_eq!(draft.word_at(3), 3..4);
        assert_eq!(draft.word_at(4), 4..5);
        // At the end of a line, and of the text, the word before.
        assert_eq!(draft.word_at(8), 5..8);
        assert_eq!(draft.word_at(14), 9..14);
        assert_eq!(draft.word_at(0), 0..3);
        assert_eq!(Draft::new().word_at(0), 0..0);
    }

    #[test]
    fn a_triple_click_selects_the_line() {
        let draft = draft("one\ntwo three\r\nfour");
        assert_eq!(draft.line_at(1), 0..3);
        assert_eq!(draft.line_at(3), 0..3);
        assert_eq!(draft.line_at(6), 4..13);
        assert_eq!(draft.line_at(15), 15..19);
    }

    #[test]
    fn a_drag_selects_whole_words_either_way() {
        let mut draft = draft("one two three");
        draft.select_spanning(&(4..7), &(8..13));
        assert_eq!(draft.selected_text(), "two three");
        assert!(!draft.is_reversed());
        draft.select_spanning(&(4..7), &(0..3));
        assert_eq!(draft.selected_text(), "one two");
        assert!(draft.is_reversed());
        draft.select_spanning(&(4..7), &(4..7));
        assert_eq!(draft.selected_text(), "two");
    }

    #[test]
    fn letters_typed_one_after_another_are_undone_together() {
        let mut draft = Draft::new();
        for c in ["h", "i"] {
            draft.commit(None, c);
        }
        draft.move_to(0);
        draft.commit(None, ">");
        assert_eq!(draft.text(), ">hi");
        assert!(draft.undo());
        assert_eq!(draft.text(), "hi", "typing elsewhere is an edit of its own");
        assert!(draft.undo());
        assert_eq!(draft.text(), "");
        assert!(!draft.undo());
        assert!(draft.redo());
        assert_eq!(draft.text(), "hi");
        assert!(draft.redo());
        assert_eq!(draft.text(), ">hi");
        assert!(!draft.redo());
    }

    #[test]
    fn taking_out_letters_one_after_another_is_undone_together() {
        let mut draft = draft("abc");
        draft.delete_backward();
        draft.delete_backward();
        assert_eq!(draft.text(), "a");
        assert!(draft.undo());
        assert_eq!((draft.text(), draft.cursor()), ("abc", 3));
        assert!(!draft.undo(), "the text set is not undone");
    }

    #[test]
    fn a_new_edit_forgets_what_was_undone() {
        let mut draft = draft("abc");
        draft.delete_backward();
        assert!(draft.undo());
        draft.commit(None, "!");
        assert!(!draft.redo());
    }

    #[test]
    fn a_paste_is_an_edit_of_its_own() {
        let mut draft = Draft::new();
        draft.commit(None, "a");
        draft.commit(None, "pasted");
        assert!(draft.undo());
        assert_eq!(draft.text(), "a");
    }

    #[test]
    fn a_composition_is_undone_whole() {
        let mut draft = draft("x");
        draft.compose(None, "k", None);
        draft.compose(None, "か", None);
        draft.commit(None, "火");
        assert_eq!(draft.text(), "x火");
        assert!(draft.undo());
        assert_eq!(draft.text(), "x");
    }

    #[test]
    fn pasted_line_breaks_become_spaces_or_newlines() {
        assert_eq!(one_line("one\ntwo\r\nthree\rfour"), "one two three four");
        assert_eq!(
            unix_line_breaks("one\ntwo\r\nthree\rfour"),
            "one\ntwo\nthree\nfour"
        );
    }
}
