//! What the server says of the name the mouse rests on: its type and its
//! documentation, drawn under the name as Markdown, as the conversation
//! draws replies, for the human to select and copy, and to click its links,
//! which open on the stage. It stays while the mouse is on the name or on
//! it.

use std::ops::Range;
use std::time::Duration;

use cox_editor::buffer::Point as Place;
use cox_editor::lsp::{Client, Error, Hover};
use gpui::{
    Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollHandle, Task, Window, anchored, deferred, div, prelude::*, px, rgb,
};

use super::Code;
use super::menu::{MENU, MENU_BORDER};
use crate::ui::LinkClicked;
use crate::ui::markdown::{self, Block};
use crate::ui::text::Reading;
use crate::ui::theme::FOREGROUND;

/// How long the mouse rests on a name before the server is asked of it.
const REST: Duration = Duration::from_millis(300);

/// How long what the server says of a name stays once the mouse leaves
/// the name, for the mouse to get to it.
const LINGER: Duration = Duration::from_millis(250);

const WIDTH: f32 = 520.;
const HEIGHT: f32 = 320.;

/// What the server says of a name.
pub(super) struct Hovered {
    /// The name's line.
    pub(super) line: usize,
    /// The bytes of the line the server speaks of, which the name is in.
    pub(super) range: Range<usize>,
    /// Where it shows in the window: under the name.
    at: Point<Pixels>,
    markdown: String,
    blocks: Vec<Block>,
    reading: Reading,
    scroll: ScrollHandle,
    /// Whether the mouse is on it.
    over: bool,
}

impl Hovered {
    /// Whether it speaks of `word`, the bytes of `line`.
    fn covers(&self, line: usize, word: &Range<usize>) -> bool {
        line == self.line && self.range.start <= word.start && word.end <= self.range.end
    }

    /// What it says, as Markdown.
    pub(super) fn markdown(&self) -> &str {
        &self.markdown
    }

    /// The text selected of it.
    pub(super) fn selected_text(&self) -> String {
        self.reading.selected_text()
    }
}

impl Code {
    /// The mouse rests on `word`, a line and the bytes of it, or on no
    /// word: once it rests a while, the server is asked of the word.
    #[expect(
        clippy::used_underscore_binding,
        reason = "the waiting is only held: dropping it stops it"
    )]
    pub(super) fn rest_on(&mut self, word: Option<(usize, Range<usize>)>, cx: &mut Context<Self>) {
        if word == self.pointed {
            return;
        }
        self.pointed.clone_from(&word);
        cx.notify();
        let covered = word.as_ref().is_some_and(|(line, word)| {
            self.hover
                .as_ref()
                .is_some_and(|hovered| hovered.covers(*line, word))
        });
        if covered {
            self._hovering = Task::ready(());
            return;
        }
        self.hide_hover_soon(cx);
        let Some((line, word)) = word else {
            self._hovering = Task::ready(());
            return;
        };
        self._hovering = cx.spawn(async move |code, cx| {
            cx.background_executor().timer(REST).await;
            let asked = code.update(cx, |code, cx| {
                code.ask(Place::new(line, word.start), Client::hover, cx)
            });
            let Ok(Some(asked)) = asked else {
                return;
            };
            let answer = asked.await;
            code.update(cx, |code, cx| code.hovered(line, word, answer, cx))
                .ok();
        });
    }

    /// The server said `answer` of `word`, the bytes of `line`.
    fn hovered(
        &mut self,
        line: usize,
        word: Range<usize>,
        answer: Result<Option<Hover>, Error>,
        cx: &mut Context<Self>,
    ) {
        let hover = match answer {
            Ok(Some(hover)) => hover,
            Ok(None) => return,
            Err(error) => {
                log::debug!("no hover: {error}");
                return;
            }
        };
        // The mouse may have left the word since.
        if self.pointed.as_ref() != Some(&(line, word.clone())) {
            return;
        }
        let encoding = self.encoding(cx);
        let range = hover
            .range
            .map(|range| {
                let start = encoding.point(&self.file.buffer, range.start);
                let end = encoding.point(&self.file.buffer, range.end);
                (start, end)
            })
            .filter(|(start, end)| {
                start.line == line
                    && end.line == line
                    && start.column <= word.start
                    && word.end <= end.column
            })
            .map_or(word, |(start, end)| start.column..end.column);
        let Some(at) = self.below(line, range.start) else {
            return;
        };
        self.hover = Some(Hovered {
            line,
            range,
            at,
            blocks: markdown::parse(&hover.markdown),
            markdown: hover.markdown,
            reading: Reading::default(),
            scroll: ScrollHandle::new(),
            over: false,
        });
        cx.notify();
    }

    /// Hides what the server says of a name at once, and stops waiting to
    /// ask of another.
    #[expect(
        clippy::used_underscore_binding,
        reason = "the waiting is only held: dropping it stops it"
    )]
    pub(super) fn unhover(&mut self) {
        self.hover = None;
        self._hovering = Task::ready(());
    }

    /// Hides what the server says of a name, unless the mouse comes back
    /// to it or to the name before long.
    #[expect(
        clippy::used_underscore_binding,
        reason = "the waiting is only held: dropping it stops it"
    )]
    pub(super) fn hide_hover_soon(&mut self, cx: &mut Context<Self>) {
        if self.hover.is_none() {
            return;
        }
        self._unhovering = cx.spawn(async move |code, cx| {
            cx.background_executor().timer(LINGER).await;
            code.update(cx, |code, cx| {
                let stays = code.hover.as_ref().is_some_and(|hovered| {
                    hovered.over
                        || code
                            .pointed
                            .as_ref()
                            .is_some_and(|(line, word)| hovered.covers(*line, word))
                });
                if !stays {
                    code.hover = None;
                    cx.notify();
                }
            })
            .ok();
        });
    }

    /// What the server says of the name the mouse rests on, if it said.
    pub(super) fn render_hover(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let hovered = self.hover.as_mut()?;
        // The frame lays the text out again.
        hovered.reading.clear();
        let text = hovered.reading.writer(0).blocks(&hovered.blocks);
        let shown = div()
            .id("code-hover")
            .debug_selector(|| "code-hover".to_owned())
            .occlude()
            .w(px(WIDTH))
            .max_h(px(HEIGHT))
            .overflow_y_scroll()
            .track_scroll(&hovered.scroll)
            .px_3()
            .py_2()
            .bg(rgb(MENU))
            .border_1()
            .border_color(rgb(MENU_BORDER))
            .rounded_md()
            .shadow_md()
            .text_color(rgb(FOREGROUND))
            .on_hover(cx.listener(|code, over: &bool, _, cx| {
                if let Some(hovered) = &mut code.hover {
                    hovered.over = *over;
                }
                if !*over {
                    code.hide_hover_soon(cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|code, event: &MouseDownEvent, window, cx| {
                    window.focus(&code.focus_handle);
                    window.prevent_default();
                    if let Some(hovered) = &mut code.hover {
                        hovered.reading.press(event);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|code, event: &MouseMoveEvent, _, cx| {
                if let Some(hovered) = &mut code.hover
                    && hovered.reading.drag(event.position)
                {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|code, event: &MouseUpEvent, _: &mut Window, cx| {
                    let clicked = code
                        .hover
                        .as_mut()
                        .and_then(|hovered| hovered.reading.release(event.position));
                    if let Some(url) = clicked {
                        cx.emit(LinkClicked(url.to_string()));
                    }
                }),
            )
            .child(text);
        Some(deferred(anchored().position(hovered.at).child(shown)).with_priority(1))
    }
}
