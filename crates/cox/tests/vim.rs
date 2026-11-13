#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;
#[expect(dead_code, reason = "these tests use only part of the fake daemon")]
mod fake_daemon;
mod fake_lsp;
mod git;
mod keys;
mod scratch;
mod waits;

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;

use cox::chat::conversation::{Entry, State};
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
use git::git;

/// Opens the window with Vim's keys on, as the app does when the
/// settings say so.
fn open_window<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    cx.update(|cx| cox::vim::turn(true, cx));
    keys::open_window(cx, scratch::folder(name))
}

/// A GPUI debug selector only records where the line drew, not its
/// text, so the text is read from what the line draws it from.
fn status(cx: &mut VisualTestContext) -> Option<cox::vim::Status> {
    cx.debug_bounds("vim-status")?;
    cx.update(|_, cx| cox::vim::status(cx))
}

fn shown_mode(cx: &mut VisualTestContext) -> Option<&'static str> {
    status(cx).map(|status| status.mode.label())
}

fn held(cx: &mut VisualTestContext) -> String {
    status(cx).expect("the status line is drawn").held
}

#[gpui::test]
fn the_window_s_foot_shows_the_mode_as_the_keys_change_it(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "status");
    keys::press(cx, &workspace, "secondary-n");
    assert_eq!(shown_mode(cx), Some("Insert"));
    let foot = cx
        .debug_bounds("vim-status")
        .expect("the status line is drawn");
    let window = cx.update(|window, _| window.viewport_size());
    assert_eq!(foot.origin.x, px(0.));
    assert_eq!(foot.size.width, window.width);
    assert_eq!(foot.bottom(), window.height);
    assert!(
        cx.debug_bounds("vim-mode").is_none(),
        "no mode beside the chat's input"
    );

    keys::press(cx, &workspace, "escape");
    assert_eq!(shown_mode(cx), Some("Normal"));
    keys::press(cx, &workspace, "f");
    assert_eq!(shown_mode(cx), Some("Hint"));
    keys::press(cx, &workspace, "escape");
    assert_eq!(shown_mode(cx), Some("Normal"));

    keys::press(cx, &workspace, ":");
    assert_eq!(shown_mode(cx), Some("Command"));
    let line = cx.debug_bounds("vim-line").expect("the line is drawn");
    assert_eq!(line, foot, "the line takes the status line's place");
    keys::press(cx, &workspace, "escape");
    assert_eq!(shown_mode(cx), Some("Normal"));
}

#[gpui::test]
fn the_input_shows_its_caret_only_while_keys_type(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "caret");
    keys::press(cx, &workspace, "secondary-n");
    assert!(cx.update(|_, cx| cox::vim::types(cx)), "insert mode types");
    keys::press(cx, &workspace, "escape");
    assert!(
        !cx.update(|_, cx| cox::vim::types(cx)),
        "normal mode does not"
    );
    keys::press(cx, &workspace, ":");
    assert!(cx.update(|_, cx| cox::vim::types(cx)), "the line types");
    keys::press(cx, &workspace, "escape i");
    assert!(
        cx.update(|_, cx| cox::vim::types(cx)),
        "i goes back to typing"
    );
    cx.update(|_, cx| cox::vim::turn(false, cx));
    assert!(
        cx.update(|_, cx| cox::vim::types(cx)),
        "without Vim, keys type"
    );
}

#[gpui::test]
fn the_window_s_foot_shows_what_is_held_until_the_command_runs(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "held");
    keys::press(cx, &workspace, "secondary-n escape");
    assert_eq!(held(cx), "");
    keys::press(cx, &workspace, "3");
    assert_eq!(held(cx), "3");
    keys::press(cx, &workspace, "g");
    assert_eq!(held(cx), "3g");
    keys::press(cx, &workspace, "g");
    assert_eq!(held(cx), "");
    keys::press(cx, &workspace, "ctrl-w");
    assert_eq!(held(cx), "ctrl-w");
    keys::press(cx, &workspace, "l");
    assert_eq!(held(cx), "");
    keys::press(cx, &workspace, "1 2 escape");
    assert_eq!(held(cx), "", "escape forgets the count");
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

/// Waits, settling the daemon again and again, until `done` holds.
fn settle_until(cx: &mut VisualTestContext, done: impl Fn(&mut VisualTestContext) -> bool) {
    let executor = cx.executor();
    executor.block_test(fake_daemon::settle_until(cx, done));
}

/// Waits until the agent of the chat `view` has ended its turn with a
/// reply.
fn wait_for_reply(cx: &mut VisualTestContext, view: &Entity<ChatView>) {
    settle_until(cx, |cx| {
        view.read_with(cx, |view, _| {
            let conversation = view.conversation();
            conversation.state() == State::Idle
                && matches!(conversation.entries().last(), Some(Entry::Reply(_)))
        })
    });
}

/// Waits until the agent of the chat `view` waits in its turn, to be
/// stopped, having said so.
fn wait_for_waiting(cx: &mut VisualTestContext, view: &Entity<ChatView>) {
    settle_until(cx, |cx| {
        view.read_with(cx, |view, _| {
            view.conversation().entries().last() == Some(&Entry::Reply("waiting".to_owned()))
        })
    });
}

/// Waits until the turn of the chat `view` has stopped.
fn wait_for_stop(cx: &mut VisualTestContext, view: &Entity<ChatView>) {
    settle_until(cx, |cx| agent_state(cx, view) == State::Idle);
}

fn agent_state(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> State {
    view.read_with(cx, |view, _| view.conversation().state())
}

#[gpui::test]
fn a_new_chat_types_at_once_and_normal_mode_types_nothing(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "new");

    keys::press(cx, &workspace, "secondary-n h e l l o");
    assert_eq!(keys::shown_input(cx, &workspace), "hello");
    assert_eq!(shown_mode(cx), Some("Insert"));

    keys::press(cx, &workspace, "escape j k z space enter backspace");
    assert_eq!(keys::shown_input(cx, &workspace), "hello");
    assert_eq!(shown_mode(cx), Some("Normal"));
}

#[gpui::test]
fn i_a_and_their_capitals_type_where_vim_does(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "places");
    keys::press(cx, &workspace, "secondary-n h e l l o escape");

    keys::press(cx, &workspace, "shift-i x escape");
    assert_eq!(keys::shown_input(cx, &workspace), "xhello");
    keys::press(cx, &workspace, "shift-a y escape");
    assert_eq!(keys::shown_input(cx, &workspace), "xhelloy");
    // Escape left the cursor on the y, as Vim's, so a goes in after the
    // l two to its left.
    keys::press(cx, &workspace, "left left a z escape");
    assert_eq!(keys::shown_input(cx, &workspace), "xhellzoy");
    keys::press(cx, &workspace, "left i w");
    assert_eq!(keys::shown_input(cx, &workspace), "xhelwlzoy");
    assert_eq!(shown_mode(cx), Some("Insert"));
}

#[gpui::test]
fn escape_never_stops_the_agent_and_control_c_in_normal_mode_does(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "stop");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    cx.simulate_input("wait");
    keys::press(cx, &workspace, "enter");
    wait_for_waiting(cx, &view);
    assert_eq!(agent_state(cx, &view), State::Busy);

    // A stop is a state of the chat's at once, which the daemon has once
    // the keys are pressed.
    keys::press(cx, &workspace, "escape");
    assert_eq!(shown_mode(cx), Some("Normal"));
    assert_eq!(agent_state(cx, &view), State::Busy, "escape in insert mode");

    keys::press(cx, &workspace, "escape");
    assert_eq!(agent_state(cx, &view), State::Busy, "escape in normal mode");

    keys::press(cx, &workspace, "ctrl-c");
    wait_for_stop(cx, &view);
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
    settle_until(cx, |cx| asked(cx) > 0);
    let options = asked(cx);
    assert!(options > 0, "the agent asks permission");

    keys::press(cx, &workspace, "escape 1");
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
    let folder = scratch::folder(name);
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
    assert_eq!(shown_mode(cx), Some("Normal"));

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
    assert_eq!(shown_mode(cx), Some("Insert"));
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
    assert_eq!(shown_mode(cx), Some("Normal"));
    assert_eq!(keys::shown_input(cx, &workspace), "");
}

#[gpui::test]
fn the_list_of_chats_shows_one_with_the_keys(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "list");
    keys::press(cx, &workspace, "secondary-n escape secondary-n escape");
    let second = shown_view(cx, &workspace);

    keys::press(cx, &workspace, "ctrl-w k ctrl-w k");
    assert!(in_part(cx, "Chats"));
    assert!(workspace.read_with(cx, |workspace, _| workspace.is_switching()));
    keys::press(cx, &workspace, "escape");
    assert!(in_part(cx, "Conversation"));
    assert!(!workspace.read_with(cx, |workspace, _| workspace.is_switching()));

    keys::press(cx, &workspace, "ctrl-w k ctrl-w k g g enter");
    let first = shown_view(cx, &workspace);
    assert!(in_part(cx, "Conversation"));
    keys::press(cx, &workspace, "ctrl-w k ctrl-w k shift-g l");
    let last = shown_view(cx, &workspace);
    assert_ne!(first, last);
    keys::press(cx, &workspace, "ctrl-w k ctrl-w k k enter");
    assert_eq!(shown_view(cx, &workspace), first);
    assert!([first, last].contains(&second));

    keys::press(cx, &workspace, "ctrl-w k ctrl-w k o");
    let chats = workspace.read_with(cx, |workspace, _| workspace.chats(0).count());
    assert_eq!(chats, 3);
    assert_eq!(shown_mode(cx), Some("Insert"));
}

#[gpui::test]
fn the_keys_of_the_stage_s_tabs_go_to_the_stage(cx: &mut TestAppContext) {
    let keyed = Keyed::default();
    let stages = FakeStages {
        keyed: Some(keyed.clone()),
        ..FakeStages::default()
    };
    cx.update(|cx| cox::vim::turn(true, cx));
    let (workspace, cx) = keys::open_window_on(cx, scratch::folder("tabs"), stages);
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
    wait_for_reply(cx, &view);
    settle_until(cx, |cx| {
        view.read_with(cx, |view, _| !view.conversation().commands().is_empty())
    });

    cx.simulate_input("/co");
    keys::press(cx, &workspace, "escape i enter");
    assert_eq!(
        keys::shown_input(cx, &workspace),
        "",
        "the line went as written"
    );
    wait_for_reply(cx, &view);

    cx.simulate_input("/co");
    keys::press(cx, &workspace, "enter");
    assert!(
        keys::shown_input(cx, &workspace).starts_with("/compact"),
        "with the list open, Enter takes its command"
    );
}

/// Starts `command` in a terminal of its own window, with the keyboard
/// and every key Cox binds, which answers once the command ends.
fn open_terminal<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
    command: &str,
) -> (
    Entity<Terminal>,
    tokio::sync::oneshot::Receiver<Result<cox::agent::mcp::Ran, String>>,
    &'a mut VisualTestContext,
) {
    cx.update(|cx| cox::vim::turn(true, cx));
    cx.update(cox::app::bind_keys);
    let cwd = scratch::folder(name);
    std::fs::create_dir_all(&cwd).unwrap();
    let running = Running::start(Program::Command(command.to_owned()), &cwd).unwrap();
    let (answer, waiting) = tokio::sync::oneshot::channel();
    let (terminal, cx) = cx.add_window_view(|_, cx| Terminal::new(running, Some(answer), cx));
    cx.update(|window, cx| {
        window.activate_window();
        window.focus(&terminal.focus_handle(cx));
        cox::vim::typing(cox::vim::Region::Terminal, window, cx);
    });
    cx.run_until_parked();
    (terminal, waiting, cx)
}

