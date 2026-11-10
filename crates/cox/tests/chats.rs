//! A human opening chats in projects, moving between them, reading the
//! closed ones again and going on with them, with each chat's agent faked
//! in memory.

#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;
#[expect(dead_code, reason = "these tests use only part of the fake daemon")]
mod fake_daemon;

use std::cell::RefCell;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use cox::agent::instructions::Instructions;
use cox::agent::mcp::{ChatRequest, Waited};
use cox::agent::setup::{Policy, Setup};
use cox::agents::Agents as Choices;
use cox::chat::conversation::{Check, CheckState, Conversation, Entry, Goal, State};
use cox::chat::sketch::Sketch;
use cox::chat::view::ChatView;
use cox::notices::{AppNotices, Notices};
use cox::ui::input::LineInput;
use cox::workflow;
use cox::workspace::{Chat, Workspace};
use cox_core::entry::Answer;
use cox_core::goal::Goal as KeptGoal;
use cox_core::settings::{Choice, Setting};
use cox_core::store::Named;
use cox_core::store::Store;
use fake_agent::{FakeAgent, LogsIn, Resumes, TITLE};
use fake_daemon::{
    Agents, Asks, FIRST_AGENT, FakeStages, Fakes, GIVEN_UP, Kept, Made, SECOND_AGENT, kept_at,
    open_again, settle, settle_on, settled,
};
use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point,
    TestAppContext, VisualTestContext,
};

/// Where a test keeps its chats, which it opens too, to keep chats there
/// first, or to read what the daemon kept.
#[derive(Clone)]
struct KeptStore {
    path: PathBuf,
    store: Rc<Store>,
}

impl Deref for KeptStore {
    type Target = Store;

    fn deref(&self) -> &Store {
        &self.store
    }
}

/// A store of its own for the test named `name`.
fn kept_store(name: &str) -> KeptStore {
    let path = kept_at(name);
    let store = Rc::new(Store::open(&path).unwrap());
    KeptStore { path, store }
}

/// Opens the window with no chat, Cox's own chats working in `home`,
/// keeping them in memory.
fn open_window(
    cx: &mut TestAppContext,
    home: PathBuf,
) -> (Agents, Entity<Workspace>, &mut VisualTestContext) {
    let fakes = Fakes::default();
    let agents = fakes.agents.clone();
    let (workspace, cx) = open_with(cx, home, Kept::Memory, fakes);
    (agents, workspace, cx)
}

/// Opens the window with chats with no stage, each with its agent faked
/// as `fakes` says, the commands a chat runs for its goal running in a
/// shell in place of a terminal. A chat chooses between two agents,
/// [`FIRST_AGENT`] and [`SECOND_AGENT`], which are faked alike.
fn open_with(
    cx: &mut TestAppContext,
    home: PathBuf,
    kept: Kept,
    fakes: Fakes,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (workspace, cx) = fake_daemon::open_window(cx, home, kept, fakes, FakeStages::default());
    settled(cx, &workspace);
    (workspace, cx)
}

/// Opens the window like [`open_window`], keeping the chats in `store`.
fn open_window_keeping(
    cx: &mut TestAppContext,
    home: PathBuf,
    store: KeptStore,
) -> (Agents, Entity<Workspace>, &mut VisualTestContext) {
    open_window_resuming(cx, home, store, Resumes::Resume)
}

/// Opens the window like [`open_window_keeping`], with agents that go on
/// with a closed chat's session as `resumes` says.
fn open_window_resuming(
    cx: &mut TestAppContext,
    home: PathBuf,
    store: KeptStore,
    resumes: Resumes,
) -> (Agents, Entity<Workspace>, &mut VisualTestContext) {
    let (agents, _, workspace, cx) = open_window_asking(cx, home, store, resumes);
    (agents, workspace, cx)
}

/// Opens the window like [`open_window_resuming`], with the way to the
/// daemon of each chat's agent too.
fn open_window_asking(
    cx: &mut TestAppContext,
    home: PathBuf,
    store: KeptStore,
    resumes: Resumes,
) -> (Agents, Asks, Entity<Workspace>, &mut VisualTestContext) {
    let fakes = Fakes {
        resumes,
        ..Fakes::default()
    };
    let (agents, asks) = (fakes.agents.clone(), fakes.asks.clone());
    let (workspace, cx) = open_with(cx, home, Kept::At(store.path), fakes);
    (agents, asks, workspace, cx)
}

/// The window of Cox started again, keeping its chats in `store`, its
/// agents added to `agents`, as a workspace the window does not show.
fn start_again(
    cx: &mut VisualTestContext,
    home: PathBuf,
    store: &KeptStore,
    agents: &Agents,
) -> Entity<Workspace> {
    let fakes = Fakes {
        agents: agents.clone(),
        ..Fakes::default()
    };
    open_again(cx, home, Kept::At(store.path.clone()), fakes)
}

/// Opens the window like [`open_window`], with agents that ask to log in
/// as `logs_in` says. The chats talk to `touch`, so that the agent's way to
/// log in in a terminal, which names a file, makes it.
fn open_window_logging_in(
    cx: &mut TestAppContext,
    home: PathBuf,
    logs_in: LogsIn,
) -> (Agents, Entity<Workspace>, &mut VisualTestContext) {
    let fakes = Fakes {
        logs_in,
        choices: Choices::new(vec!["touch".to_owned()], Vec::new(), Vec::new()),
        ..Fakes::default()
    };
    let agents = fakes.agents.clone();
    let (workspace, cx) = open_with(cx, home, Kept::Memory, fakes);
    (agents, workspace, cx)
}

/// A directory of its own for each test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("cox-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

/// Clicks what `selector` names, once the daemon has settled, so that it
/// is drawn as the daemon has it.
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = at(cx, selector);
    cx.simulate_click(position, Modifiers::none());
}

/// Where what `selector` names is drawn, once the daemon has settled,
/// opening the list of chats for it when it is in the list.
fn at(cx: &mut VisualTestContext, selector: &'static str) -> Point<Pixels> {
    assert!(drawn(cx, selector), "{selector} is drawn");
    cx.debug_bounds(selector).expect("it is drawn").center()
}

/// Whether what `selector` names is drawn, once the daemon has settled,
/// opening the list of chats first when it is in the list. What a frame
/// drew stays among the bounds the test reads, so the list is asked
/// whether it is open, not whether it was drawn.
fn drawn(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    let workspace = root(cx);
    settled(cx, &workspace);
    let in_list = ["chat-", "new-chat-in-", "archived"]
        .iter()
        .any(|start| selector.starts_with(start))
        && !selector.starts_with("chat-switcher");
    if in_list && !workspace.read_with(cx, |workspace, _| workspace.is_switching()) {
        let switcher = cx
            .debug_bounds("chat-switcher")
            .expect("the head of the panel is drawn");
        cx.simulate_click(switcher.center(), Modifiers::none());
    }
    cx.debug_bounds(selector).is_some()
}

/// The view of the chat at `index` of Cox's own, made for it unless
/// it had one, once the daemon has told it the chat's conversation.
fn view_at(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    index: usize,
) -> Entity<ChatView> {
    settled(cx, workspace);
    let view = workspace.update(cx, |workspace, cx| {
        let key = workspace
            .chats(0)
            .nth(index)
            .expect("the chat is listed")
            .key();
        workspace.watch(key, cx).expect("the chat has a view")
    });
    settled(cx, workspace);
    view
}

/// The workspace the window shows.
fn root(cx: &mut VisualTestContext) -> Entity<Workspace> {
    cx.update(|window, _| window.root::<Workspace>())
        .flatten()
        .expect("the window shows a workspace")
}

/// The view of the chat shown, once the daemon has settled.
fn shown(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Entity<ChatView> {
    settled(cx, workspace);
    workspace.read_with(cx, |workspace, _| {
        let chat = workspace.shown().expect("a chat is shown");
        chat.view().expect("the chat shown has a view").clone()
    })
}

/// The name of each project, with the names of the chats in it.
fn names(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Vec<(String, Vec<String>)> {
    settled(cx, workspace);
    workspace.read_with(cx, |workspace, _| {
        workspace
            .projects()
            .iter()
            .enumerate()
            .map(|(index, project)| {
                let chats = workspace.chats(index).map(|chat| chat.name().to_owned());
                (project.name().to_owned(), chats.collect())
            })
            .collect()
    })
}

/// `names`, as `names` returns them.
fn listed(names: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    names
        .iter()
        .map(|(project, chats)| {
            let chats = chats.iter().map(|&chat| chat.to_owned());
            ((*project).to_owned(), chats.collect())
        })
        .collect()
}

fn input_text(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> String {
    view.read_with(cx, |view, cx| view.input().read(cx).text().to_owned())
}

/// Sends `text` in the chat shown, and waits for the reply.
async fn say(cx: &mut VisualTestContext, workspace: &Entity<Workspace>, text: &str) {
    let view = shown(cx, workspace);
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    wait_for_turn(cx, &view).await;
}

/// Waits until a turn of the chat `view` has ended.
async fn wait_for_turn(cx: &mut VisualTestContext, view: &Entity<ChatView>) {
    settle(cx).await;
    let (timed_out, timeout) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(GIVEN_UP);
        let _ = timed_out.send(());
    });
    tokio::select! {
        () = cx.condition(view, |view, _| {
            view.conversation().state() == State::Idle && !view.conversation().entries().is_empty()
        }) => {}
        _ = timeout => panic!("the agent never answered"),
    }
    settle(cx).await;
}

#[gpui::test]
async fn chats_in_a_project_work_in_its_folder(cx: &mut TestAppContext) {
    let home = scratch("home");
    let folder = scratch("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let (agents, workspace, cx) = open_window(cx, home.clone());

    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    for line in ["one", "two"] {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open(Some(folder.clone()), window, cx);
        });
        say(cx, &workspace, line).await;
    }
    // "New chat" in the list of chats opens a chat in the project chosen
    // on the rail, and so does Cox's own, chosen.
    click(cx, "new-chat-in-2");
    click(cx, "project-1");
    click(cx, "new-chat-in-1");

    let cwds: Vec<_> = agents.borrow().iter().map(|agent| agent.cwd()).collect();
    assert_eq!(
        cwds[..3],
        [
            Some(home.clone()),
            Some(folder.clone()),
            Some(folder.clone())
        ]
    );
    assert!(home.is_dir(), "the home is made for the chat");
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    // A chat is named after the first line sent in it, and numbered until
    // then.
    assert_eq!(
        names(cx, &workspace),
        listed(&[
            ("Cox", &["hello", "Chat 2"]),
            (&name, &["one", "two", "Chat 3"]),
        ])
    );
    let shown = workspace.read_with(cx, |workspace, _| {
        workspace.shown().map(|chat| chat.name().to_owned())
    });
    assert_eq!(shown.as_deref(), Some("Chat 2"));
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn the_chats_of_a_folder_share_its_stage(cx: &mut TestAppContext) {
    let home = scratch("stage-home");
    let folder = scratch("stage-folder");
    std::fs::create_dir_all(&folder).unwrap();
    let made = Made::default();
    let stages = FakeStages {
        opened: None,
        made: Some(Rc::clone(&made)),
    };
    let (workspace, cx) =
        fake_daemon::open_window(cx, home.clone(), Kept::Memory, Fakes::default(), stages);
    settled(cx, &workspace);

    cx.simulate_keystrokes("secondary-n");
    for _ in 0..2 {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open(Some(folder.clone()), window, cx);
        });
    }
    settle(cx).await;
    click(cx, "project-1");
    settle(cx).await;

    // A stage for each folder, which each of its chats shows.
    let folders: Vec<PathBuf> = made
        .borrow()
        .iter()
        .map(|(folder, _)| folder.clone())
        .collect();
    assert_eq!(folders, [home.clone(), folder.clone()]);
    let view = shown(cx, &workspace);
    assert!(view.read_with(cx, ChatView::stage_in_sight));
    click(cx, "project-2");
    for chat in ["chat-2", "chat-3"] {
        click(cx, chat);
        let view = shown(cx, &workspace);
        assert!(
            view.read_with(cx, ChatView::stage_in_sight),
            "{chat} has the folder's stage"
        );
    }
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn the_rail_chooses_the_project_whose_chats_the_tabs_show(cx: &mut TestAppContext) {
    let home = scratch("rail-home");
    let folder = scratch("rail-folder");
    std::fs::create_dir_all(&folder).unwrap();
    let (_agents, workspace, cx) = open_window(cx, home.clone());
    let shown_name = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, _| {
            workspace.shown().map(|chat| chat.name().to_owned())
        })
    };

    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "here").await;
    cx.simulate_keystrokes("secondary-n");
    click(cx, "chat-1");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    say(cx, &workspace, "there").await;

    // A project chosen shows the chat of it shown last.
    click(cx, "project-1");
    assert_eq!(shown_name(cx).as_deref(), Some("here"));
    click(cx, "project-2");
    assert_eq!(shown_name(cx).as_deref(), Some("there"));

    // A project with no chat left shows none.
    workspace.update_in(cx, |workspace, window, cx| workspace.archive(2, window, cx));
    click(cx, "project-2");
    assert_eq!(shown_name(cx), None);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn the_settings_set_where_coxs_own_chats_work(cx: &mut TestAppContext) {
    let home = scratch("settings-home");
    let other = scratch("settings-other");
    let (agents, workspace, cx) = open_window(cx, home.clone());
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "before").await;

    // The gear opens the settings, with the keyboard in the home's input.
    click(cx, "open-settings");
    let settings = workspace.read_with(cx, |workspace, _| {
        workspace.settings().expect("the settings are open").clone()
    });
    let typed = |cx: &mut VisualTestContext| {
        settings.read_with(cx, |settings, cx| {
            settings.input().read(cx).text().to_owned()
        })
    };
    assert_eq!(typed(cx), home.display().to_string());

    // What is not a whole path is not set.
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("elsewhere");
    cx.simulate_keystrokes("enter");
    let kept = workspace.read_with(cx, |workspace, _| workspace.home().to_owned());
    assert_eq!(kept, home);

    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(&other.display().to_string());
    cx.simulate_keystrokes("enter");
    let kept = workspace.read_with(cx, |workspace, _| workspace.home().to_owned());
    assert_eq!(kept, other);
    assert_eq!(
        cox::settings::Saved::read().home.as_deref(),
        Some(other.as_path()),
        "the home lasts across runs"
    );

    // Escape closes them, and the next chat of Cox's own works there.
    cx.simulate_keystrokes("escape");
    let open = workspace.read_with(cx, |workspace, _| workspace.settings().is_some());
    assert!(!open);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "after").await;
    let cwds: Vec<_> = agents.borrow().iter().map(|agent| agent.cwd()).collect();
    assert_eq!(cwds, [Some(home.clone()), Some(other.clone())]);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&other);
}

