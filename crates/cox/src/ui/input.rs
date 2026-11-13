//! A text input that an input method can compose into: one line that
//! scrolls sideways, or lines that wrap at the input's width and grow it
//! downwards.
//!
//! GPUI hands typed and composed text to an [`EntityInputHandler`]: typed
//! characters and committed compositions arrive as replacements, and text
//! the input method is still composing arrives as marked text, which the
//! input shows underlined. Offsets in that trait count UTF-16 units, as the
//! platforms do, while the text is indexed in bytes.
//!
//! The human selects with the mouse or with Shift and the arrows, and cuts,
//! copies and pastes with the usual keys. Enter submits. In an input that
//! wraps, Shift+Enter starts a new line; in one that holds one line, a
//! pasted line break becomes a space. The input tells when the keyboard
//! leaves it, for what keeps what was typed without Enter. On macOS, a key with Control that no
//! binding takes goes to the input method, as macSKK's Control-J.
//!
//! [`draft`] is the text as it is typed, with its selection and
//! composition, apart from drawing it.

pub mod draft;
#[cfg(target_os = "macos")]
mod macos;

use std::ops::Range;

use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, ContentMask, Context, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    GlobalElementId, InspectorElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, SharedString, Style, Subscription,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div, fill,
    point, prelude::*, px, relative, rgb, size,
};

use self::draft::{Draft, one_line, unix_line_breaks};
use crate::ui::theme::{FAINT, FOREGROUND, SELECTION};
use crate::ui::translation::Translate;

actions!(
    line_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        Home,
        End,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectToHome,
        SelectToEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        TextStart,
        TextEnd,
        SelectToTextStart,
        SelectToTextEnd,
        DeleteWordBackward,
        DeleteWordForward,
        DeleteToHome,
        DeleteToEnd,
        SelectAll,
        Cut,
        Copy,
        Paste,
        Submit,
        Newline,
    ]
);

const KEY_CONTEXT: &str = "LineInput";

/// Binds the keys the input answers to. Call once when the app starts.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some(KEY_CONTEXT)),
        KeyBinding::new("delete", Delete, Some(KEY_CONTEXT)),
        KeyBinding::new("left", Left, Some(KEY_CONTEXT)),
        KeyBinding::new("right", Right, Some(KEY_CONTEXT)),
        KeyBinding::new("up", Up, Some(KEY_CONTEXT)),
        KeyBinding::new("down", Down, Some(KEY_CONTEXT)),
        KeyBinding::new("home", Home, Some(KEY_CONTEXT)),
        KeyBinding::new("end", End, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-home", SelectToHome, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-end", SelectToEnd, Some(KEY_CONTEXT)),
        // The secondary key is Command on macOS, Control elsewhere.
        KeyBinding::new("secondary-a", SelectAll, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-x", Cut, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-v", Paste, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", Submit, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-enter", Newline, Some(KEY_CONTEXT)),
    ]);
    if cfg!(target_os = "macos") {
        // As text fields on macOS take them, Emacs's Control keys too.
        cx.bind_keys([
            KeyBinding::new("alt-left", WordLeft, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-right", WordRight, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-shift-left", SelectWordLeft, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-shift-right", SelectWordRight, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-backspace", DeleteWordBackward, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-delete", DeleteWordForward, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-left", Home, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-right", End, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-left", SelectToHome, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-right", SelectToEnd, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-up", TextStart, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-down", TextEnd, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-up", SelectToTextStart, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-down", SelectToTextEnd, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-backspace", DeleteToHome, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-a", Home, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-e", End, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-k", DeleteToEnd, Some(KEY_CONTEXT)),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-left", WordLeft, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-right", WordRight, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-shift-right", SelectWordRight, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-backspace", DeleteWordBackward, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-delete", DeleteWordForward, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-home", TextStart, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-end", TextEnd, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-shift-home", SelectToTextStart, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-shift-end", SelectToTextEnd, Some(KEY_CONTEXT)),
        ]);
    }
}

