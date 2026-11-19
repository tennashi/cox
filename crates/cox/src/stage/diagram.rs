//! A tab of the stage that shows a Mermaid diagram of a reply, which the
//! human clicked in the chat, where the chat draws it shrunk to its
//! column: as big as fits in the tab, which the human zooms and pans to
//! read it.
//!
//! The wheel of a mouse, or a scroll with Cmd (Ctrl off the Mac) held,
//! zooms about the pointer; a trackpad's scroll and a drag pan. `=` and
//! `-` zoom about the middle, `0` fits the diagram to the tab, and `1`
//! shows it at its own size, as the bar's buttons do. The tab draws the
//! diagram afresh at each zoom, and only what is in sight, so that its
//! text stays sharp however far in it zooms.

use std::sync::Arc;

use cox_core::tabs::KeptTab;
use gpui::{
    App, Bounds, Context, Corners, CursorStyle, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, Pixels, Point, RenderImage, ScrollDelta, ScrollWheelEvent,
    Size, Window, actions, canvas, div, point, prelude::*, px, rgb, size,
};
use resvg::{tiny_skia, usvg};

use crate::stage::heading::{INSET, body, heading, note};
use crate::stage::tab::Shows;
use crate::ui::diagram;
use crate::vim::hints::Hinted as _;
use coxd::theme::{BUTTON, BUTTON_HOVER, DIM, FAILED, FOREGROUND, RAISED, SURFACE};

pub(crate) const KEY_CONTEXT: &str = "Diagram";

actions!(
    diagram,
    [
        /// Zooms in about the middle of the tab.
        ZoomIn,
        /// Zooms out about the middle of the tab.
        ZoomOut,
        /// Fits the whole diagram to the tab.
        Fit,
        /// Shows the diagram at its own size.
        ActualSize,
    ]
);

/// Binds the keys of the tab.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("=", ZoomIn, Some(KEY_CONTEXT)),
        KeyBinding::new("+", ZoomIn, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-=", ZoomIn, Some(KEY_CONTEXT)),
        KeyBinding::new("-", ZoomOut, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary--", ZoomOut, Some(KEY_CONTEXT)),
        KeyBinding::new("0", Fit, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-0", Fit, Some(KEY_CONTEXT)),
        KeyBinding::new("1", ActualSize, Some(KEY_CONTEXT)),
    ]);
}

/// How far from the tab's edges the diagram is fitted, in points.
pub const MARGIN: f32 = 16.;
/// The least and the most the diagram is zoomed to.
pub const LEAST: f32 = 0.1;
pub const MOST: f32 = 8.;
/// How much one step of zoom, as a key or a line of the wheel, zooms.
pub const STEP: f32 = 1.25;
/// How many points of a precise scroll with Cmd held zoom by `e`, the
/// base of the natural logarithm.
const POINTS_PER_ZOOM: f32 = 200.;

/// How the diagram is shown in the tab: how many of the tab's points
/// each of the diagram's own is, and where the diagram's top left corner
/// is, from the tab's, in the tab's points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f32,
    pub at: Point<f32>,
}

impl View {
    /// The diagram, `diagram` big, as big as fits in a tab `tab` big,
    /// [`MARGIN`] from its edges, in its middle.
    #[must_use]
    pub fn fitted(diagram: Size<f32>, tab: Size<f32>) -> Self {
        let room = |tab: f32| (tab - 2. * MARGIN).max(1.);
        let zoom = (room(tab.width) / diagram.width.max(1.))
            .min(room(tab.height) / diagram.height.max(1.))
            .clamp(LEAST, MOST);
        Self {
            zoom,
            at: point(0., 0.),
        }
        .kept_in(diagram, tab)
    }

    /// Zoomed by `by`, within [`LEAST`] and [`MOST`], about `about`, a
    /// point of the tab, which stays over the same point of the diagram.
    #[must_use]
    pub fn zoomed(self, by: f32, about: Point<f32>, diagram: Size<f32>, tab: Size<f32>) -> Self {
        let zoom = (self.zoom * by).clamp(LEAST, MOST);
        let grown = zoom / self.zoom;
        Self {
            zoom,
            at: point(
                about.x - (about.x - self.at.x) * grown,
                about.y - (about.y - self.at.y) * grown,
            ),
        }
        .kept_in(diagram, tab)
    }

