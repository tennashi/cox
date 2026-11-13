//! A human using the window, with the agent faked in memory.

#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;
#[expect(dead_code, reason = "these tests use only part of the fake daemon")]
mod fake_daemon;
mod scratch;
#[expect(dead_code, reason = "these tests use only part of the waits")]
mod waits;

use std::rc::Rc;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{
    McpServer as AcpMcpServer, McpServerStdio, RequestPermissionOutcome, SelectedPermissionOutcome,
};
use cox::agent::setup::{Policy, Setup};
use cox::chat::conversation::{Content, Entry, State, ToolCallResult};
use cox::chat::view::ChatView;
use cox::stage::page::address::{AddressLine, Asked, Row};
use cox::stage::page::history::{self, History};
use cox::ui::input::{Blurred, LineInput};
use cox::workspace::Workspace;
use cox_acp::McpServer;
use cox_core::diff::Line;
use fake_agent::{Event, FakeAgent, LogsIn};
use fake_daemon::{FakeStages, Fakes, Kept, Opened, open_window, settle, settle_until, shown_view};
use gpui::{
    Bounds, ClipboardItem, Entity, EntityInputHandler, Focusable, Modifiers, MouseButton,
    MouseDownEvent, MouseUpEvent, Pixels, TestAppContext, VisualTestContext, point, px,
};
use waits::given_up;

/// Opens the window, talking to a fake agent, with the input focused.
fn open(cx: &mut TestAppContext) -> (Arc<FakeAgent>, Entity<ChatView>, &mut VisualTestContext) {
    open_with(cx, Vec::new(), Opened::default(), None)
}

/// Opens the window like [`open`], on a chat whose agent is set up as
/// `setup` says.
fn open_set_up(
    cx: &mut TestAppContext,
    setup: Setup,
) -> (Arc<FakeAgent>, Entity<ChatView>, &mut VisualTestContext) {
    open_with(cx, Vec::new(), Opened::default(), Some(setup))
}

/// Opens the window like [`open`], handing the agent `mcp_servers`, and
/// noting in `opened` the links the human opens.
fn open_with(
    cx: &mut TestAppContext,
    mcp_servers: Vec<McpServer>,
    opened: Opened,
    setup: Option<Setup>,
) -> (Arc<FakeAgent>, Entity<ChatView>, &mut VisualTestContext) {
    open_with_stage(cx, mcp_servers, opened, setup, false)
}

/// Opens the window like [`open_with`], with a stand-in for the stage
/// beside the conversation if `stage` is true, closed.
fn open_with_stage(
    cx: &mut TestAppContext,
    mcp_servers: Vec<McpServer>,
    opened: Opened,
    setup: Option<Setup>,
    stage: bool,
) -> (Arc<FakeAgent>, Entity<ChatView>, &mut VisualTestContext) {
    open_agent(cx, FakeAgent::default(), mcp_servers, opened, setup, stage)
}

/// Opens the window like [`open`], talking to `agent`.
fn open_talking_to(
    cx: &mut TestAppContext,
    agent: FakeAgent,
) -> (Arc<FakeAgent>, Entity<ChatView>, &mut VisualTestContext) {
    open_agent(cx, agent, Vec::new(), Opened::default(), None, false)
}

/// The home of the test's window, where Cox's own chats work: a folder
/// of the test's own.
fn home() -> std::path::PathBuf {
    let home = scratch::data().join("home");
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// Opens the window like [`open_with_stage`], on a chat of Cox's own,
/// which works in the test's [`home`], talking to `agent`, which gets no
/// instructions and no terminal.
fn open_agent(
    cx: &mut TestAppContext,
    agent: FakeAgent,
    mcp_servers: Vec<McpServer>,
    opened: Opened,
    setup: Option<Setup>,
    stage: bool,
) -> (Arc<FakeAgent>, Entity<ChatView>, &mut VisualTestContext) {
    let fakes = Fakes {
        mcp_servers,
        setup,
        instructions: false,
        terminal: false,
        ..Fakes::default()
    };
    fakes.next.borrow_mut().push_back(agent);
    let agents = fakes.agents.clone();
    let stages = FakeStages {
        opened: stage.then_some(opened),
        made: None,
        keyed: None,
        built: None,
    };
    let (workspace, cx) = open_window(cx, home(), Kept::Memory, fakes, stages);
    cx.simulate_keystrokes("secondary-n");
    let executor = cx.executor();
    let view = executor.block_test(shown_view(cx, &workspace));
    // The agent starts once the window gave the chat its stage.
    executor.block_test(settle_until(cx, |_| !agents.borrow().is_empty()));
    let agent = Arc::clone(&agents.borrow()[0]);
    cx.update(|window, cx| window.focus(&view.focus_handle(cx)));
    (agent, view, cx)
}

/// Waits until the conversation is in `state`. The agent side runs in real
/// time, outside GPUI's clock, so a real timer ends the wait if it never
/// comes (see [`given_up`]).
async fn wait_for(cx: &mut VisualTestContext, view: &Entity<ChatView>, state: State) {
    settle(cx).await;
    let (_waiting, timeout) = given_up();
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
    assert_eq!(agent.cwd(), Some(home()));
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

    let (offset, max) = scrolled(cx, &view);
    assert!(max > gpui::px(0.), "the conversation overflows");
    assert_eq!(offset, -max);
}

#[gpui::test]
async fn stays_where_the_human_scrolled_back_to(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);
    for _ in 0..40 {
        cx.simulate_input("hello");
        cx.simulate_keystrokes("enter");
        wait_for(cx, &view, State::Idle).await;
    }
    cx.run_until_parked();

    view.update(cx, |view, cx| {
        view.scroll().scroll_by(gpui::px(-200.));
        cx.notify();
    });
    cx.run_until_parked();
    let (back, max) = scrolled(cx, &view);
    assert!(back > -max, "scrolled back from the end");

    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;
    cx.run_until_parked();

    let (offset, max) = scrolled(cx, &view);
    assert_eq!(offset, back);
    assert!(offset > -max, "the end went on out of view");
}