/// What the command printed, once it ended.
fn ran(
    cx: &mut VisualTestContext,
    waiting: &mut tokio::sync::oneshot::Receiver<Result<cox::agent::mcp::Ran, String>>,
) -> String {
    let mut ran = None;
    waits::until("the command's end", || {
        cx.run_until_parked();
        ran = waiting.try_recv().ok();
        ran.is_some()
    });
    ran.unwrap().expect("the command started").output
}

/// Waits until the terminal shows `text`.
fn shows(cx: &mut VisualTestContext, terminal: &Entity<Terminal>, text: &str) {
    waits::until(&format!("{text:?} in the terminal"), || {
        cx.run_until_parked();
        terminal.read_with(cx, |terminal, _| terminal.text().contains(text))
    });
}

fn mode(cx: &mut VisualTestContext) -> Option<Mode> {
    cx.update(|_, cx| cox::vim::mode(cx))
}

#[gpui::test]
fn a_terminal_takes_every_key_in_insert_mode_escape_too(cx: &mut TestAppContext) {
    let (_terminal, mut waiting, cx) = open_terminal(cx, "terminal-insert", "head -n 1 | cat -v");
    assert_eq!(mode(cx), Some(Mode::Insert), "as the keyboard came in");

    cx.simulate_keystrokes("x space y ctrl-w g t escape : j 1 enter");

    assert!(
        ran(cx, &mut waiting).ends_with("\nx gt^[:j1"),
        "control-w erased y, and escape went on"
    );
    assert_eq!(mode(cx), Some(Mode::Insert));
}

#[gpui::test]
fn turned_on_in_a_terminal_the_keys_start_in_normal_mode(cx: &mut TestAppContext) {
    let (_terminal, _waiting, cx) = open_terminal(cx, "terminal-turned-on", "head -n 1");
    cx.update(|window, cx| {
        cox::vim::switch(false, window, cx);
        cox::vim::switch(true, window, cx);
    });
    assert_eq!(mode(cx), Some(Mode::Normal));
}

#[gpui::test]
fn a_terminal_come_to_by_keys_is_in_normal_mode_and_a_click_types(cx: &mut TestAppContext) {
    let (terminal, _waiting, cx) = open_terminal(cx, "terminal-come-to", "head -n 1");
    assert_eq!(
        mode(cx),
        Some(Mode::Insert),
        "opened to type in, as the stage opens one"
    );

    cx.update(|window, cx| {
        window.blur();
        cox::vim::came(cox::vim::Region::Workspace, window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.focus(&terminal.focus_handle(cx)));
    cx.run_until_parked();
    assert_eq!(mode(cx), Some(Mode::Normal), "come to again, as by ctrl-w");

    cx.simulate_keystrokes("ctrl-w");
    assert_eq!(
        mode(cx),
        Some(Mode::Normal),
        "ctrl-w is Vim's, not the shell's"
    );

    let at = cx
        .debug_bounds("terminal")
        .expect("the terminal is drawn")
        .center();
    cx.simulate_click(at, gpui::Modifiers::default());
    assert_eq!(mode(cx), Some(Mode::Insert), "a click types");
}

#[gpui::test]
fn control_backslash_control_n_leaves_a_terminal_for_normal_mode(cx: &mut TestAppContext) {
    let (_terminal, mut waiting, cx) = open_terminal(cx, "terminal-normal", "head -n 1");

    cx.simulate_keystrokes("a ctrl-\\ ctrl-n");
    assert_eq!(mode(cx), Some(Mode::Normal));
    cx.simulate_keystrokes("x j escape enter ctrl-c");
    cx.simulate_keystrokes("i b ctrl-\\ c enter");

    assert_eq!(ran(cx, &mut waiting), "abc\nabc");
    assert_eq!(mode(cx), Some(Mode::Insert));
}

#[gpui::test]
fn normal_mode_scrolls_back_through_a_terminal(cx: &mut TestAppContext) {
    let (terminal, _waiting, cx) = open_terminal(cx, "terminal-scroll", "seq 1 200; cat");
    shows(cx, &terminal, "200");
    let back =
        |cx: &mut VisualTestContext| terminal.read_with(cx, |terminal, _| terminal.scrolled_back());

    cx.simulate_keystrokes("ctrl-\\ ctrl-n g g");
    let top = back(cx);
    assert!(top > 100, "at the top: {top}");
    cx.simulate_keystrokes("shift-g");
    assert_eq!(back(cx), 0);
    cx.simulate_keystrokes("k 3 k");
    assert_eq!(back(cx), 4);
    cx.simulate_keystrokes("ctrl-u");
    let half = back(cx) - 4;
    assert!(half > 5, "half the terminal: {half}");
    cx.simulate_keystrokes("j ctrl-d");
    assert_eq!(back(cx), 3);

    cx.simulate_keystrokes("i z");
    assert_eq!(back(cx), 0, "typing shows the end again");
    assert_eq!(mode(cx), Some(Mode::Insert));
}

#[gpui::test]
fn slash_finds_in_a_terminal_and_shift_v_y_copies_the_line(cx: &mut TestAppContext) {
    cx.update(cox::app::bind_keys);
    let cwd = scratch::folder("terminal-search");
    std::fs::create_dir_all(&cwd).unwrap();
    let running = Running::start(
        Program::Command("for i in $(seq 1 200); do echo \"row $i end\"; done; cat".to_owned()),
        &cwd,
    )
    .unwrap();
    let (terminal, _, cx) = hosted(cx, move |cx| Terminal::new(running, None, cx));
    cx.update(|window, _| window.activate_window());
    shows(cx, &terminal, "row 200 end");
    let selected = |cx: &mut VisualTestContext| {
        terminal.read_with(cx, |terminal, _| {
            terminal.selected_text().unwrap_or_default()
        })
    };
    let back =
        |cx: &mut VisualTestContext| terminal.read_with(cx, |terminal, _| terminal.scrolled_back());

    type_keys(cx, "ctrl-\\ ctrl-n");
    search_for(cx, "/", "w 15 e");
    assert_eq!(selected(cx), "w 15 e");
    let fifteen = back(cx);
    assert!(fifteen > 100, "scrolled to it: {fifteen}");
    assert_eq!(mode(cx), Some(Mode::Normal), "back from the line");

    search_for(cx, "?", "w 15");
    assert_eq!(selected(cx), "w 15", "the one before, going round");
    assert!(back(cx) < fifteen);
    type_keys(cx, "n shift-n");
    assert_eq!(selected(cx), "w 15");

    search_for(cx, "/", "w 150 e");
    assert_eq!(selected(cx), "w 150 e");
    type_keys(cx, "shift-v y");
    let copied = cx
        .update(|_, cx| cx.read_from_clipboard())
        .and_then(|item| item.text());
    assert_eq!(copied.as_deref().map(str::trim_end), Some("row 150 end"));
}

/// What stands for a tab running a program in these tests, as an Android
/// device's: a part of the window in its key context that keeps the
/// keys that reach it.
fn open_android(cx: &mut TestAppContext) -> (Entity<FakePage>, &mut VisualTestContext) {
    cx.update(|cx| cox::vim::turn(true, cx));
    let (android, cx) = cx.add_window_view(|_, cx| FakePage {
        focus: cx.focus_handle(),
        asked: Vec::new(),
        context: "Android",
    });
    cx.update(|window, cx| {
        window.focus(&android.read(cx).focus);
        cox::vim::came(cox::vim::Region::Android, window, cx);
    });
    assert_eq!(mode(cx), Some(Mode::Normal), "come to by keys");
    cx.update(|window, cx| cox::vim::typing(cox::vim::Region::Android, window, cx));
    cx.run_until_parked();
    (android, cx)
}

#[gpui::test]
fn an_android_device_takes_every_key_until_control_backslash_control_n(cx: &mut TestAppContext) {
    let (android, cx) = open_android(cx);
    cx.simulate_keystrokes("x escape ctrl-w g t :");
    assert_eq!(
        asked(&android, cx),
        [
            "key x",
            "key escape",
            "key ctrl-w",
            "key g",
            "key t",
            "key :"
        ]
    );
    assert_eq!(mode(cx), Some(Mode::Insert));

    cx.simulate_keystrokes("ctrl-\\ ctrl-n");
    assert_eq!(asked(&android, cx), ["leave"]);
    assert_eq!(mode(cx), Some(Mode::Normal));
    cx.simulate_keystrokes("x j escape enter g g y");
    assert_eq!(asked(&android, cx), Vec::<String>::new());

    cx.simulate_keystrokes("i y");
    assert_eq!(asked(&android, cx), ["key y"]);
}

/// A stage of a test's own: tabs it shows one of, with the keyboard
/// in it as Vim's keys ask, as the stage does.
struct Tabbed {
    tabs: Vec<(gpui::AnyView, gpui::FocusHandle)>,
    shown: usize,
}

impl Tabbed {
    fn by_keys(&mut self, tabs: &Tabs, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        let had_focus = self
            .tabs
            .get(self.shown)
            .is_some_and(|(_, focus)| focus.contains_focused(window, cx));
        match tabs {
            Tabs::Focus => {}
            Tabs::Along(step) => {
                let count = isize::try_from(self.tabs.len()).unwrap();
                self.shown = usize::try_from(
                    (isize::try_from(self.shown).unwrap() + step).rem_euclid(count),
                )
                .unwrap();
            }
            Tabs::Close => {
                self.tabs.remove(self.shown);
                self.shown = self.shown.min(self.tabs.len().saturating_sub(1));
            }
            _ => return,
        }
        if (had_focus || *tabs == Tabs::Focus)
            && let Some((_, focus)) = self.tabs.get(self.shown)
        {
            window.focus(focus);
        }
        cx.notify();
    }
}

impl gpui::Render for Tabbed {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};

        gpui::div()
            .size_full()
            .children(self.tabs.get(self.shown).map(|(view, _)| view.clone()))
    }
}

#[gpui::test]
fn a_whole_session_by_keys(cx: &mut TestAppContext) {
    use gpui::AppContext as _;

    let terminals: Rc<RefCell<Vec<Entity<Terminal>>>> = Rc::default();
    let made = Rc::clone(&terminals);
    let built: fake_daemon::Built = Rc::new(move |cwd, _, cx| {
        let page = cx.new(|cx| FakePage {
            focus: cx.focus_handle(),
            asked: Vec::new(),
            context: "Page",
        });
        let running =
            Running::start(Program::Command("seq 1 300; exec cat".to_owned()), &cwd).unwrap();
        let terminal = cx.new(|cx| Terminal::new(running, None, cx));
        made.borrow_mut().push(terminal.clone());
        let tabs = vec![
            (page.clone().into(), page.read(cx).focus.clone()),
            (terminal.clone().into(), terminal.focus_handle(cx)),
        ];
        let stage = cx.new(|_| Tabbed { tabs, shown: 0 });
        cox::chat::view::StageView {
            view: stage.clone().into(),
            panel: cx.new(|_| cox::ui::Panel::default()),
            by_keys: Rc::new(move |tabs, window, cx| {
                stage.update(cx, |stage, cx| stage.by_keys(&tabs, window, cx));
            }),
            shows_files: Rc::new(|_| false),
        }
    });
    cx.update(|cx| cox::vim::turn(true, cx));
    let (workspace, cx) = keys::open_window_on(
        cx,
        scratch::folder("session"),
        FakeStages {
            built: Some(built),
            ..FakeStages::default()
        },
    );
    cx.update(|window, _| window.activate_window());

    keys::press(cx, &workspace, "secondary-n h i escape");
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
    assert_eq!(shown_mode(cx), Some("Normal"));

    keys::press(cx, &workspace, "ctrl-w h");
    assert!(in_part(cx, "Page"));
    keys::press(cx, &workspace, "g t");
    assert!(in_part(cx, "Terminal"));
    assert_eq!(
        shown_mode(cx),
        Some("Normal"),
        "come to by keys, as Neovim's terminal"
    );
    let terminal = terminals.borrow().last().cloned().expect("a terminal");
    shows(cx, &terminal, "300");

    keys::press(cx, &workspace, "i");
    assert_eq!(shown_mode(cx), Some("Insert"));
    keys::press(cx, &workspace, "e c h o enter");
    shows(cx, &terminal, "echo\necho");
    keys::press(cx, &workspace, "ctrl-\\ ctrl-n");
    assert_eq!(shown_mode(cx), Some("Normal"));
    keys::press(cx, &workspace, "g g");
    assert!(terminal.read_with(cx, |terminal, _| terminal.scrolled_back()) > 0);

    keys::press(cx, &workspace, "ctrl-w l");
    assert!(in_part(cx, "Conversation"));
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
    assert_eq!(shown_mode(cx), Some("Normal"));

    keys::press(cx, &workspace, "ctrl-w h");
    assert!(
        in_part(cx, "Terminal"),
        "the stage shows the terminal still"
    );
    assert_eq!(shown_mode(cx), Some("Normal"), "coming back by keys");
    keys::press(cx, &workspace, ": escape");
    assert!(in_part(cx, "Terminal"));
    assert_eq!(shown_mode(cx), Some("Normal"), "back from the line");

    keys::press(cx, &workspace, ": q enter");
    assert!(in_part(cx, "Page"), "the terminal closed");
    keys::press(cx, &workspace, ": n e w enter");
    let chats = workspace.read_with(cx, |workspace, _| workspace.chats(0).count());
    assert_eq!(chats, 2);
    assert!(in_part(cx, "Conversation"));
    assert_eq!(shown_mode(cx), Some("Insert"));
    assert_eq!(keys::shown_input(cx, &workspace), "");

    keys::press(cx, &workspace, "escape : g o a l enter");
    assert!(in_part(cx, "Goal"));
    assert_eq!(shown_mode(cx), Some("Insert"), "before a key is typed");
}

