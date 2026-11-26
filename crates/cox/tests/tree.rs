//! A human finding a file of the project on the tree of its files, to
//! read it, and the tree marking the file of the tab shown.

mod git;
use coxd::testing::scratch;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use cox::stage::tree::{Chosen, FileAt, Root, Tree, Wrote};
use cox_core::tabs::{KeptTab, OnBranch};
use cox_git::changes::How;
use git::git;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};

/// A folder of its own for a test, with the files `paths`, empty, by
/// its real path, as the tree names its files: macOS's temporary folder
/// is under a link.
fn scratch(name: &str, paths: &[&str]) -> PathBuf {
    let directory = scratch::made(&format!("tree-{name}"));
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
}

#[gpui::test]
fn marks_the_file_shown_with_the_folders_it_is_in_open(cx: &mut TestAppContext) {
    let root = scratch("reveals", &["src/code/tree.rs", "src/main.rs", "README.md"]);
    let root = std::fs::canonicalize(root).unwrap();
    let (tree, _, cx) = open(cx, &root, Vec::new());
    assert_eq!(shown(&tree, &root, cx), ["src", "README.md"]);

    let file = root.join("src/code/tree.rs");
    tree.update(cx, |tree, cx| tree.reveal(Some(on_disk(&file)), cx));
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
        tree.read_with(cx, |tree, _| tree.current().map(|file| file.path.clone())),
        Some(file)
    );

    // No file, as of a tab of a file not chosen yet: the folders stay
    // open.
    tree.update(cx, |tree, cx| tree.reveal(None, cx));
    cx.run_until_parked();
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.current().map(|file| file.path.clone())),
        None
    );
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.open()),
        [root.join("src"), root.join("src/code")]
    );
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
}

#[gpui::test]
fn marks_the_files_the_agents_wrote_until_they_are_committed(cx: &mut TestAppContext) {
    let root = scratch("writers", &["src/main.rs", "README.md"]);
    git(&root, &["init", "--quiet"]);
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "Files"]);
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();

    let (tree, _, cx) = open(cx, &root, vec![root.join("src")]);
    let main = root.join("src/main.rs").display().to_string();
    let readme = root.join("README.md").display().to_string();
    let wrote: Wrote = Rc::new(move |_| {
        vec![
            (1, vec![main.clone(), readme.clone()]),
            (2, vec![main.clone()]),
        ]
    });
    tree.update(cx, |tree, _| tree.set_wrote(wrote));
    look(cx);
    let writers = |cx: &mut VisualTestContext| {
        tree.read_with(cx, |tree, _| {
            ["src/main.rs", "src/lib.rs", "README.md"]
                .map(|path| tree.writers(&root.join(path)).to_vec())
        })
    };
    // Not `README.md`, as it was at the last commit, nor `lib.rs`, which
    // the human wrote.
    assert_eq!(writers(cx), [vec![1, 2], vec![], vec![]]);

    git(&root, &["commit", "--quiet", "--all", "-m", "Main"]);
    look(cx);
    assert_eq!(writers(cx), [Vec::<u64>::new(), vec![], vec![]]);
}

fn press(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx.debug_bounds(selector).expect("it is drawn");
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
}

/// A Git repository at `folder`, with `paths` committed, empty.
fn repository(folder: &Path, paths: &[&str]) {
    for path in paths {
        let path = folder.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    git(folder, &["init", "--quiet"]);
    git(folder, &["add", "--all"]);
    git(folder, &["commit", "--quiet", "-m", "Files"]);
}

/// A project whose directories `app` and `notes` are repositories, with
/// a wiki beside them, and `app` with a worktree where Cox makes them,
/// on the branch `search`, with a file of its own, `s.txt`; and the
/// folder of that worktree.
fn project(name: &str) -> (PathBuf, PathBuf) {
    let project = scratch(name, &["wiki/index.md"]);
    let app = project.join("directories/app");
    repository(&app, &["src/main.rs", "README.md"]);
    repository(&project.join("directories/notes"), &["today.md"]);
    let search = project.join("worktrees/app/search");
    git(
        &app,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "search",
            &search.to_string_lossy(),
        ],
    );
    std::fs::write(search.join("s.txt"), "").unwrap();
    (project, search)
}

/// The folder the tree shows.
fn root(tree: &Entity<Tree>, cx: &mut VisualTestContext) -> PathBuf {
    tree.read_with(cx, |tree, _| {
        assert_eq!(tree.root().branch(), None, "a folder on disk");
        tree.root().top().to_owned()
    })
}

/// The file at `path` on disk, as a tab of it shows it.
fn on_disk(path: &Path) -> FileAt {
    FileAt {
        path: path.to_owned(),
        branch: None,
    }
}

