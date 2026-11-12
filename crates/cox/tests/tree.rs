//! A human finding a file of the project on the tree of its files, to
//! read it, and the tree marking the file of the tab shown.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use cox::stage::tree::{Chosen, Tree};
use cox_editor::changes::How;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

/// A folder of its own for a test, with the files `paths`, empty, by
/// its real path, as the tree names its files: macOS's temporary folder
/// is under a link.
fn scratch(name: &str, paths: &[&str]) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("cox-tree-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    for path in paths {
        let path = directory.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    directory.canonicalize().unwrap()
}

/// The files chosen on a tree, each with whether it was chosen for a tab
/// of its own.
type Heard = Rc<RefCell<Vec<(PathBuf, bool)>>>;

/// Opens the tree of the files under `root`, with the folders `open`
/// open, and hears the files chosen on it, and whether each was chosen
/// for a tab of its own.
fn open<'a>(
    cx: &'a mut TestAppContext,
    root: &Path,
    open: Vec<PathBuf>,
) -> (Entity<Tree>, Heard, &'a mut VisualTestContext) {
    let root = root.to_owned();
    let (tree, cx) = cx.add_window_view(|_, cx| Tree::new(root, open, cx));
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&chosen);
    cx.update(|_, cx| {
        cx.subscribe(&tree, move |_, chosen: &Chosen, _| {
            heard.borrow_mut().push((chosen.path.clone(), chosen.apart));
        })
        .detach();
    });
    cx.run_until_parked();
    (tree, chosen, cx)
}

/// What the row `number` of the tab is drawn as, for a test to find.
fn entry(number: usize) -> &'static str {
    // A test draws few rows, so it keeps few names.
    format!("entry-{number}").leak()
}

/// What the tab shows, as paths from `root`.
fn shown(tree: &Entity<Tree>, root: &Path, cx: &mut VisualTestContext) -> Vec<String> {
    tree.read_with(cx, |tree, _| tree.shown())
        .iter()
        .map(|path| path.strip_prefix(root).unwrap().display().to_string())
        .collect()
}

fn click(row: usize, cx: &mut VisualTestContext) {
    click_with(row, Modifiers::none(), cx);
}

fn click_with(row: usize, modifiers: Modifiers, cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds(entry(row)).expect("the row is drawn");
    cx.simulate_click(bounds.center(), modifiers);
    cx.run_until_parked();
}

/// Lets the tab look at its folders.
fn look(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
}

