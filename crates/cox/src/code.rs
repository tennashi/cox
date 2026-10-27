//! A tab of the stage that shows a file as it is written, such as code,
//! for the human to read beside the conversation. The agent shows it with
//! Cox's tool for that (see [`crate::mcp`]), at the lines it talks about,
//! which the tab marks and scrolls to; a click on a link to a file opens
//! it too.
//!
//! The tab draws only the lines in sight, so that a long file scrolls as a
//! short one does, each with its number beside it. It follows the file as
//! it changes (see [`crate::follow`]). The wheel scrolls it down and
//! across, and a drag selects text to copy: a double click selects a
//! word, and a triple click a line.

use std::path::{Path, PathBuf};

use cox_core::tabs::Lines;
use cox_editor::buffer::{Buffer, Point as Place};
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, Element, ElementId, Entity, FocusHandle,
    Focusable, GlobalElementId, Hsla, InspectorElementId, IntoElement, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent,
    ShapedLine, SharedString, Style, Task, TextRun, Window, div, fill, font, point, prelude::*, px,
    relative, rgb, size,
};

use crate::follow::{self, Follows, Stamp};
use crate::view::{Copy, FONT};
use crate::workspace::shortened;

const KEY_CONTEXT: &str = "Code";

const FONT_SIZE: f32 = 13.;
const LINE_HEIGHT: f32 = 1.4;

/// Room between the text and the tab's edges.
const PADDING: f32 = 8.;

/// How many columns apart the stops of a tab character are.
const TAB_STOPS: usize = 4;

/// The most of a line the tab draws, in bytes, so that a line as long as a
/// minified file's does not hold up every frame.
const LINE_LENGTH: usize = 4096;

/// How many lines a turn of the wheel scrolls.
const WHEEL_LINES: f32 = 3.;

/// Where the lines the tab is shown at go, from the top, as a part of its
/// height.
const REVEAL_AT: f32 = 1. / 3.;

const FOREGROUND: u32 = 0xd4_d4d4;
const BACKGROUND: u32 = 0x1e_1e1e;
const NUMBER: u32 = 0x6e_6e6e;
const NUMBER_MARKED: u32 = 0xd0_d0d0;
const MARKED: u32 = 0x33_3322;
const SELECTION: u32 = 0x3a_4a6a;
const DIM: u32 = 0xa0_a0a0;
const FAILED: u32 = 0xf0_8080;

/// Binds the keys of the tab.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT))]);
}

pub struct Code {
    /// The file, as an absolute path.
    path: PathBuf,
    buffer: Buffer,
    /// How many characters the longest line has, which says how far the
    /// tab scrolls across.
    longest: usize,
    /// The file's, as the tab last read it, or `None` until the tab looks
    /// at the file.
    stamp: Option<Stamp>,
    /// Why the file could not be read the last time the tab looked, if it
    /// could not.
    unread: Option<String>,
    /// The lines the tab was last shown at, if any.
    marked: Option<Lines>,
    /// The line, from 0, to bring into sight once the tab is laid out.
    reveal: Option<usize>,
    /// How far the text is scrolled across and down.
    scroll: Point<Pixels>,
    /// Where the selection started, and where it goes to, if there is one.
    selection: Option<(Place, Place)>,
    /// Whether a drag is selecting.
    selecting: bool,
    /// How the last frame laid the text out.
    laid: Option<Laid>,
    focus_handle: FocusHandle,
    _follow: Task<()>,
}

/// How a frame laid the text out, to find what is under the mouse.
struct Laid {
    /// Where the text shows.
    text: Bounds<Pixels>,
    line_height: Pixels,
    cell_width: Pixels,
    /// The lines in sight, in order.
    rows: Vec<Row>,
}

/// A line in sight, as drawn.
struct Row {
    line: usize,
    /// The line's text, at its top left.
    origin: Point<Pixels>,
    shaped: ShapedLine,
    shown: Shown,
}

