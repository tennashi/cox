//! The entries of a chat's conversation, as the window draws them: what
//! the human sent, and Cox in their place, the agent's replies as
//! Markdown, and its tool calls,
//! permission requests, thoughts and plan, each on a line of its own under
//! a mark of how it stands. What a line stands for, such as a command's
//! output, a change to a file line by line, all the agent thought, or the
//! steps of its plan, opens under it with a click, and shows at first only
//! where the human has something to read: the change a request asks to
//! make, the output of a call that failed, or what the agent is thinking
//! as it thinks it. The work the agent did between its replies, its tool
//! calls, thoughts and plan one after another, folds into one line, under
//! which only the calls still running and the thought still arriving show,
//! until the human opens it.

use std::collections::HashSet;
use std::ops::Range;

use cox_core::diff::Line;
use gpui::{AnyElement, Context, SharedString, Stateful, div, prelude::*, px, rgb};

use crate::chat::conversation::{Content, Entry, State, Step, StepState, ToolCallResult};
use crate::chat::view::ChatView;
use crate::ui::text::Writer;
use crate::ui::theme::{ASKING, DIM, FAILED, FOREGROUND, LINK};

/// How tall what a tool call shows grows before it scrolls on its own.
const CONTENT_HEIGHT: f32 = 320.;

/// The colors of a change's lines.
const REMOVED: u32 = 0xf0_8080;
const ADDED: u32 = 0x80_d080;

/// The ground of what the human sent, and of a button.
const HUMAN_BACKGROUND: u32 = 0x2c_2f36;
const BUTTON: u32 = 0x30_3030;
const HOVER: u32 = 0x40_4040;

/// How far the work of a fold, and the steps of a plan, are moved right,
/// under their line.
const INDENT: f32 = 16.;

/// What the conversation shows in one place: one entry, or the work of
/// the entries in a range, which the agent did one after another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Shown {
    Entry(usize),
    Work(Range<usize>),
}

/// Whether `entry` is work the agent does on its way to a reply.
fn is_work(entry: &Entry) -> bool {
    matches!(
        entry,
        Entry::ToolCall { .. } | Entry::Thought(_) | Entry::Plan(_)
    )
}

/// What the conversation shows of `entries`, in order: two entries of
/// work or more in a row fold into one place.
pub(super) fn shown(entries: &[Entry]) -> Vec<Shown> {
    let mut shown = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let work = entries[index..]
            .iter()
            .take_while(|entry| is_work(entry))
            .count();
        if work > 1 {
            shown.push(Shown::Work(index..index + work));
            index += work;
        } else {
            shown.push(Shown::Entry(index));
            index += 1;
        }
    }
    shown
}