/// How far the conversation is scrolled down, as a negative number, and
/// how far it scrolls at most.
fn scrolled(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> (gpui::Pixels, gpui::Pixels) {
    view.read_with(cx, |view, _| {
        let list = view.scroll();
        (
            list.scroll_px_offset_for_scrollbar().y,
            list.max_offset_for_scrollbar().height,
        )
    })
}

#[gpui::test]
async fn hands_the_agent_its_tools(cx: &mut TestAppContext) {
    let stage = McpServer {
        name: "stage".to_owned(),
        command: "/usr/bin/npx".into(),
        args: vec!["-y".to_owned(), "chrome-devtools-mcp".to_owned()],
    };
    let (agent, view, cx) = open_with(cx, vec![stage], Opened::default(), None);

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
fn undoes_and_redoes_with_the_keyboard(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("hello");
    cx.simulate_keystrokes("space");
    cx.simulate_input("world");
    cx.simulate_keystrokes("backspace backspace");
    assert_eq!(input_text(cx, &view), "hello wor");

    cx.simulate_keystrokes("secondary-z");
    assert_eq!(
        input_text(cx, &view),
        "hello world",
        "the letters taken out come back"
    );
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(
        input_text(cx, &view),
        "",
        "what was typed goes, as one edit"
    );
    cx.simulate_keystrokes("secondary-shift-z secondary-shift-z");
    assert_eq!(input_text(cx, &view), "hello wor");
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
    let (input, cx) = one_line_input(cx);

    cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo\r\nthree".to_owned()));
    cx.simulate_keystrokes("secondary-v");

    assert_eq!(
        input.read_with(cx, |input, _| input.text().to_owned()),
        "one two three"
    );
}

/// The input tells when the keyboard leaves it, as the settings keep what
/// was typed then.
#[gpui::test]
fn tells_when_the_keyboard_leaves_the_input(cx: &mut TestAppContext) {
    let (input, cx) = one_line_input(cx);
    // Only the active window tells where its keyboard goes.
    cx.update(|window, _| window.activate_window());
    let left = Rc::new(std::cell::Cell::new(0));
    let _subscription = cx.update({
        let left = left.clone();
        |_, cx| cx.subscribe(&input, move |_, Blurred, _| left.set(left.get() + 1))
    });
    cx.simulate_input("typed");
    cx.run_until_parked();
    assert_eq!(left.get(), 0);

    let elsewhere = cx.update(|_, cx| cx.focus_handle());
    cx.update(|window, _| window.focus(&elsewhere));
    cx.run_until_parked();
    assert_eq!(left.get(), 1);
}

/// An input of one line, focused, on its own in a window.
fn one_line_input(cx: &mut TestAppContext) -> (Entity<LineInput>, &mut VisualTestContext) {
    cx.update(cox::ui::input::bind_keys);
    let (input, cx) = cx.add_window_view(|_, cx| LineInput::new(cx));
    cx.update(|window, cx| window.focus(&input.focus_handle(cx)));
    (input, cx)
}

/// Clicks at `position` as the `count`th click of a run of quick ones.
fn click_times(cx: &mut VisualTestContext, position: gpui::Point<Pixels>, count: usize) {
    for click_count in 1..=count {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::none(),
            click_count,
        });
    }
}

/// A double click selects the word under the mouse, and a triple click
/// the line.
#[gpui::test]
fn a_double_click_selects_a_word_and_a_triple_click_the_line(cx: &mut TestAppContext) {
    let (input, cx) = one_line_input(cx);
    let text = |cx: &mut VisualTestContext| input.read_with(cx, |input, _| input.text().to_owned());
    cx.simulate_input("hello world");
    // Over the "h".
    let first = point(px(2.), px(4.));

    click_times(cx, first, 2);
    cx.simulate_keystrokes("secondary-x");
    assert_eq!(text(cx), " world");
    assert_eq!(clipboard(cx), Some("hello".to_owned()));

    click_times(cx, first, 3);
    cx.simulate_keystrokes("secondary-x");
    assert_eq!(text(cx), "");
    assert_eq!(clipboard(cx), Some(" world".to_owned()));
}

