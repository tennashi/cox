//! Text drawn so that the human can select and copy it and click its
//! links, as in the conversation: replies as Markdown, the rest as it
//! came.
//!
//! Each piece of text is a [`SelectableText`]. When it is laid out, it
//! notes in [`Painted`] what it says and where it went, which is how the
//! view finds the text under the mouse and what a selection copies. Every
//! frame draws the pieces again, so `Painted` holds those of the last
//! frame. [`Reading`] turns the human's mouse into what they select of
//! it, and the links they click.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId, FontStyle,
    FontWeight, GlobalElementId, HighlightStyle, Hitbox, HitboxBehavior, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseDownEvent, MouseMoveEvent, Pixels, Point, SharedString,
    StrikethroughStyle, StyledText, TextLayout, UnderlineStyle, Window, div, prelude::*, px, rgb,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui::markdown::{self, Block, Kind};
use crate::ui::selection::{Piece, Selection, Spot, TextId};
use crate::ui::theme::{
    BORDER, BORDER_STRONG, BUTTON, BUTTON_HOVER, CODE_FONT, DIM, LINK, RAISED, SELECTION,
};

const CODE: u32 = 0xe0_b080;

/// How far each list a block is in moves it right.
const INDENT: f32 = 20.;

/// A piece of text as the last frame laid it out.
struct PaintedText {
    id: TextId,
    text: SharedString,
    joined: &'static str,
    layout: TextLayout,
    /// The part of the text that shows: a piece in a box that scrolls on
    /// its own can be partly hidden.
    visible: Bounds<Pixels>,
    links: Rc<[(Range<usize>, SharedString)]>,
}

/// The pieces of text the last frame laid out, in order.
#[derive(Clone, Default)]
pub struct Painted(Rc<RefCell<Vec<PaintedText>>>);

impl Painted {
    /// Forgets the pieces, before a frame lays them out again.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
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
        let text = texts.iter().find(|text| text.visible.contains(&position))?;
        link_at(&text.layout, &text.links, position)
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
/// what they selected of it, and whether they are selecting still.
#[derive(Default)]
pub struct Reading {
    painted: Painted,
    selection: Option<Selection>,
    /// Whether a mouse button went down on the text and is still held.
    selecting: bool,
}

impl Reading {
    /// Forgets how the text was laid out, before a frame lays it out
    /// again.
    pub fn clear(&self) {
        self.painted.clear();
    }

    /// Writes entry `entry` of the text, showing what of it is selected.
    #[must_use]
    pub fn writer(&self, entry: usize) -> Writer {
        Writer::new(&self.painted, self.selection, entry)
    }

    /// A button went down: start selecting there, a word or a whole piece
    /// of text at a time on a double or triple click, or with Shift, move
    /// the end of what is selected there.
    pub fn press(&mut self, event: &MouseDownEvent) {
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
            .any(|text| text.visible.contains(&position))
    }

    /// Lets go of what is selected: answers whether something was.
    pub fn deselect(&mut self) -> bool {
        self.selection.take().is_some()
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

/// A piece of text the human can select, and whose links they can click.
pub struct SelectableText {
    id: TextId,
    text: SharedString,
    joined: &'static str,
    links: Rc<[(Range<usize>, SharedString)]>,
    styled: StyledText,
    painted: Painted,
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
        self.painted.0.borrow_mut().push(PaintedText {
            id: self.id,
            text: self.text.clone(),
            joined: self.joined,
            layout: self.styled.layout().clone(),
            visible: bounds.intersect(&window.content_mask().bounds),
            links: Rc::clone(&self.links),
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
        }
    }

    /// A piece showing `text` with its web addresses as links.
    pub fn plain(&mut self, text: &str, joined: &'static str) -> SelectableText {
        self.text(&markdown::Text::plain(text), joined)
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

    /// A code block, with a button that copies all of it.
    fn code(&mut self, text: &markdown::Text) -> gpui::Div {
        let code = text.text.clone();
        let button = ElementId::Name(format!("copy-{}-{}", self.entry, self.part).into());
        div()
            .relative()
            .p_2()
            .rounded_sm()
            .bg(rgb(RAISED))
            .font_family(CODE_FONT)
            .text_size(px(13.))
            .child(self.text(text, "\n\n"))
            .child(
                div()
                    .id(button)
                    .absolute()
                    .top_1()
                    .right_1()
                    .px_1()
                    .rounded_sm()
                    .text_size(px(11.))
                    .text_color(rgb(DIM))
                    .bg(rgb(BUTTON))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(BUTTON_HOVER)))
                    .child("Copy")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                    }),
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
