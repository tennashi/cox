//! One session in the window: draws its conversation, with its stage
//! beside it, and turns keystrokes and clicks into input for it. Escape in
//! the conversation, or its Stop button, stops the turn the agent is
//! taking.
//!
//! The human selects the conversation's text with the mouse, a word with
//! a double click and a whole paragraph or block with a triple click, and
//! copies it with the usual key while the input has nothing selected. A
//! click on a link opens it on the session's stage.

use std::rc::Rc;

use gpui::{
    AnyView, App, ClipboardItem, Context, Entity, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollHandle, SharedString, Subscription, Task,
    Window, actions, div, prelude::*, px, rgb,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::{Agent, AgentThread};
use crate::conversation::{Command, Content, Conversation, Entry, State};
use crate::diff::Line;
use crate::input::{LineInput, Submitted};
use crate::selection::Selection;
use crate::text::{Painted, Writer};

actions!(
    cox,
    [
        /// Stops the turn the agent is taking.
        Stop,
        /// Copies the text selected in the conversation.
        Copy,
    ]
);

const KEY_CONTEXT: &str = "Conversation";

/// Binds the keys of the conversation. Escape is the stage's while the
/// stage has the keyboard, and the input method's while it composes.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Stop, Some(KEY_CONTEXT)),
        // The secondary key is Command on macOS, Control elsewhere.
        KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT)),
    ]);
}

/// Opens a link the human clicked: on the session's stage, where it has
/// one.
pub type OpenLink = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// The font the window is drawn in: a monospaced family the system has.
/// A generic name such as `monospace` finds no font, and GPUI then falls
/// back to a font with no bold.
#[cfg(target_os = "macos")]
pub const FONT: &str = "Menlo";
#[cfg(not(target_os = "macos"))]
pub const FONT: &str = "DejaVu Sans Mono";

/// How wide the conversation is when the stage sits beside it.
const CONVERSATION_WIDTH: f32 = 420.;

/// How tall what a tool call shows grows before it scrolls on its own.
const CONTENT_HEIGHT: f32 = 320.;

/// The colors of a change's lines.
const REMOVED: u32 = 0xf0_8080;
const ADDED: u32 = 0x80_d080;
const DIM: u32 = 0xa0_a0a0;

pub struct SessionView {
    conversation: Conversation,
    /// Where the conversation is scrolled to.
    scroll: ScrollHandle,
    /// The line the human is typing.
    input: Entity<LineInput>,
    /// The page the agent and the human work on, beside the conversation.
    stage: Option<AnyView>,
    open_link: OpenLink,
    /// The conversation's text as the last frame laid it out.
    painted: Painted,
    /// The text the human selected in the conversation, if any.
    selection: Option<Selection>,
    /// Whether a mouse button went down on the conversation and is still
    /// held.
    selecting: bool,
    commands: UnboundedSender<Command>,
    /// The agent lives as long as the window: dropping this stops it.
    _agent: AgentThread,
    _events: Task<()>,
    _submitted: Subscription,
}

