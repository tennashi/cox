#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;
#[expect(dead_code, reason = "these tests use only part of the fake daemon")]
mod fake_daemon;
mod fake_lsp;
mod keys;

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use cox::chat::conversation::State;
use cox::chat::view::ChatView;
use cox::stage::Tabs;
use cox::stage::changes::Changes;
use cox::stage::code::Code;
use cox::stage::document::Document;
use cox::stage::terminal::{Program, Running, Terminal};
use cox::stage::tree::{Chosen, Tree};
use cox::vim::Mode;
use cox::vim::keymap::Follow;
use cox::workspace::Workspace;
use gpui::{
    Entity, EntityInputHandler as _, Focusable as _, ScrollDelta, ScrollWheelEvent, TestAppContext,
    TouchPhase, VisualTestContext, px,
};

use fake_daemon::{FakeStages, Keyed, settled};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("cox-vim-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

/// Opens the window with Vim's keys on, as the app does when the
/// settings say so.
fn open_window<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    cx.update(cox::vim::install);
    keys::open_window(cx, scratch(name))
}

/// The mode the chat's input shows, if it shows one. A GPUI debug
/// selector only records where the label drew, not its text, so the text
/// is read from the mode.
fn shown_mode(cx: &mut VisualTestContext) -> Option<&'static str> {
    cx.debug_bounds("vim-mode")?;
    cx.update(|_, cx| cox::vim::mode(cx)).map(Mode::label)
}

fn shown_view(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Entity<ChatView> {
    settled(cx, workspace);
    workspace.read_with(cx, |workspace, _| {
        workspace
            .shown()
            .and_then(|chat| chat.view().cloned())
            .expect("a chat is shown")
    })
}

/// Lets the daemon and the agent go on in real time for a while.
fn go_on(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) {
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(10));
        settled(cx, workspace);
    }
}

fn agent_state(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> State {
    view.read_with(cx, |view, _| view.conversation().state())
}

#[gpui::test]
fn a_new_chat_types_at_once_and_normal_mode_types_nothing(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "new");

    keys::press(cx, &workspace, "secondary-n h e l l o");
    assert_eq!(keys::shown_input(cx, &workspace), "hello");
    assert_eq!(shown_mode(cx), Some("INSERT"));

    keys::press(cx, &workspace, "escape j k 0 x space enter backspace");
    assert_eq!(keys::shown_input(cx, &workspace), "hello");
    assert_eq!(shown_mode(cx), Some("NORMAL"));
}

#[gpui::test]
fn i_a_and_their_capitals_type_where_vim_does(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "places");
    keys::press(cx, &workspace, "secondary-n h e l l o escape");

    keys::press(cx, &workspace, "shift-i x escape");
    assert_eq!(keys::shown_input(cx, &workspace), "xhello");
    keys::press(cx, &workspace, "shift-a y escape");
    assert_eq!(keys::shown_input(cx, &workspace), "xhelloy");
    keys::press(cx, &workspace, "left left a z escape");
    assert_eq!(keys::shown_input(cx, &workspace), "xhellozy");
    keys::press(cx, &workspace, "left i w");
    assert_eq!(keys::shown_input(cx, &workspace), "xhellowzy");
    assert_eq!(shown_mode(cx), Some("INSERT"));
}

#[gpui::test]
fn escape_never_stops_the_agent_and_control_c_in_normal_mode_does(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "stop");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    cx.simulate_input("wait");
    keys::press(cx, &workspace, "enter");
    go_on(cx, &workspace);
    assert_eq!(agent_state(cx, &view), State::Busy);

    keys::press(cx, &workspace, "escape");
    go_on(cx, &workspace);
    assert_eq!(shown_mode(cx), Some("NORMAL"));
    assert_eq!(agent_state(cx, &view), State::Busy, "escape in insert mode");

    keys::press(cx, &workspace, "escape");
    go_on(cx, &workspace);
    assert_eq!(agent_state(cx, &view), State::Busy, "escape in normal mode");

    keys::press(cx, &workspace, "ctrl-c");
    go_on(cx, &workspace);
    assert_eq!(agent_state(cx, &view), State::Idle);
}

#[gpui::test]
fn a_count_in_normal_mode_answers_no_question_of_the_agent(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "count");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    cx.simulate_input("permission");
    keys::press(cx, &workspace, "enter");
    let asked = |cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| view.conversation().options_asked())
    };
    for _ in 0..500 {
        if asked(cx) > 0 {
            break;
        }
        go_on(cx, &workspace);
    }
    let options = asked(cx);
    assert!(options > 0, "the agent asks permission");

    keys::press(cx, &workspace, "escape 1");
    go_on(cx, &workspace);
    assert_eq!(asked(cx), options);
    assert_eq!(agent_state(cx, &view), State::Asking);
}

#[gpui::test]
fn escape_in_normal_mode_closes_what_opened_over_the_chat(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "close");
    keys::press(cx, &workspace, "secondary-n");
    let open = |cx: &mut VisualTestContext, context: &str| {
        keys::keyed(cx).iter().any(|keyed| keyed == context)
    };

    keys::press(cx, &workspace, "secondary-,");
    assert!(open(cx, "Settings"));
    keys::press(cx, &workspace, "escape");
    assert!(!open(cx, "Settings"));
    keys::press(cx, &workspace, "secondary-, i escape");
    assert!(open(cx, "Settings"));
    keys::press(cx, &workspace, "escape");
    assert!(!open(cx, "Settings"));

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.edit_goal(0, window, cx);
    });
    keys::press(cx, &workspace, "d o n e escape");
    assert!(open(cx, "Goal"));
    keys::press(cx, &workspace, "escape");
    assert!(!open(cx, "Goal"));

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.rename(0, window, cx);
    });
    keys::press(cx, &workspace, "escape");
    assert!(open(cx, "Renaming"));
    keys::press(cx, &workspace, "escape");
    assert!(!open(cx, "Renaming"));
}