    /// At the diagram's own size, about the middle of the tab.
    #[must_use]
    pub fn actual(self, diagram: Size<f32>, tab: Size<f32>) -> Self {
        self.zoomed(1. / self.zoom, middle(tab), diagram, tab)
    }

    /// Moved by `by`, in the tab's points.
    #[must_use]
    pub fn panned(self, by: Point<f32>, diagram: Size<f32>, tab: Size<f32>) -> Self {
        Self {
            at: point(self.at.x + by.x, self.at.y + by.y),
            ..self
        }
        .kept_in(diagram, tab)
    }

    /// Kept where the diagram is in sight: in the middle across or down
    /// the tab where it fits, and else with no more than [`MARGIN`]
    /// between its edges and the tab's, as a scrolled view keeps it.
    #[must_use]
    pub fn kept_in(self, diagram: Size<f32>, tab: Size<f32>) -> Self {
        let kept = |at: f32, diagram: f32, tab: f32| {
            let shown = diagram * self.zoom;
            if shown + 2. * MARGIN <= tab {
                (tab - shown) / 2.
            } else {
                at.clamp(tab - MARGIN - shown, MARGIN)
            }
        };
        Self {
            at: point(
                kept(self.at.x, diagram.width, tab.width),
                kept(self.at.y, diagram.height, tab.height),
            ),
            ..self
        }
    }
}

/// The middle of a tab `tab` big.
fn middle(tab: Size<f32>) -> Point<f32> {
    point(tab.width / 2., tab.height / 2.)
}

/// What a scroll of the wheel does over the diagram.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scrolled {
    /// Zooms by this much, about the pointer.
    Zoom(f32),
    /// Pans by this much, in the tab's points.
    Pan(Point<f32>),
}

/// What `delta` does, scrolled with Cmd (Ctrl off the Mac) held if
/// `secondary` is: the lines of a mouse's wheel, or any scroll with it
/// held, zoom, in for a scroll up; a trackpad's points pan, as it moves
/// what is under the fingers.
#[must_use]
pub fn scrolled(delta: ScrollDelta, secondary: bool) -> Scrolled {
    match delta {
        ScrollDelta::Lines(lines) => Scrolled::Zoom(STEP.powf(lines.y)),
        ScrollDelta::Pixels(points) => {
            let points = point(f32::from(points.x), f32::from(points.y));
            if secondary {
                Scrolled::Zoom((points.y / POINTS_PER_ZOOM).exp())
            } else {
                Scrolled::Pan(points)
            }
        }
    }
}

/// What a picture of the diagram was drawn for: the view, and the
/// picture's size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drawn {
    view: View,
    pixels: (u32, u32),
}

pub struct Diagram {
    source: String,
    /// The diagram, read, or `None` when it cannot be drawn.
    tree: Option<usvg::Tree>,
    /// How it is shown, once the tab was laid out.
    view: Option<View>,
    /// Whether it is fitted to the tab, as the tab's size changes, until
    /// the human zooms or pans it.
    fitted: bool,
    /// Where the diagram's part of the tab is, in the window, once laid
    /// out.
    area: Option<Bounds<Pixels>>,
    /// The picture last drawn, and what for.
    picture: Option<(Drawn, Arc<RenderImage>)>,
    /// Where a drag that pans was last, while the button is held.
    dragged: Option<Point<Pixels>>,
    focus_handle: FocusHandle,
}