#[gpui::test]
fn off_the_keys_are_as_they_were(cx: &mut TestAppContext) {
    let (workspace, cx) = keys::open_window(cx, scratch::folder("off"));
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    assert!(cx.debug_bounds("vim-status").is_none());
    cx.simulate_input("wait");
    keys::press(cx, &workspace, "enter");
    wait_for_waiting(cx, &view);
    assert_eq!(agent_state(cx, &view), State::Busy);

    keys::press(cx, &workspace, "escape");
    wait_for_stop(cx, &view);

    keys::press(cx, &workspace, "ctrl-w h g t");
    assert!(!in_part(cx, "Rail"));
    assert_eq!(keys::shown_input(cx, &workspace), "hgt");

    keys::press(cx, &workspace, "f shift-f");
    assert_eq!(keys::shown_input(cx, &workspace), "hgtfF");
    assert_eq!(tags(cx).len(), 0);
}

/// Opens the settings, clicks `chip` of Vim's keys, and closes them.
fn choose_on_the_settings(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    chip: &'static str,
) {
    keys::press(cx, workspace, "secondary-,");
    let card = cx.debug_bounds("settings").expect("the settings are drawn");
    cx.simulate_event(ScrollWheelEvent {
        position: card.center(),
        delta: ScrollDelta::Lines(gpui::point(0., -1000.)),
        modifiers: gpui::Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    settled(cx, workspace);
    let at = cx.debug_bounds(chip).expect("the chip is drawn").center();
    cx.simulate_click(at, gpui::Modifiers::none());
    settled(cx, workspace);
    keys::press(cx, workspace, "escape");
    assert!(workspace.read_with(cx, |workspace, _| workspace.settings().is_none()));
}

fn kept_vim() -> Option<serde_json::Value> {
    let file = cox::storage::path(cox::storage::Kept::Settings).unwrap();
    let text = std::fs::read_to_string(file).unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|settings| settings.get("vim").cloned())
}

#[gpui::test]
fn the_settings_turn_the_keys_on_and_off_at_once(cx: &mut TestAppContext) {
    let (workspace, cx) = keys::open_window(cx, scratch::folder("setting"));
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    cx.simulate_input("wait");
    keys::press(cx, &workspace, "enter");
    wait_for_waiting(cx, &view);
    assert_eq!(agent_state(cx, &view), State::Busy);

    choose_on_the_settings(cx, &workspace, "vim-on");
    assert_eq!(kept_vim(), Some(serde_json::Value::Bool(true)));
    assert_eq!(shown_mode(cx), Some("Normal"));
    keys::press(cx, &workspace, "i h i escape");
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
    assert_eq!(shown_mode(cx), Some("Normal"));
    assert_eq!(
        agent_state(cx, &view),
        State::Busy,
        "escape with Vim's keys"
    );

    choose_on_the_settings(cx, &workspace, "vim-off");
    assert_eq!(kept_vim(), None);
    assert_eq!(shown_mode(cx), None);
    // Where Escape left the cursor, on the i.
    keys::press(cx, &workspace, "j");
    assert_eq!(keys::shown_input(cx, &workspace), "hji");
    keys::press(cx, &workspace, "escape");
    wait_for_stop(cx, &view);
}

/// A folder of its own for a test, with the files `paths`, saying their
/// names, by its real path, as the tabs name their files.
fn folder(name: &str, paths: &[&str]) -> PathBuf {
    let folder = scratch::folder(name);
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
        cx.update(|cx| cox::vim::turn(true, cx));
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
fn code_selects_characters_from_the_cursor_and_copies_them(cx: &mut TestAppContext) {
    let path = folder("code-visual-characters", &[]).join("long.rs");
    let (code, cx) = open_code(cx, path, &numbered(20), true);
    let selected = |cx: &mut VisualTestContext| code.read_with(cx, |code, _| code.selected_text());

    let mode = |cx: &mut VisualTestContext| cx.update(|_, cx| cox::vim::mode(cx).map(Mode::label));
    type_keys(cx, "w v e");
    assert_eq!(selected(cx), "line_1");
    assert_eq!(mode(cx), Some("Visual"));
    type_keys(cx, "y");
    assert_eq!(mode(cx), Some("Normal"), "y leaves visual mode");
    let copied = cx
        .update(|_, cx| cx.read_from_clipboard())
        .and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("line_1"));
    assert_eq!(selected(cx), "");

    type_keys(cx, "b v j");
    assert_eq!(selected(cx), "line_1 = 1;\nlet l");
    type_keys(cx, "shift-v");
    assert_eq!(selected(cx), "let line_1 = 1;\nlet line_2 = 2;\n");
    assert_eq!(mode(cx), Some("Visual Line"));
    type_keys(cx, "v k b");
    assert_eq!(selected(cx), "let l", "back before the start");
    type_keys(cx, "escape");
    assert_eq!(selected(cx), "");
    assert_eq!(mode(cx), Some("Normal"));
}

#[gpui::test]
fn code_finds_the_word_under_the_cursor_whole_and_again(cx: &mut TestAppContext) {
    let path = folder("code-star", &[]).join("star.rs");
    let text = "let a = helper;\nlet b = helpers;\nlet c = helper;\n\nlet d = helper;";
    let (code, cx) = open_code(cx, path, text, true);
    let place = |cx: &mut VisualTestContext| code.read_with(cx, |code, _| code.cursor_place());

    type_keys(cx, "3 w *");
    assert_eq!(place(cx), Some((2, 8)), "past `helpers`");
    type_keys(cx, "n");
    assert_eq!(place(cx), Some((4, 8)));
    type_keys(cx, "n");
    assert_eq!(place(cx), Some((0, 8)), "round");
    type_keys(cx, "#");
    assert_eq!(place(cx), Some((4, 8)), "back, round");
    type_keys(cx, "shift-n");
    assert_eq!(place(cx), Some((0, 8)), "the other way from `#`");
    type_keys(cx, "j 0 *");
    assert_eq!(place(cx), Some((2, 0)), "`let`, from the line's start");
}

#[gpui::test]
fn code_goes_to_matching_brackets_and_between_paragraphs(cx: &mut TestAppContext) {
    let path = folder("code-brackets", &[]).join("brackets.rs");
    let text = "fn main() {\n    let a = [(1), 2];\n\n    helper();\n}";
    let (code, cx) = open_code(cx, path, text, true);
    let place = |cx: &mut VisualTestContext| code.read_with(cx, |code, _| code.cursor_place());

    type_keys(cx, "%");
    assert_eq!(place(cx), Some((0, 8)), "the next bracket's match");
    type_keys(cx, "%");
    assert_eq!(place(cx), Some((0, 7)));
    type_keys(cx, "$ %");
    assert_eq!(place(cx), Some((4, 0)));
    type_keys(cx, "%");
    assert_eq!(place(cx), Some((0, 10)));

    type_keys(cx, "}");
    assert_eq!(place(cx), Some((2, 0)));
    type_keys(cx, "}");
    assert_eq!(place(cx), Some((4, 0)), "the end");
    type_keys(cx, "{");
    assert_eq!(place(cx), Some((2, 0)));
    type_keys(cx, "j 2 {");
    assert_eq!(place(cx), Some((0, 0)), "the start");
}

#[gpui::test]
fn code_scrolls_the_cursor_s_line_to_the_top_middle_and_bottom(cx: &mut TestAppContext) {
    let path = folder("code-z", &[]).join("long.rs");
    let (code, cx) = open_code(cx, path, &numbered(200), true);
    let bounds = cx.debug_bounds("code").expect("the code is drawn");
    let height = bounds.size.height;
    let y = |cx: &mut VisualTestContext| {
        code.read_with(cx, |code, _| code.point_of(99, 0))
            .expect("line 100 is in sight")
            .y
    };

    type_keys(cx, "1 0 0 shift-g z t");
    assert_eq!(cursor(&code, cx), Some(99));
    assert!(y(cx) - bounds.top() < height / 8., "at the top");
    type_keys(cx, "z b");
    assert_eq!(cursor(&code, cx), Some(99));
    assert!(bounds.bottom() - y(cx) < height / 8., "at the bottom");
    type_keys(cx, "z z");
    assert_eq!(cursor(&code, cx), Some(99));
    let middle = bounds.center().y;
    assert!((y(cx) - middle).abs() < height / 8., "in the middle");
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
    cx.update(|cx| cox::vim::turn(true, cx));
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
    cx.update(|cx| cox::vim::turn(true, cx));
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

/// Makes `folder` a Git repository with a commit of all it holds.
fn commit_all(folder: &std::path::Path) {
    git(folder, &["init", "--quiet"]);
    git(folder, &["add", "."]);
    git(folder, &["commit", "--quiet", "-m", "A"]);
}

/// A folder of its own for a test, a Git repository with a commit of
/// `paths`, each saying its name, which then change.
fn changed(name: &str, paths: &[&str]) -> PathBuf {
    let folder = folder(name, paths);
    commit_all(&folder);
    for path in paths {
        std::fs::write(folder.join(path), "changed\n").unwrap();
    }
    folder
}

#[gpui::test]
fn changes_move_from_file_to_file_and_open_one(cx: &mut TestAppContext) {
    cx.update(|cx| cox::vim::turn(true, cx));
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
    waits::until("the three files changed", || {
        cx.run_until_parked();
        let files = changes.read_with(cx, |changes, _| {
            changes
                .shown_diff()
                .and_then(|read| read.as_ref().ok())
                .map_or(0, |diff| diff.files.len())
        });
        files == 3
    });

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
fn changes_go_from_stretch_to_stretch_and_open_one_there(cx: &mut TestAppContext) {
    cx.update(|cx| cox::vim::turn(true, cx));
    let root = folder("changes-stretches", &[]);
    for name in ["a.rs", "b.rs", "c.rs"] {
        std::fs::write(root.join(name), numbered(100)).unwrap();
    }
    commit_all(&root);
    let rewrite = |name: &str, changes: fn(usize) -> bool| {
        let text: String = (1..=100)
            .map(|line| {
                if changes(line) {
                    format!("let changed_{line} = 0;\n")
                } else {
                    format!("let line_{line} = {line};\n")
                }
            })
            .collect();
        std::fs::write(root.join(name), text).unwrap();
    };
    rewrite("a.rs", |line| {
        (10..30).contains(&line) || (70..90).contains(&line)
    });
    rewrite("b.rs", |line| (40..60).contains(&line));
    rewrite("c.rs", |_| true);
    let (folder, cwd) = (root.clone(), root.clone());
    let (changes, cx) = cx.add_window_view(|_, cx| Changes::new(folder, &cwd, cx));
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let heard = Rc::clone(&chosen);
    cx.update(|window, cx| {
        window.focus(&changes.focus_handle(cx));
        cx.subscribe(
            &changes,
            move |_, chosen: &cox::stage::changes::Chosen, _| {
                heard.borrow_mut().push((chosen.path.clone(), chosen.line));
            },
        )
        .detach();
    });
    let mut stretches = Vec::new();
    waits::until("four stretches", || {
        cx.run_until_parked();
        stretches = changes.read_with(cx, |changes, _| {
            let Some(Ok(diff)) = changes.shown_diff() else {
                return Vec::new();
            };
            let mut row = 0;
            let mut stretches = Vec::new();
            for file in &diff.files {
                row += 1;
                for line in &file.lines {
                    if line.kind == cox_editor::changes::Kind::Stretch {
                        stretches.push(row);
                    }
                    row += 1;
                }
            }
            stretches
        });
        stretches.len() == 4
    });
    assert_eq!(
        stretches.len(),
        4,
        "two stretches in a.rs, one in b.rs and c.rs"
    );
    let top = |row: usize, cx: &mut VisualTestContext| {
        let selector = Box::leak(format!("changed-line-{row}").into_boxed_str());
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("row {row} in sight"))
            .top()
    };
    let height = cx.update(|window, _| window.viewport_size().height);
    let first = top(stretches[0], cx);

    type_keys(cx, "] c enter");
    let list_top = top(stretches[0], cx);
    assert!(list_top < first, "the first stretch scrolls up to the top");
    assert!(list_top < height / 4.);
    type_keys(cx, "2 ] c enter");
    assert_eq!(top(stretches[2], cx), list_top);
    type_keys(cx, "[ c enter");
    assert_eq!(top(stretches[1], cx), list_top);
    type_keys(cx, "5 [ c enter");
    assert_eq!(top(stretches[0], cx), list_top);
    type_keys(cx, "j enter");
    assert_eq!(
        *chosen.borrow(),
        [
            (root.join("a.rs"), Some(10)),
            (root.join("b.rs"), Some(40)),
            (root.join("a.rs"), Some(70)),
            (root.join("a.rs"), Some(10)),
            (root.join("b.rs"), None),
        ]
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
    wait_for_reply(cx, &view);
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
    cx.update(|cx| cox::vim::turn(true, cx));
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
        waits::until("the server's answer", || {
            cx.run_until_parked();
            done(cx)
        });
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
    context: &'static str,
}

impl gpui::Render for FakePage {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use cox::vim::search::Find;
        use cox::vim::{Address, CopyAddress, FocusField, Hint, Label, Leave, Move, Reload};
        use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};

        let keep = |page: &mut Self, asked: String| page.asked.push(asked);
        gpui::div()
            .size_full()
            .track_focus(&self.focus)
            .key_context(self.context)
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
            .on_action(cx.listener(move |page, find: &Find, _, _| {
                keep(page, format!("find {} {:?}", find.text, find.direction));
            }))
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
    cx.update(|cx| cox::vim::turn(true, cx));
    let (page, cx) = cx.add_window_view(|_, cx| FakePage {
        focus: cx.focus_handle(),
        asked: Vec::new(),
        context: "Page",
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
    // GPUI names the platform key Command on macOS and Super elsewhere.
    let copy = if cfg!(target_os = "macos") {
        "key cmd-c"
    } else {
        "key super-c"
    };
    assert_eq!(asked(&page, cx), [copy, "key k", "leave"]);
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

    cx.simulate_keystrokes("; t");
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
        let line = workspace.line().read(cx);
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
    assert_eq!(shown_mode(cx), Some("Command"));

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
    assert_eq!(shown_mode(cx), Some("Normal"));
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
    assert_eq!(shown_mode(cx), Some("Insert"), "a new chat types at once");

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
    wait_for_waiting(cx, &view);
    assert_eq!(agent_state(cx, &view), State::Busy);

    keys::press(cx, &workspace, "escape : s t o p enter");
    wait_for_stop(cx, &view);
}

#[gpui::test]
fn colon_quit_and_the_rest_go_to_the_stage(cx: &mut TestAppContext) {
    let keyed = Keyed::default();
    let stages = FakeStages {
        keyed: Some(keyed.clone()),
        ..FakeStages::default()
    };
    cx.update(|cx| cox::vim::turn(true, cx));
    let (workspace, cx) = keys::open_window_on(cx, scratch::folder("line-tabs"), stages);
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
fn t_on_a_page_and_open_with_words_ask_the_stage_to_search(cx: &mut TestAppContext) {
    use gpui::AppContext as _;

    let keyed = Keyed::default();
    let asked = keyed.clone();
    let built: fake_daemon::Built = Rc::new(move |_, _, cx| {
        let page = cx.new(|cx| FakePage {
            focus: cx.focus_handle(),
            asked: Vec::new(),
            context: "Page",
        });
        let asked = asked.clone();
        cox::chat::view::StageView {
            view: page.clone().into(),
            panel: cx.new(|_| cox::ui::Panel::default()),
            by_keys: Rc::new(move |tabs, window, cx| {
                if tabs == Tabs::Focus {
                    window.focus(&page.read(cx).focus);
                }
                asked.borrow_mut().push(tabs);
            }),
            shows_files: Rc::new(|_| false),
        }
    });
    cx.update(|cx| cox::vim::turn(true, cx));
    let (workspace, cx) = keys::open_window_on(
        cx,
        scratch::folder("search"),
        FakeStages {
            built: Some(built),
            ..FakeStages::default()
        },
    );
    keys::press(cx, &workspace, "secondary-n escape ctrl-w h");
    assert!(in_part(cx, "Page"));
    keys::press(cx, &workspace, "t");
    keys::press(cx, &workspace, ":");
    cx.simulate_input("tabopen rust gpui");
    keys::press(cx, &workspace, "enter :");
    cx.simulate_input("open 検索 する");
    keys::press(cx, &workspace, "enter :");
    cx.simulate_input("open !echo hi");
    keys::press(cx, &workspace, "enter");
    let open = |line: &str, follow| Tabs::Open {
        line: line.to_owned(),
        follow,
    };
    assert_eq!(
        *keyed.borrow(),
        [
            Tabs::Focus,
            Tabs::Menu,
            open("rust gpui", Follow::NewTab),
            open("検索 する", Follow::Here),
            open("!echo hi", Follow::Here),
        ]
    );
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
    assert_eq!(shown_mode(cx), Some("Normal"));
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
    let (workspace, cx) = keys::open_window(cx, scratch::folder("line-off"));
    keys::press(cx, &workspace, "secondary-n escape :");
    assert!(!line(cx, &workspace).open);
    assert!(!in_part(cx, "CommandLine"));
}

/// A window that keeps the line at its foot over a tab of the stage, as
/// the workspace does, for tests of a tab in a window of its own.
struct Host {
    child: gpui::AnyView,
    line: Entity<cox::vim::line::Line>,
}

impl gpui::Render for Host {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use cox::vim::line::OpenLine;
        use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};

        gpui::div()
            .relative()
            .size_full()
            .on_action(cx.listener(|host, open: &OpenLine, window, cx| {
                host.line.update(cx, |line, cx| {
                    line.open(open.prompt, cox::vim::ex::Names::default(), window, cx);
                });
            }))
            .child(self.child.clone())
            .child(self.line.clone())
    }
}

/// Opens `build`'s view in a window that hosts the line, with the
/// keyboard, and Vim's keys on.
fn hosted<V: gpui::Render + gpui::Focusable>(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut gpui::Context<V>) -> V + 'static,
) -> (Entity<V>, Entity<Host>, &mut VisualTestContext) {
    use gpui::AppContext as _;

    cx.update(|cx| cox::vim::turn(true, cx));
    cx.update(cox::ui::input::bind_keys);
    let (host, cx) = cx.add_window_view(|window, cx| Host {
        child: cx.new(build).into(),
        line: cx.new(|cx| cox::vim::line::Line::new(window, cx)),
    });
    let child = host.read_with(cx, |host, _| host.child.clone().downcast::<V>().unwrap());
    cx.update(|window, cx| window.focus(&child.focus_handle(cx)));
    cx.run_until_parked();
    (child, host, cx)
}

/// Types `text` in the line `/` or `?` opened, and runs it.
fn search_for(cx: &mut VisualTestContext, mark: &str, text: &str) {
    type_keys(cx, mark);
    cx.simulate_input(text);
    type_keys(cx, "enter");
}

#[gpui::test]
fn slash_and_n_find_text_in_code(cx: &mut TestAppContext) {
    let path = folder("search-code", &[]).join("numbered.rs");
    let text = numbered(40);
    let (code, host, cx) = hosted(cx, move |cx| Code::new(path, &text, None, None, cx));
    let selected = |cx: &mut VisualTestContext| code.read_with(cx, |code, _| code.selected_text());

    search_for(cx, "/", "line_3");
    assert!(!host.read_with(cx, |host, cx| host.line.read(cx).is_open()));
    assert_eq!(cx.update(|_, cx| cox::vim::mode(cx)), Some(Mode::Normal));
    assert_eq!(cursor(&code, cx), Some(2));
    assert_eq!(selected(cx), "line_3");
    type_keys(cx, "n");
    assert_eq!(cursor(&code, cx), Some(29));
    type_keys(cx, "n n");
    assert_eq!(cursor(&code, cx), Some(31));
    type_keys(cx, "shift-n");
    assert_eq!(cursor(&code, cx), Some(30));

    search_for(cx, "?", "LINE_1");
    assert_eq!(cursor(&code, cx), Some(30), "a capital finds its own case");
    assert!(
        cx.debug_bounds("code-notice").is_some(),
        "saying it is not found"
    );
    search_for(cx, "?", "line_1");
    assert_eq!(cursor(&code, cx), Some(18));
    type_keys(cx, "n");
    assert_eq!(cursor(&code, cx), Some(17), "n goes the search's way");
    type_keys(cx, "shift-n");
    assert_eq!(cursor(&code, cx), Some(18));

    type_keys(cx, "/ up");
    assert_eq!(
        host.read_with(cx, |host, cx| host.line.read(cx).text(cx).to_owned()),
        "line_1"
    );
    type_keys(cx, "escape");
    assert_eq!(cursor(&code, cx), Some(18), "escape found nothing");
}

#[gpui::test]
fn slash_and_n_find_text_in_a_document(cx: &mut TestAppContext) {
    cx.update(cox::stage::document::bind_keys);
    let path = folder("search-document", &[]).join("long.md");
    let mut text = String::new();
    for line in 1..=300 {
        write!(text, "Line {line}.\n\n").unwrap();
    }
    let (document, _, cx) = hosted(cx, move |cx| Document::new(path, text, cx));
    let top = |cx: &mut VisualTestContext| {
        cx.debug_bounds("document-text")
            .expect("the text is drawn")
            .origin
            .y
    };
    let selected =
        |cx: &mut VisualTestContext| document.read_with(cx, |document, _| document.selected_text());
    let start = top(cx);

    search_for(cx, "/", "line 25");
    assert_eq!(selected(cx), "Line 25");
    let first = start - top(cx);
    assert!(first > px(0.), "scrolled to it: {first:?}");
    type_keys(cx, "n");
    assert_eq!(selected(cx), "Line 25");
    let second = start - top(cx);
    assert!(second > first * 5., "on to Line 250: {second:?}");
    type_keys(cx, "shift-n");
    assert_eq!(start - top(cx), first);
}

/// `]]` and `[[` select a document's blocks one after another, from the
/// first in sight, scrolling to them, and `y` copies the one selected.
#[gpui::test]
fn brackets_go_from_block_to_block_of_a_document_and_y_copies_one(cx: &mut TestAppContext) {
    let path = folder("document-blocks", &[]).join("blocks.md");
    let mut text = String::from("# Title\n\nFirst paragraph.\n\n- an item\n- another\n\n");
    for line in 1..=200 {
        write!(text, "Line {line}.\n\n").unwrap();
    }
    let (document, _, cx) = hosted(cx, move |cx| Document::new(path, text, cx));
    let selected =
        |cx: &mut VisualTestContext| document.read_with(cx, |document, _| document.selected_text());
    let top = |cx: &mut VisualTestContext| {
        cx.debug_bounds("document-text")
            .expect("the text is drawn")
            .origin
            .y
    };

    type_keys(cx, "] ]");
    assert_eq!(selected(cx), "Title", "the first block in sight");
    type_keys(cx, "] ]");
    assert_eq!(selected(cx), "First paragraph.");
    type_keys(cx, "] ] ] ]");
    assert_eq!(selected(cx), "another");
    type_keys(cx, "[ [");
    assert_eq!(selected(cx), "an item");

    type_keys(cx, "y");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("an item"));

    let start = top(cx);
    type_keys(cx, "shift-g");
    type_keys(cx, "[ [");
    assert_eq!(selected(cx), "First paragraph.", "on from what is selected");
    assert_eq!(top(cx), start, "scrolled back to it");
    search_for(cx, "/", "line 150");
    type_keys(cx, "] ]");
    assert_eq!(selected(cx), "Line 151.");
    assert!(start - top(cx) > px(0.), "scrolled to it");
}

#[gpui::test]
fn slash_and_n_ask_the_page_to_find_the_text(cx: &mut TestAppContext) {
    let (page, _, cx) = hosted(cx, |cx| FakePage {
        focus: cx.focus_handle(),
        asked: Vec::new(),
        context: "Page",
    });
    type_keys(cx, "n");
    assert_eq!(
        asked(&page, cx),
        Vec::<String>::new(),
        "nothing searched yet"
    );
    search_for(cx, "/", "hello world");
    assert_eq!(asked(&page, cx), ["find hello world Forward"]);
    type_keys(cx, "n shift-n");
    assert_eq!(
        asked(&page, cx),
        ["find hello world Forward", "find hello world Backward"]
    );
    search_for(cx, "?", "");
    assert_eq!(
        asked(&page, cx),
        ["find hello world Backward"],
        "an empty line finds the last text again"
    );
    type_keys(cx, "n");
    assert_eq!(asked(&page, cx), ["find hello world Backward"]);
}

impl gpui::Focusable for FakePage {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}

/// The hints' labels over the window, with what each is over, each
/// drawn at the top left of what it is over.
fn tags(cx: &mut VisualTestContext) -> Vec<(String, gpui::Bounds<gpui::Pixels>)> {
    let shown = cx.update(|window, _| cox::vim::hints::shown_labels(window));
    for (label, over) in &shown {
        let selector: &'static str = Box::leak(format!("hint-{label}").into_boxed_str());
        let drawn = cx.debug_bounds(selector).expect("the label is drawn");
        assert_eq!(drawn.origin, over.origin, "{label}");
    }
    shown
}

/// The label over what `selector` finds, the innermost of those over it.
fn tag_over(cx: &mut VisualTestContext, selector: &'static str) -> String {
    let target = cx.debug_bounds(selector).expect("the target is drawn");
    tags(cx)
        .into_iter()
        .filter(|(_, over)| target.contains(&over.center()) && over.contains(&target.center()))
        .min_by(|(_, a), (_, b)| {
            let area =
                |size: gpui::Size<gpui::Pixels>| f32::from(size.width) * f32::from(size.height);
            area(a.size).total_cmp(&area(b.size))
        })
        .map_or_else(|| panic!("no hint over {selector}"), |(label, _)| label)
}

fn type_label(cx: &mut VisualTestContext, workspace: &Entity<Workspace>, label: &str) {
    let keys: Vec<String> = label.chars().map(String::from).collect();
    keys::press(cx, workspace, &keys.join(" "));
}

#[gpui::test]
fn f_on_the_rail_hints_its_buttons_and_a_label_chooses_a_project(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "hints-rail");
    keys::press(cx, &workspace, "secondary-n escape");
    open_project(cx, &workspace, "hints-rail-a");
    open_project(cx, &workspace, "hints-rail-b");
    assert_eq!(chosen(cx, &workspace), 2);

    keys::press(cx, &workspace, "ctrl-w h f");
    assert_eq!(mode(cx), Some(Mode::Hint));
    let shown = tags(cx);
    let rail = cx.debug_bounds("project-1").expect("the rail is drawn");
    assert!(
        shown.iter().all(|(_, tag)| tag.origin.x < rail.right()),
        "the rail's hints only: {shown:?}"
    );
    assert_eq!(shown.len(), 5, "three projects, new project, settings");
    let first = tag_over(cx, "project-1");
    type_label(cx, &workspace, &first);
    assert_eq!(chosen(cx, &workspace), 0);
    assert_eq!(tags(cx).len(), 0);
    assert_ne!(mode(cx), Some(Mode::Hint));
}