#[gpui::test]
async fn a_new_project_is_made_in_the_projects_of_the_home(cx: &mut TestAppContext) {
    let home = scratch("new-project-home");
    let projects = home.join("projects");
    // A folder there that is no project's is not taken.
    std::fs::create_dir_all(projects.join("untitled-2")).unwrap();
    let (agents, workspace, cx) = open_window(cx, home.clone());

    // The + at the foot of the rail makes one, and so do its keys.
    click(cx, "new-project");
    say(cx, &workspace, "one").await;
    cx.simulate_keystrokes("secondary-shift-n");
    say(cx, &workspace, "two").await;

    let cwds: Vec<_> = agents.borrow().iter().map(|agent| agent.cwd()).collect();
    assert_eq!(
        cwds,
        [
            Some(projects.join("untitled-1")),
            Some(projects.join("untitled-3"))
        ]
    );
    assert_eq!(
        names(cx, &workspace),
        listed(&[
            ("Cox", &[]),
            ("untitled-1", &["one"]),
            ("untitled-3", &["two"]),
        ])
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
fn opening_the_home_as_a_folder_opens_a_chat_of_coxs_own(cx: &mut TestAppContext) {
    let home = scratch("home-folder");
    let (_agents, workspace, cx) = open_window(cx, home.clone());

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(home.clone()), window, cx);
    });

    assert_eq!(names(cx, &workspace), listed(&[("Cox", &["Chat 1"])]));
    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
