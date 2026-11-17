//! A human reading a Markdown file on a tab of the stage.

use coxd::testing::scratch;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use cox::stage::document::Document;
use cox::stage::tab::Shows as _;
use cox::ui::LinkClicked;
use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};
use std::fmt::Write as _;

fn scratch(name: &str) -> PathBuf {
    scratch::made(&format!("document-{name}"))
}

/// Opens a tab showing `path`, which says `text`.
fn open<'a>(
    cx: &'a mut TestAppContext,
    path: PathBuf,
    text: &str,
) -> (Entity<Document>, &'a mut VisualTestContext) {
    cx.update(cox::stage::document::bind_keys);
    let text = text.to_owned();
    let (document, cx) = cx.add_window_view(|_, cx| Document::new(path, text, cx));
    cx.run_until_parked();
    (document, cx)
}

/// Lets the tab look at its file.
fn look(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
}

#[gpui::test]
fn follows_the_file_as_it_changes(cx: &mut TestAppContext) {
    let directory = scratch("follows");
    let path = directory.join("plan.md");
    std::fs::write(&path, "# Plan\n").unwrap();
    let (document, cx) = open(cx, path.clone(), "# Plan\n");
    assert_eq!(
        document.read_with(cx, |document, _| document.title()),
        "plan.md"
    );

    std::fs::write(&path, "# Plan\n\n- read the design\n").unwrap();
    look(cx);
    assert_eq!(
        document.read_with(cx, |document, _| document.text().to_owned()),
        "# Plan\n\n- read the design\n"
    );

    // Gone, the file leaves the text as it was.
    std::fs::remove_file(&path).unwrap();
    look(cx);
    assert_eq!(
        document.read_with(cx, |document, _| document.text().to_owned()),
        "# Plan\n\n- read the design\n"
    );
}

#[gpui::test]
fn a_click_on_a_link_asks_to_open_it(cx: &mut TestAppContext) {
    let directory = scratch("link");
    let path = directory.join("links.md");
    let text = "https://example.com/\n";
    std::fs::write(&path, text).unwrap();
    let (document, cx) = open(cx, path, text);
    let clicked = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&clicked);
    cx.update(|_, cx| {
        cx.subscribe(&document, move |_, LinkClicked(url), _| {
            heard.borrow_mut().push(url.clone());
        })
        .detach();
    });

    let bounds = cx.debug_bounds("document-text").expect("the text is drawn");
    let link = point(bounds.origin.x + px(20.), bounds.origin.y + px(5.));
    cx.simulate_mouse_down(link, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(link, MouseButton::Left, Modifiers::none());

    assert_eq!(*clicked.borrow(), ["https://example.com/"]);
}

#[gpui::test]
fn a_drag_held_past_the_bottom_scrolls_and_selects_on(cx: &mut TestAppContext) {
    let directory = scratch("drag-past");
    let path = directory.join("long.md");
    let mut text = String::new();
    for paragraph in 1..=500 {
        writeln!(text, "Paragraph {paragraph}.\n").unwrap();
    }
    std::fs::write(&path, &text).unwrap();
    let (document, cx) = open(cx, path, &text);
    let selected =
        |cx: &mut VisualTestContext| document.read_with(cx, |document, _| document.selected_text());

    let bounds = cx.debug_bounds("document").expect("the document is drawn");
    let first = bounds.origin + point(px(20.), px(20.));
    let bottom = point(bounds.center().x, bounds.bottom() - px(2.));
    cx.simulate_mouse_down(first, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(bottom, MouseButton::Left, Modifiers::none());
    let in_sight = selected(cx).lines().count();
    assert!(in_sight > 0);

    let below = point(bottom.x, bounds.bottom() + px(40.));
    cx.simulate_mouse_move(below, MouseButton::Left, Modifiers::none());
    for _ in 0..30 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    cx.simulate_mouse_up(below, MouseButton::Left, Modifiers::none());

    let past = selected(cx);
    assert!(past.starts_with("Paragraph 1."), "{past}");
    assert!(
        past.lines().count() > in_sight,
        "{in_sight} lines in sight, then: {past}"
    );
}