impl Code {
    /// A tab showing the file at `path`, an absolute path, which says
    /// `text`, at `lines` if any.
    pub fn new(path: PathBuf, text: &str, lines: Option<Lines>, cx: &mut Context<Self>) -> Self {
        let buffer = Buffer::new(text);
        Self {
            longest: buffer.longest_line(),
            buffer,
            path,
            stamp: None,
            unread: None,
            marked: lines,
            reveal: lines.map(|lines| lines.first - 1),
            scroll: Point::default(),
            selection: None,
            selecting: false,
            laid: None,
            focus_handle: cx.focus_handle(),
            _follow: follow::follow(cx),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The name the tab goes by: the file's.
    #[must_use]
    pub fn title(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// The text shown.
    #[must_use]
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// The lines the tab was last shown at, if any.
    #[must_use]
    pub fn marked(&self) -> Option<Lines> {
        self.marked
    }

    /// The lines in sight, from 0, as the last frame drew them.
    #[must_use]
    pub fn in_sight(&self) -> std::ops::Range<usize> {
        let rows = self.laid.as_ref().map_or(&[][..], |laid| &laid.rows[..]);
        match (rows.first(), rows.last()) {
            (Some(first), Some(last)) => first.line..last.line + 1,
            _ => 0..0,
        }
    }

    /// The text selected.
    #[must_use]
    pub fn selected_text(&self) -> String {
        match self.selection {
            Some((anchor, head)) => self.buffer.slice(anchor.min(head)..anchor.max(head)),
            None => String::new(),
        }
    }

    /// Shows `text`, which the file says now, at `lines` if any, which the
    /// tab scrolls to.
    pub fn show(&mut self, text: &str, lines: Option<Lines>, cx: &mut Context<Self>) {
        // The tab looks at the file again, which may have changed since.
        self.stamp = None;
        self.unread = None;
        self.set_text(text);
        self.marked = lines;
        self.reveal = lines.map(|lines| lines.first - 1);
        cx.notify();
    }

    fn set_text(&mut self, text: &str) {
        if text != self.buffer.text() {
            self.buffer = Buffer::new(text);
            self.longest = self.buffer.longest_line();
            // What was selected may be gone.
            self.selection = None;
            self.selecting = false;
        }
    }

    /// The place in the text under `position`: on the nearest line in
    /// sight, at the nearest character.
    fn place_at(&self, position: Point<Pixels>) -> Option<Place> {
        let laid = self.laid.as_ref()?;
        let first = laid.rows.first()?;
        let last = laid.rows.last()?;
        let row = if position.y < first.origin.y {
            first
        } else {
            laid.rows
                .iter()
                .find(|row| position.y < row.origin.y + laid.line_height)
                .unwrap_or(last)
        };
        let index = row.shaped.closest_index_for_x(position.x - row.origin.x);
        Some(Place::new(row.line, row.shown.column_of(index)))
    }

    /// How far the text scrolls, at most, laid out as `laid`.
    fn most_scroll(&self, laid: &Laid) -> Point<Pixels> {
        #[expect(
            clippy::cast_precision_loss,
            reason = "line and character counts are far below what f32 keeps exactly"
        )]
        let (lines, longest) = (self.buffer.lines() as f32, self.longest as f32);
        // The last line scrolls up to the middle.
        let down = laid.line_height * lines + px(PADDING) - laid.text.size.height / 2.;
        let across = laid.cell_width * longest + px(PADDING * 2.) - laid.text.size.width;
        point(across.max(px(0.)), down.max(px(0.)))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let Some(place) = self.place_at(event.position) else {
            return;
        };
        let line = self.buffer.line(place.line);
        self.selection = Some(match event.click_count {
            1 => (place, place),
            2 => {
                let word = word_around(&line, place.column);
                (
                    Place::new(place.line, word.start),
                    Place::new(place.line, word.end),
                )
            }
            _ => (Place::new(place.line, 0), Place::new(place.line + 1, 0)),
        });
        self.selecting = event.click_count == 1;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some(place) = self.place_at(event.position)
            && let Some((_, head)) = &mut self.selection
            && *head != place
        {
            *head = place;
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        // A click with no drag selects nothing.
        if self.selection.is_some_and(|(anchor, head)| anchor == head) {
            self.selection = None;
            cx.notify();
        }
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(laid) = &self.laid else {
            return;
        };
        let delta = event.delta.pixel_delta(laid.line_height * WHEEL_LINES);
        let most = self.most_scroll(laid);
        let scroll = point(
            (self.scroll.x - delta.x).clamp(px(0.), most.x),
            (self.scroll.y - delta.y).clamp(px(0.), most.y),
        );
        if scroll != self.scroll {
            self.scroll = scroll;
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn render_bar(&self) -> impl IntoElement {
        let lines = self.marked.map(|lines| {
            if lines.first == lines.last {
                format!("line {}", lines.first)
            } else {
                format!("lines {}–{}", lines.first, lines.last)
            }
        });
        div()
            .flex()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(0x40_4040))
            .text_color(rgb(DIM))
            .child(
                div()
                    .debug_selector(|| "code-path".to_owned())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(shortened(&self.path)),
            )
            .children(lines.map(|lines| div().flex_shrink_0().child(lines)))
            .children(self.unread.as_ref().map(|error| {
                div()
                    .flex_shrink_0()
                    .text_color(rgb(FAILED))
                    .child(format!("cannot read: {error}"))
            }))
    }
}

/// The bytes of the word around `column` of `line`: letters, digits and
/// `_`, or else the one character there.
fn word_around(line: &str, column: usize) -> std::ops::Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
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

impl Follows for Code {
    fn followed(&self) -> (PathBuf, Option<Stamp>) {
        (self.path.clone(), self.stamp)
    }

    fn looked(&mut self, looked: std::io::Result<(Stamp, String)>, cx: &mut Context<Self>) {
        match looked {
            Ok((stamp, text)) => {
                self.stamp = Some(stamp);
                self.unread = None;
                self.set_text(&text);
            }
            Err(error) => self.unread = Some(error.to_string()),
        }
        cx.notify();
    }
}

impl Focusable for Code {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Code {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_bar())
            .child(
                div()
                    .id("code")
                    .debug_selector(|| "code".to_owned())
                    .flex_1()
                    .min_h_0()
                    .bg(rgb(BACKGROUND))
                    .track_focus(&self.focus_handle)
                    .key_context(KEY_CONTEXT)
                    .on_action(cx.listener(Self::copy))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|code, _, _, _| code.selecting = false),
                    )
                    .on_scroll_wheel(cx.listener(Self::scroll))
                    .child(Text { code: cx.entity() }),
            )
    }
}

