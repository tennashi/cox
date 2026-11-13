//! A folder `settings.json` names, on the settings screen: the human's
//! workflow or their wiki, in an input that keeps the path typed, whole
//! or from `~`, on Enter or as the keyboard leaves it, kept from `~` when it is in the home directory, as
//! the human would write it, or, emptied, Cox's own folder in its place;
//! and "Choose…" asks for a folder and keeps it.

use std::path::PathBuf;

use gpui::{
    Context, Entity, PathPromptOptions, SharedString, Subscription, Window, div, prelude::*, px,
    rgb,
};
use serde_json::Value;

use super::words::Words;
use crate::ui::button::{INPUT_HEIGHT, Kind, button};
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{Blurred, LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{ACCENT, BORDER, DIM, FAILED, FAINT, FOREGROUND};
use crate::vim::hints::Hinted as _;

/// A folder of the settings, by its key in `settings.json`.
pub struct FolderField {
    key: &'static str,
    /// What the field is, as its heading says.
    title: &'static str,
    /// What the folder is for.
    about: &'static str,
    /// The folder Cox goes by when the settings name none.
    fallback: Option<PathBuf>,
    input: Entity<LineInput>,
    /// What `settings.json` named as the field read it last.
    kept: Option<Value>,
    /// What the line under the input says, and in which color.
    told: Option<(String, u32)>,
    words: Words,
    _subscriptions: [Subscription; 2],
}

impl FolderField {
    /// The field of `key`, headed `title`, saying `about`, which goes by
    /// `fallback` when the settings name no folder, writing what it says
    /// in `words`.
    pub fn new(
        key: &'static str,
        title: &'static str,
        about: &'static str,
        fallback: Option<PathBuf>,
        words: Words,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            if let Some(fallback) = &fallback {
                input.set_placeholder(shortened(fallback), cx);
            }
            input
        });
        let subscriptions = [
            cx.subscribe(&input, |field, _, Submitted, cx| field.submit(cx)),
            cx.subscribe(&input, |field, _, Blurred, cx| field.keep_typed(cx)),
        ];
        let mut field = Self {
            key,
            title,
            about,
            fallback,
            input,
            kept: None,
            told: None,
            words,
            _subscriptions: subscriptions,
        };
        field.reread(cx);
        field
    }

    /// Goes by `fallback` when the settings name no folder, as when the
    /// home it is in changed.
    pub fn set_fallback(&mut self, fallback: PathBuf, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_placeholder(shortened(&fallback), cx);
        });
        self.fallback = Some(fallback);
        cx.notify();
    }

    /// What the input holds.
    #[must_use]
    pub fn text(&self, cx: &gpui::App) -> String {
        self.input.read(cx).text().to_owned()
    }

    /// Shows what `settings.json` names now, when it changed since the
    /// field read it.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        let kept = crate::settings::kept(self.key);
        if kept != self.kept {
            let text = kept
                .as_ref()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            self.input.update(cx, |input, cx| input.set_text(&text, cx));
            self.kept = kept;
            cx.notify();
        }
    }

    /// Keeps what the input holds, when it is not what is kept, as when
    /// the keyboard leaves it, or the screen closes.
    pub fn keep_typed(&mut self, cx: &mut Context<Self>) {
        let kept = self
            .kept
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or_default();
        if self.text(cx).trim() != kept {
            self.submit(cx);
        }
    }

    /// Keeps the folder the input names, or, emptied, Cox's own.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let text = self.text(cx);
        if text.trim().is_empty() {
            return self.keep(None, cx);
        }
        let user = std::env::var_os("HOME").map(PathBuf::from);
        match crate::settings::folder_of(&text, user.as_deref()) {
            Ok(folder) => self.keep(Some(&folder), cx),
            Err(why) => self.tell(why, FAILED, cx),
        }
    }

    /// Keeps `folder`, from `~` when it is in the home directory, or
    /// names none.
    fn keep(&mut self, folder: Option<&std::path::Path>, cx: &mut Context<Self>) {
        let value = folder.map(|folder| Value::String(shortened(folder)));
        if let Err(error) = crate::settings::keep(self.key, value.clone()) {
            tracing::error!("the {} is not kept: {error:#}", self.key);
            return self.tell(format!("Cox cannot keep it: {error:#}"), FAILED, cx);
        }
        let text = value
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        self.input.update(cx, |input, cx| input.set_text(&text, cx));
        self.kept = value;
        let told = match (folder, &self.fallback) {
            (Some(_), _) => "Set.".to_owned(),
            (None, Some(fallback)) => format!("Cox's own, {}, is used.", shortened(fallback)),
            (None, None) => "None is named.".to_owned(),
        };
        self.tell(told, ACCENT, cx);
    }

    fn tell(&mut self, told: impl Into<String>, color: u32, cx: &mut Context<Self>) {
        self.told = Some((told.into(), color));
        cx.notify();
    }

    /// Asks the human for a folder, and keeps it.
    fn choose(window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let folder = match chosen.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.swap_remove(0),
                Ok(Err(error)) => {
                    tracing::error!("no folder chosen: {error:#}");
                    return;
                }
                _ => return,
            };
            this.update(cx, |field, cx| field.keep(Some(&folder), cx))
                .ok();
        })
        .detach();
    }
}

impl Render for FolderField {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.key;
        let (told, color) = self.told.clone().unwrap_or_else(|| {
            (
                "The path typed, from / or ~, is kept on Enter or as you leave it; emptied, \
                 Cox's own is used."
                    .to_owned(),
                FAINT,
            )
        });
        let choose: SharedString = format!("choose-{key}").into();
        let mut words = self.words.part();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(words.heading(self.title))
            .child(words.about(self.about))
            .child(
                div()
                    .mt_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(move || format!("{key}-input"))
                            .flex_1()
                            .min_w_0()
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
                    .child(
                        button(
                            choose,
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(icon(Icon::Folder, 14., DIM))
                                .child("Choose…"),
                            Kind::Plain,
                        )
                        .h(px(INPUT_HEIGHT))
                        .on_hint_click(cx.listener(|_, _, window, cx| Self::choose(window, cx))),
                    ),
            )
            .child(words.note(&told, color))
    }
}