#[gpui::test]
fn the_keys_are_the_input_method_s_while_it_composes(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "composing");
    keys::press(cx, &workspace, "secondary-n escape");
    let view = shown_view(cx, &workspace);
    let input = view.read_with(cx, |view, _| view.input().clone());
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(None, "に", None, window, cx);
        });
    });
    assert!(input.read_with(cx, |input, _| input.is_composing()));

    keys::press(cx, &workspace, "j");
    assert!(keys::shown_input(cx, &workspace).contains('j'));
}

/// Whether the keyboard is in the part of the window `context` names.
fn in_part(cx: &mut VisualTestContext, context: &str) -> bool {
    keys::keyed(cx).iter().any(|keyed| keyed == context)
}

/// Opens a chat in a folder of its own, a project of its own, and goes
/// to normal mode.
fn open_project(cx: &mut VisualTestContext, workspace: &Entity<Workspace>, name: &str) {
    let folder = scratch(name);
    std::fs::create_dir_all(&folder).unwrap();
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder), window, cx);
    });
    settled(cx, workspace);
    keys::press(cx, workspace, "escape");
}

fn chosen(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> usize {
    workspace.read_with(cx, |workspace, _| workspace.chosen())
}

#[gpui::test]
fn the_rail_chooses_a_project_and_opens_a_chat_in_it(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "rail");
    keys::press(cx, &workspace, "secondary-n escape");
    open_project(cx, &workspace, "rail-a");
    open_project(cx, &workspace, "rail-b");
    assert_eq!(chosen(cx, &workspace), 2);

    keys::press(cx, &workspace, "ctrl-w h");
    assert!(in_part(cx, "Rail"));
    keys::press(cx, &workspace, "g g enter");
    assert_eq!(chosen(cx, &workspace), 0);
    assert!(in_part(cx, "Conversation"));
    assert_eq!(shown_mode(cx), Some("NORMAL"));

    keys::press(cx, &workspace, "ctrl-w h j l");
    assert_eq!(chosen(cx, &workspace), 1);
    keys::press(cx, &workspace, "ctrl-w h shift-g enter");
    assert_eq!(chosen(cx, &workspace), 2);
    keys::press(cx, &workspace, "ctrl-w h 2 k enter");
    assert_eq!(chosen(cx, &workspace), 0);
    keys::press(cx, &workspace, "ctrl-w h 9 j enter");
    assert_eq!(chosen(cx, &workspace), 2);

    keys::press(cx, &workspace, "ctrl-w h k o");
    let chats = workspace.read_with(cx, |workspace, _| workspace.chats(1).count());
    assert_eq!(chats, 2);
    assert_eq!(chosen(cx, &workspace), 1);
    assert_eq!(shown_mode(cx), Some("INSERT"));
    keys::press(cx, &workspace, "h i");
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
}

#[gpui::test]
fn control_w_goes_round_the_rail_and_the_chat(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "round");
    keys::press(cx, &workspace, "secondary-n escape");

    keys::press(cx, &workspace, "ctrl-w l");
    assert!(in_part(cx, "Conversation"), "nothing is right of the chat");
    keys::press(cx, &workspace, "ctrl-w w");
    assert!(in_part(cx, "Rail"));
    keys::press(cx, &workspace, "ctrl-w h");
    assert!(in_part(cx, "Rail"), "nothing is left of the rail");
    keys::press(cx, &workspace, "ctrl-w w");
    assert!(in_part(cx, "Conversation"));
    keys::press(cx, &workspace, "ctrl-w h ctrl-w l");
    assert!(in_part(cx, "Conversation"));
    assert_eq!(shown_mode(cx), Some("NORMAL"));
    assert_eq!(keys::shown_input(cx, &workspace), "");
}

#[gpui::test]
fn the_list_of_chats_shows_one_with_the_keys(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "list");
    keys::press(cx, &workspace, "secondary-n escape secondary-n escape");
    let second = shown_view(cx, &workspace);

    keys::press(cx, &workspace, "ctrl-w k");
    assert!(in_part(cx, "Chats"));
    assert!(workspace.read_with(cx, |workspace, _| workspace.is_switching()));
    keys::press(cx, &workspace, "escape");
    assert!(in_part(cx, "Conversation"));
    assert!(!workspace.read_with(cx, |workspace, _| workspace.is_switching()));

    keys::press(cx, &workspace, "ctrl-w k g g enter");
    let first = shown_view(cx, &workspace);
    assert!(in_part(cx, "Conversation"));
    keys::press(cx, &workspace, "ctrl-w k shift-g l");
    let last = shown_view(cx, &workspace);
    assert_ne!(first, last);
    keys::press(cx, &workspace, "ctrl-w k k enter");
    assert_eq!(shown_view(cx, &workspace), first);
    assert!([first, last].contains(&second));

    keys::press(cx, &workspace, "ctrl-w k o");
    let chats = workspace.read_with(cx, |workspace, _| workspace.chats(0).count());
    assert_eq!(chats, 3);
    assert_eq!(shown_mode(cx), Some("INSERT"));
}