impl Diagram {
    /// A tab showing the diagram `source`, fitted to the tab.
    pub fn new(source: String, cx: &mut Context<Self>) -> Self {
        Self {
            tree: diagram::tree(&source),
            source,
            view: None,
            fitted: true,
            area: None,
            picture: None,
            dragged: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The diagram's Mermaid source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// How the diagram is shown, once the tab was laid out and if it is
    /// drawn.
    #[must_use]
    pub fn view(&self) -> Option<View> {
        self.view.filter(|_| self.tree.is_some())
    }

    /// Whether the diagram is fitted to the tab.
    #[must_use]
    pub fn is_fitted(&self) -> bool {
        self.fitted
    }

    /// The diagram's own size, in its points.
    #[must_use]
    pub fn size(&self) -> Option<Size<f32>> {
        let tree = self.tree.as_ref()?;
        Some(size(tree.size().width(), tree.size().height()))
    }

    /// The tab's part the diagram is in, in its points.
    fn tab(&self) -> Option<Size<f32>> {
        let area = self.area?;
        Some(size(
            f32::from(area.size.width),
            f32::from(area.size.height),
        ))
    }

    /// Shows the diagram as `change` turns the view it has, which the
    /// human chose, so that it is no longer fitted.
    fn change(
        &mut self,
        change: impl FnOnce(View, Size<f32>, Size<f32>) -> View,
        cx: &mut Context<Self>,
    ) {
        let (Some(view), Some(diagram), Some(tab)) = (self.view, self.size(), self.tab()) else {
            return;
        };
        self.view = Some(change(view, diagram, tab));
        self.fitted = false;
        cx.notify();
    }

    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        self.change(
            |view, diagram, tab| view.zoomed(STEP, middle(tab), diagram, tab),
            cx,
        );
    }

    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        self.change(
            |view, diagram, tab| view.zoomed(1. / STEP, middle(tab), diagram, tab),
            cx,
        );
    }

    /// Shows the diagram at its own size.
    pub fn actual_size(&mut self, cx: &mut Context<Self>) {
        self.change(View::actual, cx);
    }

    /// Fits the diagram to the tab, as it is until the human zooms or
    /// pans it.
    pub fn fit(&mut self, cx: &mut Context<Self>) {
        if let (Some(diagram), Some(tab)) = (self.size(), self.tab()) {
            self.view = Some(View::fitted(diagram, tab));
        }
        self.fitted = true;
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(area) = self.area else {
            return;
        };
        let pointer = event.position - area.origin;
        let pointer = point(f32::from(pointer.x), f32::from(pointer.y));
        match scrolled(event.delta, event.modifiers.secondary()) {
            Scrolled::Zoom(by) => {
                self.change(
                    |view, diagram, tab| view.zoomed(by, pointer, diagram, tab),
                    cx,
                );
            }
            Scrolled::Pan(by) => {
                self.change(|view, diagram, tab| view.panned(by, diagram, tab), cx);
            }
        }
        cx.stop_propagation();
    }

    fn press(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        self.dragged = Some(event.position);
        cx.notify();
    }