impl SessionView {
    /// Opens on `agent`, with `stage` beside the conversation when there is
    /// one. Links the human clicks go to `open_link`.
    pub fn new(
        agent: Agent,
        stage: Option<impl Into<AnyView>>,
        open_link: OpenLink,
        cx: &mut Context<Self>,
    ) -> Self {
        let Agent {
            commands,
            mut events,
            thread,
        } = agent;
        let task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let shown = this.update(cx, |view, cx| {
                    view.follow(|conversation| conversation.apply(event));
                    cx.notify();
                });
                if shown.is_err() {
                    break;
                }
            }
        });
        let input = cx.new(LineInput::wrapping);
        let submitted = cx.subscribe(&input, |view, input, Submitted, cx| {
            view.submit(&input, cx);
        });
        Self {
            conversation: Conversation::new(),
            scroll: ScrollHandle::new(),
            input,
            stage: stage.map(Into::into),
            open_link,
            painted: Painted::default(),
            selection: None,
            selecting: false,
            commands,
            _agent: thread,
            _events: task,
            _submitted: submitted,
        }
    }

    #[must_use]
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    #[must_use]
    pub fn input(&self) -> &Entity<LineInput> {
        &self.input
    }

    /// Where the conversation is scrolled to.
    #[must_use]
    pub fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// The text the human selected in the conversation.
    #[must_use]
    pub fn selected_text(&self) -> String {
        self.selection
            .map(|selection| self.painted.copy(&selection))
            .unwrap_or_default()
    }

    /// A button went down on the conversation: start selecting there, a
    /// word or a whole piece of text at a time on a double or triple
    /// click. The keyboard goes to the input, so that the human copies
    /// what they select, and types on.
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.input.focus_handle(cx));
        window.prevent_default();
        self.input.update(cx, LineInput::deselect);
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
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if let (Some(selection), Some(spot)) =
            (&mut self.selection, self.painted.spot_at(event.position))
            && selection.head != spot
        {
            selection.head = spot;
            cx.notify();
        }
    }

    /// The button came up: a click that selected nothing opens the link
    /// under it.
    fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let clicked = std::mem::take(&mut self.selecting)
            && self.selection.is_none_or(|selection| selection.is_empty());
        if clicked && let Some(url) = self.painted.link_at(event.position) {
            (self.open_link)(&url, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Changes the conversation with `change`, keeping its end in view if
    /// it was: the human reads along as entries arrive, unless they have
    /// scrolled back to read something earlier.
    fn follow<T>(&mut self, change: impl FnOnce(&mut Conversation) -> T) -> T {
        // The offset grows downward as negative numbers.
        let at_end = self.scroll.offset().y <= -self.scroll.max_offset().height + px(1.);
        let changed = change(&mut self.conversation);
        if at_end {
            self.scroll.scroll_to_bottom();
        }
        changed
    }

    /// The human pressed Enter: send the text, or answer the permission
    /// request with the number typed.
    fn submit(&mut self, input: &Entity<LineInput>, cx: &mut Context<Self>) {
        let text = input.read(cx).text().to_owned();
        if let Some(command) = self.follow(|conversation| conversation.submit(&text)) {
            self.send(command);
            input.update(cx, LineInput::clear);
            cx.notify();
        }
        // Otherwise the text stays: a turn is running, or the number was
        // not one of the options.
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(command) = self.conversation.choose(index) {
            self.send(command);
            cx.notify();
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(command) = self.conversation.cancel() {
            self.send(command);
            cx.notify();
        }
    }

    fn send(&self, command: Command) {
        // The agent side is gone once the connection closes, which the
        // conversation shows when the `Closed` event arrives.
        let _ = self.commands.send(command);
    }

    fn render_entry(
        &self,
        index: usize,
        entry: &Entry,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut writer = Writer::new(&self.painted, self.selection, index);
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

impl Focusable for SessionView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl SessionView {
    fn render_conversation(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let prompt = match self.conversation.state() {
            State::Idle => "> ",
            State::Busy => "… ",
            State::Asking => "choose> ",
            State::Stopping => "stopping… ",
            State::Closed => "closed ",
        };
        let running = matches!(self.conversation.state(), State::Busy | State::Asking);
        // The frame lays the text out again.
        self.painted.clear();
        let entries: Vec<_> = self
            .conversation
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| self.render_entry(index, entry, cx))
            .collect();
        let input = self.input.clone();
        div()
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|view, _: &Stop, _, cx| view.stop(cx)))
            .on_action(cx.listener(Self::copy))
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("conversation")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|view, _, _, _| view.selecting = false),
                    )
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(entries),
            )
            .child(
                div()
                    .debug_selector(|| "input".to_owned())
                    .flex()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(0x40_4040))
                    // A click brings the keyboard back from the stage, and
                    // keeps it from the window, which takes it otherwise.
                    // What was selected in the conversation is let go, so
                    // that the keys copy from the input.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, window, cx| {
                            window.focus(&input.focus_handle(cx));
                            window.prevent_default();
                            if view.selection.take().is_some() {
                                cx.notify();
                            }
                        }),
                    )
                    .child(prompt)
                    .child(div().flex_1().min_w_0().child(self.input.clone()))
                    .when(running, |bar| {
                        bar.child(
                            div()
                                .id("stop")
                                .debug_selector(|| "stop".to_owned())
                                .flex_shrink_0()
                                .ml_2()
                                .px_2()
                                .rounded_sm()
                                .bg(rgb(0x30_3030))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(0x40_4040)))
                                .child("Stop")
                                .on_click(cx.listener(|view, _, _, cx| view.stop(cx))),
                        )
                    }),
            )
    }
}

impl Render for SessionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let conversation = self.render_conversation(cx).into_any_element();
        let root = div()
            .size_full()
            .flex()
            .bg(rgb(0x1e_1e1e))
            .text_color(rgb(0xe0_e0e0))
            .font_family(FONT)
            .text_size(px(14.));
        match self.stage.clone() {
            Some(stage) => root
                .child(
                    div()
                        .w(px(CONVERSATION_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .border_r_1()
                        .border_color(rgb(0x40_4040))
                        .child(conversation),
                )
                .child(div().flex_1().h_full().child(stage)),
            None => root.child(conversation),
        }
    }
}
