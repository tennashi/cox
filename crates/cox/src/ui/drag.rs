//! A drag that selects text in a part that scrolls, as in any app that
//! shows text: held past the top or bottom of the part, it scrolls the
//! part, the faster the farther past, and what it selects grows as the
//! text goes by; a turn of the wheel during it grows what it selects to
//! the text the pointer is on then.
//!
//! The part keeps a [`Dragging`], starts it as a press starts selecting,
//! puts [`follow`] in what scrolls, and says, as [`Drags`], how it
//! selects to a point and how it scrolls. What it selects is of the text,
//! not of the screen, so what scrolls out of sight stays selected.

use std::time::Duration;

use gpui::{
    Bounds, Context, DispatchPhase, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, Styled as _, canvas, point, px,
};

/// How often a drag held past an edge scrolls: once a frame.
const FRAME: Duration = Duration::from_millis(16);

/// How far a drag past an edge scrolls in a frame: a few points just
/// past it, a point more for each four farther, up to this many.
const SLOWEST: f32 = 2.;
const FASTEST: f32 = 40.;

/// A drag of the left button that selects, in a part that scrolls.
#[derive(Default)]
pub struct Dragging {
    /// Where the pointer is, in the window, while the button is held.
    pointer: Option<Point<Pixels>>,
    /// Whether the pointer moved since the press: until it does, what the
    /// press selected, such as a word on a double click, stays.
    moved: bool,
    /// Where what scrolls shows, as the last frame drew it.
    bounds: Bounds<Pixels>,
    /// Whether the part scrolls a frame at a time, while the pointer is
    /// past an edge.
    scrolling: bool,
}

impl Dragging {
    /// A press at `position` starts selecting.
    pub fn start(&mut self, position: Point<Pixels>) {
        self.pointer = Some(position);
        self.moved = false;
    }

    /// Whether the button is held, selecting.
    #[must_use]
    pub fn goes_on(&self) -> bool {
        self.pointer.is_some()
    }
}

/// A part a [`Dragging`] selects in.
pub trait Drags: Sized + 'static {
    fn dragging(&mut self) -> &mut Dragging;

    /// Moves the end of what is selected to `position`, within what
    /// scrolls: answers whether it moved.
    fn select_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool;

    /// Scrolls by `distance`, toward the end when positive.
    fn scroll_by(&mut self, distance: Pixels, cx: &mut Context<Self>);
}

/// Follows the drag, while the button is held, of the part `cx` is of, put
/// in what scrolls, which has to be `relative`: it takes the whole of it,
/// and draws nothing.
pub fn follow<V: Drags>(
    dragging: &Dragging,
    cx: &mut Context<V>,
) -> Option<impl IntoElement + use<V>> {
    dragging.goes_on().then(|| {
        let entity = cx.entity();
        canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                // After the frame lays the text out anew, as after a turn
                // of the wheel, what is selected goes to it.
                window.defer(cx, {
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |view, cx| {
                            view.dragging().bounds = bounds;
                            select(view, cx);
                        });
                    }
                });
                // Outside the part too, as the pointer goes in a drag.
                let moved = entity.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    moved.update(cx, |view, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            move_to(view, event.position, cx);
                        } else {
                            // The button came up where the window did
                            // not hear of it.
                            view.dragging().pointer = None;
                            cx.notify();
                        }
                    });
                });
                let released = entity.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                        released.update(cx, |view, cx| {
                            if view
                                .dragging()
                                .pointer
                                .is_some_and(|at| at != event.position)
                            {
                                view.dragging().pointer = Some(event.position);
                                view.dragging().moved = true;
                                select(view, cx);
                            }
                            view.dragging().pointer = None;
                            cx.notify();
                        });
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    })
}

