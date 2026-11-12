//! The settings screen: a card over the window, which the gear at the
//! foot of the rail opens, as Command-, does (Control-, off macOS).
//!
//! It shows Cox's home, the folder Cox's own chats work in, in an
//! input, which sets what the human typed, a whole path or one from `~`,
//! on Enter or as the keyboard leaves it, and "Choose…" asks for a folder
//! and sets it. Set, the home is
//! kept (see [`super::keep_home`]), and the window opens the chats in no
//! project there from then on. Under it are the folders of the human's
//! workflow and wiki (see [`folder`]), the agents they run (see
//! [`agents`]), the language servers of the code tabs (see [`servers`]),
//! the language they read in (see [`translation`]), and the code a phone
//! pairs with, beside the button that unpairs every phone (see
//! [`phone`]).
//! What the screen says the human selects, copies and translates (see
//! [`words`]).
//! Escape, the ×, or a click beside the card closes it, keeping what
//! the inputs hold that is not kept yet.

mod agents;
mod folder;
mod phone;
mod servers;
mod translation;
mod words;

use std::path::PathBuf;

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, PathPromptOptions, Subscription, Window, actions, div, prelude::*, px, rgb,
};

use self::agents::AgentsSection;
use self::folder::FolderField;
use self::phone::PhoneSection;
pub use self::phone::UnpairPhones;
use self::servers::ServersSection;
use self::translation::TranslationSection;
use self::words::Words;
use crate::ui::button::{INPUT_HEIGHT, Kind, button};
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{Blurred, LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{ACCENT, BORDER, DIM, FAILED, FAINT, FOREGROUND, HOVER, LINK};
use crate::ui::{Copy, card};

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
const WIDTH: f32 = 680.;

/// Binds the keys that close the screen and copy what is selected of it.
/// The window binds the one that opens it.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseSettings, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT)),
    ]);
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
    workflow: Entity<FolderField>,
    wiki: Entity<FolderField>,
    agents: Entity<AgentsSection>,
    servers: Entity<ServersSection>,
    translation: Entity<TranslationSection>,
    phone: Entity<PhoneSection>,
    /// What the screen says, as the human selects it.
    words: Words,
    /// What has the keyboard while the human selects the words.
    focus: FocusHandle,
    _subscriptions: [Subscription; 3],
}

