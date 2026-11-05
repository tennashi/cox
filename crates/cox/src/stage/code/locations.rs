//! Going to the places in the code that the server finds: where a name is
//! defined, which the tab goes to when there is one, or where it is used.
//! More than one, the tab lists them under the code, by file, each with
//! its line, for the human to go to each in turn, while the list stays.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use cox_editor::buffer::{Buffer, Point as Place};
use cox_editor::lsp::{Client, Encoding, Error, Location, Position};
use gpui::{
    Context, FontWeight, HighlightStyle, Hsla, SharedString, StyledText, Task,
    UniformListScrollHandle, div, prelude::*, px, relative, rgb, uniform_list,
};

use super::Code;
use super::text::NUMBER;
use crate::ui::ellipsis::ellipsis;
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT_GROUND, BORDER, BUTTON_HOVER, CARD_INNER_RADIUS, CODE_FONT, DIM, FOREGROUND, GROUND,
    HOVER,
};

/// How tall the list is, as a part of the tab's height.
const HEIGHT: f32 = 0.35;

/// The stretch of a line that is the place.
const FOUND: u32 = 0xe0_b080;

/// What the human asked the server to find.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Finding {
    Definitions,
    References,
}

/// The places the server found, listed under the code.
pub(super) struct Locations {
    /// What they are, as the list's heading says.
    title: String,
    found: Vec<Found>,
    /// The list's rows: a file's, then the places in it.
    rows: Vec<Listed>,
    /// The place the tab went to last from the list, if it did.
    chosen: Option<usize>,
    scroll: UniformListScrollHandle,
    _read: Task<()>,
}

/// A place the server found.
struct Found {
    location: Location,
    /// The text of its line, without the spaces it starts with, and the
    /// bytes of it that are the place, once read.
    line: Option<(String, Range<usize>)>,
}

/// A row of the list.
#[derive(Clone, Copy)]
enum Listed {
    /// A file, as the index of the first place in it.
    File(usize),
    Place(usize),
}

impl Code {
    /// Goes to where the name at `place` is defined, or lists where, if
    /// the server says more than one place.
    pub(super) fn go_to_definition(&mut self, place: Place, cx: &mut Context<Self>) {
        self.find(place, Finding::Definitions, cx);
    }

    /// Lists where the name at `place` is used, and defined.
    pub(super) fn find_references(&mut self, place: Place, cx: &mut Context<Self>) {
        self.find(place, Finding::References, cx);
    }

    #[expect(
        clippy::used_underscore_binding,
        reason = "the asking is only held: dropping it leaves its answer unused"
    )]
    fn find(&mut self, place: Place, finding: Finding, cx: &mut Context<Self>) {
        let name = self.word_text(place);
        self.notice = None;
        let asked: Option<Pin<Box<dyn Future<Output = _>>>> = match finding {
            Finding::Definitions => self
                .ask(place, Client::definition, cx)
                .map(|asked| Box::pin(asked) as _),
            Finding::References => self
                .ask(place, Client::references, cx)
                .map(|asked| Box::pin(asked) as _),
        };
        let Some(asked) = asked else {
            self.notice = Some(match &self.server {
                Some(server) => format!("{} is not ready", server.read(cx).name()),
                None => "no language server knows this file".to_owned(),
            });
            cx.notify();
            return;
        };
        self._asking = cx.spawn(async move |code, cx| {
            let answer = asked.await;
            code.update(cx, |code, cx| code.found(&name, finding, answer, cx))
                .ok();
        });
        cx.notify();
    }

    /// The server answered `answer` to `finding` of `name`.
    fn found(
        &mut self,
        name: &str,
        finding: Finding,
        answer: Result<Vec<Location>, Error>,
        cx: &mut Context<Self>,
    ) {
        let mut found = match answer {
            Ok(found) => found,
            Err(error) => {
                self.notice = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        match (finding, found.len()) {
            (Finding::Definitions, 0) => {
                self.notice = Some(format!("no definition of {name} found"));
            }
            (Finding::References, 0) => {
                self.notice = Some(format!("no references to {name} found"));
            }
            (Finding::Definitions, 1) => {
                let Location { path, range } = found.remove(0);
                self.go_to(&path, range, cx);
            }
            (Finding::Definitions, count) => {
                self.list(format!("{count} definitions of {name}"), found, cx);
            }
            (Finding::References, count) => {
                let title = if count == 1 {
                    format!("1 reference to {name}")
                } else {
                    format!("{count} references to {name}")
                };
                self.list(title, found, cx);
            }
        }
        cx.notify();
    }

    /// Lists `found` under the code, with `title`, and reads the line of
    /// each in the background.
    fn list(&mut self, title: String, mut found: Vec<Location>, cx: &mut Context<Self>) {
        found.sort_by(|a, b| (&a.path, a.range.start).cmp(&(&b.path, b.range.start)));
        found.dedup();
        let mut rows = Vec::new();
        for (index, location) in found.iter().enumerate() {
            if index == 0 || found[index - 1].path != location.path {
                rows.push(Listed::File(index));
            }
            rows.push(Listed::Place(index));
        }
        let encoding = self.encoding(cx);
        let reading = found.clone();
        let read = cx.spawn(async move |code, cx| {
            let lines = cx
                .background_executor()
                .spawn(async move { read_lines(&reading, encoding) })
                .await;
            code.update(cx, |code, cx| {
                if let Some(locations) = &mut code.locations {
                    for (found, line) in locations.found.iter_mut().zip(lines) {
                        found.line = line;
                    }
                    cx.notify();
                }
            })
            .ok();
        });
        self.locations = Some(Locations {
            title,
            found: found
                .into_iter()
                .map(|location| Found {
                    location,
                    line: None,
                })
                .collect(),
            rows,
            chosen: None,
            scroll: UniformListScrollHandle::new(),
            _read: read,
        });
    }

    /// Goes to the place at `index` of the list.
    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(locations) = &mut self.locations else {
            return;
        };
        let Some(found) = locations.found.get(index) else {
            return;
        };
        locations.chosen = Some(index);
        let Location { path, range } = found.location.clone();
        self.notice = None;
        self.go_to(&path, range, cx);
    }

    /// The list of the places the server found last, if it is open.
    pub(super) fn render_locations(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let locations = self.locations.as_ref()?;
        let code = cx.entity();
        let heading = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_t_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .text_color(rgb(DIM))
            .child(
                div()
                    .debug_selector(|| "code-locations-title".to_owned())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(ellipsis(locations.title.clone())),
            )
            .child(
                div()
                    .id("close-locations")
                    .debug_selector(|| "close-locations".to_owned())
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(BUTTON_HOVER)).text_color(rgb(FOREGROUND)))
                    .child("×")
                    .on_click(cx.listener(|code, _, _, cx| {
                        code.locations = None;
                        cx.notify();
                    })),
            );
        let list = uniform_list(
            "code-locations",
            locations.rows.len(),
            move |range, _, cx| {
                let Some(locations) = &code.read(cx).locations else {
                    return Vec::new();
                };
                range
                    .filter_map(|row| locations.rows.get(row).map(|&listed| (row, listed)))
                    .map(|(row, listed)| match listed {
                        Listed::File(index) => file_row(&locations.found[index].location.path),
                        Listed::Place(index) => {
                            let code = code.clone();
                            place_row(
                                row,
                                &locations.found[index],
                                locations.chosen == Some(index),
                            )
                            .on_click(move |_, window, cx| {
                                code.update(cx, |code, cx| {
                                    window.focus(&code.focus_handle);
                                    code.choose(index, cx);
                                });
                            })
                            .into_any_element()
                        }
                    })
                    .collect()
            },
        )
        .track_scroll(locations.scroll.clone())
        .flex_1()
        .min_h_0();
        Some(
            div()
                .debug_selector(|| "code-locations".to_owned())
                .flex_shrink_0()
                .h(relative(HEIGHT))
                .flex()
                .flex_col()
                .bg(rgb(GROUND))
                // At the foot of the stage's card, round as it is.
                .rounded_b(px(CARD_INNER_RADIUS))
                .font_family(CODE_FONT)
                .text_size(px(12.))
                .child(heading)
                .child(list),
        )
    }
}

