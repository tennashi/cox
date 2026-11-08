//! Drawing the lines of the file in sight, with their numbers, the lines
//! marked, the selection, and the name the mouse rests on: an element of
//! its own, which lays the text out from the tab's [`Code`] each frame and
//! tells it how, for the tab to find what is under the mouse.

use cox_editor::language::Highlight;
use cox_editor::line::Shown;
use gpui::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, Entity, Font, FontStyle, FontWeight,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    Pixels, Point, ShapedLine, SharedString, Style, TextRun, Window, fill, font, point, px,
    relative, rgb, size,
};

use super::file::Spans;
use super::{Code, Laid, PADDING, Place, Row};
use crate::ui::theme::{ACCENT_GROUND, CODE_FONT, DISABLED, FOREGROUND, LINK, SELECTION};

const FONT_SIZE: f32 = 13.;
const LINE_HEIGHT: f32 = 1.4;

/// Where the lines the tab is shown at go, from the top, as a part of its
/// height.
const REVEAL_AT: f32 = 1. / 3.;

pub(super) const NUMBER: u32 = DISABLED;
/// The name the server says something of.
const HOVERED: u32 = crate::ui::theme::BUTTON_HOVER;
/// How code that is nothing in particular is drawn.
pub(super) const PLAIN: (u32, FontWeight, FontStyle) =
    (FOREGROUND, FontWeight::NORMAL, FontStyle::Normal);

/// How code is drawn, by what its syntax says it is: its color, and
/// whether it is bold or italic.
pub(crate) fn style(highlight: Highlight) -> (u32, FontWeight, FontStyle) {
    let color = match highlight {
        Highlight::Keyword
        | Highlight::VariableBuiltin
        | Highlight::Tag
        | Highlight::Heading
        | Highlight::PunctuationSpecial => 0x56_9cd6,
        Highlight::Function => 0xdc_dcaa,
        Highlight::Type | Highlight::TypeBuiltin | Highlight::Constructor => 0x4e_c9b0,
        Highlight::String | Highlight::Literal => 0xce_9178,
        Highlight::StringSpecial | Highlight::Escape => 0xd7_ba7d,
        Highlight::Number | Highlight::ConstantBuiltin => 0xb5_cea8,
        Highlight::Comment => 0x6a_9955,
        Highlight::Constant | Highlight::Uri => 0x4f_c1ff,
        Highlight::Property | Highlight::Variable | Highlight::Link => 0x9c_dcfe,
        Highlight::Attribute | Highlight::Label => 0xc5_86c0,
        Highlight::Operator | Highlight::Punctuation | Highlight::Emphasis | Highlight::Strong => {
            FOREGROUND
        }
    };
    let weight = match highlight {
        Highlight::Heading | Highlight::Strong => FontWeight::BOLD,
        _ => FontWeight::NORMAL,
    };
    let style = match highlight {
        Highlight::Emphasis => FontStyle::Italic,
        _ => FontStyle::Normal,
    };
    (color, weight, style)
}

/// Draws the lines in sight.
pub(super) struct Text {
    pub(super) code: Entity<Code>,
}

/// What a frame paints.
#[derive(Default)]
pub(super) struct Painted {
    /// The marked lines', under everything.
    marked: Vec<Bounds<Pixels>>,
    /// Under the text, within it.
    selection: Vec<Bounds<Pixels>>,
    /// The name the server says something of, under the text.
    hovered: Option<Bounds<Pixels>>,
    /// The line under the name a click goes to the definition of, over
    /// the text.
    link: Option<Bounds<Pixels>>,
    /// The lines in sight, at their top left.
    text: Vec<(Point<Pixels>, ShapedLine)>,
    /// Their numbers, at their top left.
    numbers: Vec<(Point<Pixels>, ShapedLine)>,
    /// Where the text shows, which it is cut to.
    text_bounds: Bounds<Pixels>,
    line_height: Pixels,
    /// Where the mouse is over the text: an I-beam, or the pointing hand
    /// while a click goes to the definition of the name under it.
    over_text: Option<Hitbox>,
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
        let (font, font_size, cell_width, line_height) = metrics(window);
        let run = |len, style| text_run(&font, len, style);
        let shape = |text: &str, runs: &[TextRun], window: &Window| {
            window.text_system().shape_line(
                SharedString::from(text.to_owned()),
                font_size,
                runs,
                None,
            )
        };