/// The human pressed Enter with no composition in progress.
pub struct Submitted;

/// The keyboard left the input, as when the human clicked elsewhere or
/// pressed Tab.
pub struct Blurred;

pub struct LineInput {
    focus_handle: FocusHandle,
    /// The text, with its selection and what the input method is
    /// composing, which is shown underlined.
    draft: Draft,
    /// What the mouse selects by while its button, which went down on the
    /// input, is held.
    drag: Option<Drag>,
    /// Whether lines wrap at the input's width, rather than the input
    /// holding one line.
    wraps: bool,
    /// What the last paint drew, and where, for placing the input method's
    /// candidate window and finding the text under the mouse.
    last_layout: Option<Layout>,
    last_bounds: Option<Bounds<Pixels>>,
    /// How far the text is moved left and up, so that the cursor stays in
    /// view when the text does not fit.
    scroll: Point<Pixels>,
    /// What the input shows, fainter, while it is empty.
    placeholder: SharedString,
    /// What tells the input the keyboard left it, once it is drawn in a
    /// window, which it has to be to get the keyboard at all.
    left: Option<Subscription>,
}

/// What a drag selects by: characters, after a click; words, after a
/// double click; lines, after a triple click, each word or line the mouse
/// passes adding to the one first selected.
#[derive(Clone, Debug)]
enum Drag {
    Characters,
    Words(Range<usize>),
    Lines(Range<usize>),
}

/// How many rows an input that wraps grows to before it scrolls.
const MAX_ROWS: usize = 8;

