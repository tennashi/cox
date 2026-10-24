//! One session in the window: draws its conversation, with its stage
//! beside it, and turns keystrokes and clicks into input for it. Escape in
//! the conversation, or its Stop button, stops the turn the agent is
//! taking.

use gpui::{
    AnyView, App, Context, Entity, FocusHandle, Focusable, KeyBinding, MouseButton, ScrollHandle,
    SharedString, Subscription, Task, Window, actions, div, prelude::*, px, rgb,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::{Agent, AgentThread};
use crate::conversation::{Command, Conversation, Entry, State};
use crate::input::{LineInput, Submitted};

actions!(
    cox,
    [
        /// Stops the turn the agent is taking.
        Stop,
    ]
);

const KEY_CONTEXT: &str = "Conversation";

/// Binds the keys of the conversation. Escape is the stage's while the
/// stage has the keyboard, and the input method's while it composes.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Stop, Some(KEY_CONTEXT))]);
}

/// How wide the conversation is when the stage sits beside it.
const CONVERSATION_WIDTH: f32 = 420.;

pub struct SessionView {
    conversation: Conversation,
    /// Where the conversation is scrolled to.
    scroll: ScrollHandle,
    /// The line the human is typing.
    input: Entity<LineInput>,
    /// The page the agent and the human work on, beside the conversation.
    stage: Option<AnyView>,
    commands: UnboundedSender<Command>,
    /// The agent lives as long as the window: dropping this stops it.
    _agent: AgentThread,
    _events: Task<()>,
    _submitted: Subscription,
}

impl SessionView {
    /// Opens on `agent`, with `stage` beside the conversation when there is
    /// one.
    pub fn new(agent: Agent, stage: Option<impl Into<AnyView>>, cx: &mut Context<Self>) -> Self {
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
        match entry {
            Entry::Human(text) => div()
                .text_color(rgb(0x7f_b4ff))
                .child(format!("> {text}"))
                .into_any_element(),
            Entry::Reply(text) => div().child(text.clone()).into_any_element(),
            Entry::ToolCall { title, result, .. } => {
                let state = match result {
                    Some(result) => format!("{result:?}"),
                    None => "running".to_owned(),
                };
                div()
                    .text_color(rgb(0xa0_a0a0))
                    .child(format!("[tool: {title} … {state}]"))
                    .into_any_element()
            }
            Entry::Note(text) => div()
                .text_color(rgb(0xa0_a0a0))
                .child(text.clone())
                .into_any_element(),
            Entry::Permission {
                title,
                options,
                chosen,
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
                    .child(format!("[permission: {title}]"))
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
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("conversation")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
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
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_, _, window, cx| {
                            window.focus(&input.focus_handle(cx));
                            window.prevent_default();
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
            .font_family("monospace")
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
