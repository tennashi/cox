//! Drawing the terminal's grid: each frame fits the terminal to the
//! tab, and paints the cells as the program colored them, the selection,
//! the cursor, and what the input method is composing at it. The grid is
//! where the human's typing goes too.

use std::sync::Arc;

use alacritty_terminal::index::Line;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{CursorShape, Rgb};
use gpui::{
    App, Bounds, Element, ElementId, ElementInputHandler, Entity, FontStyle, FontWeight,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, ShapedLine,
    SharedString, Style, TextRun, UnderlineStyle, Window, fill, font, outline, point, px, relative,
    rgb, size,
};

use super::Terminal;
use crate::terminal::colors::{CURSOR, FOREGROUND, dim, resolve};
use crate::terminal::pty::Size;
use crate::terminal::{FONT_SIZE, LINE_HEIGHT};
use crate::ui::theme::{CODE_FONT, SELECTION, SURFACE};

/// Room between the grid and the tab's edges.
const PADDING: f32 = 6.;

/// Where the grid was last drawn, to find the cell under the mouse.
#[derive(Clone, Copy, Debug)]
pub(super) struct Laid {
    pub(super) origin: Point<Pixels>,
    pub(super) cell: gpui::Size<Pixels>,
}

/// Draws the terminal's grid, and takes the human's typing for it.
pub(super) struct Grid {
    pub(super) terminal: Entity<Terminal>,
}

/// What a frame of the grid paints.
#[derive(Default)]
pub(super) struct Painted {
    /// Backgrounds other than the terminal's own, and the selection.
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    /// Text, at where it starts.
    text: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, CursorShape)>,
    /// What the input method composes, at the cursor.
    composing: Option<(Point<Pixels>, ShapedLine)>,
    line_height: Pixels,
}

/// A run of cells drawn alike, as it builds up.
struct Run {
    line: usize,
    column: usize,
    text: String,
    look: Look,
    /// Whether the next cell may join it: not after a wide character, so
    /// that the text after one starts where its cell does, whatever the
    /// font makes of it, nor after the cursor.
    open: bool,
}

impl Run {
    /// Puts the run after `runs`: as part of the last, if it goes on from
    /// it alike.
    fn add_to(self, runs: &mut Vec<Self>) {
        match runs.last_mut() {
            Some(last)
                if last.open
                    && self.open
                    && last.line == self.line
                    && last.look == self.look
                    && last.column + last.text.chars().count() == self.column =>
            {
                last.text.push_str(&self.text);
            }
            _ => runs.push(self),
        }
    }
}

/// How a cell's text looks.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    color: Hsla,
    weight: FontWeight,
    style: FontStyle,
    underline: bool,
    strikethrough: bool,
}

/// The cell size and the font of the grid.
struct Metrics {
    font: gpui::Font,
    font_size: Pixels,
    cell: gpui::Size<Pixels>,
}

impl Metrics {
    fn of(window: &Window) -> Self {
        let font = font(CODE_FONT);
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system();
        let width = text_system
            .advance(text_system.resolve_font(&font), font_size, 'm')
            .map_or(px(8.), |advance| advance.width);
        Self {
            font,
            font_size,
            cell: size(width, (font_size * LINE_HEIGHT).round()),
        }
    }

    /// How many cells fit in `bounds`.
    fn fitting(&self, bounds: Bounds<Pixels>) -> Size {
        let area = bounds.size - size(px(PADDING * 2.), px(PADDING * 2.));
        Size {
            columns: whole_cells(area.width / self.cell.width).max(2),
            lines: whole_cells(area.height / self.cell.height).max(1),
        }
    }