impl LineInput {
    /// An input that holds one line.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            draft: Draft::new(),
            drag: None,
            wraps: false,
            last_layout: None,
            last_bounds: None,
            scroll: Point::default(),
            placeholder: SharedString::default(),
            left: None,
        }
    }

    /// An input whose lines wrap at its width, and which grows downwards
    /// with them, up to eight rows.
    pub fn wrapping(cx: &mut Context<Self>) -> Self {
        Self {
            wraps: true,
            ..Self::new(cx)
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        self.draft.text()
    }

    /// Whether an input method is composing text.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.draft.is_composing()
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    /// Puts `text` in the input, with the cursor at its end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.edit(cx, |draft| draft.set_text(text));
    }

    /// Puts `text` in the input, with the cursor at `cursor`, a byte
    /// offset of it.
    pub fn set_text_at(&mut self, text: &str, cursor: usize, cx: &mut Context<Self>) {
        self.edit(cx, |draft| {
            draft.set_text(text);
            draft.move_to(cursor);
        });
    }

    /// Where the cursor is, as a byte offset of the text.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.draft.cursor()
    }

    /// Puts `text` in place of `range` of the text, a range of byte
    /// offsets, with the cursor after it.
    pub fn replace(&mut self, range: Range<usize>, text: &str, cx: &mut Context<Self>) {
        self.edit(cx, |draft| {
            draft.move_to(range.start);
            draft.select_to(range.end);
            draft.commit(None, text);
        });
    }

    /// Where in the window the input was last drawn. `None` before it was.
    #[must_use]
    pub fn drawn_at(&self) -> Option<Bounds<Pixels>> {
        self.last_bounds
    }

    /// Where in the window the row of offset `offset` of the text starts
    /// above it, as the input last drew it: the top left corner of what
    /// is at the offset. `None` before it was drawn.
    #[must_use]
    pub fn position_of(&self, offset: usize) -> Option<Point<Pixels>> {
        let (bounds, layout) = (self.last_bounds?, self.last_layout.as_ref()?);
        Some(self.text_origin(bounds) + layout.position(offset))
    }

    /// What the input shows, fainter, while it is empty.
    #[must_use]
    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    /// Shows `placeholder`, fainter, while the input is empty.
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let placeholder = placeholder.into();
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            cx.notify();
        }
    }

    /// Changes the text, or the selection, and draws it again.
    fn edit(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Draft)) {
        change(&mut self.draft);
        crate::ui::composing::note(cx.entity_id(), self.draft.is_composing(), cx);
        cx.notify();
    }

    fn submit(&mut self, _: &Submit, _window: &mut Window, cx: &mut Context<Self>) {
        // Enter while composing belongs to the input method.
        if !self.draft.is_composing() {
            cx.emit(Submitted);
        }
    }

    /// Shift+Enter: a new line where lines wrap, else the same as Enter.
    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if !self.wraps {
            self.submit(&Submit, window, cx);
        } else if !self.draft.is_composing() {
            self.edit(cx, |draft| draft.commit(None, "\n"));
        }
    }

    fn backspace(&mut self, _: &Backspace, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::delete_backward);
    }

    fn delete(&mut self, _: &Delete, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::delete_forward);
    }

    fn left(&mut self, _: &Left, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::move_left);
    }

    fn right(&mut self, _: &Right, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::move_right);
    }

    fn up(&mut self, _: &Up, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_away(-1.);
        self.edit(cx, |draft| draft.move_to(offset));
    }

    fn down(&mut self, _: &Down, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_away(1.);
        self.edit(cx, |draft| draft.move_to(offset));
    }

    fn home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_start();
        self.edit(cx, |draft| draft.move_to(offset));
    }

    fn end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_end();
        self.edit(cx, |draft| draft.move_to(offset));
    }

    fn select_left(&mut self, _: &SelectLeft, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::select_left);
    }

    fn select_right(&mut self, _: &SelectRight, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::select_right);
    }

    fn select_up(&mut self, _: &SelectUp, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_away(-1.);
        self.edit(cx, |draft| draft.select_to(offset));
    }

    fn select_down(&mut self, _: &SelectDown, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_away(1.);
        self.edit(cx, |draft| draft.select_to(offset));
    }

    fn select_to_home(&mut self, _: &SelectToHome, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_start();
        self.edit(cx, |draft| draft.select_to(offset));
    }

    fn select_to_end(&mut self, _: &SelectToEnd, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_end();
        self.edit(cx, |draft| draft.select_to(offset));
    }

    fn word_left(&mut self, _: &WordLeft, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::move_word_left);
    }

    fn word_right(&mut self, _: &WordRight, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::move_word_right);
    }

    fn select_word_left(
        &mut self,
        _: &SelectWordLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, Draft::select_word_left);
    }

    fn select_word_right(
        &mut self,
        _: &SelectWordRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, Draft::select_word_right);
    }

    fn text_start(&mut self, _: &TextStart, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |draft| draft.move_to(0));
    }

    fn text_end(&mut self, _: &TextEnd, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |draft| draft.move_to(draft.text().len()));
    }

    fn select_to_text_start(
        &mut self,
        _: &SelectToTextStart,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |draft| draft.select_to(0));
    }

    fn select_to_text_end(
        &mut self,
        _: &SelectToTextEnd,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |draft| draft.select_to(draft.text().len()));
    }

    fn delete_word_backward(
        &mut self,
        _: &DeleteWordBackward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, Draft::delete_word_backward);
    }

    fn delete_word_forward(
        &mut self,
        _: &DeleteWordForward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, Draft::delete_word_forward);
    }

    /// Command+Backspace: takes out the selection, or the row up to the
    /// cursor.
    fn delete_to_home(&mut self, _: &DeleteToHome, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_start();
        self.edit(cx, |draft| draft.delete_to(offset));
    }

    /// Control+K: takes out the selection, or the row from the cursor on.
    fn delete_to_end(&mut self, _: &DeleteToEnd, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_end();
        self.edit(cx, |draft| draft.delete_to(offset));
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_whole(cx);
    }

    /// Selects the whole text, so that typing replaces it.
    pub fn select_whole(&mut self, cx: &mut Context<Self>) {
        self.edit(cx, Draft::select_all);
    }

    fn cut(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        if self.copy_selection(cx) {
            self.edit(cx, |draft| draft.commit(None, ""));
        }
    }

    /// Copies the selection, or with nothing selected leaves the keys to
    /// what holds the input, such as a conversation with text selected.
    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.copy_selection(cx) {
            cx.propagate();
        }
    }

    /// Shows the translation of the text selected, or with nothing
    /// selected leaves the keys to what holds the part.
    fn translate(&mut self, _: &Translate, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.draft.selected_text().to_owned();
        if text.trim().is_empty() {
            cx.propagate();
            return;
        }
        crate::ui::translation::show(&text, window, cx);
    }

    /// A key with Control that no binding took, while nothing is being
    /// composed: the input method's to take, as macSKK's Control-J (see
    /// [`macos`]). While it composes, GPUI hands it every key first.
    #[cfg(target_os = "macos")]
    fn control_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = &event.keystroke.modifiers;
        if !modifiers.control || modifiers.platform || modifiers.function {
            return;
        }
        if self.draft.marked().is_none() && macos::hand_on_key(cx) {
            cx.stop_propagation();
        }
    }

    fn paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            let text = if self.wraps {
                unix_line_breaks(&text)
            } else {
                one_line(&text)
            };
            self.edit(cx, |draft| draft.commit(None, &text));
        }
    }

    /// Puts the selected text on the clipboard, and says whether there was
    /// any.
    fn copy_selection(&self, cx: &mut Context<Self>) -> bool {
        let text = self.draft.selected_text();
        if text.is_empty() {
            return false;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        true
    }

    /// A click puts the cursor there, or with Shift, selects up to there; a
    /// double click selects the word there, and a triple click the line.
    fn mouse_down(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let drag = match event.click_count {
            0 | 1 => {
                let offset = self.offset_at(event.position);
                if event.modifiers.shift {
                    self.edit(cx, |draft| draft.select_to(offset));
                } else {
                    self.edit(cx, |draft| draft.move_to(offset));
                }
                Drag::Characters
            }
            2 => {
                let word = self.draft.word_at(self.char_at(event.position));
                self.edit(cx, |draft| draft.select_spanning(&word, &word));
                Drag::Words(word)
            }
            _ => {
                let line = self.draft.line_at(self.char_at(event.position));
                self.edit(cx, |draft| draft.select_spanning(&line, &line));
                Drag::Lines(line)
            }
        };
        self.drag = Some(drag);
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        match self.drag.clone() {
            None => {}
            Some(Drag::Characters) => {
                let offset = self.offset_at(event.position);
                self.edit(cx, |draft| draft.select_to(offset));
            }
            Some(Drag::Words(word)) => {
                let reached = self.draft.word_at(self.char_at(event.position));
                self.edit(cx, |draft| draft.select_spanning(&word, &reached));
            }
            Some(Drag::Lines(line)) => {
                let reached = self.draft.line_at(self.char_at(event.position));
                self.edit(cx, |draft| draft.select_spanning(&line, &reached));
            }
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        self.drag = None;
    }

    /// Leaves the cursor where it is, with nothing selected.
    pub fn deselect(&mut self, cx: &mut Context<Self>) {
        self.edit(cx, Draft::deselect);
    }

    /// The offset in the text closest to `position` in the window.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(layout)) = (self.last_bounds, self.last_layout.as_ref()) else {
            return 0;
        };
        layout.index_at(position - self.text_origin(bounds))
    }

    /// The offset of the character under `position` in the window, or of
    /// the end of the line, past its end.
    fn char_at(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(layout)) = (self.last_bounds, self.last_layout.as_ref()) else {
            return 0;
        };
        layout.char_at(position - self.text_origin(bounds))
    }

    /// Where the text starts in the window, in an input at `bounds`: left
    /// of and above it once it scrolls.
    fn text_origin(&self, bounds: Bounds<Pixels>) -> Point<Pixels> {
        bounds.origin - self.scroll
    }

    /// The offset `rows` rows below the cursor (above, if negative), as near
    /// as it can be to straight below. Above the first row is the start of
    /// the text, and below the last is its end.
    fn row_away(&self, rows: f32) -> usize {
        let Some(layout) = self.last_layout.as_ref() else {
            return self.draft.cursor();
        };
        let at = layout.position(self.draft.cursor());
        let y = at.y + layout.line_height * (rows + 0.5);
        if y < px(0.) {
            0
        } else if y >= layout.height() {
            self.draft.text().len()
        } else {
            layout.index_at(point(at.x, y))
        }
    }

    /// The start of the cursor's row.
    fn row_start(&self) -> usize {
        self.on_cursor_row(px(0.))
    }

    /// The end of the cursor's row.
    fn row_end(&self) -> usize {
        self.on_cursor_row(px(f32::MAX))
    }

    /// The offset nearest `x` on the cursor's row.
    fn on_cursor_row(&self, x: Pixels) -> usize {
        let Some(layout) = self.last_layout.as_ref() else {
            return self.draft.cursor();
        };
        let at = layout.position(self.draft.cursor());
        layout.index_at(point(x, at.y + layout.line_height / 2.))
    }

    /// The text, and how to shape it: plain, but for the part being
    /// composed, which is underlined.
    fn styled(&self, window: &Window) -> (SharedString, Vec<TextRun>) {
        let text = self.draft.text();
        let style = window.text_style();
        let plain = TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match self.draft.marked() {
            Some(marked) => vec![
                TextRun {
                    len: marked.start,
                    ..plain.clone()
                },
                TextRun {
                    len: marked.len(),
                    underline: Some(UnderlineStyle {
                        color: Some(style.color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..plain.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..plain
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![plain],
        };
        (text.to_owned().into(), runs)
    }
}

impl EventEmitter<Submitted> for LineInput {}

impl EventEmitter<Blurred> for LineInput {}

impl Focusable for LineInput {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for LineInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let (text, adjusted) = self.draft.text_in(&range_utf16);
        *adjusted_range = Some(adjusted);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.draft.range_to_utf16(self.draft.selected()),
            reversed: self.draft.is_reversed(),
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.draft
            .marked()
            .map(|range| self.draft.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, Draft::unmark);
    }

    /// Final text: typed characters, or a composition the input method
    /// committed.
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A key with Control the input method does not take comes back as
        // its control character, which is no text.
        let typed = typed(text);
        if typed.is_empty() && !text.is_empty() {
            return;
        }
        self.edit(cx, |draft| draft.commit(range_utf16.as_ref(), &typed));
    }

    /// Text the input method is still composing.
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |draft| {
            draft.compose(
                range_utf16.as_ref(),
                new_text,
                new_selected_range_utf16.as_ref(),
            );
        });
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.draft.range_from_utf16(&range_utf16);
        let origin = self.text_origin(element_bounds);
        let start = layout.position(range.start);
        let end = layout.position(range.end);
        // On the start's row, even when the range goes on to the next.
        Some(Bounds::from_corners(
            origin + start,
            origin + point(end.x.max(start.x), start.y + layout.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let index = self
            .last_layout
            .as_ref()?
            .index_at(point - self.text_origin(bounds));
        Some(self.draft.offset_to_utf16(index))
    }
}

impl Render for LineInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.left.is_none() {
            self.left = Some(cx.on_blur(&self.focus_handle, window, |_, _, cx| cx.emit(Blurred)));
        }
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_to_home))
            .on_action(cx.listener(Self::select_to_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::text_start))
            .on_action(cx.listener(Self::text_end))
            .on_action(cx.listener(Self::select_to_text_start))
            .on_action(cx.listener(Self::select_to_text_end))
            .on_action(cx.listener(Self::delete_word_backward))
            .on_action(cx.listener(Self::delete_word_forward))
            .on_action(cx.listener(Self::delete_to_home))
            .on_action(cx.listener(Self::delete_to_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::translate))
            .on_action(cx.listener(Self::paste))
            .map(|input| {
                #[cfg(target_os = "macos")]
                let input = input.on_key_down(cx.listener(Self::control_key));
                input
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|input, _, window, cx| {
                    let text = input.draft.selected_text().to_owned();
                    if crate::ui::translation::offer(&text, None, window, cx) {
                        cx.stop_propagation();
                    }
                }),
            )
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .cursor_text()
            .w_full()
            .child(LineElement { input: cx.entity() })
            .child(crate::vim::hints::spot(crate::vim::hints::Take::Field(
                self.focus_handle.clone(),
            )))
    }
}