/// The keys of the platform move and delete by words, and go to the start
/// of the text.
#[gpui::test]
fn moves_and_deletes_by_words(cx: &mut TestAppContext) {
    let (word, top) = if cfg!(target_os = "macos") {
        ("alt", "cmd-up")
    } else {
        ("ctrl", "ctrl-home")
    };
    let (input, cx) = one_line_input(cx);
    cx.simulate_input("one two three");

    cx.simulate_keystrokes(&format!("{word}-left {word}-backspace"));
    cx.simulate_keystrokes(&format!("{word}-shift-right secondary-x {top}"));
    cx.simulate_input("X");

    assert_eq!(
        input.read_with(cx, |input, _| input.text().to_owned()),
        "Xone "
    );
    assert_eq!(clipboard(cx), Some("three".to_owned()));
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

/// What the chat's input shows while it is empty.
fn placeholder(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> String {
    view.read_with(cx, |view, cx| {
        view.input().read(cx).placeholder().to_owned()
    })
}

fn input_text(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> String {
    view.read_with(cx, |view, cx| view.input().read(cx).text().to_owned())
}

#[gpui::test]
async fn answers_a_permission_request_with_a_click(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);

    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
    assert_eq!(tool_call(cx, &view), ("Edit x.txt".to_owned(), None));
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
        ("Edit x.txt".to_owned(), Some(ToolCallResult::Completed))
    );
}

#[gpui::test]
async fn stops_a_turn_with_escape_and_sends_the_next_line(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);

    cx.simulate_input("wait");
    cx.simulate_keystrokes("enter");
    settle_until(cx, |_| agent.did(&Event::Waiting)).await;
    cx.simulate_keystrokes("escape");
    wait_for(cx, &view, State::Idle).await;
    assert_eq!(agent.cancels(), 1);

    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(agent.prompts(), ["wait", "hello"]);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.conversation().entries(),
            [
                Entry::Human("wait".to_owned()),
                Entry::Reply("waiting".to_owned()),
                Entry::Note("Stopped.".to_owned()),
                Entry::Human("hello".to_owned()),
                Entry::Reply("echo: hello".to_owned()),
            ]
        );
    });
}

#[gpui::test]
async fn answers_a_permission_request_with_the_number_of_an_option(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);

    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
    cx.run_until_parked();
    // The input says how to answer.
    assert!(placeholder(cx, &view).contains("choose with 1"));
    // No option has the number.
    cx.simulate_keystrokes("9");
    cx.simulate_keystrokes("1");
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(
        agent.answers(),
        [RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new("allow")
        )]
    );
    // The number that chose went to no input.
    assert_eq!(input_text(cx, &view), "9");
}

#[gpui::test]
async fn stops_a_turn_that_asks_permission_with_the_stop_button(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);

    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
    let stop = cx.debug_bounds("stop").expect("the stop button is drawn");
    cx.simulate_click(stop.center(), Modifiers::none());
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(agent.cancels(), 1);
    assert_eq!(agent.answers(), [RequestPermissionOutcome::Cancelled]);
    assert_eq!(
        tool_call(cx, &view),
        ("Edit x.txt".to_owned(), Some(ToolCallResult::Cancelled))
    );
}

/// A file that is never there, so that the agent counts as logged in only
/// once Cox logs in with its key.
fn no_login_file() -> std::path::PathBuf {
    scratch::folder("no-login").join("logged-in")
}

/// What the conversation shows of each time the agent asked to log in:
/// why the way tried last failed, and the way chosen.
fn logins(
    cx: &mut VisualTestContext,
    view: &Entity<ChatView>,
) -> Vec<(Option<String>, Option<usize>)> {
    view.read_with(cx, |view, _| {
        view.conversation()
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::Login { failed, chosen, .. } => Some((failed.clone(), *chosen)),
                _ => None,
            })
            .collect()
    })
}

#[gpui::test]
async fn logs_in_as_the_agent_asks_before_it_opens_the_session(cx: &mut TestAppContext) {
    let (agent, view, cx) = open_talking_to(
        cx,
        FakeAgent::logging_in(LogsIn::AtSession(no_login_file())),
    );

    wait_for(cx, &view, State::Asking).await;
    view.read_with(cx, |view, _| {
        let Some(Entry::Login { options, .. }) = view.conversation().entries().last() else {
            panic!("the agent asks to log in");
        };
        let names: Vec<_> = options.iter().map(|option| option.name.as_str()).collect();
        assert_eq!(names, ["API key", "Old key", "Terminal"]);
        assert!(options[2].terminal.is_some());
        assert_eq!(view.conversation().asking_for(), Some("log in"));
    });
    cx.run_until_parked();
    assert!(placeholder(cx, &view).contains("log in"));
    cx.simulate_keystrokes("1");
    wait_for(cx, &view, State::Idle).await;
    say(cx, &view, "hello").await;

    assert_eq!(agent.logged_in(), [fake_agent::KEY]);
    assert_eq!(agent.sessions_opened(), 1);
    assert_eq!(agent.prompts(), ["hello"]);
    assert_eq!(logins(cx, &view), [(None, Some(0))]);
}

#[gpui::test]
async fn sends_the_line_again_once_the_human_logs_in_as_the_turn_asks(cx: &mut TestAppContext) {
    let (agent, view, cx) =
        open_talking_to(cx, FakeAgent::logging_in(LogsIn::AtPrompt(no_login_file())));

    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
    let option = cx
        .debug_bounds("option-2")
        .expect("the second way is drawn");
    cx.simulate_click(option.center(), Modifiers::none());
    // The old key fails, and the agent asks again, telling why.
    cx.condition(&view, |view, _| {
        view.conversation().logging_in() && view.conversation().entries().len() > 2
    })
    .await;
    cx.simulate_keystrokes("1");
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(agent.logged_in(), [fake_agent::KEY]);
    assert_eq!(agent.prompts(), ["hello"]);
    let failed = logins(cx, &view)[1].0.clone().expect("the old key failed");
    assert!(failed.contains(fake_agent::WRONG), "{failed}");
    assert_eq!(
        logins(cx, &view),
        [(None, Some(1)), (Some(failed), Some(0))]
    );
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.conversation().entries().last(),
            Some(&Entry::Reply("echo: hello".to_owned()))
        );
    });
}

#[gpui::test]
async fn gives_up_logging_in_with_escape(cx: &mut TestAppContext) {
    let (agent, view, cx) =
        open_talking_to(cx, FakeAgent::logging_in(LogsIn::AtPrompt(no_login_file())));

    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
    cx.simulate_keystrokes("escape");
    wait_for(cx, &view, State::Idle).await;

    assert_eq!(agent.logged_in(), Vec::<String>::new());
    assert_eq!(logins(cx, &view), [(None, None)]);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.conversation().entries().last(),
            Some(&Entry::Note(
                "The turn failed: the human did not log in".to_owned()
            ))
        );
    });
}

