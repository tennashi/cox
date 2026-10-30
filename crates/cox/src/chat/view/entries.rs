//! The entries of a chat's conversation, as the window draws them: what
//! the human sent, the agent's replies as Markdown, and its tool calls and
//! permission requests, each on a line of its own under a mark of how it
//! stands. What a call or a request shows, such as a command's output or
//! a change to a file line by line, opens under it with a click, and shows
//! at first only where the human has something to read: the change a
//! request asks to make, or the output of a call that failed. Tool calls
//! the agent made one after another fold into one line, under which only
//! those still running show, until the human opens it.

use std::collections::HashSet;
use std::ops::Range;

use cox_core::diff::Line;
use gpui::{AnyElement, Context, SharedString, Stateful, div, prelude::*, px, rgb};

use crate::chat::conversation::{Content, Entry, State, ToolCallResult};
use crate::chat::view::ChatView;
use crate::ui::text::Writer;
use crate::ui::theme::{ASKING, DIM, FAILED, LINK};

/// How tall what a tool call shows grows before it scrolls on its own.
const CONTENT_HEIGHT: f32 = 320.;

/// The colors of a change's lines.
const REMOVED: u32 = 0xf0_8080;
const ADDED: u32 = 0x80_d080;

/// The ground of what the human sent, and of a button.
const HUMAN_BACKGROUND: u32 = 0x2c_2f36;
const BUTTON: u32 = 0x30_3030;
const HOVER: u32 = 0x40_4040;

/// How far the tool calls of a fold are moved right, under its line.
const INDENT: f32 = 16.;

/// What the conversation shows in one place: one entry, or the tool calls
/// of the entries in a range, which the agent made one after another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Shown {
    Entry(usize),
    Calls(Range<usize>),
}

/// What the conversation shows of `entries`, in order: two tool calls or
/// more in a row fold into one place.
pub(super) fn shown(entries: &[Entry]) -> Vec<Shown> {
    let mut shown = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let calls = entries[index..]
            .iter()
            .take_while(|entry| matches!(entry, Entry::ToolCall { .. }))
            .count();
        if calls > 1 {
            shown.push(Shown::Calls(index..index + calls));
            index += calls;
        } else {
            shown.push(Shown::Entry(index));
            index += 1;
        }
    }
    shown
}

/// A part of the conversation that the human opens and closes: what the
/// entry at an index shows under its line, or the tool calls folded from
/// the entry at an index on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Fold {
    Content(usize),
    Calls(usize),
}

/// The parts the human opened or closed, which show otherwise as they do
/// at first.
#[derive(Debug, Default)]
pub(super) struct Folds(HashSet<Fold>);

impl Folds {
    /// Whether `fold` is open, which it is at first when `at_first` is.
    fn is_open(&self, fold: Fold, at_first: bool) -> bool {
        at_first != self.0.contains(&fold)
    }

    /// Opens `fold` if it is closed, or else closes it.
    fn flip(&mut self, fold: Fold) {
        if !self.0.remove(&fold) {
            self.0.insert(fold);
        }
    }
}

impl ChatView {
    /// The conversation's entries, folded as the human left them.
    pub(super) fn render_entries(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let entries = self.conversation.entries();
        let mut rendered = Vec::new();
        for shown in shown(entries) {
            rendered.push(match shown {
                Shown::Entry(index) => self.render_entry(index, &entries[index], cx),
                Shown::Calls(calls) => self.render_calls(calls, cx),
            });
        }
        rendered
    }