/// What of `text`, as the platform hands it in place of a key, is typed:
/// all but its control characters, apart from tabs.
fn typed(text: &str) -> String {
    text.chars()
        .filter(|&character| character == '\t' || !character.is_control())
        .collect()
}

/// How far to scroll a view `view` long over content `content` long, so
/// that the part `shown` of the content is in view. It moves no more than
/// it has to from `scroll`, and never so far as to leave room after the end
/// of the content.
fn scroll_to_show(scroll: Pixels, shown: Range<Pixels>, content: Pixels, view: Pixels) -> Pixels {
    let scroll = scroll.min(shown.start).max(shown.end - view);
    scroll.min(content - view).max(px(0.))
}

const CURSOR_WIDTH: Pixels = px(1.);

/// The text's size, and the height of its lines, in the text style the
/// window is drawing with.
fn text_size(window: &Window) -> (Pixels, Pixels) {
    let font_size = window.text_style().font_size.to_pixels(window.rem_size());
    (font_size, window.line_height())
}

/// Shapes `text` for the input, at `size` from [`text_size`], wrapping it
/// at `wrap_width` if any.
fn shape(
    text: SharedString,
    runs: &[TextRun],
    (font_size, line_height): (Pixels, Pixels),
    wrap_width: Option<Pixels>,
    window: &Window,
) -> Layout {
    let lines = window
        .text_system()
        .shape_text(text, font_size, runs, wrap_width, None)
        .map(|lines| lines.into_iter().collect())
        .unwrap_or_default();
    Layout { lines, line_height }
}

