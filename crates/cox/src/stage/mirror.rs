//! What the tabs that show another program's screen share, as an Android
//! emulator's display or a Mac app's window: the picture as it last came,
//! drawn as big as fits in the middle of the tab, with the agent's pointer
//! over it, for a program the agent uses; and, for a program that takes
//! it, the mouse over it, passed on as presses of its left button, drags
//! and the wheel, where they fall across the picture.

use std::sync::Arc;

use cox_browser::pointer::{Look, Spot};
use gpui::{
    App, Bounds, ContentMask, Corners, DispatchPhase, Element, ElementId, Entity, FocusHandle,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderImage,
    ScrollDelta, ScrollWheelEvent, Style, Window, fill, point, px, relative, rgb, size,
};

use crate::chat::region;
use crate::stage::pointer::paint_agent;
use coxd::theme::RAISED;

/// How many logical pixels one line of a wheel scroll is.
pub(super) const LINE_HEIGHT: f32 = 40.;

/// What the left button of the mouse does over the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Press {
    Down,
    /// Moves while held down.
    Drag,
    Up,
}

/// The picture a tab shows, and where it drew it.
#[derive(Default)]
pub(super) struct Screen {
    /// The picture as last uploaded to the GPU.
    image: Option<Arc<RenderImage>>,
    /// A picture that came, waiting to be uploaded as the tab is drawn:
    /// its width, its height, and its pixels as BGRA.
    waiting: Option<(u32, u32, Vec<u8>)>,
    /// The size the program was last asked to draw at, in pixels.
    asked: Option<(u32, u32)>,
    /// Where the picture was drawn last, in the window.
    drawn: Option<Bounds<Pixels>>,
    /// Where the tab drew the picture in, in the window, as big as it is.
    area: Option<Bounds<Pixels>>,
    /// Whether the mouse's left button is down on the picture.
    pressed: bool,
}

impl Screen {
    /// Keeps a picture that came, to upload as the tab is drawn.
    pub(super) fn put(&mut self, width: u32, height: u32, bgra: Vec<u8>) {
        self.waiting = Some((width, height, bgra));
    }

    /// Asks the size again the next time the tab is drawn, as when the
    /// program starts again.
    pub(super) fn ask_again(&mut self) {
        self.asked = None;
    }

    /// The picture as it is now, as BGRA: the one waiting to be drawn, or
    /// else the one drawn last.
    pub(super) fn picture(&self) -> Option<(u32, u32, Vec<u8>)> {
        if let Some(waiting) = &self.waiting {
            return Some(waiting.clone());
        }
        let image = self.image.as_ref()?;
        let size = image.size(0);
        let width = u32::try_from(size.width.0).ok()?;
        let height = u32::try_from(size.height.0).ok()?;
        Some((width, height, image.as_bytes(0)?.to_vec()))
    }

    /// How big the picture is now, in pixels: the one waiting to be drawn,
    /// or else the one drawn last.
    pub(super) fn size(&self) -> Option<(u32, u32)> {
        if let Some((width, height, _)) = &self.waiting {
            return Some((*width, *height));
        }
        let size = self.image.as_ref()?.size(0);
        Some((
            u32::try_from(size.width.0).ok()?,
            u32::try_from(size.height.0).ok()?,
        ))
    }

    /// Where the picture was drawn last, from the top left corner of the
    /// tab's area for it.
    pub(super) fn drawn_within(&self) -> Option<Bounds<Pixels>> {
        let (drawn, area) = (self.drawn?, self.area?);
        Some(Bounds {
            origin: drawn.origin - area.origin,
            size: drawn.size,
        })
    }

    /// Where on the picture `over`, a stretch of a region in the window's
    /// points, is, in the pixels of a picture of it `width` by `height`,
    /// as the agent gets it, if it is over the picture as drawn last.
    pub(super) fn covered(
        &self,
        over: Bounds<Pixels>,
        (width, height): (u32, u32),
    ) -> Option<String> {
        let drawn = self.drawn?;
        let over = over.intersect(&drawn);
        if over.size.width <= px(0.) || over.size.height <= px(0.) {
            return None;
        }
        let (left, right) = region::along(
            (over.left(), over.right()),
            (drawn.left(), drawn.size.width),
            width,
        );
        let (top, bottom) = region::along(
            (over.top(), over.bottom()),
            (drawn.top(), drawn.size.height),
            height,
        );
        Some(format!(
            "from {left},{top} to {right},{bottom} in the pixels of the {width}×{height} picture \
            of it the agent's tools answer and take"
        ))
    }

    /// Where `position` is across the picture, from 0 to 1 each way, as it
    /// was drawn last.
    fn across(&self, position: Point<Pixels>) -> Option<(f32, f32)> {
        let drawn = self.drawn?;
        let x = (position.x - drawn.origin.x) / drawn.size.width;
        let y = (position.y - drawn.origin.y) / drawn.size.height;
        Some((x.clamp(0., 1.), y.clamp(0., 1.)))
    }
}

/// A tab that shows another program's screen.
pub(super) trait Mirrored: 'static {
    fn screen(&mut self) -> &mut Screen;
    /// Whether the program is there to be asked a size of.
    fn is_live(&self) -> bool;
    /// Asks the program for pictures `width` by `height` pixels.
    fn resize(&mut self, width: u32, height: u32);
    /// How the agent's pointer looks over the picture, if it shows, and how
    /// big the program's screen is, in the units the look has its spots.
    fn agent(&self) -> Option<(&Look, (f32, f32))> {
        None
    }
    /// Has the mouse over the picture, `over`, go on to the program, if it
    /// takes it, as [`pass_on`] does.
    fn listen(_mirrored: &Entity<Self>, _over: &Hitbox, _window: &mut Window)
    where
        Self: Sized,
    {
    }
}