#[gpui::test]
fn the_keys_of_the_stage_s_tabs_go_to_the_stage(cx: &mut TestAppContext) {
    let keyed = Keyed::default();
    let stages = FakeStages {
        keyed: Some(keyed.clone()),
        ..FakeStages::default()
    };
    cx.update(cox::vim::install);
    let (workspace, cx) = keys::open_window_on(cx, scratch("tabs"), stages);
    keys::press(cx, &workspace, "secondary-n escape");

    keys::press(
        cx,
        &workspace,
        "g t g shift-t 3 g t 2 g shift-t ctrl-w c ctrl-w h",
    );
    assert_eq!(
        *keyed.borrow(),
        [
            Tabs::Along(1),
            Tabs::Along(-1),
            Tabs::Number(3),
            Tabs::Along(-2),
            Tabs::Close,
            Tabs::Focus,
        ]
    );
    assert_eq!(keys::shown_input(cx, &workspace), "");

    keyed.borrow_mut().clear();
    keys::press(cx, &workspace, "ctrl-w w");
    assert!(in_part(cx, "Rail"));
    keys::press(cx, &workspace, "ctrl-w l");
    assert_eq!(*keyed.borrow(), [Tabs::Focus]);
}

#[gpui::test]
fn escape_closes_the_list_over_the_input(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "completion");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    cx.simulate_input("hello");
    keys::press(cx, &workspace, "enter");
    for _ in 0..500 {
        let known = view.read_with(cx, |view, _| !view.conversation().commands().is_empty());
        if known && agent_state(cx, &view) == State::Idle {
            break;
        }
        go_on(cx, &workspace);
    }

    cx.simulate_input("/co");
    keys::press(cx, &workspace, "escape i enter");
    assert_eq!(
        keys::shown_input(cx, &workspace),
        "",
        "the line went as written"
    );
    for _ in 0..500 {
        if agent_state(cx, &view) == State::Idle {
            break;
        }
        go_on(cx, &workspace);
    }

    cx.simulate_input("/co");
    keys::press(cx, &workspace, "enter");
    assert!(
        keys::shown_input(cx, &workspace).starts_with("/compact"),
        "with the list open, Enter takes its command"
    );
}

