//! What the human selected, read in their language: [`Translate`] in any
//! part of the window with text selected, or "Translate" in the menu the
//! right button opens on it (see [`offer`]), shows a card by the mouse with the text's
//! translation as it arrives (see [`coxd::translate`]), which the human
//! copies, and closes with Escape, the ×, or a click beside it. The menu
//! also hands the selection to the chat shown, as [`crate::chat::hand`]
//! says, where it knows where the selection is from.

use gpui::{
    App, AppContext, ClipboardItem, Context, Entity, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement, Pixels, Point, Render,
    StatefulInteractiveElement, Styled, Task, Window, actions, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px, rgb,
};

use cox_core::entry::Attachment;

use crate::vim::hints::Hinted as _;
use coxd::theme::{BORDER_STRONG, BUTTON_HOVER, DIM, FAILED, FAINT, FOREGROUND, RAISED};
use coxd::translate::{Arriving, Way};

actions!(
    cox,
    [
        /// Translates the text selected in the part with the keyboard.
        Translate,
        /// Closes the translation.
        CloseTranslation,
    ]
);

/// The keys of the card.
pub(crate) const KEY_CONTEXT: &str = "Translation";

/// How wide the card is.
const WIDTH: f32 = 380.;

/// How far under the mouse the card hangs.
const LINE: f32 = 14.;

/// How tall the card grows before it scrolls.
const HEIGHT: f32 = 320.;

/// What the human is told when they name no language to read in.
const NO_LANGUAGE: &str = "Name the language you read in settings.json, as \
    {\"translation\": {\"language\": \"Japanese\"}}, to translate what you select.";

/// Binds the keys that translate, and close the card.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-shift-t", Translate, None),
        KeyBinding::new("escape", CloseTranslation, Some(KEY_CONTEXT)),
    ]);
    cx.set_global(Shown::default());
}

/// The card the window shows, if it shows one.
#[derive(Default)]
pub struct Shown(pub Option<Entity<TranslationCard>>);

impl Global for Shown {}

/// Shows the translation of `text`, by the mouse, unless it is only
/// space: the card takes the keyboard until it closes.
pub fn show(text: &str, window: &mut Window, cx: &mut App) {
    open(text, None, false, window, cx);
}

/// Offers to translate or copy `text`, the selection the right button
/// was pressed on, in a menu by the mouse, unless it is only space,
/// saying whether it does, for the part to keep the press to itself.
/// The menu offers to hand it to the chat shown as `handed`, when given.
pub fn offer(text: &str, handed: Option<Attachment>, window: &mut Window, cx: &mut App) -> bool {
    open(text, handed, true, window, cx)
}

/// Opens the card on `text`, translating it or `offered`, unless it is
/// only space.
fn open(
    text: &str,
    handed: Option<Attachment>,
    offered: bool,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    let at = window.mouse_position();
    let before = window.focused(cx);
    let card = cx.new(|cx| {
        let mut card = TranslationCard::new(text, handed, at, before, cx);
        if !offered {
            card.translate(cx);
        }
        card
    });
    window.focus(&card.focus_handle(cx));
    cx.set_global(Shown(Some(card)));
    true
}

/// Closes the card, giving the keyboard back to where it was.
pub fn close(window: &mut Window, cx: &mut App) {
    if !cx.has_global::<Shown>() {
        return;
    }
    let Some(card) = cx.global_mut::<Shown>().0.take() else {
        return;
    };
    if let Some(before) = card.read(cx).before.clone() {
        window.focus(&before);
    }
    cx.refresh_windows();
}

/// The card of a translation, or, before it, the menu that offers it.
pub struct TranslationCard {
    /// What the human selected.
    text: String,
    /// What the menu hands the chat shown, when the part knows where the
    /// selection is from.
    handed: Option<Attachment>,
    /// Whether the menu still offers the translation, which it has not
    /// asked for.
    offered: bool,
    /// Which way it is translated, if it is.
    way: Option<Way>,
    /// As much of it as has arrived, while it arrives.
    so_far: Option<String>,
    /// What it came to, once it did.
    translated: Option<Result<String, String>>,
    /// Where the mouse was, which the card hangs from.
    at: Point<Pixels>,
    focus: FocusHandle,
    /// What had the keyboard before.
    before: Option<FocusHandle>,
    /// The translation on its way, dropped with the card.
    waits: Option<Task<()>>,
}