/// A line as it is drawn: with its tab characters as spaces to the next
/// stop, and cut at [`LINE_LENGTH`].
struct Shown {
    text: String,
    /// Where each character of the line starts, in the text drawn and in
    /// the line, in order.
    starts: Vec<(usize, usize)>,
    /// How long the line is, in bytes.
    len: usize,
}

impl Shown {
    fn of(line: &str) -> Self {
        let mut text = String::with_capacity(line.len().min(LINE_LENGTH));
        let mut starts = Vec::with_capacity(line.len().min(LINE_LENGTH));
        let mut columns = 0;
        for (at, c) in line.char_indices() {
            if text.len() >= LINE_LENGTH {
                break;
            }
            starts.push((text.len(), at));
            if c == '\t' {
                let spaces = TAB_STOPS - columns % TAB_STOPS;
                text.extend(std::iter::repeat_n(' ', spaces));
                columns += spaces;
            } else {
                text.push(c);
                columns += 1;
            }
        }
        Self {
            text,
            starts,
            len: line.len(),
        }
    }

    /// The byte of the line that `index` of the text drawn is at.
    fn column_of(&self, index: usize) -> usize {
        match self.starts.iter().rposition(|&(shown, _)| shown <= index) {
            Some(at) if self.starts[at].0 == index => self.starts[at].1,
            // Past the last character drawn, or within a tab's spaces,
            // which go to the next character.
            Some(at) => self.starts.get(at + 1).map_or(self.len, |&(_, line)| line),
            None => 0,
        }
    }

    /// The index of the text drawn that `column` of the line is at.
    fn index_of(&self, column: usize) -> usize {
        self.starts
            .iter()
            .find(|&&(_, line)| line >= column)
            .map_or(self.text.len(), |&(shown, _)| shown)
    }
}

/// Draws the lines in sight.
struct Text {
    code: Entity<Code>,
}

/// What a frame paints.
#[derive(Default)]
struct Painted {
    /// The marked lines', under everything.
    marked: Vec<Bounds<Pixels>>,
    /// Under the text, within it.
    selection: Vec<Bounds<Pixels>>,
    /// The lines in sight, at their top left.
    text: Vec<(Point<Pixels>, ShapedLine)>,
    /// Their numbers, at their top left.
    numbers: Vec<(Point<Pixels>, ShapedLine)>,
    /// Where the text shows, which it is cut to.
    text_bounds: Bounds<Pixels>,
    line_height: Pixels,
}