#[gpui::test]
fn a_terminal_has_control_w_and_g_t(cx: &mut TestAppContext) {
    cx.update(cox::vim::install);
    cx.update(cox::app::bind_keys);
    let cwd = scratch("terminal-keys");
    std::fs::create_dir_all(&cwd).unwrap();
    let running = Running::start(Program::Command("head -n 1".to_owned()), &cwd).unwrap();
    let (answer, mut waiting) = tokio::sync::oneshot::channel();
    let (terminal, cx) = cx.add_window_view(|_, cx| Terminal::new(running, Some(answer), cx));
    cx.update(|window, cx| window.focus(&terminal.focus_handle(cx)));
    cx.run_until_parked();

    cx.simulate_keystrokes("x space y ctrl-w g t enter");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let ran = loop {
        cx.run_until_parked();
        if let Ok(ran) = waiting.try_recv() {
            break ran.expect("the command started");
        }
        assert!(std::time::Instant::now() < deadline, "head took too long");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(ran.output.ends_with("\nx gt"), "{:?}", ran.output);
    let _ = std::fs::remove_dir_all(cwd);
}

#[gpui::test]
fn a_terminal_has_every_key(cx: &mut TestAppContext) {
    cx.update(cox::vim::install);
    cx.update(cox::app::bind_keys);
    let cwd = scratch("terminal");
    std::fs::create_dir_all(&cwd).unwrap();
    let running = Running::start(Program::Command("head -n 1".to_owned()), &cwd).unwrap();
    let (answer, mut waiting) = tokio::sync::oneshot::channel();
    let (terminal, cx) = cx.add_window_view(|_, cx| Terminal::new(running, Some(answer), cx));
    cx.update(|window, cx| window.focus(&terminal.focus_handle(cx)));
    cx.run_until_parked();

    cx.simulate_keystrokes("j 1 i k enter");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let ran = loop {
        cx.run_until_parked();
        if let Ok(ran) = waiting.try_recv() {
            break ran.expect("the command started");
        }
        assert!(std::time::Instant::now() < deadline, "head took too long");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(ran.output, "j1ik\nj1ik");
    let _ = std::fs::remove_dir_all(cwd);
}

#[gpui::test]
fn off_the_keys_are_as_they_were(cx: &mut TestAppContext) {
    let (workspace, cx) = keys::open_window(cx, scratch("off"));
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    assert_eq!(shown_mode(cx), None);
    cx.simulate_input("wait");
    keys::press(cx, &workspace, "enter");
    go_on(cx, &workspace);
    assert_eq!(agent_state(cx, &view), State::Busy);

    keys::press(cx, &workspace, "escape");
    go_on(cx, &workspace);
    assert_eq!(agent_state(cx, &view), State::Idle);

    keys::press(cx, &workspace, "ctrl-w h g t");
    assert!(!in_part(cx, "Rail"));
    assert_eq!(keys::shown_input(cx, &workspace), "hgt");
}

/// A folder of its own for a test, with the files `paths`, saying their
/// names, by its real path, as the tabs name their files.
fn folder(name: &str, paths: &[&str]) -> PathBuf {
    let folder = scratch(name);
    std::fs::create_dir_all(&folder).unwrap();
    for path in paths {
        let path = folder.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{}\n", path.display())).unwrap();
    }
    folder.canonicalize().unwrap()
}

/// A file of `count` numbered lines.
fn numbered(count: usize) -> String {
    let mut text = String::new();
    for line in 1..=count {
        writeln!(text, "let line_{line} = {line};").unwrap();
    }
    text
}

/// Opens a tab of code, with the keyboard, showing `path`, which says
/// `text`, with Vim's keys on if `vim`.
fn open_code<'a>(
    cx: &'a mut TestAppContext,
    path: PathBuf,
    text: &str,
    vim: bool,
) -> (Entity<Code>, &'a mut VisualTestContext) {
    if vim {
        cx.update(cox::vim::install);
    }
    cx.update(cox::stage::code::bind_keys);
    cx.update(cox::chat::hand::bind_keys);
    let text = text.to_owned();
    let (code, cx) = cx.add_window_view(|_, cx| Code::new(path, &text, None, None, cx));
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    cx.run_until_parked();
    (code, cx)
}

fn cursor(code: &Entity<Code>, cx: &mut VisualTestContext) -> Option<usize> {
    code.read_with(cx, |code, _| code.cursor())
}

fn type_keys(cx: &mut VisualTestContext, keys: &str) {
    for key in keys.split(' ') {
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
    }
}

#[gpui::test]
fn code_has_a_cursor_that_moves_by_lines_halves_and_counts(cx: &mut TestAppContext) {
    let path = folder("code-cursor", &[]).join("long.rs");
    let (code, cx) = open_code(cx, path, &numbered(200), true);
    assert_eq!(cursor(&code, cx), Some(0));

    type_keys(cx, "3 j");
    assert_eq!(cursor(&code, cx), Some(3));
    type_keys(cx, "k");
    assert_eq!(cursor(&code, cx), Some(2));
    type_keys(cx, "1 2 shift-g");
    assert_eq!(cursor(&code, cx), Some(11));
    type_keys(cx, "shift-g");
    let last = cursor(&code, cx).unwrap();
    assert!(last >= 199, "{last}");
    assert!(
        code.read_with(cx, |code, _| code.in_sight())
            .contains(&last)
    );
    type_keys(cx, "g g");
    assert_eq!(cursor(&code, cx), Some(0));
    assert_eq!(code.read_with(cx, |code, _| code.in_sight()).start, 0);

    type_keys(cx, "ctrl-d");
    let half = cursor(&code, cx).unwrap();
    assert!(half > 3, "{half}");
    assert!(code.read_with(cx, |code, _| code.in_sight()).start > 0);
    type_keys(cx, "2 ctrl-d");
    assert_eq!(cursor(&code, cx), Some(half * 3));
    type_keys(cx, "3 ctrl-u");
    assert_eq!(cursor(&code, cx), Some(0));

    type_keys(cx, "5 0 j");
    let bounds = cx.debug_bounds("code").expect("the code is drawn");
    cx.simulate_event(ScrollWheelEvent {
        position: bounds.center(),
        delta: ScrollDelta::Lines(gpui::point(0., -40.)),
        modifiers: gpui::Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    let sight = code.read_with(cx, |code, _| code.in_sight());
    let at = cursor(&code, cx).unwrap();
    assert!(at > 50 && sight.contains(&at), "{at} {sight:?}");
}

#[gpui::test]
fn code_selects_whole_lines_and_copies_them(cx: &mut TestAppContext) {
    let path = folder("code-visual", &[]).join("long.rs");
    let (code, cx) = open_code(cx, path, &numbered(20), true);

    type_keys(cx, "3 j shift-v 2 j k j");
    let selected = code.read_with(cx, |code, _| code.selected_text());
    assert_eq!(
        selected,
        "let line_4 = 4;\nlet line_5 = 5;\nlet line_6 = 6;\n"
    );
    type_keys(cx, "y");
    let copied = cx
        .update(|_, cx| cx.read_from_clipboard())
        .and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some(selected.as_str()));
    assert_eq!(code.read_with(cx, |code, _| code.selected_text()), "");

    type_keys(cx, "shift-v 2 k escape");
    assert_eq!(code.read_with(cx, |code, _| code.selected_text()), "");
    assert_eq!(cursor(&code, cx), Some(3));
}

#[gpui::test]
fn code_hands_the_lines_selected_or_the_cursor_s_to_the_chat(cx: &mut TestAppContext) {
    let path = folder("code-hand", &[]).join("long.rs");
    let (code, cx) = open_code(cx, path, &numbered(20), true);

    type_keys(cx, "j shift-v j g a");
    let handed = cx.update(|_, cx| cox::chat::hand::take(cx));
    assert_eq!(handed.len(), 1);
    assert_eq!(handed[0].label, "long.rs:2-3");
    assert_eq!(handed[0].text, "let line_2 = 2;\nlet line_3 = 3;\n");
    assert_eq!(code.read_with(cx, |code, _| code.selected_text()), "");

    type_keys(cx, "4 j g a");
    let handed = cx.update(|_, cx| cox::chat::hand::take(cx));
    assert_eq!(handed.len(), 1);
    assert_eq!(handed[0].label, "long.rs:7");
    assert_eq!(handed[0].text, "let line_7 = 7;");
}

#[gpui::test]
fn code_goes_back_and_forward_with_control_o_and_control_i(cx: &mut TestAppContext) {
    let folder = folder("code-back", &["a.rs", "b.rs", "c.rs"]);
    let (code, cx) = open_code(cx, folder.join("a.rs"), "a\n", true);
    for name in ["b.rs", "c.rs"] {
        code.update(cx, |code, cx| code.go_to_file(&folder.join(name), cx));
    }
    let shown = |cx: &mut VisualTestContext| {
        code.read_with(cx, |code, _| code.path().file_name().unwrap().to_owned())
    };

    type_keys(cx, "2 ctrl-o");
    assert_eq!(shown(cx), "a.rs");
    type_keys(cx, "ctrl-i");
    assert_eq!(shown(cx), "b.rs");
}

#[gpui::test]
fn off_code_has_no_cursor_and_its_keys_are_as_they_were(cx: &mut TestAppContext) {
    let path = folder("code-off", &[]).join("long.rs");
    let (code, cx) = open_code(cx, path, &numbered(200), false);

    type_keys(cx, "3 j shift-g ctrl-d shift-v y g a");
    assert_eq!(cursor(&code, cx), None);
    assert_eq!(code.read_with(cx, |code, _| code.in_sight()).start, 0);
    assert_eq!(code.read_with(cx, |code, _| code.selected_text()), "");
    assert_eq!(cx.update(|_, cx| cox::chat::hand::take(cx)).len(), 0);
}

#[gpui::test]
fn a_document_scrolls_by_lines_halves_and_to_its_ends(cx: &mut TestAppContext) {
    cx.update(cox::vim::install);
    cx.update(cox::stage::document::bind_keys);
    let path = folder("document", &[]).join("long.md");
    let mut text = String::new();
    for line in 1..=300 {
        write!(text, "Line {line}.\n\n").unwrap();
    }
    let (document, cx) = cx.add_window_view(|_, cx| Document::new(path, text, cx));
    cx.update(|window, cx| window.focus(&document.focus_handle(cx)));
    cx.run_until_parked();
    let top = |cx: &mut VisualTestContext| {
        cx.debug_bounds("document-text")
            .expect("the text is drawn")
            .origin
            .y
    };
    let start = top(cx);

    type_keys(cx, "j");
    let line = start - top(cx);
    assert!(line > px(0.), "{line:?}");
    type_keys(cx, "3 j");
    assert_eq!(start - top(cx), line * 4.);
    type_keys(cx, "2 k");
    assert_eq!(start - top(cx), line * 2.);
    type_keys(cx, "g g");
    assert_eq!(top(cx), start);
    type_keys(cx, "ctrl-d");
    let half = start - top(cx);
    assert!(half > line * 4., "{half:?}");
    type_keys(cx, "ctrl-u");
    assert_eq!(top(cx), start);
    type_keys(cx, "shift-g");
    let bottom = start - top(cx);
    assert!(bottom > half * 10., "{bottom:?}");
    type_keys(cx, "j");
    assert_eq!(start - top(cx), bottom);
}

#[gpui::test]
fn the_tree_moves_a_selection_and_opens_and_closes_folders(cx: &mut TestAppContext) {
    cx.update(cox::vim::install);
    let root = folder("tree", &["a/one.rs", "a/two.rs", "b.rs", "c.rs"]);
    let open = root.clone();
    let (tree, cx) = cx.add_window_view(|_, cx| Tree::new(open, Vec::new(), cx));
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&chosen);
    cx.update(|window, cx| {
        window.focus(&tree.focus_handle(cx));
        cx.subscribe(&tree, move |_, chosen: &Chosen, _| {
            heard.borrow_mut().push(chosen.path.clone());
        })
        .detach();
    });
    cx.run_until_parked();
    let rows = |cx: &mut VisualTestContext| tree.read_with(cx, |tree, _| tree.shown().len());
    assert_eq!(rows(cx), 3);

    type_keys(cx, "l");
    assert_eq!(rows(cx), 5, "the first row, a folder, opens");
    type_keys(cx, "l l");
    assert_eq!(*chosen.borrow(), [root.join("a/one.rs")]);
    type_keys(cx, "j h");
    assert_eq!(rows(cx), 5, "a file goes to its folder");
    type_keys(cx, "h");
    assert_eq!(rows(cx), 3, "the folder closes");
    type_keys(cx, "shift-g enter");
    type_keys(cx, "2 k j enter");
    type_keys(cx, "g g 2 j l");
    type_keys(cx, "2 g g enter");
    assert_eq!(
        *chosen.borrow(),
        [
            root.join("a/one.rs"),
            root.join("c.rs"),
            root.join("b.rs"),
            root.join("c.rs"),
            root.join("b.rs"),
        ]
    );
}

/// A folder of its own for a test, a Git repository with a commit of
/// `paths`, each saying its name, which then change.
fn changed(name: &str, paths: &[&str]) -> PathBuf {
    let folder = folder(name, paths);
    for args in [
        &["init", "--quiet"][..],
        &["add", "."],
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
        let ran = std::process::Command::new("git")
            .arg("-C")
            .arg(&folder)
            .args(args)
            .output()
            .unwrap();
        assert!(ran.status.success(), "{ran:?}");
    }
    for path in paths {
        std::fs::write(folder.join(path), "changed\n").unwrap();
    }
    folder
}

#[gpui::test]
fn changes_move_from_file_to_file_and_open_one(cx: &mut TestAppContext) {
    cx.update(cox::vim::install);
    let root = changed("changes", &["a.txt", "b.txt", "c.txt"]);
    let (folder, cwd) = (root.clone(), root.clone());
    let (changes, cx) = cx.add_window_view(|_, cx| Changes::new(folder, &cwd, cx));
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&chosen);
    cx.update(|window, cx| {
        window.focus(&changes.focus_handle(cx));
        cx.subscribe(
            &changes,
            move |_, chosen: &cox::stage::changes::Chosen, _| {
                heard.borrow_mut().push(chosen.path.clone());
            },
        )
        .detach();
    });
    for _ in 0..500 {
        cx.run_until_parked();
        let files = changes.read_with(cx, |changes, _| {
            changes
                .shown_diff()
                .and_then(|read| read.as_ref().ok())
                .map_or(0, |diff| diff.files.len())
        });
        if files == 3 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    type_keys(
        cx,
        "enter j enter 5 j enter 2 k enter g g enter 2 shift-g enter",
    );
    assert_eq!(
        *chosen.borrow(),
        ["a.txt", "c.txt", "a.txt", "a.txt", "b.txt"].map(|name| root.join(name))
    );
}

#[gpui::test]
fn the_conversation_scrolls_from_the_input_in_normal_mode(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "conversation");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    let words = "word ".repeat(1500);
    cx.simulate_input(&words);
    keys::press(cx, &workspace, "enter");
    for _ in 0..500 {
        if agent_state(cx, &view) == State::Idle {
            break;
        }
        go_on(cx, &workspace);
    }
    keys::press(cx, &workspace, "escape");
    let down = |cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            -view.scroll().scroll_px_offset_for_scrollbar().y
        })
    };
    let end = down(cx);
    assert!(end > px(0.), "the conversation is taller than the window");

    keys::press(cx, &workspace, "g g");
    assert_eq!(down(cx), px(0.));
    keys::press(cx, &workspace, "j");
    let line = down(cx);
    assert!(line > px(0.));
    keys::press(cx, &workspace, "3 j");
    assert_eq!(down(cx), line * 4.);
    keys::press(cx, &workspace, "k");
    assert_eq!(down(cx), line * 3.);
    keys::press(cx, &workspace, "ctrl-d");
    let half = down(cx) - line * 3.;
    assert!(half > line * 3., "{half:?}");
    keys::press(cx, &workspace, "ctrl-u");
    assert_eq!(down(cx), line * 3.);
    keys::press(cx, &workspace, "shift-g");
    assert_eq!(down(cx), end);
    assert_eq!(keys::shown_input(cx, &workspace), "");

    keys::press(cx, &workspace, "shift-a o k");
    assert_eq!(keys::shown_input(cx, &workspace), "ok");
}

