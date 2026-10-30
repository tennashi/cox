//! A human opening chats in projects, moving between them, reading the
//! closed ones again and going on with them, with each chat's agent faked
//! in memory.

#[expect(dead_code, reason = "these tests use only part of the fake agent")]
mod fake_agent;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use cox::agent::Start;
use cox::agent::instructions::Instructions;
use cox::chat::conversation::{Entry, State};
use cox::chat::view::{Again, ChatView};
use cox::chat::{Closed, Opens};
use cox::workspace::{Chat, Workspace};
use cox_core::store::{Record, Store};
use fake_agent::{FakeAgent, Resumes};
use gpui::{
    App, AppContext as _, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext, Window,
};

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

/// Opens chats with no stage, each with its agent faked, added to
/// `agents`. Sent to, a closed chat's agent starts again, faked, going on
/// with its session as `resumes` says.
struct FakeOpens {
    agents: Agents,
    resumes: Resumes,
}

impl Opens for FakeOpens {
    fn open(
        &self,
        cwd: PathBuf,
        instructions: Option<Instructions>,
        record: Option<Record>,
        _: &mut Window,
        cx: &mut App,
    ) -> Entity<ChatView> {
        let agent = Arc::new(FakeAgent::default());
        self.agents.borrow_mut().push(Arc::clone(&agent));
        let start = Start::New(instructions);
        let agent = cox::agent::spawn(move || Ok(agent.connect()), cwd, Vec::new(), start);
        cx.new(|cx| ChatView::new(agent, None, Rc::new(|_, _, _| {}), record, cx))
    }

    fn read(&self, closed: Closed, window: &mut Window, cx: &mut App) -> Entity<ChatView> {
        let Closed {
            cwd,
            entries,
            session,
            record,
            ..
        } = closed;
        let opened = Rc::clone(&self.agents);
        let resumes = self.resumes;
        let again: Again = Box::new(move |entries, _, _| {
            let agent = Arc::new(FakeAgent::resuming(resumes));
            opened.borrow_mut().push(Arc::clone(&agent));
            let start = Start::again(&cwd, session, entries);
            cox::agent::spawn(move || Ok(agent.connect()), cwd, Vec::new(), start)
        });
        let link = Rc::new(|_: &str, _: &mut Window, _: &mut App| {});
        cx.new(|cx| ChatView::closed(entries, None, link, record, Some(again), window, cx))
    }
}

/// What opens the chats of a test's window, with their agents added to
/// `agents`.
fn opens(agents: &Agents, resumes: Resumes) -> Rc<dyn Opens> {
    Rc::new(FakeOpens {
        agents: Rc::clone(agents),
        resumes,
    })
}

/// Opens the window like [`open_window`], keeping the chats in `store`.
fn open_window_keeping(
    cx: &mut TestAppContext,
    home: PathBuf,
    store: Rc<Store>,
) -> (Agents, Entity<Workspace>, &mut VisualTestContext) {
    open_window_resuming(cx, home, store, Resumes::Resume)
}

/// Opens the window like [`open_window_keeping`], with agents that go on
/// with a closed chat's session as `resumes` says.
fn open_window_resuming(
    cx: &mut TestAppContext,
    home: PathBuf,
    store: Rc<Store>,
    resumes: Resumes,
) -> (Agents, Entity<Workspace>, &mut VisualTestContext) {
    // The agents run on threads of their own, so the test waits for them
    // in real time instead of only running GPUI's own tasks.
    cx.executor().allow_parking();
    cx.update(|cx| {
        cox::ui::input::bind_keys(cx);
        cox::chat::view::bind_keys(cx);
        cox::workspace::bind_keys(cx);
    });
    let agents = Agents::default();
    let opens = opens(&agents, resumes);
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(home, store, opens, cx));
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

fn shown(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) -> Entity<ChatView> {
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
    assert!(note.starts_with("Instructions: "), "{note}");
    assert!(note.ends_with("COX.md"), "{note}");
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

/// The entries of the chat `view` shows.
fn entries(cx: &mut VisualTestContext, view: &Entity<ChatView>) -> Vec<Entry> {
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
        (0..workspace.projects().len())
            .map(|project| workspace.chats(project).map(Chat::is_open).collect())
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
        let opens = opens(&agents, Resumes::Resume);
        cx.new(|cx| Workspace::new(home.clone(), store, opens, cx))
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
) -> (Rc<Store>, cox_core::store::ChatId) {
    let store = Rc::new(Store::in_memory().unwrap());
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
/// after no project, which has none.
const FIRST_KEPT: usize = 0;

#[gpui::test]
async fn a_closed_chat_goes_on_with_its_session_when_the_human_sends_to_it(
    cx: &mut TestAppContext,
) {
    let folder = scratch("resume");
    let (store, kept) = kept_chat(&folder, Some("session-1"));
    let (agents, workspace, cx) = open_window_resuming(
        cx,
        scratch("resume-home"),
        Rc::clone(&store),
        Resumes::Resume,
    );
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
/// chat says so.
fn assert_retold(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    agent: &FakeAgent,
    folder: &std::path::Path,
) {
    let instructions = Instructions::read(folder).unwrap();
    let blocks = agent.prompt_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].len(), 3, "{blocks:?}");
    assert_eq!(blocks[0][0], instructions.prompt());
    assert!(
        blocks[0][1].ends_with("[human]\nhello\n\n[you]\necho: hello"),
        "{blocks:?}"
    );
    assert_eq!(blocks[0][2], "again");
    let shown = shown_entries(cx, workspace);
    assert_eq!(shown[3], Entry::Human("again".to_owned()));
    assert!(
        matches!(&shown[4], Entry::Note(note) if note.contains("could not resume")),
        "{shown:?}"
    );
    assert!(
        matches!(&shown[5], Entry::Note(note) if note.starts_with("Instructions: ")),
        "{shown:?}"
    );
}

#[gpui::test]
async fn an_agent_that_cannot_resume_is_told_the_conversation_so_far(cx: &mut TestAppContext) {
    let folder = scratch("retell");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("COX.md"), "Answer in Japanese.\n").unwrap();
    let (store, kept) = kept_chat(&folder, Some("session-0"));
    let (agents, workspace, cx) = open_window_resuming(
        cx,
        scratch("retell-home"),
        Rc::clone(&store),
        Resumes::Never,
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.show(FIRST_KEPT, window, cx);
    });

    say(cx, &workspace, "again").await;

    let agent = Arc::clone(&agents.borrow()[0]);
    assert_eq!(agent.resumed(), Vec::<String>::new());
    assert_eq!(agent.sessions_opened(), 1);
    assert_retold(cx, &workspace, &agent, &folder);
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
    assert_retold(cx, &workspace, &agent, &folder);
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
    assert_retold(cx, &workspace, &agent, &folder);
    let _ = std::fs::remove_dir_all(&folder);
}

#[gpui::test]
async fn a_new_chat_keeps_its_agents_session(cx: &mut TestAppContext) {
    let store = Rc::new(Store::in_memory().unwrap());
    let (_, workspace, cx) = open_window_keeping(cx, scratch("session"), Rc::clone(&store));
    click(cx, "new-chat");
    say(cx, &workspace, "hello").await;

    let chats = store.chats().unwrap();
    assert_eq!(
        store.session(chats[0].id).unwrap().as_deref(),
        Some("session-1")
    );
}