    fn drag(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.dragged else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragged = None;
            cx.notify();
            return;
        }
        let by = event.position - from;
        self.dragged = Some(event.position);
        let by = point(f32::from(by.x), f32::from(by.y));
        self.change(|view, diagram, tab| view.panned(by, diagram, tab), cx);
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        self.dragged = None;
        cx.notify();
    }

    /// Called as the tab is laid out in `area`: shows the diagram fitted
    /// to it, if it is fitted or not shown yet, and in sight, and draws
    /// it, if it was not drawn so already.
    fn lay_out(
        &mut self,
        area: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        let resized = self.area.map(|was| was.size) != Some(area.size);
        self.area = Some(area);
        let diagram = self.size()?;
        let tab = self.tab()?;
        if resized || self.view.is_none() {
            let view = match self.view {
                Some(view) if !self.fitted => view.kept_in(diagram, tab),
                _ => View::fitted(diagram, tab),
            };
            if self.view != Some(view) {
                self.view = Some(view);
                // The bar shows how far it is zoomed: drawn again once
                // this frame is, as a frame being drawn is not.
                cx.on_next_frame(window, |_, _, cx| cx.notify());
            }
        }
        let view = self.view?;
        let scale = window.scale_factor();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a tab's pixels, rounded up, and at least one"
        )]
        let pixels = |points: f32| (points * scale).ceil().max(1.) as u32;
        let drawn = Drawn {
            view,
            pixels: (pixels(tab.width), pixels(tab.height)),
        };
        if self.picture.as_ref().map(|(was, _)| *was) != Some(drawn) {
            let zoom = view.zoom * scale;
            let transform = tiny_skia::Transform::from_row(
                zoom,
                0.,
                0.,
                zoom,
                view.at.x * scale,
                view.at.y * scale,
            );
            let picture = diagram::picture(
                self.tree.as_ref()?,
                drawn.pixels,
                transform,
                SURFACE,
                RAISED,
            )
            .inspect_err(|error| tracing::debug!("the diagram is not drawn: {error:#}"))
            .ok()?;
            if let Some((_, old)) = self.picture.replace((drawn, picture)) {
                window.drop_image(old).ok();
            }
        }
        self.picture
            .as_ref()
            .map(|(_, picture)| Arc::clone(picture))
    }

    /// A button of the bar, which a test finds as `selector`.
    fn button(
        selector: &'static str,
        label: &'static str,
        clicked: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let tab = cx.weak_entity();
        div()
            .id(selector)
            .debug_selector(move || selector.to_owned())
            .px_2()
            .rounded_sm()
            .bg(rgb(BUTTON))
            .text_color(rgb(FOREGROUND))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON_HOVER)))
            .child(label)
            .on_hint_click(move |_, _, cx| {
                tab.update(cx, |tab, cx| clicked(tab, cx)).ok();
            })
    }

    fn render_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let zoom = self
            .view()
            .map_or_else(String::new, |view| format!("{:.0}%", view.zoom * 100.));
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(Self::button("diagram-zoom-out", "−", Self::zoom_out, cx))
            .child(
                div()
                    .debug_selector(|| "diagram-zoom".to_owned())
                    .w(px(48.))
                    .flex()
                    .justify_center()
                    .text_color(rgb(DIM))
                    .child(zoom),
            )
            .child(Self::button("diagram-zoom-in", "+", Self::zoom_in, cx))
            .child(Self::button("diagram-fit", "Fit", Self::fit, cx))
            .child(Self::button(
                "diagram-actual",
                "100%",
                Self::actual_size,
                cx,
            ))
    }
}

impl Shows for Diagram {
    fn title(&self) -> String {
        "Diagram".to_owned()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Diagram {
            source: self.source.clone(),
        }
    }
}

