//! The window over a daemon of its own, for tests: the daemon's agents
//! faked in memory (see [`super::fake_agent`]), and the chats with no
//! stage, or a stand-in for one.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use cox::agent::Start;
use cox::agent::mcp::{ChatRequest, Ended, Ran, ToolRequest};
use cox::agent::setup::Setup;
use cox::agents::{Agents as Choices, Known};
use cox::chat::Stages;
use cox::chat::judge::Starts;
use cox::chat::view::{ChatView, OpenLink, StageView};
use cox::daemon::{Running, Spawned, Spawns};
use cox::ui::Panel;
use cox::workspace::Workspace;
use cox_acp::McpServer;
use cox_browser::relay::Relay;
use cox_core::store::Store;
use cox_core::tabs::KeptTabs;
use gpui::{
    App, AppContext as _, Context, Entity, Focusable, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, div,
};
use tokio::sync::mpsc::UnboundedSender;

use super::fake_agent::{FakeAgent, LogsIn, Resumes};

/// What the test and the daemon's thread share.
pub struct Shared<T>(Arc<Mutex<T>>);

impl<T> Shared<T> {
    pub fn borrow(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }

    pub fn borrow_mut(&self) -> MutexGuard<'_, T> {
        self.borrow()
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Default> Default for Shared<T> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

/// The agent of each chat started, and of each goal's judge, in the
/// order they started.
pub type Agents = Shared<Vec<Arc<FakeAgent>>>;

/// The way to the daemon of each chat's agent, in the order they
/// started, for a test to ask of it what the agent would with Cox's
/// tools.
pub type Asks = Shared<Vec<UnboundedSender<ChatRequest>>>;

/// The agent a chat starts with, and the other it can switch to.
pub const FIRST_AGENT: &str = "fake-first";
pub const SECOND_AGENT: &str = "Fake second";

/// The two agents a chat chooses between.
pub fn fake_choices() -> Choices {
    let second = Known {
        id: SECOND_AGENT.to_owned(),
        name: SECOND_AGENT.to_owned(),
        command: vec!["fake-second".to_owned()],
        listed: false,
        download: None,
    };
    Choices::new(vec![FIRST_AGENT.to_owned()], vec![second], Vec::new())
}

/// How the daemon's agents are faked.
#[derive(Clone)]
pub struct Fakes {
    pub agents: Agents,
    pub asks: Asks,
    /// The agents for the chats to talk to, in the order they start,
    /// ahead of those made as `logs_in` and `resumes` say.
    pub next: Shared<VecDeque<FakeAgent>>,
    /// How the agents of new chats ask to log in.
    pub logs_in: LogsIn,
    /// How the agents of closed chats started again go on with their
    /// sessions.
    pub resumes: Resumes,
    pub choices: Choices,
    /// What each chat's agent gets of MCP servers.
    pub mcp_servers: Vec<McpServer>,
    /// How each chat the human opens is set up, in place of none.
    pub setup: Option<Setup>,
    /// Whether each chat's agent gets the instructions of its project.
    pub instructions: bool,
    /// Whether the chats run commands, as their goals', in a shell, as a
    /// terminal would.
    pub terminal: bool,
    /// Whether the chats with no such terminal ask their stage, through
    /// the daemon and the window, to run commands, as Cox's chats do.
    pub staged_terminal: bool,
}

impl Default for Fakes {
    fn default() -> Self {
        Self {
            agents: Agents::default(),
            asks: Asks::default(),
            next: Shared::default(),
            logs_in: LogsIn::Never,
            resumes: Resumes::Resume,
            choices: fake_choices(),
            mcp_servers: Vec::new(),
            setup: None,
            instructions: true,
            terminal: true,
            staged_terminal: false,
        }
    }
}

/// Starts the daemon's agents as `Fakes` says.
struct FakeSpawns(Fakes);

impl Spawns for FakeSpawns {
    fn agents(&self) -> Choices {
        self.0.choices.clone()
    }

