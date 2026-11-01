//! A human reading what changed in the project's files, to open a file
//! where it changed.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use cox::stage::changes::{Changes, Chosen};
use cox::stage::tab::Shows as _;
use cox_editor::changes::Since;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

/// A folder of its own for a test, made a Git repository with a commit of
/// `a.txt`, saying `text`.
fn repository(name: &str, text: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("cox-changes-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    std::fs::write(folder.join("a.txt"), text).unwrap();
    for args in [
        &["init", "--quiet"][..],
        &["add", "a.txt"],
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--quiet",
            "-m",
            "A",
        ],
    ] {
        let ran = Command::new("git")
            .arg("-C")
            .arg(&folder)
            .args(args)
            .output()
            .unwrap();
        assert!(ran.status.success(), "{ran:?}");
    }
    folder
}

/// The files the human chose, at the lines they chose if any.
type Heard = Rc<RefCell<Vec<(PathBuf, Option<usize>)>>>;

/// Opens a tab showing what changed in `folder`, for a chat working in
/// `cwd`, and hears the files chosen on it.
fn open<'a>(
    cx: &'a mut TestAppContext,
    folder: &Path,
    cwd: &Path,
) -> (Entity<Changes>, Heard, &'a mut VisualTestContext) {
    let (folder, cwd) = (folder.to_owned(), cwd.to_owned());
    let (changes, cx) = cx.add_window_view(|_, cx| Changes::new(folder, &cwd, cx));
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&chosen);
    cx.update(|_, cx| {
        cx.subscribe(&changes, move |_, Chosen { path, line }, _| {
            heard.borrow_mut().push((path.clone(), *line));
        })
        .detach();
    });
    cx.run_until_parked();
    (changes, chosen, cx)
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds(selector).expect("the row is drawn");
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

/// The files the tab shows as changed, and how many lines each gained
/// and lost.
fn shown(changes: &Entity<Changes>, cx: &mut VisualTestContext) -> Vec<(String, usize, usize)> {
    changes.read_with(cx, |changes, _| {
        changes.diff().map_or_else(Vec::new, |diff| {
            diff.files
                .iter()
                .map(|file| (file.path.display().to_string(), file.added, file.removed))
                .collect()
        })
    })
}

#[gpui::test]
fn shows_the_lines_that_changed_and_opens_a_file_there(cx: &mut TestAppContext) {
    let folder = repository("opens", "one\ntwo\nthree\n");
    std::fs::write(folder.join("a.txt"), "one\n2\nthree\n").unwrap();
    std::fs::write(folder.join("b.txt"), "new\n").unwrap();
    let (changes, chosen, cx) = open(cx, &folder, &folder);
    assert_eq!(
        changes.read_with(cx, |changes, _| changes.title()),
        "Changes"
    );
    assert!(changes.read_with(cx, |changes, _| matches!(
        changes.diff().map(|diff| &diff.since),
        Some(Since::Head(_))
    )));
    assert_eq!(
        shown(&changes, cx),
        [("a.txt".to_owned(), 1, 1), ("b.txt".to_owned(), 1, 0)]
    );

    // The rows: a.txt, its stretch, "one", "two", "2", "three", then b.txt.
    click("changed-line-4", cx);
    click("changed-file-6", cx);
    click("changed-line-3", cx);
    assert_eq!(
        *chosen.borrow(),
        [
            (folder.join("a.txt"), Some(2)),
            (folder.join("b.txt"), None),
            (folder.join("a.txt"), Some(2)),
        ]
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[gpui::test]
fn follows_what_changes_as_the_agent_works(cx: &mut TestAppContext) {
    let folder = repository("follows", "one\n");
    let (changes, _, cx) = open(cx, &folder, Path::new("/elsewhere"));
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        changes.read_with(cx, |changes, _| changes.title()),
        format!("Changes in {name}")
    );
    assert_eq!(shown(&changes, cx), []);

    std::fs::write(folder.join("a.txt"), "one\ntwo\n").unwrap();
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(shown(&changes, cx), [("a.txt".to_owned(), 1, 0)]);
    std::fs::remove_dir_all(folder).unwrap();
}
