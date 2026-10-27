//! A human reading a file of code on a tab of the stage, at the lines the
//! agent shows it at.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use cox::code::Code;
use cox_core::tabs::Lines;
use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, ScrollDelta, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, point, px,
};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("cox-code-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// A file of `count` numbered lines.
fn numbered(count: usize) -> String {
    let mut text = String::new();
    for line in 1..=count {
        writeln!(text, "let line_{line} = {line};").unwrap();
    }
    text
}

/// Opens a tab showing `path`, which says `text`, at `lines`.
fn open<'a>(
    cx: &'a mut TestAppContext,
    path: PathBuf,
    text: &str,
    lines: Option<Lines>,
) -> (Entity<Code>, &'a mut VisualTestContext) {
    cx.update(cox::code::bind_keys);
    let text = text.to_owned();
    let (code, cx) = cx.add_window_view(|_, cx| Code::new(path, &text, lines, cx));
    cx.run_until_parked();
    (code, cx)
}

/// Lets the tab look at its file.
fn look(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
}

fn in_sight(code: &Entity<Code>, cx: &mut VisualTestContext) -> std::ops::Range<usize> {
    code.read_with(cx, |code, _| code.in_sight())
}

#[gpui::test]
fn opens_at_the_lines_it_is_shown_at(cx: &mut TestAppContext) {
    let directory = scratch("lines");
    let path = directory.join("long.rs");
    let text = numbered(5_000);
    std::fs::write(&path, &text).unwrap();
    let lines = Lines {
        first: 4_000,
        last: 4_002,
    };
    let (code, cx) = open(cx, path, &text, Some(lines));

    let sight = in_sight(&code, cx);
    assert!(
        sight.contains(&3_999) && sight.contains(&4_001),
        "{sight:?}"
    );
    // Only the lines in sight are drawn.
    assert!(sight.len() < 200, "{sight:?}");

    // Shown again at other lines, it goes there.
    let lines = Lines {
        first: 10,
        last: 10,
    };
    code.update(cx, |code, cx| code.show(&text, Some(lines), cx));
    cx.run_until_parked();
    assert!(in_sight(&code, cx).contains(&9));
    assert_eq!(code.read_with(cx, |code, _| code.marked()), Some(lines));
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn scrolls_with_the_wheel_within_the_file(cx: &mut TestAppContext) {
    let directory = scratch("scroll");
    let path = directory.join("long.rs");
    let text = numbered(1_000);
    std::fs::write(&path, &text).unwrap();
    let (code, cx) = open(cx, path, &text, None);
    assert_eq!(in_sight(&code, cx).start, 0);

    let bounds = cx.debug_bounds("code").expect("the code is drawn");
    let wheel = |cx: &mut VisualTestContext, lines: f32| {
        cx.simulate_event(ScrollWheelEvent {
            position: bounds.center(),
            delta: ScrollDelta::Lines(point(0., lines)),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        cx.run_until_parked();
    };
    wheel(cx, -10.);
    assert!(in_sight(&code, cx).start > 0);
    // Not above the first line, nor past the last.
    wheel(cx, 100.);
    assert_eq!(in_sight(&code, cx).start, 0);
    wheel(cx, -100_000.);
    // With the empty line after the last line break.
    assert_eq!(in_sight(&code, cx).end, 1_001);
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn follows_the_file_as_it_changes(cx: &mut TestAppContext) {
    let directory = scratch("follows");
    let path = directory.join("main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let (code, cx) = open(cx, path.clone(), "fn main() {}\n", None);
    assert_eq!(code.read_with(cx, |code, _| code.title()), "main.rs");

    std::fs::write(&path, "fn main() {\n    run();\n}\n").unwrap();
    look(cx);
    assert_eq!(
        code.read_with(cx, |code, _| code.text()),
        "fn main() {\n    run();\n}\n"
    );

    // Gone, the file leaves the text as it was.
    std::fs::remove_file(&path).unwrap();
    look(cx);
    assert_eq!(
        code.read_with(cx, |code, _| code.text()),
        "fn main() {\n    run();\n}\n"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn selects_lines_to_copy(cx: &mut TestAppContext) {
    let directory = scratch("copy");
    let path = directory.join("main.rs");
    let text = "fn main() {\nrun_all();\n}\n";
    std::fs::write(&path, text).unwrap();
    let (code, cx) = open(cx, path, text, None);

    // A drag from the first line to the end of the last selects across.
    let bounds = cx.debug_bounds("code").expect("the code is drawn");
    let first = bounds.origin + point(px(1.), px(12.));
    let last = bounds.origin + point(bounds.size.width - px(1.), px(48.));
    cx.simulate_mouse_down(first, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(last, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(last, MouseButton::Left, Modifiers::none());
    assert_eq!(
        code.read_with(cx, |code, _| code.selected_text()),
        text.trim_end()
    );

    // Two clicks select a word: here, the first of the second line, as
    // the line numbers are nearer the left.
    let word = bounds.origin + point(px(1.), px(30.));
    for click_count in 1..=2 {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: word,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: word,
            modifiers: Modifiers::none(),
            click_count,
        });
    }
    assert_eq!(
        code.read_with(cx, |code, _| code.selected_text()),
        "run_all"
    );

    cx.simulate_keystrokes("secondary-c");
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("run_all")
    );
    std::fs::remove_dir_all(directory).unwrap();
}
