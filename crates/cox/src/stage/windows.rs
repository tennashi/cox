//! A tab of the stage listing the windows of the Mac's other apps, for the
//! human to show one on the stage, as the agent tries out an app it makes:
//! each by its app and its title, which a click shows on a tab of its own
//! (see [`crate::stage::mac_window`]), or brings to the front, to use the
//! window itself. Over them, the tab asks the human to allow Cox to record
//! the screen, which macOS asks of it for the pictures and the titles.

use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_mac::Window as AppWindow;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, SharedString, Task, Window,
    div, prelude::*, px, rgb,
};

use crate::stage::heading::{INSET, TEXT_INSET, body, card, heading, note, parted};
use crate::stage::tab::Shows;
use crate::ui::button::{Kind, button};
use crate::vim::hints::Hinted as _;
use coxd::theme::{ASKING, DIM, FAILED, FAINT, FOREGROUND};

pub(crate) const KEY_CONTEXT: &str = "Windows";

/// How often the tab reads the windows again, as apps open and close them.
const READ_EVERY: Duration = Duration::from_secs(2);

/// The title the tab goes by.
pub const TITLE: &str = "Mac windows";

/// What the tab read of the windows last.
#[derive(Clone, Debug, PartialEq)]
struct Read {
    windows: Result<Vec<AppWindow>, String>,
    may_record: bool,
}

pub struct Windows {
    read: Option<Read>,
    focus_handle: FocusHandle,
    _reading: Task<()>,
}

/// What the human asks the stage to show.
pub struct ShowWindow(pub AppWindow);

impl EventEmitter<ShowWindow> for Windows {}

impl Windows {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let reading = cx.spawn(async move |this, cx| {
            loop {
                let read = cx
                    .background_executor()
                    .spawn(async {
                        Read {
                            windows: cox_mac::windows().map_err(|error| format!("{error:#}")),
                            may_record: cox_mac::may_record(),
                        }
                    })
                    .await;
                let kept = this.update(cx, |windows, cx| {
                    if windows.read.as_ref() != Some(&read) {
                        windows.read = Some(read);
                        cx.notify();
                    }
                });
                if kept.is_err() {
                    break;
                }
                cx.background_executor().timer(READ_EVERY).await;
            }
        });
        Self {
            read: None,
            focus_handle: cx.focus_handle(),
            _reading: reading,
        }
    }

    fn render_window(index: usize, window: &AppWindow, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = window.clone();
        let pid = window.pid;
        parted(div(), index == 0)
            .debug_selector(move || format!("mac-window-{index}"))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_color(rgb(FOREGROUND)).child(window.app.clone()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(FAINT))
                            .child(what(window)),
                    ),
            )
            .child(
                button(("mac-window-show", index), "Show", Kind::Plain).on_hint_click(cx.listener(
                    move |_, _, _, cx| {
                        cx.emit(ShowWindow(shown.clone()));
                    },
                )),
            )
            .child(
                button(("mac-window-front", index), "Bring to front", Kind::Plain)
                    .on_hint_click(move |_, _, _| cox_mac::bring_to_front(pid)),
            )
    }
}

/// What the line under a window's app says of it: its title, and its size.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a window's points fit in i64"
)]
fn what(window: &AppWindow) -> String {
    let size = format!(
        "{}×{}",
        window.frame.width.round() as i64,
        window.frame.height.round() as i64
    );
    if window.title.is_empty() {
        size
    } else {
        format!("{} · {size}", window.title)
    }
}

/// A line over the windows that asks the human to allow Cox `what`, with
/// a button that asks macOS.
fn asking(id: &'static str, what: &'static str, ask: fn()) -> impl IntoElement {
    div()
        .debug_selector(move || id.to_owned())
        .flex()
        .items_center()
        .gap_2()
        .px(px(TEXT_INSET))
        .py_2()
        .child(div().flex_1().text_color(rgb(ASKING)).child(what))
        .child(button((id, 0_usize), "Allow…", Kind::Primary).on_hint_click(move |_, _, _| ask()))
}

impl Shows for Windows {
    fn title(&self) -> String {
        TITLE.to_owned()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::MacWindows
    }
}

impl Focusable for Windows {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Windows {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let read = self.read.clone();
        let said = match read.as_ref().map(|read| &read.windows) {
            None => "Reading the windows on screen…".to_owned(),
            Some(Err(_)) => String::new(),
            Some(Ok(windows)) if windows.len() == 1 => "1 window of another app".to_owned(),
            Some(Ok(windows)) => format!("{} windows of other apps", windows.len()),
        };
        let mut list = body()
            .id("mac-windows")
            .debug_selector(|| "mac-windows".to_owned())
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .overflow_y_scroll()
            .text_color(rgb(FOREGROUND))
            .child(heading("mac-windows-heading", TITLE, said, None));
        let Some(read) = read else {
            return list;
        };
        if !read.may_record {
            list = list.child(asking(
                "mac-may-record",
                "Cox shows other apps' windows once it may record the screen.",
                cox_mac::ask_to_record,
            ));
        }
        let note = |text: SharedString, color: u32| note("mac-windows-note", text, color);
        match read.windows {
            // The row above asks for what the system would not say.
            Err(_) if !read.may_record => list,
            Err(why) => list.child(note(why.into(), FAILED)),
            Ok(windows) if windows.is_empty() => {
                list.child(note("No other app's window is on screen.".into(), DIM))
            }
            Ok(windows) => {
                let rows: Vec<_> = windows
                    .iter()
                    .enumerate()
                    .map(|(index, window)| {
                        Self::render_window(index, window, cx).into_any_element()
                    })
                    .collect();
                list.child(
                    div()
                        .px(px(INSET))
                        .pt_1()
                        .pb_3()
                        .child(card().children(rows)),
                )
            }
        }
    }
}
