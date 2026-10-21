//! The window: draws the conversation and turns keystrokes and clicks into
//! input for it.

use gpui::{
    App, Context, FocusHandle, Focusable, KeyDownEvent, SharedString, Task, Window, div,
    prelude::*, px, rgb,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::Agent;
use crate::conversation::{Command, Conversation, Entry, State};

pub struct CoxView {
    conversation: Conversation,
    /// The line the human is typing.
    input: String,
    commands: UnboundedSender<Command>,
    focus_handle: FocusHandle,
    _events: Task<()>,
}

impl CoxView {
    pub fn new(agent: Agent, cx: &mut Context<Self>) -> Self {
        let Agent {
            commands,
            mut events,
        } = agent;
        let task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let shown = this.update(cx, |view, cx| {
                    view.conversation.apply(event);
                    cx.notify();
                });
                if shown.is_err() {
                    break;
                }
            }
        });
        Self {
            conversation: Conversation::new(),
            input: String::new(),
            commands,
            focus_handle: cx.focus_handle(),
            _events: task,
        }
    }

    #[must_use]
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        match keystroke.key.as_str() {
            "enter" => {
                let input = std::mem::take(&mut self.input);
                if let Some(command) = self.conversation.submit(&input) {
                    self.send(command);
                } else if self.conversation.state() == State::Busy {
                    // Keep what the human typed until the turn ends.
                    self.input = input;
                }
            }
            "backspace" => {
                self.input.pop();
            }
            _ => {
                let modifiers = keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform {
                    return;
                }
                let Some(text) = &keystroke.key_char else {
                    return;
                };
                self.input.push_str(text);
            }
        }
        cx.notify();
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(command) = self.conversation.choose(index) {
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
            Entry::ToolCall { title, result } => div()
                .text_color(rgb(0xa0_a0a0))
                .child(format!("[tool: {title} … {result:?}]"))
                .into_any_element(),
            Entry::Note(text) => div()
                .text_color(rgb(0xa0_a0a0))
                .child(text.clone())
                .into_any_element(),
            Entry::Permission {
                title,
                options,
                chosen,
            } => {
                let asking = chosen.is_none() && index + 1 == self.conversation.entries().len();
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

impl Focusable for CoxView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CoxView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prompt = match self.conversation.state() {
            State::Idle => "> ",
            State::Busy => "… ",
            State::Asking => "choose> ",
            State::Closed => "closed ",
        };
        let entries: Vec<_> = self
            .conversation
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| self.render_entry(index, entry, cx))
            .collect();
        div()
            .key_context("Cox")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e_1e1e))
            .text_color(rgb(0xe0_e0e0))
            .font_family("monospace")
            .text_size(px(14.))
            .child(
                div()
                    .id("conversation")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(entries),
            )
            .child(
                div()
                    .debug_selector(|| "input".to_owned())
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(0x40_4040))
                    .child(format!("{prompt}{}▏", self.input)),
            )
    }
}