/// The pointer moved to `position`, holding the button.
fn move_to<V: Drags>(view: &mut V, position: Point<Pixels>, cx: &mut Context<V>) {
    let dragging = view.dragging();
    if dragging.pointer.is_none_or(|at| at == position) {
        return;
    }
    dragging.pointer = Some(position);
    dragging.moved = true;
    let scrolls = speed(dragging.bounds, position.y) != px(0.) && !dragging.scrolling;
    select(view, cx);
    if scrolls {
        view.dragging().scrolling = true;
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(FRAME).await;
                let goes_on = view.update(cx, |view, cx| {
                    let dragging = view.dragging();
                    let distance = dragging
                        .pointer
                        .map_or(px(0.), |pointer| speed(dragging.bounds, pointer.y));
                    if distance == px(0.) {
                        dragging.scrolling = false;
                        return false;
                    }
                    view.scroll_by(distance, cx);
                    // Of the text as the last frame laid it out, until
                    // the next frame lays it out anew: a window that
                    // others cover draws no frames.
                    select(view, cx);
                    cx.notify();
                    true
                });
                if !matches!(goes_on, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }
}

/// Selects to where the pointer is, or to the nearest point of what
/// scrolls while it is past it.
fn select<V: Drags>(view: &mut V, cx: &mut Context<V>) {
    let dragging = view.dragging();
    let Some(pointer) = dragging.pointer.filter(|_| dragging.moved) else {
        return;
    };
    if dragging.bounds.is_empty() {
        return;
    }
    let to = within(dragging.bounds, pointer);
    if view.select_to(to, cx) {
        cx.notify();
    }
}

/// The point of `bounds` nearest `position`.
fn within(bounds: Bounds<Pixels>, position: Point<Pixels>) -> Point<Pixels> {
    let last = |start: Pixels, length: Pixels| start + (length - px(1.)).max(px(0.));
    point(
        position
            .x
            .clamp(bounds.left(), last(bounds.left(), bounds.size.width)),
        position
            .y
            .clamp(bounds.top(), last(bounds.top(), bounds.size.height)),
    )
}

/// How far a drag at `y` scrolls what shows in `bounds` in a frame: up
/// when negative, past its top, down past its bottom, and not at all
/// within it.
fn speed(bounds: Bounds<Pixels>, y: Pixels) -> Pixels {
    let past = if bounds.is_empty() {
        return px(0.);
    } else if y < bounds.top() {
        y - bounds.top()
    } else if y >= bounds.bottom() {
        y - bounds.bottom() + px(1.)
    } else {
        return px(0.);
    };
    let points = (f32::from(past.abs()) / 4.).clamp(SLOWEST, FASTEST);
    px(points.copysign(f32::from(past)))
}

#[cfg(test)]
mod tests {
    use gpui::size;

    use super::*;

    fn shown() -> Bounds<Pixels> {
        Bounds::new(point(px(10.), px(100.)), size(px(200.), px(300.)))
    }

    #[test]
    fn a_drag_within_what_shows_does_not_scroll() {
        assert_eq!(speed(shown(), px(100.)), px(0.));
        assert_eq!(speed(shown(), px(250.)), px(0.));
        assert_eq!(speed(shown(), px(399.)), px(0.));
    }

    #[test]
    fn a_drag_in_a_part_not_drawn_yet_does_not_scroll() {
        assert_eq!(speed(Bounds::default(), px(500.)), px(0.));
    }

    #[test]
    fn a_drag_past_the_top_scrolls_up_and_past_the_bottom_down() {
        assert!(speed(shown(), px(99.)) < px(0.));
        assert!(speed(shown(), px(400.)) > px(0.));
    }

    #[test]
    fn the_farther_past_the_edge_the_faster_up_to_a_most() {
        let near = speed(shown(), px(410.));
        let far = speed(shown(), px(500.));
        let farthest = speed(shown(), px(5000.));
        assert!(px(0.) < near && near < far && far <= farthest);
        assert_eq!(farthest, speed(shown(), px(9000.)));
        assert_eq!(speed(shown(), px(-5000.)), -farthest);
    }

    #[test]
    fn a_pointer_past_what_shows_selects_to_its_nearest_point() {
        assert_eq!(
            within(shown(), point(px(0.), px(0.))),
            point(px(10.), px(100.))
        );
        assert_eq!(
            within(shown(), point(px(500.), px(900.))),
            point(px(209.), px(399.))
        );
        let inside = point(px(50.), px(200.));
        assert_eq!(within(shown(), inside), inside);
    }
}
