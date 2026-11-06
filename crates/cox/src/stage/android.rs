//! A tab of the stage showing an Android emulator, as Android Studio's
//! "Running Devices" does: Cox starts the emulator of a virtual device
//! with no window of its own, or reaches it if it runs already, and the
//! tab draws its display, which the human touches with the mouse, types
//! into, scrolls with the wheel, and presses Back, Home and Overview of
//! on the bar above it (see [`cox_android::device`]).
//!
//! The emulator Cox started stops as its tab closes, or as Cox quits,
//! saving its state for its next start; one that ran already goes on.

use std::sync::Arc;

use cox_android::device::{Boot, Device, Event, Frame, Input, Status};
use cox_core::tabs::KeptTab;
use gpui::{
    App, Bounds, Context, Corners, DispatchPhase, Element, ElementId, Entity, FocusHandle,
    Focusable, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement,
    KeyDownEvent, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, RenderImage, ScrollDelta, ScrollWheelEvent, Style, Subscription, Task, Window, div,
    fill, point, prelude::*, px, relative, rgb, size,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::stage::tab::Shows;
use crate::ui::theme::{BORDER, BUTTON, DIM, FAILED, FOREGROUND, RAISED, SELECTED};

/// How many logical pixels one line of a wheel scroll is.
const LINE_HEIGHT: f32 = 40.;

/// How far one click of a mouse's wheel turns it, as the emulator counts.
const WHEEL_CLICK: f32 = 120.;

pub struct Android {
    /// The device, by its name as `emulator -avd` takes it.
    name: String,
    /// The emulator, while the tab talks to one.
    device: Option<Device>,
    status: Status,
    /// The display as last uploaded to the GPU.
    image: Option<Arc<RenderImage>>,
    /// A frame that came, waiting to be uploaded as the tab is drawn.
    frame: Option<Frame>,
    /// The size the emulator was last asked to draw at, in pixels.
    asked: Option<(u32, u32)>,
    /// Where the display was drawn last, in the window.
    drawn: Option<Bounds<Pixels>>,
    /// Whether the mouse's button touches the display.
    touching: bool,
    focus_handle: FocusHandle,
    /// Hears the emulator, while the tab talks to one.
    hearing: Option<Task<()>>,
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
            image: None,
            frame: None,
            asked: None,
            drawn: None,
            touching: false,
            focus_handle: cx.focus_handle(),
            hearing: None,
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

    /// Starts the device's emulator as `boot` says, or reaches it if it
    /// runs.
    pub fn open(&mut self, boot: Boot, cx: &mut Context<Self>) {
        let (device, events) = Device::open(cox_android::sdk::emulator(), &self.name, boot);
        self.device = Some(device);
        self.asked = None;
        self.hearing = Some(Self::hear(events, cx));
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
                                android.frame = Some(frame);
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

    /// Where `position` is across the display, from 0 to 1 each way, as
    /// it was drawn last.
    fn across(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let drawn = self.drawn?;
        let x = (position.x - drawn.origin.x) / drawn.size.width;
        let y = (position.y - drawn.origin.y) / drawn.size.height;
        Some((x.clamp(0., 1.), y.clamp(0., 1.)))
    }

    fn touch(&mut self, position: Point<Pixels>, down: bool) {
        if let Some((x, y)) = self.across(position) {
            self.send(Input::Touch { x, y, down });
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

    /// Called from paint: asks the emulator for frames the size of
    /// `bounds`, and uploads the newest.
    fn prepare(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> Option<Arc<RenderImage>> {
        let scale = window.scale_factor();
        let wanted = (
            pixels(bounds.size.width, scale),
            pixels(bounds.size.height, scale),
        );
        if self.asked != Some(wanted) && wanted.0 > 0 && wanted.1 > 0 && self.device.is_some() {
            self.send(Input::Size {
                width: wanted.0,
                height: wanted.1,
            });
            self.asked = Some(wanted);
        }
        if let Some(image) = self.frame.take().and_then(upload)
            && let Some(old) = self.image.replace(image)
        {
            window.drop_image(old).ok();
        }
        self.image.clone()
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
                bar.child(button("android-back", "Back").on_click(key("GoBack")))
                    .child(button("android-home", "Home").on_click(key("GoHome")))
                    .child(button("android-overview", "Overview").on_click(key("AppSwitch")))
                    .child(
                        button("android-stop", "Stop")
                            .on_click(cx.listener(|android, _, _, cx| android.close(cx))),
                    )
            })
            .when(!running, |bar| {
                bar.child(
                    button("android-start", "Start").on_click(
                        cx.listener(|android, _, _, cx| android.open(Boot::default(), cx)),
                    ),
                )
            })
    }
}

/// `length` in whole pixels at `scale`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a stage's pixels fit in u32; rounding is the point"
)]
fn pixels(length: Pixels, scale: f32) -> u32 {
    (f32::from(length) * scale).round().max(0.) as u32
}

/// A frame of the emulator's as a GPUI image, which keeps BGRA as the
/// frame does.
fn upload(frame: Frame) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
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

impl Render for Android {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .on_key_down(cx.listener(Self::key_down))
                    .child(Display {
                        android: cx.entity(),
                    })
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

/// Paints the emulator's display, as big as fits, in the middle of the
/// tab, and sends the mouse over it to the emulator as touches and the
/// wheel.
struct Display {
    android: Entity<Android>,
}

impl IntoElement for Display {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Display {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "a frame's pixels are far below what f32 keeps exactly"
    )]
    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_quad(fill(bounds, rgb(RAISED)));
        let image = self
            .android
            .update(cx, |android, _| android.prepare(bounds, window));
        let drawn = image.map(|image| {
            let scale = window.scale_factor();
            let painted = image.size(0);
            let shown = size(
                px(painted.width.0 as f32 / scale),
                px(painted.height.0 as f32 / scale),
            );
            let at = Bounds {
                origin: bounds.origin
                    + point(
                        (bounds.size.width - shown.width) / 2.,
                        (bounds.size.height - shown.height) / 2.,
                    ),
                size: shown,
            };
            window
                .paint_image(at, Corners::default(), image, 0, false)
                .ok();
            at
        });
        self.android.update(cx, |android, _| android.drawn = drawn);

        let android = self.android.clone();
        let over = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && over.is_hovered(window)
            {
                android.update(cx, |android, cx| {
                    window.focus(&android.focus_handle);
                    android.touching = true;
                    android.touch(event.position, true);
                    cx.stop_propagation();
                });
            }
        });
        let android = self.android.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble {
                android.update(cx, |android, _| {
                    if android.touching {
                        android.touch(event.position, true);
                    }
                });
            }
        });
        let android = self.android.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                android.update(cx, |android, _| {
                    if android.touching {
                        android.touching = false;
                        android.touch(event.position, false);
                    }
                });
            }
        });
        let android = self.android.clone();
        let over = hitbox.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && over.is_hovered(window) {
                let (dx, dy) = match event.delta {
                    ScrollDelta::Lines(lines) => (lines.x, lines.y),
                    ScrollDelta::Pixels(pixels) => (
                        f32::from(pixels.x) / LINE_HEIGHT,
                        f32::from(pixels.y) / LINE_HEIGHT,
                    ),
                };
                android.update(cx, |android, _| {
                    android.send(Input::Wheel {
                        dx: clicks(dx),
                        dy: clicks(dy),
                    });
                });
            }
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
