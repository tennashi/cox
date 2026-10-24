//! A human opening sessions in the window, moving between them and closing
//! them, with each session's agent faked in memory.

#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cox::conversation::{Entry, State};
use cox::view::SessionView;
use cox::workspace::{Open, Workspace};
use fake_agent::FakeAgent;
use gpui::{
    AnyView, AppContext as _, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext,
};

/// The agent of each session opened, in the order they were opened.
type Agents = Rc<RefCell<Vec<Arc<FakeAgent>>>>;

/// Opens the window with no session, sessions with no folder working in
/// `home`.
fn open_window(
    cx: &mut TestAppContext,
    home: PathBuf,
) -> (Agents, Entity<Workspace>, &mut VisualTestContext) {
    // The agents run on threads of their own, so the test waits for them
    // in real time instead of only running GPUI's own tasks.
    cx.executor().allow_parking();
    cx.update(|cx| {
        cox::input::bind_keys(cx);
        cox::view::bind_keys(cx);
        cox::workspace::bind_keys(cx);
    });
    let agents = Agents::default();
    let opened = Rc::clone(&agents);
    let open: Open = Rc::new(move |cwd, _, cx| {
        let agent = Arc::new(FakeAgent::default());
        opened.borrow_mut().push(Arc::clone(&agent));
        let agent = cox::agent::spawn(move || Ok(agent.connect()), cwd, Vec::new());
        cx.new(|cx| SessionView::new(agent, None::<AnyView>, cx))
    });
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(home, open, cx));
    cx.update(|window, cx| window.focus(&workspace.focus_handle(cx)));
    (agents, workspace, cx)
}

/// A directory of its own for each test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("cox-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
}

fn shown(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Entity<SessionView> {
    workspace.read_with(cx, |workspace, _| {
        workspace
            .shown()
            .expect("a session is shown")
            .view()
            .clone()
    })
}

fn names(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Vec<String> {
    workspace.read_with(cx, |workspace, _| {
        workspace
            .sessions()
            .iter()
            .map(|session| session.name().to_owned())
            .collect()
    })
}

fn input_text(cx: &mut VisualTestContext, view: &Entity<SessionView>) -> String {
    view.read_with(cx, |view, cx| view.input().read(cx).text().to_owned())
}

/// Sends `text` in the session shown, and waits for the reply.
async fn say(cx: &mut VisualTestContext, workspace: &Entity<Workspace>, text: &str) {
    let view = shown(cx, workspace);
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    wait_for_turn(cx, &view).await;
}

/// Waits until a turn of the session `view` has ended.
async fn wait_for_turn(cx: &mut VisualTestContext, view: &Entity<SessionView>) {
    let (timed_out, timeout) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let _ = timed_out.send(());
    });
    tokio::select! {
        () = cx.condition(view, |view, _| {
            view.conversation().state() == State::Idle && !view.conversation().entries().is_empty()
        }) => {}
        _ = timeout => panic!("the agent never answered"),
    }
}

#[gpui::test]
async fn each_session_works_where_it_was_opened(cx: &mut TestAppContext) {
    let home = scratch("home");
    let folder = scratch("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let (agents, workspace, cx) = open_window(cx, home.clone());

    click(cx, "new-session");
    say(cx, &workspace, "hello").await;
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    say(cx, &workspace, "hello").await;

    let cwds: Vec<_> = agents.borrow().iter().map(|agent| agent.cwd()).collect();
    assert_eq!(cwds, [Some(home.clone()), Some(folder.clone())]);
    assert!(home.is_dir(), "the home is made for the session");
    assert_eq!(
        names(cx, &workspace),
        [
            "Session 1".to_owned(),
            folder.file_name().unwrap().to_string_lossy().into_owned()
        ]
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn switching_keeps_each_sessions_conversation_and_draft(cx: &mut TestAppContext) {
    let (agents, workspace, cx) = open_window(cx, scratch("switching"));

    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "first").await;
    cx.simulate_keystrokes("secondary-n");
    let second = shown(cx, &workspace);
    cx.simulate_input("not sent yet");

    click(cx, "session-1");
    let first = shown(cx, &workspace);
    assert_ne!(first, second);
    say(cx, &workspace, "again").await;

    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(shown(cx, &workspace), second);
    assert_eq!(input_text(cx, &second), "not sent yet");
    let replies = |cx: &mut VisualTestContext, view: &Entity<SessionView>| {
        view.read_with(cx, |view, _| {
            view.conversation()
                .entries()
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Reply(reply) => Some(reply.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(replies(cx, &first), ["echo: first", "echo: again"]);
    assert_eq!(replies(cx, &second), Vec::<String>::new());
    assert_eq!(agents.borrow()[0].prompts(), ["first", "again"]);
    assert_eq!(agents.borrow()[1].prompts(), Vec::<String>::new());
}

#[gpui::test]
fn closing_a_session_shows_the_next_and_lets_it_go(cx: &mut TestAppContext) {
    let (_agents, workspace, cx) = open_window(cx, scratch("closing"));
    for _ in 0..3 {
        cx.simulate_keystrokes("secondary-n");
    }
    click(cx, "session-2");
    let closed = shown(cx, &workspace).downgrade();

    click(cx, "close-2");
    cx.run_until_parked();

    assert_eq!(names(cx, &workspace), ["Session 1", "Session 3"]);
    let shown_name = workspace.read_with(cx, |workspace, _| {
        workspace.shown().map(|session| session.name().to_owned())
    });
    assert_eq!(shown_name.as_deref(), Some("Session 3"));
    assert!(closed.upgrade().is_none(), "the closed session is gone");

    click(cx, "close-2");
    click(cx, "close-1");
    let shown_any = workspace.read_with(cx, |workspace, _| workspace.shown().is_some());
    assert!(!shown_any);
}

#[gpui::test]
async fn stopping_with_the_button_leaves_the_keyboard_in_the_input(cx: &mut TestAppContext) {
    let (agents, workspace, cx) = open_window(cx, scratch("stop"));
    click(cx, "new-session");
    let view = shown(cx, &workspace);

    cx.simulate_input("wait");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    click(cx, "stop");
    wait_for_turn(cx, &view).await;
    cx.simulate_input("next");

    assert_eq!(agents.borrow()[0].cancels(), 1);
    assert_eq!(input_text(cx, &view), "next");
}
