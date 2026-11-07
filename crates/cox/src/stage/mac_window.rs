//! A tab of the stage showing a window of another app on the Mac, as it
//! changes, for the human to see what the agent tries out, beside the chat
//! (see [`cox_mac`]). The agent clicks, types and scrolls in the window
//! through its tools, which the tab passes on to the window's app, drawing
//! the agent's pointer where it acts, so that the human sees what the
//! agent sees and does; the human only watches: "Bring to front" brings
//! the window itself up, for them to use it.
//!
//! A tab kept for a closed chat's stage opens again on the window of the
//! same app and title, if one is open.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cox_browser::pointer::{AgentPointer, Look};
use cox_browser::relay::AgentInput;
use cox_core::tabs::KeptTab;
use cox_mac::{Button, Event, Input, Mirror, Rect, Stroke, Window as AppWindow};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, Task, Window, div, prelude::*, px,
    rgb,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::agent::mcp::MacAct;
use crate::stage::mirror::{Display, Mirrored, Screen, scaled};
use crate::stage::pointer;
use crate::stage::tab::Shows;
use crate::ui::theme::{BORDER, BUTTON, DIM, FAILED, FOREGROUND, RAISED, SELECTED};

/// Why a tab that was not shown its window ended, as it opens.
const NOT_SHOWN: &str = "Not shown";

/// Why a tab whose window the human stopped showing ended.
const STOPPED: &str = "Stopped";

/// The most pixels the picture the agent gets is, each way: a model
/// scales a larger picture down itself, and would then point off from
/// where it means.
const PICTURE_MOST: f64 = 1568.;

/// Why the agent's act goes nowhere while Cox may not post to apps.
const MAY_NOT_CONTROL: &str = "Cox may not control the computer yet, which a click or a key in \
    another app's window needs: the human allows it in System Settings, Privacy & Security, \
    Accessibility, then opens Cox again.";

/// Whether Cox asked the human, in this run, to allow it to control the
/// computer: the system asks once, and after only lists Cox for them.
static ASKED_TO_CONTROL: AtomicBool = AtomicBool::new(false);

/// How many pixels of the agent's picture of a window at `bounds` a point
/// of it is.
fn picture_scale(bounds: Rect) -> f64 {
    (PICTURE_MOST / bounds.width.max(bounds.height).max(1.)).min(1.)
}

/// A window as the agent sees it: its picture, its width and height and
/// pixels as BGRA, and the size the agent gets it at, in pixels, where it
/// points.
pub type Picture = ((u32, u32, Vec<u8>), (u32, u32));

/// How the tab stands with its window.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Status {
    /// It looks for the window, or waits for its pictures.
    Finding,
    Showing,
    /// It shows the window no more: why.
    Ended(String),
}

pub struct MacWindow {
    /// The window's app, by its name.
    app: String,
    /// The window's title, as it had it last.
    title: String,
    /// The window, once found.
    window: Option<AppWindow>,
    /// The window's pictures, while the tab shows them.
    mirror: Option<Mirror>,
    status: Status,
    screen: Screen,
    focus_handle: FocusHandle,
    /// Looks for the window, or hears its mirror.
    working: Option<Task<()>>,
    /// The chat whose agent acts in the window, until its turn ends.
    agent: Option<u64>,
    /// The agent's pointer, and how it looks as of the last render.
    pointer: AgentPointer,
    look: Option<Look>,
    /// Renders the tab again when the pointer's look next changes.
    _pointer_changes: Option<Task<()>>,
}

impl MacWindow {
    /// A tab showing `window`.
    pub fn new(window: AppWindow, cx: &mut Context<Self>) -> Self {
        let mut tab = Self::waiting(window.app.clone(), window.title.clone(), cx);
        tab.show(window, cx);
        tab
    }

    /// A tab of the window of `app` titled `title`, which it has yet to
    /// find.
    fn waiting(app: String, title: String, cx: &mut Context<Self>) -> Self {
        Self {
            app,
            title,
            window: None,
            mirror: None,
            status: Status::Ended(NOT_SHOWN.to_owned()),
            screen: Screen::default(),
            focus_handle: cx.focus_handle(),
            working: None,
            agent: None,
            pointer: AgentPointer::default(),
            look: None,
            _pointer_changes: None,
        }
    }

    /// The window shown, as it was found.
    #[must_use]
    pub fn window(&self) -> Option<&AppWindow> {
        self.window.as_ref()
    }

    /// Whether the tab shows its window, or is on its way to.
    #[must_use]
    pub fn is_showing(&self) -> bool {
        !matches!(self.status, Status::Ended(_))
    }

    /// Where the window is, in points, once the tab found it.
    #[must_use]
    pub fn bounds(&self) -> Option<Rect> {
        self.mirror.as_ref()?.bounds()
    }

    /// The chat whose agent acts in the window, until its turn ends.
    #[must_use]
    pub fn agent(&self) -> Option<u64> {
        self.agent
    }