    /// What the input method composes, shaped to paint over the grid.
    fn shape_composing(&self, text: &str, window: &Window) -> ShapedLine {
        let look = Look {
            color: rgb(FOREGROUND).into(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            underline: true,
            strikethrough: false,
        };
        self.shape(text, look, Some(rgb(SURFACE).into()), window)
    }

    /// `text` as it looks with `look`, shaped to paint.
    fn shape(
        &self,
        text: &str,
        look: Look,
        background: Option<Hsla>,
        window: &Window,
    ) -> ShapedLine {
        let font = gpui::Font {
            weight: look.weight,
            style: look.style,
            ..self.font.clone()
        };
        window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            self.font_size,
            &[TextRun {
                len: text.len(),
                font,
                color: look.color,
                background_color: background,
                underline: look.underline.then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(look.color),
                    wavy: false,
                }),
                strikethrough: look.strikethrough.then_some(gpui::StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(look.color),
                }),
            }],
            None,
        )
    }
}

/// The line of the screen that `line` of the grid is on, scrolled back by
/// `offset`, if it is on the screen.
fn on_screen(line: Line, offset: usize) -> Option<usize> {
    usize::try_from(i64::from(line.0) + i64::try_from(offset).ok()?).ok()
}

/// How a cell looks: its background, if not the terminal's, and its text.
fn look_of(cell: &alacritty_terminal::term::cell::Cell, colors: &Colors) -> (Option<Hsla>, Look) {
    let flags = cell.flags;
    let mut foreground = resolve(cell.fg, colors);
    let mut background = resolve(cell.bg, colors);
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut foreground, &mut background);
    }
    if flags.contains(Flags::DIM) {
        foreground = dim(foreground);
    }
    if flags.contains(Flags::HIDDEN) {
        foreground = background;
    }
    let background = hsla(background);
    let look = Look {
        color: hsla(foreground),
        weight: if flags.contains(Flags::BOLD) {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if flags.contains(Flags::ITALIC) {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
        underline: flags.intersects(Flags::ALL_UNDERLINES),
        strikethrough: flags.contains(Flags::STRIKEOUT),
    };
    let background = (background != Hsla::from(rgb(SURFACE))).then_some(background);
    (background, look)
}

impl Grid {
    /// Fits the terminal to `bounds`, and works out what to paint there.
    #[expect(
        clippy::cast_precision_loss,
        reason = "cell counts are far below what f32 keeps exactly"
    )]
    fn lay_out(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Painted {
        let metrics = Metrics::of(window);
        let cell = metrics.cell;
        let fits = metrics.fitting(bounds);
        let origin = bounds.origin + point(px(PADDING), px(PADDING));
        let focused = self.terminal.read(cx).focus_handle.is_focused(window);
        // Nothing types once the program has ended.
        let ended = self.terminal.read(cx).ended.is_some();
        let (term, composing) = self.terminal.update(cx, |terminal, _| {
            terminal.resize(fits, cell);
            terminal.laid = Some(Laid { origin, cell });
            (Arc::clone(&terminal.term), terminal.composing.clone())
        });
        let term = term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset;
        let at = |line: usize, column: usize| {
            origin + point(cell.width * column as f32, cell.height * line as f32)
        };
        let cursor = content.cursor;
        let cursor_line =
            on_screen(cursor.point.line, offset).filter(|&line| line < fits.lines && !ended);
        let block_cursor = focused && cursor.shape == CursorShape::Block;

        let mut painted = Painted::default();
        let mut runs: Vec<Run> = Vec::new();
        for indexed in content.display_iter {
            if indexed
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let Some(line) = on_screen(indexed.point.line, offset) else {
                continue;
            };
            let column = indexed.point.column.0;
            let wide = indexed.flags.contains(Flags::WIDE_CHAR);
            let (mut background, mut look) = look_of(indexed.cell, content.colors);
            if content
                .selection
                .is_some_and(|selection| selection.contains(indexed.point))
            {
                background = Some(rgb(SELECTION).into());
            }
            let under_cursor = cursor_line == Some(line) && cursor.point.column.0 == column;
            if under_cursor && block_cursor {
                background = Some(rgb(CURSOR).into());
                look.color = rgb(SURFACE).into();
            }
            if let Some(background) = background {
                let width = if wide { cell.width * 2. } else { cell.width };
                painted.backgrounds.push((
                    Bounds::new(at(line, column), size(width, cell.height)),
                    background,
                ));
            }
            let mut text = String::from(indexed.c);
            if let Some(zerowidth) = indexed.zerowidth() {
                text.extend(zerowidth);
            }
            Run {
                line,
                column,
                text,
                look,
                open: !wide && !under_cursor,
            }
            .add_to(&mut runs);
        }
        for run in runs {
            if run.text.trim().is_empty() && !run.look.underline {
                continue;
            }
            let shaped = metrics.shape(&run.text, run.look, None, window);
            painted.text.push((at(run.line, run.column), shaped));
        }
        if let Some(line) = cursor_line
            && cursor.shape != CursorShape::Hidden
        {
            let wide = term.grid()[cursor.point].flags.contains(Flags::WIDE_CHAR);
            let cells = if wide { 2. } else { 1. };
            let at = at(line, cursor.point.column.0);
            let shape = if focused {
                cursor.shape
            } else {
                CursorShape::HollowBlock
            };
            painted.cursor = Some((
                Bounds::new(at, size(cell.width * cells, cell.height)),
                shape,
            ));
            painted.composing =
                composing.map(|composing| (at, metrics.shape_composing(&composing, window)));
        }
        painted.line_height = cell.height;
        painted
    }
}