#[gpui::test]
fn opens_folders_and_chooses_files(cx: &mut TestAppContext) {
    let root = scratch(
        "opens",
        &[
            ".gitignore",
            "Cargo.toml",
            "src/main.rs",
            "target/debug/cox",
        ],
    );
    std::fs::write(root.join(".gitignore"), "/target\n").unwrap();
    let (tree, chosen, cx) = open(cx, &root, Vec::new());
    // Not `target`, which Git ignores.
    assert_eq!(shown(&tree, &root, cx), ["src", ".gitignore", "Cargo.toml"]);

    click(1, cx);
    assert_eq!(
        shown(&tree, &root, cx),
        ["src", "src/main.rs", ".gitignore", "Cargo.toml"]
    );
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.open()),
        [root.join("src")]
    );

    // `main.rs`, in `src`: on the tab shown, and then on one of its own.
    click(2, cx);
    click_with(2, Modifiers::secondary_key(), cx);
    assert_eq!(
        *chosen.borrow(),
        [
            (root.join("src/main.rs"), false),
            (root.join("src/main.rs"), true)
        ]
    );

    click(1, cx);
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.open()),
        Vec::<PathBuf>::new()
    );
    assert_eq!(shown(&tree, &root, cx), ["src", ".gitignore", "Cargo.toml"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn opens_with_the_folders_it_had_open(cx: &mut TestAppContext) {
    let root = scratch("kept", &["src/code/tree.rs", "README.md"]);
    let (tree, _, cx) = open(cx, &root, vec![root.join("src"), root.join("src/code")]);
    // `src / code` is one row, as `code` is all there is in `src`.
    assert_eq!(
        shown(&tree, &root, cx),
        ["src/code", "src/code/tree.rs", "README.md"]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn opens_a_folder_with_the_folders_that_are_all_there_is_in_it(cx: &mut TestAppContext) {
    let root = scratch(
        "compact",
        &[
            "docs/design/files/tree.md",
            "docs/design/tabs.md",
            "README.md",
        ],
    );
    let (tree, _, cx) = open(cx, &root, Vec::new());
    assert_eq!(shown(&tree, &root, cx), ["docs", "README.md"]);

    // `docs` opens with `design`, which is all there is in it, as one
    // row; `files` is not all there is in `design`.
    click(1, cx);
    assert_eq!(
        shown(&tree, &root, cx),
        [
            "docs/design",
            "docs/design/files",
            "docs/design/tabs.md",
            "README.md"
        ]
    );

    // The row closes `design`, and `docs` stays open with it in it.
    click(1, cx);
    assert_eq!(shown(&tree, &root, cx), ["docs/design", "README.md"]);
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.open()),
        [root.join("docs")]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn keeps_the_folders_of_the_rows_at_the_top_there(cx: &mut TestAppContext) {
    let files: Vec<String> = (0..60)
        .map(|number| format!("src/file{number:02}.rs"))
        .collect();
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let root = scratch("pinned", &[files.as_slice(), &["README.md"]].concat());
    let (_, chosen, cx) = open(cx, &root, vec![root.join("src")]);
    // Nothing is above the first rows to keep at the top.
    assert!(cx.debug_bounds("pinned-1").is_none());

    let tree = cx.debug_bounds("tree").expect("the tree is drawn");
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: tree.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-300.))),
        ..Default::default()
    });
    cx.run_until_parked();
    let pinned = cx.debug_bounds("pinned-1").expect("`src` stays at the top");
    // Where the first row is, before the tree scrolls.
    assert_eq!(pinned.top(), tree.top() + gpui::px(4.));

    // Choosing it scrolls back to it, at the top.
    cx.simulate_click(pinned.center(), Modifiers::none());
    cx.run_until_parked();
    let row = cx.debug_bounds("entry-1").expect("`src` is drawn");
    assert_eq!(row.top(), tree.top() + gpui::px(4.));
    // Not the file that was under it.
    assert!(chosen.borrow().is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn follows_the_files_as_they_come_and_go(cx: &mut TestAppContext) {
    let root = scratch("follows", &["src/main.rs"]);
    let (tree, _, cx) = open(cx, &root, vec![root.join("src")]);
    assert_eq!(shown(&tree, &root, cx), ["src", "src/main.rs"]);

    std::fs::write(root.join("src/lib.rs"), "").unwrap();
    look(cx);
    assert_eq!(
        shown(&tree, &root, cx),
        ["src", "src/lib.rs", "src/main.rs"]
    );

    std::fs::remove_file(root.join("src/main.rs")).unwrap();
    look(cx);
    assert_eq!(shown(&tree, &root, cx), ["src", "src/lib.rs"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[gpui::test]
fn marks_the_file_shown_with_the_folders_it_is_in_open(cx: &mut TestAppContext) {
    let root = scratch("reveals", &["src/code/tree.rs", "src/main.rs", "README.md"]);
    let root = std::fs::canonicalize(root).unwrap();
    let (tree, _, cx) = open(cx, &root, Vec::new());
    assert_eq!(shown(&tree, &root, cx), ["src", "README.md"]);

    let file = root.join("src/code/tree.rs");
    tree.update(cx, |tree, cx| tree.reveal(Some(file.clone()), cx));
    cx.run_until_parked();
    assert_eq!(
        shown(&tree, &root, cx),
        [
            "src",
            "src/code",
            "src/code/tree.rs",
            "src/main.rs",
            "README.md"
        ]
    );
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.current().map(Path::to_owned)),
        Some(file)
    );

    // No file, as of a tab of a file not chosen yet: the folders stay
    // open.
    tree.update(cx, |tree, cx| tree.reveal(None, cx));
    cx.run_until_parked();
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.current().map(Path::to_owned)),
        None
    );
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.open()),
        [root.join("src"), root.join("src/code")]
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// Runs Git in `folder`, as a test's repository.
fn git(folder: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());
}

#[gpui::test]
fn says_how_each_file_changed(cx: &mut TestAppContext) {
    let root = scratch("hows", &["src/main.rs", "README.md"]);
    git(&root, &["init", "--quiet"]);
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "Files"]);
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();

    let (tree, _, cx) = open(cx, &root, vec![root.join("src")]);
    let hows = |cx: &mut VisualTestContext| {
        tree.read_with(cx, |tree, _| {
            ["src/lib.rs", "src/main.rs", "README.md"].map(|path| tree.how(&root.join(path)))
        })
    };
    assert_eq!(hows(cx), [Some(How::Added), Some(How::Changed), None]);

    // As the agent writes the file over as it was.
    std::fs::write(root.join("src/main.rs"), "").unwrap();
    look(cx);
    assert_eq!(hows(cx), [Some(How::Added), None, None]);
    std::fs::remove_dir_all(root).unwrap();
}
