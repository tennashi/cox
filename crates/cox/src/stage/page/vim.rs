//! What Vim's keys do in a page (see [`crate::vim`]), as in Vimperator:
//! normal mode scrolls, goes back and forth and draws hints, insert mode
//! is the page's while one of its fields has the keyboard, and hint mode
//! takes the label typed of a hint.

use cox_browser::{Hint, How, Kind, Scroll};
use gpui::{ClipboardItem, Context, Focusable as _, Window};

use super::hints::{Hinting, Shown, Typed};
use super::{Opens, Page};
use crate::stage::Tabs;
use crate::stage::code::{Back, Forward};
use crate::vim::keymap::{Follow, Motion};
use crate::vim::{self, Address, CopyAddress, FocusField, Label, Leave, Mode, Move, Reload};

impl Page {
    /// The mode the keys are in while the page has the keyboard: insert
    /// while a field of it has it, as in Vimperator.
    fn mode(&self) -> Mode {
        if self.editable {
            Mode::Insert
        } else {
            Mode::Normal
        }
    }

    fn enter(mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if vim::mode(cx).is_some_and(|was| was != mode) {
            vim::enter(mode, cx);
            window.refresh();
        }
    }

    pub(super) fn focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.browser.focus();
        Self::enter(self.mode(), window, cx);
    }

    pub(super) fn blurred(&mut self) {
        self.browser.blur();
        self.stop_hinting();
    }

    pub(super) fn editable_focus(
        &mut self,
        editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editable = editable;
        if self.focus_handle.is_focused(window) {
            self.stop_hinting();
            Self::enter(self.mode(), window, cx);
        }
    }

    /// The page went to another, which takes the keyboard from it: it
    /// gets it again, while it has it with Vim's keys.
    pub(super) fn refocus(&self, window: &Window, cx: &Context<Self>) {
        if vim::mode(cx).is_some() && self.focus_handle.is_focused(window) {
            self.browser.focus();
        }
    }

    /// Takes the hints away, and says whether there were any.
    fn stop_hinting(&mut self) -> bool {
        let hinting = self.hinting.take().is_some();
        if hinting {
            self.browser.clear_hints();
        }
        hinting
    }

    pub(super) fn move_by_keys(&mut self, motion: &Move, _: &mut Window, _: &mut Context<Self>) {
        let times = i32::try_from(motion.times()).unwrap_or(i32::MAX);
        self.browser.scroll(match motion.motion {
            Motion::Down => Scroll::Lines(times),
            Motion::Up => Scroll::Lines(-times),
            Motion::HalfDown => Scroll::HalfPages(times),
            Motion::HalfUp => Scroll::HalfPages(-times),
            Motion::Top => Scroll::Top,
            Motion::Bottom => Scroll::Bottom,
        });
    }

    /// Leaves the page for `url`, as Vim's `:open` asks.
    pub(in crate::stage) fn load(&self, url: &str) {
        self.browser.load(url);
    }

    /// Goes back or forward, or loads the page again, as Vim's `:back`,
    /// `:forward` and `:reload` ask.
    pub(in crate::stage) fn browse(&self, tabs: &Tabs) {
        match tabs {
            Tabs::Reload => self.browser.reload(),
            Tabs::Back => self.browser.go_back(),
            Tabs::Forward => self.browser.go_forward(),
            _ => {}
        }
    }

    pub(super) fn back(&mut self, _: &Back, _: &mut Window, _: &mut Context<Self>) {
        self.browser.go_back();
    }

    pub(super) fn forward(&mut self, _: &Forward, _: &mut Window, _: &mut Context<Self>) {
        self.browser.go_forward();
    }

    pub(super) fn reload(&mut self, _: &Reload, _: &mut Window, _: &mut Context<Self>) {
        self.browser.reload();
    }

    pub(super) fn focus_field(&mut self, _: &FocusField, _: &mut Window, _: &mut Context<Self>) {
        self.browser.focus_field();
    }

    pub(super) fn copy_address(&mut self, _: &CopyAddress, _: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.navigation.url.clone()));
    }

    pub(super) fn go_to_address(
        &mut self,
        _: &Address,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_plugin() {
            return;
        }
        window.focus(&self.address.focus_handle(cx));
        Self::enter(Mode::Insert, window, cx);
    }

    /// Escape took the keys to normal mode: the hints go, or the address
    /// line gives the keyboard back to the page, or the field that has
    /// it lets it go.
    pub(super) fn leave(&mut self, _: &Leave, window: &mut Window, cx: &mut Context<Self>) {
        if self.stop_hinting() {
            return;
        }
        if self.address.focus_handle(cx).is_focused(window) {
            window.focus(&self.focus_handle);
            return;
        }
        self.browser.blur_field();
    }

    pub(super) fn hint(&mut self, hint: &vim::Hint, _: &mut Window, _: &mut Context<Self>) {
        self.stop_hinting();
        self.hinting = Some(Hinting::Asked(hint.follow));
        self.browser.hints();
    }

    /// The page listed what can be clicked, as `hint` asked.
    pub(super) fn hints_listed(
        &mut self,
        hints: Vec<Hint>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Hinting::Asked(follow)) = self.hinting else {
            return;
        };
        if let Some(shown) = Shown::new(follow, hints) {
            self.browser.show_hints(&shown.labels());
            self.hinting = Some(Hinting::Shown(shown));
        } else {
            self.hinting = None;
            Self::enter(Mode::Normal, window, cx);
        }
    }

    pub(super) fn label(&mut self, label: &Label, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Hinting::Shown(shown)) = &mut self.hinting else {
            return;
        };
        match shown.typed(label.key) {
            Typed::Narrowed(labels) => self.browser.show_hints(&labels),
            Typed::Chosen(hint, follow) => {
                self.stop_hinting();
                Self::enter(Mode::Normal, window, cx);
                match (follow, &hint.kind) {
                    (Follow::NewTab, Kind::Link(href)) => cx.emit(Opens::Tab(href.clone())),
                    (_, Kind::Field) => self.browser.activate(&hint, How::Focus),
                    _ => self.browser.activate(&hint, How::Click),
                }
            }
            Typed::Missed => {
                self.stop_hinting();
                Self::enter(Mode::Normal, window, cx);
            }
        }
    }
}