#[gpui::test]
fn shows_the_first_directory_of_a_project_until_the_human_chooses_another(cx: &mut TestAppContext) {
    let (project, _) = project("directories");
    let (tree, _, cx) = open(cx, &project, Vec::new());
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.directories()),
        ["app", "notes"]
    );
    let app = project.join("directories/app");
    assert_eq!(root(&tree, cx), app);
    assert_eq!(shown(&tree, &app, cx), ["src", "README.md"]);

    press("files-directory-1", cx);
    let notes = project.join("directories/notes");
    assert_eq!(root(&tree, cx), notes);
    assert_eq!(shown(&tree, &notes, cx), ["today.md"]);
}

#[gpui::test]
fn keeps_quiet_with_one_directory_of_one_checkout(cx: &mut TestAppContext) {
    let project = scratch("one-directory", &["wiki/index.md"]);
    let app = project.join("directories/app");
    repository(&app, &["README.md"]);
    let (tree, _, cx) = open(cx, &project, Vec::new());
    assert_eq!(root(&tree, cx), app);
    // The directory, but no menu of checkouts, with one to choose.
    assert!(cx.debug_bounds("files-directory-0").is_some());
    assert!(cx.debug_bounds("files-checkout").is_none());
}

#[gpui::test]
fn shows_a_worktree_of_a_directory_as_the_human_chooses_it(cx: &mut TestAppContext) {
    let (project, search) = project("worktree");
    let (tree, _, cx) = open(cx, &project, Vec::new());
    assert_eq!(
        tree.read_with(cx, |tree, _| tree
            .checkouts()
            .iter()
            .map(coxd::repositories::Checkout::label)
            .collect::<Vec<_>>()),
        ["main", "search"]
    );

    press("files-checkout", cx);
    assert!(cx.debug_bounds("files-checkouts").is_some());
    press("files-checkout-choice-1", cx);
    assert_eq!(root(&tree, cx), search);
    assert_eq!(shown(&tree, &search, cx), ["src", "README.md", "s.txt"]);

    // Its own checkout again, by the menu.
    press("files-checkout", cx);
    press("files-checkout-choice-0", cx);
    assert_eq!(root(&tree, cx), project.join("directories/app"));
}

#[gpui::test]
fn shows_a_folder_with_no_directories_as_it_is(cx: &mut TestAppContext) {
    // As a worktree a chat works in, or a sketch.
    let (_, search) = project("no-directories");
    let (tree, _, cx) = open(cx, &search, Vec::new());
    look(cx);
    assert_eq!(root(&tree, cx), search);
    assert_eq!(shown(&tree, &search, cx), ["src", "README.md", "s.txt"]);
    assert!(cx.debug_bounds("files-chooser").is_none());
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.directories()),
        Vec::<String>::new()
    );
}

#[gpui::test]
fn keeps_what_the_human_chose_to_show(cx: &mut TestAppContext) {
    let (project, search) = project("keeps");
    let (tree, _, cx) = open(cx, &project, Vec::new());
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.kept()),
        KeptTab::Files {
            open: Vec::new(),
            checkouts: Vec::new(),
            branches: Vec::new(),
            shown: None,
        }
    );
    press("files-checkout", cx);
    press("files-checkout-choice-1", cx);
    let kept = tree.read_with(cx, |tree, _| tree.kept());
    assert_eq!(
        kept,
        KeptTab::Files {
            open: Vec::new(),
            checkouts: vec![("app".to_owned(), search.clone())],
            branches: Vec::new(),
            shown: Some("app".to_owned()),
        }
    );

    // A stage that opens again shows it again.
    let KeptTab::Files {
        checkouts,
        branches,
        shown,
        ..
    } = kept
    else {
        unreachable!()
    };
    let folder = project.clone();
    let (again, cx) = cx.add_window_view(|_, cx| Tree::new(folder, Vec::new(), cx));
    again.update(cx, |tree, cx| tree.choosing(checkouts, branches, shown, cx));
    cx.run_until_parked();
    assert_eq!(root(&again, cx), search);
}

#[gpui::test]
fn follows_the_file_shown_to_the_checkout_it_is_in(cx: &mut TestAppContext) {
    let (project, search) = project("follows-file");
    let elsewhere = scratch("follows-file-elsewhere", &["a.txt"]);
    let (tree, _, cx) = open(cx, &project, Vec::new());
    let marked = |tree: &Entity<Tree>, cx: &mut VisualTestContext| {
        tree.read_with(cx, |tree, _| tree.current().map(|file| file.path.clone()))
    };

    let file = search.join("s.txt");
    tree.update(cx, |tree, cx| tree.reveal(Some(on_disk(&file)), cx));
    cx.run_until_parked();
    assert_eq!(root(&tree, cx), search);
    assert_eq!(marked(&tree, cx), Some(file));

    // A file of the project's in none of the checkouts, as of its wiki, or
    // of no project's: the tree stays, marking none of its rows.
    for file in [project.join("wiki/index.md"), elsewhere.join("a.txt")] {
        tree.update(cx, |tree, cx| tree.reveal(Some(on_disk(&file)), cx));
        cx.run_until_parked();
        assert_eq!(root(&tree, cx), search);
        assert!(!tree.read_with(cx, |tree, _| tree.shown()).contains(&file));
    }
}

