//! A human reading a Markdown file on a tab of the stage.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use cox::stage::document::Document;
use cox::ui::LinkClicked;
use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("cox-document-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
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
    std::fs::remove_dir_all(directory).unwrap();
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
    std::fs::remove_dir_all(directory).unwrap();
}
