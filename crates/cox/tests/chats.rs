//! A human opening chats in projects, moving between them, closing them and
//! reading them again, with each chat's agent faked in memory.

#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cox::conversation::{Entry, State};
use cox::instructions::Instructions;
use cox::view::SessionView;
use cox::workspace::{Chat, Open, Place, Read, Workspace};
use cox_core::store::Store;
use fake_agent::FakeAgent;
use gpui::{AppContext as _, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};

/// The agent of each chat opened, in the order they were opened.
type Agents = Rc<RefCell<Vec<Arc<FakeAgent>>>>;

/// Opens the window with no chat, chats in no project working in `home`,
/// keeping them in memory.
fn open_window(
    cx: &mut TestAppContext,
    home: PathBuf,
) -> (Agents, Entity<Workspace>, &mut VisualTestContext) {
    open_window_keeping(cx, home, Rc::new(Store::in_memory().unwrap()))
}

/// Starts each chat's agent, faked, and adds it to `agents`.
fn opener(agents: &Agents) -> Open {
    let opened = Rc::clone(agents);
    Rc::new(move |cwd, instructions, record, _, cx| {
        let agent = Arc::new(FakeAgent::default());
        opened.borrow_mut().push(Arc::clone(&agent));
        let agent = cox::agent::spawn(move || Ok(agent.connect()), cwd, Vec::new(), instructions);
        cx.new(|cx| SessionView::new(agent, None, Rc::new(|_, _, _| {}), record, cx))
    })
}

/// Shows closed chats, with no stage.
fn reader() -> Read {
    Rc::new(|_, entries, _, _, cx| {
        cx.new(|cx| SessionView::closed(entries, None, Rc::new(|_, _, _| {}), cx))
    })
}

/// Opens the window like [`open_window`], keeping the chats in `store`.
fn open_window_keeping(
    cx: &mut TestAppContext,
    home: PathBuf,
    store: Rc<Store>,
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
    let open = opener(&agents);
    let (workspace, cx) =
        cx.add_window_view(|_, cx| Workspace::new(home, store, open, reader(), cx));
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
        let chat = workspace.shown().expect("a chat is shown");
        chat.view().expect("the chat shown has a view").clone()
    })
}

