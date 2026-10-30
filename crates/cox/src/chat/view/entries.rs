//! The entries of a chat's conversation, as the window draws them: what
//! the human sent, the agent's replies as Markdown, and what its tool
//! calls and permission requests show, such as a command's output or a
//! change to a file line by line, with the options of a request to
//! choose from.

use cox_core::diff::Line;
use gpui::{Context, SharedString, div, prelude::*, px, rgb};

use crate::chat::conversation::{Content, Entry, State};
use crate::chat::view::ChatView;
use crate::ui::text::Writer;
use crate::ui::theme::DIM;

/// How tall what a tool call shows grows before it scrolls on its own.
const CONTENT_HEIGHT: f32 = 320.;

/// The colors of a change's lines.
const REMOVED: u32 = 0xf0_8080;
const ADDED: u32 = 0x80_d080;

impl ChatView {
    pub(super) fn render_entry(
        &self,
        index: usize,
        entry: &Entry,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut writer = self.reading.writer(index);
        match entry {
            Entry::Human(text) => div()
                .flex()
                .text_color(rgb(0x7f_b4ff))
                .child(div().flex_shrink_0().child("> "))
                .child(div().flex_1().min_w_0().child(writer.plain(text, "\n\n")))
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
                let state = match result {
                    Some(result) => format!("{result:?}"),
                    None => "running".to_owned(),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_color(rgb(DIM))
                            .child(writer.plain(&format!("[tool: {title} … {state}]"), "\n\n")),
                    )
                    .children(render_content(index, content, &mut writer))
                    .into_any_element()
            }
            Entry::Note(text) => div()
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
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(rgb(0xd0_a040))
                    .child(writer.plain(&format!("[permission: {title}]"), "\n\n"))
                    .children(render_content(index, content, &mut writer))
                    .children(options.iter().enumerate().map(|(i, name)| {
                        let label: SharedString = format!("{}. {name}", i + 1).into();
                        let mut option = div()
                            .id(("option", i))
                            .debug_selector(|| format!("option-{}", i + 1))
                            .px_2()
                            .child(label);
                        if *chosen == Some(i) {
                            option = option.bg(rgb(0x40_5070));
                        }
                        if asking {
                            option = option
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(0x30_3030)))
                                .on_click(cx.listener(move |view, _, _, cx| view.choose(i, cx)));
                        }
                        option
                    }))
                    .into_any_element()
            }
        }
    }
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
