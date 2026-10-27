//! A human finding a file of the project on the tree of its files, to
//! open it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use cox::tree::{Chosen, Tree};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

/// A folder of its own for a test, with the files `paths`, empty.
fn scratch(name: &str, paths: &[&str]) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("cox-tree-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    for path in paths {
        let path = directory.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    directory
}

/// Opens a tab showing the files under `root`, with the folders `open`
/// open, and hears the files chosen on it.
fn open<'a>(
    cx: &'a mut TestAppContext,
    root: &Path,
    open: Vec<PathBuf>,
) -> (
    Entity<Tree>,
    Rc<RefCell<Vec<PathBuf>>>,
    &'a mut VisualTestContext,
) {
    let root = root.to_owned();
    let (tree, cx) = cx.add_window_view(|_, cx| Tree::new(root, open, cx));
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&chosen);
    cx.update(|_, cx| {
        cx.subscribe(&tree, move |_, Chosen(path), _| {
            heard.borrow_mut().push(path.clone());
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
    let bounds = cx.debug_bounds(entry(row)).expect("the row is drawn");
    cx.simulate_click(bounds.center(), Modifiers::none());
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
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.title()),
        format!("{}/", root.file_name().unwrap().to_string_lossy())
    );
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

    // `main.rs`, in `src`.
    click(2, cx);
    assert_eq!(*chosen.borrow(), [root.join("src/main.rs")]);

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
    assert_eq!(
        shown(&tree, &root, cx),
        ["src", "src/code", "src/code/tree.rs", "README.md"]
    );
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