#[gpui::test]
fn f_in_the_chat_answers_the_agent_s_question_by_its_label(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "hints-ask");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    cx.simulate_input("permission");
    keys::press(cx, &workspace, "enter");
    let asked = |cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| view.conversation().options_asked())
    };
    settle_until(cx, |cx| asked(cx) > 0);
    assert!(asked(cx) > 0, "the agent asks permission");

    keys::press(cx, &workspace, "escape f");
    let panel = cx.debug_bounds("chat-panel").expect("the chat is drawn");
    let shown = tags(cx);
    assert!(
        shown.iter().all(|(_, tag)| panel.contains(&tag.origin)),
        "the chat's hints only: {shown:?}"
    );
    let option = tag_over(cx, "option-1");
    type_label(cx, &workspace, &option);
    settle_until(cx, |cx| asked(cx) == 0);
    assert_ne!(agent_state(cx, &view), State::Asking);
}

#[gpui::test]
fn a_field_s_hint_gives_it_the_keyboard_in_insert_mode(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "hints-field");
    keys::press(cx, &workspace, "secondary-n escape");
    open_project(cx, &workspace, "hints-field-a");
    keys::press(cx, &workspace, "ctrl-w h");
    assert!(in_part(cx, "Rail"));

    keys::press(cx, &workspace, "shift-f");
    let input = cx.debug_bounds("input").expect("the input is drawn");
    let field = tags(cx)
        .into_iter()
        .filter(|(_, over)| input.contains(&over.center()))
        .min_by(|(_, a), (_, b)| {
            (a.origin.y, a.origin.x)
                .partial_cmp(&(b.origin.y, b.origin.x))
                .unwrap()
        })
        .map(|(label, _)| label)
        .expect("a hint over the input");
    type_label(cx, &workspace, &field);
    assert_eq!(shown_mode(cx), Some("Insert"));
    assert!(in_part(cx, "LineInput"));
    keys::press(cx, &workspace, "h i");
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
}

