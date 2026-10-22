//! A one-line text input that an input method can compose into.
//!
//! GPUI hands typed and composed text to an [`EntityInputHandler`]: typed
//! characters and committed compositions arrive as replacements, and text
//! the input method is still composing arrives as marked text, which the
//! input shows underlined. Offsets in that trait count UTF-16 units, as the
//! platforms do, while `content` is indexed in bytes.
//!
//! The human selects with the mouse or with Shift and the arrows, and cuts,
//! copies and pastes with the usual keys. The input holds one line, so a
//! pasted line break becomes a space.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, InspectorElementId,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ShapedLine, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions,
    div, fill, point, prelude::*, px, relative, rgb, rgba, size,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    line_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Home,
        End,
        SelectLeft,
        SelectRight,
        SelectToHome,
        SelectToEnd,
        SelectAll,
        Cut,
        Copy,
        Paste,
        Submit,
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
        KeyBinding::new("home", Home, Some(KEY_CONTEXT)),
        KeyBinding::new("end", End, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-home", SelectToHome, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-end", SelectToEnd, Some(KEY_CONTEXT)),
        // The secondary key is Command on macOS, Control elsewhere.
        KeyBinding::new("secondary-a", SelectAll, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-x", Cut, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-v", Paste, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", Submit, Some(KEY_CONTEXT)),
    ]);
}

/// The human pressed Enter with no composition in progress.
pub struct Submitted;

pub struct LineInput {
    focus_handle: FocusHandle,
    content: String,
    /// Byte range of the selection; empty when it is just the cursor.
    selected_range: Range<usize>,
    /// Whether the cursor is at the start of the selection rather than its
    /// end: the selection grew to the left.
    selection_reversed: bool,
    /// Whether a mouse button went down on the line and is still held.
    selecting: bool,
    /// Byte range the input method is composing, shown underlined.
    marked_range: Option<Range<usize>>,
    /// What the last paint drew, and where, for placing the input method's
    /// candidate window and finding the text under the mouse.
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    /// How far the text is moved to the left, so that the cursor of a line
    /// longer than the field stays in view.
    scroll: Pixels,
}

impl LineInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            selected_range: 0..0,
            selection_reversed: false,
            selecting: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll: px(0.),
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.content
    }

    /// Whether an input method is composing text.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.marked_range.is_some()
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_text("", cx);
    }

    /// Puts `text` in the line, with the cursor at its end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content.clear();
        self.content.push_str(text);
        self.move_to(self.content.len(), cx);
        self.marked_range = None;
    }

    fn submit(&mut self, _: &Submit, _window: &mut Window, cx: &mut Context<Self>) {
        // Enter while composing belongs to the input method.
        if self.marked_range.is_none() {
            cx.emit(Submitted);
        }
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let start = self.previous_boundary(self.selected_range.start);
            self.selected_range = start..self.selected_range.end;
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let end = self.next_boundary(self.selected_range.end);
            self.selected_range = self.selected_range.start..end;
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn left(&mut self, _: &Left, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            self.previous_boundary(self.selected_range.start)
        } else {
            self.selected_range.start
        };
        self.move_to(offset, cx);
    }

    fn right(&mut self, _: &Right, _window: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            self.next_boundary(self.selected_range.end)
        } else {
            self.selected_range.end
        };
        self.move_to(offset, cx);
    }

    fn home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx);
    }

    fn select_to_home(&mut self, _: &SelectToHome, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_to_end(&mut self, _: &SelectToEnd, _window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.copy_selection(cx) {
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &one_line(&text), window, cx);
        }
    }

    /// Puts the selected text on the clipboard, and says whether there was
    /// any.
    fn copy_selection(&self, cx: &mut Context<Self>) -> bool {
        if self.selected_range.is_empty() {
            return false;
        }
        let text = self.content[self.selected_range.clone()].to_owned();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.selecting = true;
        let offset = self.offset_at(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.select_to(self.offset_at(event.position), cx);
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        self.selecting = false;
    }

    /// Where the cursor is: the end of the selection it moves.
    fn cursor(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    /// Moves the cursor to `offset`, keeping the other end of the selection
    /// where it is.
    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let anchor = if self.selection_reversed {
            self.selected_range.end
        } else {
            self.selected_range.start
        };
        self.selection_reversed = offset < anchor;
        self.selected_range = anchor.min(offset)..anchor.max(offset);
        cx.notify();
    }

    /// The offset in the text closest to `position` in the window.
    fn offset_at(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (self.last_bounds, self.last_layout.as_ref()) else {
            return 0;
        };
        line.closest_index_for_x(position.x - self.text_left(bounds))
    }

    /// Where the text starts in the window, in a field at `bounds`: left
    /// of the field once it scrolls.
    fn text_left(&self, bounds: Bounds<Pixels>) -> Pixels {
        bounds.left() - self.scroll
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .map(|(index, _)| index)
            .find(|&index| index < offset)
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .find(|&index| index > offset)
            .unwrap_or(self.content.len())
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf16 = 0;
        let mut bytes = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            bytes += ch.len_utf8();
        }
        bytes
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content[..offset].encode_utf16().count()
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    /// The range a replacement applies to: the one given, else the text
    /// being composed, else the selection.
    fn target_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone())
    }

    /// Puts `text` in place of `range`, and returns where it ended up.
    fn replace(&mut self, range: Range<usize>, text: &str) -> Range<usize> {
        self.content.replace_range(range.clone(), text);
        range.start..range.start + text.len()
    }
}