#[gpui::test]
async fn giving_up_logging_in_before_the_session_opens_closes_the_agent(cx: &mut TestAppContext) {
    let (agent, view, cx) = open_talking_to(
        cx,
        FakeAgent::logging_in(LogsIn::AtSession(no_login_file())),
    );

    wait_for(cx, &view, State::Asking).await;
    cx.simulate_keystrokes("escape");
    wait_for(cx, &view, State::Closed).await;

    assert_eq!(agent.sessions_opened(), 0);
    view.read_with(cx, |view, _| {
        let Some(Entry::Note(note)) = view.conversation().entries().last() else {
            panic!("the conversation ends with a note");
        };
        assert!(note.contains("the human did not log in"), "{note}");
    });
}

#[gpui::test]
async fn a_chat_with_no_terminal_says_it_cannot_log_in_there(cx: &mut TestAppContext) {
    let (agent, view, cx) =
        open_talking_to(cx, FakeAgent::logging_in(LogsIn::AtPrompt(no_login_file())));

    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
    cx.simulate_keystrokes("3");
    // Not logged in, the agent asks again.
    cx.condition(&view, |view, _| {
        view.conversation().logging_in() && view.conversation().entries().len() > 3
    })
    .await;

    assert_eq!(agent.logged_in(), Vec::<String>::new());
    view.read_with(cx, |view, _| {
        assert!(view.conversation().entries().contains(&Entry::Note(
            "Cox cannot run the agent's command here to log in.".to_owned()
        )));
    });
}

/// A setup whose policy answers requests titled as `reject` says with no,
/// and as `allow` says with yes, with a context to keep to.
fn set_up(allow: &[&str], reject: &[&str]) -> Setup {
    Setup {
        context: Some("Change nothing.".to_owned()),
        policy: Policy {
            allow: allow.iter().map(|&s| s.to_owned()).collect(),
            reject: reject.iter().map(|&s| s.to_owned()).collect(),
            within: None,
        },
        ..Setup::default()
    }
}

#[gpui::test]
async fn the_setups_policy_answers_a_permission_request_in_the_humans_place(
    cx: &mut TestAppContext,
) {
    let (agent, view, cx) = open_set_up(cx, set_up(&[], &["Edit *"]));

    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;

    // The agent was told no, and heard nothing from the human.
    assert_eq!(
        agent.answers(),
        [RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new("reject")
        )]
    );
    // The note of the context, which the chat that opens one with a setup
    // makes, is the chats' tests'.
    view.read_with(cx, |view, _| {
        let entries = view.conversation().entries();
        assert!(
            matches!(
                &entries[2],
                Entry::Permission {
                    chosen: Some(1),
                    by_setup: true,
                    ..
                }
            ),
            "{entries:?}"
        );
    });
    // The context is told after the instructions, with the first line.
    assert_eq!(
        agent.prompt_blocks()[0],
        [
            "You were set up with this context for the whole session. Keep to it.\n\nChange nothing.\n".to_owned(),
            "permission".to_owned()
        ]
    );
}

#[gpui::test]
async fn a_request_the_policy_allows_goes_through_and_the_rest_wait_on_the_human(
    cx: &mut TestAppContext,
) {
    let (agent, view, cx) = open_set_up(cx, set_up(&["Edit x.txt"], &[]));

    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;
    assert_eq!(
        agent.answers(),
        [RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new("allow")
        )]
    );

    // A setup that says nothing of the request asks the human.
    let (_agent, view, cx) = open_set_up(cx, set_up(&["Read *"], &["Write *"]));
    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
}

#[gpui::test]
async fn a_policy_answers_by_the_kind_the_call_was_told_to_be(cx: &mut TestAppContext) {
    // The request leaves the kind out; the call told it before.
    let (agent, view, cx) = open_set_up(cx, set_up(&["edit:*"], &[]));
    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;
    assert_eq!(
        agent.answers(),
        [RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new("allow")
        )]
    );

    // Kept to the folder the file is in, it goes through too.
    let mut kept = set_up(&["edit:*"], &[]);
    kept.policy.within = Some("/work".into());
    let (agent, view, cx) = open_set_up(cx, kept);
    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;
    assert_eq!(
        agent.answers(),
        [RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new("allow")
        )]
    );

    // Kept to another folder, it is asked of the human.
    let mut elsewhere = set_up(&["edit:*"], &[]);
    elsewhere.policy.within = Some("/sketches/scratch-1".into());
    let (_agent, view, cx) = open_set_up(cx, elsewhere);
    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;

    // A pattern of another kind says nothing of it.
    let (_agent, view, cx) = open_set_up(cx, set_up(&["execute:*"], &[]));
    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;
}

#[gpui::test]
async fn shows_the_change_a_permission_request_asks_for(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Asking).await;

    let change = Content::Diff {
        path: "/work/x.txt".to_owned(),
        lines: vec![
            Line::Same("one".to_owned()),
            Line::Removed("two".to_owned()),
            Line::Added("2".to_owned()),
            Line::Same("three".to_owned()),
        ],
    };
    view.read_with(cx, |view, _| {
        let entries = view.conversation().entries();
        assert!(
            matches!(&entries[1], Entry::ToolCall { content, .. } if content.is_empty()),
            "the change shows once, with the request"
        );
        assert!(matches!(&entries[2], Entry::Permission { content, .. } if *content == [change]));
    });
}

