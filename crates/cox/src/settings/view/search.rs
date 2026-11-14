//! The search engine on the settings screen: the address of its results,
//! with `%s` where the words go, as `settings.json` keeps it under
//! `search` (see [`crate::settings::SearchEngine`]), in an input that
//! keeps it on Enter or as the keyboard leaves it, or, emptied, goes by
//! Google's.

use gpui::{Context, Entity, Subscription, Window, div, prelude::*, px, rgb};
use serde_json::Value;

use super::words::Words;
use crate::settings::{DEFAULT_SEARCH, SearchEngine};
use crate::ui::button::INPUT_HEIGHT;
use crate::ui::input::{Blurred, LineInput, Submitted};
use coxd::theme::{ACCENT, BORDER, FAILED, FAINT, FOREGROUND};

/// The key of the engine in `settings.json`.
const KEY: &str = "search";

/// The engine the words typed after Vim's `t`, `:open` or `:tabopen` are
/// searched with.
pub struct SearchSection {
    input: Entity<LineInput>,
    /// What `settings.json` held as the section read it last.
    kept: Option<Value>,
    /// What the line under the input says, and in which color.
    told: Option<(String, u32)>,
    words: Words,
    _subscriptions: [Subscription; 2],
}

impl SearchSection {
    /// The section, as `settings.json` says now, writing what it says in
    /// `words`.
    pub fn new(words: Words, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder(DEFAULT_SEARCH, cx);
            input
        });
        let subscriptions = [
            cx.subscribe(&input, |section, _, Submitted, cx| section.submit(cx)),
            cx.subscribe(&input, |section, _, Blurred, cx| section.keep_typed(cx)),
        ];
        let mut section = Self {
            input,
            kept: None,
            told: None,
            words,
            _subscriptions: subscriptions,
        };
        section.reread(cx);
        section
    }

    /// Shows what `settings.json` holds now, when it changed since the
    /// section read it, saying what is wrong with it, if anything is.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        let kept = crate::settings::kept(KEY);
        if kept == self.kept {
            return;
        }
        let text = match &kept {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        self.input.update(cx, |input, cx| input.set_text(&text, cx));
        self.told = kept.as_ref().and_then(|_| {
            SearchEngine::of(&text)
                .err()
                .map(|why| (format!("{why} Google is searched until it is."), FAILED))
        });
        self.kept = kept;
        cx.notify();
    }

    /// What the section shows, for `cox --inspect state`.
    #[must_use]
    pub fn described(&self, cx: &gpui::App) -> String {
        let told = self.told.as_ref().map_or("", |(told, _)| told.as_str());
        format!(
            "Search engine input holding {:?}, saying {told:?}",
            self.input.read(cx).text()
        )
    }

    /// Keeps what the input holds, when it is not what is kept, as when
    /// the keyboard leaves it, or the screen closes.
    pub fn keep_typed(&mut self, cx: &mut Context<Self>) {
        let kept = self
            .kept
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or_default();
        if self.input.read(cx).text().trim() != kept {
            self.submit(cx);
        }
    }

    /// Keeps the engine the input names, or, emptied, none. One that is
    /// wrong is not kept, and the line under the input says why.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().trim().to_owned();
        let engine = if text.is_empty() {
            None
        } else {
            match SearchEngine::of(&text) {
                Ok(engine) => Some(engine),
                Err(why) => return self.tell(why.to_owned(), FAILED, cx),
            }
        };
        let value = engine.map(|engine| Value::String(engine.as_str().to_owned()));
        if let Err(error) = crate::settings::keep(KEY, value.clone()) {
            tracing::error!("the search engine is not kept: {error:#}");
            return self.tell(format!("Cox cannot keep it: {error:#}"), FAILED, cx);
        }
        self.input.update(cx, |input, cx| input.set_text(&text, cx));
        let told = if value.is_some() {
            "Set."
        } else {
            "Google is searched."
        };
        self.kept = value;
        self.tell(told.to_owned(), ACCENT, cx);
    }

    fn tell(&mut self, told: String, color: u32, cx: &mut Context<Self>) {
        self.told = Some((told, color));
        cx.notify();
    }
}

impl Render for SearchSection {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut words = self.words.part();
        let (told, color) = self.told.clone().unwrap_or_else(|| {
            (
                "The address of the results, from https://, with %s where the words go, is kept \
                 on Enter or as you leave it; emptied, Google is searched."
                    .to_owned(),
                FAINT,
            )
        });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(words.heading("SEARCH ENGINE"))
            .child(words.about(
                "Where the words typed after Vim's t, :open or :tabopen are searched for, \
                 when they are no address, file or path; !command runs the command.",
            ))
            .child(
                div()
                    .debug_selector(|| "search-input".to_owned())
                    .mt_1()
                    .h(px(INPUT_HEIGHT))
                    .px_2()
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .rounded(px(10.))
                    .text_size(px(13.))
                    .text_color(rgb(FOREGROUND))
                    .child(self.input.clone()),
            )
            .child(words.note(&told, color))
    }
}