/// A project whose directory `app` is a repository on `main`, with a
/// branch `side` no checkout has, whose files are not on disk: `NOTES.md`
/// and `src/deep/lib.rs`, `deep` all there is in `src`.
fn project_with_a_branch(name: &str) -> (PathBuf, PathBuf) {
    let project = scratch(name, &[]);
    let app = project.join("directories/app");
    repository(&app, &["README.md"]);
    git(&app, &["checkout", "--quiet", "-b", "side"]);
    std::fs::create_dir_all(app.join("src/deep")).unwrap();
    std::fs::write(app.join("src/deep/lib.rs"), "pub fn side() {}\n").unwrap();
    std::fs::write(app.join("NOTES.md"), "# Side\n").unwrap();
    git(&app, &["add", "--all"]);
    git(&app, &["commit", "--quiet", "-m", "Side"]);
    git(&app, &["checkout", "--quiet", "main"]);
    assert!(!app.join("src").exists());
    (project, app)
}

/// The branch `branch` of the repository at `app`.
fn side(app: &Path, branch: &str) -> OnBranch {
    OnBranch {
        repository: app.to_owned(),
        branch: branch.to_owned(),
    }
}

#[gpui::test]
fn shows_the_files_of_a_branch_no_checkout_has_from_git(cx: &mut TestAppContext) {
    let (project, app) = project_with_a_branch("branch");
    let (tree, chosen, cx) = open(cx, &project, Vec::new());
    assert_eq!(
        tree.read_with(cx, |tree, _| tree
            .checkouts()
            .iter()
            .map(|checkout| (checkout.label(), checkout.branch().is_some()))
            .collect::<Vec<_>>()),
        [("main".to_owned(), false), ("side".to_owned(), true)]
    );

    press("files-checkout", cx);
    press("files-checkout-choice-1", cx);
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.root().clone()),
        Root::Branch(side(&app, "side"))
    );
    // The branch's files, from Git.
    assert_eq!(shown(&tree, &app, cx), ["src", "NOTES.md", "README.md"]);
    // `src` opens with `deep`, all there is in it, as one row, as on disk.
    click(1, cx);
    assert_eq!(
        shown(&tree, &app, cx),
        ["src/deep", "src/deep/lib.rs", "NOTES.md", "README.md"]
    );
    // A file of the branch is chosen as the branch's, not as one on disk.
    let branches = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&branches);
    cx.update(|_, cx| {
        cx.subscribe(&tree, move |_, chosen: &Chosen, _| {
            heard.borrow_mut().push(chosen.branch.clone());
        })
        .detach();
    });
    click(2, cx);
    assert_eq!(*chosen.borrow(), [(app.join("src/deep/lib.rs"), false)]);
    assert_eq!(*branches.borrow(), [Some(side(&app, "side"))]);
    // What changed on disk is not marked on the branch's files.
    std::fs::write(app.join("README.md"), "changed on disk\n").unwrap();
    look(cx);
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.how(&app.join("README.md"))),
        None
    );
    // Nothing of it was written to disk.
    assert!(!app.join("src").exists());
}

#[gpui::test]
fn lists_a_branch_again_as_it_moves(cx: &mut TestAppContext) {
    let (project, app) = project_with_a_branch("branch-moves");
    let (tree, _, cx) = open(cx, &project, Vec::new());
    tree.update(cx, |tree, cx| {
        tree.show_branch("app".to_owned(), "side".to_owned(), cx);
    });
    cx.run_until_parked();
    assert_eq!(shown(&tree, &app, cx), ["src", "NOTES.md", "README.md"]);

    // A commit to the branch from elsewhere, as an agent's in a checkout
    // it then let go of.
    git(&app, &["checkout", "--quiet", "side"]);
    std::fs::write(app.join("MORE.md"), "more\n").unwrap();
    git(&app, &["add", "--all"]);
    git(&app, &["commit", "--quiet", "-m", "More"]);
    git(&app, &["checkout", "--quiet", "main"]);
    look(cx);
    assert_eq!(
        shown(&tree, &app, cx),
        ["src", "MORE.md", "NOTES.md", "README.md"]
    );
}