#[gpui::test]
async fn shows_what_a_tool_call_printed(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("tool");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;

    view.read_with(cx, |view, _| {
        assert!(matches!(
            &view.conversation().entries()[1],
            Entry::ToolCall { content, result: Some(ToolCallResult::Completed), .. }
                if *content == [Content::Text(fake_agent::LS_OUTPUT.to_owned())]
        ));
    });
}

/// Clicks the middle of what `selector` names.
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
}

/// How far down the window the top of what `selector` names is, as the
/// last frame drew it. What a frame drew stays known after, so a test can
/// see where something is, but not that it has gone.
fn top(cx: &mut VisualTestContext, selector: &'static str) -> Pixels {
    cx.run_until_parked();
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .top()
}

#[gpui::test]
async fn what_a_tool_call_printed_opens_with_a_click(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("tool");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;
    let closed = top(cx, "reply-2");
    assert!(cx.debug_bounds("content-1").is_none());

    // The call shows as a step, whose chip opens its line.
    click(cx, "step-1");
    assert!(top(cx, "reply-2") > closed);
    let line = top(cx, "reply-2");
    click(cx, "line-1");
    assert!(cx.debug_bounds("content-1").is_some());
    assert!(top(cx, "reply-2") > line);
    click(cx, "line-1");
    assert_eq!(top(cx, "reply-2"), line);
    click(cx, "step-1");
    assert_eq!(top(cx, "reply-2"), closed);
}

#[gpui::test]
async fn tool_calls_in_a_row_fold_into_one_line(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("tool twice");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;
    let folded = top(cx, "reply-3");
    assert!(folded > top(cx, "work-1"));
    // Each call is a step, side by side.
    assert_eq!(top(cx, "step-1"), top(cx, "step-2"));
    assert!(top(cx, "step-1") > top(cx, "work-1"));

    click(cx, "work-1");
    assert!(top(cx, "line-1") > top(cx, "work-1"));
    assert!(top(cx, "line-2") > top(cx, "line-1"));
    assert!(top(cx, "reply-3") > top(cx, "line-2"));
    assert!(top(cx, "reply-3") > folded);
    click(cx, "line-2");
    assert!(cx.debug_bounds("content-2").is_some());
    click(cx, "work-1");
    assert_eq!(top(cx, "reply-3"), folded);
}

#[gpui::test]
async fn what_the_agent_thought_folds_with_its_tool_calls(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("think, then run a tool");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.conversation().entries()[1],
            Entry::Thought(fake_agent::THOUGHT.to_owned())
        );
    });
    let folded = top(cx, "reply-3");

    click(cx, "work-1");
    click(cx, "line-1");
    assert!(top(cx, "reply-3") > folded);
}

#[gpui::test]
async fn the_plan_stays_in_sight_until_the_next_line(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.simulate_input("plan, and wait");
    cx.simulate_keystrokes("enter");
    cx.condition(&view, |view, _| view.conversation().last_plan().is_some())
        .await;
    // On a card over the top of the conversation, at its right.
    let card = cx.debug_bounds("plan").expect("the plan is drawn");
    let conversation = cx.debug_bounds("reply-2").expect("the reply is drawn");
    assert!(card.top() < conversation.bottom());
    assert!(card.right() > conversation.center().x);

    // What the turn did stays in sight once it ends.
    cx.simulate_keystrokes("escape");
    wait_for(cx, &view, State::Idle).await;
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.conversation().last_plan(),
            Some(steps) if steps.len() == 2
        ));
    });
    // A click folds it to its line.
    let height = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        cx.debug_bounds("plan")
            .expect("the plan is drawn")
            .size
            .height
    };
    let open = height(cx);
    click(cx, "plan-line");
    assert!(height(cx) < open);

    cx.simulate_input("again");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Idle).await;
    view.read_with(cx, |view, _| {
        assert_eq!(view.conversation().last_plan(), None);
    });
}

/// The title and result of the first tool call in the conversation.
fn tool_call(
    cx: &mut VisualTestContext,
    view: &Entity<ChatView>,
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

/// Sends `line` and waits for the turn to end.
async fn say(cx: &mut VisualTestContext, view: &Entity<ChatView>, line: &str) {
    cx.simulate_input(line);
    cx.simulate_keystrokes("enter");
    wait_for(cx, view, State::Idle).await;
}

/// Where the reply at entry 1 is drawn.
fn first_reply(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds("reply-1").expect("the reply is drawn")
}

/// How wide a character is in the tests, whose text is laid out as if
/// every character were as wide as the font is tall times 0.6.
const CHARACTER: f32 = 14. * 0.6;

#[gpui::test]
async fn shows_a_reply_as_markdown_and_copies_what_the_human_selects(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);
    say(cx, &view, "**bold** text").await;

    let reply = first_reply(cx);
    cx.simulate_mouse_down(reply.origin, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(reply.bottom_right(), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(reply.bottom_right(), MouseButton::Left, Modifiers::none());
    cx.simulate_keystrokes("secondary-c");

    assert_eq!(clipboard(cx), Some("echo: bold text".to_owned()));
    // The keyboard is still the input's.
    cx.simulate_input("x");
    assert_eq!(input_text(cx, &view), "x");
}

#[gpui::test]
async fn selects_a_word_with_a_double_click(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);
    say(cx, &view, "hello world").await;

    let reply = first_reply(cx);
    // On the "o" of "world": "echo: hello w" comes before it.
    let world = point(
        reply.origin.x + px(CHARACTER * 13.5),
        reply.origin.y + px(5.),
    );
    for click_count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            position: world,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position: world,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count,
        });
    }

    assert_eq!(view.read_with(cx, |view, _| view.selected_text()), "world");
}