#[gpui::test]
fn g_d_goes_to_the_definition_of_the_first_name_of_the_line(cx: &mut TestAppContext) {
    cx.update(cox::vim::install);
    cx.update(cox::stage::code::bind_keys);
    let directory = folder("definition", &[]);
    fake_lsp::write_project(&directory);
    let servers = cox::stage::servers::Servers::new(fake_lsp::start(std::sync::Arc::default()));
    let path = directory.join("main.rs");
    let server = cx
        .update(|cx| servers.of(&directory, &path, cx))
        .expect("the editor knows Rust's server");
    let ready = server.clone();
    let (code, cx) =
        cx.add_window_view(|_, cx| Code::new(path, fake_lsp::MAIN, None, Some(server), cx));
    cx.update(|window, cx| window.focus(&code.focus_handle(cx)));
    let wait_until = |cx: &mut VisualTestContext, done: &dyn Fn(&mut VisualTestContext) -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the server took too long"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    wait_until(cx, &|cx| {
        ready.read_with(cx, |server, _| server.client().is_some())
    });
    let shown = |cx: &mut VisualTestContext| {
        code.read_with(cx, |code, _| code.path().file_name().unwrap().to_owned())
    };

    type_keys(cx, "j g d");
    wait_until(cx, &|cx| shown(cx) == "lib.rs");
    assert_eq!(code.read_with(cx, |code, _| code.selected_text()), "helper");
    type_keys(cx, "ctrl-o");
    assert_eq!(shown(cx), "main.rs");
}