impl EventEmitter<Submitted> for LineInput {}

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
        let range = self.range_from_utf16(&range_utf16);
        *adjusted_range = Some(self.range_to_utf16(&range));
        Some(self.content[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        cx.notify();
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
        let range = self.target_range(range_utf16.as_ref());
        let placed = self.replace(range, text);
        self.move_to(placed.end, cx);
        self.marked_range = None;
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
        let range = self.target_range(range_utf16.as_ref());
        let placed = self.replace(range, new_text);
        self.marked_range = (!new_text.is_empty()).then(|| placed.clone());
        // The selection the input method gives is relative to the new text.
        self.selected_range = match new_selected_range_utf16 {
            Some(selected) => {
                let offset = self.offset_to_utf16(placed.start);
                self.range_from_utf16(&(selected.start + offset..selected.end + offset))
            }
            None => placed.end..placed.end,
        };
        self.selection_reversed = false;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let left = self.text_left(element_bounds);
        Some(Bounds::from_corners(
            point(left + layout.x_for_index(range.start), element_bounds.top()),
            point(
                left + layout.x_for_index(range.end),
                element_bounds.bottom(),
            ),
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
            .index_for_x(point.x - self.text_left(bounds))?;
        Some(self.offset_to_utf16(index))
    }
}

impl Render for LineInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_to_home))
            .on_action(cx.listener(Self::select_to_end))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .w_full()
            .child(LineElement { input: cx.entity() })
    }
}

/// A pasted text as one line: each line break, of any kind, becomes a
/// space.
fn one_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// How far to move the text left in a field `width` wide, so that the
/// cursor, `cursor` from the start of a line `line_width` long, shows. It
/// moves no more than it has to from `scroll`, and never so far as to
/// leave room after the end of the line.
fn scroll_to_show(scroll: Pixels, cursor: Pixels, line_width: Pixels, width: Pixels) -> Pixels {
    // Room for the cursor itself after the last character.
    let width = width - CURSOR_WIDTH;
    let scroll = scroll.min(cursor).max(cursor - width);
    scroll.min(line_width - width).max(px(0.))
}

const CURSOR_WIDTH: Pixels = px(1.);

/// Draws the line: the text, the composed part underlined, the selection,
/// and the cursor.
struct LineElement {
    input: Entity<LineInput>,
}

/// What `prepaint` shaped, taken by `paint` of the same frame.
struct Prepainted {
    line: Option<ShapedLine>,
    /// Where the text starts, left of the field once it scrolls.
    left: Pixels,
    selection: Option<PaintQuad>,
    cursor: Option<PaintQuad>,
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
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
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
        let text = input.content.clone();
        let scroll = input.scroll;
        let selected = input.selected_range.clone();
        let cursor = input.cursor();
        let style = window.text_style();
        let plain = TextRun {
            len: text.len(),
            font: style.font(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match &input.marked_range {
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
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(text.into(), font_size, &runs, None);
        let scroll = scroll_to_show(
            scroll,
            line.x_for_index(cursor),
            line.width,
            bounds.size.width,
        );
        self.input.update(cx, |input, _| input.scroll = scroll);
        let left = bounds.left() - scroll;
        let selection = (!selected.is_empty()).then(|| {
            fill(
                Bounds::from_corners(
                    point(left + line.x_for_index(selected.start), bounds.top()),
                    point(left + line.x_for_index(selected.end), bounds.bottom()),
                ),
                rgba(0x4080_ff60),
            )
        });
        let cursor = fill(
            Bounds::new(
                point(left + line.x_for_index(cursor), bounds.top()),
                size(CURSOR_WIDTH, bounds.size.height),
            ),
            rgb(0xe0_e0e0),
        );
        Prepainted {
            line: Some(line),
            left,
            selection,
            cursor: Some(cursor),
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
        let line = prepainted.line.take().expect("prepaint shaped the line");
        // What scrolled out of the field is not drawn.
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = prepainted.selection.take() {
                window.paint_quad(selection);
            }
            line.paint(
                point(prepainted.left, bounds.top()),
                window.line_height(),
                window,
                cx,
            )
            .expect("the line was shaped for this window");
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepainted.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::scroll_to_show;

    #[test]
    fn a_line_that_fits_does_not_scroll() {
        assert_eq!(scroll_to_show(px(0.), px(50.), px(80.), px(100.)), px(0.));
    }

    #[test]
    fn the_line_scrolls_to_keep_the_cursor_at_the_right_edge() {
        // The cursor at the end of a line 30 pixels longer than the field.
        assert_eq!(
            scroll_to_show(px(0.), px(130.), px(130.), px(100.)),
            px(31.)
        );
    }

    #[test]
    fn the_line_stays_put_while_the_cursor_is_in_view() {
        assert_eq!(
            scroll_to_show(px(31.), px(60.), px(130.), px(100.)),
            px(31.)
        );
    }

    #[test]
    fn the_line_scrolls_back_to_show_a_cursor_left_of_the_field() {
        assert_eq!(
            scroll_to_show(px(31.), px(10.), px(130.), px(100.)),
            px(10.)
        );
    }

    #[test]
    fn a_line_that_got_shorter_scrolls_back_to_its_end() {
        // Deleted down to 110 pixels, the cursor at the end.
        assert_eq!(
            scroll_to_show(px(31.), px(110.), px(110.), px(100.)),
            px(11.)
        );
        // Deleted until it fits.
        assert_eq!(scroll_to_show(px(31.), px(90.), px(90.), px(100.)), px(0.));
    }
}
