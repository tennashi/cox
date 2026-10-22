//! A human using the window, with the agent faked in memory.

mod fake_agent;

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    McpServer as AcpMcpServer, McpServerStdio, RequestPermissionOutcome, SelectedPermissionOutcome,
};
use cox::conversation::{Entry, State};
use cox::input::LineInput;
use cox::view::CoxView;
use cox_acp::{McpServer, ToolCallResult};
use fake_agent::FakeAgent;
use gpui::{
    ClipboardItem, Entity, EntityInputHandler, Focusable, Modifiers, TestAppContext,
    VisualTestContext,
};

/// Opens the window, talking to a fake agent, with the input focused.
fn open(cx: &mut TestAppContext) -> (Arc<FakeAgent>, Entity<CoxView>, &mut VisualTestContext) {
    open_with(cx, Vec::new())
}

/// Opens the window like [`open`], handing the agent `mcp_servers`.
fn open_with(
    cx: &mut TestAppContext,
    mcp_servers: Vec<McpServer>,
) -> (Arc<FakeAgent>, Entity<CoxView>, &mut VisualTestContext) {
    // The agent side runs on a thread of its own, so the test waits for it
    // in real time instead of only running GPUI's own tasks.
    cx.executor().allow_parking();
    cx.update(cox::input::bind_keys);
    let agent = Arc::new(FakeAgent::default());
    let fake = Arc::clone(&agent);
    let (view, cx) = cx.add_window_view(|_, cx| {
        CoxView::new(
            cox::agent::spawn(move || Ok(fake.connect()), mcp_servers),
            None::<gpui::AnyView>,
            cx,
        )
    });
    cx.update(|window, cx| window.focus(&view.focus_handle(cx)));
    (agent, view, cx)
}

/// Waits until the conversation is in `state`. The agent side runs in real
/// time, outside GPUI's clock, so a real timer ends the wait if it never
/// comes.
async fn wait_for(cx: &mut VisualTestContext, view: &Entity<CoxView>, state: State) {
    let (timed_out, timeout) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        let _ = timed_out.send(());
    });
    tokio::select! {
        () = cx.condition(view, |view, _| view.conversation().state() == state) => {}
        _ = timeout => panic!("the conversation never became {state:?}"),
    }
}

#[gpui::test]
async fn sends_what_the_human_types_and_shows_the_reply(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);

    cx.simulate_input("hello");
    assert_eq!(input_text(cx, &view), "hello");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(agent.sessions_opened(), 1);
    assert_eq!(agent.prompts(), ["hello"]);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.conversation().entries()[..2],
            [
                Entry::Human("hello".to_owned()),
                Entry::Reply("echo: hello".to_owned())
            ]
        );
    });
    assert_eq!(input_text(cx, &view), "");
}

#[gpui::test]
async fn keeps_the_newest_entries_in_view(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    for _ in 0..40 {
        cx.simulate_input("hello");
        cx.simulate_keystrokes("enter");
        wait_for(cx, &view, State::Idle).await;
    }
    cx.run_until_parked();

    let (offset, max) = view.read_with(cx, |view, _| {
        (view.scroll().offset().y, view.scroll().max_offset().height)
    });
    assert!(max > gpui::px(0.), "the conversation overflows");
    assert_eq!(offset, -max);
}

#[gpui::test]
async fn hands_the_agent_its_tools(cx: &mut TestAppContext) {
    let stage = McpServer {
        name: "stage".to_owned(),
        command: "/usr/bin/npx".into(),
        args: vec!["-y".to_owned(), "chrome-devtools-mcp".to_owned()],
    };
    let (agent, view, cx) = open_with(cx, vec![stage]);

    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(
        agent.mcp_servers(),
        [AcpMcpServer::Stdio(
            McpServerStdio::new("stage", "/usr/bin/npx")
                .args(vec!["-y".to_owned(), "chrome-devtools-mcp".to_owned()])
        )]
    );
}