/// The text as shaped for the input: each line between line breaks, with
/// the rows it wraps into where the input wraps. Positions are from the
/// text's top left corner.
struct Layout {
    lines: Vec<WrappedLine>,
    line_height: Pixels,
}

impl Layout {
    /// Each line, with the offset it starts at and the top of its first
    /// row.
    fn placed(&self) -> impl Iterator<Item = (usize, Pixels, &WrappedLine)> {
        let mut start = 0;
        let mut top = px(0.);
        self.lines.iter().map(move |line| {
            let placed = (start, top, line);
            start += line.len() + 1;
            top += rows_height(self.line_height, rows(line));
            placed
        })
    }

    fn rows(&self) -> usize {
        self.lines.iter().map(rows).sum::<usize>().max(1)
    }

    fn height(&self) -> Pixels {
        rows_height(self.line_height, self.rows())
    }

    fn width(&self) -> Pixels {
        self.lines
            .iter()
            .map(|line| line.width())
            .fold(px(0.), Pixels::max)
    }

    /// Where offset `index` is. At the end of a row that wraps, that is
    /// the end of the row rather than the start of the next.
    fn position(&self, index: usize) -> Point<Pixels> {
        self.placed()
            .find(|&(start, _, line)| index <= start + line.len())
            .and_then(|(start, top, line)| {
                let at = line.position_for_index(index - start, self.line_height)?;
                Some(at + point(px(0.), top))
            })
            .unwrap_or_default()
    }