    /// Tool calls made one after another, under a line that tells how many
    /// they are and how many failed. Closed, only those still running show.
    fn render_calls(&self, calls: Range<usize>, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.conversation.entries();
        let fold = Fold::Calls(calls.start);
        let open = self.folds.is_open(fold, false);
        let failed = entries[calls.clone()]
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    Entry::ToolCall {
                        result: Some(ToolCallResult::Failed),
                        ..
                    }
                )
            })
            .count();
        let start = calls.start;
        let summary = div()
            .id(("calls", start))
            .debug_selector(move || format!("calls-{start}"))
            .flex()
            .gap_2()
            .text_color(rgb(DIM))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON)))
            .child(disclosure(Some(open)))
            .child(format!("{} tool calls", calls.len()))
            .when(failed > 0, |summary| {
                summary.child(
                    div()
                        .text_color(rgb(FAILED))
                        .child(format!("{failed} failed")),
                )
            })
            .on_click(cx.listener(move |view, _, _, cx| {
                view.folds.flip(fold);
                cx.notify();
            }));
        let shown: Vec<_> = calls
            .filter(|&index| open || matches!(entries[index], Entry::ToolCall { result: None, .. }))
            .map(|index| self.render_entry(index, &entries[index], cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(summary)
            .when(!shown.is_empty(), |calls| {
                calls.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .pl(px(INDENT))
                        .children(shown),
                )
            })
            .into_any_element()
    }

    fn render_entry(&self, index: usize, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let mut writer = self.reading.writer(index);
        match entry {
            Entry::Human(text) => div()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(rgb(HUMAN_BACKGROUND))
                .child(writer.plain(text, "\n\n"))
                .into_any_element(),
            Entry::Reply(text) => writer
                .markdown(text)
                .debug_selector(|| format!("reply-{index}"))
                .into_any_element(),
            Entry::ToolCall {
                title,
                content,
                result,
                ..
            } => {
                let (mark, color) = match result {
                    None => ("●", LINK),
                    Some(ToolCallResult::Completed) => ("✓", ADDED),
                    Some(ToolCallResult::Failed) => ("✗", FAILED),
                    Some(ToolCallResult::Cancelled) => ("–", DIM),
                };
                let fold = Fold::Content(index);
                // The output of a call that failed tells why.
                let open = self
                    .folds
                    .is_open(fold, *result == Some(ToolCallResult::Failed));
                let opens = (!content.is_empty()).then_some(open);
                let line = line_of(index, opens, (mark, color), title, &mut writer);
                let line = folding(line, opens.map(|_| fold), cx);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(line)
                    .when(open, |call| {
                        call.children(render_content(index, content, &mut writer))
                    })
                    .into_any_element()
            }
            Entry::Note(text) => div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.plain(text, "\n\n"))
                .into_any_element(),
            Entry::Permission {
                title,
                content,
                options,
                chosen,
                ..
            } => {
                let asking = self.conversation.state() == State::Asking
                    && chosen.is_none()
                    && index + 1 == self.conversation.entries().len();
                if asking {
                    render_asking(index, title, content, options, &mut writer, cx)
                } else {
                    let answer = chosen
                        .and_then(|chosen| options.get(chosen))
                        .map_or("no answer", String::as_str);
                    // What was asked was read when it was answered.
                    let fold = Fold::Content(index);
                    let open = self.folds.is_open(fold, false);
                    let opens = (!content.is_empty()).then_some(open);
                    let line = line_of(index, opens, ("?", ASKING), title, &mut writer).child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(DIM))
                            .child(SharedString::from(answer.to_owned())),
                    );
                    let line = folding(line, opens.map(|_| fold), cx);
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(line)
                        .when(open, |asked| {
                            asked.children(render_content(index, content, &mut writer))
                        })
                        .into_any_element()
                }
            }
        }
    }
}

/// The permission request the agent waits on: what it asks to do, and
/// its options, to click, or to choose by their numbers.
fn render_asking(
    index: usize,
    title: &str,
    content: &[Content],
    options: &[String],
    writer: &mut Writer,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let options = options.iter().enumerate().map(|(i, name)| {
        div()
            .id(("option", i))
            .debug_selector(|| format!("option-{}", i + 1))
            .flex()
            .gap_2()
            .px_2()
            .rounded_sm()
            .bg(rgb(BUTTON))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(HOVER)))
            .child(div().text_color(rgb(DIM)).child(format!("{}", i + 1)))
            .child(SharedString::from(name.clone()))
            .on_click(cx.listener(move |view, _, _, cx| view.choose(i, cx)))
    });
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .rounded_sm()
        .border_1()
        .border_color(rgb(ASKING))
        .child(line_of(index, None, ("?", ASKING), title, writer))
        .children(render_content(index, content, writer))
        .child(div().flex().flex_wrap().gap_2().children(options))
        .into_any_element()
}

/// The line of a tool call or a permission request: whether what it
/// shows is open, if it shows something, its mark, in its color, and its
/// title, as Markdown.
fn line_of(
    index: usize,
    open: Option<bool>,
    (mark, color): (&'static str, u32),
    title: &str,
    writer: &mut Writer,
) -> Stateful<gpui::Div> {
    div()
        .id(("line", index))
        .debug_selector(move || format!("line-{index}"))
        .flex()
        .gap_2()
        .child(disclosure(open))
        .child(div().flex_shrink_0().text_color(rgb(color)).child(mark))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(rgb(DIM))
                .child(writer.markdown(title)),
        )
}

