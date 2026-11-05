//! The settings screen: a card over the window, which the gear at the
//! foot of the rail opens, as Command-, does (Control-, off macOS).
//!
//! It shows Cox's home, the folder chats in no project work in, in an
//! input, where Enter sets what the human typed, a whole path or one from
//! `~`, and "Choose…" asks for a folder and sets it. Set, the home is
//! kept (see [`super::keep_home`]), and the window opens the chats in no
//! project there from then on. Escape, the ×, or a click beside the card
//! closes it, leaving the home as it was last set.

use std::path::PathBuf;

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, PathPromptOptions, Subscription, Window, actions, div, prelude::*, px, rgb,
};

use crate::ui::card;
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT, BORDER, BUTTON, BUTTON_HOVER, DIM, FAILED, FAINT, FOREGROUND, HOVER, LINK,
};

actions!(
    cox,
    [
        /// Opens the settings screen.
        OpenSettings,
        /// Closes the settings screen.
        CloseSettings,
    ]
);

/// The key context of the settings screen.
const KEY_CONTEXT: &str = "Settings";

/// How wide the card is.
const WIDTH: f32 = 560.;

/// Binds the key that closes the screen. The window binds the one that
/// opens it.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseSettings, Some(KEY_CONTEXT))]);
}

/// The human set the home to this folder, which is kept.
pub struct HomeChanged(pub PathBuf);

/// The human closed the screen.
pub struct Dismissed;

/// What the line under the input says.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Told {
    /// How to set the home.
    Hint,
    /// The home was set, and kept.
    Set,
    /// What the human typed is not a folder, or it could not be kept.
    Failed(String),
}

pub struct SettingsView {
    /// The home the window has now.
    home: PathBuf,
    /// Whether `--home` gave the home, for this run, ahead of the
    /// setting, and the human has not set it since.
    given: bool,
    input: Entity<LineInput>,
    told: Told,
    _submitted: Subscription,
}

impl SettingsView {
    /// The screen of the window whose chats in no project work in `home`,
    /// which `--home` gave when `given` says so.
    pub fn new(home: PathBuf, given: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_text(&shortened(&home), cx);
            input.set_placeholder("~/Cox", cx);
            input
        });
        let submitted = cx.subscribe_in(&input, window, |view, _, Submitted, _, cx| {
            view.submit(cx);
        });
        Self {
            home,
            given,
            input,
            told: Told::Hint,
            _submitted: submitted,
        }
    }

    /// The input of the home's folder.
    #[must_use]
    pub fn input(&self) -> &Entity<LineInput> {
        &self.input
    }

    /// Sets the home to the folder the input names, if it names one.
    fn submit(&mut self, cx: &mut Context<Self>) {
        let user = std::env::var_os("HOME").map(PathBuf::from);
        let text = self.input.read(cx).text().to_owned();
        match super::folder_of(&text, user.as_deref()) {
            Ok(folder) => self.set(folder, cx),
            Err(why) => self.tell(Told::Failed(why.to_owned()), cx),
        }
    }

    /// Sets the home to `folder`, and keeps it.
    fn set(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        if let Err(error) = super::keep_home(&folder) {
            tracing::error!("the home is not kept: {error:#}");
            self.tell(Told::Failed(format!("Cox cannot keep it: {error:#}")), cx);
            return;
        }
        let shown = shortened(&folder);
        self.input
            .update(cx, |input, cx| input.set_text(&shown, cx));
        self.home.clone_from(&folder);
        self.given = false;
        self.tell(Told::Set, cx);
        cx.emit(HomeChanged(folder));
    }

    fn tell(&mut self, told: Told, cx: &mut Context<Self>) {
        self.told = told;
        cx.notify();
    }

    /// Asks the human for a folder, and sets the home to it.
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
            this.update(cx, |view, cx| view.set(folder, cx)).ok();
        })
        .detach();
    }

    /// What the line under the input says, and in which color.
    fn told(&self) -> (String, u32) {
        match &self.told {
            Told::Failed(why) => (why.clone(), FAILED),
            Told::Set => (
                format!(
                    "Set: chats in no project open in {} from now on.",
                    shortened(&self.home)
                ),
                ACCENT,
            ),
            Told::Hint if self.given => (
                "Cox was started with --home, which stands for this run; \
                 what is set here goes from now on, and for the runs without it."
                    .to_owned(),
                FAINT,
            ),
            Told::Hint => (
                "Enter sets the path typed, from / or ~; Escape closes.".to_owned(),
                FAINT,
            ),
        }
    }

    /// The button that asks for a folder.
    fn render_choose(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("choose-home")
            .debug_selector(|| "choose-home".to_owned())
            .flex_shrink_0()
            .h(px(32.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(10.))
            .bg(rgb(BUTTON))
            .text_size(px(13.))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON_HOVER)))
            .child(icon(Icon::Folder, 14., DIM))
            .child("Choose…")
            .on_click(cx.listener(|_, _, window, cx| Self::choose(window, cx)))
    }
}

impl EventEmitter<HomeChanged> for SettingsView {}
impl EventEmitter<Dismissed> for SettingsView {}

impl Focusable for SettingsView {
    /// The input, which has the keyboard while the screen is open.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (told, color) = self.told();
        card()
            .id("settings")
            .debug_selector(|| "settings".to_owned())
            .key_context(KEY_CONTEXT)
            .occlude()
            .w(px(WIDTH))
            .max_w_full()
            .flex()
            .flex_col()
            .shadow_lg()
            // A click on the card stays there, not closing it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|_, _: &CloseSettings, _, cx| cx.emit(Dismissed)))
            .child(
                div()
                    .px(px(22.))
                    .py(px(16.))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    )
                    .child(
                        div()
                            .id("close-settings")
                            .debug_selector(|| "close-settings".to_owned())
                            .size(px(26.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(HOVER)))
                            .child(icon(Icon::Close, 14., FAINT))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Dismissed))),
                    ),
            )
            .child(
                div()
                    .px(px(22.))
                    .pt(px(18.))
                    .pb(px(22.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(FAINT))
                            .child("HOME"),
                    )
                    .child(div().text_size(px(13.)).text_color(rgb(DIM)).child(
                        "The folder Cox starts chats in when they are in no project, \
                         as New chat opens them; the rail shows it as ~.",
                    ))
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .debug_selector(|| "home-input".to_owned())
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(32.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .border_1()
                                    .border_color(rgb(LINK))
                                    .rounded(px(10.))
                                    .text_size(px(13.))
                                    .text_color(rgb(FOREGROUND))
                                    .child(self.input.clone()),
                            )
                            .child(Self::render_choose(cx)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "home-told".to_owned())
                            .text_size(px(12.))
                            .text_color(rgb(color))
                            .child(told),
                    ),
            )
    }
}