async fn the_agent_gets_the_projects_instructions_with_the_first_line(cx: &mut TestAppContext) {
    let home = scratch("instructions-home");
    let folder = scratch("instructions");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("COX.md"), "Answer in Japanese.\n").unwrap();
    let (agents, workspace, cx) = open_window(cx, home.clone());

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    say(cx, &workspace, "hello").await;
    say(cx, &workspace, "again").await;
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;

    let instructions = Instructions::project(&folder).unwrap();
    assert_eq!(
        agents.borrow()[0].prompt_blocks(),
        [
            [instructions.prompts(), vec!["hello".to_owned()]].concat(),
            vec!["again".to_owned()],
        ]
    );
    // The home has no instructions.
    assert_eq!(
        agents.borrow()[1].prompt_blocks(),
        [vec!["hello".to_owned()]]
    );
    // Each new session is told first how to work with Cox's tools, as Cox
    // has it for a human who keeps no instructions of their own.
    for agent in agents.borrow().iter() {
        let told = agent.told_by_cox();
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(told[0].contains("start_sketch"), "{told:?}");
    }
    click(cx, "project-2");
    click(cx, "chat-2");
    let view = shown(cx, &workspace);
    let first = view.read_with(cx, |view, _| view.conversation().entries()[0].clone());
    let Entry::Note(note) = first else {
        panic!("the chat opens with a note, not {first:?}");
    };
    assert!(note.starts_with("Instructions: "), "{note}");
    assert!(note.ends_with("COX.md"), "{note}");
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn a_command_goes_first_and_the_instructions_with_the_next_line(cx: &mut TestAppContext) {
    let home = scratch("command-home");
    let folder = scratch("command");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("COX.md"), "Answer in Japanese.\n").unwrap();
    let (agents, workspace, cx) = open_window(cx, home.clone());

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    let command = format!("/{}", fake_agent::COMMAND);
    say(cx, &workspace, &command).await;
    say(cx, &workspace, "hello").await;

    // The chat knows the agent's commands, for the list over its input.
    let view = shown(cx, &workspace);
    let commands = view.read_with(cx, |view, _| {
        view.conversation()
            .commands()
            .iter()
            .map(|command| command.name.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(commands, [fake_agent::COMMAND]);
    // The agent knows a command by the prompt's first block.
    let instructions = Instructions::project(&folder).unwrap();
    assert_eq!(
        agents.borrow()[0].prompt_blocks(),
        [
            vec![command],
            [instructions.prompts(), vec!["hello".to_owned()]].concat()
        ]
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn the_files_a_line_points_to_go_with_it(cx: &mut TestAppContext) {
    let home = scratch("mention-home");
    let folder = scratch("mention");
    std::fs::create_dir_all(folder.join("docs")).unwrap();
    std::fs::write(folder.join("docs/notes.md"), "# Notes\n").unwrap();
    std::fs::write(folder.join("a logo.png"), [0x89, b'P', b'N', b'G', 0xff]).unwrap();
    let (agents, workspace, cx) = open_window(cx, home.clone());

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    let line = "read @docs/notes.md, @\"a logo.png\" and @gone.rs";
    say(cx, &workspace, line).await;

    // The line goes as written, then each file that is there: a text
    // file whole, as the agent takes it, and another as a link.
    let notes = folder.join("docs/notes.md");
    let logo = folder.join("a logo.png");
    assert_eq!(
        agents.borrow()[0].prompt_blocks(),
        [vec![
            line.to_owned(),
            format!("file file://{}: # Notes\n", notes.display()),
            format!(
                "link file://{}",
                logo.display().to_string().replace(' ', "%20")
            ),
        ]]
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn switching_keeps_each_chats_conversation_and_draft(cx: &mut TestAppContext) {
    let (agents, workspace, cx) = open_window(cx, scratch("switching"));

    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "first").await;
    cx.simulate_keystrokes("secondary-n");
    let second = shown(cx, &workspace);
    cx.simulate_input("not sent yet");

    click(cx, "chat-1");
    let first = shown(cx, &workspace);
    assert_ne!(first, second);
    say(cx, &workspace, "again").await;

    cx.simulate_keystrokes("ctrl-tab");
    assert_eq!(shown(cx, &workspace), second);
    assert_eq!(input_text(cx, &second), "not sent yet");
    let replies = |cx: &mut VisualTestContext, view: &Entity<ChatView>| {
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
async fn how_cox_stands_is_told_in_words(cx: &mut TestAppContext) {
    let home = scratch("stands");
    let (_, workspace, cx) = open_window(cx, home.clone());
    let describe = |cx: &mut VisualTestContext, chat: Option<u64>| {
        cx.update(|window, cx| cox::inspect::describe(workspace.read(cx), chat, None, window, cx))
    };

    let told = describe(cx, None);
    assert!(told.contains("The chats, as listed:\n  none\n"), "{told}");
    assert!(
        told.contains("the keyboard is in the window, which shows no chat"),
        "{told}"
    );
    assert!(told.ends_with("\nNo chat is shown.\n"), "{told}");

    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "first").await;
    cx.simulate_keystrokes("secondary-n");
    let view = shown(cx, &workspace);
    cx.simulate_input("not sent yet");
    wait_until(cx, &view, |conversation| {
        !conversation.settings().is_empty()
    })
    .await;

    let told = describe(cx, None);
    let listed = format!(
        "The chats, as listed:\n  Cox ({}), chosen\n    chat 1 \"first\": idle\n    chat 2 \"Chat 2\": idle, shown\n",
        home.display()
    );
    assert!(told.contains(&listed), "{told}");
    assert!(told.contains("the keyboard is in the input"), "{told}");
    assert!(
        told.ends_with(
            "Chat 2 \"Chat 2\":\n  idle\n  Input: \"not sent yet\"\n  \
            Agent \"fake-first\" (fake-first), of 2 agents\n  \
            Setting \"Model\": \"Large\", of Large, Small\n  Setting \"Mode\": \"Ask\", of Ask, Plan\n  \
            Stage: none\n  Conversation, 0 entries:\n"
        ),
        "{told}"
    );
    let told = describe(cx, Some(1));
    assert!(
        told.ends_with(
            "Chat 1 \"first\":\n  idle\n  Not shown: its input and stage are as they were left.\n  \
            Input: \"\"\n  Agent \"fake-first\" (fake-first), of 2 agents\n  \
            Setting \"Model\": \"Large\", of Large, Small\n  \
            Setting \"Mode\": \"Ask\", of Ask, Plan\n  Stage: none\n  Conversation, 2 entries:\n    1 human: first\n    2 reply: echo: first\n"
        ),
        "{told}"
    );
    assert!(describe(cx, Some(9)).ends_with("\nThere is no chat 9.\n"));
}

/// The entries of the chat `view` shows.
fn entries(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> Vec<Entry> {
    let workspace = root(cx);
    settled(cx, &workspace);
    view.read_with(cx, |view, _| view.conversation().entries().to_vec())
}

/// The entries of the chat shown.
fn shown_entries(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Vec<Entry> {
    let view = shown(cx, workspace);
    entries(cx, &view)
}

/// Whether each chat of each project is open.
fn opened(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Vec<Vec<bool>> {
    settled(cx, workspace);
    workspace.read_with(cx, |workspace, _| {
        (0..workspace.projects().len())
            .map(|project| workspace.chats(project).map(Chat::is_open).collect())
            .collect()
    })
}

#[gpui::test]
fn a_closed_chat_is_there_to_read(cx: &mut TestAppContext) {
    let store = kept_store("chats-1");
    let kept = store.add_chat(None, "Chat 1").unwrap();
    let said = [
        Entry::Human("hello".to_owned()),
        Entry::Reply("echo: hello".to_owned()),
        Entry::Note("[EndTurn]".to_owned()),
    ];
    for (number, entry) in said.iter().enumerate() {
        store.keep_entry(kept, number, entry).unwrap();
    }
    let (agents, workspace, cx) = open_window_keeping(cx, scratch("closed"), store);
    cx.simulate_keystrokes("secondary-n");

    click(cx, "chat-1");
    let closed = shown(cx, &workspace);
    assert_eq!(shown_entries(cx, &workspace), said);
    assert_eq!(
        closed.read_with(cx, |view, _| view.conversation().state()),
        State::Closed
    );
    assert_eq!(
        agents.borrow().len(),
        1,
        "reading a closed chat starts no agent"
    );
    // Moving away and back reads it as it was.
    click(cx, "chat-2");
    click(cx, "chat-1");
    assert_eq!(shown(cx, &workspace), closed);
    assert_eq!(opened(cx, &workspace), [vec![false, true]]);
}

#[gpui::test]
async fn chats_are_there_to_read_once_cox_starts_again(cx: &mut TestAppContext) {
    let home = scratch("again-home");
    let folder = scratch("again-folder");
    std::fs::create_dir_all(&folder).unwrap();
    let store = kept_store("again-kept");
    let (agents, workspace, cx) = open_window_keeping(cx, home.clone(), store.clone());
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    let view = shown(cx, &workspace);
    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    wait_until(cx, &view, |conversation| {
        conversation.state() == State::Asking
    })
    .await;
    click(cx, "option-1");
    wait_for_turn(cx, &view).await;
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let first = shown_entries(cx, &workspace);
    // The chat is in the project, whose tabs the rail shows.
    click(cx, "project-2");
    click(cx, "chat-2");
    let second = shown_entries(cx, &workspace);

    // As Cox opens its window when it starts again, with every chat
    // closed.
    let again = start_again(cx, home.clone(), &store, &agents);
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        names(cx, &again),
        listed(&[("Cox", &["hello"]), (&name, &["permission"])])
    );
    assert_eq!(opened(cx, &again), [vec![false], vec![false]]);
    let read = |cx: &mut VisualTestContext, place| {
        again.update_in(cx, |workspace, window, cx| {
            workspace.show(place, window, cx);
        });
        shown_entries(cx, &again)
    };
    assert_eq!(read(cx, 0), first);
    let read_second = read(cx, 1);
    assert_eq!(read_second, second);
    // The change the agent asked to make shows, with the option chosen.
    assert!(
        read_second.iter().any(|entry| matches!(
            entry,
            Entry::Permission { chosen: Some(0), content, .. } if !content.is_empty()
        )),
        "{read_second:?}"
    );

    // A new chat in a project kept goes under it, named after those kept.
    again.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    assert_eq!(
        names(cx, &again),
        listed(&[("Cox", &["hello"]), (&name, &["permission", "Chat 2"])])
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

#[gpui::test]
async fn stopping_with_the_button_leaves_the_keyboard_in_the_input(cx: &mut TestAppContext) {
    let (agents, workspace, cx) = open_window(cx, scratch("stop"));
    cx.simulate_keystrokes("secondary-n");
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

#[gpui::test]
async fn a_line_sent_while_the_agent_works_goes_into_its_turn(cx: &mut TestAppContext) {
    let (agents, workspace, cx) = open_window(cx, scratch("steer"));
    cx.simulate_keystrokes("secondary-n");
    let view = shown(cx, &workspace);

    cx.simulate_input("wait");
    cx.simulate_keystrokes("enter");
    settle(cx).await;
    cx.condition(&view, |view, _| view.conversation().steers())
        .await;
    say(cx, &workspace, "and then this").await;

    assert_eq!(input_text(cx, &view), "");
    assert_eq!(agents.borrow()[0].cancels(), 0);
    let entries = cx.read(|cx| view.read(cx).conversation().entries().to_vec());
    assert_eq!(
        entries,
        [
            Entry::Human("wait".to_owned()),
            Entry::Reply("waiting".to_owned()),
            Entry::Human("and then this".to_owned()),
            Entry::Reply("echo: and then this".to_owned()),
        ]
    );
}

/// What the closed chat of [`kept_chat`] showed.
fn said() -> Vec<Entry> {
    vec![
        Entry::Human("hello".to_owned()),
        Entry::Reply("echo: hello".to_owned()),
        Entry::Note("[EndTurn]".to_owned()),
    ]
}

/// A store keeping one chat, in the project of `folder`, which showed
/// [`said`], and whose agent's session was `session`, if any.
fn kept_chat(
    folder: &std::path::Path,
    session: Option<&str>,
) -> (KeptStore, cox_core::store::ChatId) {
    let name = folder.file_name().unwrap().to_string_lossy();
    let store = kept_store(&format!("kept-{name}"));
    // Its chats start again in it while it is there.
    std::fs::create_dir_all(folder).unwrap();
    let project = store.project(folder, "project").unwrap();
    let kept = store.add_chat(Some(project), "Chat 1").unwrap();
    for (number, entry) in said().iter().enumerate() {
        store.keep_entry(kept, number, entry).unwrap();
    }
    if let Some(session) = session {
        store.keep_session(kept, session).unwrap();
    }
    (store, kept)
}

/// The chat kept, at the top of the list: the first project's only one,
/// after Cox's own, which has none.
const FIRST_KEPT: usize = 0;

#[gpui::test]
async fn a_closed_chat_goes_on_with_its_session_when_the_human_sends_to_it(
    cx: &mut TestAppContext,
) {
    let folder = scratch("resume");
    let (store, kept) = kept_chat(&folder, Some("session-1"));
    let (agents, workspace, cx) =
        open_window_resuming(cx, scratch("resume-home"), store.clone(), Resumes::Resume);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });
    assert_eq!(opened(cx, &workspace), [vec![], vec![false]]);

    say(cx, &workspace, "again").await;

    let agent = Arc::clone(&agents.borrow()[0]);
    assert_eq!(agent.resumed(), ["session-1"]);
    assert_eq!(agent.sessions_opened(), 0);
    // The agent remembers the chat, so it is told nothing more.
    assert_eq!(agent.prompt_blocks(), [vec!["again".to_owned()]]);
    let mut went_on = said();
    went_on.extend([
        Entry::Human("again".to_owned()),
        Entry::Note("Resumed: the agent goes on from here.".to_owned()),
        Entry::Reply("echo: again".to_owned()),
    ]);
    assert_eq!(shown_entries(cx, &workspace), went_on);
    assert_eq!(opened(cx, &workspace), [vec![], vec![true]]);
    // It goes on being kept.
    assert_eq!(store.entries(kept).unwrap(), went_on);
    assert_eq!(store.session(kept).unwrap().as_deref(), Some("session-1"));

    // It talks on with the same agent.
    say(cx, &workspace, "more").await;
    assert_eq!(agents.borrow().len(), 1);
    assert_eq!(agent.prompts(), ["again", "more"]);
}

#[gpui::test]
fn a_chat_whose_folder_is_gone_does_not_start_again(cx: &mut TestAppContext) {
    let folder = scratch("gone-from");
    let (store, _) = kept_chat(&folder, Some("session-1"));
    std::fs::rename(&folder, scratch("gone-to")).unwrap();
    let (agents, workspace, cx) =
        open_window_resuming(cx, scratch("gone-home"), store.clone(), Resumes::Resume);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });
    let view = shown(cx, &workspace);
    // Its folder is not there, so its agent does not start again.
    let waiting = view.read_with(cx, |view, _| view.waiting());
    assert_eq!(
        waiting.as_deref(),
        Some("Its folder is gone: its agent starts again once it is back.")
    );
    assert!(agents.borrow().is_empty());

    // The card says so, and does not rename it.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_project_card(1, window, cx);
    });
    let card = workspace.read_with(cx, |workspace, _| {
        workspace.project_card().expect("the card is open").clone()
    });
    let described = card.read_with(cx, |card, cx| card.described(cx).join("\n"));
    assert!(
        described.contains("Cox cannot find the project's folder"),
        "{described}"
    );
    let input = card.read_with(cx, |card, _| card.name_input().clone());
    input.update(cx, |input, cx| input.set_text("elsewhere", cx));
    cx.simulate_keystrokes("enter");
    settled(cx, &workspace);
    assert_eq!(store.projects().unwrap()[0].folder, folder);
}

#[gpui::test]
async fn renaming_a_project_renames_its_folder(cx: &mut TestAppContext) {
    let projects = scratch("rename-projects");
    let app = projects.join("untitled-1");
    let (store, kept) = kept_chat(&app, Some("session-1"));
    std::fs::create_dir_all(projects.join("taken")).unwrap();
    // Its directory named as it follows its name, and the others stay.
    std::fs::create_dir_all(app.join("directories").join("untitled-1")).unwrap();
    std::fs::create_dir_all(app.join("directories").join("lib")).unwrap();
    let (agents, workspace, cx) =
        open_window_resuming(cx, scratch("rename-home"), store.clone(), Resumes::Resume);
    let name = |cx: &mut VisualTestContext| {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_project_card(1, window, cx);
        });
        let card = workspace.read_with(cx, |workspace, _| {
            workspace.project_card().expect("the card is open").clone()
        });
        card.read_with(cx, |card, _| card.name_input().clone())
    };

    // The card holds the folder's name.
    let input = name(cx);
    assert_eq!(
        input.read_with(cx, |input, _| input.text().to_owned()),
        "untitled-1"
    );
    // A name another folder there has is not taken, nor one that is no
    // folder's.
    for refused in ["taken", "a/b", ".hidden", ""] {
        input.update(cx, |input, cx| input.set_text(refused, cx));
        cx.simulate_keystrokes("enter");
        settled(cx, &workspace);
        assert_eq!(store.projects().unwrap()[0].folder, app, "{refused:?}");
    }

    // Enter renames the project, and its folder with it.
    input.update(cx, |input, cx| input.set_text("shop", cx));
    cx.simulate_keystrokes("enter");
    settled(cx, &workspace);
    let shop = projects.join("shop");
    let kept_projects = store.projects().unwrap();
    assert_eq!(
        (kept_projects[0].name.as_str(), &kept_projects[0].folder),
        ("shop", &shop)
    );
    assert!(shop.is_dir() && !app.exists());
    assert!(shop.join("directories").join("shop").is_dir());
    assert!(!shop.join("directories").join("untitled-1").exists());
    assert!(shop.join("directories").join("lib").is_dir());
    assert_eq!(store.session(kept).unwrap(), None);
    cx.simulate_keystrokes("escape");

    // Closing the card renames it too.
    let input = name(cx);
    input.update(cx, |input, cx| input.set_text("store", cx));
    cx.simulate_keystrokes("escape");
    settled(cx, &workspace);
    let store_folder = projects.join("store");
    assert_eq!(store.projects().unwrap()[0].folder, store_folder);
    assert!(store_folder.is_dir());
    assert!(store_folder.join("directories").join("store").is_dir());

    // Its chat starts again there, told the conversation so far.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });
    say(cx, &workspace, "again").await;
    let agent = Arc::clone(&agents.borrow()[0]);
    assert_eq!(agent.cwd(), Some(store_folder));
    assert_eq!(agent.resumed(), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&projects);
}

#[gpui::test]
fn a_project_is_archived_and_deleted_from_its_menu(cx: &mut TestAppContext) {
    let root = scratch("project-away");
    let trash = root.join("Trash");
    cox::trash::keep_in(trash.clone());
    let app = root.join("projects").join("away-app");
    let (store, _) = kept_chat(&app, None);
    std::fs::write(app.join("notes.md"), "kept\n").unwrap();
    let (_, workspace, cx) =
        open_window_resuming(cx, root.join("home"), store.clone(), Resumes::Resume);
    let rail = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, _| {
            workspace
                .projects()
                .iter()
                .filter(|project| project.is_archived())
                .count()
        })
    };

    // Archived, it leaves the rail for the menu of those put away, its
    // chats kept, and comes back from there.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.archive_project(1, true, window, cx);
    });
    settled(cx, &workspace);
    assert!(store.projects().unwrap()[0].archived);
    assert_eq!(rail(cx), 1);
    assert!(drawn(cx, "archived-projects"));
    assert_eq!(store.chats().unwrap().len(), 1);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.archive_project(1, false, window, cx);
    });
    settled(cx, &workspace);
    assert!(!store.projects().unwrap()[0].archived);
    assert_eq!(rail(cx), 0);
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.chosen()),
        1
    );

    // Deleted on a second click, its folder goes to the trash whole, and
    // its chats are forgotten.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.delete_project(1, window, cx);
    });
    settled(cx, &workspace);
    assert!(app.is_dir(), "one click does not delete it");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.delete_project(1, window, cx);
    });
    settled(cx, &workspace);
    assert!(!app.exists());
    assert_eq!(
        std::fs::read_to_string(trash.join("files").join("away-app").join("notes.md")).unwrap(),
        "kept\n"
    );
    assert_eq!(store.projects().unwrap(), []);
    assert_eq!(store.chats().unwrap(), []);
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.chosen(), 0);
        assert_eq!(workspace.chats(1).count(), 0);
    });
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
async fn an_agent_proposes_the_projects_name_which_the_human_renames_it_to(
    cx: &mut TestAppContext,
) {
    let projects = scratch("propose-name-projects");
    let app = projects.join("untitled-1");
    let (store, _) = kept_chat(&app, Some("session-1"));
    let (_, asks, workspace, cx) = open_window_asking(
        cx,
        scratch("propose-name-home"),
        store.clone(),
        Resumes::Resume,
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });
    say(cx, &workspace, "hello").await;
    let propose = |name: &str, why: Option<&str>| {
        let (answer, now) = tokio::sync::oneshot::channel();
        asks.borrow()[0]
            .send(ChatRequest::ProposeName {
                name: name.to_owned(),
                why: why.map(str::to_owned),
                answer,
            })
            .unwrap();
        now
    };

    // The agent hears the project's name now, and the name shows on a
    // card, for the human to answer.
    let now = propose("market", None);
    settled(cx, &workspace);
    assert_eq!(now.await.unwrap(), Ok("untitled-1".to_owned()));
    let now = propose("shop", Some("It sells things."));
    settled(cx, &workspace);
    assert_eq!(now.await.unwrap(), Ok("untitled-1".to_owned()));
    let proposed = |cx: &mut VisualTestContext| {
        shown_entries(cx, &workspace)
            .into_iter()
            .enumerate()
            .filter_map(|(index, entry)| match entry {
                Entry::NameProposal { name, answer, .. } => Some((index, name, answer)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let cards = proposed(cx);
    assert_eq!(cards.len(), 2, "{cards:?}");
    let (market, shop) = (cards[0].0, cards[1].0);

    // Dismiss lets one be.
    click(
        cx,
        Box::leak(format!("dismiss-name-{market}").into_boxed_str()),
    );
    settled(cx, &workspace);
    assert_eq!(proposed(cx)[0].2, Some(Answer::Dismissed));
    assert!(app.is_dir());

    // Rename renames the project, and its folder with it.
    click(
        cx,
        Box::leak(format!("rename-project-{shop}").into_boxed_str()),
    );
    settled(cx, &workspace);
    let renamed = projects.join("shop");
    assert_eq!(store.projects().unwrap()[0].folder, renamed);
    assert!(renamed.is_dir() && !app.exists());
    assert_eq!(proposed(cx)[1].2, Some(Answer::Set));
    let _ = std::fs::remove_dir_all(&projects);
}

#[gpui::test]
async fn a_chat_in_no_project_has_no_name_to_propose(cx: &mut TestAppContext) {
    let home = scratch("propose-name-own");
    let (_, asks, workspace, cx) = open_window_asking(
        cx,
        home,
        kept_store("propose-name-own-kept"),
        Resumes::Resume,
    );
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let (answer, now) = tokio::sync::oneshot::channel();
    asks.borrow()[0]
        .send(ChatRequest::ProposeName {
            name: "shop".to_owned(),
            why: None,
            answer,
        })
        .unwrap();
    settled(cx, &workspace);
    assert!(
        now.await
            .unwrap()
            .is_err_and(|why| why.contains("in no project"))
    );
    assert!(
        !shown_entries(cx, &workspace)
            .iter()
            .any(|entry| matches!(entry, Entry::NameProposal { .. }))
    );
}

#[gpui::test]
async fn a_chat_is_told_the_projects_other_chats_and_reads_them(cx: &mut TestAppContext) {
    let app = scratch("other-chats");
    let (store, kept) = kept_chat(&app, None);
    let (agents, asks, workspace, cx) = open_window_asking(
        cx,
        scratch("other-chats-home"),
        store.clone(),
        Resumes::Resume,
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(app.clone()), window, cx);
    });
    say(cx, &workspace, "hello").await;

    // The new chat's agent is told the chat kept, by its number, first.
    let told = agents.borrow()[0].prompt_blocks();
    let chats = told[0]
        .iter()
        .find(|block| block.starts_with("The other chats of this project"))
        .unwrap_or_else(|| panic!("no chats are told: {told:?}"));
    assert!(
        chats.contains(&format!("- chat {kept}: \"Chat 1\"")),
        "{chats}"
    );
    assert_eq!(told[0].last().map(String::as_str), Some("hello"));

    // It reads what that chat talked through, and no chat of another
    // project.
    let read = |chat: i64| {
        let (answer, read) = tokio::sync::oneshot::channel();
        asks.borrow()[0]
            .send(ChatRequest::ReadChat { chat, answer })
            .unwrap();
        read
    };
    let first = read(kept.number());
    settled(cx, &workspace);
    let first = first.await.unwrap().unwrap();
    assert!(
        first.starts_with(&format!("Chat {kept}, \"Chat 1\"")),
        "{first}"
    );
    assert!(
        first.ends_with("[human]\nhello\n\n[you]\necho: hello"),
        "{first}"
    );
    let none = read(kept.number() + 100);
    settled(cx, &workspace);
    assert!(none.await.unwrap().is_err());
    let _ = std::fs::remove_dir_all(&app);
}

#[gpui::test]
async fn an_agent_proposes_what_the_project_is_which_the_next_chats_are_told(
    cx: &mut TestAppContext,
) {
    let app = scratch("propose-about");
    let (store, _) = kept_chat(&app, Some("session-1"));
    let (agents, asks, workspace, cx) = open_window_asking(
        cx,
        scratch("propose-about-home"),
        store.clone(),
        Resumes::Resume,
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });
    say(cx, &workspace, "hello").await;
    let tea = "A shop that sells tea.";
    let propose = |about: &str| {
        let (answer, now) = tokio::sync::oneshot::channel();
        asks.borrow()[0]
            .send(ChatRequest::ProposeAbout {
                about: about.to_owned(),
                why: None,
                answer,
            })
            .unwrap();
        now
    };
    let proposed = |cx: &mut VisualTestContext| {
        shown_entries(cx, &workspace)
            .into_iter()
            .enumerate()
            .filter_map(|(index, entry)| match entry {
                Entry::AboutProposal { about, answer, .. } => Some((index, about, answer)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    // The agent hears that the project says nothing of what it is yet,
    // and what it proposes shows on a card, for the human to set.
    let now = propose(tea);
    settled(cx, &workspace);
    assert_eq!(now.await.unwrap(), Ok(None));
    let card = proposed(cx)[0].0;
    click(cx, Box::leak(format!("set-about-{card}").into_boxed_str()));
    settled(cx, &workspace);
    assert_eq!(proposed(cx)[0].2, Some(Answer::Set));
    assert_eq!(store.projects().unwrap()[0].about.as_deref(), Some(tea));
    // The next proposal hears what the project is now; one dismissed
    // leaves it be.
    let now = propose("A shop that sells coffee.");
    settled(cx, &workspace);
    assert_eq!(now.await.unwrap(), Ok(Some(tea.to_owned())));
    let card = proposed(cx)[1].0;
    click(
        cx,
        Box::leak(format!("dismiss-about-{card}").into_boxed_str()),
    );
    settled(cx, &workspace);
    assert_eq!(store.projects().unwrap()[0].about.as_deref(), Some(tea));

    // A chat opened in the project from now on is told it first.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(app.clone()), window, cx);
    });
    say(cx, &workspace, "again").await;
    let told = agents.borrow()[1].prompt_blocks();
    assert_eq!(told.len(), 1, "{told:?}");
    assert!(
        told[0][0].starts_with("This chat is in the project project. "),
        "{told:?}"
    );
    assert!(told[0][0].ends_with(tea), "{told:?}");
    assert_eq!(told[0].last().map(String::as_str), Some("again"));
    let _ = std::fs::remove_dir_all(&app);
}

/// Has the agent of the chat whose asks are at `from` propose the project
/// `name`, starting from the sketch named `sketch`, if given, and
/// answers where the project is to be, or why not, once it is told.
fn propose_project(
    asks: &Asks,
    from: usize,
    name: &str,
    sketch: Option<&str>,
) -> tokio::sync::oneshot::Receiver<Result<PathBuf, String>> {
    let (answer, folder) = tokio::sync::oneshot::channel();
    asks.borrow()[from]
        .send(ChatRequest::ProposeProject {
            name: name.to_owned(),
            why: Some("We agreed on it.".to_owned()),
            sketch: sketch.map(str::to_owned),
            answer,
        })
        .unwrap();
    folder
}

#[gpui::test]
async fn an_agent_proposes_a_project_which_the_chat_moves_into_with_its_sketch(
    cx: &mut TestAppContext,
) {
    let home = repository("propose-project-home");
    let store = kept_store("propose-project-kept");
    let (agents, asks, workspace, cx) =
        open_window_asking(cx, home.clone(), store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let made = open_sketch(cx, &workspace, &asks, Sketch::Empty, 2).await;
    std::fs::write(made.join("index.html"), "<p>shop</p>\n").unwrap();
    let (first, sketch) = workspace.read_with(cx, |workspace, _| {
        let mut chats = workspace.chats(0);
        let first = chats.next().unwrap().key();
        (first, chats.next().unwrap().name().to_owned())
    });

    // A chat that is no sketch of the chat's is no sketch to start from,
    // and the agent hears the names of those that are.
    let refused = propose_project(&asks, 0, "shop", Some("hello"));
    settled(cx, &workspace);
    let refused = refused.await.unwrap();
    assert!(refused.is_err_and(
        |why| why.contains("is named \"hello\"") && why.contains(&format!("\"{sketch}\""))
    ));
    // The agent hears where the project is to be, and it shows on a card.
    let projects = home.join("projects");
    let market = propose_project(&asks, 0, "market", None);
    settled(cx, &workspace);
    assert_eq!(market.await.unwrap(), Ok(projects.join("market")));
    // The sketch goes by the name the human's list shows.
    let shop = propose_project(&asks, 0, "shop", Some(&sketch));
    settled(cx, &workspace);
    assert_eq!(shop.await.unwrap(), Ok(projects.join("shop")));
    let shop = projects.join("shop");
    let cards: Vec<(usize, Option<PathBuf>, Option<String>)> = shown_entries(cx, &workspace)
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| match entry {
            Entry::ProjectProposal {
                sketch,
                sketch_name,
                ..
            } => Some((index, sketch, sketch_name)),
            _ => None,
        })
        .collect();
    assert_eq!(cards.len(), 2, "{cards:?}");
    assert_eq!(cards[1].1.as_ref(), Some(&made));
    assert_eq!(cards[1].2.as_ref(), Some(&sketch));
    let (market, at) = (cards[0].0, cards[1].0);

    // Dismiss lets one be; Make project makes the other, the sketch's
    // files its directory, and the chat and its sketch move into it.
    click(
        cx,
        Box::leak(format!("dismiss-project-{market}").into_boxed_str()),
    );
    settled(cx, &workspace);
    assert!(!projects.join("market").exists());
    click(cx, Box::leak(format!("make-project-{at}").into_boxed_str()));
    settled(cx, &workspace);
    assert_eq!(
        std::fs::read_to_string(shop.join("directories/shop/index.html")).unwrap(),
        "<p>shop</p>\n"
    );
    // Another test may make a sketch from nothing in a folder of the same
    // name once this one is gone, so what is gone is what this sketch made.
    assert!(!made.join("index.html").exists());
    let kept = store.projects().unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].folder, shop);
    let chats = store.chats().unwrap();
    assert_eq!(chats.len(), 2);
    assert!(chats.iter().all(|chat| chat.project == Some(kept[0].id)));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.chats(0).count(), 0);
        assert_eq!(workspace.chats(1).count(), 2);
        assert_eq!(workspace.shown().map(Chat::key), Some(first));
    });
    let entries = shown_entries(cx, &workspace);
    assert!(
        matches!(
            &entries[market],
            Entry::ProjectProposal {
                answer: Some(Answer::Dismissed),
                ..
            }
        ) && matches!(
            &entries[at],
            Entry::ProjectProposal {
                answer: Some(Answer::Set),
                ..
            }
        ),
        "{entries:?}"
    );
    assert!(
        entries.iter().any(|entry| matches!(entry, Entry::Note(note)
            if note.starts_with("The chat moved into the project shop"))),
        "{entries:?}"
    );

    // The next line starts its agent again in the project's folder, where
    // it has a project already.
    say(cx, &workspace, "go on").await;
    assert_eq!(agents.borrow().last().unwrap().cwd(), Some(shop.clone()));
    let last = asks.borrow().len() - 1;
    let again = propose_project(&asks, last, "shop-2", None);
    settled(cx, &workspace);
    let again = again.await.unwrap();
    assert!(again.is_err_and(|why| why.contains("in a project already")));
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