impl TranslationCard {
    fn new(
        text: &str,
        handed: Option<Attachment>,
        at: Point<Pixels>,
        before: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            text: text.trim().to_owned(),
            handed,
            offered: true,
            way: None,
            so_far: None,
            translated: None,
            at,
            focus: cx.focus_handle(),
            before,
            waits: None,
        }
    }

    /// Asks for the translation, which the card shows once it arrives.
    fn translate(&mut self, cx: &mut Context<Self>) {
        self.offered = false;
        let Some((way, mut answer)) = coxd::translate::text(&self.text) else {
            self.translated = Some(Err(NO_LANGUAGE.to_owned()));
            cx.notify();
            return;
        };
        self.way = Some(way);
        self.waits = Some(cx.spawn(async move |card, cx| {
            loop {
                let arriving = answer.recv().await;
                let done = !matches!(arriving, Some(Arriving::SoFar(_)));
                let updated = card.update(cx, |card, cx| {
                    match arriving {
                        Some(Arriving::SoFar(so_far)) => card.so_far = Some(so_far),
                        Some(Arriving::Done(translated)) => card.translated = Some(translated),
                        None => card.translated = Some(Err("the translator stopped".to_owned())),
                    }
                    cx.notify();
                });
                if done || updated.is_err() {
                    return;
                }
            }
        }));
        cx.notify();
    }

    /// What the card says it is at, as `inspect` tells it.
    #[must_use]
    pub fn said(&self) -> String {
        if self.offered {
            return format!("{:?}, offered in a menu to translate or copy", self.text);
        }
        let way = self
            .way
            .as_ref()
            .map(|way| format!("{} to {}", way.from, way.to))
            .unwrap_or_default();
        let at = match (&self.translated, &self.so_far) {
            (None, None) => "translating".to_owned(),
            (None, Some(so_far)) => format!("translating, so far {so_far:?}"),
            (Some(Ok(text)), _) => format!("{text:?}"),
            (Some(Err(error)), _) => format!("failed: {error}"),
        };
        format!("{:?}, {way}: {at}", self.text)
    }
}

impl Focusable for TranslationCard {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TranslationCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.offered {
            return self.render_menu(cx).into_any_element();
        }
        let way = self.way.as_ref().map_or_else(
            || "Translate".to_owned(),
            |way| format!("{} → {}", way.from, way.to),
        );
        let copied = self.translated.clone().and_then(Result::ok);
        let head = div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(11.5))
            .text_color(rgb(FAINT))
            .child(div().flex_1().child(way))
            .when_some(copied, |head, text| {
                head.child(
                    button("translation-copy", "Copy").on_hint_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    }),
                )
            })
            .child(
                button("translation-close", "×").on_hint_click(|_, window, cx| close(window, cx)),
            );
        let body = match &self.translated {
            None => match &self.so_far {
                None => div().text_color(rgb(DIM)).child("Translating…"),
                // What has arrived, and a mark that more is to come.
                Some(so_far) => div()
                    .text_color(rgb(FOREGROUND))
                    .child(format!("{so_far} …")),
            },
            Some(Ok(text)) => div().text_color(rgb(FOREGROUND)).child(text.clone()),
            Some(Err(error)) => div().text_color(rgb(FAILED)).child(error.clone()),
        };
        let card = div()
            .id("translation")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(|_: &CloseTranslation, window, cx| close(window, cx))
            // Not through the card, which `close` reads.
            .on_mouse_down_out(|_, window, cx| close(window, cx))
            // Keeping the keyboard, for Escape, after a click on it.
            .occlude()
            .child(crate::vim::hints::cover())
            .on_mouse_down(MouseButton::Left, {
                let focus = self.focus.clone();
                move |_, window, _| window.focus(&focus)
            })
            .w(px(WIDTH))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(BORDER_STRONG))
            .rounded(px(10.))
            .shadow_lg()
            .text_size(px(13.))
            .child(head)
            .child(
                div()
                    .id("translation-text")
                    .max_h(px(HEIGHT))
                    .overflow_y_scroll()
                    .child(body),
            );
        deferred(
            anchored()
                .position(self.at)
                // Under the mouse, clear of the line it selected.
                .offset(point(px(0.), px(LINE)))
                .snap_to_window_with_margin(px(8.))
                .child(card),
        )
        .with_priority(3)
        .into_any_element()
    }
}

impl TranslationCard {
    /// The menu by the mouse that offers to translate the selection, or
    /// copy it.
    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // Copy's keys are the part's own, as a terminal's off macOS.
        let translate = if cfg!(target_os = "macos") {
            "⌘⇧T"
        } else {
            "Ctrl-Shift-T"
        };
        let text = self.text.clone();
        let handed = self.handed.clone();
        let menu = crate::ui::menu::menu()
            .id("translation")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(|_: &CloseTranslation, window, cx| close(window, cx))
            .on_action(cx.listener(|card, _: &Translate, _, cx| card.translate(cx)))
            .on_mouse_down_out(|_, window, cx| close(window, cx))
            .min_w(px(200.))
            .child(
                item("translation-translate", "Translate", translate)
                    .on_hint_click(cx.listener(|card, _, _, cx| card.translate(cx))),
            )
            .when_some(handed, |menu, handed| {
                menu.child(
                    item(
                        "translation-add-to-chat",
                        "Add to chat",
                        crate::chat::hand::keys(),
                    )
                    .on_hint_click(move |_, window, cx| {
                        crate::chat::hand::hand(handed.clone(), cx);
                        close(window, cx);
                    }),
                )
            })
            .child(
                item("translation-copy-selection", "Copy", "").on_hint_click(
                    move |_, window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        close(window, cx);
                    },
                ),
            );
        deferred(
            anchored()
                .position(self.at)
                .snap_to_window_with_margin(px(8.))
                .child(menu),
        )
        .with_priority(3)
    }
}

/// An item of the menu, with the keys that do the same.
fn item(id: &'static str, label: &'static str, keys: &'static str) -> gpui::Stateful<gpui::Div> {
    crate::ui::menu::item(id, label, keys)
}

/// A small button of the card's head.
fn button(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_1()
        .rounded_sm()
        .cursor_pointer()
        .text_color(rgb(DIM))
        .hover(|style| style.bg(rgb(BUTTON_HOVER)).text_color(rgb(FOREGROUND)))
        .child(label)
}
