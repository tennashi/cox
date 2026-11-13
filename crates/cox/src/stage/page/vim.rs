//! What Vim's keys do in a page (see [`crate::vim`]), as in Vimperator:
//! normal mode scrolls, goes back and forth and draws hints, insert mode
//! is the page's while one of its fields has the keyboard, and hint mode
//! takes the label typed of a hint.

use cox_browser::{Hint, How, Kind, Scroll};
use gpui::{App, ClipboardItem, Context, WeakEntity, Window, div, prelude::*, rgb};

use super::hints::{Hinting, Shown, Typed};
use super::{Opens, Page};
use crate::stage::Tabs;
use crate::stage::code::{Back, Forward};
use crate::ui::theme::DIM;
use crate::vim::keymap::{Follow, Motion};
use crate::vim::search::{self, Direction, Find};
use crate::vim::{self, Address, CopyAddress, FocusField, Label, Leave, Mode, Move, Reload, hints};

/// What Vim's `/` looks for in a page, and how many matches the page
/// found of it, and which it selected, once it says.
pub(super) struct Finding {
    text: String,
    found: Option<(u32, u32)>,
}

impl Finding {
    /// Which match is selected, of how many, beside the address line.
    pub(super) fn render(&self) -> impl IntoElement + use<> {
        let said = match self.found {
            None => String::new(),
            Some((0, _)) => format!("{} not found", self.text),
            Some((count, active)) => format!("{active}/{count}"),
        };
        div()
            .debug_selector(|| "page-found".to_owned())
            .flex_shrink_0()
            .px_2()
            .text_color(rgb(DIM))
            .child(said)
    }
}

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

    /// Finds the text Vim's `/` or `n` asks for: the match after the one
    /// selected, when it is the same text, or else the first.
    pub(super) fn find(&mut self, find: &Find, _: &mut Window, cx: &mut Context<Self>) {
        let again = self
            .finding
            .as_ref()
            .is_some_and(|finding| finding.text == find.text);
        self.browser.find(
            &find.text,
            find.direction == Direction::Forward,
            search::exact(&find.text),
            again,
        );
        if !again {
            self.finding = Some(Finding {
                text: find.text.clone(),
                found: None,
            });
        }
        cx.notify();
    }

    /// The page counted the matches of what it was asked to find.
    pub(super) fn found(&mut self, count: u32, active: u32) {
        if let Some(finding) = &mut self.finding {
            finding.found = Some((count, active));
        }
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
        self.address
            .update(cx, |address, cx| address.focus(window, cx));
    }

    /// Escape took the keys to normal mode: the hints go, or the field
    /// that has the keyboard lets it go.
    pub(super) fn leave(&mut self, _: &Leave, _: &mut Window, _: &mut Context<Self>) {
        if self.stop_hinting() {
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
        if matches!(self.hinting, Some(Hinting::AskedByWindow(_))) {
            let ids = hints.iter().map(|hint| hint.id).collect();
            if let Some(Hinting::AskedByWindow(answer)) =
                self.hinting.replace(Hinting::Window(hints))
            {
                answer(ids, window, cx);
            }
            return;
        }
        let Some(Hinting::Asked(follow)) = self.hinting else {
            return;
        };
        if let Some(shown) = Shown::new(follow, hints) {
            self.browser
                .show_hints(&shown.labels(), &crate::vim::hints::page_look());
            self.hinting = Some(Hinting::Shown(shown));
        } else {
            self.hinting = None;
            Self::enter(Mode::Normal, window, cx);
        }
    }

    /// Takes the hint of `id`, of those listed for the window's hints,
    /// with the keyboard in the page.
    fn take_for_window(&mut self, id: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Hinting::Window(hints)) = self.hinting.take() else {
            return;
        };
        self.browser.clear_hints();
        let Some(hint) = hints.into_iter().find(|hint| hint.id == id) else {
            return;
        };
        window.focus(&self.focus_handle);
        self.browser.focus();
        let how = if hint.kind == Kind::Field {
            How::Focus
        } else {
            How::Click
        };
        self.browser.activate(&hint, how);
        cx.notify();
    }

    pub(super) fn label(&mut self, label: &Label, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Hinting::Shown(shown)) = &mut self.hinting else {
            return;
        };
        match shown.typed(label.key) {
            Typed::Narrowed(labels) => self
                .browser
                .show_hints(&labels, &crate::vim::hints::page_look()),
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

/// The page shown, whose hints join the window's.
pub(super) struct WindowHints(pub(super) WeakEntity<Page>);

impl hints::Page for WindowHints {
    fn ask(&self, answer: hints::Answer, cx: &mut App) {
        let Some(page) = self.0.upgrade() else {
            return;
        };
        page.update(cx, |page, _| {
            page.stop_hinting();
            page.hinting = Some(Hinting::AskedByWindow(answer));
            page.browser.hints();
        });
    }

    fn show(&self, labels: &[(u32, String)], cx: &mut App) {
        if let Some(page) = self.0.upgrade() {
            page.read(cx)
                .browser
                .show_hints(labels, &crate::vim::hints::page_look());
        }
    }

    fn take(&self, id: u32, window: &mut Window, cx: &mut App) {
        if let Some(page) = self.0.upgrade() {
            page.update(cx, |page, cx| page.take_for_window(id, window, cx));
        }
    }

    fn clear(&self, cx: &mut App) {
        if let Some(page) = self.0.upgrade() {
            page.update(cx, |page, _| {
                if matches!(
                    page.hinting,
                    Some(Hinting::AskedByWindow(_) | Hinting::Window(_))
                ) {
                    page.stop_hinting();
                }
            });
        }
    }
}
