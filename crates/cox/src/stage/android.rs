//! A tab of the stage showing an Android emulator, as Android Studio's
//! "Running Devices" does: Cox starts the emulator of a virtual device
//! with no window of its own, or reaches it if it runs already, and the
//! tab draws its display, which the human touches with the mouse, types
//! into, scrolls with the wheel, and presses Back, Home and Overview of
//! on the bar above it (see [`cox_android::device`]).
//!
//! The agent taps, swipes, types and presses keys on it through its tools,
//! which the tab passes on to the emulator, drawing the agent's pointer
//! where it touches, so that the human sees what the agent sees and does.
//!
//! The emulator Cox started stops as its tab closes, or as Cox quits,
//! saving its state for its next start; one that ran already goes on.

use std::time::Instant;

use cox_android::device::{Boot, Device, Event, Input, Status};
use cox_browser::pointer::{AgentPointer, Look};
use cox_browser::relay::AgentInput;
use cox_core::tabs::KeptTab;
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, Hitbox, IntoElement, KeyDownEvent,
    MouseButton, Pixels, Subscription, Task, Window, div, prelude::*, px, rgb,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::agent::mcp::AndroidAct;
use crate::chat::region::{self, Described};
use crate::stage::mirror::{Display, Mirrored, Press, Screen, Touched, pass_on, scaled};
use crate::stage::pointer;
use crate::stage::tab::Shows;
use crate::ui::button::{Kind, button};
use crate::vim::hints::Hinted as _;
use coxd::theme::{BORDER, DIM, FAILED, FOREGROUND, RAISED};

pub(crate) const KEY_CONTEXT: &str = "Android";

/// How far one click of a mouse's wheel turns it, as the emulator counts.
const WHEEL_CLICK: f32 = 120.;

/// How many pixels the longer side of the picture the agent gets is,
/// whatever size the tab draws the display at, so that where the agent
/// points stays where it was as the tab changes size.
const PICTURE_SIDE: f64 = 1280.;

/// A display as the agent sees it: its picture, its width and height and
/// pixels as BGRA, and the size the agent gets it at, in pixels, where it
/// points.
pub type Picture = ((u32, u32, Vec<u8>), (u32, u32));

pub struct Android {
    /// The device, by its name as `emulator -avd` takes it.
    name: String,
    /// The emulator, while the tab talks to one.
    device: Option<Device>,
    status: Status,
    /// The display, as it came last.
    screen: Screen,
    focus_handle: FocusHandle,
    /// Tells Vim's keys when the tab gets the keyboard, once drawn.
    focused: Option<Subscription>,
    /// Hears the emulator, while the tab talks to one.
    hearing: Option<Task<()>>,
    /// Whether the agent's finger is on the display.
    finger_down: bool,
    /// The agent's pointer, and how it looks as of the last render.
    pointer: AgentPointer,
    look: Option<Look>,
    /// Renders the tab again when the pointer's look next changes.
    _pointer_changes: Option<Task<()>>,
    _quit: Subscription,
}

impl Android {
    /// A tab of the device `name`, which starts its emulator as `boot`
    /// says, or reaches it if it runs, given a `boot`, and else waits for
    /// the human to start it.
    pub fn new(name: String, boot: Option<Boot>, cx: &mut Context<Self>) -> Self {
        let quit = cx.on_app_quit(|android, _| {
            if let Some(device) = &android.device {
                device.stop();
            }
            async {}
        });
        let mut android = Self {
            name,
            device: None,
            status: Status::Ended("Not running".to_owned()),
            screen: Screen::default(),
            focus_handle: cx.focus_handle(),
            focused: None,
            hearing: None,
            finger_down: false,
            pointer: AgentPointer::default(),
            look: None,
            _pointer_changes: None,
            _quit: quit,
        };
        if let Some(boot) = boot {
            android.open(boot, cx);
        }
        android
    }