    fn chat(&self, cwd: PathBuf, start: Start, _: &str, _: Option<u16>) -> Spawned {
        let fakes = &self.0;
        let made = fakes.next.borrow_mut().pop_front();
        let agent = Arc::new(made.unwrap_or_else(|| match &start {
            Start::New { .. } => FakeAgent::logging_in(fakes.logs_in.clone()),
            Start::Again { .. } => FakeAgent::resuming(fakes.resumes),
        }));
        fakes.agents.borrow_mut().push(Arc::clone(&agent));
        let start = match start {
            Start::New {
                instructions,
                setup,
            } => Start::New {
                instructions: instructions.filter(|_| fakes.instructions),
                setup: setup.or_else(|| fakes.setup.clone()),
            },
            again @ Start::Again { .. } => again,
        };
        let mut spawned = cox::agent::spawn(
            move |_| Ok(agent.connect()),
            cwd,
            fakes.mcp_servers.clone(),
            start,
        );
        // The chat's commands run in a shell here, or else go through the
        // daemon to the window, as on a terminal of the stage.
        let stage = if fakes.terminal {
            spawned.tools = Some(fake_terminal());
            None
        } else if fakes.staged_terminal {
            let (tools, stage) = tokio::sync::mpsc::unbounded_channel();
            spawned.tools = Some(tools);
            Some(stage)
        } else {
            None
        };
        let (asks, chat_requests) = tokio::sync::mpsc::unbounded_channel();
        fakes.asks.borrow_mut().push(asks);
        spawned.chat_requests = Some(chat_requests);
        Spawned {
            agent: spawned,
            stage,
        }
    }

