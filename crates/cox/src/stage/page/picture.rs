//! The browser's picture of the page, painted over the tab, and the
//! human's mouse over it, which goes to the page. The picture arrives
//! from CEF as BGRA pixels whenever the page changes; each frame uploads
//! the newest as a GPUI image and paints it, with the dropdown of a
//! `<select>`, which arrives as a picture of its own, over it where CEF
//! says it goes, and the agent's pointer over both.

use std::sync::Arc;

use gpui::{
    App, Bounds, ContentMask, Corners, DispatchPhase, Element, ElementId, ElementInputHandler,
    Entity, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderImage, ScrollDelta,
    ScrollWheelEvent, Style, Window, fill, point, px, relative, rgb, size,
};

use super::Page;
use super::agent::paint_agent;
use super::keys::modifiers;
use cox_browser::{Frame, MouseButton};

/// How many logical pixels one line of a wheel scroll moves the page.
const LINE_HEIGHT: f32 = 40.;

impl Page {
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
        if let Some(image) = upload(self.browser.take_frame())
            && let Some(old) = self.image.replace(image)
        {
            window.drop_image(old).ok();
        }
        self.image.clone()
    }

    /// Called from paint: picks up the popup's newest picture, and where
    /// it goes over the stage, while one is open.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel counts are far below what f32 keeps exactly"
    )]
    fn prepare_popup(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> Option<(Arc<RenderImage>, Bounds<Pixels>)> {
        let Some(rect) = self.browser.popup() else {
            if let Some(old) = self.popup.take() {
                window.drop_image(old).ok();
            }
            return None;
        };
        if let Some(image) = upload(self.browser.take_popup_frame())
            && let Some(old) = self.popup.replace(image)
        {
            window.drop_image(old).ok();
        }
        let popup = Bounds {
            origin: bounds.origin + point(px(rect.x as f32), px(rect.y as f32)),
            size: size(px(rect.width as f32), px(rect.height as f32)),
        };
        self.popup.clone().map(|image| (image, popup))
    }
}

/// A picture from CEF as a GPUI image. GPUI keeps images as BGRA too, so
/// the bytes go in as they are.
fn upload(frame: Option<Frame>) -> Option<Arc<RenderImage>> {
    let frame = frame?;
    let buffer = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
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

/// Where the picture goes: at its own size from the stage's top left
/// corner, so a picture painted for another size is cut off or leaves a
/// gap rather than being stretched.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel counts are far below what f32 keeps exactly"
)]
fn image_bounds(image: &RenderImage, bounds: Bounds<Pixels>, scale: f32) -> Bounds<Pixels> {
    let painted = image.size(0);
    Bounds {
        origin: bounds.origin,
        size: size(
            px(painted.width.0 as f32 / scale),
            px(painted.height.0 as f32 / scale),
        ),
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

/// Paints the browser's picture and sends mouse events over it to the page.
pub(super) struct Picture {
    pub(super) page: Entity<Page>,
}

impl Picture {
    /// Paints the page, the popup over it, and the agent's pointer over
    /// both.
    fn paint_page(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let (image, popup, look) = self.page.update(cx, |page, _| {
            (
                page.prepare_frame(bounds, window),
                page.prepare_popup(bounds, window),
                page.look.clone(),
            )
        });
        window.paint_quad(fill(bounds, rgb(0x33_3333)));
        if let Some(image) = image {
            let at = image_bounds(&image, bounds, window.scale_factor());
            window
                .paint_image(at, Corners::default(), image, 0, false)
                .ok();
        }
        if let Some((image, popup)) = popup {
            window
                .paint_image(popup, Corners::default(), image, 0, false)
                .ok();
        }
        if let Some(look) = look {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                paint_agent(&look, bounds, window);
            });
            if look.next_change.is_zero() {
                window.request_animation_frame();
            }
        }
    }
}

impl IntoElement for Picture {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Picture {
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
            let page = self.page.read(cx);
            (page.focus_handle.clone(), page.browser.clone())
        };
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.page.clone()),
            cx,
        );

        self.paint_page(bounds, window, cx);

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
