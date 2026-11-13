//! A human reading a file of code on a tab of the stage, at the lines the
//! agent shows it at, and asking the language server what the code means.

mod fake_lsp;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use cox::stage::code::{Back, Code, Forward};
use cox::stage::servers::{LanguageServer, Servers};
use cox::stage::tab::Shows as _;
use cox_core::tabs::Lines;
use cox_editor::language::Highlight;
use gpui::{
    Entity, Focusable as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Point,
    ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, point, px,
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
    cx.update(cox::stage::code::bind_keys);
    let text = text.to_owned();
    let (code, cx) = cx.add_window_view(|_, cx| Code::new(path, &text, lines, None, cx));
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
fn colors_the_code_as_it_changes(cx: &mut TestAppContext) {
    let directory = scratch("colors");
    let path = directory.join("main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let (code, cx) = open(cx, path.clone(), "fn main() {}\n", None);
    let highlights = |code: &Entity<Code>, cx: &mut VisualTestContext, line| {
        code.read_with(cx, |code, _| code.highlights(line))
    };
    assert!(highlights(&code, cx, 0).contains(&(0..2, Highlight::Keyword)));

    std::fs::write(&path, "// Runs.\nfn main() {}\n").unwrap();
    look(cx);
    assert_eq!(
        highlights(&code, cx, 0),
        [(0.."// Runs.".len(), Highlight::Comment)]
    );
    assert!(highlights(&code, cx, 1).contains(&(0..2, Highlight::Keyword)));

    // A file in a language the editor does not know is plain.
    let path = directory.join("notes.txt");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let (code, cx) = open(cx, path, "fn main() {}\n", None);
    assert_eq!(highlights(&code, cx, 0).first(), None);
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

/// Opens a tab showing `main.rs` of the fake server's project in
/// `directory`, asking a server of `servers`, once it is ready.
fn open_project<'a>(
    cx: &'a mut TestAppContext,
    directory: &Path,
    servers: &Servers,
) -> (Entity<Code>, &'a mut VisualTestContext) {
    fake_lsp::write_project(directory);
    cx.update(cox::stage::code::bind_keys);
    let path = directory.join("main.rs");
    let server: Entity<LanguageServer> = cx
        .update(|cx| servers.of(directory, &path, cx))
        .expect("the editor knows Rust's server");
    let ready = server.clone();
    let (code, cx) =
        cx.add_window_view(|_, cx| Code::new(path, fake_lsp::MAIN, None, Some(server), cx));
    wait_until(cx, |cx| {
        ready.read_with(cx, |server, _| server.client().is_some())
    });
    (code, cx)
}

/// Lets the server answer until `done` holds, which it must before long.
fn wait_until(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "the server took too long");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Where the tab draws `column` of `line`.
fn at(
    code: &Entity<Code>,
    cx: &mut VisualTestContext,
    line: usize,
    column: usize,
) -> Point<gpui::Pixels> {
    code.read_with(cx, |code, _| code.point_of(line, column))
        .expect("the line is in sight")
}

fn hover(code: &Entity<Code>, cx: &mut VisualTestContext) -> Option<String> {
    code.read_with(cx, |code, _| code.hover().map(str::to_owned))
}

fn shown(code: &Entity<Code>, cx: &mut VisualTestContext) -> (String, Option<Lines>) {
    code.read_with(cx, |code, _| {
        let name = code
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        (name, code.marked())
    })
}

#[gpui::test]
fn shares_a_projects_server_between_its_files(cx: &mut TestAppContext) {
    let directory = scratch("shared");
    let started = Arc::new(AtomicUsize::new(0));
    let servers = Servers::new(fake_lsp::start(Arc::clone(&started)));
    let elsewhere = scratch("elsewhere");
    cx.update(|cx| {
        let main = servers
            .of(&directory, &directory.join("main.rs"), cx)
            .unwrap();
        let lib = servers
            .of(&directory, &directory.join("lib.rs"), cx)
            .unwrap();
        assert_eq!(main.entity_id(), lib.entity_id());
        // A file outside the project gets a server of its own, and a file
        // in a language with no server none.
        let outside = servers
            .of(&directory, &elsewhere.join("main.rs"), cx)
            .unwrap();
        assert_ne!(main.entity_id(), outside.entity_id());
        assert!(
            servers
                .of(&directory, &directory.join("Cargo.toml"), cx)
                .is_none()
        );
    });
    assert_eq!(started.load(Ordering::Relaxed), 2);
    std::fs::remove_dir_all(directory).unwrap();
    std::fs::remove_dir_all(elsewhere).unwrap();
}

#[gpui::test]
fn a_project_says_which_server_knows_a_language(cx: &mut TestAppContext) {
    let directory = scratch("languages");
    let servers = Servers::new(fake_lsp::start(Arc::default()));
    let name = |path: &str, cx: &mut gpui::App| {
        servers
            .of(&directory, &directory.join(path), cx)
            .map(|server| server.read(cx).name().to_owned())
    };
    cx.update(|cx| {
        // A language Cox draws no colors of has its server all the same.
        assert_eq!(name("main.go", cx).as_deref(), Some("gopls"));
        assert_eq!(name("app.py", cx).as_deref(), Some("pyright"));
    });
    std::fs::write(
        directory.join("servers.json"),
        r#"{ "servers": {
            "pylsp": { "command": ["pylsp"], "languages": ["python"] },
            "gopls": null
        } }"#,
    )
    .unwrap();
    cx.update(|cx| {
        assert_eq!(name("main.go", cx), None);
        assert_eq!(name("app.py", cx).as_deref(), Some("pylsp"));
        assert_eq!(name("main.rs", cx).as_deref(), Some("rust-analyzer"));
    });
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn shows_what_the_server_says_of_the_name_the_mouse_rests_on(cx: &mut TestAppContext) {
    let directory = scratch("hover");
    let servers = Servers::new(fake_lsp::start(Arc::default()));
    let (code, cx) = open_project(cx, &directory, &servers);

    // On `helper`, the mouse rests a while before the server is asked.
    let helper = at(&code, cx, 1, 6);
    cx.simulate_mouse_move(helper, None, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(hover(&code, cx), None);
    cx.executor().advance_clock(Duration::from_millis(400));
    wait_until(cx, |cx| hover(&code, cx).is_some());
    assert_eq!(hover(&code, cx).as_deref(), Some(fake_lsp::HOVER));
    let shown = cx.debug_bounds("code-hover").unwrap();
    assert!(shown.top() > helper.y, "{shown:?} is under {helper:?}");

    // It stays while the mouse moves along the name, and goes once the
    // mouse leaves it.
    let along = at(&code, cx, 1, 8);
    cx.simulate_mouse_move(along, None, Modifiers::none());
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(hover(&code, cx).is_some());
    // To the spaces before it, which are no name.
    let away = at(&code, cx, 1, 1);
    cx.simulate_mouse_move(away, None, Modifiers::none());
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(hover(&code, cx), None);
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn vim_s_keys_move_the_cursor_onto_a_name_and_the_server_says_what_it_is(cx: &mut TestAppContext) {
    let directory = scratch("hover-keys");
    let servers = Servers::new(fake_lsp::start(Arc::default()));
    cx.update(|cx| cox::vim::turn(true, cx));
    let (code, cx) = open_project(cx, &directory, &servers);
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.run_until_parked();
    let place = |cx: &mut VisualTestContext| code.read_with(cx, |code, _| code.cursor_place());
    let keys = |cx: &mut VisualTestContext, keys: &str| {
        for key in keys.split(' ') {
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
        }
    };

    keys(cx, "l l l");
    assert_eq!(place(cx), Some((0, 3)), "on `main`, by characters");
    keys(cx, "j");
    assert_eq!(place(cx), Some((1, 3)), "down, on the same character");
    keys(cx, "w");
    assert_eq!(place(cx), Some((1, 4)), "on to `helper`");
    assert_eq!(hover(&code, cx), None, "not before the cursor rests");
    cx.executor().advance_clock(Duration::from_millis(400));
    wait_until(cx, |cx| hover(&code, cx).is_some());
    assert_eq!(hover(&code, cx).as_deref(), Some(fake_lsp::HOVER));
    let shown = cx.debug_bounds("code-hover").unwrap();
    assert!(shown.top() > at(&code, cx, 1, 4).y, "under the name");

    keys(cx, "e");
    assert_eq!(place(cx), Some((1, 9)), "along the name, it stays");
    assert!(hover(&code, cx).is_some());
    keys(cx, "w");
    assert_eq!(place(cx), Some((1, 10)), "off the name, to `();`");
    assert_eq!(hover(&code, cx), None, "it goes at once");
    keys(cx, "0 $");
    assert_eq!(place(cx), Some((1, 12)));
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn shift_k_says_what_the_name_is_at_once_and_gr_lists_where_it_is_used(cx: &mut TestAppContext) {
    let directory = scratch("look-up-keys");
    let servers = Servers::new(fake_lsp::start(Arc::default()));
    cx.update(|cx| cox::vim::turn(true, cx));
    let (code, cx) = open_project(cx, &directory, &servers);
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.run_until_parked();
    let keys = |cx: &mut VisualTestContext, keys: &str| {
        for key in keys.split(' ') {
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
        }
    };

    keys(cx, "j w");
    assert_eq!(hover(&code, cx), None, "not before the cursor rests");
    keys(cx, "shift-k");
    wait_until(cx, |cx| hover(&code, cx).is_some());
    assert_eq!(hover(&code, cx).as_deref(), Some(fake_lsp::HOVER));

    keys(cx, "escape g r");
    wait_until(cx, |cx| cx.debug_bounds("code-locations").is_some());
    assert!(cx.debug_bounds("code-place-1").is_some(), "in lib.rs");
    assert!(cx.debug_bounds("code-place-3").is_some(), "in main.rs");
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn goes_to_the_definition_and_back(cx: &mut TestAppContext) {
    let directory = scratch("definition");
    let servers = Servers::new(fake_lsp::start(Arc::default()));
    let (code, cx) = open_project(cx, &directory, &servers);

    let helper = at(&code, cx, 1, 6);
    cx.simulate_mouse_down(helper, MouseButton::Left, Modifiers::secondary_key());
    cx.simulate_mouse_up(helper, MouseButton::Left, Modifiers::secondary_key());
    wait_until(cx, |cx| shown(&code, cx).0 == "lib.rs");
    assert_eq!(shown(&code, cx).1, Some(Lines { first: 1, last: 1 }));
    assert_eq!(code.read_with(cx, |code, _| code.selected_text()), "helper");
    assert_eq!(code.read_with(cx, |code, _| code.text()), fake_lsp::LIB);

    cx.dispatch_action(Back);
    assert_eq!(shown(&code, cx), ("main.rs".to_owned(), None));
    assert_eq!(code.read_with(cx, |code, _| code.text()), fake_lsp::MAIN);
    cx.dispatch_action(Forward);
    assert_eq!(shown(&code, cx).0, "lib.rs");
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn lists_where_a_name_is_used_to_go_to_each(cx: &mut TestAppContext) {
    let directory = scratch("references");
    let servers = Servers::new(fake_lsp::start(Arc::default()));
    let (code, cx) = open_project(cx, &directory, &servers);

    let helper = at(&code, cx, 1, 6);
    cx.simulate_mouse_down(helper, MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
    let find = cx.debug_bounds("find-references").expect("the menu opens");
    cx.simulate_click(find.center(), Modifiers::none());
    wait_until(cx, |cx| cx.debug_bounds("code-locations").is_some());

    // By file: `lib.rs`, then `main.rs`, each with its place.
    assert!(cx.debug_bounds("code-place-0").is_none());
    let place = cx.debug_bounds("code-place-1").unwrap();
    cx.simulate_click(place.center(), Modifiers::none());
    assert_eq!(
        shown(&code, cx),
        ("lib.rs".to_owned(), Some(Lines { first: 1, last: 1 }))
    );
    let place = cx.debug_bounds("code-place-3").unwrap();
    cx.simulate_click(place.center(), Modifiers::none());
    assert_eq!(
        shown(&code, cx),
        ("main.rs".to_owned(), Some(Lines { first: 2, last: 2 }))
    );
    // The list stays, and back goes through the places it went to.
    assert!(cx.debug_bounds("code-locations").is_some());
    cx.dispatch_action(Back);
    assert_eq!(shown(&code, cx).0, "lib.rs");
    std::fs::remove_dir_all(directory).unwrap();
}