/// What stands for a page in these tests, which need CEF to be one: a
/// part of the window in the page's key context that keeps what Vim's
/// keys ask of it, and the keys that reach it.
struct FakePage {
    focus: gpui::FocusHandle,
    asked: Vec<String>,
}

impl gpui::Render for FakePage {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use cox::vim::{Address, CopyAddress, FocusField, Hint, Label, Leave, Move, Reload};
        use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};

        let keep = |page: &mut Self, asked: String| page.asked.push(asked);
        gpui::div()
            .size_full()
            .track_focus(&self.focus)
            .key_context("Page")
            .on_key_down(cx.listener(move |page, event: &gpui::KeyDownEvent, _, _| {
                keep(page, format!("key {}", event.keystroke.unparse()));
            }))
            .on_action(cx.listener(move |page, motion: &Move, _, _| {
                keep(page, format!("{:?} {:?}", motion.motion, motion.count));
            }))
            .on_action(cx.listener(move |page, hint: &Hint, _, _| {
                keep(page, format!("hint {:?}", hint.follow));
            }))
            .on_action(cx.listener(move |page, label: &Label, _, _| {
                keep(page, format!("label {}", label.key));
            }))
            .on_action(cx.listener(move |page, _: &Leave, _, _| keep(page, "leave".into())))
            .on_action(cx.listener(move |page, _: &Reload, _, _| keep(page, "reload".into())))
            .on_action(cx.listener(move |page, _: &FocusField, _, _| keep(page, "field".into())))
            .on_action(cx.listener(move |page, _: &CopyAddress, _, _| keep(page, "copy".into())))
            .on_action(cx.listener(move |page, _: &Address, _, _| keep(page, "address".into())))
            .on_action(cx.listener(move |page, _: &cox::stage::code::Back, _, _| {
                keep(page, "back".into());
            }))
            .on_action(
                cx.listener(move |page, _: &cox::stage::code::Forward, _, _| {
                    keep(page, "forward".into());
                }),
            )
            .child("page")
    }
}