#[gpui::test]
fn shows_where_a_branch_is_once_it_is_gone_or_checked_out(cx: &mut TestAppContext) {
    let (project, app) = project_with_a_branch("branch-falls-back");
    let (tree, _, cx) = open(cx, &project, Vec::new());
    tree.update(cx, |tree, cx| {
        tree.show_branch("app".to_owned(), "side".to_owned(), cx);
    });
    cx.run_until_parked();

    // A checkout has it now: the tree shows that checkout, once it lists
    // the checkouts again.
    let worktree = project.join("worktrees/app/side");
    git(
        &app,
        &[
            "worktree",
            "add",
            "--quiet",
            &worktree.to_string_lossy(),
            "side",
        ],
    );
    for _ in 0..10 {
        look(cx);
    }
    assert_eq!(root(&tree, cx), worktree);

    // Gone: the directory's own checkout.
    git(&app, &["branch", "--quiet", "other"]);
    let (tree, _, cx) = open(cx, &project, Vec::new());
    tree.update(cx, |tree, cx| {
        tree.show_branch("app".to_owned(), "other".to_owned(), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.root().clone()),
        Root::Branch(side(&app, "other"))
    );
    git(&app, &["branch", "--quiet", "-D", "other"]);
    look(cx);
    assert_eq!(root(&tree, cx), app);
}

#[gpui::test]
fn says_a_submodule_of_a_branch_cannot_be_listed(cx: &mut TestAppContext) {
    let (project, app) = project_with_a_branch("branch-submodule");
    git(&app, &["checkout", "--quiet", "side"]);
    let commit = git(&app, &["rev-parse", "HEAD"]).trim().to_owned();
    git(
        &app,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},vendor"),
        ],
    );
    git(&app, &["commit", "--quiet", "-m", "Vendor"]);
    git(&app, &["checkout", "--quiet", "main"]);
    let (tree, _, cx) = open(cx, &project, Vec::new());
    tree.update(cx, |tree, cx| {
        tree.show_branch("app".to_owned(), "side".to_owned(), cx);
    });
    cx.run_until_parked();
    assert_eq!(
        shown(&tree, &app, cx),
        ["src", "vendor", "NOTES.md", "README.md"]
    );

    // It opens as a folder whose files it cannot read, as the tree says.
    click(2, cx);
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.open()),
        [app.join("vendor")]
    );
    assert_eq!(
        shown(&tree, &app, cx),
        ["src", "vendor", "NOTES.md", "README.md"]
    );
}

#[gpui::test]
fn keeps_the_branch_the_human_chose(cx: &mut TestAppContext) {
    let (project, app) = project_with_a_branch("branch-keeps");
    let (tree, _, cx) = open(cx, &project, Vec::new());
    tree.update(cx, |tree, cx| {
        tree.show_branch("app".to_owned(), "side".to_owned(), cx);
    });
    let kept = tree.read_with(cx, |tree, _| tree.kept());
    assert_eq!(
        kept,
        KeptTab::Files {
            open: Vec::new(),
            checkouts: Vec::new(),
            branches: vec![("app".to_owned(), "side".to_owned())],
            shown: Some("app".to_owned()),
        }
    );

    let folder = project.clone();
    let (again, cx) = cx.add_window_view(|_, cx| Tree::new(folder, Vec::new(), cx));
    again.update(cx, |tree, cx| {
        tree.choosing(
            Vec::new(),
            vec![("app".to_owned(), "side".to_owned())],
            Some("app".to_owned()),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(
        again.read_with(cx, |tree, _| tree.root().clone()),
        Root::Branch(side(&app, "side"))
    );
    assert_eq!(shown(&again, &app, cx), ["src", "NOTES.md", "README.md"]);
}

#[gpui::test]
fn follows_a_file_of_a_branch_to_the_branch(cx: &mut TestAppContext) {
    let (project, app) = project_with_a_branch("branch-reveals");
    let (tree, _, cx) = open(cx, &project, Vec::new());
    let marked = |tree: &Entity<Tree>, cx: &mut VisualTestContext| {
        tree.read_with(cx, |tree, _| tree.current().cloned())
    };

    let file = FileAt {
        path: app.join("src/deep/lib.rs"),
        branch: Some(side(&app, "side")),
    };
    tree.update(cx, |tree, cx| tree.reveal(Some(file.clone()), cx));
    cx.run_until_parked();
    assert_eq!(
        tree.read_with(cx, |tree, _| tree.root().clone()),
        Root::Branch(side(&app, "side"))
    );
    assert_eq!(marked(&tree, cx), Some(file));
    assert_eq!(
        shown(&tree, &app, cx),
        ["src/deep", "src/deep/lib.rs", "NOTES.md", "README.md"]
    );

    // The file on disk of the same path as one of the branch's is the
    // checkout's, not the branch's.
    let readme = on_disk(&app.join("README.md"));
    tree.update(cx, |tree, cx| tree.reveal(Some(readme), cx));
    cx.run_until_parked();
    assert_eq!(root(&tree, cx), app);
}
