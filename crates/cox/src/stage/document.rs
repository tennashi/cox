//! A tab of the stage that shows a Markdown file, drawn as the
//! conversation draws replies, for the human to read beside the
//! conversation. The agent opens it with Cox's tool for that (see
//! [`crate::agent::mcp`]).
//!
//! The tab follows the file as it changes (see [`crate::stage::follow`]). The
//! human selects and copies its text
//! as in the conversation, and a click on a link asks for the link to open
//! on the stage.

use std::path::{Path, PathBuf};

use cox_core::tabs::{KeptTab, Lines};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollHandle, Task, Window, div, prelude::*, px,
    rgb,
};

use crate::stage::follow::{self, Follows, Stamp};
use crate::stage::tab::{Shows, ShowsFile, TabContent};
use crate::ui::markdown::{self, Block};
use crate::ui::paths::shortened;
use crate::ui::text::Reading;
use crate::ui::theme::DIM;
use crate::ui::{Copy, LinkClicked};

const KEY_CONTEXT: &str = "Document";

/// How wide the text grows, at most.
const TEXT_WIDTH: f32 = 760.;

/// Binds the keys of the tab.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT))]);
}

impl EventEmitter<LinkClicked> for Document {}

pub struct Document {
    /// The file, as an absolute path.
    path: PathBuf,
    text: String,
    blocks: Vec<Block>,
    /// The file's, as the tab last read it, or `None` until the tab
    /// looks at the file.
    stamp: Option<Stamp>,
    /// Why the file could not be read the last time the tab looked, if it
    /// could not.
    unread: Option<String>,
    reading: Reading,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    _follow: Task<()>,
}

impl Document {
    /// A tab showing the file at `path`, an absolute path, which says
    /// `text`.
    pub fn new(path: PathBuf, text: String, cx: &mut Context<Self>) -> Self {
        Self {
            blocks: markdown::parse(&text),
            path,
            text,
            stamp: None,
            unread: None,
            reading: Reading::default(),
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _follow: follow::follow(cx),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The text shown.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text selected.
    #[must_use]
    pub fn selected_text(&self) -> String {
        self.reading.selected_text()
    }

    /// Shows `text`, which the file says now.
    pub fn show(&mut self, text: String, cx: &mut Context<Self>) {
        // The tab looks at the file again, which may have changed since.
        self.stamp = None;
        self.unread = None;
        self.set_text(text);
        cx.notify();
    }

    fn set_text(&mut self, text: String) {
        if text != self.text {
            self.blocks = markdown::parse(&text);
            self.text = text;
            // What was selected is of pieces of text that are gone.
            self.reading.deselect();
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        window.prevent_default();
        self.reading.press(event);
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.drag(event.position) {
            cx.notify();
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.reading.release(event.position) {
            cx.emit(LinkClicked(url.to_string()));
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }
}

impl Follows for Document {
    fn followed(&self) -> (PathBuf, Option<Stamp>) {
        (self.path.clone(), self.stamp)
    }

    fn looked(&mut self, looked: std::io::Result<(Stamp, String)>, cx: &mut Context<Self>) {
        match looked {
            Ok((stamp, text)) => {
                self.stamp = Some(stamp);
                self.unread = None;
                self.set_text(text);
            }
            Err(error) => self.unread = Some(error.to_string()),
        }
        cx.notify();
    }
}

impl Shows for Document {
    /// The file's name.
    fn title(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Markdown {
            path: self.path.clone(),
        }
    }
}

impl ShowsFile for Document {
    fn open(_: &Path, path: PathBuf, text: &str, _: Option<Lines>, cx: &mut Context<Self>) -> Self {
        Self::new(path, text.to_owned(), cx)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn show(&mut self, text: &str, _: Option<Lines>, cx: &mut Context<Self>) {
        self.show(text.to_owned(), cx);
    }

    fn content(this: Entity<Self>) -> TabContent {
        TabContent::Document(this)
    }

    fn of(content: &TabContent) -> Option<&Entity<Self>> {
        match content {
            TabContent::Document(document) => Some(document),
            TabContent::Page { .. }
            | TabContent::Code(_)
            | TabContent::Tree(_)
            | TabContent::Changes(_)
            | TabContent::Map(_)
            | TabContent::Flow(_)
            | TabContent::Terminal(_) => None,
        }
    }
}

impl Focusable for Document {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Document {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The frame lays the text out again.
        self.reading.clear();
        let text = self
            .reading
            .writer(0)
            .blocks(&self.blocks)
            .debug_selector(|| "document-text".to_owned());
        let bar = div()
            .flex()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(0x40_4040))
            .text_color(rgb(DIM))
            .child(
                div()
                    .debug_selector(|| "document-path".to_owned())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(shortened(&self.path)),
            )
            .children(self.unread.as_ref().map(|error| {
                div()
                    .flex_shrink_0()
                    .text_color(rgb(0xf0_8080))
                    .child(format!("cannot read: {error}"))
            }));
        div()
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::copy))
            .size_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(
                div()
                    .id("document")
                    .debug_selector(|| "document".to_owned())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|document, _, _, _| document.reading.stop()),
                    )
                    .child(div().max_w(px(TEXT_WIDTH)).px_4().py_3().child(text)),
            )
    }
}
