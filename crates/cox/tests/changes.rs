//! A human reading what changed in the project's files, to open a file
//! where it changed.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use cox::stage::changes::{Changes, Chosen, Pick, Repository, ShowTurn};
use cox::stage::tab::Shows as _;
use cox_editor::changes::{Diff, Since};
use cox_editor::commits::{self, Turn};
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

/// The files the tab shows as changed, of the repository it shows, each
/// after the name of its directory if it has one, and how many lines
/// each gained and lost.
fn shown(changes: &Entity<Changes>, cx: &mut VisualTestContext) -> Vec<(String, usize, usize)> {
    changes.read_with(cx, |changes, _| {
        let Some(Repository {
            name,
            read: Ok(diff),
        }) = changes.shown()
        else {
            return Vec::new();
        };
        diff.files
            .iter()
            .map(|file| {
                let path = file.path.display().to_string();
                let path = match name {
                    Some(name) => format!("{name}/{path}"),
                    None => path,
                };
                (path, file.added, file.removed)
            })
            .collect()
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
        changes.repositories(),
        [Repository {
            name: None,
            read: Ok(Diff {
                since: Since::Head(_),
                ..
            }),
        }]
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

#[gpui::test]
fn shows_what_changed_in_each_repository_among_a_projects_directories(cx: &mut TestAppContext) {
    let project = std::env::temp_dir().join(format!("cox-changes-project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    let directories = project.join("directories");
    std::fs::create_dir_all(directories.join("notes")).unwrap();
    let project = project.canonicalize().unwrap();
    // The project's own folder is no repository, nor is its directory
    // `notes`; `app` and `lib` are.
    for name in ["app", "lib"] {
        let folder = repository(&format!("project-{name}"), "one\n");
        std::fs::rename(&folder, project.join("directories").join(name)).unwrap();
    }
    let app = project.join("directories/app");
    std::fs::write(app.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(project.join("directories/notes/n.txt"), "note\n").unwrap();

    let (changes, chosen, cx) = open(cx, &project, &project);
    assert_eq!(
        changes.read_with(cx, |changes, _| changes
            .repositories()
            .iter()
            .map(|repository| repository.name.clone())
            .collect::<Vec<_>>()),
        [Some("app".to_owned()), Some("lib".to_owned())]
    );
    // The first that changed anything is shown, with each to choose.
    assert_eq!(shown(&changes, cx), [("app/a.txt".to_owned(), 1, 0)]);
    assert_eq!(cox::stage::changes::count(&project), Ok(1));
    assert!(cx.debug_bounds("changed-repository-0").is_some());
    assert!(cx.debug_bounds("changed-repository-1").is_some());

    // The rows: a.txt, its stretch, "one", "two".
    click("changed-line-3", cx);
    assert_eq!(*chosen.borrow(), [(app.join("a.txt"), Some(2))]);

    // What the agent changes in another repository is counted, and shows
    // as the human chooses it.
    std::fs::write(project.join("directories/lib/b.txt"), "new\n").unwrap();
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(shown(&changes, cx), [("app/a.txt".to_owned(), 1, 0)]);
    assert_eq!(cox::stage::changes::count(&project), Ok(2));
    click("changed-repository-1", cx);
    assert_eq!(shown(&changes, cx), [("lib/b.txt".to_owned(), 1, 0)]);
    click("changed-file-0", cx);
    assert_eq!(
        chosen.borrow().last(),
        Some(&(project.join("directories/lib/b.txt"), None))
    );

    // It stays on the one chosen as nothing changes in it any more.
    std::fs::remove_file(project.join("directories/lib/b.txt")).unwrap();
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(shown(&changes, cx), []);
    assert!(changes.read_with(cx, |changes, _| {
        changes
            .shown()
            .is_some_and(|shown| shown.name.as_deref() == Some("lib"))
    }));
    std::fs::remove_dir_all(project).unwrap();
}

/// Commits all that changed in `folder`, as `message` says, and answers
/// the commit's full name.
fn commit(folder: &Path, message: &str) -> String {
    for args in [
        &["add", "--all"][..],
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    ] {
        let ran = Command::new("git")
            .arg("-C")
            .arg(folder)
            .args(args)
            .output()
            .unwrap();
        assert!(ran.status.success(), "{ran:?}");
    }
    commits::head(folder).unwrap()
}

/// The paths of the files the tab shows as changed, of what it shows of
/// the repository shown, and how many lines each gained and lost.
fn shown_now(changes: &Entity<Changes>, cx: &mut VisualTestContext) -> Vec<(String, usize, usize)> {
    changes.read_with(cx, |changes, _| {
        let Some(Ok(diff)) = changes.shown_diff() else {
            return Vec::new();
        };
        diff.files
            .iter()
            .map(|file| (file.path.display().to_string(), file.added, file.removed))
            .collect()
    })
}

#[gpui::test]
fn lists_the_commits_and_shows_what_one_changed_and_the_turn_it_was_made_in(
    cx: &mut TestAppContext,
) {
    let folder = repository("history", "one\n");
    cox_editor::changes::start(&folder).unwrap();
    std::fs::write(folder.join("a.txt"), "one\ntwo\n").unwrap();
    let two = commit(&folder, "Two");
    let turn = Turn {
        chat: 7,
        entry: 3,
        line: "Add two".to_owned(),
    };
    commits::mark(&folder, &two, &turn).unwrap();
    std::fs::write(folder.join("b.txt"), "b\n").unwrap();
    commit(&folder, "B");
    std::fs::write(folder.join("a.txt"), "one\ntwo\nthree\n").unwrap();

    let (changes, _, cx) = open(cx, &folder, &folder);
    let turns = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&turns);
    cx.update(|_, cx| {
        cx.subscribe(&changes, move |_, &ShowTurn { chat, entry }, _| {
            heard.borrow_mut().push((chat, entry));
        })
        .detach();
    });
    let subjects = changes.read_with(cx, |changes, _| {
        changes
            .history()
            .map(|history| {
                history
                    .commits
                    .iter()
                    .map(|commit| commit.subject.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    assert_eq!(subjects, ["B", "Two"]);
    // A sketch shows everything it changed at first.
    assert_eq!(
        changes.read_with(cx, |changes, _| changes.shown_pick()),
        Pick::Everything
    );
    assert_eq!(
        shown_now(&changes, cx),
        [("a.txt".to_owned(), 2, 0), ("b.txt".to_owned(), 1, 0)]
    );

    // The rows: everything, what is not committed, B, then Two.
    click("history-commit-1", cx);
    assert_eq!(
        changes.read_with(cx, |changes, _| changes.shown_pick()),
        Pick::Commit(two)
    );
    assert_eq!(shown_now(&changes, cx), [("a.txt".to_owned(), 1, 0)]);
    click("commit-turn", cx);
    assert_eq!(*turns.borrow(), [(7, 3)]);

    click("history-uncommitted", cx);
    assert_eq!(shown_now(&changes, cx), [("a.txt".to_owned(), 1, 0)]);
    click("history-everything", cx);
    assert_eq!(
        shown_now(&changes, cx),
        [("a.txt".to_owned(), 2, 0), ("b.txt".to_owned(), 1, 0)]
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn a_folder_with_no_repository_counts_none() {
    let folder = std::env::temp_dir().join(format!("cox-changes-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("directories/notes")).unwrap();
    assert!(cox::stage::changes::count(&folder).is_err());
    std::fs::remove_dir_all(folder).unwrap();
}