    /// The offset nearest `position`.
    fn index_at(&self, position: Point<Pixels>) -> usize {
        self.hit(position, true)
    }

    /// The offset of the character under `position`, or of the end of its
    /// line, past the line's end.
    fn char_at(&self, position: Point<Pixels>) -> usize {
        self.hit(position, false)
    }

    /// The offset `position` is at: the nearest, if `closest`, or else that
    /// of the character under it.
    fn hit(&self, position: Point<Pixels>, closest: bool) -> usize {
        let last = self.lines.len().saturating_sub(1);
        for (n, (start, top, line)) in self.placed().enumerate() {
            let height = rows_height(self.line_height, rows(line));
            if position.y < top + height || n == last {
                // Within the line's own rows, where its lookup finds them.
                let y = (position.y - top)
                    .max(px(0.))
                    .min(height - self.line_height / 2.);
                let at = point(position.x.max(px(0.)), y);
                let (Ok(index) | Err(index)) = if closest {
                    line.closest_index_for_position(at, self.line_height)
                } else {
                    line.index_for_position(at, self.line_height)
                };
                return start + index;
            }
        }
        0
    }

    fn paint(&self, origin: Point<Pixels>, window: &mut Window, cx: &mut App) {
        for (_, top, line) in self.placed() {
            line.paint(
                origin + point(px(0.), top),
                self.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .expect("the text was shaped for this window");
        }
    }

    /// The boxes that show `selected`, one per row it covers, for text at
    /// `origin` in an input `width` wide.
    fn selection(
        &self,
        selected: &Range<usize>,
        origin: Point<Pixels>,
        width: Pixels,
    ) -> Vec<PaintQuad> {
        let mut boxes = Vec::new();
        if selected.is_empty() {
            return boxes;
        }
        let start = self.position(selected.start);
        let end = self.position(selected.end);
        let (mut left, mut top) = (start.x, start.y);
        loop {
            let last = top + self.line_height / 2. > end.y;
            let right = if last { end.x } else { width.max(left) };
            boxes.push(fill(
                Bounds::from_corners(
                    origin + point(left, top),
                    origin + point(right, top + self.line_height),
                ),
                rgb(SELECTION),
            ));
            if last {
                return boxes;
            }
            left = px(0.);
            top += self.line_height;
        }
    }
}

/// How many rows `line` wraps into.
fn rows(line: &WrappedLine) -> usize {
    line.wrap_boundaries().len() + 1
}

#[expect(
    clippy::cast_precision_loss,
    reason = "row counts are far below what f32 keeps exactly"
)]
fn rows_height(line_height: Pixels, rows: usize) -> Pixels {
    line_height * rows as f32
}

