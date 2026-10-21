//! The stage: the part of the window where the browser's picture is shown,
//! and where the human's mouse, keys and input-method text go to the page.
//!
//! The picture arrives from CEF as BGRA pixels whenever the page changes;
//! the stage uploads it as a GPUI image and paints it over its bounds.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, Context, Corners, DispatchPhase, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, IntoElement, KeyDownEvent, KeyUpEvent, LayoutId, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderImage, ScrollDelta, ScrollWheelEvent, Style,
    Task, UTF16Selection, Window, div, fill, point, prelude::*, px, relative, rgb, size,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::browser::{Browser, Caret, Event, Modifiers, MouseButton};

/// How many logical pixels one line of a wheel scroll moves the page.
const LINE_HEIGHT: f32 = 40.;

pub struct Stage {
    browser: Browser,
    focus_handle: FocusHandle,
    /// The picture currently uploaded to the GPU.
    image: Option<Arc<RenderImage>>,
    /// Text the input method is composing, if any.
    composing: Option<String>,
    _events: Task<()>,
}

impl Stage {
    pub fn new(
        browser: Browser,
        mut events: UnboundedReceiver<Event>,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.spawn(async move |this, cx| {
            // Painted, created or closed: the stage looks different either way.
            while let Some(_event) = events.recv().await {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self {
            browser,
            focus_handle: cx.focus_handle(),
            image: None,
            composing: None,
            _events: task,
        }
    }

    /// Called from paint: tells the browser the stage's size, and picks up
    /// the newest picture.
    fn prepare_frame(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> Option<Arc<RenderImage>> {
        self.browser.resize(
            whole_pixels(bounds.size.width),
            whole_pixels(bounds.size.height),
            window.scale_factor(),
        );
        if let Some(frame) = self.browser.take_frame()
            && let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)
        {
            // GPUI keeps images as BGRA too, so the bytes go in as they are.
            let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
            if let Some(old) = self.image.replace(image) {
                window.drop_image(old).ok();
            }
        }
        self.image.clone()
    }

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        self.browser
            .key_down(&keystroke.key, modifiers(keystroke.modifiers));
    }

    fn key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        self.browser
            .key_up(&keystroke.key, modifiers(keystroke.modifiers));
    }
}

/// A position in the window, as a position in the stage in whole logical
/// pixels.
fn in_stage(position: Point<Pixels>, bounds: Bounds<Pixels>) -> (i32, i32) {
    (
        whole_pixels(position.x - bounds.origin.x),
        whole_pixels(position.y - bounds.origin.y),
    )
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "pixel counts fit in i32; rounding is the point"
)]
fn whole_pixels(pixels: Pixels) -> i32 {
    f32::from(pixels).round() as i32
}

fn modifiers(modifiers: gpui::Modifiers) -> Modifiers {
    Modifiers {
        shift: modifiers.shift,
        control: modifiers.control,
        alt: modifiers.alt,
    }
}

fn button(button: gpui::MouseButton) -> Option<MouseButton> {
    match button {
        gpui::MouseButton::Left => Some(MouseButton::Left),
        gpui::MouseButton::Middle => Some(MouseButton::Middle),
        gpui::MouseButton::Right => Some(MouseButton::Right),
        gpui::MouseButton::Navigate(_) => None,
    }
}

fn clicks(count: usize) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

impl Focusable for Stage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// What an input method, or GPUI itself for plain typing, tells the stage.
/// The page keeps the text, so the stage only remembers what is being
/// composed.
impl EntityInputHandler for Stage {
    fn text_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .composing
            .as_ref()
            .map_or(0, |text| text.encode_utf16().count());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.composing
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.composing.take().is_some() {
            self.browser.finish_composition();
        }
        cx.notify();
    }

    /// Final text: typed characters, or a composition the input method
    /// committed.
    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = None;
        self.browser.commit(text);
        cx.notify();
    }

    /// Text the input method is still composing.
    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browser.compose(new_text, new_selected_range_utf16);
        self.composing = Some(new_text.to_owned());
        cx.notify();
    }

    /// Where the input method puts its candidate window: at the page's
    /// composition caret, or the stage's top left corner before the page
    /// says.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel counts are far below what f32 keeps exactly"
    )]
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let caret = self.browser.caret().unwrap_or(Caret {
            x: 0,
            y: 0,
            width: 1,
            height: 20,
        });
        Some(Bounds {
            origin: element_bounds.origin + point(px(caret.x as f32), px(caret.y as f32)),
            size: size(px(caret.width as f32), px(caret.height as f32)),
        })
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for Stage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("stage")
            .debug_selector(|| "stage".to_owned())
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("Stage")
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .child(StageElement { stage: cx.entity() })
    }
}

/// Paints the browser's picture and sends mouse events over it to the page.
struct StageElement {
    stage: Entity<Stage>,
}

impl IntoElement for StageElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for StageElement {
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

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, browser) = {
            let stage = self.stage.read(cx);
            (stage.focus_handle.clone(), stage.browser.clone())
        };
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.stage.clone()),
            cx,
        );

        let image = self
            .stage
            .update(cx, |stage, _| stage.prepare_frame(bounds, window));
        match image {
            Some(image) => {
                window
                    .paint_image(bounds, Corners::default(), image, 0, false)
                    .ok();
            }
            None => window.paint_quad(fill(bounds, rgb(0x33_3333))),
        }

        let hovered = hitbox.clone();
        let target = browser.clone();
        window.on_mouse_event(
            move |event: &MouseDownEvent, phase: DispatchPhase, window, _| {
                if !phase.bubble() || !hovered.is_hovered(window) {
                    return;
                }
                window.focus(&focus_handle);
                if let Some(button) = button(event.button) {
                    let (x, y) = in_stage(event.position, bounds);
                    target.mouse_down(
                        x,
                        y,
                        button,
                        clicks(event.click_count),
                        modifiers(event.modifiers),
                    );
                }
            },
        );

        let target = browser.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase: DispatchPhase, _, _| {
            if !phase.bubble() {
                return;
            }
            if let Some(button) = button(event.button) {
                let (x, y) = in_stage(event.position, bounds);
                target.mouse_up(
                    x,
                    y,
                    button,
                    clicks(event.click_count),
                    modifiers(event.modifiers),
                );
            }
        });

        let hovered = hitbox.clone();
        let target = browser.clone();
        window.on_mouse_event(
            move |event: &MouseMoveEvent, phase: DispatchPhase, window, _| {
                // Moves outside the stage matter only while dragging from it.
                if !phase.bubble()
                    || (!hovered.is_hovered(window) && event.pressed_button.is_none())
                {
                    return;
                }
                let (x, y) = in_stage(event.position, bounds);
                let pressed = event.pressed_button.and_then(button);
                target.mouse_move(x, y, pressed, modifiers(event.modifiers));
            },
        );

        let hovered = hitbox.clone();
        window.on_mouse_event(
            move |event: &ScrollWheelEvent, phase: DispatchPhase, window, _| {
                if !phase.bubble() || !hovered.should_handle_scroll(window) {
                    return;
                }
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => delta,
                    ScrollDelta::Lines(lines) => {
                        point(px(lines.x * LINE_HEIGHT), px(lines.y * LINE_HEIGHT))
                    }
                };
                let (x, y) = in_stage(event.position, bounds);
                browser.wheel(
                    x,
                    y,
                    whole_pixels(delta.x),
                    whole_pixels(delta.y),
                    modifiers(event.modifiers),
                );
            },
        );
    }
}
