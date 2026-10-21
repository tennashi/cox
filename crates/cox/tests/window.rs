//! A human using the window, with the agent faked in memory.

mod fake_agent;

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{RequestPermissionOutcome, SelectedPermissionOutcome};
use cox::conversation::{Entry, State};

use cox::view::CoxView;
use fake_agent::FakeAgent;
use gpui::{Entity, EntityInputHandler, Focusable, Modifiers, TestAppContext, VisualTestContext};

/// Opens the window, talking to a fake agent, with the input focused.
fn open(cx: &mut TestAppContext) -> (Arc<FakeAgent>, Entity<CoxView>, &mut VisualTestContext) {
    // The agent side runs on a thread of its own, so the test waits for it
    // in real time instead of only running GPUI's own tasks.
    cx.executor().allow_parking();
    cx.update(cox::input::bind_keys);
    let agent = Arc::new(FakeAgent::default());
    let fake = Arc::clone(&agent);
    let (view, cx) = cx.add_window_view(|_, cx| {
        CoxView::new(
            cox::agent::spawn(move || Ok(fake.connect())),
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

fn input_text(cx: &mut VisualTestContext, view: &Entity<CoxView>) -> String {
    view.read_with(cx, |view, cx| view.input().read(cx).text().to_owned())
}

#[gpui::test]
async fn answers_a_permission_request_with_a_click(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);

    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
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
}