/// Makes a Git repository at `folder` with one commit, on `main`.
fn repository_with_a_commit(folder: &Path) {
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(folder)
            .status()
            .unwrap();
        assert!(status.success());
    };
    std::fs::create_dir_all(folder).unwrap();
    git(&["init", "-q", "--initial-branch=main"]);
    git(&[
        "-c",
        "user.name=Cox",
        "-c",
        "user.email=cox@example.com",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "first",
    ]);
}

#[gpui::test]
fn directories_are_made_cloned_and_linked_on_the_projects_card(cx: &mut TestAppContext) {
    let root = scratch("directories");
    let origin = root.join("origin.git");
    repository_with_a_commit(&origin);
    let app = root.join("projects").join("untitled-1");
    let directories = app.join("directories");
    let (store, _) = kept_chat(&app, None);
    let (_, workspace, cx) = open_window_resuming(cx, root.join("home"), store, Resumes::Resume);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_project_card(1, window, cx);
    });
    let card = workspace.read_with(cx, |workspace, _| {
        workspace.project_card().expect("the card is open").clone()
    });
    let described = |cx: &mut VisualTestContext| cx.read(|cx| card.read(cx).described(cx));
    let input = card.read_with(cx, |card, _| card.directory_input().clone());
    // The card has the project's icon too, which it has none of yet.
    let said = described(cx);
    assert!(
        said.contains(&"Icon: none, a folder".to_owned()),
        "{said:?}"
    );
    let enter = |text: &str, input: &Entity<LineInput>, cx: &mut VisualTestContext| {
        input.update(cx, |input, cx| input.set_text(text, cx));
        cx.update(|window, cx| window.focus(&input.focus_handle(cx)));
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
    };

    // A URL clones the repository into a directory named after it.
    let url = origin.display().to_string();
    enter(&url, &input, cx);
    assert!(directories.join("origin").join(".git").is_dir());
    let said = described(cx);
    assert!(
        said.contains(&format!("Directory origin (Git, remote {url})"))
            && said.contains(&"Under it: Cloned into origin.".to_owned()),
        "{said:?}"
    );
    // The same again is not cloned over it.
    enter(&url, &input, cx);
    let said = described(cx);
    assert!(
        said.contains(&"Under it: The project has a directory origin already.".to_owned()),
        "{said:?}"
    );

    // A name alone makes a directory, which is no repository yet.
    enter("app", &input, cx);
    assert!(directories.join("app").is_dir());
    assert!(!directories.join("app").join(".git").exists());
    let said = described(cx);
    assert!(
        said.contains(&"Directory app".to_owned())
            && said.contains(&"Under it: Made app.".to_owned()),
        "{said:?}"
    );

    // It becomes one, with no remote.
    click(cx, "project-directory-action-0");
    cx.run_until_parked();
    assert!(directories.join("app").join(".git").is_dir());
    let said = described(cx);
    assert!(
        said.contains(&"Directory app (Git, no remote)".to_owned()),
        "{said:?}"
    );

    // And, linked to a remote while it has nothing yet, takes its branch.
    click(cx, "project-directory-action-0");
    let remote = card
        .read_with(cx, |card, _| {
            card.row_input().map(|(_, input)| input.clone())
        })
        .expect("the remote's input is open");
    enter(&url, &remote, cx);
    let said = described(cx);
    assert!(
        said.contains(&format!("Directory app (Git, remote {url})"))
            && said.contains(&"Under it: Linked app to its remote, and took its main.".to_owned()),
        "{said:?}"
    );
    assert!(card.read_with(cx, |card, _| card.row_input().is_none()));

    // Nothing typed makes a directory named as the project.
    enter("", &input, cx);
    assert!(directories.join("untitled-1").is_dir());

    // A directory is renamed on its row, its folder with it.
    click(cx, "project-directory-rename-0");
    let name = card
        .read_with(cx, |card, _| {
            card.row_input().map(|(_, input)| input.clone())
        })
        .expect("the name's input is open");
    enter("web", &name, cx);
    assert!(directories.join("web").join(".git").is_dir());
    assert!(!directories.join("app").exists());
    let said = described(cx);
    assert!(
        said.contains(&format!("Directory web (Git, remote {url})"))
            && said.contains(&"Under it: Renamed app to web.".to_owned()),
        "{said:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
async fn what_an_agent_tells_of_a_session_it_loads_is_not_shown_again(cx: &mut TestAppContext) {
    let folder = scratch("load");
    let (store, _) = kept_chat(&folder, Some("session-1"));
    let (agents, workspace, cx) =
        open_window_resuming(cx, scratch("load-home"), store, Resumes::Load);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });

    say(cx, &workspace, "again").await;

    assert_eq!(agents.borrow()[0].resumed(), ["session-1"]);
    let shown = shown_entries(cx, &workspace);
    assert_eq!(
        shown[3..5],
        [
            Entry::Human("again".to_owned()),
            Entry::Note("Resumed: the agent goes on from here.".to_owned()),
        ]
    );
    assert_eq!(shown[5], Entry::Reply("echo: again".to_owned()));
}

/// Asserts that the agent of the chat shown was told the conversation so
/// far, after the instructions of `folder`, ahead of `again`, and that the
/// chat says so if the session it had was `lost`.
fn assert_retold(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    agent: &FakeAgent,
    folder: &std::path::Path,
    lost: bool,
) {
    let told = Instructions::project(folder).unwrap().prompts();
    let blocks = agent.prompt_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].len(), told.len() + 2, "{blocks:?}");
    assert_eq!(blocks[0][..told.len()], told);
    let [earlier, line] = &blocks[0][told.len()..] else {
        unreachable!()
    };
    assert!(
        earlier.ends_with("[human]\nhello\n\n[you]\necho: hello"),
        "{blocks:?}"
    );
    assert_eq!(line, "again");
    assert_eq!(earlier.contains("could not be resumed"), lost, "{blocks:?}");
    let shown = shown_entries(cx, workspace);
    assert_eq!(shown[3], Entry::Human("again".to_owned()));
    let notes = if lost {
        assert!(
            matches!(&shown[4], Entry::Note(note) if note.contains("could not resume")),
            "{shown:?}"
        );
        5
    } else {
        4
    };
    assert!(
        matches!(&shown[notes], Entry::Note(note) if note.starts_with("Instructions: ")),
        "{shown:?}"
    );
    assert!(
        !shown
            .iter()
            .skip(notes)
            .any(|entry| matches!(entry, Entry::Note(note) if note.contains("could not resume"))),
        "{shown:?}"
    );
}

#[gpui::test]
async fn an_agent_that_cannot_resume_is_told_the_conversation_so_far(cx: &mut TestAppContext) {
    let folder = scratch("retell");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("COX.md"), "Answer in Japanese.\n").unwrap();
    let (store, kept) = kept_chat(&folder, Some("session-0"));
    let (agents, workspace, cx) =
        open_window_resuming(cx, scratch("retell-home"), store.clone(), Resumes::Never);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });

    say(cx, &workspace, "again").await;

    let agent = Arc::clone(&agents.borrow()[0]);
    assert_eq!(agent.resumed(), Vec::<String>::new());
    assert_eq!(agent.sessions_opened(), 1);
    assert_retold(cx, &workspace, &agent, &folder, true);
    // The new session is the one to go on with from now on.
    assert_eq!(store.session(kept).unwrap().as_deref(), Some("session-1"));
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn an_agent_that_refuses_to_resume_is_told_the_conversation_so_far(cx: &mut TestAppContext) {
    let folder = scratch("refuse");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("COX.md"), "Answer in Japanese.\n").unwrap();
    let (store, _) = kept_chat(&folder, Some("session-0"));
    let (agents, workspace, cx) =
        open_window_resuming(cx, scratch("refuse-home"), store, Resumes::Refuse);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });

    say(cx, &workspace, "again").await;

    let agent = Arc::clone(&agents.borrow()[0]);
    assert_eq!(agent.resumed(), ["session-0"]);
    assert_eq!(agent.sessions_opened(), 1);
    assert_retold(cx, &workspace, &agent, &folder, true);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn a_chat_kept_with_no_session_is_told_the_conversation_so_far(cx: &mut TestAppContext) {
    let folder = scratch("no-session");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("COX.md"), "Answer in Japanese.\n").unwrap();
    let (store, _) = kept_chat(&folder, None);
    let (agents, workspace, cx) =
        open_window_resuming(cx, scratch("no-session-home"), store, Resumes::Resume);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });

    say(cx, &workspace, "again").await;

    let agent = Arc::clone(&agents.borrow()[0]);
    assert_eq!(agent.resumed(), Vec::<String>::new());
    assert_retold(cx, &workspace, &agent, &folder, false);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn a_new_chat_keeps_its_agents_session(cx: &mut TestAppContext) {
    let store = kept_store("chats-3");
    let (_, workspace, cx) = open_window_keeping(cx, scratch("session"), store.clone());
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;

    let chats = store.chats().unwrap();
    assert_eq!(
        store.session(chats[0].id).unwrap().as_deref(),
        Some("session-1")
    );
}

/// The fake agent's settings, set as `model` and `mode` say.
fn fake_settings(model: &str, mode: &str) -> Vec<Setting> {
    let choices = |ids: [(&str, &str); 2]| {
        ids.map(|(id, name)| Choice {
            id: id.to_owned(),
            name: name.to_owned(),
            description: None,
            group: None,
        })
        .to_vec()
    };
    vec![
        Setting {
            id: fake_agent::MODEL.to_owned(),
            name: "Model".to_owned(),
            description: None,
            category: Some("model".to_owned()),
            value: model.to_owned(),
            choices: choices([("large", "Large"), ("small", "Small")]),
        },
        Setting {
            id: fake_agent::MODE.to_owned(),
            name: "Mode".to_owned(),
            description: None,
            category: Some("mode".to_owned()),
            value: mode.to_owned(),
            choices: choices([("ask", "Ask"), ("plan", "Plan")]),
        },
    ]
}