/// A part of the conversation that the human opens and closes: what the
/// entry at an index shows under its line, or the work folded from the
/// entry at an index on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Fold {
    Content(usize),
    Work(usize),
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
                Shown::Work(work) => self.render_work(work, cx),
            });
        }
        rendered
    }

    /// Whether the entry at `index` is still arriving: the last of a turn
    /// that runs.
    fn is_arriving(&self, index: usize) -> bool {
        matches!(self.conversation.state(), State::Busy | State::Stopping)
            && index + 1 == self.conversation.entries().len()
    }

    /// The work the agent did one after another, under a line that tells
    /// how many tool calls it made and how many of them failed, how often
    /// it thought, and how far its plan has gone. Closed, only the calls
    /// still running and the thought still arriving show.
    fn render_work(&self, work: Range<usize>, cx: &mut Context<Self>) -> AnyElement {
        let entries = self.conversation.entries();
        let fold = Fold::Work(work.start);
        let open = self.folds.is_open(fold, false);
        let (mut calls, mut failed, mut thoughts, mut plan) = (0, 0, 0, None);
        for entry in &entries[work.clone()] {
            match entry {
                Entry::ToolCall { result, .. } => {
                    calls += 1;
                    if *result == Some(ToolCallResult::Failed) {
                        failed += 1;
                    }
                }
                Entry::Thought(_) => thoughts += 1,
                Entry::Plan(steps) => plan = Some(steps),
                _ => {}
            }
        }
        let mut told = Vec::new();
        if calls > 0 {
            told.push((counted(calls, "tool call"), DIM));
        }
        if failed > 0 {
            told.push((format!("{failed} failed"), FAILED));
        }
        if thoughts > 0 {
            told.push((counted(thoughts, "thought"), DIM));
        }
        if let Some(steps) = plan {
            told.push((format!("plan {}", progress(steps)), DIM));
        }
        let told = told.into_iter().enumerate().map(|(i, (text, color))| {
            div()
                .text_color(rgb(color))
                .child(if i == 0 { text } else { format!("· {text}") })
        });
        let start = work.start;
        let summary = div()
            .id(("work", start))
            .debug_selector(move || format!("work-{start}"))
            .flex()
            .gap_2()
            .text_color(rgb(DIM))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON)))
            .child(disclosure(Some(open)))
            .children(told)
            .on_click(cx.listener(move |view, _, _, cx| {
                view.folds.flip(fold);
                cx.notify();
            }));
        let shown: Vec<_> = work
            .filter(|&index| {
                open || matches!(entries[index], Entry::ToolCall { result: None, .. })
                    || (matches!(entries[index], Entry::Thought(_)) && self.is_arriving(index))
            })
            .map(|index| self.render_entry(index, &entries[index], cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(summary)
            .when(!shown.is_empty(), |work| {
                work.child(
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
            // Sent in the human's place, so drawn in theirs, but as Cox's
            // notes are.
            Entry::Cox(text) => div()
                .debug_selector(|| format!("cox-{index}"))
                .px_3()
                .py_2()
                .rounded_md()
                .bg(rgb(HUMAN_BACKGROUND))
                .text_size(px(12.))
                .text_color(rgb(DIM))
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
            Entry::Thought(text) => self.render_thought(index, text, &mut writer, cx),
            Entry::Plan(steps) => self.render_plan_entry(index, steps, cx),
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
                by_setup,
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
                    // The human sees that they were not asked, and why.
                    let answer = if *by_setup {
                        format!("{answer}, as the chat is set up")
                    } else {
                        answer.to_owned()
                    };
                    self.render_answered(index, title, content, answer, &mut writer, cx)
                }
            }
        }
    }

    /// A permission request answered with `answer`, on a line that opens
    /// to what was asked, if the request showed something.
    fn render_answered(
        &self,
        index: usize,
        title: &str,
        content: &[Content],
        answer: String,
        writer: &mut Writer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // What was asked was read when it was answered.
        let fold = Fold::Content(index);
        let open = self.folds.is_open(fold, false);
        let opens = (!content.is_empty()).then_some(open);
        let line = line_of(index, opens, ("?", ASKING), title, writer).child(
            div()
                .flex_shrink_0()
                .text_color(rgb(DIM))
                .child(SharedString::from(answer)),
        );
        let line = folding(line, opens.map(|_| fold), cx);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(line)
            .when(open, |asked| {
                asked.children(render_content(index, content, writer))
            })
            .into_any_element()
    }

    /// What the agent thought, on a line that shows its start, opening to
    /// all of it.
    fn render_thought(
        &self,
        index: usize,
        text: &str,
        writer: &mut Writer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold = Fold::Content(index);
        // What the agent is thinking shows as it thinks it.
        let open = self.folds.is_open(fold, self.is_arriving(index));
        let line = div()
            .id(("line", index))
            .debug_selector(move || format!("line-{index}"))
            .flex()
            .gap_2()
            .text_color(rgb(DIM))
            .child(disclosure(Some(open)))
            .child(div().flex_shrink_0().child("∴"))
            .child(div().flex_1().min_w_0().truncate().italic().child(if open {
                "Thinking".to_owned()
            } else {
                text.lines().next().unwrap_or_default().to_owned()
            }));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(folding(line, Some(fold), cx))
            .when(open, |thought| {
                thought.child(
                    div()
                        .pl(px(INDENT))
                        .text_color(rgb(DIM))
                        .child(writer.markdown(text)),
                )
            })
            .into_any_element()
    }

    /// The plan the agent worked to, on a line that tells how far it went,
    /// opening to its steps.
    fn render_plan_entry(
        &self,
        index: usize,
        steps: &[Step],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold = Fold::Content(index);
        let open = self.folds.is_open(fold, false);
        let line = div()
            .id(("line", index))
            .debug_selector(move || format!("line-{index}"))
            .flex()
            .gap_2()
            .text_color(rgb(DIM))
            .child(disclosure(Some(open)))
            .child(div().flex_shrink_0().child("≡"))
            .child(format!("Plan {}", progress(steps)));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(folding(line, Some(fold), cx))
            .when(open, |plan| plan.child(render_steps(steps).pl(px(INDENT))))
            .into_any_element()
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

/// `count` of what `one` names, as "1 thought" or "3 thoughts".
fn counted(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

/// How far a plan has gone, as how many of its steps are done of all.
pub(super) fn progress(steps: &[Step]) -> String {
    let done = steps
        .iter()
        .filter(|step| step.state == StepState::Completed)
        .count();
    format!("{done}/{}", steps.len())
}

/// The steps of a plan, each under a mark of how far it has gone.
pub(super) fn render_steps(steps: &[Step]) -> gpui::Div {
    div().flex().flex_col().children(steps.iter().map(|step| {
        let (mark, color, text) = match step.state {
            StepState::Completed => ("✓", ADDED, DIM),
            StepState::InProgress => ("●", LINK, FOREGROUND),
            StepState::Pending => ("○", DIM, FOREGROUND),
        };
        div()
            .flex()
            .gap_2()
            .child(div().flex_shrink_0().text_color(rgb(color)).child(mark))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(rgb(text))
                    .child(SharedString::from(step.text.clone())),
            )
    }))
}

/// The mark of whether a part is open, `None` for one with nothing to
/// open, which keeps the place of the mark.
pub(super) fn disclosure(open: Option<bool>) -> gpui::Div {
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
                Shown::Work(3..6),
                Shown::Entry(6),
                Shown::Work(7..9),
            ]
        );
    }

    #[test]
    fn folds_thoughts_and_plans_with_the_tool_calls() {
        let entries = [
            Entry::Human("look".to_owned()),
            Entry::Thought("Look where?".to_owned()),
            Entry::Reply("Here.".to_owned()),
            Entry::Plan(Vec::new()),
            call("call-1"),
            Entry::Thought("Found it.".to_owned()),
            Entry::Reply("Done.".to_owned()),
        ];

        assert_eq!(
            shown(&entries),
            [
                Shown::Entry(0),
                Shown::Entry(1),
                Shown::Entry(2),
                Shown::Work(3..6),
                Shown::Entry(6),
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
        assert!(!folds.is_open(Fold::Work(3), false));

        folds.flip(fold);
        assert!(!folds.is_open(fold, false));
    }
}