    /// The device, by its name as `emulator -avd` takes it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the tab talks to the device's emulator, or is on its way to.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.device.is_some() && !matches!(self.status, Status::Ended(_))
    }

    /// Whether the device's emulator is starting, or being reached, so
    /// that its display has yet to come.
    #[must_use]
    pub fn is_booting(&self) -> bool {
        self.device.is_some() && matches!(self.status, Status::Starting | Status::Reaching)
    }

    /// Starts the device's emulator as `boot` says, or reaches it if it
    /// runs.
    pub fn open(&mut self, boot: Boot, cx: &mut Context<Self>) {
        let (device, events) = Device::open(cox_android::sdk::emulator(), &self.name, boot);
        self.device = Some(device);
        self.screen.ask_again();
        self.hearing = Some(Self::hear(events, cx));
        cx.notify();
    }

    /// The size the agent gets the display's picture at, in pixels, once a
    /// picture came: its longer side [`PICTURE_SIDE`].
    #[must_use]
    pub fn picture_size(&self) -> Option<(u32, u32)> {
        let (width, height) = self.screen.size()?;
        let scale = PICTURE_SIDE / f64::from(width.max(height).max(1));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a picture's size fits in u32"
        )]
        let side = |length: u32| (f64::from(length) * scale).round().max(1.) as u32;
        Some((side(width), side(height)))
    }

    /// The display as the agent sees it now, once a picture came.
    #[must_use]
    pub fn picture(&self) -> Option<Picture> {
        Some((self.screen.picture()?, self.picture_size()?))
    }

    /// Does `act`, which the agent asks, on the device: where it falls is
    /// in the pixels of the agent's picture of the display (see
    /// [`Android::picture`]). A swipe only lands; [`Android::touch`]
    /// moves the finger on.
    ///
    /// # Errors
    ///
    /// When the emulator does not run, or shows no display yet.
    pub fn act(&mut self, act: &AndroidAct, cx: &mut Context<Self>) -> Result<(), String> {
        if matches!(act, AndroidAct::List | AndroidAct::Look { .. }) {
            return Ok(());
        }
        if !self.is_running() {
            return Err(format!("{} does not run yet: look at it first.", self.name));
        }
        match act {
            AndroidAct::List | AndroidAct::Look { .. } => {}
            AndroidAct::Tap { x, y, .. } => {
                self.touch(*x, *y, true, cx)?;
                self.touch(*x, *y, false, cx)?;
            }
            AndroidAct::Swipe { x, y, .. } => self.touch(*x, *y, true, cx)?,
            AndroidAct::Type { text, .. } => {
                self.send(Input::Text(text.clone()));
                self.agent_input(AgentInput::Type, cx);
            }
            AndroidAct::Key { key, .. } => {
                self.send(Input::Key(key.clone()));
                self.agent_input(AgentInput::Type, cx);
            }
        }
        Ok(())
    }

    /// The agent's finger at `x` and `y`, in the pixels of its picture of
    /// the display, touching while `down` is, and lifting as it is not.
    ///
    /// # Errors
    ///
    /// When the display shows no picture yet.
    pub fn touch(
        &mut self,
        x: u32,
        y: u32,
        down: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let (width, height) = self
            .picture_size()
            .ok_or_else(|| format!("{} shows no display yet: look again.", self.name))?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a picture's pixels are far below what f32 keeps exactly"
        )]
        let (x, y) = (x.min(width) as f32, y.min(height) as f32);
        #[expect(
            clippy::cast_precision_loss,
            reason = "a picture's pixels are far below what f32 keeps exactly"
        )]
        let across = (x / width as f32, y / height as f32);
        let landing = down && !self.finger_down;
        self.finger_down = down;
        self.send(Input::Touch {
            x: across.0,
            y: across.1,
            down,
        });
        let seen = if landing {
            AgentInput::Press { x, y }
        } else {
            AgentInput::Move { x, y }
        };
        self.agent_input(seen, cx);
        Ok(())
    }

    /// Has the agent's pointer show `input`.
    fn agent_input(&mut self, input: AgentInput, cx: &mut Context<Self>) {
        self.pointer.see(input, Instant::now());
        cx.notify();
    }

    /// Stops the emulator, if Cox started it, and lets the tab go of it.
    fn close(&mut self, cx: &mut Context<Self>) {
        // The device stops its emulator as it drops.
        self.device = None;
        self.hearing = None;
        self.status = Status::Ended("Stopped".to_owned());
        cx.notify();
    }

    fn hear(mut events: UnboundedReceiver<Event>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let heard = this.update(cx, |android, cx| {
                    match event {
                        Event::Status(status) => android.status = status,
                        Event::Frame => {
                            if let Some(frame) =
                                android.device.as_ref().and_then(Device::take_frame)
                            {
                                android.screen.put(frame.width, frame.height, frame.bgra);
                            }
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

    fn send(&self, input: Input) {
        if let Some(device) = &self.device {
            device.send(input);
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        // Cox's own shortcuts go on to Cox.
        if keystroke.modifiers.platform || keystroke.modifiers.control || keystroke.modifiers.alt {
            return;
        }
        let input = match keystroke.key.as_str() {
            "enter" => Input::Key("Enter".to_owned()),
            "backspace" => Input::Key("Backspace".to_owned()),
            "delete" => Input::Key("Delete".to_owned()),
            "tab" => Input::Key("Tab".to_owned()),
            "escape" => Input::Key("GoBack".to_owned()),
            "left" => Input::Key("ArrowLeft".to_owned()),
            "right" => Input::Key("ArrowRight".to_owned()),
            "up" => Input::Key("ArrowUp".to_owned()),
            "down" => Input::Key("ArrowDown".to_owned()),
            _ => match &keystroke.key_char {
                Some(text) if !text.is_empty() => Input::Text(text.clone()),
                _ => return,
            },
        };
        self.send(input);
        cx.stop_propagation();
    }

    fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, label: &'static str| button(id, label, Kind::Plain);
        let key = |key: &'static str| {
            cx.listener(move |android: &mut Self, _, window: &mut Window, cx| {
                android.send(Input::Key(key.to_owned()));
                window.focus(&android.focus_handle);
                cx.stop_propagation();
            })
        };
        let state = match &self.status {
            Status::Starting => Some((format!("Starting {}…", self.name), DIM)),
            Status::Reaching => Some((format!("Reaching {}…", self.name), DIM)),
            Status::Showing => None,
            Status::Ended(_) => Some(("Not running".to_owned(), DIM)),
        };
        let running = self.is_running();
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
                    .debug_selector(|| "android-device".to_owned())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(rgb(FOREGROUND))
                    .child(self.name.clone()),
            )
            .children(state.map(|(state, color)| {
                div()
                    .debug_selector(|| "android-state".to_owned())
                    .text_color(rgb(color))
                    .child(state)
            }))
            .when(running, |bar| {
                bar.child(button("android-back", "Back").on_hint_click(key("GoBack")))
                    .child(button("android-home", "Home").on_hint_click(key("GoHome")))
                    .child(button("android-overview", "Overview").on_hint_click(key("AppSwitch")))
                    .child(
                        button("android-stop", "Stop")
                            .on_hint_click(cx.listener(|android, _, _, cx| android.close(cx))),
                    )
            })
            .when(!running, |bar| {
                bar.child(button("android-start", "Start").on_hint_click(
                    cx.listener(|android, _, _, cx| android.open(Boot::default(), cx)),
                ))
            })
    }
}

impl Shows for Android {
    fn title(&self) -> String {
        self.name.clone()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Android {
            device: self.name.clone(),
        }
    }
}

impl Focusable for Android {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Android {
    /// What the tab shows under `over`, a stretch of a region in the
    /// window's points: the device, and where on its display.
    fn described(&self, over: Bounds<Pixels>) -> Described {
        let at = self
            .picture_size()
            .and_then(|size| self.screen.covered(over, size))
            .map_or_else(String::new, |at| format!(", {at}"));
        Described {
            name: self.name.clone(),
            text: format!("the display of the Android device `{}`{at}", self.name),
        }
    }
}

impl Render for Android {
    #[expect(
        clippy::used_underscore_binding,
        reason = "the timer is only held: dropping it cancels it"
    )]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focused.is_none() {
            self.focused = Some(crate::vim::follow_focus(
                crate::vim::Region::Android,
                &self.focus_handle,
                window,
                cx,
            ));
        }
        (self.look, self._pointer_changes) = pointer::look_now(&self.pointer, cx);
        let label = Mirrored::agent(self).and_then(|(look, (width, _))| {
            let drawn = self.screen.drawn_within()?;
            let look = scaled(look, f32::from(drawn.size.width) / width);
            Some(pointer::label(&look, drawn.origin))
        });
        let ended = match &self.status {
            Status::Ended(why) if self.device.is_some() => Some(why.clone()),
            _ => None,
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_bar(cx))
            .child(
                div()
                    .id("android")
                    .debug_selector(|| "android".to_owned())
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .track_focus(&self.focus_handle)
                    .key_context(KEY_CONTEXT)
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        crate::vim::typing(crate::vim::Region::Android, window, cx);
                    })
                    .on_key_down(cx.listener(Self::key_down))
                    .child(Display {
                        mirrored: cx.entity(),
                    })
                    .child(region::mark({
                        let android = cx.weak_entity();
                        move |_, over, cx| Some(android.upgrade()?.read(cx).described(over))
                    }))
                    .children(label)
                    .children(ended.map(|why| {
                        div()
                            .debug_selector(|| "android-ended".to_owned())
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

impl Mirrored for Android {
    fn screen(&mut self) -> &mut Screen {
        &mut self.screen
    }

    fn is_live(&self) -> bool {
        self.device.is_some()
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.send(Input::Size { width, height });
    }

    fn listen(mirrored: &Entity<Self>, over: &Hitbox, window: &mut Window) {
        pass_on(mirrored, over, window);
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "a picture's pixels are far below what f32 keeps exactly"
    )]
    fn agent(&self) -> Option<(&Look, (f32, f32))> {
        let (width, height) = self.picture_size()?;
        Some((self.look.as_ref()?, (width as f32, height as f32)))
    }
}

impl Touched for Android {
    fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// The left button is a finger.
    fn press(&mut self, x: f32, y: f32, press: Press) {
        let down = press != Press::Up;
        self.send(Input::Touch { x, y, down });
    }

    fn scroll(&mut self, _x: f32, _y: f32, (dx, dy): (f32, f32)) {
        self.send(Input::Wheel {
            dx: clicks(dx),
            dy: clicks(dy),
        });
    }
}

/// `lines` of a wheel scroll as the emulator counts the wheel's turns.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a wheel turns a few clicks at a time"
)]
fn clicks(lines: f32) -> i32 {
    (lines * WHEEL_CLICK).round() as i32
}

/// The tab of the device `name`, kept for a closed chat's stage, opens
/// again reaching its emulator if it runs, and else waits to be started.
pub(super) fn reopened(name: String, cx: &mut App) -> Entity<Android> {
    let running = cox_android::running::of(&name).is_some();
    cx.new(|cx| Android::new(name, running.then(Boot::default), cx))
}