#[gpui::test]
async fn a_closed_chat_sets_its_session_as_it_was_set(cx: &mut TestAppContext) {
    let folder = scratch("settings");
    let (store, kept) = kept_chat(&folder, Some("session-1"));
    store
        .keep_settings(kept, &fake_settings("small", "plan"))
        .unwrap();
    let (agents, workspace, cx) =
        open_window_resuming(cx, scratch("settings-home"), store.clone(), Resumes::Never);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });

    // Closed, the chat is set as it was, and the human sets it anew, for
    // its agent to set once it starts again.
    click(cx, "settings");
    click(cx, "setting-0");
    click(cx, "choice-0");
    assert_eq!(
        store.settings(kept).unwrap(),
        fake_settings("large", "plan")
    );
    assert_eq!(
        store.choices(FIRST_AGENT).unwrap(),
        [(fake_agent::MODEL.to_owned(), "large".to_owned())]
    );

    say(cx, &workspace, "again").await;

    let agent = Arc::clone(&agents.borrow()[0]);
    // A new session is set as the chat was: its model is large already.
    assert_eq!(
        agent.settings_set(),
        [(fake_agent::MODE.to_owned(), "plan".to_owned())]
    );
    let view = shown(cx, &workspace);
    let set = view.read_with(cx, |view, _| view.conversation().settings().to_vec());
    assert_eq!(set, fake_settings("large", "plan"));
    assert_eq!(store.settings(kept).unwrap(), set);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn a_new_chat_starts_on_the_model_chosen_last(cx: &mut TestAppContext) {
    let store = kept_store("chats-4");
    store
        .keep_choice(FIRST_AGENT, fake_agent::MODEL, "small")
        .unwrap();
    store.keep_choice(FIRST_AGENT, "gone", "anything").unwrap();
    store
        .keep_choice(SECOND_AGENT, fake_agent::MODE, "plan")
        .unwrap();
    let (agents, workspace, cx) = open_window_keeping(cx, scratch("chosen"), store.clone());
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;

    let agent = Arc::clone(&agents.borrow()[0]);
    // What the session has no setting for is left out.
    assert_eq!(
        agent.settings_set(),
        [(fake_agent::MODEL.to_owned(), "small".to_owned())]
    );
    let chats = store.chats().unwrap();
    assert_eq!(
        store.settings(chats[0].id).unwrap(),
        fake_settings("small", "ask")
    );
}

#[gpui::test]
async fn a_chat_switches_to_another_agent_which_goes_on_from_the_conversation(
    cx: &mut TestAppContext,
) {
    let store = kept_store("chats-5");
    store
        .keep_choice(SECOND_AGENT, fake_agent::MODE, "plan")
        .unwrap();
    let (agents, workspace, cx) = open_window_keeping(cx, scratch("switch"), store.clone());
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let kept = store.chats().unwrap()[0].id;
    store.keep_session(kept, "session-1").unwrap();

    click(cx, "settings");
    click(cx, "agent");
    click(cx, "choice-1");
    let view = shown(cx, &workspace);
    cx.condition(&view, |view, _| !view.conversation().settings().is_empty())
        .await;

    // The chat talks to the other agent, in a session of its own, set as
    // the human chose last for that agent.
    assert_eq!(store.agent(kept).unwrap().as_deref(), Some(SECOND_AGENT));
    assert_eq!(
        view.read_with(cx, |view, _| view.agent_id().to_owned()),
        SECOND_AGENT
    );
    assert_eq!(agents.borrow().len(), 2);
    let second = Arc::clone(&agents.borrow()[1]);
    assert_eq!(second.resumed(), Vec::<String>::new());
    assert_eq!(
        second.settings_set(),
        [(fake_agent::MODE.to_owned(), "plan".to_owned())]
    );
    let switched = Entry::Switched {
        from: FIRST_AGENT.to_owned(),
        to: SECOND_AGENT.to_owned(),
    };
    assert!(shown_entries(cx, &workspace).contains(&switched));

    // It goes on from the conversation, told who said what.
    say(cx, &workspace, "go on").await;
    let prompt = second.prompts().concat();
    assert!(prompt.contains("[human]\nhello"), "{prompt}");
    assert!(
        prompt.contains(&format!(
            "[the human switched the chat from the agent {FIRST_AGENT} to the agent {SECOND_AGENT}]"
        )),
        "{prompt}"
    );
    assert!(prompt.ends_with("go on"), "{prompt}");
    assert_eq!(agents.borrow()[0].prompts(), ["hello"]);
}

#[gpui::test]
async fn a_chat_switched_before_anything_was_said_starts_anew(cx: &mut TestAppContext) {
    let store = kept_store("chats-6");
    let (agents, workspace, cx) = open_window_keeping(cx, scratch("switch-new"), store.clone());
    cx.simulate_keystrokes("secondary-n");
    let view = shown(cx, &workspace);
    cx.condition(&view, |view, _| !view.conversation().settings().is_empty())
        .await;

    click(cx, "settings");
    click(cx, "agent");
    click(cx, "choice-1");
    say(cx, &workspace, "hello").await;

    assert_eq!(agents.borrow().len(), 2);
    assert_eq!(agents.borrow()[1].prompts(), ["hello"]);
    // A chat its agent opens talks to the same agent.
    let chats = store.chats().unwrap();
    let child = store
        .add_chat_under(chats[0].id, None, "Chat 2", None)
        .unwrap();
    assert_eq!(store.agent(child).unwrap().as_deref(), Some(SECOND_AGENT));
}

/// The name of each chat kept, with where it came from and whether it is
/// archived.
fn kept_names(store: &Store) -> Vec<(String, Named, bool)> {
    store
        .chats()
        .unwrap()
        .into_iter()
        .map(|chat| (chat.name, chat.named, chat.archived))
        .collect()
}

/// Clicks what `selector` names twice, quickly.
fn double_click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = at(cx, selector);
    for click_count in 1..=2 {
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

/// Opens the menu of the right button on what `selector` names.
fn right_click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = at(cx, selector);
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
}

#[gpui::test]
async fn a_chat_is_named_by_its_agent_until_the_human_names_it(cx: &mut TestAppContext) {
    let store = kept_store("chats-7");
    let (_, workspace, cx) = open_window_keeping(cx, scratch("named"), store.clone());
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "a title, please").await;
    assert_eq!(names(cx, &workspace), listed(&[("Cox", &[TITLE])]));
    assert_eq!(
        kept_names(&store),
        [(TITLE.to_owned(), Named::Agent, false)]
    );

    // A double click on the chat shown, at the head of the panel,
    // renames it.
    double_click(cx, "chat-switcher");
    cx.simulate_input("Mine");
    cx.simulate_keystrokes("enter");
    say(cx, &workspace, "another title").await;
    assert_eq!(names(cx, &workspace), listed(&[("Cox", &["Mine"])]));
    assert_eq!(
        kept_names(&store),
        [("Mine".to_owned(), Named::Human, false)]
    );

    // Escape leaves the name as it was.
    right_click(cx, "chat-1");
    click(cx, "rename-chat");
    cx.simulate_input("Not mine");
    cx.simulate_keystrokes("escape");
    assert_eq!(names(cx, &workspace), listed(&[("Cox", &["Mine"])]));
}

#[gpui::test]
async fn a_turn_that_ends_unseen_marks_its_chat_until_it_is_shown(cx: &mut TestAppContext) {
    let (_, workspace, cx) = open_window(cx, scratch("unread"));
    cx.simulate_keystrokes("secondary-n");
    let first = shown(cx, &workspace);
    cx.simulate_input("slowly");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("secondary-n");
    wait_for_turn(cx, &first).await;
    let unread = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, _| {
            workspace.chats(0).map(Chat::is_unread).collect::<Vec<_>>()
        })
    };
    assert_eq!(unread(cx), [true, false]);

    // A turn the human sees end leaves its chat read.
    say(cx, &workspace, "hello").await;
    assert_eq!(unread(cx), [true, false]);
    click(cx, "chat-1");
    assert_eq!(unread(cx), [false, false]);
}

#[gpui::test]
async fn an_archived_chat_leaves_the_list_until_it_is_brought_back(cx: &mut TestAppContext) {
    let home = scratch("archived");
    let store = kept_store("archived-kept");
    let (agents, workspace, cx) = open_window_keeping(cx, home.clone(), store.clone());
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "first").await;
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "second").await;

    right_click(cx, "chat-2");
    click(cx, "archive-chat");
    assert_eq!(names(cx, &workspace), listed(&[("Cox", &["first"])]));
    // The chat after it in the list, or before it, is shown instead.
    let shown_name = |cx: &mut VisualTestContext, workspace: &Entity<Workspace>| {
        workspace.read_with(cx, |workspace, _| {
            workspace.shown().map(|chat| chat.name().to_owned())
        })
    };
    assert_eq!(shown_name(cx, &workspace).as_deref(), Some("first"));
    assert_eq!(
        kept_names(&store),
        [
            ("first".to_owned(), Named::FirstLine, false),
            ("second".to_owned(), Named::FirstLine, true),
        ]
    );

    // It stays archived once Cox starts again.
    let again = start_again(cx, home.clone(), &store, &agents);
    assert_eq!(names(cx, &again), listed(&[("Cox", &["first"])]));
    let archived = again.read_with(cx, |workspace, _| {
        workspace
            .archived()
            .iter()
            .map(|chat| chat.name().to_owned())
            .collect::<Vec<_>>()
    });
    assert_eq!(archived, ["second"]);

    click(cx, "archived");
    click(cx, "archived-1");
    assert_eq!(
        names(cx, &workspace),
        listed(&[("Cox", &["first", "second"])])
    );
    assert_eq!(shown_name(cx, &workspace).as_deref(), Some("second"));
    assert!(!kept_names(&store)[1].2);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

/// What Cox told the human through the app's [`Notices`].
#[derive(Debug, PartialEq, Eq)]
enum Told {
    /// A chat's key, name, and what it told.
    Notice(u64, String, String),
    Badge(usize),
}

/// Keeps what Cox tells the human, for a test to read.
#[derive(Default)]
struct Recorded(RefCell<Vec<Told>>);

impl Notices for Recorded {
    fn notify(&self, chat: u64, title: &str, body: &str) {
        let told = Told::Notice(chat, title.to_owned(), body.to_owned());
        self.0.borrow_mut().push(told);
    }

    fn badge(&self, count: usize) {
        self.0.borrow_mut().push(Told::Badge(count));
    }
}

/// Waits until the conversation of the chat `view` is as `until` says.
async fn wait_until(
    cx: &mut VisualTestContext,
    view: &Entity<ChatView>,
    until: impl Fn(&Conversation) -> bool,
) {
    let (timed_out, timeout) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(GIVEN_UP);
        let _ = timed_out.send(());
    });
    tokio::select! {
        () = cx.condition(view, |view, _| until(view.conversation())) => {}
        _ = timeout => panic!("the agent never got there"),
    }
}

#[gpui::test]
async fn a_chat_tells_the_human_what_they_do_not_see(cx: &mut TestAppContext) {
    let (_, workspace, cx) = open_window(cx, scratch("notices"));
    let recorded = Rc::new(Recorded::default());
    let notices = Rc::clone(&recorded);
    cx.update(|window, cx| {
        cx.set_global(AppNotices(notices));
        window.activate_window();
    });
    cx.run_until_parked();
    let told = || recorded.0.borrow_mut().drain(..).collect::<Vec<_>>();
    let key = |cx: &mut VisualTestContext, index| {
        workspace.read_with(cx, |workspace, _| {
            workspace.chats(0).nth(index).unwrap().key()
        })
    };

    // The first chat comes to wait for the human while the second is
    // shown.
    cx.simulate_keystrokes("secondary-n");
    let first = shown(cx, &workspace);
    cx.simulate_input("slow permission");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("secondary-n");
    wait_until(cx, &first, |conversation| {
        conversation.state() == State::Asking
    })
    .await;
    let first_key = key(cx, 0);
    assert_eq!(
        told(),
        [
            Told::Notice(
                first_key,
                "slow permission".to_owned(),
                "Asks you: Edit x.txt".to_owned()
            ),
            Told::Badge(1),
        ]
    );

    // A click on the notice shows the chat, whose turn then ends in sight.
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| workspace.reveal(first_key, window, cx));
    });
    assert_eq!(shown(cx, &workspace), first);
    cx.simulate_keystrokes("1");
    wait_for_turn(cx, &first).await;
    assert_eq!(told(), [Told::Badge(0)]);

    // A turn that ends in the chat shown, with Cox behind other windows.
    cx.simulate_input("slowly");
    cx.simulate_keystrokes("enter");
    cx.deactivate_window();
    wait_for_turn(cx, &first).await;
    assert_eq!(
        told(),
        [Told::Notice(
            first_key,
            "slow permission".to_owned(),
            "echo: slowly".to_owned()
        )]
    );

    // An archived chat, revealed, is back in the list.
    right_click(cx, "chat-1");
    click(cx, "archive-chat");
    settle_on(cx, &workspace).await;
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| workspace.reveal(first_key, window, cx));
    });
    settle_on(cx, &workspace).await;
    assert_eq!(shown(cx, &workspace), first);
}

