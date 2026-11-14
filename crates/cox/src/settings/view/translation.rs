//! The translation on the settings screen: the language the human reads
//! the wiki, and what they select, in, and what translates it, an agent
//! Cox knows or a command, as `settings.json` keeps it under
//! `translation` (see [`crate::settings::Translation`]). Enter, or the
//! keyboard leaving it, keeps the language, or the command, and a click
//! keeps the choice of what translates; emptied, the language takes the translation out, so that
//! Cox translates nothing. What else the file holds of it stays.

use gpui::{App, Context, Entity, Focusable, Subscription, Window, div, prelude::*, px, rgb};
use serde_json::{Map, Value};

use super::chip;
use super::words::Words;
use crate::settings::window::AppAgents;
use crate::ui::button::INPUT_HEIGHT;
use crate::ui::input::{Blurred, LineInput, Submitted};
use crate::vim::hints::Hinted as _;
use coxd::agents::line;
use coxd::theme::{ACCENT, BORDER, FAILED, FAINT, FOREGROUND};

/// What translates.
#[derive(Debug, Clone, PartialEq, Eq)]
enum By {
    /// An agent, by its name, or the one a new chat talks to.
    Agent(Option<String>),
    /// The command the input holds.
    Command,
}

/// The translation, to change.
pub struct TranslationSection {
    language: Entity<LineInput>,
    command: Entity<LineInput>,
    by: By,
    /// What `settings.json` held under `translation` as the section read
    /// it last, or kept it.
    kept: Option<Value>,
    /// What the line under the section says, and in which color.
    told: Option<(String, u32)>,
    words: Words,
    _subscriptions: [Subscription; 4],
}

impl TranslationSection {
    /// The section, as `settings.json` holds the translation now, writing
    /// what it says in `words`.
    pub fn new(words: Words, cx: &mut Context<Self>) -> Self {
        let language = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_placeholder("Japanese", cx);
            input
        });
        let command = cx.new(|cx| {
            let mut input = LineInput::wrapping(cx);
            input.set_placeholder("plamo-translate --from {from} --to {to}", cx);
            input
        });
        let [language_submitted, command_submitted] = [&language, &command]
            .map(|input| cx.subscribe(input, |section, _, Submitted, cx| section.save(cx)));
        let [language_left, command_left] = [&language, &command]
            .map(|input| cx.subscribe(input, |section, _, Blurred, cx| section.keep_typed(cx)));
        let mut section = Self {
            language,
            command,
            by: By::Agent(None),
            kept: Some(Value::Null),
            told: None,
            words,
            _subscriptions: [
                language_submitted,
                command_submitted,
                language_left,
                command_left,
            ],
        };
        section.reread(cx);
        section
    }

    /// Shows what `settings.json` holds now, when it changed since the
    /// section read it.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        let kept = crate::settings::kept("translation");
        if kept == self.kept {
            return;
        }
        let text = |key: &str| {
            kept.as_ref()
                .and_then(|kept| kept.get(key))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let language = text("language");
        let command: Option<Vec<String>> = kept
            .as_ref()
            .and_then(|kept| kept.get("command"))
            .and_then(Value::as_array)
            .map(|words| {
                words
                    .iter()
                    .filter_map(|word| word.as_str().map(str::to_owned))
                    .collect()
            });
        self.by = match &command {
            Some(_) => By::Command,
            None => By::Agent(Some(text("agent")).filter(|agent| !agent.is_empty())),
        };
        let command = command
            .map(|command| line::joined(&command))
            .unwrap_or_default();
        self.language
            .update(cx, |input, cx| input.set_text(&language, cx));
        self.command
            .update(cx, |input, cx| input.set_text(&command, cx));
        self.kept = kept;
        cx.notify();
    }

    /// What the section shows, for `cox --inspect state`.
    #[must_use]
    pub fn described(&self, cx: &App) -> String {
        let language = self.language.read(cx).text();
        let by = match &self.by {
            By::Agent(None) => "the agent a new chat talks to".to_owned(),
            By::Agent(Some(agent)) => format!("the agent {agent}"),
            By::Command => format!("the command {}", self.command.read(cx).text()),
        };
        let mut described = format!("Translation into {language:?}, by {by}");
        if let Some((told, _)) = &self.told {
            described.push_str(" — ");
            described.push_str(told);
        }
        described
    }

    /// Chooses `by` to translate, keeping it once there is a language.
    fn choose(&mut self, by: By, cx: &mut Context<Self>) {
        self.by = by;
        if self.language.read(cx).text().trim().is_empty() {
            self.tell("Name the language first.", FAINT, cx);
        } else if self.by != By::Command || !self.command.read(cx).text().trim().is_empty() {
            self.save(cx);
        } else {
            self.tell(
                "Write the command, which is kept on Enter or as you leave it.",
                FAINT,
                cx,
            );
        }
        cx.notify();
    }

    /// Keeps the language, or the command, the inputs hold, when it is not
    /// what is kept, as when the keyboard leaves them, or the screen
    /// closes: with no command yet, it waits for one.
    pub fn keep_typed(&mut self, cx: &mut Context<Self>) {
        let kept = |key: &str| self.kept.as_ref().and_then(|kept| kept.get(key));
        let language = self.language.read(cx).text().trim().to_owned();
        let kept_language = kept("language").and_then(Value::as_str).unwrap_or_default();
        let command = self.command.read(cx).text().trim().to_owned();
        let kept_command = kept("command")
            .and_then(Value::as_array)
            .map(|words| {
                let words: Vec<String> = words
                    .iter()
                    .filter_map(|word| word.as_str().map(str::to_owned))
                    .collect();
                line::joined(&words)
            })
            .unwrap_or_default();
        let by_command = self.by == By::Command;
        if language == kept_language && (!by_command || command == kept_command) {
            return;
        }
        // Translation is off, and stays so until there is a language.
        if language.is_empty() && kept_language.is_empty() {
            return;
        }
        if by_command && command.is_empty() && !language.is_empty() {
            return self.tell(
                "Write the command, which is kept on Enter or as you leave it.",
                FAINT,
                cx,
            );
        }
        self.save(cx);
    }

    /// Keeps the translation as the section shows it, or, with no
    /// language, takes it out.
    fn save(&mut self, cx: &mut Context<Self>) {
        let language = self.language.read(cx).text().trim().to_owned();
        if language.is_empty() {
            if let Err(error) = crate::settings::keep("translation", None) {
                return self.tell(format!("Cox cannot keep it: {error:#}"), FAILED, cx);
            }
            self.kept = None;
            return self.tell("Off: Cox translates nothing.", ACCENT, cx);
        }
        let mut translation = match &self.kept {
            Some(Value::Object(kept)) => kept.clone(),
            _ => Map::new(),
        };
        translation.insert("language".to_owned(), Value::String(language));
        match &self.by {
            By::Agent(agent) => {
                translation.remove("command");
                match agent {
                    Some(agent) => {
                        translation.insert("agent".to_owned(), Value::String(agent.clone()));
                    }
                    None => {
                        translation.remove("agent");
                    }
                }
            }
            By::Command => {
                let command = match line::split(self.command.read(cx).text()) {
                    Ok(command) if !command.is_empty() => command,
                    Ok(_) => return self.tell("Write the command that translates.", FAILED, cx),
                    Err(why) => return self.tell(why, FAILED, cx),
                };
                translation.remove("agent");
                translation.insert(
                    "command".to_owned(),
                    Value::Array(command.into_iter().map(Value::String).collect()),
                );
            }
        }
        let translation = Value::Object(translation);
        if let Err(error) = crate::settings::keep("translation", Some(translation.clone())) {
            return self.tell(format!("Cox cannot keep it: {error:#}"), FAILED, cx);
        }
        self.kept = Some(translation);
        self.tell("Set.", ACCENT, cx);
    }

    fn tell(&mut self, told: impl Into<String>, color: u32, cx: &mut Context<Self>) {
        self.told = Some((told.into(), color));
        cx.notify();
    }

    /// The agents that can translate: the human's, as they name them, and
    /// the one the settings name, if it is none of them.
    fn agents(&self, cx: &App) -> Vec<String> {
        let mut names: Vec<String> = cx
            .try_global::<AppAgents>()
            .map(|agents| agents.0.all())
            .unwrap_or_default()
            .into_iter()
            .filter(|known| !known.listed && !known.command.is_empty())
            .map(|known| known.name)
            .collect();
        if let By::Agent(Some(agent)) = &self.by
            && !names.contains(agent)
        {
            names.push(agent.clone());
        }
        names
    }
}