impl Text {
    /// Works out what to paint in `bounds`, and keeps how it laid the text
    /// out.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "line counts are far below what f32 keeps exactly, and the \
                  lines in sight are counted from a scroll that is not negative"
    )]
    fn lay_out(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Painted {
        let font = font(FONT);
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system();
        let cell_width = text_system
            .advance(text_system.resolve_font(&font), font_size, 'm')
            .map_or(px(8.), |advance| advance.width);
        let line_height = (font_size * LINE_HEIGHT).round();
        let shape = |text: &str, color: u32, window: &Window| {
            window.text_system().shape_line(
                SharedString::from(text.to_owned()),
                font_size,
                &[TextRun {
                    len: text.len(),
                    font: font.clone(),
                    color: Hsla::from(rgb(color)),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
        };

        let code = self.code.read(cx);
        let lines = code.buffer.lines();
        let digits = lines.to_string().len();
        let gutter = cell_width * digits as f32 + px(PADDING * 2.);
        let text_bounds = Bounds::new(
            bounds.origin + point(gutter, px(0.)),
            size((bounds.size.width - gutter).max(px(0.)), bounds.size.height),
        );
        let mut laid = Laid {
            text: text_bounds,
            line_height,
            cell_width,
            rows: Vec::new(),
        };
        // Once the tab knows its size, the lines it is shown at come into
        // sight, and the scroll stays within the text.
        let mut scroll = code.scroll;
        if let Some(line) = code.reveal {
            scroll.y = line_height * line as f32 - text_bounds.size.height * REVEAL_AT;
        }
        let most = code.most_scroll(&laid);
        let scroll = point(
            scroll.x.clamp(px(0.), most.x),
            scroll.y.clamp(px(0.), most.y),
        );
        let top = text_bounds.origin.y + px(PADDING) - scroll.y;
        let left = text_bounds.origin.x + px(PADDING) - scroll.x;
        let first = ((scroll.y - px(PADDING)).max(px(0.)) / line_height) as usize;
        let after = ((scroll.y + bounds.size.height) / line_height).ceil() as usize + 1;
        let marked = code
            .marked
            .map_or(0..0, |lines| lines.first - 1..lines.last);
        let selection = code
            .selection
            .map(|(anchor, head)| (anchor.min(head), anchor.max(head)));

        let mut painted = Painted {
            text_bounds,
            line_height,
            ..Painted::default()
        };
        for line in first..after.min(lines) {
            let y = top + line_height * line as f32;
            let shown = Shown::of(&code.buffer.line(line));
            let shaped = shape(&shown.text, FOREGROUND, window);
            let is_marked = marked.contains(&line);
            if is_marked {
                painted.marked.push(Bounds::new(
                    point(bounds.origin.x, y),
                    size(bounds.size.width, line_height),
                ));
            }
            if let Some((from, width)) = selection
                .and_then(|selection| selected(line, &shown, &shaped, selection, cell_width))
            {
                painted
                    .selection
                    .push(Bounds::new(point(left + from, y), size(width, line_height)));
            }
            let number = (line + 1).to_string();
            let color = if is_marked { NUMBER_MARKED } else { NUMBER };
            let number = shape(&number, color, window);
            let x = bounds.origin.x + gutter - px(PADDING) - number.width;
            painted.numbers.push((point(x, y), number));
            painted.text.push((point(left, y), shaped.clone()));
            laid.rows.push(Row {
                line,
                origin: point(left, y),
                shaped,
                shown,
            });
        }
        self.code.update(cx, |code, _| {
            code.reveal = None;
            code.scroll = scroll;
            code.laid = Some(laid);
        });
        painted
    }
}

/// Where on `line`, shown as `shown` and shaped as `shaped`, `selection`
/// starts, and how wide it is there, if it is on the line.
fn selected(
    line: usize,
    shown: &Shown,
    shaped: &ShapedLine,
    (start, end): (Place, Place),
    cell_width: Pixels,
) -> Option<(Pixels, Pixels)> {
    if !(start.line..=end.line).contains(&line) {
        return None;
    }
    let from = if line == start.line { start.column } else { 0 };
    let to = if line == end.line {
        end.column
    } else {
        shown.len
    };
    let x = |column| shaped.x_for_index(shown.index_of(column));
    // A selection that goes on to the next line covers the line break too.
    let line_break = if line < end.line { cell_width } else { px(0.) };
    let width = x(to) - x(from) + line_break;
    (width > px(0.)).then(|| (x(from), width))
}

impl IntoElement for Text {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Text {
    type RequestLayoutState = ();
    type PrepaintState = Painted;

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
        style.size.height = relative(1.).into();
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
        self.lay_out(bounds, window, cx)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        painted: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = painted.line_height;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for at in &painted.marked {
                window.paint_quad(fill(*at, rgb(MARKED)));
            }
            for (origin, number) in &painted.numbers {
                number.paint(*origin, line_height, window, cx).ok();
            }
            let text = ContentMask {
                bounds: painted.text_bounds,
            };
            window.with_content_mask(Some(text), |window| {
                for at in &painted.selection {
                    window.paint_quad(fill(*at, rgb(SELECTION)));
                }
                for (origin, line) in &painted.text {
                    line.paint(*origin, line_height, window, cx).ok();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_tab_characters_as_spaces_to_the_next_stop() {
        let shown = Shown::of("\tab\tc");
        assert_eq!(shown.text, "    ab  c");
        // In the spaces of the first tab, and right on `a`.
        assert_eq!(shown.column_of(2), 1);
        assert_eq!(shown.column_of(4), 1);
        assert_eq!(shown.column_of(9), 5);
        assert_eq!(shown.index_of(3), 6);
        assert_eq!(shown.index_of(5), 9);
    }

    #[test]
    fn cuts_a_long_line() {
        let shown = Shown::of(&"x".repeat(LINE_LENGTH * 2));
        assert_eq!(shown.text.len(), LINE_LENGTH);
        assert_eq!(shown.column_of(LINE_LENGTH), LINE_LENGTH * 2);
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
