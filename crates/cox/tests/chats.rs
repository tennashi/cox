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
use cox::agent::mcp::{ChatRequest, Ended, Ran, ToolRequest, Waited};
use cox::agent::setup::Setup;
use cox::chat::conversation::{Conversation, Entry, Goal, State};
use cox::chat::view::{Again, ChatView};
use cox::chat::{Closed, Opens};
use cox::notices::{AppNotices, Notices};
use cox::workspace::{Chat, Workspace};
use cox_core::goal::Goal as KeptGoal;
use cox_core::store::Named;
use cox_core::store::{Record, Store};
use fake_agent::{FakeAgent, Resumes, TITLE};
use gpui::{
    App, AppContext as _, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    TestAppContext, VisualTestContext, Window,
};
use tokio::sync::mpsc::UnboundedSender;

/// The agent of each chat opened, in the order they were opened.
type Agents = Rc<RefCell<Vec<Arc<FakeAgent>>>>;

/// The way to the window of each chat opened, in the order they were
/// opened, for a test to ask of it what the chat's agent would with Cox's
/// tools.
type Asks = Rc<RefCell<Vec<UnboundedSender<ChatRequest>>>>;

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
/// with its session as `resumes` says. The commands a chat runs for its
/// goal run in a shell in place of a terminal.
struct FakeOpens {
    agents: Agents,
    asks: Asks,
    resumes: Resumes,
}

/// Runs the commands a chat asks its stage to run, as a terminal would,
/// in a shell, and answers how each went. Dropping the sender ends it.
fn fake_terminal() -> tokio::sync::mpsc::UnboundedSender<ToolRequest> {
    let (asks, mut requests) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Some(request) = requests.blocking_recv() {
            let ToolRequest::RunCommand {
                command,
                cwd,
                answer,
            } = request
            else {
                continue;
            };
            let ran = std::process::Command::new("sh")
                .args(["-c", &command])
                .current_dir(cwd)
                .output()
                .map(|output| Ran {
                    output: String::from_utf8_lossy(&output.stdout).into_owned()
                        + &String::from_utf8_lossy(&output.stderr),
                    ended: output.status.code().map_or(Ended::Unknown, Ended::Exited),
                })
                .map_err(|error| error.to_string());
            let _ = answer.send(ran);
        }
    });
    asks
}

impl Opens for FakeOpens {
    fn open(
        &self,
        cwd: PathBuf,
        instructions: Option<Instructions>,
        setup: Option<Setup>,
        record: Option<Record>,
        _: &mut Window,
        cx: &mut App,
    ) -> Entity<ChatView> {
        let agent = Arc::new(FakeAgent::default());
        self.agents.borrow_mut().push(Arc::clone(&agent));
        let start = Start::New {
            instructions,
            setup,
        };
        let mut agent = cox::agent::spawn(move || Ok(agent.connect()), cwd, Vec::new(), start);
        agent.tools = Some(fake_terminal());
        let (asks, chat_requests) = tokio::sync::mpsc::unbounded_channel();
        self.asks.borrow_mut().push(asks);
        agent.chat_requests = Some(chat_requests);
        cx.new(|cx| ChatView::new(agent, None, Rc::new(|_, _, _| {}), record, cx))
    }

    fn read(&self, closed: Closed, window: &mut Window, cx: &mut App) -> Entity<ChatView> {
        let Closed {
            cwd,
            entries,
            session,
            goal,
            setup,
            record,
            ..
        } = closed;
        let opened = Rc::clone(&self.agents);
        let asked = Rc::clone(&self.asks);
        let resumes = self.resumes;
        let again: Again = Box::new(move |entries, _, _| {
            let agent = Arc::new(FakeAgent::resuming(resumes));
            opened.borrow_mut().push(Arc::clone(&agent));
            let start = Start::again(&cwd, session, entries, setup);
            let mut agent = cox::agent::spawn(move || Ok(agent.connect()), cwd, Vec::new(), start);
            agent.tools = Some(fake_terminal());
            let (asks, chat_requests) = tokio::sync::mpsc::unbounded_channel();
            asked.borrow_mut().push(asks);
            agent.chat_requests = Some(chat_requests);
            agent
        });
        let link = Rc::new(|_: &str, _: &mut Window, _: &mut App| {});
        let conversation = Conversation::closed(entries).with_goal(goal);
        cx.new(|cx| ChatView::closed(conversation, None, link, record, Some(again), window, cx))
    }
}