/// A tab whose program takes the mouse over its picture.
pub(super) trait Touched: Mirrored {
    fn focus_handle(&self) -> &FocusHandle;
    /// The mouse's left button, at `x` and `y` across the picture.
    fn press(&mut self, x: f32, y: f32, press: Press);
    /// The wheel turned by `lines` each way, at `x` and `y` across the
    /// picture.
    fn scroll(&mut self, x: f32, y: f32, lines: (f32, f32));
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

/// A picture as a GPUI image, which keeps BGRA as the picture does.
fn upload(width: u32, height: u32, bgra: Vec<u8>) -> Option<Arc<RenderImage>> {
    let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// Called from paint: asks the program for pictures the size of `bounds`,
/// and uploads the newest.
fn prepare<T: Mirrored>(
    mirrored: &mut T,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) -> Option<Arc<RenderImage>> {
    let scale = window.scale_factor();
    let wanted = (
        pixels(bounds.size.width, scale),
        pixels(bounds.size.height, scale),
    );
    let live = mirrored.is_live();
    if mirrored.screen().asked != Some(wanted) && wanted.0 > 0 && wanted.1 > 0 && live {
        mirrored.resize(wanted.0, wanted.1);
        mirrored.screen().asked = Some(wanted);
    }
    let screen = mirrored.screen();
    if let Some(image) = screen
        .waiting
        .take()
        .and_then(|(width, height, bgra)| upload(width, height, bgra))
        && let Some(old) = screen.image.replace(image)
    {
        window.drop_image(old).ok();
    }
    screen.image.clone()
}

/// Paints the picture of `T`'s program, as big as fits, in the middle of
/// the tab, and passes the mouse over it on, if the program takes it.
pub(super) struct Display<T: Mirrored> {
    pub(super) mirrored: Entity<T>,
}

impl<T: Mirrored> IntoElement for Display<T> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<T: Mirrored> Element for Display<T> {
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
        reason = "a picture's pixels are far below what f32 keeps exactly"
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
            .mirrored
            .update(cx, |mirrored, _| prepare(mirrored, bounds, window));
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
        let agent = self.mirrored.update(cx, |mirrored, _| {
            let screen = mirrored.screen();
            screen.drawn = drawn;
            screen.area = Some(bounds);
            let (look, (width, _)) = mirrored.agent()?;
            Some((scaled(look, f32::from(drawn?.size.width) / width), drawn?))
        });
        if let Some((look, drawn)) = agent {
            window.with_content_mask(Some(ContentMask { bounds: drawn }), |window| {
                paint_agent(&look, drawn, window);
            });
            if look.next_change.is_zero() {
                window.request_animation_frame();
            }
        }

        T::listen(&self.mirrored, hitbox, window);
    }
}

/// `look` as big as the picture is drawn, `scale` times what it says.
pub(super) fn scaled(look: &Look, scale: f32) -> Look {
    let spot = |spot: Spot| Spot {
        x: spot.x * scale,
        y: spot.y * scale,
    };
    let mut look = look.clone();
    look.pointer = spot(look.pointer);
    for ripple in &mut look.ripples {
        ripple.at = spot(ripple.at);
    }
    if let Some(field) = &mut look.field {
        let corner = spot(Spot {
            x: field.x,
            y: field.y,
        });
        field.x = corner.x;
        field.y = corner.y;
        field.width *= scale;
        field.height *= scale;
    }
    look
}

/// Passes the mouse over the picture, `over`, on to `mirrored`'s program:
/// the left button, as it goes down, drags and comes up, and the wheel.
pub(super) fn pass_on<T: Touched>(mirrored: &Entity<T>, over: &Hitbox, window: &mut Window) {
    let (to, hovered) = (mirrored.clone(), over.clone());
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble
            && event.button == MouseButton::Left
            && hovered.is_hovered(window)
        {
            to.update(cx, |mirrored, cx| {
                window.focus(mirrored.focus_handle());
                let screen = mirrored.screen();
                screen.pressed = true;
                if let Some((x, y)) = screen.across(event.position) {
                    mirrored.press(x, y, Press::Down);
                }
                cx.stop_propagation();
            });
        }
    });
    let to = mirrored.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            to.update(cx, |mirrored, _| {
                let screen = mirrored.screen();
                if screen.pressed
                    && let Some((x, y)) = screen.across(event.position)
                {
                    mirrored.press(x, y, Press::Drag);
                }
            });
        }
    });
    let to = mirrored.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            to.update(cx, |mirrored, _| {
                let screen = mirrored.screen();
                if screen.pressed {
                    screen.pressed = false;
                    if let Some((x, y)) = screen.across(event.position) {
                        mirrored.press(x, y, Press::Up);
                    }
                }
            });
        }
    });
    let (to, hovered) = (mirrored.clone(), over.clone());
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && hovered.is_hovered(window) {
            let lines = match event.delta {
                ScrollDelta::Lines(lines) => (lines.x, lines.y),
                ScrollDelta::Pixels(pixels) => (
                    f32::from(pixels.x) / LINE_HEIGHT,
                    f32::from(pixels.y) / LINE_HEIGHT,
                ),
            };
            to.update(cx, |mirrored, _| {
                if let Some((x, y)) = mirrored.screen().across(event.position) {
                    mirrored.scroll(x, y, lines);
                }
            });
        }
    });
}
