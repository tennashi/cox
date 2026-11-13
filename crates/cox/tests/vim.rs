#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;
#[expect(dead_code, reason = "these tests use only part of the fake daemon")]
mod fake_daemon;
mod keys;

use std::path::PathBuf;
use std::time::Duration;

use cox::chat::conversation::State;
use cox::chat::view::ChatView;
use cox::stage::terminal::{Program, Running, Terminal};
use cox::vim::Mode;
use cox::workspace::Workspace;
use gpui::{Entity, EntityInputHandler as _, Focusable as _, TestAppContext, VisualTestContext};

use fake_daemon::settled;

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
}