#[gpui::test]
async fn the_triggers_of_a_permission_request_gather_before_the_human_is_told(
    cx: &mut TestAppContext,
) {
    let home = scratch("gather");
    let triggers = home.join(".cox/triggers");
    std::fs::create_dir_all(&triggers).unwrap();
    std::fs::write(
        triggers.join("touches.md"),
        "---\non: permission.asked\nby: shell\n---\nprintf 'Only x.txt changes\\nNothing else.\\n'\n",
    )
    .unwrap();
    let (_, workspace, cx) = open_window(cx, home.clone());
    let recorded = Rc::new(Recorded::default());
    let notices = Rc::clone(&recorded);
    cx.update(|window, cx| {
        cx.set_global(AppNotices(notices));
        window.activate_window();
    });
    cx.run_until_parked();
    let told = || recorded.0.borrow_mut().drain(..).collect::<Vec<_>>();
    cx.simulate_keystrokes("secondary-n");
    let first = shown(cx, &workspace);
    // Asked while another chat is shown, which the human sees.
    cx.simulate_input("slow permission");
    cx.simulate_keystrokes("enter");
    cx.simulate_keystrokes("secondary-n");

    wait_until(cx, &first, |conversation| {
        conversation.state() == State::Asking && !conversation.gathering()
    })
    .await;
    settle_on(cx, &workspace).await;

    // The human hears of the request once the trigger gathered what they
    // need to answer it, its headline with it.
    let key = workspace.read_with(cx, |workspace, _| workspace.chats(0).next().unwrap().key());
    let notices: Vec<Told> = told()
        .into_iter()
        .filter(|told| matches!(told, Told::Notice(..)))
        .collect();
    assert_eq!(
        notices,
        [Told::Notice(
            key,
            "slow permission".to_owned(),
            "Asks you: Edit x.txt\nOnly x.txt changes".to_owned()
        )]
    );
    let entries = first.read_with(cx, |view, _| view.conversation().entries().to_vec());
    let Some(Entry::Permission { gathered, .. }) = entries.last() else {
        panic!("{entries:?}");
    };
    assert_eq!(gathered.len(), 1, "{gathered:?}");
    assert_eq!(gathered[0].name, "touches");
    assert_eq!(gathered[0].state, CheckState::Passed);
    assert_eq!(
        gathered[0].said.as_deref(),
        Some("Only x.txt changes\nNothing else.")
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// A command that fails the first time it runs, leaving a mark in
/// `folder`, and passes from then on. It mentions "tool", so that the
/// fake agent, told what it printed, calls a tool.
fn passing_the_second_time(folder: &std::path::Path) -> String {
    let mark = folder.join("passed").display().to_string();
    format!("test -e {mark} || {{ touch {mark}; echo 'no tool ran yet'; exit 1; }}")
}

/// The goal of the chats of the project at `project`, in the order of the
/// list.
fn goals(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    project: usize,
) -> Vec<Option<KeptGoal>> {
    settled(cx, workspace);
    workspace.read_with(cx, |workspace, _| {
        workspace
            .chats(project)
            .map(|chat| chat.goal().cloned())
            .collect()
    })
}

#[gpui::test]
async fn a_chat_with_a_goal_goes_on_until_its_command_passes(cx: &mut TestAppContext) {
    let home = scratch("goal");
    std::fs::create_dir_all(&home).unwrap();
    let store = kept_store("chats-8");
    let (agents, workspace, cx) = open_window_keeping(cx, home.clone(), store.clone());
    cx.simulate_keystrokes("secondary-n");
    let view = shown(cx, &workspace);
    let command = passing_the_second_time(&home);
    view.read_with(cx, |view, _| {
        view.set_goal(Some(KeptGoal::Command(command.clone())));
    });
    assert_eq!(
        goals(cx, &workspace, 0),
        [Some(KeptGoal::Command(command.clone()))]
    );

    say(cx, &workspace, "tool").await;

    let prompts = agents.borrow()[0].prompts();
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(
        prompts[1].starts_with(&format!(
            "Goal not met: `{command}` exit code 1. Go on until it passes. \
             It printed, at the end:\n\nno tool ran yet"
        )),
        "{}",
        prompts[1]
    );
    let entries = shown_entries(cx, &workspace);
    assert_eq!(
        entries[0],
        Entry::Note(format!("Goal: until `{command}` passes."))
    );
    assert_eq!(
        entries[entries.len() - 2],
        Entry::Note(format!("Goal met: `{command}` passed."))
    );
    // The goal shows as a chip of what ran as the turn ended.
    assert!(
        matches!(
            &entries[entries.len() - 3],
            Entry::Checks(checks) if checks.len() == 1 && checks[0].name == "goal"
        ),
        "{:?}",
        entries[entries.len() - 3]
    );
    assert_eq!(entries[entries.len() - 5], Entry::Cox(prompts[1].clone()));
    // The goal, and the way to it, are kept.
    let kept = store.chats().unwrap();
    assert_eq!(kept[0].goal, Some(KeptGoal::Command(command.clone())));
    assert_eq!(store.entries(kept[0].id).unwrap(), entries);

    // Met, the goal is a milestone at which the human is asked whether
    // to keep what the chat learned in the wiki; "Save to wiki" asks the
    // agent, in the human's place, as the menu's does.
    let asked = entries.len() - 1;
    assert_eq!(
        entries[asked],
        Entry::Keep {
            why: "The goal is met.".to_owned(),
            chat: None,
            kept: None,
        }
    );
    cx.run_until_parked();
    click(
        cx,
        Box::leak(format!("keep-in-wiki-{asked}").into_boxed_str()),
    );
    wait_for_turn(cx, &view).await;
    let save = workflow::WIKI_SAVE.say(&[]);
    assert_eq!(agents.borrow()[0].prompts().last(), Some(&save));
    let entries = shown_entries(cx, &workspace);
    assert_eq!(
        entries[asked],
        Entry::Keep {
            why: "The goal is met.".to_owned(),
            chat: None,
            kept: Some(true),
        }
    );
    assert!(entries.contains(&Entry::Cox(save)));

    view.read_with(cx, |view, _| view.set_goal(None));
    assert_eq!(goals(cx, &workspace, 0), [None]);
    assert_eq!(store.chats().unwrap()[0].goal, None);
}

#[gpui::test]
async fn a_trigger_of_the_folder_runs_as_a_turn_ends_until_it_passes(cx: &mut TestAppContext) {
    let home = scratch("trigger");
    let triggers = home.join(".cox/triggers");
    std::fs::create_dir_all(&triggers).unwrap();
    let command = passing_the_second_time(&home);
    std::fs::write(
        triggers.join("check.md"),
        format!("---\non: turn.end\nby: shell\n---\n{command}\n"),
    )
    .unwrap();
    let (agents, workspace, cx) = open_window(cx, home.clone());
    cx.simulate_keystrokes("secondary-n");

    say(cx, &workspace, "tool").await;

    let prompts = agents.borrow()[0].prompts();
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(
        prompts[1].starts_with(&format!(
            "The trigger `check` did not pass: `{command}` exit code 1. Fix what it found; the \
             triggers run again as your turn ends. It printed, at the end:\n\nno tool ran yet"
        )),
        "{}",
        prompts[1]
    );
    // Each turn's end shows the trigger as a chip, which failed, and then
    // passed, saying what its command printed; nothing asks to keep
    // anything, since the chat has no goal.
    let entries = shown_entries(cx, &workspace);
    let checks: Vec<&Vec<Check>> = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Checks(checks) => Some(checks),
            _ => None,
        })
        .collect();
    assert_eq!(checks.len(), 2, "{entries:?}");
    assert_eq!(checks[0][0].name, "check");
    assert_eq!(checks[0][0].state, CheckState::Failed);
    assert_eq!(checks[0][0].said.as_deref(), Some("no tool ran yet"));
    assert_eq!(checks[1][0].state, CheckState::Passed);
    assert!(
        matches!(entries.last(), Some(Entry::Checks(_))),
        "{entries:?}"
    );
}

#[gpui::test]
async fn a_command_the_window_runs_answers_once_it_ends(cx: &mut TestAppContext) {
    let home = scratch("window-runs");
    std::fs::create_dir_all(&home).unwrap();
    // The chat's commands go through the daemon to the window, which runs
    // them, as on a terminal of the stage, and answers how each ended.
    let fakes = Fakes {
        terminal: false,
        staged_terminal: true,
        ..Fakes::default()
    };
    let (workspace, cx) = open_with(cx, home.clone(), Kept::Memory, fakes);
    cx.simulate_keystrokes("secondary-n");
    let view = shown(cx, &workspace);
    let command = "echo ran".to_owned();
    view.read_with(cx, |view, _| {
        view.set_goal(Some(KeptGoal::Command(command.clone())));
    });

    say(cx, &workspace, "hello").await;

    let entries = shown_entries(cx, &workspace);
    assert!(
        entries.contains(&Entry::Note(format!("Goal met: `{command}` passed."))),
        "{entries:?}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
async fn a_goal_in_words_is_judged_by_an_agent_until_it_is_met(cx: &mut TestAppContext) {
    let home = scratch("goal-words");
    std::fs::create_dir_all(&home).unwrap();
    let store = kept_store("chats-9");
    let (agents, workspace, cx) = open_window_keeping(cx, home.clone(), store.clone());
    cx.simulate_keystrokes("secondary-n");
    right_click(cx, "chat-1");
    click(cx, "set-goal");
    cx.simulate_input("The greeting names the user.");
    cx.simulate_keystrokes("enter");
    let goal = KeptGoal::Words("The greeting names the user.".to_owned());
    assert_eq!(goals(cx, &workspace, 0), [Some(goal.clone())]);
    assert_eq!(store.chats().unwrap()[0].goal, Some(goal));

    say(cx, &workspace, "tool").await;

    // The judge is an agent of its own, with no chat, working in the
    // project's folder with no tools of Cox's, asked once a turn.
    let agents = agents.borrow();
    assert_eq!(agents.len(), 2);
    let judge = &agents[1];
    assert_eq!(judge.cwd(), Some(home));
    assert_eq!(judge.mcp_servers(), []);
    let asked = judge.prompt_blocks();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(
        asked[0][0].contains("\n\nYou judge whether"),
        "{}",
        asked[0][0]
    );
    assert!(
        asked[0][0].ends_with("The goal:\n\nThe greeting names the user.\n"),
        "{}",
        asked[0][0]
    );
    let question = asked[0].last().unwrap();
    assert!(
        question.starts_with("Is the goal met now?\n\nThe greeting names the user.\n"),
        "{question}"
    );
    assert!(question.contains("last said:\n\necho: tool"), "{question}");
    assert_eq!(asked[1].len(), 1, "{:?}", asked[1]);

    // The working agent hears what the judge said, until it passes.
    let prompts = agents[0].prompts();
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert_eq!(
        prompts[1],
        "Goal not met, as judged: the greeting still says hello Go on until it is met. \
         The judge said:\n\nI read src/main.rs.\nFAIL: the greeting still says hello"
    );
    let entries = shown_entries(cx, &workspace);
    assert_eq!(
        entries[0],
        Entry::Note("Goal: The greeting names the user.".to_owned())
    );
    assert!(entries.contains(&Entry::Cox(prompts[1].clone())));
    assert_eq!(
        entries.iter().rev().nth(1),
        Some(&Entry::Note("Goal met, as judged.".to_owned()))
    );
    assert_eq!(
        store.entries(store.chats().unwrap()[0].id).unwrap(),
        entries
    );
}

#[gpui::test]
async fn a_chats_goal_lasts_past_cox_starting_again(cx: &mut TestAppContext) {
    let folder = scratch("goal-again");
    std::fs::create_dir_all(&folder).unwrap();
    let (store, kept) = kept_chat(&folder, Some("session-1"));
    store
        .keep_goal(kept, Some(&KeptGoal::Command("true".to_owned())))
        .unwrap();
    let (agents, workspace, cx) =
        open_window_keeping(cx, scratch("goal-again-home"), store.clone());
    assert_eq!(
        goals(cx, &workspace, 1),
        [Some(KeptGoal::Command("true".to_owned()))]
    );

    click(cx, "project-2");
    click(cx, "chat-1");
    let view = shown(cx, &workspace);
    assert_eq!(
        view.read_with(cx, |view, _| {
            let conversation = view.conversation();
            (
                conversation.state(),
                conversation
                    .goal()
                    .and_then(Goal::command)
                    .map(str::to_owned),
            )
        }),
        (State::Closed, Some("true".to_owned()))
    );

    // Sent to, the chat goes on toward its goal.
    say(cx, &workspace, "hello").await;
    assert_eq!(agents.borrow()[0].prompts().len(), 1);
    assert_eq!(
        shown_entries(cx, &workspace).iter().rev().nth(1),
        Some(&Entry::Note("Goal met: `true` passed.".to_owned()))
    );
}

#[gpui::test]
async fn escape_stops_the_check_of_the_goal(cx: &mut TestAppContext) {
    let (_, workspace, cx) = open_window(cx, scratch("goal-stop"));
    cx.simulate_keystrokes("secondary-n");
    let view = shown(cx, &workspace);
    view.read_with(cx, |view, _| {
        view.set_goal(Some(KeptGoal::Command("sleep 3".to_owned())));
    });
    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    wait_until(cx, &view, |conversation| {
        conversation.state() == State::Checking
    })
    .await;

    cx.simulate_keystrokes("escape");
    settled(cx, &workspace);

    assert_eq!(
        view.read_with(cx, |view, _| view.conversation().state()),
        State::Idle
    );
    assert_eq!(
        shown_entries(cx, &workspace).last(),
        Some(&Entry::Note("Stopped.".to_owned()))
    );
}

#[gpui::test]
async fn the_human_sets_a_chats_goal_from_its_menu(cx: &mut TestAppContext) {
    let store = kept_store("chats-10");
    let (agents, workspace, cx) = open_window_keeping(cx, scratch("goal-menu"), store.clone());
    cx.simulate_keystrokes("secondary-n");
    assert!(!drawn(cx, "chat-1-goal"));

    right_click(cx, "chat-1");
    assert!(cx.debug_bounds("clear-goal").is_none());
    click(cx, "set-goal");
    cx.simulate_input("$ true");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        goals(cx, &workspace, 0),
        [Some(KeptGoal::Command("true".to_owned()))]
    );
    assert_eq!(
        store.chats().unwrap()[0].goal,
        Some(KeptGoal::Command("true".to_owned()))
    );
    cx.run_until_parked();
    assert!(drawn(cx, "chat-1-goal"));
    // The input that sets the goal is closed, and the keyboard is back in
    // the chat's own.
    say(cx, &workspace, "hello").await;
    assert_eq!(agents.borrow()[0].prompts(), ["hello"]);
    assert_eq!(
        shown_entries(cx, &workspace).iter().rev().nth(1),
        Some(&Entry::Note("Goal met: `true` passed.".to_owned()))
    );

    // Escape leaves the goal as it was.
    right_click(cx, "chat-1");
    click(cx, "set-goal");
    cx.simulate_input("$ false");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        goals(cx, &workspace, 0),
        [Some(KeptGoal::Command("true".to_owned()))]
    );

    right_click(cx, "chat-1");
    click(cx, "clear-goal");
    assert_eq!(goals(cx, &workspace, 0), [None]);
    assert_eq!(store.chats().unwrap()[0].goal, None);
}

/// The key of the chat at `index` of Cox's own, and that of the chat
/// whose agent opened it, if an agent did.
fn keys(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    index: usize,
) -> (u64, Option<u64>) {
    settled(cx, workspace);
    workspace.read_with(cx, |workspace, _| {
        let chat = workspace.chats(0).nth(index).unwrap();
        (chat.key(), chat.parent())
    })
}