fn open_page(cx: &mut TestAppContext) -> (Entity<FakePage>, &mut VisualTestContext) {
    cx.update(cox::vim::install);
    let (page, cx) = cx.add_window_view(|_, cx| FakePage {
        focus: cx.focus_handle(),
        asked: Vec::new(),
    });
    cx.update(|window, cx| window.focus(&page.read(cx).focus));
    cx.run_until_parked();
    (page, cx)
}

fn asked(page: &Entity<FakePage>, cx: &mut VisualTestContext) -> Vec<String> {
    page.update(cx, |page, _| std::mem::take(&mut page.asked))
}

#[gpui::test]
fn a_page_in_normal_mode_scrolls_goes_back_and_reloads_with_vimperator_s_keys(
    cx: &mut TestAppContext,
) {
    let (page, cx) = open_page(cx);
    cx.simulate_keystrokes("j 3 j ctrl-d ctrl-u g g shift-g");
    assert_eq!(
        asked(&page, cx),
        [
            "Down None",
            "Down Some(3)",
            "HalfDown None",
            "HalfUp None",
            "Top None",
            "Bottom None"
        ]
    );
    cx.simulate_keystrokes("2 shift-h shift-l r g i y y o x");
    assert_eq!(
        asked(&page, cx),
        [
            "back", "back", "forward", "reload", "field", "copy", "address"
        ]
    );
    assert_eq!(cx.update(|_, cx| cox::vim::mode(cx)), Some(Mode::Normal));
}

#[gpui::test]
fn a_page_s_field_takes_every_key_and_escape_leaves_it(cx: &mut TestAppContext) {
    let (page, cx) = open_page(cx);
    cx.update(|_, cx| cox::vim::enter(Mode::Insert, cx));
    cx.simulate_keystrokes("j f g g escape");
    assert_eq!(
        asked(&page, cx),
        ["key j", "key f", "key g", "key g", "leave"]
    );
    assert_eq!(cx.update(|_, cx| cox::vim::mode(cx)), Some(Mode::Normal));

    cx.simulate_keystrokes("i cmd-c k escape");
    assert_eq!(asked(&page, cx), ["key cmd-c", "key k", "leave"]);
}

#[gpui::test]
fn hint_mode_sends_the_labels_typed_to_the_page_until_escape(cx: &mut TestAppContext) {
    let (page, cx) = open_page(cx);
    cx.simulate_keystrokes("f");
    assert_eq!(asked(&page, cx), ["hint Here"]);
    assert_eq!(cx.update(|_, cx| cox::vim::mode(cx)), Some(Mode::Hint));
    cx.simulate_keystrokes("s j shift-k escape");
    assert_eq!(asked(&page, cx), ["label s", "label j", "label k", "leave"]);
    assert_eq!(cx.update(|_, cx| cox::vim::mode(cx)), Some(Mode::Normal));

    cx.simulate_keystrokes("shift-f");
    assert_eq!(asked(&page, cx), ["hint NewTab"]);
    cx.update(|_, cx| cox::vim::enter(Mode::Normal, cx));
    cx.simulate_keystrokes("j");
    assert_eq!(asked(&page, cx), ["Down None"]);
}

/// What the line at the window's foot shows: whether it is open, what is
/// typed in it, what the list over it holds, and what it said.
#[derive(Debug, PartialEq, Eq)]
struct Shown {
    open: bool,
    text: String,
    fits: Vec<String>,
    said: Option<String>,
}

fn line(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Shown {
    workspace.read_with(cx, |workspace, cx| {
        let line = workspace.line().expect("Vim's keys are on").read(cx);
        Shown {
            open: line.is_open(),
            text: line.text(cx).to_owned(),
            fits: line.fits().into_iter().map(|fit| fit.line).collect(),
            said: line.said().map(str::to_owned),
        }
    })
}

#[gpui::test]
fn colon_opens_the_line_whose_list_narrows_and_escape_closes_it(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "line");
    keys::press(cx, &workspace, "secondary-n escape :");
    let shown = line(cx, &workspace);
    assert!(shown.open);
    assert_eq!(shown.fits.len(), cox::vim::ex::COMMANDS.len());
    assert!(in_part(cx, "CommandLine"));
    assert_eq!(shown_mode(cx), Some("COMMAND"));

    keys::press(cx, &workspace, "s t");
    assert_eq!(line(cx, &workspace).fits[0], "stop");
    keys::press(cx, &workspace, "tab");
    assert_eq!(line(cx, &workspace).text, "stop");
    keys::press(cx, &workspace, "tab");
    let second = line(cx, &workspace).fits[1].clone();
    assert_eq!(line(cx, &workspace).text, second, "Tab goes along the list");
    keys::press(cx, &workspace, "shift-tab");
    assert_eq!(line(cx, &workspace).text, "stop");
    keys::press(cx, &workspace, "ctrl-n ctrl-p");
    assert_eq!(line(cx, &workspace).text, "stop");

    keys::press(cx, &workspace, "escape");
    assert!(!line(cx, &workspace).open);
    assert!(in_part(cx, "Conversation"));
    assert_eq!(shown_mode(cx), Some("NORMAL"));
    assert_eq!(keys::shown_input(cx, &workspace), "");
    let chats = workspace.read_with(cx, |workspace, _| workspace.chats(0).count());
    assert_eq!(chats, 1, "escape ran nothing");
}