#[gpui::test]
fn shift_f_hints_the_whole_window_and_typing_narrows_them_until_escape(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "hints-window");
    keys::press(cx, &workspace, "secondary-n escape");
    for name in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"] {
        open_project(cx, &workspace, &format!("hints-window-{name}"));
    }
    keys::press(cx, &workspace, "shift-f");
    let all = tags(cx);
    assert!(all.len() > 14, "two letters each: {}", all.len());
    let rail = cx.debug_bounds("project-1").expect("the rail is drawn");
    let panel = cx.debug_bounds("chat-panel").expect("the chat is drawn");
    assert!(all.iter().any(|(_, tag)| tag.origin.x < rail.right()));
    assert!(all.iter().any(|(_, tag)| panel.contains(&tag.origin)));

    let first = all[0].0.chars().next().unwrap().to_string();
    keys::press(cx, &workspace, &first);
    let left = tags(cx);
    assert!(!left.is_empty() && left.len() < all.len());
    assert!(left.iter().all(|(label, _)| label.starts_with(&first)));
    assert_eq!(mode(cx), Some(Mode::Hint));

    keys::press(cx, &workspace, "escape");
    assert_eq!(tags(cx).len(), 0);
    assert_eq!(mode(cx), Some(Mode::Normal));
    let chosen_before = chosen(cx, &workspace);
    keys::press(cx, &workspace, "s");
    assert_eq!(chosen(cx, &workspace), chosen_before, "s is no label now");
}

#[gpui::test]
fn f_on_the_settings_flips_a_chip_in_sight(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "hints-settings");
    keys::press(cx, &workspace, "secondary-n escape secondary-,");
    assert!(in_part(cx, "Settings"));
    keys::press(cx, &workspace, "f");
    let settings = cx.debug_bounds("settings").expect("the settings are drawn");
    let shown = tags(cx);
    let off = cx.debug_bounds("vim-off").expect("the chip is laid out");
    assert!(off.origin.y > settings.bottom(), "the chip is out of sight");
    assert!(
        shown.iter().all(|(_, over)| !off.contains(&over.center())),
        "no hint for what is out of sight"
    );
    assert_ne!(shown.len(), 0);
    assert!(
        shown.iter().all(|(_, tag)| settings.contains(&tag.origin)),
        "nothing under the settings: {shown:?}"
    );
    keys::press(cx, &workspace, "escape");

    cx.simulate_event(ScrollWheelEvent {
        position: settings.center(),
        delta: ScrollDelta::Lines(gpui::point(0., -1000.)),
        modifiers: gpui::Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    settled(cx, &workspace);
    keys::press(cx, &workspace, "f");
    let off = tag_over(cx, "vim-off");
    type_label(cx, &workspace, &off);
    assert_eq!(kept_vim(), None);
    assert_eq!(cx.update(|_, cx| cox::vim::mode(cx)), None);
}

/// A window with a button of its own and a page, in the page's key
/// context, which keeps what the window's hints ask of the page.
struct HintedPage {
    focus: gpui::FocusHandle,
    listed: Rc<std::cell::Cell<bool>>,
    asked: Rc<RefCell<Vec<String>>>,
    answer: Rc<RefCell<Option<cox::vim::hints::Answer>>>,
}

/// The page of a [`HintedPage`], as the window's hints see it.
struct PageHints {
    page: gpui::WeakEntity<HintedPage>,
    listed: Rc<std::cell::Cell<bool>>,
    asked: Rc<RefCell<Vec<String>>>,
    answer: Rc<RefCell<Option<cox::vim::hints::Answer>>>,
}

impl cox::vim::hints::Page for PageHints {
    fn ask(&self, answer: cox::vim::hints::Answer, _: &mut gpui::App) {
        self.asked.borrow_mut().push("ask".into());
        self.listed.set(true);
        *self.answer.borrow_mut() = Some(answer);
    }

    fn show(&self, labels: &[(u32, String)], cx: &mut gpui::App) {
        if let Some(page) = self.page.upgrade() {
            page.read(cx);
        }
        let shown: Vec<String> = labels
            .iter()
            .map(|(id, label)| format!("{id}={label}"))
            .collect();
        self.asked
            .borrow_mut()
            .push(format!("show {}", shown.join(" ")));
    }

    fn take(&self, id: u32, _: &mut gpui::Window, _: &mut gpui::App) {
        let lost = if self.listed.get() {
            ""
        } else {
            " after its hints were cleared"
        };
        self.asked.borrow_mut().push(format!("take {id}{lost}"));
    }

    fn clear(&self, _: &mut gpui::App) {
        self.asked.borrow_mut().push("clear".into());
        self.listed.set(false);
    }
}

impl gpui::Render for HintedPage {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use cox::vim::hints::Hinted as _;
        use gpui::{InteractiveElement as _, ParentElement as _, Styled as _};

        cox::vim::hints::forget_marks(window);
        let asked = self.asked.clone();
        gpui::div()
            .size_full()
            .track_focus(&self.focus)
            .key_context("Page")
            .child(
                gpui::div()
                    .id("native")
                    .debug_selector(|| "native".to_owned())
                    .w(px(40.))
                    .h(px(20.))
                    .child("button")
                    .on_hint_click(move |_, _, _| asked.borrow_mut().push("native".into())),
            )
            .child(cox::vim::hints::page(Rc::new(PageHints {
                page: cx.entity().downgrade(),
                listed: self.listed.clone(),
                asked: self.asked.clone(),
                answer: self.answer.clone(),
            })))
            .children(cox::vim::hints::overlay(window))
    }
}