impl Focusable for Diagram {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Diagram {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self
            .source
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let shown: gpui::AnyElement = if self.tree.is_some() {
            let tab = cx.entity();
            div()
                .id("diagram-picture")
                .debug_selector(|| "diagram-picture".to_owned())
                .flex_1()
                .min_h_0()
                .mx(px(INSET))
                .mb(px(INSET))
                .rounded_md()
                .overflow_hidden()
                .cursor(if self.dragged.is_some() {
                    CursorStyle::ClosedHand
                } else {
                    CursorStyle::OpenHand
                })
                .on_scroll_wheel(cx.listener(Self::scroll))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::press))
                .on_mouse_move(cx.listener(Self::drag))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|tab, _, _, cx| tab.release(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|tab, _, _, cx| tab.release(cx)),
                )
                .child(
                    canvas(
                        move |area, window, cx| {
                            tab.update(cx, |tab, cx| tab.lay_out(area, window, cx))
                        },
                        |area, picture, window, _| {
                            if let Some(picture) = picture {
                                window
                                    .paint_image(area, Corners::default(), picture, 0, false)
                                    .ok();
                            }
                        },
                    )
                    .size_full(),
                )
                .into_any_element()
        } else {
            note("diagram-error", "This diagram cannot be drawn.", FAILED).into_any_element()
        };
        body()
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|tab, _: &ZoomIn, _, cx| tab.zoom_in(cx)))
            .on_action(cx.listener(|tab, _: &ZoomOut, _, cx| tab.zoom_out(cx)))
            .on_action(cx.listener(|tab, _: &Fit, _, cx| tab.fit(cx)))
            .on_action(cx.listener(|tab, _: &ActualSize, _, cx| tab.actual_size(cx)))
            .text_color(rgb(FOREGROUND))
            .child(
                div()
                    .flex()
                    .items_center()
                    .pr(px(INSET))
                    .child(div().flex_1().min_w_0().child(heading(
                        "diagram-heading",
                        "Diagram",
                        kind,
                        None,
                    )))
                    .child(self.render_bar(cx)),
            )
            .child(shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Size<f32> = Size {
        width: 2000.,
        height: 200.,
    };
    const TAB: Size<f32> = Size {
        width: 800.,
        height: 600.,
    };

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_wide_diagram_is_fitted_to_the_tabs_width_in_its_middle() {
        let view = View::fitted(WIDE, TAB);
        assert!(close(view.zoom, (800. - 2. * MARGIN) / 2000.), "{view:?}");
        assert!(close(view.at.x, MARGIN), "{view:?}");
        let height = 200. * view.zoom;
        assert!(close(view.at.y, (600. - height) / 2.), "{view:?}");
    }

    #[test]
    fn a_small_diagram_is_fitted_larger_but_no_more_than_the_most() {
        let view = View::fitted(size(10., 10.), TAB);
        assert!(close(view.zoom, MOST), "{view:?}");
        let view = View::fitted(size(100., 100.), TAB);
        assert!(close(view.zoom, (600. - 2. * MARGIN) / 100.), "{view:?}");
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let view = View::fitted(WIDE, TAB);
        let about = point(300., 310.);
        let under = |view: View| {
            point(
                (about.x - view.at.x) / view.zoom,
                (about.y - view.at.y) / view.zoom,
            )
        };
        let zoomed = view.zoomed(4., about, WIDE, TAB);
        assert!(close(zoomed.zoom, view.zoom * 4.));
        assert!(close(under(zoomed).x, under(view).x), "{zoomed:?}");
        // Down, it still fits, so it stays in the middle.
        assert!(
            close(zoomed.at.y, (600. - 200. * zoomed.zoom) / 2.),
            "{zoomed:?}"
        );
    }

    #[test]
    fn zoom_stays_within_the_least_and_the_most() {
        let view = View::fitted(WIDE, TAB);
        assert!(close(view.zoomed(1000., middle(TAB), WIDE, TAB).zoom, MOST));
        assert!(close(
            view.zoomed(0.0001, middle(TAB), WIDE, TAB).zoom,
            LEAST
        ));
    }

    #[test]
    fn at_its_own_size_the_middle_of_the_tab_stays_over_the_same_point() {
        let view = View::fitted(WIDE, TAB);
        let actual = view.actual(WIDE, TAB);
        assert!(close(actual.zoom, 1.));
        // The middle of the diagram, across, stays in the middle.
        assert!(close(actual.at.x, 400. - 1000.), "{actual:?}");
        // Down, 200 points fit in 600: it stays in the middle.
        assert!(close(actual.at.y, 200.), "{actual:?}");
    }

    #[test]
    fn panning_stops_at_the_diagrams_edges() {
        let actual = View::fitted(WIDE, TAB).actual(WIDE, TAB);
        let far_left = actual.panned(point(5000., 0.), WIDE, TAB);
        assert!(close(far_left.at.x, MARGIN), "{far_left:?}");
        let far_right = actual.panned(point(-5000., 0.), WIDE, TAB);
        assert!(
            close(far_right.at.x, 800. - MARGIN - 2000.),
            "{far_right:?}"
        );
        // Down it fits, so it stays in the middle.
        let down = actual.panned(point(0., 100.), WIDE, TAB);
        assert!(close(down.at.y, 200.), "{down:?}");
    }

    #[test]
    fn a_mouses_wheel_zooms_and_a_trackpad_pans_unless_cmd_is_held() {
        assert_eq!(
            scrolled(ScrollDelta::Lines(point(0., 1.)), false),
            Scrolled::Zoom(STEP)
        );
        let points = ScrollDelta::Pixels(point(px(3.), px(-4.)));
        assert_eq!(scrolled(points, false), Scrolled::Pan(point(3., -4.)));
        let Scrolled::Zoom(by) = scrolled(points, true) else {
            panic!("a scroll with Cmd held zooms");
        };
        assert!(by < 1., "a scroll down zooms out: {by}");
    }
}