/// Types "ab", then composes with an input method the way GPUI relays it:
/// the text under composition is marked, and only the committed text is
/// sent.
#[gpui::test]
async fn sends_what_the_input_method_commits(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);
    let input = view.read_with(cx, |view, _| view.input().clone());
    let compose = |cx: &mut VisualTestContext, text: &str| {
        input.update_in(cx, |input, window, cx| {
            input.replace_and_mark_text_in_range(None, text, None, window, cx);
        });
    };

    cx.simulate_input("ab");
    compose(cx, "k");
    compose(cx, "こんにちは");
    assert_eq!(input_text(cx, &view), "abこんにちは");
    assert!(input.read_with(cx, |input, _| input.is_composing()));
    // Enter while composing is for the input method, not for Cox.
    cx.simulate_keystrokes("enter");
    assert_eq!(agent.prompts(), Vec::<String>::new());

    input.update_in(cx, |input, window, cx| {
        input.replace_text_in_range(None, "今日は", window, cx);
    });
    assert_eq!(input_text(cx, &view), "ab今日は");
    assert!(!input.read_with(cx, |input, _| input.is_composing()));
    cx.simulate_keystrokes("backspace");
    assert_eq!(input_text(cx, &view), "ab今日");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(agent.prompts(), ["ab今日"]);
}

#[gpui::test]
fn cuts_copies_and_pastes_with_the_keyboard(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("hello world");
    cx.simulate_keystrokes("shift-left shift-left shift-left shift-left shift-left secondary-x");
    assert_eq!(input_text(cx, &view), "hello ");
    assert_eq!(clipboard(cx), Some("world".to_owned()));

    cx.simulate_keystrokes("home secondary-v");
    assert_eq!(input_text(cx, &view), "worldhello ");

    cx.simulate_keystrokes("secondary-a secondary-c");
    assert_eq!(clipboard(cx), Some("worldhello ".to_owned()));
    cx.simulate_input("x");
    assert_eq!(input_text(cx, &view), "x");
}

#[gpui::test]
fn keeps_pasted_line_breaks(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo\r\nthree".to_owned()));
    cx.simulate_keystrokes("secondary-v");

    assert_eq!(input_text(cx, &view), "one\ntwo\nthree");
}

/// The address line holds one line.
#[gpui::test]
fn an_input_of_one_line_pastes_line_breaks_as_spaces(cx: &mut TestAppContext) {
    cx.update(cox::input::bind_keys);
    let (input, cx) = cx.add_window_view(|_, cx| LineInput::new(cx));
    cx.update(|window, cx| window.focus(&input.focus_handle(cx)));

    cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo\r\nthree".to_owned()));
    cx.simulate_keystrokes("secondary-v");

    assert_eq!(
        input.read_with(cx, |input, _| input.text().to_owned()),
        "one two three"
    );
}

#[gpui::test]
async fn sends_lines_started_with_shift_enter_as_one_prompt(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);

    cx.simulate_input("first");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("second");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(agent.prompts(), ["first\nsecond"]);
}

#[gpui::test]
fn moves_between_lines_with_up_and_down(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("ab");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("cd");
    cx.simulate_keystrokes("up end");
    cx.simulate_input("X");
    cx.simulate_keystrokes("down home");
    cx.simulate_input("Y");

    assert_eq!(input_text(cx, &view), "abX\nYcd");
}

/// Selecting leftward and then rightward past where it started turns the
/// selection around that point.
#[gpui::test]
fn a_selection_keeps_its_anchor(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("abcd");
    cx.simulate_keystrokes("left left shift-left shift-right shift-right shift-right secondary-x");

    assert_eq!(input_text(cx, &view), "ab");
    assert_eq!(clipboard(cx), Some("cd".to_owned()));
}

fn clipboard(cx: &mut VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn input_text(cx: &mut VisualTestContext, view: &Entity<CoxView>) -> String {
    view.read_with(cx, |view, cx| view.input().read(cx).text().to_owned())
}

#[gpui::test]
async fn answers_a_permission_request_with_a_click(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);

    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
    assert_eq!(tool_call(cx, &view), ("touch x.txt".to_owned(), None));
    let option = cx
        .debug_bounds("option-1")
        .expect("the first option is drawn");
    cx.simulate_click(option.center(), Modifiers::none());
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(
        agent.answers(),
        [RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new("allow")
        )]
    );
    assert_eq!(
        tool_call(cx, &view),
        ("touch x.txt".to_owned(), Some(ToolCallResult::Completed))
    );
}

/// The title and result of the first tool call in the conversation.
fn tool_call(
    cx: &mut VisualTestContext,
    view: &Entity<CoxView>,
) -> (String, Option<ToolCallResult>) {
    view.read_with(cx, |view, _| {
        view.conversation()
            .entries()
            .iter()
            .find_map(|entry| match entry {
                Entry::ToolCall { title, result, .. } => Some((title.clone(), *result)),
                _ => None,
            })
            .expect("a tool call is shown")
    })
}