    /// What starts an agent with no chat, such as the judge of a goal,
    /// faked and added to the agents like the chats' own.
    fn starts(&self, _: &str) -> Starts {
        let agents = self.0.agents.clone();
        Rc::new(move |cwd, start| {
            let agent = Arc::new(FakeAgent::default());
            agents.borrow_mut().push(Arc::clone(&agent));
            cox::agent::spawn(move |_| Ok(agent.connect()), cwd, Vec::new(), start)
        })
    }
}

/// Runs the commands a chat asks its stage to run, as a terminal would,
/// in a shell, and answers how each went. Dropping the sender ends it.
pub fn fake_terminal() -> UnboundedSender<ToolRequest> {
    let (asks, mut requests) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Some(request) = requests.blocking_recv() {
            let ToolRequest::RunCommand {
                command,
                cwd,
                answer,
                ..
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

/// The links the human opened, in order.
pub type Opened = Rc<std::cell::RefCell<Vec<String>>>;

/// A stand-in for the stage, which needs CEF.
struct FakeStage;

impl Render for FakeStage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child("stage")
    }
}

/// The stages made, each by the folder of its chats and the tabs it
/// was made on, in order.
pub type Made = Rc<std::cell::RefCell<Vec<(PathBuf, Option<KeptTabs<u64>>)>>>;

/// Makes the chats' stages: none, or, when `opened` or `made` is given, a
/// stand-in for each, which notes in `opened` the links the human opens,
/// and in `made` the stages made. The commands asked of a chat with none
/// run in a shell.
#[derive(Default)]
pub struct FakeStages {
    pub opened: Option<Opened>,
    pub made: Option<Made>,
}

impl Stages for FakeStages {
    fn stage(
        &self,
        cwd: PathBuf,
        tabs: Option<KeptTabs<u64>>,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<(StageView, OpenLink)> {
        if self.opened.is_none() && self.made.is_none() {
            return None;
        }
        if let Some(made) = &self.made {
            made.borrow_mut().push((cwd, tabs));
        }
        let opened = self.opened.clone().unwrap_or_default();
        let view = StageView {
            view: cx.new(|_| FakeStage).into(),
            panel: cx.new(|_| Panel::default()),
        };
        let open_link: OpenLink =
            Rc::new(move |url, _, _| opened.borrow_mut().push(url.to_owned()));
        Some((view, open_link))
    }

    fn relay(&self) -> Option<Relay> {
        None
    }

    /// Runs the commands asked of a chat with no stage, as a terminal
    /// would, in a shell.
    fn unstaged(&self, request: ToolRequest) {
        let _ = fake_terminal().send(request);
    }
}

/// Where the daemon keeps the chats: in memory, or in the store at a
/// path, which the test opens too, to keep chats there first, or to read
/// what was kept.
pub enum Kept {
    Memory,
    At(PathBuf),
}

/// A store of its own at a path, for each test that keeps chats there.
pub fn kept_at(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("cox-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder.join("chats.sqlite")
}

/// The window's daemon, which stops with it.
pub struct Daemon(#[expect(dead_code, reason = "it is held to run")] Running);

/// Opens the window with no chat, over a daemon of its own, which keeps
/// the chats as `kept` says, starts their agents as `fakes` says, and
/// has Cox's own chats work in `home`. The chats' stages are made by
/// `stages`.
pub fn open_window(
    cx: &mut TestAppContext,
    home: PathBuf,
    kept: Kept,
    fakes: Fakes,
    stages: FakeStages,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    // The agents and the daemon run on threads of their own, so the test
    // waits for them in real time instead of only running GPUI's own
    // tasks.
    cx.executor().allow_parking();
    cx.update(|cx| {
        cox::ui::input::bind_keys(cx);
        cox::chat::view::bind_keys(cx);
        cox::settings::view::bind_keys(cx);
        cox::workspace::bind_keys(cx);
    });
    let ticker = Ticker::start(cx.executor());
    let (workspace, cx) =
        cx.add_window_view(|window, cx| over_daemon(home, kept, fakes, stages, window, cx));
    workspace.update(cx, |_, cx| cx.on_release(move |_, _| drop(ticker)).detach());
    cx.update(|window, cx| window.focus(&workspace.focus_handle(cx)));
    (workspace, cx)
}

/// Wakes the test's executor every few milliseconds while the window is
/// open. GPUI's test dispatcher can miss a wake from another thread, as
/// the daemon's, that comes as it goes to park, and would then park until
/// something else woke it; a task spawned now and then wakes it again.
struct Ticker(Arc<std::sync::atomic::AtomicBool>);

impl Ticker {
    fn start(executor: gpui::BackgroundExecutor) -> Self {
        let ticking = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let goes_on = Arc::clone(&ticking);
        std::thread::spawn(move || {
            while goes_on.load(std::sync::atomic::Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
                executor.spawn(async {}).detach();
            }
        });
        Self(ticking)
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// A workspace over a daemon of its own in the window `cx` has, which
/// it does not show, as the window of Cox started again, on what an
/// earlier daemon kept at the same path.
pub fn open_again(
    cx: &mut VisualTestContext,
    home: PathBuf,
    kept: Kept,
    fakes: Fakes,
) -> Entity<Workspace> {
    let stages = FakeStages::default();
    cx.update(|window, cx| cx.new(|cx| over_daemon(home, kept, fakes, stages, window, cx)))
}

/// A workspace over a daemon of its own, which stops as the workspace
/// goes.
fn over_daemon(
    home: PathBuf,
    kept: Kept,
    fakes: Fakes,
    stages: FakeStages,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Workspace {
    let choices = fakes.choices.clone();
    let (to_window, from_daemon) = tokio::sync::mpsc::unbounded_channel();
    let running = cox::daemon::start(
        home.clone(),
        false,
        move || {
            Rc::new(match kept {
                Kept::Memory => Store::in_memory().unwrap(),
                Kept::At(path) => Store::open(&path).unwrap(),
            })
        },
        move || Rc::new(FakeSpawns(fakes)),
        to_window,
    );
    let link = running.link.clone();
    let daemon = Daemon(running);
    cx.on_release(move |_, _| drop(daemon)).detach();
    Workspace::new(
        link,
        from_daemon,
        home,
        false,
        Rc::new(stages),
        Some(choices),
        window,
        cx,
    )
}

/// How long a test waits for what it waits for before it fails, saying
/// it never came. It only catches what hangs: a debug build lays out a
/// long conversation slowly, which can take seconds on a slow machine,
/// as CI's.
pub const GIVEN_UP: Duration = Duration::from_secs(30);

/// Waits until the daemon has done all the window told it, and the
/// window has heard all the daemon said of it.
pub async fn settle(cx: &mut VisualTestContext) {
    let workspace = cx
        .update(|window, _| window.root::<Workspace>())
        .flatten()
        .expect("the window shows a workspace");
    settle_on(cx, &workspace).await;
}

/// Waits until the daemon of `workspace` has done all it was told, and
/// `workspace` has heard all the daemon said of it, twice over, for
/// what the workspace asks on hearing it, and what the daemon answers:
/// as an agent that drives the stage, which the daemon starts once the
/// window has the chat's view, has the daemon tell the chat's
/// conversation and tabs, and gives the stage a relay.
pub async fn settle_on(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) {
    for _ in 0..4 {
        let settled = workspace.update(cx, |workspace, _| workspace.sync());
        let (timed_out, timeout) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(GIVEN_UP);
            let _ = timed_out.send(());
        });
        tokio::select! {
            _ = settled => {}
            _ = timeout => panic!("the daemon never settled"),
        }
        cx.run_until_parked();
    }
}

/// Settles `workspace`, as [`settle_on`], from a test that does not
/// await.
pub fn settled(cx: &mut VisualTestContext, workspace: &Entity<Workspace>) {
    let executor = cx.executor();
    executor.block_test(settle_on(cx, workspace));
}

/// Waits, settling again and again, until `done` says the test can go
/// on, as once an agent has started.
pub async fn settle_until(
    cx: &mut VisualTestContext,
    done: impl Fn(&mut VisualTestContext) -> bool,
) {
    for _ in 0..500 {
        settle(cx).await;
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("what the test waited for never came");
}

/// The view of the chat shown, once the daemon has settled.
pub async fn shown_view(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
) -> Entity<ChatView> {
    settle_on(cx, workspace).await;
    workspace.read_with(cx, |workspace, _| {
        let chat = workspace.shown().expect("a chat is shown");
        chat.view().expect("the chat shown has a view").clone()
    })
}