    /// Does `act`, which the agent of the chat `chat` asks, in the window:
    /// where it falls is in the pixels of the agent's picture of it (see
    /// [`MacWindow::picture`]).
    ///
    /// # Errors
    ///
    /// When the tab does not show the window, or Cox may not post to its
    /// app, or the act names no key: says why.
    pub fn act(
        &mut self,
        act: &MacAct,
        chat: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let mirror = self
            .mirror
            .as_ref()
            .ok_or("The tab no longer shows the window.")?;
        let bounds = mirror.bounds().ok_or("The window is not found yet.")?;
        let scale = picture_scale(bounds);
        let at = |x: u32, y: u32| {
            (
                (f64::from(x) / scale).min(bounds.width),
                (f64::from(y) / scale).min(bounds.height),
            )
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a point of a window is far within what f32 keeps"
        )]
        let spot = |(x, y): (f64, f64)| (x as f32, y as f32);
        let (input, seen) = match act {
            MacAct::List | MacAct::Look { .. } => return Ok(()),
            MacAct::Click {
                x, y, right, count, ..
            } => {
                let (x, y) = at(*x, *y);
                let button = if *right { Button::Right } else { Button::Left };
                let (sx, sy) = spot((x, y));
                let click = Input::Click {
                    x,
                    y,
                    button,
                    count: *count,
                };
                (click, AgentInput::Press { x: sx, y: sy })
            }
            MacAct::Scroll { x, y, dx, dy, .. } => {
                let (x, y) = at(*x, *y);
                let (sx, sy) = spot((x, y));
                let scroll = Input::Scroll {
                    x,
                    y,
                    dx: *dx,
                    dy: *dy,
                };
                (scroll, AgentInput::Wheel { x: sx, y: sy })
            }
            MacAct::Type { text, .. } => (Input::Text(text.clone()), AgentInput::Type),
            MacAct::Key { key, .. } => (Input::Key(Stroke::parse(key)?), AgentInput::Type),
        };
        if !cox_mac::may_control() {
            if !ASKED_TO_CONTROL.swap(true, Ordering::Relaxed) {
                cox_mac::ask_to_control();
            }
            return Err(MAY_NOT_CONTROL.to_owned());
        }
        mirror.send(input);
        self.pointer.see(seen, Instant::now());
        self.agent = chat;
        cx.notify();
        Ok(())
    }

    /// The agent's turn ended: its app no longer takes itself for the
    /// active one, as it did while the agent clicked in it.
    pub fn leave(&mut self) {
        if self.agent.take().is_some()
            && let Some(mirror) = &self.mirror
        {
            mirror.send(Input::Away);
        }
    }

    /// The window as the agent sees it now: its picture, as BGRA, and the
    /// size the agent gets it at, in pixels, where it points.
    #[must_use]
    pub fn picture(&self) -> Option<Picture> {
        let bounds = self.bounds()?;
        let scale = picture_scale(bounds);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a picture's size fits in u32"
        )]
        let size = (
            (bounds.width * scale).round().max(1.) as u32,
            (bounds.height * scale).round().max(1.) as u32,
        );
        Some((self.screen.picture()?, size))
    }

    /// Shows `window`, as it changes.
    pub fn show(&mut self, window: AppWindow, cx: &mut Context<Self>) {
        let (mirror, events) = Mirror::open(window.id);
        self.app.clone_from(&window.app);
        self.title.clone_from(&window.title);
        self.window = Some(window);
        self.mirror = Some(mirror);
        self.status = Status::Finding;
        self.screen.ask_again();
        self.working = Some(Self::hear(events, cx));
        cx.notify();
    }

    /// Looks for the window of the tab's app and title again, and shows it.
    fn find(&mut self, cx: &mut Context<Self>) {
        self.status = Status::Finding;
        let (app, title) = (self.app.clone(), self.title.clone());
        self.working = Some(cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { found(&app, &title) })
                .await;
            this.update(cx, |tab, cx| match found {
                Ok(window) => tab.show(window, cx),
                Err(why) => {
                    tab.status = Status::Ended(why);
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    /// Lets the window go: its app goes on.
    fn stop(&mut self, cx: &mut Context<Self>) {
        self.mirror = None;
        self.working = None;
        self.status = Status::Ended(STOPPED.to_owned());
        cx.notify();
    }

    fn hear(mut events: UnboundedReceiver<Event>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let heard = this.update(cx, |tab, cx| {
                    match event {
                        Event::Showing => tab.status = Status::Showing,
                        Event::Frame => {
                            if let Some(frame) = tab.mirror.as_ref().and_then(Mirror::take_frame) {
                                tab.screen.put(frame.width, frame.height, frame.bgra);
                            }
                        }
                        Event::Ended(why) => {
                            tab.mirror = None;
                            tab.status = Status::Ended(why);
                        }
                    }
                    cx.notify();
                });
                if heard.is_err() {
                    break;
                }
            }
        })
    }

    fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .px_2()
                .rounded(px(6.))
                .cursor_pointer()
                .bg(rgb(BUTTON))
                .hover(|style| style.bg(rgb(SELECTED)))
                .child(label)
        };
        let state = match &self.status {
            Status::Finding => Some(("Finding the window…", DIM)),
            Status::Showing => None,
            Status::Ended(_) => Some(("Not shown", DIM)),
        };
        let pid = self.window.as_ref().map(|window| window.pid);
        let showing = self.is_showing();
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .text_color(rgb(DIM))
            .child(
                div()
                    .debug_selector(|| "mac-window-name".to_owned())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(rgb(FOREGROUND))
                    .child(self.title()),
            )
            .children(state.map(|(state, color)| {
                div()
                    .debug_selector(|| "mac-window-state".to_owned())
                    .text_color(rgb(color))
                    .child(state)
            }))
            .children(pid.map(|pid| {
                button("mac-window-front", "Bring to front")
                    .on_click(move |_, _, _| cox_mac::bring_to_front(pid))
            }))
            .when(showing, |bar| {
                bar.child(
                    button("mac-window-stop", "Stop")
                        .on_click(cx.listener(|tab, _, _, cx| tab.stop(cx))),
                )
            })
            .when(!showing, |bar| {
                bar.child(
                    button("mac-window-show", "Show")
                        .on_click(cx.listener(|tab, _, _, cx| tab.find(cx))),
                )
            })
    }
}

