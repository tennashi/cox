//! What the parts of the window do alike with the text the human selects
//! in them: [`Copy`] copies it, [`Translate`] shows it in their language,
//! and [`AddToChat`] hands it, with where it is from, to the chat shown.
//! A part says what its selection is as [`Selected`], and takes the keys
//! with [`on_actions`].

use cox_core::entry::Attachment;
use gpui::{ClipboardItem, Context, InteractiveElement, Window};

use crate::chat::hand::AddToChat;
use crate::ui::Copy;
use crate::ui::translation::Translate;

/// A part of the window the human selects text in.
pub trait Selected: Sized + 'static {
    /// The text that copying and translating take, empty when none is
    /// selected.
    fn chosen_text(&self) -> String;

    /// What handing to the chat shown hands: the text, with where it is
    /// from, when there is any.
    fn attachment(&self) -> Option<Attachment>;

    /// Ends visual mode, as copying or handing the selection does, as
    /// after Vim's `y`: answers whether it was on.
    fn end_visual(&mut self) -> bool {
        false
    }
}

/// Takes [`Copy`], [`Translate`] and [`AddToChat`] in `element`, for the
/// part `cx` is of.
pub fn on_actions<V: Selected, E: InteractiveElement>(element: E, cx: &mut Context<V>) -> E {
    element
        .on_action(cx.listener(copy::<V>))
        .on_action(cx.listener(translate::<V>))
        .on_action(cx.listener(add_to_chat::<V>))
}

/// Copies the text selected, if any is.
fn copy<V: Selected>(view: &mut V, _: &Copy, _: &mut Window, cx: &mut Context<V>) {
    let text = view.chosen_text();
    if !text.is_empty() {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
    if view.end_visual() {
        cx.notify();
    }
}

/// Shows the translation of the text selected, or with nothing
/// selected leaves the keys to what holds the part.
pub fn translate<V: Selected>(
    view: &mut V,
    _: &Translate,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let text = view.chosen_text();
    if text.trim().is_empty() {
        cx.propagate();
        return;
    }
    crate::ui::translation::show(&text, window, cx);
}

/// Hands the selection to the chat shown, or with nothing to hand leaves
/// the keys to what holds the part.
fn add_to_chat<V: Selected>(view: &mut V, _: &AddToChat, _: &mut Window, cx: &mut Context<V>) {
    let handed = view
        .attachment()
        .is_some_and(|attachment| crate::chat::hand::hand(attachment, cx));
    if !handed {
        cx.propagate();
    }
    if view.end_visual() {
        cx.notify();
    }
}
