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
    cx.update(|cx| cox::vim::turn(true, cx));
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
    cx.update(|cx| cox::vim::turn(true, cx));
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
    let cwd = scratch(name);
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
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if let Ok(ran) = waiting.try_recv() {
            break ran.expect("the command started").output;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the command took too long"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits until the terminal shows `text`.
fn shows(cx: &mut VisualTestContext, terminal: &Entity<Terminal>, text: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !terminal.read_with(cx, |terminal, _| terminal.text().contains(text)) {
        assert!(
            std::time::Instant::now() < deadline,
            "no {text:?} in the terminal"
        );
        std::thread::sleep(Duration::from_millis(10));
        cx.run_until_parked();
    }
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
    let cwd = scratch("terminal-search");
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
        }
    });
    cx.update(|cx| cox::vim::turn(true, cx));
    let (workspace, cx) = keys::open_window_on(
        cx,
        scratch("session"),
        FakeStages {
            built: Some(built),
            ..FakeStages::default()
        },
    );
    cx.update(|window, _| window.activate_window());

    keys::press(cx, &workspace, "secondary-n h i escape");
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
    assert_eq!(shown_mode(cx), Some("NORMAL"));

    keys::press(cx, &workspace, "ctrl-w h");
    assert!(in_part(cx, "Page"));
    keys::press(cx, &workspace, "g t");
    assert!(in_part(cx, "Terminal"));
    assert_eq!(
        shown_mode(cx),
        Some("NORMAL"),
        "come to by keys, as Neovim's terminal"
    );
    let terminal = terminals.borrow().last().cloned().expect("a terminal");
    shows(cx, &terminal, "300");

    keys::press(cx, &workspace, "i");
    assert_eq!(shown_mode(cx), Some("INSERT"));
    keys::press(cx, &workspace, "e c h o enter");
    shows(cx, &terminal, "echo\necho");
    keys::press(cx, &workspace, "ctrl-\\ ctrl-n");
    assert_eq!(shown_mode(cx), Some("NORMAL"));
    keys::press(cx, &workspace, "g g");
    assert!(terminal.read_with(cx, |terminal, _| terminal.scrolled_back()) > 0);

    keys::press(cx, &workspace, "ctrl-w l");
    assert!(in_part(cx, "Conversation"));
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
    assert_eq!(shown_mode(cx), Some("NORMAL"));

    keys::press(cx, &workspace, "ctrl-w h");
    assert!(
        in_part(cx, "Terminal"),
        "the stage shows the terminal still"
    );
    assert_eq!(shown_mode(cx), Some("NORMAL"), "coming back by keys");
    keys::press(cx, &workspace, ": escape");
    assert!(in_part(cx, "Terminal"));
    assert_eq!(shown_mode(cx), Some("NORMAL"), "back from the line");

    keys::press(cx, &workspace, ": q enter");
    assert!(in_part(cx, "Page"), "the terminal closed");
    keys::press(cx, &workspace, ": n e w enter");
    let chats = workspace.read_with(cx, |workspace, _| workspace.chats(0).count());
    assert_eq!(chats, 2);
    assert!(in_part(cx, "Conversation"));
    assert_eq!(shown_mode(cx), Some("INSERT"));
    assert_eq!(keys::shown_input(cx, &workspace), "");

    keys::press(cx, &workspace, "escape : g o a l enter");
    assert!(in_part(cx, "Goal"));
    assert_eq!(shown_mode(cx), Some("INSERT"), "before a key is typed");
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
    let (workspace, cx) = keys::open_window(cx, scratch("setting"));
    keys::press(cx, &workspace, "secondary-n");
    let view = shown_view(cx, &workspace);
    cx.simulate_input("wait");
    keys::press(cx, &workspace, "enter");
    go_on(cx, &workspace);
    assert_eq!(agent_state(cx, &view), State::Busy);

    choose_on_the_settings(cx, &workspace, "vim-on");
    assert_eq!(kept_vim(), Some(serde_json::Value::Bool(true)));
    assert_eq!(shown_mode(cx), Some("NORMAL"));
    keys::press(cx, &workspace, "i h i escape");
    go_on(cx, &workspace);
    assert_eq!(keys::shown_input(cx, &workspace), "hi");
    assert_eq!(shown_mode(cx), Some("NORMAL"));
    assert_eq!(
        agent_state(cx, &view),
        State::Busy,
        "escape with Vim's keys"
    );

    choose_on_the_settings(cx, &workspace, "vim-off");
    assert_eq!(kept_vim(), None);
    assert_eq!(shown_mode(cx), None);
    keys::press(cx, &workspace, "j");
    assert_eq!(keys::shown_input(cx, &workspace), "hij");
    keys::press(cx, &workspace, "escape");
    go_on(cx, &workspace);
    assert_eq!(
        agent_state(cx, &view),
        State::Idle,
        "escape without Vim's keys"
    );
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
    cx.update(|cx| cox::vim::turn(true, cx));
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
        }
    });
    cx.update(|cx| cox::vim::turn(true, cx));
    let (workspace, cx) = keys::open_window_on(
        cx,
        scratch("search"),
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
    for _ in 0..500 {
        if asked(cx) > 0 {
            break;
        }
        go_on(cx, &workspace);
    }
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
    go_on(cx, &workspace);
    assert_eq!(asked(cx), 0, "the option was chosen");
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
    assert_eq!(shown_mode(cx), Some("INSERT"));
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