impl SettingsView {
    /// The screen of the window whose own chats work in `home`,
    /// which `--home` gave when `given` says so.
    pub fn new(home: PathBuf, given: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_text(&shortened(&home), cx);
            input.set_placeholder("~/cox", cx);
            input
        });
        let words = Words::default();
        let phone = cx.new(|_| PhoneSection::new(words.clone()));
        let subscriptions = [
            cx.subscribe_in(&input, window, |view, _, Submitted, _, cx| {
                view.submit(cx);
            }),
            cx.subscribe_in(&input, window, |view, _, Blurred, _, cx| {
                view.keep_typed(cx);
            }),
            cx.subscribe(&phone, |_, _, UnpairPhones, cx| cx.emit(UnpairPhones)),
        ];
        Self {
            home,
            given,
            input,
            told: Told::Hint,
            workflow: cx.new(|cx| {
                FolderField::new(
                    "workflow",
                    "WORKFLOW",
                    "The folder of how you work with Cox, whatever the project: its COX.md, \
                     which every chat's agent is told first, and its sketch/ and wiki/, what Cox \
                     tells the agent in your place.",
                    Some(crate::settings::Folder::Workflow.in_home()),
                    words.clone(),
                    cx,
                )
            }),
            wiki: cx.new(|cx| {
                FolderField::new(
                    "wiki",
                    "WIKI",
                    "The folder of your wiki, one for all your projects, where the agents keep \
                     what they learned, as an Obsidian vault or a Git repository of yours.",
                    Some(crate::settings::Folder::Wiki.in_home()),
                    words.clone(),
                    cx,
                )
            }),
            agents: cx.new(|cx| AgentsSection::new(words.clone(), cx)),
            servers: cx.new(|cx| ServersSection::new(words.clone(), cx)),
            translation: cx.new(|cx| TranslationSection::new(words.clone(), cx)),
            phone,
            words,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The agents the screen lists.
    #[must_use]
    pub fn agents(&self) -> &Entity<AgentsSection> {
        &self.agents
    }

    /// Every phone is unpaired, as the human asked, unless `failed` says
    /// why not.
    pub fn phones_unpaired(&mut self, failed: Option<String>, cx: &mut Context<Self>) {
        self.phone
            .update(cx, |phone, cx| phone.unpaired(failed, cx));
    }

    /// Reads the settings again, as another program changed them.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        self.workflow.update(cx, FolderField::reread);
        self.wiki.update(cx, FolderField::reread);
        self.agents.update(cx, AgentsSection::reread);
        self.servers.update(cx, ServersSection::reread);
        self.translation.update(cx, TranslationSection::reread);
    }

    /// What the screen shows past the home and the agents, a line each, for
    /// `cox --inspect state`.
    #[must_use]
    pub fn described(&self, cx: &App) -> Vec<String> {
        let mut lines = vec![
            format!(
                "Workflow input holding {:?}",
                self.workflow.read(cx).text(cx)
            ),
            format!("Wiki input holding {:?}", self.wiki.read(cx).text(cx)),
        ];
        lines.extend(self.servers.read(cx).described(cx));
        lines.push(self.translation.read(cx).described(cx));
        lines.push(self.phone.read(cx).described());
        lines
    }

    /// Keeps what the screen holds that is not kept yet, as it closes.
    pub fn save_all(&mut self, cx: &mut Context<Self>) {
        self.keep_typed(cx);
        self.workflow.update(cx, FolderField::keep_typed);
        self.wiki.update(cx, FolderField::keep_typed);
        self.agents.update(cx, AgentsSection::save_all);
        self.servers.update(cx, ServersSection::save_all);
        self.translation.update(cx, TranslationSection::keep_typed);
    }

    /// The input of the home's folder.
    #[must_use]
    pub fn input(&self) -> &Entity<LineInput> {
        &self.input
    }

    /// What has the keyboard while the human selects what the screen says.
    #[must_use]
    pub fn words_focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// What the human selected of what the screen says.
    #[must_use]
    pub fn selected(&self) -> String {
        self.words.selected()
    }

    /// Sets the home to the folder the input names, when it is not the
    /// home already, as when the keyboard leaves the input.
    fn keep_typed(&mut self, cx: &mut Context<Self>) {
        if self.input.read(cx).text().trim() != shortened(&self.home) {
            self.submit(cx);
        }
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
        // The folders the settings name none of are the new home's.
        self.workflow.update(cx, |field, cx| {
            field.set_fallback(super::Folder::Workflow.in_home(), cx);
        });
        self.wiki.update(cx, |field, cx| {
            field.set_fallback(super::Folder::Wiki.in_home(), cx);
        });
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
                    "Set: Cox's own chats open in {} from now on.",
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
                "The path typed, from / or ~, is set on Enter or as you leave it; \
                 Escape closes."
                    .to_owned(),
                FAINT,
            ),
        }
    }

    /// The card's heading, with the × that closes it.
    fn render_header(cx: &mut Context<Self>) -> impl IntoElement + use<> {
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
            )
    }

    /// The button that asks for a folder.
    fn render_choose(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        button(
            "choose-home",
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(icon(Icon::Folder, 14., DIM))
                .child("Choose…"),
            Kind::Plain,
        )
        .h(px(INPUT_HEIGHT))
        .on_click(cx.listener(|_, _, window, cx| Self::choose(window, cx)))
    }
}

impl EventEmitter<HomeChanged> for SettingsView {}
impl EventEmitter<Dismissed> for SettingsView {}
impl EventEmitter<UnpairPhones> for SettingsView {}

impl Focusable for SettingsView {
    /// The input, which has the keyboard while the screen is open.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (told, color) = self.told();
        // The sections write their words after the home's, as they render
        // after the screen.
        self.words.start();
        let mut words = self.words.part();
        let body = self.words.selectable(
            div()
                .id("settings-body")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(22.))
                .pt(px(18.))
                .pb(px(22.))
                .flex()
                .flex_col()
                .gap_2(),
            &self.focus,
        );
        card()
            .id("settings")
            .debug_selector(|| "settings".to_owned())
            .key_context(KEY_CONTEXT)
            .occlude()
            .w(px(WIDTH))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .shadow_lg()
            // A click on the card stays there, not closing it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|_, _: &CloseSettings, _, cx| cx.emit(Dismissed)))
            .child(Self::render_header(cx))
            .child(
                body.child(words.heading("HOME"))
                    .child(words.about(
                        "The folder Cox starts its own chats in, of no project, as New \
                         chat opens them; the rail shows it as Cox's icon.",
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
                                    .h(px(INPUT_HEIGHT))
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
                        words
                            .note(&told, color)
                            .debug_selector(|| "home-told".to_owned()),
                    )
                    .child(div().mt_4().child(self.workflow.clone()))
                    .child(div().mt_4().child(self.wiki.clone()))
                    .child(div().mt_4().child(self.agents.clone()))
                    .child(div().mt_4().child(self.servers.clone()))
                    .child(div().mt_4().child(self.translation.clone()))
                    .child(div().mt_4().child(self.phone.clone())),
            )
    }
}
