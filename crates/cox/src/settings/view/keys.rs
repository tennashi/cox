//! Vim's keys on the settings screen: whether the keys work as in Vim,
//! as `settings.json` keeps it under `vim` (see [`crate::vim`]). A click
//! keeps the choice, and the keys work so from the next one.

use gpui::{Context, Window, div, prelude::*};
use serde_json::Value;

use super::chip;
use super::words::Words;
use crate::vim::hints::Hinted as _;
use coxd::theme::{FAILED, FAINT};

/// Whether the keys work as in Vim, to change.
pub struct KeysSection {
    on: bool,
    /// Why the choice could not be kept, if it could not.
    failed: Option<String>,
    words: Words,
}

impl KeysSection {
    /// The section, as `settings.json` says now, writing what it says in
    /// `words`.
    pub fn new(words: Words) -> Self {
        Self {
            on: coxd::settings::vim(),
            failed: None,
            words,
        }
    }

    /// Shows what `settings.json` says now.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        self.on = coxd::settings::vim();
        cx.notify();
    }

    /// What the section shows, for `cox --inspect state`.
    #[must_use]
    pub fn described(&self) -> String {
        format!("Vim's keys {}", if self.on { "on" } else { "off" })
    }

    /// Keeps `on`, and turns Vim's keys on or off with it.
    fn choose(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = coxd::settings::keep("vim", on.then_some(Value::Bool(true))) {
            tracing::error!("Vim's keys are not kept: {error:#}");
            self.failed = Some(format!("Cox cannot keep it: {error:#}"));
            cx.notify();
            return;
        }
        self.on = on;
        self.failed = None;
        crate::vim::switch(on, window, cx);
        cx.notify();
    }
}

impl Render for KeysSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut words = self.words.part();
        let chips = [(false, "vim-off", "Off"), (true, "vim-on", "On")].map(|(on, id, label)| {
            chip(id.to_owned(), label.to_owned(), self.on == on).on_hint_click(
                cx.listener(move |section, _, window, cx| section.choose(on, window, cx)),
            )
        });
        let (told, color) = self.failed.as_ref().map_or(
            (
                "Esc goes to normal mode, where j and k move, : opens a line of commands, \
                 and i types again; Ctrl-C stops the agent. Takes effect at once.",
                FAINT,
            ),
            |why| (why.as_str(), FAILED),
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(words.heading("VIM'S KEYS"))
            .child(words.about(
                "The keys work as in Vim across the window, and in a web page as in \
                 Vimperator.",
            ))
            .child(div().flex().flex_wrap().gap_2().children(chips))
            .child(words.note(told, color))
    }
}