impl IntoElement for Grid {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Grid {
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
        let focus_handle = self.terminal.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.terminal.clone()),
            cx,
        );
        let line_height = painted.line_height;
        for (at, color) in &painted.backgrounds {
            window.paint_quad(fill(*at, *color));
        }
        for (origin, line) in &painted.text {
            line.paint(*origin, line_height, window, cx).ok();
        }
        if let Some((at, shape)) = painted.cursor {
            let color = rgb(CURSOR);
            match shape {
                // Painted under the text, as its cell's background.
                CursorShape::Block | CursorShape::Hidden => {}
                CursorShape::HollowBlock => {
                    window.paint_quad(outline(at, color, gpui::BorderStyle::Solid));
                }
                CursorShape::Beam => {
                    window.paint_quad(fill(
                        Bounds::new(at.origin, size(px(2.), at.size.height)),
                        color,
                    ));
                }
                CursorShape::Underline => {
                    let origin = at.origin + point(px(0.), at.size.height - px(2.));
                    window.paint_quad(fill(
                        Bounds::new(origin, size(at.size.width, px(2.))),
                        color,
                    ));
                }
            }
        }
        if let Some((origin, line)) = &painted.composing {
            line.paint_background(*origin, line_height, window, cx).ok();
            line.paint(*origin, line_height, window, cx).ok();
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a count of cells on the screen, floored, and none below 0"
)]
pub(super) fn whole_cells(cells: f32) -> usize {
    cells.floor().max(0.) as usize
}