#[gpui::test]
async fn an_agent_opens_a_chat_under_its_own_set_up_as_it_says(cx: &mut TestAppContext) {
    let home = scratch("child-home");
    let store = kept_store("child-kept");
    let (agents, asks, workspace, cx) =
        open_window_asking(cx, home.clone(), store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "second").await;
    let (parent, _) = keys(cx, &workspace, 0);

    // The first chat's agent opens a chat, which keeps to a context and
    // may edit nothing, on a line that asks to edit a file.
    let setup = Setup {
        context: Some("Change nothing.".to_owned()),
        policy: cox::agent::setup::Policy {
            allow: Vec::new(),
            reject: vec!["Edit *".to_owned()],
            within: None,
        },
        ..Setup::default()
    };
    let (answer, opened) = tokio::sync::oneshot::channel();
    asks.borrow()[0]
        .send(ChatRequest::OpenChat {
            text: "permission".to_owned(),
            setup: setup.clone(),
            goal: None,
            sketch: None,
            answer,
        })
        .unwrap();
    cx.condition(&workspace, |workspace, _| workspace.chats(0).count() == 3)
        .await;

    // It goes under the chat that opened it, which stays shown, and
    // works on the line, sent in the human's place, which names it as
    // the first line of any chat does.
    assert_eq!(
        names(cx, &workspace),
        listed(&[("Cox", &["hello", "permission", "second"])])
    );
    let (child, opened_by) = keys(cx, &workspace, 1);
    assert_eq!(opened_by, Some(parent));
    let opened = opened.await.unwrap().unwrap();
    assert_eq!((opened.number, opened.name.as_str()), (child, "Chat 3"));
    assert_eq!(
        shown_entries(cx, &workspace)[0],
        Entry::Human("second".to_owned())
    );
    let view = view_at(cx, &workspace, 1);
    wait_for_turn(cx, &view).await;
    let entries = entries(cx, &view);
    assert_eq!(
        entries[..3],
        [
            Entry::Note("Context: Change nothing.".to_owned()),
            Entry::Note("Opened by: hello".to_owned()),
            Entry::Cox("permission".to_owned()),
        ]
    );
    // Cox answered the request to edit, as the chat is set up.
    assert!(
        matches!(
            &entries[4],
            Entry::Permission {
                chosen: Some(1),
                by_setup: true,
                ..
            }
        ),
        "{entries:?}"
    );
    let agent = Arc::clone(&agents.borrow()[2]);
    assert_eq!(
        agent.prompt_blocks(),
        [[
            "You were set up with this context for the whole session. Keep to it.\n\nChange nothing.\n".to_owned(),
            "permission".to_owned()
        ]]
    );
    // Which chat opened it, and how, is kept: Cox started again lists it
    // under its parent, set up the same.
    let again = start_again(cx, home.clone(), &store, &agents);
    assert_eq!(
        names(cx, &again),
        listed(&[("Cox", &["hello", "permission", "second"])])
    );
    let (parent, _) = keys(cx, &again, 0);
    let (_, opened_by) = keys(cx, &again, 1);
    assert_eq!(opened_by, Some(parent));
    let chats = store.chats().unwrap();
    assert_eq!(chats[2].parent, Some(chats[0].id));
    assert_eq!(
        Setup::from_json(chats[2].setup.as_deref().unwrap()).unwrap(),
        setup
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

/// A Git repository at a scratch folder named after `name`, with
/// `main.rs` committed, and someone to commit as.
fn repository(name: &str) -> PathBuf {
    let home = scratch(name);
    std::fs::create_dir_all(&home).unwrap();
    let git = |args: &[&str]| {
        let done = std::process::Command::new("git")
            .arg("-C")
            .arg(&home)
            .args(args)
            .output()
            .unwrap();
        assert!(done.status.success(), "{done:?}");
    };
    git(&["init", "-q"]);
    git(&["config", "user.name", "t"]);
    git(&["config", "user.email", "t@t"]);
    std::fs::write(home.join("main.rs"), "fn main() {}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "first"]);
    home
}

/// Checks that the bundle the note `note` of a discarded sketch names makes
/// the sketch again, with `file` holding `text`, and takes it away.
fn assert_made_again(note: &str, file: &str, text: &str) {
    let bundle = note
        .split_once("kept in ")
        .and_then(|(_, rest)| rest.split_once(", which"))
        .map_or_else(|| panic!("{note}"), |(bundle, _)| PathBuf::from(bundle));
    let again = scratch("sketch-made-again");
    let cloned = std::process::Command::new("git")
        .arg("clone")
        .arg("-q")
        .arg(&bundle)
        .arg(&again)
        .status()
        .unwrap();
    assert!(cloned.success());
    assert_eq!(std::fs::read_to_string(again.join(file)).unwrap(), text);
    let _ = std::fs::remove_file(&bundle);
    let _ = std::fs::remove_dir_all(&again);
}

/// Waits until `made`, made in a sketch the human discards, is gone with
/// the sketch's folder, once the sketch's agent has summed the sketch up.
async fn wait_for_discarded(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    made: &std::path::Path,
) {
    let (timed_out, timeout) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(GIVEN_UP);
        let _ = timed_out.send(());
    });
    let made = made.to_owned();
    tokio::select! {
        () = cx.condition(workspace, move |_, _| !made.exists()) => {}
        _ = timeout => panic!("the sketch was never discarded"),
    }
}

/// Opens a sketch that starts from `sketch` from the first chat, as the
/// chat's agent would, waits for the list to hold `count` chats, and
/// answers the folder the sketch works in.
async fn open_sketch(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    asks: &Asks,
    sketch: Sketch,
    count: usize,
) -> PathBuf {
    let (answer, opened) = tokio::sync::oneshot::channel();
    asks.borrow()[0]
        .send(ChatRequest::OpenChat {
            text: "try it".to_owned(),
            setup: Setup::default(),
            goal: None,
            sketch: Some(sketch),
            answer,
        })
        .unwrap();
    cx.condition(workspace, move |workspace, _| {
        workspace.chats(0).count() == count
    })
    .await;
    let folder = opened
        .await
        .unwrap()
        .unwrap()
        .folder
        .expect("a sketch's folder");
    let sketches = cox::storage::path(cox::storage::Kept::Sketches).unwrap();
    assert!(folder.starts_with(&sketches), "{}", folder.display());
    settle_on(cx, workspace).await;
    folder
}

#[gpui::test]
async fn a_sketch_works_in_a_folder_apart_from_the_project(cx: &mut TestAppContext) {
    let home = repository("sketch-home");
    let store = kept_store("sketch-kept");
    let (agents, asks, workspace, cx) =
        open_window_asking(cx, home.clone(), store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;

    let folder = open_sketch(cx, &workspace, &asks, Sketch::Project, 2).await;
    assert_eq!(
        std::fs::read_to_string(folder.join("main.rs")).unwrap(),
        "fn main() {}\n"
    );
    // Its agent works there, changes files unasked, and the human is
    // told where.
    let child = view_at(cx, &workspace, 1);
    wait_for_turn(cx, &child).await;
    assert_eq!(agents.borrow()[1].cwd(), Some(folder.clone()));
    let entries = entries(cx, &child);
    assert!(
        entries.iter().any(
            |entry| matches!(entry, Entry::Note(note) if note.starts_with("Sketch: works in"))
        ),
        "{entries:?}"
    );
    let setup = workspace.read_with(cx, |workspace, _| {
        workspace.chats(0).nth(1).unwrap().setup().cloned().unwrap()
    });
    assert_eq!(setup.folder.as_ref(), Some(&folder));
    assert_eq!(setup.sketch, Some(Sketch::Project));
    assert!(setup.policy.allow.contains(&"edit:*".to_owned()));
    assert_eq!(setup.policy.within.as_ref(), Some(&folder));

    // A sketch from nothing works in an empty repository of its own.
    let empty = open_sketch(cx, &workspace, &asks, Sketch::Empty, 3).await;
    assert!(empty.join(".git").is_dir());
    assert!(!empty.join("main.rs").exists());

    let _ = std::fs::remove_dir_all(&empty);
    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

#[gpui::test]
async fn a_chat_opens_a_sketch_beside_it(cx: &mut TestAppContext) {
    let home = repository("sketch-beside-home");
    let store = kept_store("sketch-beside-kept");
    let (agents, asks, workspace, cx) =
        open_window_asking(cx, home.clone(), store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let start = |cx: &mut VisualTestContext, from: usize| {
        let (answer, made) = tokio::sync::oneshot::channel();
        asks.borrow()[from]
            .send(ChatRequest::StartSketch {
                sketch: Sketch::Project,
                what: "A warmer greeting.".to_owned(),
                show: false,
                answer,
            })
            .unwrap();
        cx.run_until_parked();
        made
    };

    // The sketch opens as a chat of its own, under the chat, whose agent
    // works in the sketch's folder, told what the chat talked through and
    // what it tries.
    let opened = start(cx, 0).await.unwrap().unwrap();
    let folder = opened.folder.clone().expect("a sketch's folder");
    let sketches = cox::storage::path(cox::storage::Kept::Sketches).unwrap();
    assert!(folder.starts_with(&sketches), "{}", folder.display());
    assert_eq!(opened.name, "A warmer greeting.");
    cx.condition(&workspace, |workspace, _| workspace.chats(0).count() == 2)
        .await;
    let sketch = view_at(cx, &workspace, 1);
    let (parent, setup, shown_view) = workspace.read_with(cx, |workspace, _| {
        (
            workspace.chats(0).next().unwrap().setup().cloned(),
            workspace.chats(0).nth(1).unwrap().setup().cloned().unwrap(),
            workspace.shown().and_then(Chat::view).cloned(),
        )
    });
    wait_for_turn(cx, &sketch).await;
    assert_eq!(agents.borrow().len(), 2);
    assert_eq!(agents.borrow()[1].cwd(), Some(folder.clone()));
    let told = agents.borrow()[1].prompts().concat();
    assert!(
        told.contains("This sketch was opened beside a chat"),
        "{told}"
    );
    assert!(told.contains("[human]\nhello"), "{told}");
    assert!(told.contains("You work in a sketch now"), "{told}");
    assert_eq!(setup.sketch, Some(Sketch::Project));
    assert_eq!(setup.folder.as_ref(), Some(&folder));
    assert!(setup.policy.allow.contains(&"edit:*".to_owned()));
    assert_eq!(setup.policy.within.as_ref(), Some(&folder));
    // The chat stays in the project, shown, and notes the sketch.
    assert!(parent.is_none_or(|setup| setup.sketch.is_none() && setup.folder.is_none()));
    assert_eq!(agents.borrow()[0].cwd(), Some(home.clone()));
    let first = shown_view.unwrap();
    let entries = entries(cx, &first);
    assert!(
        entries.iter().any(|entry| matches!(entry, Entry::Note(note)
            if note.starts_with(&format!("Sketch: chat {} opens beside this chat", opened.number)))),
        "{entries:?}"
    );
    assert_eq!(
        names(cx, &workspace),
        listed(&[("Cox", &["hello", "A warmer greeting."])])
    );

    // A sketch does not open a sketch beside it.
    let again = start(cx, 1).await.unwrap();
    assert!(
        again
            .as_ref()
            .is_err_and(|error| error.contains("a sketch already")),
        "{again:?}"
    );

    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

#[gpui::test]
async fn the_human_starts_a_sketch_of_what_they_would_have_tried(cx: &mut TestAppContext) {
    let home = repository("sketch-human-home");
    let store = kept_store("sketch-human-kept");
    let (agents, _asks, workspace, cx) =
        open_window_asking(cx, home.clone(), store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;

    // "Start sketch…" opens the input on the chat; Escape closes it, and
    // the chat stays as it was.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.edit_sketch(0, window, cx);
    });
    let view = shown(cx, &workspace);
    let from = view.read_with(cx, |view, _| view.sketch_input().map(|(_, from)| from));
    // The project is a Git repository, so the sketch starts from it.
    assert_eq!(from, Some(Sketch::Project));
    click(cx, "sketch-from");
    let from = view.read_with(cx, |view, _| view.sketch_input().map(|(_, from)| from));
    assert_eq!(from, Some(Sketch::Empty));
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |view, _| assert!(view.sketch_input().is_none()));
    assert_eq!(agents.borrow().len(), 1);

    // Where the human says what to try, Enter opens a sketch of it beside
    // the chat, and shows it.
    right_click(cx, "chat-1");
    click(cx, "start-sketch");
    cx.simulate_input("Whether the greeting can name the user.");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.condition(&workspace, |workspace, _| workspace.chats(0).count() == 2)
        .await;
    let sketch = shown(cx, &workspace);
    assert_ne!(sketch, view);
    wait_for_turn(cx, &sketch).await;
    let setup = workspace.read_with(cx, |workspace, _| {
        workspace.chats(0).nth(1).unwrap().setup().cloned().unwrap()
    });
    assert_eq!(setup.sketch, Some(Sketch::Project));
    let folder = setup.folder.unwrap();
    assert_eq!(agents.borrow().last().unwrap().cwd(), Some(folder.clone()));
    assert_eq!(
        names(cx, &workspace),
        listed(&[("Cox", &["hello", "Whether the greeting can name the user."])])
    );
    let entries = entries(cx, &sketch);
    assert!(
        entries.iter().any(|entry| matches!(entry, Entry::Cox(line)
            if line.starts_with("You work in a sketch now")
                && line.ends_with("Whether the greeting can name the user."))),
        "{entries:?}"
    );

    // A sketch does not open a sketch beside it.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.edit_sketch(1, window, cx);
    });
    sketch.read_with(cx, |view, _| assert!(view.sketch_input().is_none()));

    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

#[gpui::test]
async fn an_ended_sketch_goes_on_in_the_project(cx: &mut TestAppContext) {
    let home = repository("sketch-on-home");
    let store = kept_store("sketch-on-kept");
    let (agents, asks, workspace, cx) =
        open_window_asking(cx, home.clone(), store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let folder = open_sketch(cx, &workspace, &asks, Sketch::Project, 2).await;
    // The sketch's agent opens a chat, which works in the sketch's folder.
    let (answer, _opened) = tokio::sync::oneshot::channel();
    asks.borrow()[1]
        .send(ChatRequest::OpenChat {
            text: "help".to_owned(),
            setup: Setup::default(),
            goal: None,
            sketch: None,
            answer,
        })
        .unwrap();
    cx.condition(&workspace, |workspace, _| workspace.chats(0).count() == 3)
        .await;
    let opened = view_at(cx, &workspace, 2);
    workspace.read_with(cx, |workspace, _| {
        let chat = workspace.chats(0).nth(2).unwrap();
        assert_eq!(chat.setup().unwrap().folder.as_ref(), Some(&folder));
    });

    // Discarded while its turn goes on, the sketch waits for the turn to
    // end, and for its agent to sum it up.
    workspace.update_in(cx, |workspace, window, cx| workspace.show(1, window, cx));
    let sketch = shown(cx, &workspace);
    cx.simulate_input("slow");
    cx.simulate_keystrokes("enter");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.end_sketch_as(1, own_ending("Discard sketch"), window, cx);
    });
    settled(cx, &workspace);
    let last = sketch.read_with(cx, |view, _| {
        assert_eq!(view.conversation().state(), State::Busy);
        view.conversation().entries().last().cloned()
    });
    assert!(
        matches!(&last, Some(Entry::Note(note)) if note.contains("once this turn ends")),
        "{last:?}"
    );
    assert!(folder.exists());
    wait_for_discarded(cx, &workspace, &folder).await;
    let prompts = agents.borrow()[1].prompts();
    assert!(
        prompts
            .last()
            .is_some_and(|prompt| prompt.starts_with("The human discards this sketch")),
        "{prompts:?}"
    );
    // Both close, and are set up to work in the project, asking the human.
    let setups = workspace.read_with(cx, |workspace, _| {
        workspace
            .chats(0)
            .skip(1)
            .map(|chat| (chat.is_open(), chat.setup().cloned().unwrap()))
            .collect::<Vec<_>>()
    });
    for (open, setup) in setups {
        assert!(!open);
        assert_eq!((setup.folder, setup.sketch), (None, None));
        assert_eq!(setup.policy, Policy::default());
    }
    let last = opened.read_with(cx, |view, _| {
        assert_eq!(view.conversation().state(), State::Closed);
        view.conversation().entries().last().cloned()
    });
    assert!(
        matches!(&last, Some(Entry::Note(note)) if note.contains("has ended")),
        "{last:?}"
    );

    // The chat the sketch was opened beside is shown; a line sent to the
    // sketch's chat starts its agent again, in the project.
    let first = view_at(cx, &workspace, 0);
    assert_eq!(shown(cx, &workspace), first);
    workspace.update_in(cx, |workspace, window, cx| workspace.show(1, window, cx));
    say(cx, &workspace, "go on").await;
    assert_eq!(agents.borrow().last().unwrap().cwd(), Some(home.clone()));
    let open = workspace.read_with(cx, |workspace, _| {
        workspace.chats(0).nth(1).unwrap().is_open()
    });
    assert!(open);

    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

/// The way to end a sketch of Cox's own workflow named `name`.
fn own_ending(name: &str) -> workflow::Ending {
    workflow::sketch_in(None)
        .endings
        .into_iter()
        .find(|ending| ending.name == name)
        .unwrap()
}

#[gpui::test]
async fn a_sketch_ends_in_a_way_of_the_human_s_own(cx: &mut TestAppContext) {
    let home = repository("sketch-rewrite-home");
    let store = kept_store("sketch-rewrite-kept");
    let (agents, asks, workspace, cx) =
        open_window_asking(cx, home.clone(), store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let folder = open_sketch(cx, &workspace, &asks, Sketch::Project, 2).await;

    // An ending of the human's own keeps what the sketch of the project
    // changed apart, and the chat the sketch was opened beside goes on
    // toward the ending's goal, its agent told what the human says then,
    // with where what the sketch changed is kept, and what the sketch's
    // agent said last.
    let sketch = view_at(cx, &workspace, 1);
    wait_for_turn(cx, &sketch).await;
    let summary = entries(cx, &sketch)
        .iter()
        .rev()
        .find_map(|entry| match entry {
            Entry::Reply(text) => Some(text.clone()),
            _ => None,
        })
        .unwrap();
    let then = format!("sketch/rewrite-{}.md", std::process::id());
    // The workflow is one of the test's own, where this Cox keeps what
    // lasts, rather than the human's in their home.
    let workflow = cox::storage::root().unwrap().join("workflow");
    cox::settings::keep(
        "workflow",
        Some(serde_json::Value::String(workflow.display().to_string())),
    )
    .unwrap();
    let then_file = cox::settings::workflow().join(&then);
    std::fs::create_dir_all(then_file.parent().unwrap()).unwrap();
    std::fs::write(&then_file, "Rewrite what {kept} shows, from: {summary}\n").unwrap();
    std::fs::write(folder.join("main.rs"), "fn main() { sketch() }\n").unwrap();
    let rewrite = workflow::Ending {
        name: "Rewrite in project".to_owned(),
        from: Some(Sketch::Project),
        sum_up: None,
        keep: true,
        open: false,
        then: Some(then),
        goal: Some("$ true".to_owned()),
    };
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.end_sketch_as(1, rewrite, window, cx);
    });
    settled(cx, &workspace);
    wait_for_discarded(cx, &workspace, &folder).await;
    settle(cx).await;
    let _ = std::fs::remove_file(&then_file);
    let name = format!(
        "refs/cox/discarded/{}",
        folder.file_name().unwrap().to_string_lossy()
    );
    let in_git = std::process::Command::new("git")
        .arg("-C")
        .arg(&home)
        .arg("show")
        .arg(format!("{name}:main.rs"))
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&in_git.stdout),
        "fn main() { sketch() }\n"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("main.rs")).unwrap(),
        "fn main() {}\n"
    );
    assert_eq!(
        goals(cx, &workspace, 0),
        [Some(KeptGoal::Command("true".to_owned())), None]
    );
    // It is shown, for the human to go on there.
    let first = shown(cx, &workspace);
    let parent_view = view_at(cx, &workspace, 0);
    assert_eq!(first, parent_view);
    let entries = entries(cx, &first);
    assert!(
        entries.iter().any(|entry| matches!(entry, Entry::Note(note)
            if note.contains("ended: The sketch is discarded"))),
        "{entries:?}"
    );
    let told = format!("Rewrite what {name} shows, from: {summary}");
    let rewriting = agents.clone();
    let home_again = home.clone();
    cx.condition(&workspace, move |_, _| {
        let agent = &rewriting.borrow()[0];
        agent.cwd().as_ref() == Some(&home_again)
            && agent
                .prompts()
                .last()
                .is_some_and(|prompt| prompt.contains(&told))
    })
    .await;
    let endings = workspace.read_with(cx, |workspace, _| workspace.endings(1));
    assert!(endings.is_empty(), "{endings:?}");

    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