/// The window of `app` titled `title`, or else `app`'s only window.
fn found(app: &str, title: &str) -> Result<AppWindow, String> {
    let windows = cox_mac::windows().map_err(|error| format!("{error:#}"))?;
    let of_app: Vec<&AppWindow> = windows.iter().filter(|window| window.app == app).collect();
    of_app
        .iter()
        .find(|window| window.title == title)
        .or_else(|| (of_app.len() == 1).then(|| &of_app[0]))
        .map(|window| (*window).clone())
        .ok_or_else(|| format!("No window of {app} titled {title:?} is open."))
}

impl Mirrored for MacWindow {
    fn screen(&mut self) -> &mut Screen {
        &mut self.screen
    }

    fn is_live(&self) -> bool {
        self.mirror.is_some()
    }

    fn resize(&mut self, width: u32, height: u32) {
        if let Some(mirror) = &self.mirror {
            mirror.resize(width, height);
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "a window's points are far within what f32 keeps"
    )]
    fn agent(&self) -> Option<(&Look, (f32, f32))> {
        let bounds = self.bounds()?;
        Some((
            self.look.as_ref()?,
            (bounds.width as f32, bounds.height as f32),
        ))
    }
}

impl Shows for MacWindow {
    fn title(&self) -> String {
        if self.title.is_empty() {
            self.app.clone()
        } else {
            format!("{} · {}", self.app, self.title)
        }
    }

    fn kept(&self) -> KeptTab {
        KeptTab::MacWindow {
            app: self.app.clone(),
            title: self.title.clone(),
        }
    }
}

impl Focusable for MacWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MacWindow {
    #[expect(
        clippy::used_underscore_binding,
        reason = "the timer is only held: dropping it cancels it"
    )]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        (self.look, self._pointer_changes) = pointer::look_now(&self.pointer, cx);
        let label = Mirrored::agent(self).and_then(|(look, (width, _))| {
            let drawn = self.screen.drawn_within()?;
            let look = scaled(look, f32::from(drawn.size.width) / width);
            Some(pointer::label(&look, drawn.origin))
        });
        let ended = match &self.status {
            Status::Ended(why) if why != NOT_SHOWN && why != STOPPED => Some(why.clone()),
            _ => None,
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_bar(cx))
            .child(
                div()
                    .id("mac-window")
                    .debug_selector(|| "mac-window".to_owned())
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .track_focus(&self.focus_handle)
                    .child(Display {
                        mirrored: cx.entity(),
                    })
                    .children(label)
                    .children(ended.map(|why| {
                        div()
                            .debug_selector(|| "mac-window-ended".to_owned())
                            .absolute()
                            .top(px(16.))
                            .left(px(16.))
                            .right(px(16.))
                            .p_3()
                            .rounded(px(8.))
                            .bg(rgb(RAISED))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .text_color(rgb(FAILED))
                            .whitespace_normal()
                            .child(why)
                    })),
            )
    }
}

/// The tab of the window of `app` titled `title`, kept for a closed chat's
/// stage, which opens again on that window if it is open.
pub(super) fn reopened(app: String, title: String, cx: &mut App) -> Entity<MacWindow> {
    cx.new(|cx| {
        let mut tab = MacWindow::waiting(app, title, cx);
        tab.find(cx);
        tab
    })
}
