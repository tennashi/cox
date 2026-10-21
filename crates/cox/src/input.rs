//! A one-line text input that an input method can compose into.
//!
//! GPUI hands typed and composed text to an [`EntityInputHandler`]: typed
//! characters and committed compositions arrive as replacements, and text
//! the input method is still composing arrives as marked text, which the
//! input shows underlined. Offsets in that trait count UTF-16 units, as the
//! platforms do, while `content` is indexed in bytes.

use std::ops::Range;

use gpui::{
    App, Bounds, Context, ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, GlobalElementId, InspectorElementId, KeyBinding, LayoutId, PaintQuad,
    Pixels, ShapedLine, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill,
    point, prelude::*, px, relative, rgb, size,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    line_input,
    [Backspace, Delete, Left, Right, Home, End, Submit]
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
    /// Byte range the input method is composing, shown underlined.
    marked_range: Option<Range<usize>>,
    /// What the last paint drew, for placing the input method's candidate
    /// window.
    last_layout: Option<ShapedLine>,
}

impl LineInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: String::new(),
            selected_range: 0..0,
            marked_range: None,
            last_layout: None,
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
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        cx.notify();
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

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify();
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
            reversed: false,
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
        self.selected_range = placed.end..placed.end;
        self.marked_range = None;
        cx.notify();
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
        Some(Bounds::from_corners(
            point(
                element_bounds.left() + layout.x_for_index(range.start),
                element_bounds.top(),
            ),
            point(
                element_bounds.left() + layout.x_for_index(range.end),
                element_bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
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
            .w_full()
            .child(LineElement { input: cx.entity() })
    }
}

/// Draws the line: the text, the composed part underlined, and the cursor.
struct LineElement {
    input: Entity<LineInput>,
}

/// What `prepaint` shaped, taken by `paint` of the same frame.
struct Prepainted {
    line: Option<ShapedLine>,
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
        let cursor = input.selected_range.end;
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
        let cursor = fill(
            Bounds::new(
                point(bounds.left() + line.x_for_index(cursor), bounds.top()),
                size(px(1.), bounds.size.height),
            ),
            rgb(0xe0_e0e0),
        );
        Prepainted {
            line: Some(line),
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
        line.paint(bounds.origin, window.line_height(), window, cx)
            .expect("the line was shaped for this window");
        if focus_handle.is_focused(window)
            && let Some(cursor) = prepainted.cursor.take()
        {
            window.paint_quad(cursor);
        }
        self.input
            .update(cx, |input, _| input.last_layout = Some(line));
    }
}