/// `line`, for a click to open or close `fold`, if there is something to
/// open.
fn folding(
    line: Stateful<gpui::Div>,
    fold: Option<Fold>,
    cx: &mut Context<ChatView>,
) -> Stateful<gpui::Div> {
    let Some(fold) = fold else {
        return line;
    };
    line.cursor_pointer()
        .hover(|style| style.bg(rgb(BUTTON)))
        .on_click(cx.listener(move |view, _, _, cx| {
            view.folds.flip(fold);
            cx.notify();
        }))
}

/// The mark of whether a part is open, `None` for one with nothing to
/// open, which keeps the place of the mark.
fn disclosure(open: Option<bool>) -> gpui::Div {
    let mark = match open {
        Some(true) => "▾",
        Some(false) => "▸",
        None => "",
    };
    div()
        .flex_shrink_0()
        .w(px(12.))
        .text_color(rgb(DIM))
        .child(mark)
}

/// What a tool call or a permission request shows, under its title, or
/// `None` when it shows nothing: text as Markdown, as the agent sends
/// what a command printed in a code block. It scrolls on its own once it
/// is tall, such as a long output or a change to a large file.
fn render_content(
    index: usize,
    content: &[Content],
    writer: &mut Writer,
) -> Option<gpui::AnyElement> {
    if content.is_empty() {
        return None;
    }
    let shown: Vec<_> = content
        .iter()
        .map(|content| match content {
            Content::Text(text) => writer.markdown(text),
            Content::Diff { path, lines } => div()
                .flex()
                .flex_col()
                .child(div().text_color(rgb(DIM)).child(writer.plain(path, "\n")))
                .children(lines.iter().map(|line| render_line(line, writer))),
        })
        .collect();
    let content = div()
        .id(("content", index))
        .debug_selector(move || format!("content-{index}"))
        .max_h(px(CONTENT_HEIGHT))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_1()
        .pl_2()
        .border_l_1()
        .border_color(rgb(0x40_4040))
        .text_size(px(12.))
        .children(shown);
    Some(content.into_any_element())
}

/// One line of a change, its mark in a column of its own, so that a line
/// too long for the conversation wraps clear of it, and copies without it.
fn render_line(line: &Line, writer: &mut Writer) -> gpui::Div {
    let (color, mark, text) = match line {
        Line::Same(text) => (DIM, "", text.clone()),
        Line::Removed(text) => (REMOVED, "-", text.clone()),
        Line::Added(text) => (ADDED, "+", text.clone()),
        Line::Skipped(1) => (DIM, "", "⋯ 1 line".to_owned()),
        Line::Skipped(count) => (DIM, "", format!("⋯ {count} lines")),
    };
    div()
        .flex()
        .text_color(rgb(color))
        .child(div().flex_shrink_0().w(px(14.)).child(mark))
        .child(div().flex_1().min_w_0().child(writer.plain(&text, "\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str) -> Entry {
        Entry::ToolCall {
            id: id.to_owned(),
            title: "ls".to_owned(),
            content: Vec::new(),
            result: None,
        }
    }

    #[test]
    fn folds_tool_calls_in_a_row() {
        let entries = [
            Entry::Human("look".to_owned()),
            call("call-1"),
            Entry::Reply("one".to_owned()),
            call("call-2"),
            call("call-3"),
            call("call-4"),
            Entry::Reply("three".to_owned()),
            call("call-5"),
            call("call-6"),
        ];

        assert_eq!(
            shown(&entries),
            [
                Shown::Entry(0),
                Shown::Entry(1),
                Shown::Entry(2),
                Shown::Calls(3..6),
                Shown::Entry(6),
                Shown::Calls(7..9),
            ]
        );
    }

    #[test]
    fn a_fold_opens_and_closes_from_how_it_shows_at_first() {
        let mut folds = Folds::default();
        let fold = Fold::Content(3);
        assert!(!folds.is_open(fold, false));
        assert!(folds.is_open(fold, true));

        folds.flip(fold);
        assert!(folds.is_open(fold, false));
        assert!(!folds.is_open(fold, true));
        assert!(!folds.is_open(Fold::Calls(3), false));

        folds.flip(fold);
        assert!(!folds.is_open(fold, false));
    }
}