/// Draws the input: the text, the composed part underlined, the
/// selection, and the cursor.
struct LineElement {
    input: Entity<LineInput>,
}

/// What `prepaint` shaped, taken by `paint` of the same frame.
struct Prepainted {
    layout: Option<Layout>,
    /// Where the text starts, left of and above the input once it scrolls.
    origin: Point<Pixels>,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    /// The placeholder, shaped, while the input is empty.
    placeholder: Option<Layout>,
}

impl IntoElement for LineElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LineElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let input = self.input.read(cx);
        if !input.wraps {
            style.size.height = window.line_height().into();
            return (window.request_layout(style, [], cx), ());
        }
        // As many rows as the text wraps into at the width the input gets.
        // The text style is the input's only now, not when GPUI measures.
        let (text, runs) = input.styled(window);
        let text_size = text_size(window);
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                _ => None,
            });
            let layout = shape(
                text.clone(),
                &runs,
                text_size,
                width.map(|width| width - CURSOR_WIDTH),
                window,
            );
            size(
                width.unwrap_or_default(),
                rows_height(layout.line_height, layout.rows().min(MAX_ROWS)),
            )
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let (text, runs) = input.styled(window);
        let wraps = input.wraps;
        let selected = input.draft.selected().clone();
        let cursor = input.draft.cursor();
        let scroll = input.scroll;
        let placeholder =
            (input.draft.text().is_empty() && !input.placeholder.is_empty()).then(|| {
                let placeholder = input.placeholder.clone();
                let run = TextRun {
                    len: placeholder.len(),
                    font: window.text_style().font(),
                    color: rgb(FAINT).into(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                // On one row: what does not fit is cut off.
                shape(placeholder, &[run], text_size(window), None, window)
            });
        let layout = shape(
            text,
            &runs,
            text_size(window),
            wraps.then(|| bounds.size.width - CURSOR_WIDTH),
            window,
        );
        // Moved as far as it takes for the cursor to show.
        let at = layout.position(cursor);
        let scroll = point(
            if wraps {
                px(0.)
            } else {
                scroll_to_show(
                    scroll.x,
                    at.x..at.x + CURSOR_WIDTH,
                    layout.width() + CURSOR_WIDTH,
                    bounds.size.width,
                )
            },
            scroll_to_show(
                scroll.y,
                at.y..at.y + layout.line_height,
                layout.height(),
                bounds.size.height,
            ),
        );
        self.input.update(cx, |input, _| input.scroll = scroll);
        let origin = bounds.origin - scroll;
        let selection = layout.selection(&selected, origin, bounds.size.width);
        let cursor = fill(
            Bounds::new(origin + at, size(CURSOR_WIDTH, layout.line_height)),
            rgb(FOREGROUND),
        );
        Prepainted {
            layout: Some(layout),
            origin,
            selection,
            cursor: Some(cursor),
            placeholder,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepainted: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let layout = prepainted.layout.take().expect("prepaint shaped the text");
        // What scrolled out of the input is not drawn.
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for selection in prepainted.selection.drain(..) {
                window.paint_quad(selection);
            }
            layout.paint(prepainted.origin, window, cx);
            if let Some(placeholder) = prepainted.placeholder.take() {
                placeholder.paint(prepainted.origin, window, cx);
            }
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepainted.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        self.input.update(cx, |input, _| {
            input.last_layout = Some(layout);
            input.last_bounds = Some(bounds);
        });
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{scroll_to_show, typed};

    #[test]
    fn a_control_key_handed_back_types_nothing() {
        assert_eq!(typed("\n"), "");
        assert_eq!(typed("\u{18}"), "");
    }

    #[test]
    fn typed_text_and_tabs_stay() {
        assert_eq!(typed("かな\tkana"), "かな\tkana");
    }

    #[test]
    fn content_that_fits_does_not_scroll() {
        assert_eq!(
            scroll_to_show(px(0.), px(50.)..px(51.), px(81.), px(100.)),
            px(0.)
        );
    }

    #[test]
    fn content_scrolls_to_show_what_is_past_the_end_of_the_view() {
        // The cursor at the end of a line 30 pixels longer than the input.
        assert_eq!(
            scroll_to_show(px(0.), px(130.)..px(131.), px(131.), px(100.)),
            px(31.)
        );
    }

    #[test]
    fn content_stays_put_while_what_to_show_is_in_view() {
        assert_eq!(
            scroll_to_show(px(31.), px(60.)..px(61.), px(131.), px(100.)),
            px(31.)
        );
    }

    #[test]
    fn content_scrolls_back_to_show_what_is_before_the_view() {
        assert_eq!(
            scroll_to_show(px(31.), px(10.)..px(11.), px(131.), px(100.)),
            px(10.)
        );
    }

    #[test]
    fn content_that_got_shorter_scrolls_back_to_its_end() {
        // Deleted down to 110 pixels, the cursor at the end.
        assert_eq!(
            scroll_to_show(px(31.), px(110.)..px(111.), px(111.), px(100.)),
            px(11.)
        );
        // Deleted until it fits.
        assert_eq!(
            scroll_to_show(px(31.), px(90.)..px(91.), px(91.), px(100.)),
            px(0.)
        );
    }
}