fn hsla(color: Rgb) -> Hsla {
    rgb(u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)).into()
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::term::cell::Cell;
    use alacritty_terminal::vte::ansi::{Color, Rgb};

    use super::*;

    fn plain() -> Look {
        Look {
            color: rgb(FOREGROUND).into(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            underline: false,
            strikethrough: false,
        }
    }

    fn run(line: usize, column: usize, text: &str, look: Look, open: bool) -> Run {
        Run {
            line,
            column,
            text: text.to_owned(),
            look,
            open,
        }
    }

    fn texts(runs: &[Run]) -> Vec<&str> {
        runs.iter().map(|run| run.text.as_str()).collect()
    }

    #[test]
    fn cells_drawn_alike_one_after_another_are_one_run() {
        let mut runs = Vec::new();

        run(0, 0, "ab", plain(), true).add_to(&mut runs);
        run(0, 2, "c", plain(), true).add_to(&mut runs);

        assert_eq!(texts(&runs), ["abc"]);
    }

    #[test]
    fn cells_drawn_otherwise_are_runs_of_their_own() {
        let bold = Look {
            weight: FontWeight::BOLD,
            ..plain()
        };
        let mut runs = Vec::new();

        run(0, 0, "a", plain(), true).add_to(&mut runs);
        run(0, 1, "b", bold, true).add_to(&mut runs);

        assert_eq!(texts(&runs), ["a", "b"]);
    }

    #[test]
    fn a_run_does_not_go_on_past_a_gap_nor_onto_the_next_line() {
        let mut runs = Vec::new();

        run(0, 0, "a", plain(), true).add_to(&mut runs);
        run(0, 2, "b", plain(), true).add_to(&mut runs);
        run(1, 3, "c", plain(), true).add_to(&mut runs);

        assert_eq!(texts(&runs), ["a", "b", "c"]);
    }

    #[test]
    fn nothing_joins_a_run_closed_as_after_a_wide_character() {
        let mut runs = Vec::new();

        run(0, 0, "字", plain(), false).add_to(&mut runs);
        run(0, 1, "a", plain(), true).add_to(&mut runs);

        assert_eq!(texts(&runs), ["字", "a"]);
    }

    fn metrics() -> Metrics {
        Metrics {
            font: font(CODE_FONT),
            font_size: px(FONT_SIZE),
            cell: size(px(8.), px(17.)),
        }
    }

    #[test]
    fn as_many_whole_cells_fit_as_the_room_inside_the_padding_holds() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(80.)));

        assert_eq!(
            metrics().fitting(bounds),
            Size {
                columns: 11,
                lines: 4
            }
        );
    }

    #[test]
    fn a_grid_too_small_for_any_cell_still_has_two_columns_and_a_line() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(4.), px(4.)));

        assert_eq!(
            metrics().fitting(bounds),
            Size {
                columns: 2,
                lines: 1
            }
        );
    }

    #[test]
    fn a_line_scrolled_back_onto_the_screen_is_where_the_offset_puts_it() {
        assert_eq!(on_screen(Line(-3), 5), Some(2));
    }

    #[test]
    fn a_line_scrolled_off_the_screen_is_on_none_of_it() {
        assert_eq!(on_screen(Line(-3), 0), None);
    }

    fn red_on_blue(flags: Flags) -> Cell {
        Cell {
            fg: Color::Spec(Rgb {
                r: 0xff,
                g: 0,
                b: 0,
            }),
            bg: Color::Spec(Rgb {
                r: 0,
                g: 0,
                b: 0xff,
            }),
            flags,
            ..Cell::default()
        }
    }

    const RED: u32 = 0xff_0000;
    const BLUE: u32 = 0x00_00ff;

    #[test]
    fn a_cell_is_drawn_in_the_colors_the_program_gave_it() {
        let (background, look) = look_of(&red_on_blue(Flags::empty()), &Colors::default());

        assert_eq!(background, Some(rgb(BLUE).into()));
        assert_eq!(look.color, Hsla::from(rgb(RED)));
    }

    #[test]
    fn an_inverse_cell_swaps_its_colors() {
        let (background, look) = look_of(&red_on_blue(Flags::INVERSE), &Colors::default());

        assert_eq!(background, Some(rgb(RED).into()));
        assert_eq!(look.color, Hsla::from(rgb(BLUE)));
    }

    #[test]
    fn a_hidden_cell_has_its_text_in_its_background() {
        let (_, look) = look_of(&red_on_blue(Flags::HIDDEN), &Colors::default());

        assert_eq!(look.color, Hsla::from(rgb(BLUE)));
    }

    #[test]
    fn a_cell_on_the_terminals_own_background_draws_none() {
        let (background, _) = look_of(&Cell::default(), &Colors::default());

        assert_eq!(background, None);
    }

    #[test]
    fn a_bold_italic_underlined_struck_cell_is_drawn_so() {
        let flags = Flags::BOLD | Flags::ITALIC | Flags::UNDERLINE | Flags::STRIKEOUT;

        let (_, look) = look_of(&red_on_blue(flags), &Colors::default());

        assert_eq!(look.weight, FontWeight::BOLD);
        assert_eq!(look.style, FontStyle::Italic);
        assert!(look.underline && look.strikethrough);
    }
}