#[gpui::test]
async fn opens_a_link_the_human_clicks(cx: &mut TestAppContext) {
    let opened = Opened::default();
    let (_agent, view, cx) = open_with_stage(cx, Vec::new(), Rc::clone(&opened), None, true);
    say(cx, &view, "see https://example.com/x").await;

    let reply = first_reply(cx);
    let at = |column: f32| {
        point(
            reply.origin.x + px(CHARACTER * column),
            reply.origin.y + px(5.),
        )
    };
    // On "echo", then on the address, which starts at the eleventh
    // character.
    cx.simulate_click(at(1.5), Modifiers::none());
    cx.simulate_click(at(15.5), Modifiers::none());
    // Dragging over the address selects it instead.
    cx.simulate_mouse_down(at(12.5), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(at(20.5), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(at(20.5), MouseButton::Left, Modifiers::none());

    assert_eq!(*opened.borrow(), ["https://example.com/x"]);
}

fn stage_in_sight(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> bool {
    view.read_with(cx, ChatView::stage_in_sight)
}

/// Clicks what `selector` names.
fn click_on(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
}

#[gpui::test]
fn folds_the_chat_beside_the_stage_and_unfolds_it(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open_with_stage(cx, Vec::new(), Opened::default(), None, true);
    cx.run_until_parked();

    // The stage is in the middle of the window, the chat at its right.
    assert!(stage_in_sight(cx, &view));
    assert!(cx.debug_bounds("stage-panel").is_some());
    assert!(cx.debug_bounds("chat-panel").is_some());

    let folded = |cx: &mut VisualTestContext| {
        let workspace = cx.update(|window, _| window.root::<Workspace>().flatten());
        workspace
            .expect("the window shows the chats")
            .read_with(cx, |workspace, _| workspace.is_folded())
    };
    click_on(cx, "fold-chat");
    assert!(folded(cx));
    assert!(cx.debug_bounds("chat-dot-1").is_some());
    assert!(stage_in_sight(cx, &view));

    click_on(cx, "unfold-chat");
    assert!(!folded(cx));
    // The keyboard is back in the input.
    cx.simulate_input("x");
    assert_eq!(input_text(cx, &view), "x");

    // The key folds it too, and a click on the chat's dot shows it.
    cx.simulate_keystrokes("secondary-\\");
    assert!(folded(cx));
    click_on(cx, "chat-dot-1");
    assert!(!folded(cx));
}

#[gpui::test]
async fn opens_a_link_the_human_clicks_on_the_stage(cx: &mut TestAppContext) {
    let opened = Opened::default();
    let (_agent, view, cx) = open_with_stage(cx, Vec::new(), Rc::clone(&opened), None, true);
    say(cx, &view, "see https://example.com/x").await;

    let reply = first_reply(cx);
    cx.simulate_click(
        point(
            reply.origin.x + px(CHARACTER * 15.5),
            reply.origin.y + px(5.),
        ),
        Modifiers::none(),
    );

    assert!(stage_in_sight(cx, &view));
    assert_eq!(*opened.borrow(), ["https://example.com/x"]);
}

#[gpui::test]
fn a_chat_with_no_stage_shows_none(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    assert!(!stage_in_sight(cx, &view));
    assert!(cx.debug_bounds("stage-panel").is_none());
}

/// Waits until the chat's session has the setting `id` set to `value`.
async fn wait_for_setting(
    cx: &mut VisualTestContext,
    view: &Entity<ChatView>,
    id: &'static str,
    value: &'static str,
) {
    let (_waiting, timeout) = given_up();
    let set = cx.condition(view, move |view, _| {
        view.conversation()
            .settings()
            .iter()
            .any(|setting| setting.id == id && setting.value == value)
    });
    tokio::select! {
        () = set => {}
        _ = timeout => panic!("{id} was never set to {value}"),
    }
}

#[gpui::test]
async fn sets_a_setting_of_the_session_with_a_click(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);
    wait_for_setting(cx, &view, fake_agent::MODEL, "large").await;

    click(cx, "settings");
    click(cx, "setting-0");
    assert!(cx.debug_bounds("setting-choices").is_some());
    click(cx, "choice-1");
    wait_for_setting(cx, &view, fake_agent::MODEL, "small").await;

    assert_eq!(
        agent.settings_set(),
        [(fake_agent::MODEL.to_owned(), "small".to_owned())]
    );
    // The mode stays as it was.
    wait_for_setting(cx, &view, fake_agent::MODE, "ask").await;
}

#[gpui::test]
async fn sets_a_setting_while_a_turn_runs(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);
    cx.simulate_input("wait");
    cx.simulate_keystrokes("enter");
    wait_for(cx, &view, State::Busy).await;

    click(cx, "settings");
    click(cx, "setting-1");
    click(cx, "choice-1");
    wait_for_setting(cx, &view, fake_agent::MODE, "plan").await;

    assert_eq!(
        agent.settings_set(),
        [(fake_agent::MODE.to_owned(), "plan".to_owned())]
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.conversation().state()),
        State::Busy
    );
}

#[gpui::test]
async fn shows_a_setting_the_agent_changes_on_its_own(cx: &mut TestAppContext) {
    let (agent, view, cx) = open(cx);
    say(cx, &view, "switch").await;

    wait_for_setting(cx, &view, fake_agent::MODE, "plan").await;
    assert_eq!(agent.settings_set(), []);
}