#[gpui::test]
async fn a_sketch_ends_as_cox_s_own_workflow_says(cx: &mut TestAppContext) {
    let home = repository("sketch-end-home");
    let store = kept_store("sketch-end-kept");
    let (_agents, asks, workspace, cx) =
        open_window_asking(cx, home.clone(), store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let folder = open_sketch(cx, &workspace, &asks, Sketch::Project, 2).await;
    let empty = open_sketch(cx, &workspace, &asks, Sketch::Empty, 3).await;
    let ended = |cx: &mut VisualTestContext, index: usize| {
        let view = view_at(cx, &workspace, index);
        workspace.read_with(cx, |workspace, cx| {
            let chat = workspace.chats(0).nth(index).unwrap();
            let view = view.read(cx);
            let last = view.conversation().entries().last().cloned();
            (chat.is_open(), view.conversation().state(), last)
        })
    };
    let names = |cx: &mut VisualTestContext, index: usize| {
        workspace.read_with(cx, |workspace, _| {
            workspace
                .endings(index)
                .into_iter()
                .map(|ending| ending.name)
                .collect::<Vec<_>>()
        })
    };
    // Cox's own workflow opens only a sketch from nothing as a project.
    assert_eq!(names(cx, 1), ["Discard sketch"]);
    assert_eq!(names(cx, 2), ["Open as project", "Discard sketch"]);

    // The sketch from nothing is discarded, its folder with it. Another
    // test may make a sketch from nothing in a folder of the same name once
    // this one is gone, so what goes is what this sketch made.
    let made = empty.join("made.txt");
    std::fs::write(&made, "made in the sketch\n").unwrap();
    // The menu of the right button offers the ways it ends.
    right_click(cx, "chat-3");
    click(cx, "sketch-ending-1");
    wait_for_discarded(cx, &workspace, &made).await;
    let (open, state, last) = ended(cx, 2);
    assert!(!open);
    assert_eq!(state, State::Closed);
    // Its agent summed it up first, asked in the human's place.
    let entries = {
        let view = view_at(cx, &workspace, 2);
        self::entries(cx, &view)
    };
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            Entry::Cox(line) if line.starts_with("The human discards this sketch")
        )),
        "{entries:?}"
    );
    // What it made is kept in a bundle, which makes it again.
    let Some(Entry::Note(note)) = &last else {
        panic!("{last:?}");
    };
    assert_made_again(note, "made.txt", "made in the sketch\n");

    // Where the human goes on, the chat it was opened beside, they are
    // asked whether to keep what the sketch found in the wiki, which its
    // own agent, which found it, is then asked to, in the project.
    let beside = shown(cx, &workspace);
    let sketch = view_at(cx, &workspace, 2);
    let sketch_kept =
        workspace.read_with(cx, |workspace, _| workspace.chats(0).nth(2).unwrap().kept());
    let asked = self::entries(cx, &beside);
    let at = asked.len() - 1;
    assert!(
        matches!(&asked[at], Entry::Keep { why, chat, kept: None }
            if why.starts_with("The sketch \"") && why.ends_with("\" ended.") && chat == &sketch_kept),
        "{asked:?}"
    );
    assert!(sketch_kept.is_some());
    cx.run_until_parked();
    click(cx, Box::leak(format!("keep-in-wiki-{at}").into_boxed_str()));
    wait_for_turn(cx, &sketch).await;
    let save = workflow::WIKI_SAVE.say(&[]);
    assert!(self::entries(cx, &sketch).contains(&Entry::Cox(save)));

    // Another opens as a project of its own, in the projects of the home,
    // its folder a directory of it named as it is, which the human goes
    // on with, so it is no longer discarded.
    let empty = open_sketch(cx, &workspace, &asks, Sketch::Empty, 4).await;
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.end_sketch_as(3, own_ending("Open as project"), window, cx);
        workspace.end_sketch_as(3, own_ending("Discard sketch"), window, cx);
    });
    settled(cx, &workspace);
    assert!(!empty.exists());
    let project = opened_as_project(cx, &workspace, &home);

    let _ = std::fs::remove_dir_all(&project);
    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(store.path.parent().unwrap());
}

/// The project the sketch at 3 was opened as, the first with nothing
/// set yet in the projects of `home`, after checking that the window
/// lists it, that the sketch's folder is its directory named as it is,
/// and that the sketch says so.
fn opened_as_project(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    home: &Path,
) -> PathBuf {
    let project = home.join("projects").join("untitled-1");
    let listed: Vec<PathBuf> = workspace.read_with(cx, |workspace, _| {
        workspace
            .projects()
            .iter()
            .map(|kept| kept.folder().to_owned())
            .collect()
    });
    // What the sketch says of how it ended tells why, when it did not.
    let sketch = view_at(cx, workspace, 3);
    let said = self::entries(cx, &sketch);
    assert!(
        listed.contains(&project),
        "{} is not among {listed:?}: {said:?}",
        project.display()
    );
    assert!(
        project
            .join("directories")
            .join("untitled-1")
            .join(".git")
            .exists()
    );
    assert!(
        said.iter().any(|entry| matches!(entry,
            Entry::Note(note) if note.starts_with("The sketch was opened as the project untitled-1"))),
    );
    project
}

/// Opens a chat from the first chat, on `text`, set up with nothing, as
/// the chat's agent would, and waits for the list to hold `count` chats.
async fn open_child(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    asks: &Asks,
    text: &str,
    count: usize,
) {
    let (answer, _opened) = tokio::sync::oneshot::channel();
    asks.borrow()[0]
        .send(ChatRequest::OpenChat {
            text: text.to_owned(),
            setup: Setup::default(),
            goal: None,
            sketch: None,
            answer,
        })
        .unwrap();
    cx.condition(workspace, move |workspace, _| {
        workspace.chats(0).count() == count
    })
    .await;
    settle_on(cx, workspace).await;
}

#[gpui::test]
async fn an_agent_waits_for_the_chat_it_opened(cx: &mut TestAppContext) {
    let home = scratch("wait-home");
    let store = kept_store("chats-11");
    let (_, asks, workspace, cx) = open_window_asking(cx, home.clone(), store, Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;
    let ask = |request| asks.borrow()[0].send(request).unwrap();
    let view_of = |cx: &mut VisualTestContext, index| view_at(cx, &workspace, index);

    // Waiting for a chat at work answers once its turn ends.
    open_child(cx, &workspace, &asks, "slow hello", 2).await;
    let (slow, _) = keys(cx, &workspace, 1);
    let (answer, mut waited) = tokio::sync::oneshot::channel();
    ask(ChatRequest::WaitChat { chat: slow, answer });
    cx.run_until_parked();
    assert!(waited.try_recv().is_err(), "answered before the turn ended");
    let view = view_of(cx, 1);
    wait_for_turn(cx, &view).await;
    cx.run_until_parked();
    assert_eq!(
        waited.try_recv().unwrap(),
        Ok(Waited {
            asking: None,
            said: Some("echo: slow hello".to_owned()),
        })
    );

    // Waiting for a chat that comes to wait for the human answers then,
    // with what it asks.
    open_child(cx, &workspace, &asks, "permission", 3).await;
    let (asking, _) = keys(cx, &workspace, 2);
    let view = view_of(cx, 2);
    let (answer, waited) = tokio::sync::oneshot::channel();
    ask(ChatRequest::WaitChat {
        chat: asking,
        answer,
    });
    wait_until(cx, &view, |conversation| {
        conversation.state() == State::Asking
    })
    .await;
    cx.run_until_parked();
    assert_eq!(
        waited.await.unwrap(),
        Ok(Waited {
            asking: Some("Edit x.txt".to_owned()),
            said: None,
        })
    );
    // A chat not at work answers at once, as it stands.
    click(cx, "chat-3");
    click(cx, "option-1");
    wait_for_turn(cx, &view).await;
    let (answer, waited) = tokio::sync::oneshot::channel();
    ask(ChatRequest::WaitChat {
        chat: asking,
        answer,
    });
    cx.run_until_parked();
    assert_eq!(
        waited.await.unwrap(),
        Ok(Waited {
            asking: None,
            said: Some("echo: permission".to_owned()),
        })
    );

    // A chat there is none of says so.
    let (answer, waited) = tokio::sync::oneshot::channel();
    ask(ChatRequest::WaitChat { chat: 99, answer });
    cx.run_until_parked();
    assert_eq!(waited.await.unwrap(), Err("no chat 99".to_owned()));
    let _ = std::fs::remove_dir_all(&home);
}

#[gpui::test]
async fn logs_in_in_a_terminal_and_sends_the_line_again(cx: &mut TestAppContext) {
    let home = scratch("login-home");
    std::fs::create_dir_all(&home).unwrap();
    let file = home.join("logged in");
    let (agents, workspace, cx) = open_window_logging_in(cx, home, LogsIn::AtPrompt(file.clone()));

    cx.simulate_keystrokes("secondary-n");
    let view = shown(cx, &workspace);
    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    cx.condition(&view, |view, _| view.conversation().logging_in())
        .await;
    // The third way runs the agent's command in a terminal, as
    // `touch '<file>'`, which logs the agent in.
    cx.simulate_keystrokes("3");
    wait_for_turn(cx, &view).await;

    assert!(file.exists());
    let agent = Arc::clone(&agents.borrow()[0]);
    assert_eq!(agent.logged_in(), Vec::<String>::new());
    assert_eq!(agent.prompts(), ["hello"]);
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.conversation().entries().last(),
            Some(&Entry::Reply("echo: hello".to_owned()))
        );
    });
}

#[gpui::test]
async fn the_human_asks_a_chat_to_keep_what_it_learned_in_the_wiki(cx: &mut TestAppContext) {
    let home = scratch("wiki-home");
    let store = kept_store("wiki-kept");
    let (agents, asks, workspace, cx) =
        open_window_asking(cx, home, store.clone(), Resumes::Resume);
    cx.simulate_keystrokes("secondary-n");
    say(cx, &workspace, "hello").await;

    // A page the agent writes says where it was used: the chat's
    // project, and the chat, by its name and the number it is kept by.
    let (answer, heard) = tokio::sync::oneshot::channel();
    asks.borrow()[0]
        .send(ChatRequest::Whereabouts { answer })
        .unwrap();
    cx.run_until_parked();
    let whereabouts = heard.await.unwrap();
    assert_eq!(whereabouts.project, "Cox");
    assert_eq!(whereabouts.chat, "hello");
    assert!(whereabouts.kept.is_some());

    // "Save to wiki" asks the agent, in the human's place, to keep what
    // the chat learned there.
    right_click(cx, "chat-1");
    click(cx, "save-to-wiki");
    let view = shown(cx, &workspace);
    wait_for_turn(cx, &view).await;
    let asked = workflow::WIKI_SAVE.say(&[]);
    assert_eq!(agents.borrow()[0].prompts().last(), Some(&asked));
    assert!(asked.contains("wiki_write"), "{asked}");
    assert!(entries(cx, &view).contains(&Entry::Cox(asked)));
}