impl Render for TranslationSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = |input: &Entity<LineInput>| {
            div()
                .min_h(px(INPUT_HEIGHT))
                .px_2()
                .py(px(4.))
                .flex()
                .items_center()
                .border_1()
                .border_color(rgb(BORDER))
                .rounded(px(10.))
                .text_size(px(13.))
                .text_color(rgb(FOREGROUND))
                .child(input.clone())
        };
        let mut chips = vec![
            chip(
                "translate-by-default".to_owned(),
                "The default agent".to_owned(),
                self.by == By::Agent(None),
            )
            .on_hint_click(cx.listener(|section, _, _, cx| section.choose(By::Agent(None), cx))),
        ];
        for name in self.agents(cx) {
            let chosen = self.by == By::Agent(Some(name.clone()));
            let by = By::Agent(Some(name.clone()));
            chips.push(
                chip(format!("translate-by-{name}"), name, chosen).on_hint_click(
                    cx.listener(move |section, _, _, cx| section.choose(by.clone(), cx)),
                ),
            );
        }
        chips.push(
            chip(
                "translate-by-command".to_owned(),
                "A command".to_owned(),
                self.by == By::Command,
            )
            .on_hint_click(cx.listener(|section, _, window, cx| {
                window.focus(&section.command.focus_handle(cx));
                section.choose(By::Command, cx);
            })),
        );
        let (told, color) = self.told.clone().unwrap_or_else(|| {
            (
                "The language, or the command, is kept on Enter or as you leave it; \
                 emptied, the language turns translation off."
                    .to_owned(),
                FAINT,
            )
        });
        let mut words = self.words.part();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(words.heading("TRANSLATION"))
            .child(words.about(
                "The language you read the wiki, and what you select, in: English goes into \
                 it, and the rest into English, by an agent, or by a command that reads the \
                 text on its input, with {from} and {to} standing for the languages.",
            ))
            .child(
                input(&self.language)
                    .mt_1()
                    .debug_selector(|| "translation-language".to_owned()),
            )
            .child(div().flex().flex_wrap().gap_2().children(chips))
            .when(self.by == By::Command, |section| {
                section
                    .child(input(&self.command).debug_selector(|| "translation-command".to_owned()))
            })
            .child(words.note(&told, color))
    }
}