#[gpui::test]
fn shift_f_labels_the_page_s_hints_and_the_window_s_in_one_sequence(cx: &mut TestAppContext) {
    cx.update(|cx| cox::vim::turn(true, cx));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let answer = Rc::new(RefCell::new(None));
    let (page, cx) = cx.add_window_view(|_, cx| HintedPage {
        focus: cx.focus_handle(),
        listed: Rc::default(),
        asked: asked.clone(),
        answer: answer.clone(),
    });
    cx.update(|window, cx| window.focus(&page.read(cx).focus));
    cx.run_until_parked();
    let taken = |asked: &Rc<RefCell<Vec<String>>>| std::mem::take(&mut *asked.borrow_mut());

    cx.simulate_keystrokes("shift-f");
    assert_eq!(taken(&asked), ["ask"]);
    assert_eq!(tags(cx).len(), 0, "no labels until the page answers");
    let answered = answer.borrow_mut().take().expect("the page was asked");
    page.update_in(cx, |_, window, cx| answered(vec![7, 9], window, cx));
    cx.run_until_parked();
    assert_eq!(taken(&asked), ["show 7=a 9=d"]);
    assert_eq!(tag_over(cx, "native"), "s");

    cx.simulate_keystrokes("d");
    assert_eq!(taken(&asked), ["take 9", "clear"]);
    assert_eq!(tags(cx).len(), 0);
    assert_eq!(cx.update(|_, cx| cox::vim::mode(cx)), Some(Mode::Normal));

    cx.simulate_keystrokes("shift-f");
    let answered = answer.borrow_mut().take().expect("the page was asked");
    cx.update(|window, cx| answered(vec![7], window, cx));
    cx.run_until_parked();
    taken(&asked);
    cx.simulate_keystrokes("s");
    assert_eq!(taken(&asked), ["native", "clear"]);

    cx.simulate_keystrokes("shift-f");
    let answered = answer.borrow_mut().take().expect("the page was asked");
    cx.update(|window, cx| answered(Vec::new(), window, cx));
    cx.run_until_parked();
    taken(&asked);
    cx.simulate_keystrokes("escape");
    assert_eq!(taken(&asked), ["clear"]);
    assert_eq!(tags(cx).len(), 0);
    assert_eq!(cx.update(|_, cx| cox::vim::mode(cx)), Some(Mode::Normal));
}

#[gpui::test]
fn f_after_folding_the_chat_by_its_label_labels_the_folded_chat(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "hints-folded");
    keys::press(cx, &workspace, "secondary-n escape f");
    let fold = tag_over(cx, "fold-chat");
    type_label(cx, &workspace, &fold);
    assert!(workspace.read_with(cx, |workspace, _| workspace.is_folded()));

    keys::press(cx, &workspace, "f");
    let unfold = tag_over(cx, "unfold-chat");
    type_label(cx, &workspace, &unfold);
    assert!(!workspace.read_with(cx, |workspace, _| workspace.is_folded()));
}

/// Vim's Escape in a page's address line puts the page's address back, as
/// Escape does with Vim's keys off, and leaves insert mode.
#[gpui::test]
fn escape_in_the_address_line_puts_the_page_s_address_back(cx: &mut TestAppContext) {
    use cox::stage::page::address::{AddressLine, Asked};

    cx.update(cox::ui::input::bind_keys);
    cx.update(cox::stage::page::address::bind_keys);
    cx.update(|cx| cox::vim::turn(true, cx));
    cx.update(|cx| {
        cox::stage::page::history::keep(cox::stage::page::history::History::default(), None, cx);
    });
    let (line, cx) = cx.add_window_view(|_, cx| AddressLine::new("https://example.com/", cx));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let keep = Rc::clone(&asked);
    cx.update(|_, cx| {
        cx.subscribe(&line, move |_, asked: &Asked, _| {
            keep.borrow_mut().push(asked.clone());
        })
        .detach();
    });
    cx.update(|window, cx| line.update(cx, |line, cx| line.focus(window, cx)));
    cx.run_until_parked();
    cx.simulate_input("rust gpui");
    assert_eq!(
        cx.update(|_, cx| line.read(cx).text(cx).to_owned()),
        "rust gpui"
    );
    assert_eq!(mode(cx), Some(Mode::Insert));

    cx.simulate_keystrokes("escape");
    assert_eq!(
        cx.update(|_, cx| line.read(cx).text(cx).to_owned()),
        "https://example.com/"
    );
    assert_eq!(*asked.borrow(), [Asked::Back]);
    assert_eq!(mode(cx), Some(Mode::Normal));
}

/// With Vim's keys on, the address line keeps Up, Down, Control-N and
/// Control-P for the list of pages it offers, and Enter opens the one
/// chosen.
#[gpui::test]
fn the_address_line_s_list_takes_its_keys_with_vim_on(cx: &mut TestAppContext) {
    use cox::stage::page::address::{AddressLine, Asked, Row};
    use cox::stage::page::history::{self, History};

    cx.update(cox::ui::input::bind_keys);
    cx.update(cox::stage::page::address::bind_keys);
    cx.update(|cx| cox::vim::turn(true, cx));
    let now = history::now();
    let mut visited = History::default();
    visited.visit("https://github.com/", "GitHub", now);
    visited.visit("https://github.com/", "GitHub", now);
    visited.visit("https://github.com/zed-industries/zed", "Zed", now);
    visited.visit("https://docs.rs/gpui", "gpui on GitHub", now);
    cx.update(|cx| history::keep(visited, None, cx));
    let (line, cx) = cx.add_window_view(|_, cx| AddressLine::new("https://example.com/", cx));
    let asked = Rc::new(RefCell::new(Vec::new()));
    let keep = Rc::clone(&asked);
    cx.update(|_, cx| {
        cx.subscribe(&line, move |_, asked: &Asked, _| {
            keep.borrow_mut().push(asked.clone());
        })
        .detach();
    });
    cx.update(|window, cx| line.update(cx, |line, cx| line.focus(window, cx)));
    cx.run_until_parked();
    cx.simulate_input("gi");
    let chosen = |cx: &mut VisualTestContext| cx.update(|_, cx| line.read(cx).chosen());
    cx.simulate_keystrokes("ctrl-n");
    assert_eq!(chosen(cx), Row::Page(0));
    cx.simulate_keystrokes("down down");
    assert_eq!(chosen(cx), Row::Line);
    cx.simulate_keystrokes("up ctrl-p");
    assert_eq!(chosen(cx), Row::Page(0));
    assert_eq!(mode(cx), Some(Mode::Insert));
    cx.simulate_keystrokes("enter");
    assert_eq!(
        *asked.borrow(),
        [Asked::Go(
            "https://github.com/zed-industries/zed".to_owned()
        )]
    );
}

#[gpui::test]
fn the_part_with_the_keyboard_has_a_lighter_edge(cx: &mut TestAppContext) {
    use gpui::AppContext as _;

    let built: fake_daemon::Built = Rc::new(move |_, _, cx| {
        let page = cx.new(|cx| FakePage {
            focus: cx.focus_handle(),
            asked: Vec::new(),
            context: "Page",
        });
        cox::chat::view::StageView {
            view: page.clone().into(),
            panel: cx.new(|_| cox::ui::Panel::default()),
            by_keys: Rc::new(move |tabs, window, cx| {
                if tabs == Tabs::Focus {
                    window.focus(&page.read(cx).focus);
                }
            }),
            shows_files: Rc::new(|_| false),
        }
    });
    cx.update(|cx| cox::vim::turn(true, cx));
    let (workspace, cx) = keys::open_window_on(
        cx,
        scratch::folder("keys-edge"),
        FakeStages {
            built: Some(built),
            ..FakeStages::default()
        },
    );
    // GPUI keeps where a selector was drawn once it is no longer drawn, so
    // the test reads which card the one edge was drawn in last.
    let edge_in = |cx: &mut VisualTestContext| {
        let edge = cx.debug_bounds("keys-edge")?;
        let stage = cx.debug_bounds("stage-panel")?;
        if edge.right() <= stage.left() {
            return Some("rail");
        }
        ["stage-panel", "chat-panel"].into_iter().find(|card| {
            cx.debug_bounds(card)
                .is_some_and(|bounds| bounds.contains(&edge.center()))
        })
    };

    keys::press(cx, &workspace, "secondary-n escape");
    assert_eq!(
        edge_in(cx),
        Some("chat-panel"),
        "the chat's input has the keyboard"
    );
    keys::press(cx, &workspace, "ctrl-w h");
    assert!(in_part(cx, "Page"));
    assert_eq!(
        edge_in(cx),
        Some("stage-panel"),
        "ctrl-w h went to the stage"
    );
    keys::press(cx, &workspace, "ctrl-w l");
    assert_eq!(edge_in(cx), Some("chat-panel"), "ctrl-w l came back");
    keys::press(cx, &workspace, "ctrl-w w");
    assert!(in_part(cx, "Rail"));
    assert_eq!(edge_in(cx), Some("rail"), "ctrl-w w went to the rail");
}

/// A stand-in for the stage on the tab of a file, with the tree of files
/// beside it.
struct FilesBeside {
    tree: Entity<Tree>,
    page: Entity<FakePage>,
}

impl gpui::Render for FilesBeside {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};

        gpui::div()
            .size_full()
            .flex()
            .child(
                gpui::div()
                    .w(gpui::px(200.))
                    .h_full()
                    .child(self.tree.clone()),
            )
            .child(gpui::div().flex_1().h_full().child(self.page.clone()))
    }
}

#[gpui::test]
fn control_w_stops_at_the_tree_of_files_as_at_a_window(cx: &mut TestAppContext) {
    use gpui::AppContext as _;

    let root = folder("tree-window", &["a.rs"]);
    let built: fake_daemon::Built = Rc::new(move |_, _, cx| {
        let tree = cx.new(|cx| Tree::new(root.clone(), Vec::new(), cx));
        let page = cx.new(|cx| FakePage {
            focus: cx.focus_handle(),
            asked: Vec::new(),
            context: "Page",
        });
        let (to_tree, to_page) = (tree.clone(), page.clone());
        cox::chat::view::StageView {
            view: cx.new(|_| FilesBeside { tree, page }).into(),
            panel: cx.new(|_| cox::ui::Panel::default()),
            by_keys: Rc::new(move |tabs, window, cx| match tabs {
                Tabs::Focus => window.focus(&to_page.read(cx).focus),
                Tabs::FocusFiles => window.focus(&to_tree.focus_handle(cx)),
                _ => {}
            }),
            shows_files: Rc::new(|_| true),
        }
    });
    cx.update(|cx| cox::vim::turn(true, cx));
    let (workspace, cx) = keys::open_window_on(
        cx,
        scratch::folder("tree-window-chats"),
        FakeStages {
            built: Some(built),
            ..FakeStages::default()
        },
    );
    let tree = "Tree";

    keys::press(cx, &workspace, "secondary-n escape ctrl-w h");
    assert!(in_part(cx, "Page"), "from the chat, the file's tab");
    keys::press(cx, &workspace, "ctrl-w h");
    assert!(in_part(cx, tree), "then the tree, left of it");
    keys::press(cx, &workspace, "ctrl-w h");
    assert!(in_part(cx, "Rail"), "then the rail");
    keys::press(cx, &workspace, "ctrl-w l");
    assert!(in_part(cx, tree), "back to the tree");
    keys::press(cx, &workspace, "ctrl-w l");
    assert!(in_part(cx, "Page"), "and the file");
    keys::press(cx, &workspace, "ctrl-w w ctrl-w w");
    assert!(in_part(cx, "Rail"), "round, past the chat to the rail");
    keys::press(cx, &workspace, "ctrl-w w");
    assert!(in_part(cx, tree), "round, on to the tree");
}