/// The row of a file of the list: its name, then where it is.
fn file_row(path: &Path) -> gpui::AnyElement {
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let folder = path.parent().map(shortened).unwrap_or_default();
    div()
        .w_full()
        .flex()
        .gap_2()
        .px_3()
        .whitespace_nowrap()
        .overflow_hidden()
        .child(div().text_color(rgb(FOREGROUND)).child(name))
        .child(
            div()
                .text_color(rgb(DIM))
                .overflow_hidden()
                .child(ellipsis(folder)),
        )
        .into_any_element()
}

/// The row of `found`, the place at `row` of the list, which the tab
/// went to last if `chosen`.
fn place_row(row: usize, found: &Found, chosen: bool) -> gpui::Stateful<gpui::Div> {
    let number = format!("{:>5}", found.location.range.start.line + 1);
    let line: gpui::AnyElement = match &found.line {
        Some((text, range)) => {
            let found = HighlightStyle {
                color: Some(Hsla::from(rgb(FOUND))),
                font_weight: Some(FontWeight::BOLD),
                ..HighlightStyle::default()
            };
            StyledText::new(SharedString::from(text.clone()))
                .with_highlights([(range.clone(), found)])
                .into_any_element()
        }
        None => div().into_any_element(),
    };
    div()
        .id(("code-place", row))
        .debug_selector(move || format!("code-place-{row}"))
        .w_full()
        .flex()
        .gap_3()
        .px_3()
        .whitespace_nowrap()
        .overflow_hidden()
        .cursor_pointer()
        .text_color(rgb(FOREGROUND))
        .map(|row| {
            if chosen {
                row.bg(rgb(ACCENT_GROUND))
            } else {
                row.hover(|style| style.bg(rgb(HOVER)))
            }
        })
        .child(div().flex_shrink_0().text_color(rgb(NUMBER)).child(number))
        .child(div().min_w_0().truncate().child(line))
}

/// The line of each of `found`, as it reads now, without the spaces it
/// starts with, and the bytes of it that are the place, counted in
/// `encoding`: `None` for a file that cannot be read.
fn read_lines(found: &[Location], encoding: Encoding) -> Vec<Option<(String, Range<usize>)>> {
    let mut read: Option<(PathBuf, Option<Buffer>)> = None;
    found
        .iter()
        .map(|location| {
            if read.as_ref().is_none_or(|(path, _)| *path != location.path) {
                let buffer = std::fs::read_to_string(&location.path)
                    .ok()
                    .map(|text| Buffer::new(&text));
                read = Some((location.path.clone(), buffer));
            }
            let buffer = read.as_ref()?.1.as_ref()?;
            let Range { start, end } = location.range;
            let line = start.line as usize;
            let text = buffer.line(line);
            let column = |position: Position| encoding.point(buffer, position).column;
            let from = column(start);
            let to = if end.line == start.line {
                column(end)
            } else {
                text.len()
            };
            let indent = text.len() - text.trim_start().len();
            let from = from.saturating_sub(indent).min(text.len() - indent);
            let to = to.saturating_sub(indent).clamp(from, text.len() - indent);
            Some((text[indent..].to_owned(), from..to))
        })
        .collect()
}