/// What opens the chats of a test's window, with their agents added to
/// `agents`, and their ways to the window to `asks`.
fn opens(agents: &Agents, asks: &Asks, resumes: Resumes) -> Rc<dyn Opens> {
    Rc::new(FakeOpens {
        agents: Rc::clone(agents),
        asks: Rc::clone(asks),
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
    let (agents, _, workspace, cx) = open_window_asking(cx, home, store, resumes);
    (agents, workspace, cx)
}

/// Opens the window like [`open_window_resuming`], with the way to the
/// window of each chat's agent too.
fn open_window_asking(
    cx: &mut TestAppContext,
    home: PathBuf,
    store: Rc<Store>,
    resumes: Resumes,
) -> (Agents, Asks, Entity<Workspace>, &mut VisualTestContext) {
    // The agents run on threads of their own, so the test waits for them
    // in real time instead of only running GPUI's own tasks.
    cx.executor().allow_parking();
    cx.update(|cx| {
        cox::ui::input::bind_keys(cx);
        cox::chat::view::bind_keys(cx);
        cox::workspace::bind_keys(cx);
    });
    let agents = Agents::default();
    let asks = Asks::default();
    let opens = opens(&agents, &asks, resumes);
    let (workspace, cx) = cx.add_window_view(|_, cx| Workspace::new(home, store, opens, cx));
    cx.update(|window, cx| window.focus(&workspace.focus_handle(cx)));
    (agents, asks, workspace, cx)
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
    for line in ["one", "two"] {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open(Some(folder.clone()), window, cx);
        });
        say(cx, &workspace, line).await;
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
    // A chat is named after the first line sent in it, and numbered until
    // then.
    assert_eq!(
        names(cx, &workspace),
        listed(&[
            ("No project", &["hello", "Chat 2"]),
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
        let opens = opens(&agents, &Asks::default(), Resumes::Resume);
        cx.new(|cx| Workspace::new(home.clone(), store, opens, cx))
    });
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        names(cx, &again),
        listed(&[("No project", &["hello"]), (&name, &["permission"])])
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
        listed(&[
            ("No project", &["hello"]),
            (&name, &["permission", "Chat 2"])
        ])
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
    let position = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center();
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
    let position = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center();
    cx.simulate_mouse_down(position, MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(position, MouseButton::Right, Modifiers::none());
}

#[gpui::test]
async fn a_chat_is_named_by_its_agent_until_the_human_names_it(cx: &mut TestAppContext) {
    let store = Rc::new(Store::in_memory().unwrap());
    let (_, workspace, cx) = open_window_keeping(cx, scratch("named"), Rc::clone(&store));
    click(cx, "new-chat");
    say(cx, &workspace, "a title, please").await;
    assert_eq!(names(cx, &workspace), listed(&[("No project", &[TITLE])]));
    assert_eq!(
        kept_names(&store),
        [(TITLE.to_owned(), Named::Agent, false)]
    );

    double_click(cx, "chat-1");
    cx.simulate_input("Mine");
    cx.simulate_keystrokes("enter");
    say(cx, &workspace, "another title").await;
    assert_eq!(names(cx, &workspace), listed(&[("No project", &["Mine"])]));
    assert_eq!(
        kept_names(&store),
        [("Mine".to_owned(), Named::Human, false)]
    );

    // Escape leaves the name as it was.
    right_click(cx, "chat-1");
    click(cx, "rename-chat");
    cx.simulate_input("Not mine");
    cx.simulate_keystrokes("escape");
    assert_eq!(names(cx, &workspace), listed(&[("No project", &["Mine"])]));
}

#[gpui::test]
async fn a_turn_that_ends_unseen_marks_its_chat_until_it_is_shown(cx: &mut TestAppContext) {
    let (_, workspace, cx) = open_window(cx, scratch("unread"));
    click(cx, "new-chat");
    let first = shown(cx, &workspace);
    cx.simulate_input("slowly");
    cx.simulate_keystrokes("enter");
    click(cx, "new-chat");
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
    let kept = scratch("archived-kept").join("chats.sqlite");
    let store = Rc::new(Store::open(&kept).unwrap());
    let (agents, workspace, cx) = open_window_keeping(cx, home.clone(), Rc::clone(&store));
    click(cx, "new-chat");
    say(cx, &workspace, "first").await;
    click(cx, "new-chat");
    say(cx, &workspace, "second").await;

    right_click(cx, "chat-2");
    click(cx, "archive-chat");
    assert_eq!(names(cx, &workspace), listed(&[("No project", &["first"])]));
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
    let again = cx.update(|_, cx| {
        let store = Rc::new(Store::open(&kept).unwrap());
        let opens = opens(&agents, &Asks::default(), Resumes::Resume);
        cx.new(|cx| Workspace::new(home.clone(), store, opens, cx))
    });
    assert_eq!(names(cx, &again), listed(&[("No project", &["first"])]));
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
        listed(&[("No project", &["first", "second"])])
    );
    assert_eq!(shown_name(cx, &workspace).as_deref(), Some("second"));
    assert!(!kept_names(&store)[1].2);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(kept.parent().unwrap());
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
        std::thread::sleep(Duration::from_secs(5));
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
    click(cx, "new-chat");
    let first = shown(cx, &workspace);
    cx.simulate_input("slow permission");
    cx.simulate_keystrokes("enter");
    click(cx, "new-chat");
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
    cx.update(|window, cx| {
        workspace.update(cx, |workspace, cx| workspace.reveal(first_key, window, cx));
    });
    assert_eq!(shown(cx, &workspace), first);
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
) -> Vec<Option<String>> {
    workspace.read_with(cx, |workspace, _| {
        workspace
            .chats(project)
            .map(|chat| chat.goal().map(str::to_owned))
            .collect()
    })
}

#[gpui::test]
async fn a_chat_with_a_goal_goes_on_until_its_command_passes(cx: &mut TestAppContext) {
    let home = scratch("goal");
    std::fs::create_dir_all(&home).unwrap();
    let store = Rc::new(Store::in_memory().unwrap());
    let (agents, workspace, cx) = open_window_keeping(cx, home.clone(), Rc::clone(&store));
    click(cx, "new-chat");
    let view = shown(cx, &workspace);
    let command = passing_the_second_time(&home);
    view.update(cx, |view, cx| view.set_goal(Some(command.clone()), cx));
    assert_eq!(goals(cx, &workspace, 0), [Some(command.clone())]);

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
        entries.last(),
        Some(&Entry::Note(format!("Goal met: `{command}` passed.")))
    );
    assert_eq!(entries[entries.len() - 3], Entry::Cox(prompts[1].clone()));
    // The goal, and the way to it, are kept.
    let kept = store.chats().unwrap();
    assert_eq!(kept[0].goal, Some(KeptGoal::Command(command.clone())));
    assert_eq!(store.entries(kept[0].id).unwrap(), entries);

    view.update(cx, |view, cx| view.set_goal(None, cx));
    assert_eq!(goals(cx, &workspace, 0), [None]);
    assert_eq!(store.chats().unwrap()[0].goal, None);
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
        open_window_keeping(cx, scratch("goal-again-home"), Rc::clone(&store));
    assert_eq!(goals(cx, &workspace, 1), [Some("true".to_owned())]);

    click(cx, "chat-1");
    let view = shown(cx, &workspace);
    assert_eq!(
        view.read_with(cx, |view, _| {
            let conversation = view.conversation();
            (
                conversation.state(),
                conversation.goal().map(Goal::command).map(str::to_owned),
            )
        }),
        (State::Closed, Some("true".to_owned()))
    );

    // Sent to, the chat goes on toward its goal.
    say(cx, &workspace, "hello").await;
    assert_eq!(agents.borrow()[0].prompts().len(), 1);
    assert_eq!(
        shown_entries(cx, &workspace).last(),
        Some(&Entry::Note("Goal met: `true` passed.".to_owned()))
    );
}

#[gpui::test]
async fn escape_stops_the_check_of_the_goal(cx: &mut TestAppContext) {
    let (_, workspace, cx) = open_window(cx, scratch("goal-stop"));
    click(cx, "new-chat");
    let view = shown(cx, &workspace);
    view.update(cx, |view, cx| view.set_goal(Some("sleep 3".to_owned()), cx));
    cx.simulate_input("hello");
    cx.simulate_keystrokes("enter");
    wait_until(cx, &view, |conversation| {
        conversation.state() == State::Checking
    })
    .await;

    cx.simulate_keystrokes("escape");

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
    let store = Rc::new(Store::in_memory().unwrap());
    let (agents, workspace, cx) = open_window_keeping(cx, scratch("goal-menu"), Rc::clone(&store));
    click(cx, "new-chat");
    assert!(cx.debug_bounds("goal-1").is_none());

    right_click(cx, "chat-1");
    assert!(cx.debug_bounds("clear-goal").is_none());
    click(cx, "set-goal");
    cx.simulate_input("true");
    cx.simulate_keystrokes("enter");
    assert_eq!(goals(cx, &workspace, 0), [Some("true".to_owned())]);
    assert_eq!(
        store.chats().unwrap()[0].goal,
        Some(KeptGoal::Command("true".to_owned()))
    );
    cx.run_until_parked();
    assert!(cx.debug_bounds("goal-1").is_some());
    // The input that sets the goal is closed, and the keyboard is back in
    // the chat's own.
    say(cx, &workspace, "hello").await;
    assert_eq!(agents.borrow()[0].prompts(), ["hello"]);
    assert_eq!(
        shown_entries(cx, &workspace).last(),
        Some(&Entry::Note("Goal met: `true` passed.".to_owned()))
    );

    // Escape leaves the goal as it was.
    right_click(cx, "chat-1");
    click(cx, "set-goal");
    cx.simulate_input("false");
    cx.simulate_keystrokes("escape");
    assert_eq!(goals(cx, &workspace, 0), [Some("true".to_owned())]);

    right_click(cx, "chat-1");
    click(cx, "clear-goal");
    assert_eq!(goals(cx, &workspace, 0), [None]);
    assert_eq!(store.chats().unwrap()[0].goal, None);
}

/// The key of the chat at `index` in no project, and that of the chat
/// whose agent opened it, if an agent did.
fn keys(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    index: usize,
) -> (u64, Option<u64>) {
    workspace.read_with(cx, |workspace, _| {
        let chat = workspace.chats(0).nth(index).unwrap();
        (chat.key(), chat.parent())
    })
}

#[gpui::test]
async fn an_agent_opens_a_chat_under_its_own_set_up_as_it_says(cx: &mut TestAppContext) {
    let home = scratch("child-home");
    let kept = scratch("child-kept").join("chats.sqlite");
    let store = Rc::new(Store::open(&kept).unwrap());
    let (agents, asks, workspace, cx) =
        open_window_asking(cx, home.clone(), Rc::clone(&store), Resumes::Resume);
    click(cx, "new-chat");
    say(cx, &workspace, "hello").await;
    click(cx, "new-chat");
    say(cx, &workspace, "second").await;
    let (parent, _) = keys(cx, &workspace, 0);

    // The first chat's agent opens a chat, which keeps to a context and
    // may edit nothing, on a line that asks to edit a file.
    let setup = Setup {
        context: Some("Change nothing.".to_owned()),
        policy: cox::agent::setup::Policy {
            allow: Vec::new(),
            reject: vec!["Edit *".to_owned()],
        },
        ..Setup::default()
    };
    let (answer, opened) = tokio::sync::oneshot::channel();
    asks.borrow()[0]
        .send(ChatRequest::OpenChat {
            text: "permission".to_owned(),
            setup: setup.clone(),
            goal: None,
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
        listed(&[("No project", &["hello", "permission", "second"])])
    );
    let (child, opened_by) = keys(cx, &workspace, 1);
    assert_eq!(opened_by, Some(parent));
    let opened = opened.await.unwrap().unwrap();
    assert_eq!((opened.number, opened.name.as_str()), (child, "Chat 3"));
    assert_eq!(
        shown_entries(cx, &workspace)[0],
        Entry::Human("second".to_owned())
    );
    let view = workspace.read_with(cx, |workspace, _| {
        workspace.chats(0).nth(1).unwrap().view().unwrap().clone()
    });
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
            "The chat that opened this one set it up with this context. Keep to it for the whole session.\n\nChange nothing.\n".to_owned(),
            "permission".to_owned()
        ]]
    );
    // Which chat opened it, and how, is kept: Cox started again lists it
    // under its parent, set up the same.
    let again = cx.update(|_, cx| {
        let store = Rc::new(Store::open(&kept).unwrap());
        let opens = opens(&agents, &Asks::default(), Resumes::Resume);
        cx.new(|cx| Workspace::new(home.clone(), store, opens, cx))
    });
    assert_eq!(
        names(cx, &again),
        listed(&[("No project", &["hello", "permission", "second"])])
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
    let _ = std::fs::remove_dir_all(kept.parent().unwrap());
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
            answer,
        })
        .unwrap();
    cx.condition(workspace, move |workspace, _| {
        workspace.chats(0).count() == count
    })
    .await;
}

#[gpui::test]
async fn an_agent_waits_for_the_chat_it_opened(cx: &mut TestAppContext) {
    let home = scratch("wait-home");
    let store = Rc::new(Store::in_memory().unwrap());
    let (_, asks, workspace, cx) = open_window_asking(cx, home.clone(), store, Resumes::Resume);
    click(cx, "new-chat");
    say(cx, &workspace, "hello").await;
    let ask = |request| asks.borrow()[0].send(request).unwrap();
    let view_of = |cx: &mut VisualTestContext, index| {
        workspace.read_with(cx, |workspace, _| {
            workspace
                .chats(0)
                .nth(index)
                .unwrap()
                .view()
                .unwrap()
                .clone()
        })
    };

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