#[gpui::test]
fn enter_runs_new_settings_and_an_action_by_its_name(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "line-run");
    keys::press(cx, &workspace, "secondary-n escape");
    let chats = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, _| workspace.chats(0).count())
    };

    keys::press(cx, &workspace, ": n e w enter");
    assert_eq!(chats(cx), 2);
    assert_eq!(shown_mode(cx), Some("INSERT"), "a new chat types at once");

    keys::press(cx, &workspace, "escape : s e t enter");
    assert!(in_part(cx, "Settings"));
    keys::press(cx, &workspace, "escape");

    keys::press(cx, &workspace, ":");
    cx.simulate_input("cox::NewChat");
    keys::press(cx, &workspace, "enter");
    assert_eq!(chats(cx), 3);
}

#[gpui::test]
fn colon_stop_stops_the_agent(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "line-stop");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    cx.simulate_input("wait");
    keys::press(cx, &workspace, "enter");
    go_on(cx, &workspace);
    assert_eq!(agent_state(cx, &view), State::Busy);

    keys::press(cx, &workspace, "escape : s t o p enter");
    go_on(cx, &workspace);
    assert_eq!(agent_state(cx, &view), State::Idle);
}

#[gpui::test]
fn colon_quit_and_the_rest_go_to_the_stage(cx: &mut TestAppContext) {
    let keyed = Keyed::default();
    let stages = FakeStages {
        keyed: Some(keyed.clone()),
        ..FakeStages::default()
    };
    cx.update(cox::vim::install);
    let (workspace, cx) = keys::open_window_on(cx, scratch("line-tabs"), stages);
    keys::press(cx, &workspace, "secondary-n escape");

    keys::press(
        cx,
        &workspace,
        ": q enter : r e enter : b a enter : f o enter",
    );
    keys::press(cx, &workspace, ":");
    cx.simulate_input("tabopen example.com");
    keys::press(cx, &workspace, "enter :");
    cx.simulate_input("o src/main.rs");
    keys::press(cx, &workspace, "enter");
    assert_eq!(
        *keyed.borrow(),
        [
            Tabs::Close,
            Tabs::Reload,
            Tabs::Back,
            Tabs::Forward,
            Tabs::Open {
                line: "example.com".to_owned(),
                follow: Follow::NewTab
            },
            Tabs::Open {
                line: "src/main.rs".to_owned(),
                follow: Follow::Here
            },
        ]
    );
    assert_eq!(keys::shown_input(cx, &workspace), "");
}

#[gpui::test]
fn the_line_keeps_what_was_typed_for_up_and_down(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "line-history");
    keys::press(cx, &workspace, "secondary-n escape");
    keys::press(cx, &workspace, ": q enter : h e l p enter");
    assert!(line(cx, &workspace).open, ":help opens the line again");
    assert_eq!(
        line(cx, &workspace).fits.len(),
        cox::vim::ex::COMMANDS.len(),
        "listing every command"
    );

    keys::press(cx, &workspace, "up");
    assert_eq!(line(cx, &workspace).text, "help");
    keys::press(cx, &workspace, "up");
    assert_eq!(line(cx, &workspace).text, "q");
    keys::press(cx, &workspace, "up");
    assert_eq!(line(cx, &workspace).text, "q", "there is none older");
    keys::press(cx, &workspace, "down down");
    assert_eq!(line(cx, &workspace).text, "");

    keys::press(cx, &workspace, "h up");
    assert_eq!(line(cx, &workspace).text, "help", "of those that start so");
    keys::press(cx, &workspace, "up");
    assert_eq!(line(cx, &workspace).text, "help");
    keys::press(cx, &workspace, "escape");
}

#[gpui::test]
fn the_line_says_so_of_a_command_it_does_not_know(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "line-unknown");
    keys::press(cx, &workspace, "secondary-n escape : f o o enter");
    let shown = line(cx, &workspace);
    assert!(!shown.open);
    assert_eq!(shown.said.as_deref(), Some("Not a command: foo"));
    assert_eq!(shown_mode(cx), Some("NORMAL"));
    keys::press(cx, &workspace, ":");
    assert_eq!(line(cx, &workspace).said, None);
}

#[gpui::test]
fn colon_chat_shows_the_chat_whose_name_fits(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "line-chat");
    keys::press(cx, &workspace, "secondary-n escape");
    keys::press(cx, &workspace, "secondary-n escape");
    let names: Vec<String> = workspace.read_with(cx, |workspace, _| {
        workspace
            .chats(0)
            .map(|chat| chat.name().to_owned())
            .collect()
    });
    let shown_key = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, _| {
            workspace.shown().map(cox::workspace::Chat::key)
        })
    };
    let first = workspace.read_with(cx, |workspace, _| workspace.chats(0).next().unwrap().key());
    keys::press(cx, &workspace, ":");
    cx.simulate_input("chat ");
    assert_eq!(line(cx, &workspace).fits.len(), names.len());
    keys::press(cx, &workspace, "escape");
    assert_ne!(shown_key(cx), Some(first));
    keys::press(cx, &workspace, ":");
    cx.simulate_input(&format!("ch {}", names[0]));
    keys::press(cx, &workspace, "enter");
    assert_eq!(shown_key(cx), Some(first));
}

#[gpui::test]
fn off_there_is_no_line(cx: &mut TestAppContext) {
    let (workspace, cx) = keys::open_window(cx, scratch("line-off"));
    keys::press(cx, &workspace, "secondary-n escape :");
    assert!(workspace.read_with(cx, |workspace, _| workspace.line().is_none()));
    assert!(!in_part(cx, "CommandLine"));
}
