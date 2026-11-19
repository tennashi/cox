//! A human reading a wide Mermaid diagram of a reply on a tab of the
//! stage: fitted to the tab first, then zoomed and panned.

use cox::stage::diagram::{Diagram, MARGIN, STEP, View};
use cox::stage::tab::Shows as _;
use cox_core::tabs::KeptTab;
use gpui::{
    Bounds, Entity, Focusable as _, Modifiers, MouseButton, Pixels, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, point, px, size,
};

/// A flowchart of twelve steps in a row, far wider than it is tall.
const WIDE: &str = "flowchart LR\n  A[Plan] --> B[Read] --> C[Write] --> D[Test] --> E[Review] \
                    --> F[Fix] --> G[Build] --> H[Check] --> I[Merge] --> J[Ship] --> K[Watch] \
                    --> L[Done]";

/// Opens a tab showing the diagram `source`, with the keyboard in it, in
/// a window 800 points by 600.
fn open<'a>(
    cx: &'a mut TestAppContext,
    source: &str,
) -> (Entity<Diagram>, &'a mut VisualTestContext) {
    cx.update(cox::stage::diagram::bind_keys);
    let source = source.to_owned();
    let (diagram, cx) = cx.add_window_view(|_, cx| Diagram::new(source, cx));
    cx.update(|window, cx| window.focus(&diagram.focus_handle(cx)));
    cx.simulate_resize(size(px(800.), px(600.)));
    cx.run_until_parked();
    (diagram, cx)
}

fn view(diagram: &Entity<Diagram>, cx: &mut VisualTestContext) -> View {
    diagram
        .read_with(cx, |diagram, _| diagram.view())
        .expect("the diagram is drawn")
}

/// Where the diagram is drawn in the window.
fn picture(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds("diagram-picture")
        .expect("the diagram's part of the tab is drawn")
}

fn zoom_shown(cx: &mut VisualTestContext) -> bool {
    cx.debug_bounds("diagram-zoom").is_some()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-2
}

#[gpui::test]
fn a_wide_diagram_opens_fitted_to_the_tab_and_is_kept_by_its_source(cx: &mut TestAppContext) {
    let (diagram, cx) = open(cx, WIDE);
    let shown = view(&diagram, cx);
    let area = picture(cx);
    let own = diagram.read_with(cx, |diagram, _| diagram.size()).unwrap();
    assert!(own.width > own.height * 4., "a wide one: {own:?}");
    let width = f32::from(area.size.width);
    assert!(
        close(shown.zoom * own.width, width - 2. * MARGIN),
        "fitted across: {shown:?} in {area:?}"
    );
    assert!(shown.zoom < 1., "shrunk to fit: {shown:?}");
    assert!(zoom_shown(cx));
    assert_eq!(
        diagram.read_with(cx, |diagram, _| diagram.kept()),
        KeptTab::Diagram {
            source: WIDE.to_owned()
        }
    );
}

#[gpui::test]
fn keys_zoom_show_its_own_size_and_fit_it_again(cx: &mut TestAppContext) {
    let (diagram, cx) = open(cx, WIDE);
    let fitted = view(&diagram, cx);

    cx.simulate_keystrokes("=");
    assert!(close(view(&diagram, cx).zoom, fitted.zoom * STEP));
    assert!(!diagram.read_with(cx, |diagram, _| diagram.is_fitted()));
    cx.simulate_keystrokes("- -");
    assert!(close(view(&diagram, cx).zoom, fitted.zoom / STEP));

    cx.simulate_keystrokes("1");
    assert!(close(view(&diagram, cx).zoom, 1.));

    cx.simulate_keystrokes("0");
    assert_eq!(view(&diagram, cx), fitted);
    assert!(diagram.read_with(cx, |diagram, _| diagram.is_fitted()));
}

#[gpui::test]
fn the_bars_buttons_zoom_and_fit(cx: &mut TestAppContext) {
    let (diagram, cx) = open(cx, WIDE);
    let fitted = view(&diagram, cx);
    let click = |cx: &mut VisualTestContext, button: &'static str| {
        let bounds = cx.debug_bounds(button).expect("the button is drawn");
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
    };
    click(cx, "diagram-zoom-in");
    assert!(close(view(&diagram, cx).zoom, fitted.zoom * STEP));
    click(cx, "diagram-actual");
    assert!(close(view(&diagram, cx).zoom, 1.));
    click(cx, "diagram-zoom-out");
    assert!(close(view(&diagram, cx).zoom, 1. / STEP));
    click(cx, "diagram-fit");
    assert_eq!(view(&diagram, cx), fitted);
}

#[gpui::test]
fn the_wheel_zooms_about_the_pointer_and_a_drag_pans(cx: &mut TestAppContext) {
    let (diagram, cx) = open(cx, WIDE);
    let fitted = view(&diagram, cx);
    let area = picture(cx);
    // A point a quarter of the way across the diagram's part of the tab.
    let pointer = area.origin + point(area.size.width / 4., area.size.height / 2.);
    let at = |view: View| {
        let x = f32::from(pointer.x - area.origin.x);
        (x - view.at.x) / view.zoom
    };
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: ScrollDelta::Lines(point(0., 3.)),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    let zoomed = view(&diagram, cx);
    assert!(close(zoomed.zoom, fitted.zoom * STEP.powi(3)), "{zoomed:?}");
    assert!(
        close(at(zoomed), at(fitted)),
        "the point under the pointer stays: {zoomed:?}"
    );

    // Dragged to the left, it shows more of its right.
    let to = pointer - point(px(100.), px(0.));
    cx.simulate_mouse_down(pointer, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    let dragged = view(&diagram, cx);
    assert!(close(dragged.at.x, zoomed.at.x - 100.), "{dragged:?}");

    // A trackpad's scroll pans too.
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: ScrollDelta::Pixels(point(px(-30.), px(0.))),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    let scrolled = view(&diagram, cx);
    assert!(close(scrolled.at.x, dragged.at.x - 30.), "{scrolled:?}");
    assert!(close(scrolled.zoom, dragged.zoom));
}

#[gpui::test]
fn a_fitted_diagram_stays_fitted_as_the_tab_is_resized(cx: &mut TestAppContext) {
    let (diagram, cx) = open(cx, WIDE);
    let before = view(&diagram, cx);
    cx.simulate_resize(size(px(600.), px(400.)));
    cx.run_until_parked();
    let after = view(&diagram, cx);
    assert!(after.zoom < before.zoom, "{before:?} then {after:?}");
    let own = diagram.read_with(cx, |diagram, _| diagram.size()).unwrap();
    let width = f32::from(picture(cx).size.width);
    assert!(close(after.zoom * own.width, width - 2. * MARGIN));
}

#[gpui::test]
fn what_is_no_diagram_says_it_cannot_be_drawn(cx: &mut TestAppContext) {
    let (diagram, cx) = open(cx, "not a diagram at all");
    assert!(diagram.read_with(cx, |diagram, _| diagram.view()).is_none());
    assert!(cx.debug_bounds("diagram-error").is_some());
    assert!(cx.debug_bounds("diagram-picture").is_none());
}