        let code = self.code.read(cx);
        let lines = code.file.buffer.lines();
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
        let mut scroll = code.look.scroll;
        if let Some(line) = code.look.reveal {
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
            .look
            .marked
            .map_or(0..0, |lines| lines.first - 1..lines.last);
        let selection = code
            .look
            .selection
            .map(|(anchor, head)| (anchor.min(head), anchor.max(head)));
        let mut highlighted = Vec::new();
        let mut painted = Painted {
            text_bounds,
            line_height,
            ..Painted::default()
        };
        for line in first..after.min(lines) {
            let y = top + line_height * line as f32;
            let shown = Shown::of(&code.file.buffer.line(line));
            let highlights = code.file.highlights_at(line, &mut highlighted);
            let shaped = shape(&shown.text, &text_runs(&font, &shown, &highlights), window);
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
            let color = if is_marked { FOREGROUND } else { NUMBER };
            let number = shape(
                &number,
                &[run(number.len(), (color, PLAIN.1, PLAIN.2))],
                window,
            );
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
        (painted.hovered, painted.link) = code.names(&laid);
        self.code.update(cx, |code, _| {
            code.look.reveal = None;
            code.look.scroll = scroll;
            code.look.laid = Some(laid);
            code.file.highlighted.extend(highlighted);
        });
        painted
    }
}

/// The font the code is drawn in, its size, how wide a character of it
/// is, and how tall a line is.
fn metrics(window: &Window) -> (Font, Pixels, Pixels, Pixels) {
    let font = font(CODE_FONT);
    let font_size = px(FONT_SIZE);
    let text_system = window.text_system();
    let cell_width = text_system
        .advance(text_system.resolve_font(&font), font_size, 'm')
        .map_or(px(8.), |advance| advance.width);
    let line_height = (font_size * LINE_HEIGHT).round();
    (font, font_size, cell_width, line_height)
}

/// The runs of `shown` in `font`, drawn as `highlights`, what the
/// stretches of its line are, say.
fn text_runs(font: &Font, shown: &Shown, highlights: &Spans) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = shown
        .runs(highlights)
        .into_iter()
        .map(|(len, highlight)| text_run(font, len, highlight.map_or(PLAIN, style)))
        .collect();
    // Even an empty line is shaped with a run.
    if runs.is_empty() {
        runs.push(text_run(font, 0, PLAIN));
    }
    runs
}

/// A run of `len` bytes of text in `font`, drawn as `style` says.
fn text_run(
    font: &Font,
    len: usize,
    (color, weight, style): (u32, FontWeight, FontStyle),
) -> TextRun {
    TextRun {
        len,
        font: Font {
            weight,
            style,
            ..font.clone()
        },
        color: Hsla::from(rgb(color)),
        background_color: None,
        underline: None,
        strikethrough: None,
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
        let mut painted = self.lay_out(bounds, window, cx);
        painted.over_text = Some(window.insert_hitbox(painted.text_bounds, HitboxBehavior::Normal));
        painted
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
        if let Some(hitbox) = &painted.over_text {
            let style = if painted.link.is_some() {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            };
            window.set_cursor_style(style, hitbox);
        }
        // The key that makes a click go to a definition underlines the name
        // under the mouse as it goes down, and stops as it comes up.
        let code = self.code.clone();
        window.on_modifiers_changed(move |event, _, cx| {
            code.update(cx, |code, cx| code.link(event.modifiers.secondary(), cx));
        });
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for at in &painted.marked {
                window.paint_quad(fill(*at, rgb(ACCENT_GROUND)));
            }
            for (origin, number) in &painted.numbers {
                number.paint(*origin, line_height, window, cx).ok();
            }
            let text = ContentMask {
                bounds: painted.text_bounds,
            };
            window.with_content_mask(Some(text), |window| {
                if let Some(at) = painted.hovered {
                    window.paint_quad(fill(at, rgb(HOVERED)));
                }
                for at in &painted.selection {
                    window.paint_quad(fill(*at, rgb(SELECTION)));
                }
                for (origin, line) in &painted.text {
                    line.paint(*origin, line_height, window, cx).ok();
                }
                if let Some(at) = painted.link {
                    window.paint_quad(fill(at, rgb(LINK)));
                }
            });
        });
    }
}