/// The project card comes in normal mode, but an input opened on it, for
/// a row's remote, types at once, and Enter gives what was typed.
#[gpui::test]
fn the_project_card_s_row_input_types_at_once_and_enter_links(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "card-link");
    let folder = scratch::folder("card-link-project");
    let repository = folder.join("directories").join("app");
    std::fs::create_dir_all(&repository).unwrap();
    git(&repository, &["init", "-q"]);
    // With a commit of its own, linking only names the remote, and
    // fetches nothing.
    git(
        &repository,
        &["commit", "-q", "--allow-empty", "-m", "first"],
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    settled(cx, &workspace);
    keys::press(cx, &workspace, "escape");
    workspace.update_in(cx, |workspace, window, cx| {
        let project = workspace.chosen();
        workspace.open_project_card(project, window, cx);
    });
    cx.run_until_parked();
    let card = workspace.read_with(cx, |workspace, _| {
        workspace.project_card().expect("the card is open").clone()
    });
    assert_eq!(
        shown_mode(cx),
        Some("Normal"),
        "the card comes in normal mode"
    );

    let center = cx
        .debug_bounds("project-directory-action-0")
        .expect("the row's Link is drawn")
        .center();
    cx.simulate_click(center, gpui::Modifiers::none());
    assert_eq!(shown_mode(cx), Some("Insert"), "the remote's input types");
    cx.simulate_input("https://example.com/app.git");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    let said = cx.read(|cx| card.read(cx).described(cx));
    assert!(
        said.contains(&"Directory app (Git, remote https://example.com/app.git)".to_owned()),
        "{said:?}"
    );
}

/// The settings come in normal mode, but an input of them clicked types,
/// and Escape leaves it for normal mode before it closes them.
#[gpui::test]
fn an_input_of_the_settings_types_as_it_gets_the_keyboard(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "settings-input");
    keys::press(cx, &workspace, "escape");
    keys::press(cx, &workspace, "secondary-,");
    assert_eq!(shown_mode(cx), Some("Normal"));

    let at = cx
        .debug_bounds("home-input")
        .expect("the home's input is drawn")
        .center();
    cx.simulate_click(at, gpui::Modifiers::none());
    settled(cx, &workspace);
    assert_eq!(shown_mode(cx), Some("Insert"), "the input types");

    keys::press(cx, &workspace, "escape");
    assert_eq!(shown_mode(cx), Some("Normal"));
    assert!(workspace.read_with(cx, |workspace, _| workspace.settings().is_some()));
    keys::press(cx, &workspace, "escape");
    assert!(workspace.read_with(cx, |workspace, _| workspace.settings().is_none()));
}

/// In insert mode Control-W takes out the word before the cursor and
/// Control-U the line before it, as in Vim and a shell; in normal mode
/// they are still the window's and the scroll's.
#[gpui::test]
fn control_w_and_control_u_take_out_what_is_typed_in_insert_mode(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "insert-ctrl");
    keys::press(cx, &workspace, "secondary-n");
    assert_eq!(shown_mode(cx), Some("Insert"));
    cx.simulate_input("say hello world");
    keys::press(cx, &workspace, "ctrl-w");
    assert_eq!(keys::shown_input(cx, &workspace), "say hello ");
    keys::press(cx, &workspace, "ctrl-u");
    assert_eq!(keys::shown_input(cx, &workspace), "");
    assert_eq!(shown_mode(cx), Some("Insert"));

    cx.simulate_input("again");
    keys::press(cx, &workspace, "escape ctrl-w");
    assert!(
        !held(cx).is_empty(),
        "normal mode's Control-W waits for its window key"
    );
    assert_eq!(keys::shown_input(cx, &workspace), "again");
}

/// In normal mode the keys edit the chat's input as Vim edits a line:
/// Escape puts the cursor on the last character, motions move it, and
/// `d`, `c`, `x`, `y` and `p` take out, change, keep and put, with `u`
/// to undo.
#[gpui::test]
fn normal_mode_edits_the_chat_s_input_as_vim_edits_a_line(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "edit-line");
    keys::press(cx, &workspace, "secondary-n");
    cx.simulate_input("say hello world");
    keys::press(cx, &workspace, "escape");
    assert_eq!(shown_mode(cx), Some("Normal"));
    let typed = |cx: &mut VisualTestContext| keys::shown_input(cx, &workspace);

    keys::press(cx, &workspace, "0 w d w");
    assert_eq!(typed(cx), "say world");
    keys::press(cx, &workspace, "u");
    assert_eq!(typed(cx), "say hello world");

    keys::press(cx, &workspace, "$ x");
    assert_eq!(typed(cx), "say hello worl");
    keys::press(cx, &workspace, "b c i w");
    assert_eq!(shown_mode(cx), Some("Insert"), "c goes on to type");
    cx.simulate_input("there");
    keys::press(cx, &workspace, "escape");
    assert_eq!(typed(cx), "say hello there");

    keys::press(cx, &workspace, "0 y w $ p");
    assert_eq!(typed(cx), "say hello theresay ");
    keys::press(cx, &workspace, "d d");
    assert_eq!(typed(cx), "");
    keys::press(cx, &workspace, "u");
    assert_eq!(typed(cx), "say hello theresay ");

    // Escape out of hints leaves the cursor where it is.
    keys::press(cx, &workspace, "$ f escape x");
    assert_eq!(typed(cx), "say hello theresay");
}

/// `/` finds text in the conversation, out of sight too: the entry is
/// scrolled to and the match selected; `n` goes on to the next, round
/// from the end to the start.
#[gpui::test]
fn slash_finds_in_the_conversation_and_n_goes_on(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "chat-search");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    // Pasted, as typing a long line lays the input out again each key.
    let send = |cx: &mut VisualTestContext, line: &str| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(line.to_owned()));
        keys::press(cx, &workspace, "secondary-v enter");
        wait_for_reply(cx, &view);
    };
    send(cx, "the first needle");
    let filler = "filler ".repeat(400);
    for _ in 0..6 {
        send(cx, &filler);
    }
    send(cx, "a second needle");
    keys::press(cx, &workspace, "escape");
    let down = |cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            -view.scroll().scroll_px_offset_for_scrollbar().y
        })
    };
    let end = down(cx);
    assert!(end > px(1000.), "the first needle is far out of sight");
    let selected = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.selected_text());

    keys::press(cx, &workspace, "/ n e e d l e enter");
    assert_eq!(selected(cx), "needle");
    let second = down(cx);
    assert!(second > end - px(1000.), "the second needle, near the end");

    // Each needle has the agent's echo of it after it.
    keys::press(cx, &workspace, "n");
    assert!(down(cx) > end - px(1000.), "the echo of the second");
    keys::press(cx, &workspace, "n");
    assert_eq!(selected(cx), "needle");
    let first = down(cx);
    assert!(
        first < px(1000.),
        "round to the first, near the start: {first:?}"
    );

    keys::press(cx, &workspace, "shift-n");
    assert!(
        down(cx) > end - px(1000.),
        "N goes back round to the second"
    );
    assert_eq!(keys::shown_input(cx, &workspace), "", "nothing was typed");
}

/// `]]` and `[[` select the conversation's entries one after another,
/// from the first in sight, and `gy` copies the one selected.
#[gpui::test]
fn brackets_go_from_entry_to_entry_and_gy_copies_one(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "chat-entries");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    for line in ["alpha", "beta"] {
        cx.simulate_input(line);
        keys::press(cx, &workspace, "enter");
        wait_for_reply(cx, &view);
    }
    keys::press(cx, &workspace, "escape g g");
    let selected = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.selected_text());

    keys::press(cx, &workspace, "] ]");
    assert_eq!(selected(cx), "alpha", "the first entry in sight");
    keys::press(cx, &workspace, "] ]");
    assert_eq!(selected(cx), "echo: alpha");
    keys::press(cx, &workspace, "] ] ] ]");
    assert_eq!(selected(cx), "echo: beta");
    keys::press(cx, &workspace, "[ [");
    assert_eq!(selected(cx), "beta");

    keys::press(cx, &workspace, "g y");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("beta"));
    assert_eq!(keys::shown_input(cx, &workspace), "", "nothing was typed");
}

/// In normal mode Vim's keys scroll the settings and a project's card as
/// they scroll a document, and in an input of them in insert mode they
/// type.
#[gpui::test]
fn the_settings_and_a_project_s_card_scroll_by_vim_s_keys(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "forms-scroll");
    let folder = scratch::folder("forms-scroll-project");
    std::fs::create_dir_all(&folder).unwrap();
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    settled(cx, &workspace);
    keys::press(cx, &workspace, "escape");
    cx.simulate_resize(gpui::size(px(900.), px(320.)));
    settled(cx, &workspace);
    let top = |cx: &mut VisualTestContext, selector: &'static str| {
        cx.debug_bounds(selector)
            .expect("the input is drawn")
            .origin
            .y
    };

    keys::press(cx, &workspace, "secondary-,");
    assert_eq!(shown_mode(cx), Some("Normal"));
    let start = top(cx, "home-input");
    keys::press(cx, &workspace, "j");
    let line = start - top(cx, "home-input");
    assert!(line > px(0.), "{line:?}");
    keys::press(cx, &workspace, "2 j");
    assert_eq!(start - top(cx, "home-input"), line * 3.);
    keys::press(cx, &workspace, "k");
    assert_eq!(start - top(cx, "home-input"), line * 2.);
    keys::press(cx, &workspace, "g g");
    assert_eq!(top(cx, "home-input"), start);
    keys::press(cx, &workspace, "ctrl-d");
    let half = start - top(cx, "home-input");
    assert!(half > line * 4., "{half:?}");
    keys::press(cx, &workspace, "ctrl-u");
    assert_eq!(top(cx, "home-input"), start);
    keys::press(cx, &workspace, "shift-g");
    assert!(start - top(cx, "home-input") > half, "scrolled to the end");
    keys::press(cx, &workspace, "g g");

    let at = cx
        .debug_bounds("home-input")
        .expect("the home's input is drawn")
        .center();
    cx.simulate_click(at, gpui::Modifiers::none());
    settled(cx, &workspace);
    assert_eq!(shown_mode(cx), Some("Insert"));
    keys::press(cx, &workspace, "end j k");
    let typed = workspace.read_with(cx, |workspace, cx| {
        let settings = workspace.settings().expect("the settings are open");
        settings.read(cx).input().read(cx).text().to_owned()
    });
    assert!(typed.ends_with("jk"), "{typed:?}");
    assert_eq!(top(cx, "home-input"), start, "typing scrolls nothing");
    keys::press(cx, &workspace, "escape escape escape");
    assert!(workspace.read_with(cx, |workspace, _| workspace.settings().is_none()));

    workspace.update_in(cx, |workspace, window, cx| {
        let project = workspace.chosen();
        workspace.open_project_card(project, window, cx);
    });
    settled(cx, &workspace);
    assert_eq!(shown_mode(cx), Some("Normal"));
    let start = top(cx, "project-name-input");
    keys::press(cx, &workspace, "j");
    assert!(top(cx, "project-name-input") < start, "the card scrolls");
    keys::press(cx, &workspace, "g g");
    assert_eq!(top(cx, "project-name-input"), start);
    keys::press(cx, &workspace, "shift-g");
    assert!(top(cx, "project-name-input") < start, "to the end");
}

