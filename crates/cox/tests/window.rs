//! A human using the window, with the agent faked in memory.

#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    McpServer as AcpMcpServer, McpServerStdio, RequestPermissionOutcome, SelectedPermissionOutcome,
};
use cox::agent::setup::{Policy, Setup};
use cox::chat::conversation::{Content, Entry, State, ToolCallResult};
use cox::chat::view::{ChatView, StageView};
use cox::ui::Panel;
use cox::ui::input::LineInput;
use cox_acp::McpServer;
use cox_core::diff::Line;
use fake_agent::FakeAgent;
use gpui::{
    AppContext as _, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, Focusable,
    IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement as _, Pixels,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, div, point, px,
};

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

/// The links the human opened, in order.
type Opened = Rc<RefCell<Vec<String>>>;

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

/// A stand-in for the stage, which needs CEF.
struct FakeStage;

impl Render for FakeStage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child("stage")
    }
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
    // The agent side runs on a thread of its own, so the test waits for it
    // in real time instead of only running GPUI's own tasks.
    cx.executor().allow_parking();
    cx.update(|cx| {
        cox::ui::input::bind_keys(cx);
        cox::chat::view::bind_keys(cx);
    });
    let agent = Arc::new(FakeAgent::default());
    let fake = Arc::clone(&agent);
    let (view, cx) = cx.add_window_view(|_, cx| {
        let stage = stage.then(|| StageView {
            view: cx.new(|_| FakeStage).into(),
            panel: cx.new(|_| Panel::default()),
        });
        ChatView::new(
            cox::agent::spawn(
                move || Ok(fake.connect()),
                std::env::temp_dir(),
                mcp_servers,
                cox::agent::Start::New {
                    instructions: None,
                    setup,
                },
            ),
            stage,
            Rc::new(move |url, _, _| opened.borrow_mut().push(url.to_owned())),
            None,
            cx,
        )
    });
    cx.update(|window, cx| window.focus(&view.focus_handle(cx)));
    (agent, view, cx)
}

/// Waits until the conversation is in `state`. The agent side runs in real
/// time, outside GPUI's clock, so a real timer ends the wait if it never
/// comes.
async fn wait_for(cx: &mut VisualTestContext, view: &Entity<ChatView>, state: State) {
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
    assert_eq!(agent.cwd(), Some(std::env::temp_dir()));
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
fn keeps_pasted_line_breaks(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open(cx);

    cx.write_to_clipboard(ClipboardItem::new_string("one\ntwo\r\nthree".to_owned()));
    cx.simulate_keystrokes("secondary-v");

    assert_eq!(input_text(cx, &view), "one\ntwo\nthree");
}

/// The address line holds one line.
#[gpui::test]
fn an_input_of_one_line_pastes_line_breaks_as_spaces(cx: &mut TestAppContext) {
    cx.update(cox::ui::input::bind_keys);
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
    assert!(cx.debug_bounds("waiting").is_some());
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

/// A setup whose policy answers requests titled as `reject` says with no,
/// and as `allow` says with yes, with a context to keep to.
fn set_up(allow: &[&str], reject: &[&str]) -> Setup {
    Setup {
        context: Some("Change nothing.".to_owned()),
        policy: Policy {
            allow: allow.iter().map(|&s| s.to_owned()).collect(),
            reject: reject.iter().map(|&s| s.to_owned()).collect(),
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
    view.read_with(cx, |view, _| {
        let entries = view.conversation().entries();
        assert!(
            matches!(&entries[0], Entry::Note(note) if note == "Context: Change nothing."),
            "{entries:?}"
        );
        assert!(
            matches!(
                &entries[3],
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
            "The chat that opened this one set it up with this context. Keep to it for the whole session.\n\nChange nothing.\n".to_owned(),
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

    click(cx, "line-1");
    assert!(cx.debug_bounds("content-1").is_some());
    assert!(top(cx, "reply-2") > closed);
    click(cx, "line-1");
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
    let (_agent, view, cx) = open_with(cx, Vec::new(), Rc::clone(&opened), None);
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

fn stage_is_open(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> bool {
    view.read_with(cx, ChatView::stage_is_open)
}

#[gpui::test]
fn opens_and_closes_the_stage_with_its_button(cx: &mut TestAppContext) {
    let (_agent, view, cx) = open_with_stage(cx, Vec::new(), Opened::default(), None, true);

    assert!(!stage_is_open(cx, &view));
    assert!(cx.debug_bounds("stage-panel").is_none());
    let toggle = cx
        .debug_bounds("toggle-stage")
        .expect("the button is drawn");
    cx.simulate_click(toggle.center(), Modifiers::none());
    assert!(stage_is_open(cx, &view));
    assert!(cx.debug_bounds("stage-panel").is_some());

    let toggle = cx
        .debug_bounds("toggle-stage")
        .expect("the button is drawn");
    cx.simulate_click(toggle.center(), Modifiers::none());
    assert!(!stage_is_open(cx, &view));
    // The keyboard is back in the input.
    cx.simulate_input("x");
    assert_eq!(input_text(cx, &view), "x");
}

#[gpui::test]
async fn opens_the_stage_for_a_link_the_human_clicks(cx: &mut TestAppContext) {
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

    assert!(stage_is_open(cx, &view));
    assert_eq!(*opened.borrow(), ["https://example.com/x"]);
}

#[gpui::test]
fn a_chat_with_no_stage_has_no_button_for_it(cx: &mut TestAppContext) {
    let (_agent, _view, cx) = open(cx);

    assert!(cx.debug_bounds("toggle-stage").is_none());
}
