//! The words of the settings screen, as the human reads them: its
//! headings, what each setting is for, and the lines under the inputs,
//! which they select, copy and translate as they do the conversation's.
//!
//! Every section writes its words through [`Words`], a [`Part`] each, so
//! that what a section says is selectable without the section doing more:
//! the screen numbers the parts down the screen as they render, and takes
//! the mouse and the keys for all of them (see [`Words::selectable`]).

use std::cell::RefCell;
use std::rc::Rc;

use cox_core::entry::Attachment;
use gpui::{
    Div, FocusHandle, FontWeight, MouseButton, MouseDownEvent, Stateful, div, prelude::*, px, rgb,
};

use crate::chat::hand::AddToChat;
use crate::ui::Copy;
use crate::ui::text::{Reading, Writer};
use crate::ui::translation::Translate;
use coxd::theme::{DIM, FAINT};

/// The words of the whole screen, which the screen and each of its
/// sections share.
#[derive(Clone, Default)]
pub struct Words(Rc<RefCell<Shared>>);

#[derive(Default)]
struct Shared {
    reading: Reading,
    /// How many parts the frame has written so far.
    parts: usize,
}

impl Words {
    /// Starts a frame: the screen calls it as it renders, ahead of its
    /// sections, which render after it in order down the screen.
    pub fn start(&self) {
        let mut shared = self.0.borrow_mut();
        shared.reading.clear();
        shared.parts = 0;
    }

    /// The part of the words that comes next down the screen, for a
    /// section to write as it renders.
    #[must_use]
    pub fn part(&self) -> Part {
        let mut shared = self.0.borrow_mut();
        let entry = shared.parts;
        shared.parts += 1;
        Part(shared.reading.writer(entry))
    }

    /// The text selected.
    #[must_use]
    pub fn selected(&self) -> String {
        self.0.borrow().reading.selected_text()
    }

    /// `body`, the part of the screen that holds the words, taking the
    /// mouse to select them and the keys to copy and translate them, with
    /// `focus` to have the keyboard while they are selected.
    pub fn selectable(&self, body: Stateful<Div>, focus: &FocusHandle) -> Stateful<Div> {
        let pressed = {
            let words = self.clone();
            let focus = focus.clone();
            move |event: &MouseDownEvent, window: &mut gpui::Window, _: &mut gpui::App| {
                let mut shared = words.0.borrow_mut();
                if shared.reading.is_on_text(event.position) {
                    // Off the text, as on an input or a button, the press
                    // is theirs.
                    window.focus(&focus);
                    window.prevent_default();
                    shared.reading.press(event);
                } else if !shared.reading.deselect() {
                    return;
                }
                window.refresh();
            }
        };
        let moved = self.clone();
        let released = self.clone();
        let stopped = self.clone();
        let offered = self.clone();
        let copied = self.clone();
        let translated = self.clone();
        let handed = self.clone();
        body.track_focus(focus)
            .on_mouse_down(MouseButton::Left, pressed)
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                let text = offered.selected();
                let handed = Some(of_settings(text.clone()));
                if crate::ui::translation::offer(&text, handed, window, cx) {
                    cx.stop_propagation();
                }
            })
            .on_mouse_move(move |event, window, _| {
                if moved.0.borrow_mut().reading.drag(event.position) {
                    window.refresh();
                }
            })
            .on_mouse_up(MouseButton::Left, move |event, _, _| {
                released.0.borrow_mut().reading.release(event.position);
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, _| {
                stopped.0.borrow_mut().reading.stop();
            })
            .on_action(move |_: &Copy, _, cx| {
                let text = copied.selected();
                if !text.is_empty() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            })
            .on_action(move |_: &Translate, window, cx| {
                let text = translated.selected();
                if text.trim().is_empty() {
                    cx.propagate();
                } else {
                    crate::ui::translation::show(&text, window, cx);
                }
            })
            .on_action(move |_: &AddToChat, _, cx| {
                if !crate::chat::hand::hand(of_settings(handed.selected()), cx) {
                    cx.propagate();
                }
            })
    }
}

/// `text`, selected in the settings, as it goes to the agent when handed
/// to the chat.
fn of_settings(text: String) -> Attachment {
    Attachment {
        label: "Settings".to_owned(),
        uri: "cox://settings".to_owned(),
        text,
        picture: None,
    }
}

/// A section's words, as it writes them, each a line the human selects.
pub struct Part(Writer);

impl Part {
    /// The heading of a section, as `HOME`.
    pub fn heading(&mut self, text: &str) -> Div {
        div()
            .text_size(px(11.))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(FAINT))
            .child(self.0.plain(text, "\n"))
    }

    /// What a setting is for, under its heading.
    pub fn about(&mut self, text: &str) -> Div {
        div()
            .text_size(px(13.))
            .text_color(rgb(DIM))
            .child(self.0.plain(text, "\n"))
    }

    /// A line under an input or a row, as how to use it or what came of
    /// it, in `color`.
    pub fn note(&mut self, text: &str, color: u32) -> Div {
        div()
            .text_size(px(12.))
            .text_color(rgb(color))
            .child(self.0.plain(text, "\n"))
    }
}