#[gpui::test]
async fn the_files_chip_starts_a_path_at_the_cursor(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    // The chips settle once the agent has told its commands, whose chip
    // comes ahead of the files'.
    cx.condition(&view, |view, _| !view.conversation().commands().is_empty())
        .await;
    cx.run_until_parked();
    click(cx, "start-file");
    assert_eq!(input_text(cx, &view), "@");
    cx.simulate_input("src ");
    // After a word, the path starts a word of its own.
    cx.simulate_input("and");
    click(cx, "start-file");
    assert_eq!(input_text(cx, &view), "@src and @");
}

/// A page's address line, on its own in a window, showing the page's
/// address, without the keyboard, and what it asked of the page.
fn address_line(
    cx: &mut TestAppContext,
) -> (
    Entity<AddressLine>,
    Rc<std::cell::RefCell<Vec<Asked>>>,
    &mut VisualTestContext,
) {
    address_line_over(cx, History::default())
}

/// The address line, as [`address_line`] makes it, over `history`, the
/// pages visited.
fn address_line_over(
    cx: &mut TestAppContext,
    history: History,
) -> (
    Entity<AddressLine>,
    Rc<std::cell::RefCell<Vec<Asked>>>,
    &mut VisualTestContext,
) {
    cx.update(cox::ui::input::bind_keys);
    cx.update(cox::stage::page::address::bind_keys);
    cx.update(|cx| cox::stage::page::history::keep(history, None, cx));
    let (line, cx) = cx.add_window_view(|_, cx| AddressLine::new("https://example.com/", cx));
    let asked: Rc<std::cell::RefCell<Vec<Asked>>> = Rc::default();
    let keep = Rc::clone(&asked);
    cx.update(|_, cx| {
        cx.subscribe(&line, move |_, asked: &Asked, _| {
            keep.borrow_mut().push(asked.clone());
        })
        .detach();
    });
    cx.run_until_parked();
    (line, asked, cx)
}

fn address_text(cx: &mut VisualTestContext, line: &Entity<AddressLine>) -> String {
    cx.update(|_, cx| line.read(cx).text(cx).to_owned())
}

fn address_said(cx: &mut VisualTestContext, line: &Entity<AddressLine>) -> Option<String> {
    cx.update(|window, cx| line.read(cx).said(window, cx))
}

/// The first click on the address line selects its whole text, so that
/// typing replaces it; a click while it has the keyboard puts the cursor
/// there. A line under it says what Enter does.
#[gpui::test]
fn typing_in_the_address_line_replaces_the_address_and_says_what_enter_does(
    cx: &mut TestAppContext,
) {
    let (line, asked, cx) = address_line(cx);
    assert_eq!(address_said(cx, &line), None);

    click_on(cx, "address");
    cx.run_until_parked();
    assert_eq!(address_said(cx, &line), None);
    cx.simulate_input("rust gpui");
    assert_eq!(address_text(cx, &line), "rust gpui");
    assert_eq!(
        address_said(cx, &line).as_deref(),
        Some("Search google.com for rust gpui")
    );
    assert!(cx.debug_bounds("address-says").is_some());

    click_on(cx, "address");
    cx.run_until_parked();
    cx.simulate_input("!");
    let text = address_text(cx, &line);
    assert_eq!(text.len(), "rust gpui!".len(), "{text}");
    assert_ne!(text, "!");

    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("example.com/a b");
    assert_eq!(
        address_said(cx, &line).as_deref(),
        Some("Go to https://example.com/a%20b")
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(
        *asked.borrow(),
        [Asked::Go("https://example.com/a%20b".to_owned())]
    );
}

/// Escape puts the page's address back and gives the keyboard back to the
/// page; the keys that bring the keyboard to the line select its text.
#[gpui::test]
fn escape_in_the_address_line_puts_the_page_s_address_back(cx: &mut TestAppContext) {
    let (line, asked, cx) = address_line(cx);
    cx.update(|window, cx| line.update(cx, |line, cx| line.focus(window, cx)));
    cx.run_until_parked();
    cx.simulate_input("gpui");
    assert_eq!(address_text(cx, &line), "gpui");

    cx.simulate_keystrokes("escape");
    assert_eq!(address_text(cx, &line), "https://example.com/");
    assert_eq!(*asked.borrow(), [Asked::Back]);
    assert_eq!(address_said(cx, &line), None);
}

/// While the human types in the address line, the page going elsewhere
/// leaves what they typed; once the line has lost the keyboard, it shows
/// where the page went.
#[gpui::test]
fn the_address_line_shows_where_the_page_went_unless_it_is_typed_in(cx: &mut TestAppContext) {
    let (line, _, cx) = address_line(cx);
    let navigated = |cx: &mut VisualTestContext, url: &'static str| {
        cx.update(|window, cx| line.update(cx, |line, cx| line.navigated(url, window, cx)));
    };
    cx.update(|window, cx| line.update(cx, |line, cx| line.focus(window, cx)));
    cx.run_until_parked();
    cx.simulate_input("gpui");
    navigated(cx, "https://example.com/next");
    assert_eq!(address_text(cx, &line), "gpui");

    let elsewhere = cx.update(|_, cx| cx.focus_handle());
    cx.update(|window, _| window.focus(&elsewhere));
    cx.run_until_parked();
    navigated(cx, "https://example.com/last");
    assert_eq!(address_text(cx, &line), "https://example.com/last");
}