/// `v` and `V` select in the chat's input as in Vim's visual mode, which
/// the window's foot shows: motions grow it, `d`, `c` and `y` take it, and
/// Escape lets it go.
#[gpui::test]
fn v_and_shift_v_select_in_the_chat_s_input_in_visual_mode(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "chat-visual");
    keys::press(cx, &workspace, "secondary-n");
    cx.simulate_input("say hello world");
    keys::press(cx, &workspace, "escape 0 v e");
    assert_eq!(shown_mode(cx), Some("Visual"));
    keys::press(cx, &workspace, "d");
    assert_eq!(keys::shown_input(cx, &workspace), " hello world");
    assert_eq!(shown_mode(cx), Some("Normal"));

    keys::press(cx, &workspace, "w v e c");
    assert_eq!(shown_mode(cx), Some("Insert"));
    cx.simulate_input("there");
    keys::press(cx, &workspace, "escape");
    assert_eq!(keys::shown_input(cx, &workspace), " there world");

    keys::press(cx, &workspace, "shift-v");
    assert_eq!(shown_mode(cx), Some("Visual Line"));
    keys::press(cx, &workspace, "y");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some(" there world\n"));

    keys::press(cx, &workspace, "v $ escape x");
    assert_eq!(shown_mode(cx), Some("Normal"));
    assert_eq!(
        keys::shown_input(cx, &workspace),
        " there worl",
        "Escape let go, leaving the cursor where it went"
    );
}

/// Sends `lines` to the chat shown, one after another, each once the
/// agent is done with the one before.
fn converse(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    view: &Entity<ChatView>,
    lines: &[&str],
) {
    for line in lines {
        cx.simulate_input(line);
        keys::press(cx, workspace, "enter");
        wait_for_reply(cx, view);
    }
}

/// Where the cursor of the conversation is: the entry, the piece of it,
/// and the byte in the piece.
fn reading_cursor(
    cx: &mut VisualTestContext,
    view: &Entity<ChatView>,
) -> Option<(usize, usize, usize)> {
    view.read_with(cx, |view, _| {
        view.reading_cursor()
            .map(|spot| (spot.text.entry, spot.text.part, spot.offset))
    })
}

/// `ctrl-w k` takes the keyboard from the chat's input up to its
/// conversation, in normal mode with a cursor, and on to the list of
/// chats; `ctrl-w j`, `i` and `a` take it back to the input.
#[gpui::test]
fn control_w_k_goes_up_to_the_conversation_and_j_i_and_a_back(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "reading-go");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    converse(cx, &workspace, &view, &["alpha beta"]);
    keys::press(cx, &workspace, "escape ctrl-w k");
    assert!(
        in_part(cx, "ChatReading"),
        "the conversation has the keyboard"
    );
    assert_eq!(shown_mode(cx), Some("Normal"));
    assert_eq!(
        reading_cursor(cx, &view),
        Some((0, 0, 0)),
        "the cursor starts on the first row in sight"
    );

    keys::press(cx, &workspace, "3 escape");
    assert!(
        in_part(cx, "ChatReading"),
        "escape leaves the keyboard there"
    );
    assert_eq!(agent_state(cx, &view), State::Idle);
    assert_eq!(held(cx), "", "escape forgets the count");

    keys::press(cx, &workspace, "ctrl-w j");
    assert!(in_part(cx, "LineInput"), "ctrl-w j goes back to the input");
    keys::press(cx, &workspace, "ctrl-w k ctrl-w k");
    assert!(in_part(cx, "Chats"), "up from the conversation is the list");
    keys::press(cx, &workspace, "escape");
    assert!(in_part(cx, "LineInput"));

    keys::press(cx, &workspace, "ctrl-w k i");
    assert!(in_part(cx, "LineInput"));
    assert_eq!(shown_mode(cx), Some("Insert"));
    cx.simulate_input("xz");
    keys::press(cx, &workspace, "escape h ctrl-w k a");
    assert_eq!(shown_mode(cx), Some("Insert"));
    cx.simulate_input("y");
    assert_eq!(
        keys::shown_input(cx, &workspace),
        "xyz",
        "a types after the input's cursor"
    );

    keys::press(cx, &workspace, "escape ctrl-w k ctrl-w h");
    assert!(
        in_part(cx, "Rail"),
        "left of the conversation, as of the chat"
    );
    keys::press(cx, &workspace, "ctrl-w l");
    assert!(in_part(cx, "LineInput"));
    keys::press(cx, &workspace, "0 x");
    assert_eq!(
        keys::shown_input(cx, &workspace),
        "yz",
        "normal mode edits the input as before"
    );
}

/// In the conversation `h`, `l`, `w`, `b`, `e`, `0` and `$` move the
/// cursor along the text and `j` and `k` from row to row, entry to entry.
#[gpui::test]
fn the_conversation_s_cursor_moves_by_characters_words_and_rows(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "reading-move");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    converse(cx, &workspace, &view, &["alpha beta", "gamma"]);
    keys::press(cx, &workspace, "escape ctrl-w k g g");
    let moves = [
        ("w", (0, 0, 6)),
        ("e", (0, 0, 9)),
        ("b", (0, 0, 6)),
        ("2 h", (0, 0, 4)),
        ("l", (0, 0, 5)),
        ("0", (0, 0, 0)),
        ("$", (0, 0, 9)),
        ("j", (1, 0, 15)),
        ("j", (2, 0, 4)),
        ("j", (3, 0, 10)),
        ("2 k", (1, 0, 15)),
        ("0", (1, 0, 0)),
        ("3 w", (1, 0, 12)),
        ("w", (2, 0, 0)),
        ("b", (1, 0, 12)),
        ("5 e", (3, 0, 10)),
        ("g g", (0, 0, 0)),
    ];
    for (keys, at) in moves {
        keys::press(cx, &workspace, keys);
        assert_eq!(reading_cursor(cx, &view), Some(at), "after {keys}");
    }
    assert_eq!(keys::shown_input(cx, &workspace), "", "nothing was typed");
}

/// `v` selects characters from the cursor and `V` whole pieces of text,
/// motions grow what they select, `y` copies it and `escape` lets it go.
#[gpui::test]
fn visual_mode_selects_in_the_conversation_and_y_copies(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "reading-visual");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    converse(cx, &workspace, &view, &["alpha beta", "gamma"]);
    keys::press(cx, &workspace, "escape ctrl-w k g g");
    let copied = |cx: &mut VisualTestContext| cx.read_from_clipboard().and_then(|item| item.text());
    let selected = |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.selected_text());

    keys::press(cx, &workspace, "w v");
    assert_eq!(shown_mode(cx), Some("Visual"));
    assert_eq!(selected(cx), "b", "the character under the cursor");
    keys::press(cx, &workspace, "e");
    assert_eq!(selected(cx), "beta");
    keys::press(cx, &workspace, "y");
    assert_eq!(copied(cx).as_deref(), Some("beta"));
    assert_eq!(shown_mode(cx), Some("Normal"));
    assert_eq!(selected(cx), "", "y lets go of what it copied");
    assert_eq!(reading_cursor(cx, &view), Some((0, 0, 6)), "at its start");

    keys::press(cx, &workspace, "j l v 3 l y");
    assert_eq!(
        copied(cx).as_deref(),
        Some("lpha"),
        "j keeps to the x, in the echo below"
    );

    keys::press(cx, &workspace, "j shift-v");
    assert_eq!(shown_mode(cx), Some("Visual Line"));
    assert_eq!(selected(cx), "gamma", "the whole piece");
    keys::press(cx, &workspace, "j");
    assert!(
        selected(cx).starts_with("gamma") && selected(cx).ends_with("echo: gamma"),
        "grown to the next piece: {:?}",
        selected(cx)
    );
    keys::press(cx, &workspace, "escape");
    assert_eq!(shown_mode(cx), Some("Normal"));
    assert_eq!(selected(cx), "", "escape lets it go");

    keys::press(cx, &workspace, "g g v $ g a");
    let attached = view.read_with(cx, |view, _| view.attached().to_vec());
    assert_eq!(attached.len(), 1, "ga hands what is selected to the chat");
    assert_eq!(attached[0].text, "alpha beta");
    assert_eq!(keys::shown_input(cx, &workspace), "", "nothing was typed");
}

/// `/`, `n` and `]]` put the conversation's cursor on the start of what
/// they select.
#[gpui::test]
fn finding_in_the_conversation_puts_its_cursor_on_what_is_found(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "reading-find");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    converse(cx, &workspace, &view, &["alpha beta", "gamma"]);
    keys::press(cx, &workspace, "escape ctrl-w k g g");

    keys::press(cx, &workspace, "/ g a m m a enter");
    assert!(in_part(cx, "ChatReading"), "back from the line");
    assert_eq!(reading_cursor(cx, &view), Some((2, 0, 0)));
    keys::press(cx, &workspace, "n");
    assert_eq!(reading_cursor(cx, &view), Some((3, 0, 6)));
    keys::press(cx, &workspace, "[ [");
    assert_eq!(reading_cursor(cx, &view), Some((2, 0, 0)));
    keys::press(cx, &workspace, "[ [");
    assert_eq!(reading_cursor(cx, &view), Some((1, 0, 0)));
    keys::press(cx, &workspace, "e");
    assert_eq!(
        reading_cursor(cx, &view),
        Some((1, 0, 3)),
        "and moves on from there"
    );
}

/// The cursor keeps in sight: the conversation scrolls as it moves, and
/// it moves into sight as the conversation scrolls.
#[gpui::test]
fn the_conversation_s_cursor_keeps_in_sight(cx: &mut TestAppContext) {
    let (workspace, cx) = open_window(cx, "reading-sight");
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    let filler = "filler ".repeat(400);
    for _ in 0..4 {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(filler.clone()));
        keys::press(cx, &workspace, "secondary-v enter");
        wait_for_reply(cx, &view);
    }
    keys::press(cx, &workspace, "escape ctrl-w k g g");
    let down = |cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            -view.scroll().scroll_px_offset_for_scrollbar().y
        })
    };
    assert_eq!(down(cx), px(0.));
    assert_eq!(reading_cursor(cx, &view), Some((0, 0, 0)));

    let mut rows = 0;
    while down(cx) == px(0.) && rows < 500 {
        keys::press(cx, &workspace, "j");
        rows += 1;
    }
    assert!(down(cx) > px(0.), "the rows below scroll into sight");

    keys::press(cx, &workspace, "g g");
    let first = reading_cursor(cx, &view);
    keys::press(cx, &workspace, "ctrl-d");
    assert!(down(cx) > px(0.));
    assert_ne!(
        reading_cursor(cx, &view),
        first,
        "scrolled out of sight, the cursor goes to the first row in sight"
    );

    keys::press(cx, &workspace, "g g");
    let middle = view.read_with(cx, |view, _| view.scroll().viewport_bounds().center());
    cx.simulate_event(ScrollWheelEvent {
        position: middle,
        delta: ScrollDelta::Lines(gpui::point(0., -20.)),
        modifiers: gpui::Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });
    settled(cx, &workspace);
    assert!(down(cx) > px(0.), "the wheel scrolls");
    assert_ne!(
        reading_cursor(cx, &view),
        first,
        "scrolled by the wheel, the cursor goes into sight too"
    );

    keys::press(cx, &workspace, "shift-g");
    let last = reading_cursor(cx, &view).expect("a cursor");
    assert!(last.0 >= 2, "G puts the cursor near the end: {last:?}");
    keys::press(cx, &workspace, "g g");
    assert_eq!(down(cx), px(0.));
    assert_eq!(reading_cursor(cx, &view), Some((0, 0, 0)));
}