/// The name of each project, with the names of the chats in it.
fn names(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Vec<(String, Vec<String>)> {
    workspace.read_with(cx, |workspace, _| {
        workspace
            .projects()
            .iter()
            .map(|project| {
                let chats = project.chats().iter().map(|chat| chat.name().to_owned());
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

fn input_text(cx: &mut VisualTestContext, view: &Entity<SessionView>) -> String {
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
async fn chats_in_a_project_work_in_its_folder(cx: &mut TestAppContext) {
    let home = scratch("home");
    let folder = scratch("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let (agents, workspace, cx) = open_window(cx, home.clone());

    click(cx, "new-chat");
    say(cx, &workspace, "hello").await;
    for _ in 0..2 {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open(Some(folder.clone()), window, cx);
        });
        say(cx, &workspace, "hello").await;
    }
    // The heading of the project opens a chat in it, and so does no
    // project's.
    click(cx, "new-chat-in-2");
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
    assert_eq!(
        names(cx, &workspace),
        listed(&[
            ("No project", &["Chat 1", "Chat 2"]),
            (&name, &["Chat 1", "Chat 2", "Chat 3"]),
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
fn opening_the_home_as_a_folder_opens_a_chat_in_no_project(cx: &mut TestAppContext) {
    let home = scratch("home-folder");
    let (_agents, workspace, cx) = open_window(cx, home.clone());

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(home.clone()), window, cx);
    });

    assert_eq!(
        names(cx, &workspace),
        listed(&[("No project", &["Chat 1"])])
    );
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
    click(cx, "new-chat");
    say(cx, &workspace, "hello").await;

    let instructions = Instructions::read(&folder).unwrap();
    assert_eq!(
        agents.borrow()[0].prompt_blocks(),
        [
            vec![instructions.prompt(), "hello".to_owned()],
            vec!["again".to_owned()],
        ]
    );
    // The home has no instructions.
    assert_eq!(
        agents.borrow()[1].prompt_blocks(),
        [vec!["hello".to_owned()]]
    );
    click(cx, "chat-2");
    let view = shown(cx, &workspace);
    let first = view.read_with(cx, |view, _| view.conversation().entries()[0].clone());
    let Entry::Note(note) = first else {
        panic!("the chat opens with a note, not {first:?}");
    };
    assert!(note.starts_with("[instructions: "), "{note}");
    assert!(note.ends_with("COX.md]"), "{note}");
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

/// The entries of the chat `view` shows.
fn entries(cx: &mut VisualTestContext, view: &Entity<SessionView>) -> Vec<Entry> {
    view.read_with(cx, |view, _| view.conversation().entries().to_vec())
}

/// The entries of the chat shown.
fn shown_entries(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Vec<Entry> {
    let view = shown(cx, workspace);
    entries(cx, &view)
}

/// Whether each chat of each project is open.
fn opened(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Vec<Vec<bool>> {
    workspace.read_with(cx, |workspace, _| {
        workspace
            .projects()
            .iter()
            .map(|project| project.chats().iter().map(Chat::is_open).collect())
            .collect()
    })
}

#[gpui::test]
fn a_closed_chat_is_there_to_read(cx: &mut TestAppContext) {
    let store = Rc::new(Store::in_memory().unwrap());
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
    cx.simulate_input("again");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(shown_entries(cx, &workspace), said);
    assert_eq!(agents.borrow().len(), 1, "a closed chat starts no agent");

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
    let kept = scratch("again-kept").join("chats.sqlite");
    let store = Rc::new(Store::open(&kept).unwrap());
    let (agents, workspace, cx) = open_window_keeping(cx, home.clone(), store);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open(Some(folder.clone()), window, cx);
    });
    cx.simulate_input("permission");
    cx.simulate_keystrokes("enter");
    let view = shown(cx, &workspace);
    cx.condition(&view, |view, _| {
        view.conversation().state() == State::Asking
    })
    .await;
    click(cx, "option-1");
    wait_for_turn(cx, &view).await;
    click(cx, "new-chat");
    say(cx, &workspace, "hello").await;
    let first = shown_entries(cx, &workspace);
    click(cx, "chat-2");
    let second = shown_entries(cx, &workspace);

    // As Cox opens its window when it starts again, with every chat
    // closed.
    let again = cx.update(|_, cx| {
        let store = Rc::new(Store::open(&kept).unwrap());
        cx.new(|cx| Workspace::new(home.clone(), store, opener(&agents), reader(), cx))
    });
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        names(cx, &again),
        listed(&[("No project", &["Chat 1"]), (&name, &["Chat 1"])])
    );
    assert_eq!(opened(cx, &again), [vec![false], vec![false]]);
    let read = |cx: &mut VisualTestContext, place| {
        again.update_in(cx, |workspace, window, cx| {
            workspace.show(place, window, cx);
        });
        shown_entries(cx, &again)
    };
    assert_eq!(
        read(
            cx,
            Place {
                project: 0,
                chat: 0
            }
        ),
        first
    );
    let read_second = read(
        cx,
        Place {
            project: 1,
            chat: 0,
        },
    );
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
        listed(&[("No project", &["Chat 1"]), (&name, &["Chat 1", "Chat 2"])])
    );
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&folder);
    let _ = std::fs::remove_dir_all(kept.parent().unwrap());
}

#[gpui::test]
async fn stopping_with_the_button_leaves_the_keyboard_in_the_input(cx: &mut TestAppContext) {
    let (agents, workspace, cx) = open_window(cx, scratch("stop"));
    click(cx, "new-chat");
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