/// Pages visited: GitHub's home twice, Zed's repository on it once, and
/// gpui's docs, titled with a word GitHub's address starts with.
fn visited() -> History {
    let now = history::now();
    let mut history = History::default();
    history.visit("https://github.com/", "GitHub", now);
    history.visit("https://github.com/", "GitHub", now);
    history.visit("https://github.com/zed-industries/zed", "Zed", now);
    history.visit("https://docs.rs/gpui", "gpui on GitHub", now);
    history
}

fn offered(cx: &mut VisualTestContext, line: &Entity<AddressLine>) -> Vec<String> {
    cx.update(|window, cx| line.read(cx).offered(window, cx))
}

fn typing_in(
    cx: &mut TestAppContext,
) -> (
    Entity<AddressLine>,
    Rc<std::cell::RefCell<Vec<Asked>>>,
    &mut VisualTestContext,
) {
    let (line, asked, cx) = address_line_over(cx, visited());
    cx.update(|window, cx| line.update(cx, |line, cx| line.focus(window, cx)));
    cx.run_until_parked();
    (line, asked, cx)
}

/// Typed at the end of the line, the start of an address visited is
/// filled in, and the list under the line offers the other pages that fit
/// after what Enter does; Up, Down, Control-N and Control-P choose the
/// row Enter opens.
#[gpui::test]
fn the_address_line_offers_the_pages_visited_that_fit(cx: &mut TestAppContext) {
    let (line, asked, cx) = typing_in(cx);
    cx.simulate_input("gi");
    assert_eq!(address_text(cx, &line), "github.com/");
    assert_eq!(
        address_said(cx, &line).as_deref(),
        Some("Go to https://github.com/")
    );
    assert_eq!(
        offered(cx, &line),
        [
            "https://github.com/zed-industries/zed",
            "https://docs.rs/gpui"
        ]
    );
    assert!(cx.debug_bounds("address-page-1").is_some());

    let chosen = |cx: &mut VisualTestContext| cx.update(|_, cx| line.read(cx).chosen());
    cx.simulate_keystrokes("down");
    assert_eq!(chosen(cx), Row::Page(0));
    cx.simulate_keystrokes("ctrl-n");
    assert_eq!(chosen(cx), Row::Page(1));
    cx.simulate_keystrokes("ctrl-n");
    assert_eq!(chosen(cx), Row::Line, "round to the first");
    cx.simulate_keystrokes("up");
    assert_eq!(chosen(cx), Row::Page(1), "round to the last");
    cx.simulate_keystrokes("ctrl-p");
    assert_eq!(chosen(cx), Row::Page(0));
    assert_eq!(address_text(cx, &line), "github.com/");

    cx.simulate_keystrokes("enter");
    assert_eq!(
        *asked.borrow(),
        [Asked::Go(
            "https://github.com/zed-industries/zed".to_owned()
        )]
    );
}

/// What is filled in is selected: typing goes on over it, Backspace takes
/// it out and it stays out, Right or End keeps it, and Enter opens the
/// page it was filled in from.
#[gpui::test]
fn the_address_filled_in_gives_way_to_typing(cx: &mut TestAppContext) {
    let (line, asked, cx) = typing_in(cx);
    cx.simulate_input("gi");
    assert_eq!(address_text(cx, &line), "github.com/");
    cx.simulate_keystrokes("backspace");
    assert_eq!(address_text(cx, &line), "gi");
    assert_eq!(
        address_said(cx, &line).as_deref(),
        Some("Search google.com for gi")
    );
    cx.simulate_keystrokes("backspace");
    assert_eq!(address_text(cx, &line), "g");

    cx.simulate_input("it");
    assert_eq!(address_text(cx, &line), "github.com/");
    cx.simulate_input("x");
    assert_eq!(
        address_text(cx, &line),
        "gitx",
        "typed over what was filled in"
    );
    assert_eq!(offered(cx, &line), Vec::<String>::new());

    for _ in 0..4 {
        cx.simulate_keystrokes("backspace");
    }
    cx.simulate_input("github.com/z");
    assert_eq!(address_text(cx, &line), "github.com/zed-industries/zed");
    cx.simulate_keystrokes("left");
    cx.simulate_input("!");
    assert_eq!(
        address_text(cx, &line),
        "github.com/z!ed-industries/zed",
        "Left goes to the start of what was filled in"
    );

    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("do");
    assert_eq!(address_text(cx, &line), "docs.rs/gpui");
    cx.simulate_keystrokes("end");
    cx.simulate_input("x");
    assert_eq!(address_text(cx, &line), "docs.rs/gpuix", "End keeps it");

    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("gi");
    cx.simulate_keystrokes("right enter");
    assert_eq!(
        *asked.borrow(),
        [Asked::Go("https://github.com/".to_owned())]
    );
}

/// Escape, with the list open, puts the page's address back and closes
/// the list; a click on a row opens its page.
#[gpui::test]
fn escape_closes_the_offer_and_a_click_opens_a_page(cx: &mut TestAppContext) {
    let (line, asked, cx) = typing_in(cx);
    cx.simulate_input("gi");
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("escape");
    assert_eq!(address_text(cx, &line), "https://example.com/");
    assert_eq!(offered(cx, &line), Vec::<String>::new());
    assert_eq!(address_said(cx, &line), None);
    assert_eq!(*asked.borrow(), [Asked::Back]);

    cx.update(|window, cx| line.update(cx, |line, cx| line.focus(window, cx)));
    cx.run_until_parked();
    cx.simulate_input("gi");
    assert_eq!(cx.update(|_, cx| line.read(cx).chosen()), Row::Line);
    cx.run_until_parked();
    click_on(cx, "address-page-1");
    assert_eq!(
        *asked.borrow(),
        [Asked::Back, Asked::Go("https://docs.rs/gpui".to_owned())]
    );
}
