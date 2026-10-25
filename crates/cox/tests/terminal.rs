//! A terminal on a tab of the stage: the human types into it, and a
//! command the agent runs there answers how it went.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cox::mcp::{Ended, Ran};
use cox::terminal::{Program, Running, Terminal};
use gpui::{
    Entity, Focusable as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext,
    VisualTestContext, point, px,
};
use tokio::sync::oneshot;

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("cox-terminal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::canonicalize(directory).unwrap()
}

/// The agent's command, waited for.
type Waiting = oneshot::Receiver<Result<Ran, String>>;

/// Opens a tab running `command` in `cwd`, as the agent asks, with the
/// keyboard in it.
fn run<'a>(
    cx: &'a mut TestAppContext,
    command: &str,
    cwd: &std::path::Path,
) -> (Entity<Terminal>, Waiting, &'a mut VisualTestContext) {
    cx.update(cox::terminal::bind_keys);
    let running = Running::start(Program::Command(command.to_owned()), cwd).unwrap();
    let (answer, waiting) = oneshot::channel();
    let (terminal, cx) = cx.add_window_view(|_, cx| Terminal::new(running, Some(answer), cx));
    cx.update(|window, cx| window.focus(&terminal.focus_handle(cx)));
    cx.run_until_parked();
    (terminal, waiting, cx)
}

/// Lets the terminal go on until `done` holds, which it must before long.
fn wait_until(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "the terminal took too long");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// How the command went, once it says.
fn ran(cx: &mut VisualTestContext, waiting: &mut Waiting) -> Ran {
    let mut ran = None;
    wait_until(cx, |_| {
        ran = waiting.try_recv().ok();
        ran.is_some()
    });
    ran.unwrap().expect("the command started")
}

fn ended(cx: &mut VisualTestContext, terminal: &Entity<Terminal>) -> Option<Ended> {
    terminal.read_with(cx, |terminal, _| terminal.ended())
}

#[gpui::test]
fn a_command_answers_what_it_printed_and_how_it_ended(cx: &mut TestAppContext) {
    let cwd = scratch("ended");
    let (terminal, mut waiting, cx) = run(cx, "pwd; printf 'built\\n'; exit 3", &cwd);

    let ran = ran(cx, &mut waiting);
    assert_eq!(
        ran,
        Ran {
            output: format!("{}\nbuilt", cwd.display()),
            ended: Ended::Exited(3),
        }
    );
    assert_eq!(ended(cx, &terminal), Some(Ended::Exited(3)));
    assert_eq!(
        terminal.read_with(cx, |terminal, _| terminal.title()),
        "pwd; printf 'built\\n'; exit 3"
    );
    std::fs::remove_dir_all(cwd).unwrap();
}

#[gpui::test]
fn the_human_types_into_what_runs(cx: &mut TestAppContext) {
    let cwd = scratch("typed");
    let (_terminal, mut waiting, cx) = run(cx, "head -n 1", &cwd);

    cx.simulate_input("こんにちは");
    cx.simulate_keystrokes("enter");

    let ran = ran(cx, &mut waiting);
    // The terminal shows the line as typed, then as `head` printed it.
    assert_eq!(ran.output, "こんにちは\nこんにちは");
    assert_eq!(ran.ended, Ended::Exited(0));
    std::fs::remove_dir_all(cwd).unwrap();
}

#[gpui::test]
fn control_c_interrupts_what_runs(cx: &mut TestAppContext) {
    let cwd = scratch("interrupt");
    let (_terminal, mut waiting, cx) = run(cx, "sleep 30", &cwd);

    cx.simulate_keystrokes("ctrl-c");

    let ended = ran(cx, &mut waiting).ended;
    assert!(
        matches!(ended, Ended::Signal(2) | Ended::Exited(130)),
        "{ended:?}"
    );
    std::fs::remove_dir_all(cwd).unwrap();
}

#[gpui::test]
fn a_command_the_agent_stops_waiting_for_is_interrupted(cx: &mut TestAppContext) {
    let cwd = scratch("cancelled");
    let (terminal, waiting, cx) = run(cx, "sleep 30", &cwd);

    drop(waiting);
    wait_until(cx, |cx| {
        cx.executor().advance_clock(Duration::from_secs(1));
        ended(cx, &terminal).is_some()
    });
    assert_ne!(ended(cx, &terminal), Some(Ended::Exited(0)));
    std::fs::remove_dir_all(cwd).unwrap();
}

#[gpui::test]
fn closing_the_tab_answers_what_the_command_printed_so_far(cx: &mut TestAppContext) {
    let cwd = scratch("closed");
    let (terminal, mut waiting, cx) = run(cx, "printf started; sleep 30", &cwd);
    wait_until(cx, |cx| {
        terminal.read_with(cx, |terminal, _| terminal.text()) == "started"
    });

    drop(terminal);
    cx.update(|window, _| window.remove_window());

    assert_eq!(
        ran(cx, &mut waiting),
        Ran {
            output: "started".to_owned(),
            ended: Ended::Closed,
        }
    );
    std::fs::remove_dir_all(cwd).unwrap();
}

#[gpui::test]
fn copies_the_line_the_human_selects(cx: &mut TestAppContext) {
    let cwd = scratch("copied");
    let (terminal, mut waiting, cx) = run(cx, "printf 'first\\nsecond\\n'", &cwd);
    ran(cx, &mut waiting);

    // Three clicks on the first line select it.
    let bounds = cx.debug_bounds("terminal").expect("the terminal is drawn");
    let first = bounds.origin + point(px(20.), px(10.));
    for click_count in 1..=3 {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: first,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: first,
            modifiers: Modifiers::none(),
            click_count,
        });
    }
    assert_eq!(
        terminal
            .read_with(cx, |terminal, _| terminal.selected_text())
            .as_deref(),
        Some("first\n")
    );
    let copy = if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-shift-c"
    };
    cx.simulate_keystrokes(copy);
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("first\n")
    );
    std::fs::remove_dir_all(cwd).unwrap();
}
