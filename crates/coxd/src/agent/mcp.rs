//! Cox's own tools for a chat's agent, as an MCP server that answers inside
//! Cox, where the tools reach what the chat holds, such as its stage.
//!
//! The agent starts its MCP servers itself, as programs that speak MCP on
//! their standard input and output. So the server Cox hands the agent is
//! Cox itself, run as `cox --mcp <socket>`: a [`bridge`] that only carries
//! what the agent says to the chat's [`ToolServer`], on a Unix socket, and the
//! answers back. Each chat has a socket of its own, in a directory that
//! only the human's user can enter, so that an agent reaches its own chat
//! and no other. The socket goes when the chat does.
//!
//! What goes over the socket is MCP as it goes over standard input and
//! output: JSON-RPC messages, one to a line. The server answers each
//! request as it is done, so a command that runs long holds up nothing
//! else, and stops the command when the agent cancels the request. The
//! server itself serves whatever [`Serves`] its tools: the socket that
//! answers for Cox as a whole, in `cox::inspect`, is one of these
//! servers too, and the same bridge reaches it.
//!
//! The tools themselves are in [`calls`]: what the agent learns of them,
//! and what each does when called, through what it asks of the chat: of
//! its stage, as a [`ToolRequest`], or of the window, as a
//! [`ChatRequest`], such as opening another chat.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::future::Future;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use anyhow::Context as _;
use cox_core::goal::Goal;
use cox_core::store::ChatId;
use cox_core::tabs::Lines;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

use crate::agent::setup::Setup;
use crate::chat::sketch::Sketch;

mod calls;

/// The option that runs `cox` as the bridge, followed by the socket.
pub const OPTION: &str = "--mcp";

/// The target under which each line the agent and the server send each
/// other over the socket is said, with `from` saying whose it is: `cox` or
/// `agent`. At `debug` it is said in [`brief`], with `bytes`, how long it
/// is; at `trace`, whole.
pub const WIRE: &str = "cox::agent::mcp::wire";

const STRING_KEPT_WHOLE: usize = 256;

const STRING_CUT_TO: usize = 200;

/// The target under which a tool asking something of the chat's stage is
/// said, at `debug`, as "asks the stage to " and [`ToolRequest::said`].
pub const ASKS: &str = "cox::agent::mcp::asks";

/// The target under which the stage carrying out what a tool asked is
/// said, at `debug`, as "the stage carries it out: " and
/// [`ToolRequest::said`].
pub const ASKED: &str = "cox::stage::asked";

/// What the agent's tools ask of its chat.
#[derive(Debug)]
pub enum ToolRequest {
    /// Show the human the Markdown file at `path`, an absolute path,
    /// which says `text`, on a tab of the stage.
    ShowMarkdown { path: PathBuf, text: String },
    /// Show the human the file at `path`, an absolute path, which says
    /// `text`, as it is written, at `lines` if any, on a tab of the stage.
    ShowFile {
        path: PathBuf,
        text: String,
        lines: Option<Lines>,
    },
    /// Show the human what changed in the files of the Git repository of
    /// `folder`, an absolute path, on a tab of the stage.
    ShowChanges { folder: PathBuf },
    /// Show the human a map of the project on a tab of the stage: the
    /// map named `map`, or its first unless named, open `at`, a tier and
    /// a box in it by their ids, or at its top when empty.
    ShowMap {
        map: Option<String>,
        at: Vec<String>,
    },
    /// Run `command` in a terminal on the stage, working in `cwd`, and
    /// answer how it went, or why it could not start, once it ends. The
    /// agent no longer waits for the answer once `answer` is closed: the
    /// command is to stop. While no window is there, coxd runs it on a
    /// terminal no one sees, unless it `asks_human`, as a login does,
    /// which then cannot run.
    RunCommand {
        command: String,
        cwd: PathBuf,
        answer: oneshot::Sender<Result<Ran, String>>,
        /// Whether the command asks the human what it needs, in its
        /// terminal, so that it runs only where they see it.
        asks_human: bool,
    },
    /// Do `act` with the windows of the Mac's other apps, on a tab of the
    /// stage that shows the window, and answer what the agent sees then.
    /// While no window is there, there is nothing to show them on.
    MacWindow {
        act: MacAct,
        answer: oneshot::Sender<Result<Seen, String>>,
    },
    /// Do `act` with the Android emulators, on a tab of the stage that
    /// shows the device, and answer what the agent sees then. While no
    /// window is there, there is nothing to show them on.
    Android {
        act: AndroidAct,
        answer: oneshot::Sender<Result<Seen, String>>,
    },
}

impl ToolRequest {
    /// What is asked, in a few words, as the log says it: "show the map
    /// at cox › stage", "run `cargo test` in /work".
    #[must_use]
    pub fn said(&self) -> String {
        match self {
            Self::ShowMarkdown { path, .. } => format!("show {} as Markdown", path.display()),
            Self::ShowFile { path, lines, .. } => match lines {
                Some(lines) => format!(
                    "show {} at lines {}–{}",
                    path.display(),
                    lines.first,
                    lines.last
                ),
                None => format!("show {}", path.display()),
            },
            Self::ShowChanges { folder } => format!("show what changed in {}", folder.display()),
            Self::ShowMap { map, at } => {
                let which = map
                    .as_ref()
                    .map_or_else(|| "the map".to_owned(), |map| format!("the map `{map}`"));
                if at.is_empty() {
                    format!("show {which} at its top")
                } else {
                    format!("show {which} at {}", at.join(" › "))
                }
            }
            Self::RunCommand { command, cwd, .. } => {
                format!("run `{command}` in {}", cwd.display())
            }
            Self::MacWindow { act, .. } => act.said(),
            Self::Android { act, .. } => act.said(),
        }
    }
}

/// What the agent's tools ask of the window its chat is in: of the chat's
/// conversation, or past the chat itself.
#[derive(Debug)]
pub enum ChatRequest {
    /// Show the human `shown`, a picture or a video kept from the path
    /// `from`, with `caption`, if the agent gave one, in the
    /// conversation; answer once it is there.
    ShowMedia {
        shown: crate::media::Shown,
        from: String,
        caption: Option<String>,
        answer: oneshot::Sender<()>,
    },
    /// Show the human `goal`, proposed for the chat, for them to set,
    /// change, or let be, with `why`, if the agent said; answer the goal
    /// the chat has now, if any.
    ProposeGoal {
        goal: Goal,
        why: Option<String>,
        answer: oneshot::Sender<Option<Goal>>,
    },
    /// Show the human `name`, proposed for the chat's project, for them to
    /// rename it to or let be, with `why`, if the agent said; answer the
    /// project's name now, or why it cannot be proposed.
    ProposeName {
        name: String,
        why: Option<String>,
        answer: oneshot::Sender<Result<String, String>>,
    },
    /// Answer what the chat of the chat's project kept as `chat` talked
    /// through, or why it cannot.
    ReadChat {
        chat: i64,
        answer: oneshot::Sender<Result<String, String>>,
    },
    /// Show the human `about`, what the chat's project is as proposed, for
    /// them to set or let be, with `why`, if the agent said; answer what
    /// the project is now, if it says, or why it cannot be proposed.
    ProposeAbout {
        about: String,
        why: Option<String>,
        answer: oneshot::Sender<Result<Option<String>, String>>,
    },
    /// Show the human the icon kept as `file`, proposed for the chat's
    /// project, made from the picture at `from`, for them to set or let
    /// be, with `why`, if the agent said; answer whether the project has
    /// an icon now, or why it cannot be proposed.
    ProposeIcon {
        file: String,
        from: String,
        why: Option<String>,
        answer: oneshot::Sender<Result<bool, String>>,
    },
    /// Show the human the project named `name`, proposed to be made of the
    /// chat, which is in none, for them to make, the chat moving into it,
    /// or to let be, with `why`, if the agent said, and the files of the
    /// sketch named `sketch`, or of the chat of that number, going into
    /// it, if given; answer where the project is to be, or why it cannot be
    /// proposed.
    ProposeProject {
        name: String,
        why: Option<String>,
        sketch: Option<String>,
        answer: oneshot::Sender<Result<PathBuf, String>>,
    },
    /// Open a chat in the chat's project, under it in the list, whose
    /// agent is set up as `setup` says, works until `goal` is met if
    /// given, and gets `text` as its first line; answer how the chat goes
    /// by, or why it could not open.
    OpenChat {
        text: String,
        setup: Setup,
        goal: Option<Goal>,
        /// What the chat starts from when it is a sketch, working in a
        /// folder of its own.
        sketch: Option<Sketch>,
        answer: oneshot::Sender<Result<OpenedChat, String>>,
    },
    /// Open a sketch beside the chat, under it in the list, that starts
    /// from `sketch`, to try out `what`, its agent set up as the chat's
    /// and told what the chat talked through so far; show it if `show`
    /// says to, as for the human who asked for it; answer how it goes by,
    /// or why it cannot open.
    StartSketch {
        sketch: Sketch,
        what: String,
        show: bool,
        answer: oneshot::Sender<Result<OpenedChat, String>>,
    },
    /// Wait for the chat that goes by `chat` to end its turn, or to come
    /// to wait for the human, and answer how it stands then; or at once,
    /// when it is not at work; or why there is no such chat.
    WaitChat {
        chat: u64,
        answer: oneshot::Sender<Result<Waited, String>>,
    },
    /// Answer where the chat is, for a page of the wiki its agent writes
    /// to say where it was used.
    Whereabouts {
        answer: oneshot::Sender<Whereabouts>,
    },
    /// Ask the agent of the chat, or of the chat kept as `chat`, such as
    /// a sketch that ended beside it, to keep what it learned in the wiki,
    /// as the human chose on the card that asked them: no tool asks it.
    SaveToWiki { chat: Option<ChatId> },
    /// The chat's agent wrote `page` of a wiki, which the triggers of
    /// the chat's folder run on: no answer.
    WikiWritten { page: String },
}

/// Where a chat is, as a page of the wiki its agent writes notes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Whereabouts {
    /// The chat's project, by its name.
    pub project: String,
    /// The folder of the chat's project, when it is in one, not in the
    /// home.
    pub folder: Option<PathBuf>,
    /// The chat, by its name.
    pub chat: String,
    /// The number the chat is kept by, if it is kept.
    pub kept: Option<String>,
}

/// How a chat stands, as the agent that waited for it is told.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Waited {
    /// What it asks the human, when it waits for them to answer.
    pub asking: Option<String>,
    /// What came last in its last turn: its agent's reply, or Cox's note
    /// of how the turn ended, if anything did.
    pub said: Option<String>,
}

/// A chat opened from a chat, as its agent is told of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedChat {
    /// What the chat goes by to the agent's tools, for this run of Cox.
    pub number: u64,
    /// What the human sees it as, in the list.
    pub name: String,
    /// The folder it works in, when it is a sketch.
    pub folder: Option<PathBuf>,
    /// What it was not given of the setup asked for it, as more than the
    /// chat that opened it may do.
    pub withheld: crate::agent::setup::Withheld,
}

/// How a command the agent ran went.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ran {
    /// What its terminal shows, with what scrolled off.
    pub output: String,
    pub ended: Ended,
}

/// What the agent does with the windows of the Mac's other apps. Where it
/// does it is in the pixels of the window's picture, as the agent last saw
/// it (see [`Seen`]), from its top left corner.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MacAct {
    /// Lists the windows.
    List,
    /// Looks at the window numbered `window`, as it is now.
    Look { window: u32 },
    /// Clicks there, with the right button if `right`, `count` times in a
    /// row.
    Click {
        window: u32,
        x: u32,
        y: u32,
        right: bool,
        count: u8,
    },
    /// Types `text` into the window, where its app has the keyboard.
    Type { window: u32, text: String },
    /// Presses `key`, as `Enter` or `cmd+a`.
    Key { window: u32, key: String },
    /// Turns the wheel there, by `dx` and `dy` pixels.
    Scroll {
        window: u32,
        x: u32,
        y: u32,
        dx: i32,
        dy: i32,
    },
}

impl MacAct {
    /// The window acted on, unless the act lists them.
    #[must_use]
    pub fn window(&self) -> Option<u32> {
        match self {
            Self::List => None,
            Self::Look { window }
            | Self::Click { window, .. }
            | Self::Type { window, .. }
            | Self::Key { window, .. }
            | Self::Scroll { window, .. } => Some(*window),
        }
    }

    /// What is done, as the log says it. What is typed may be a password:
    /// it says only how long it is.
    #[must_use]
    pub fn said(&self) -> String {
        match self {
            Self::List => "list the Mac's windows".to_owned(),
            Self::Look { window } => format!("look at Mac window {window}"),
            Self::Click {
                window,
                x,
                y,
                right,
                count,
            } => {
                let button = if *right { "right-click" } else { "click" };
                format!("{button} Mac window {window} at {x},{y} ×{count}")
            }
            Self::Type { window, text } => {
                format!(
                    "type {} characters in Mac window {window}",
                    text.chars().count()
                )
            }
            Self::Key { window, key } => format!("press {key} in Mac window {window}"),
            Self::Scroll {
                window,
                x,
                y,
                dx,
                dy,
            } => format!("scroll Mac window {window} at {x},{y} by {dx},{dy}"),
        }
    }
}

/// What the agent does with the Android emulators' virtual devices, by
/// their names. Where it does it is in the pixels of the device's
/// picture, as the agent last saw it (see [`Seen`]), from its top left
/// corner.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AndroidAct {
    /// Lists the devices.
    List,
    /// Looks at the device's display, starting its emulator if it does
    /// not run.
    Look { device: String },
    /// Taps there.
    Tap { device: String, x: u32, y: u32 },
    /// Swipes from there to `to_x` and `to_y`.
    Swipe {
        device: String,
        x: u32,
        y: u32,
        to_x: u32,
        to_y: u32,
    },
    /// Types `text`, where the device has the keyboard.
    Type { device: String, text: String },
    /// Presses `key`, as `Enter` or `GoBack`.
    Key { device: String, key: String },
}

impl AndroidAct {
    /// The device acted on, unless the act lists them.
    #[must_use]
    pub fn device(&self) -> Option<&str> {
        match self {
            Self::List => None,
            Self::Look { device }
            | Self::Tap { device, .. }
            | Self::Swipe { device, .. }
            | Self::Type { device, .. }
            | Self::Key { device, .. } => Some(device),
        }
    }

    /// What is done, as the log says it. What is typed may be a password:
    /// it says only how long it is.
    #[must_use]
    pub fn said(&self) -> String {
        match self {
            Self::List => "list the Android devices".to_owned(),
            Self::Look { device } => format!("look at Android device {device}"),
            Self::Tap { device, x, y } => format!("tap Android device {device} at {x},{y}"),
            Self::Swipe {
                device,
                x,
                y,
                to_x,
                to_y,
            } => format!("swipe Android device {device} from {x},{y} to {to_x},{to_y}"),
            Self::Type { device, text } => format!(
                "type {} characters on Android device {device}",
                text.chars().count()
            ),
            Self::Key { device, key } => format!("press {key} on Android device {device}"),
        }
    }
}

/// What the agent sees of the Mac's windows, or of the Android devices:
/// what is said of them, and a picture of the one acted on.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Seen {
    pub text: String,
    /// The picture, as PNG in base64.
    pub png: Option<String>,
}

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Ended {
    /// It exited with this code.
    Exited(i32),
    /// This signal ended it.
    Signal(i32),
    /// It ended, but how could not be learned.
    Unknown,
    /// The human closed its terminal while it ran.
    Closed,
}

impl fmt::Display for Ended {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited(code) => write!(f, "exit code {code}"),
            Self::Signal(signal) => write!(f, "ended by signal {signal}"),
            Self::Unknown => f.write_str("ended, with no exit code"),
            Self::Closed => f.write_str("stopped: the human closed its terminal"),
        }
    }
}

/// One chat's server, as the chat holds it.
pub struct ToolServer {
    /// Where the bridge reaches the server.
    pub socket: PathBuf,
    /// What the agent's tools ask of the chat's stage. Dropping it stops
    /// the server, and removes the socket.
    pub requests: UnboundedReceiver<ToolRequest>,
    /// Where they ask it, for the chat to ask the same of its stage.
    pub asks: UnboundedSender<ToolRequest>,
    /// What the agent's tools ask of the window.
    pub chat_requests: UnboundedReceiver<ChatRequest>,
}

/// The versions of MCP the server speaks, newest first. Its tools work
/// the same in each.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// JSON-RPC's codes for what went wrong with a request.
const PARSE_ERROR: i64 = -32700;
const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

/// What a request's tool answers: MCP's result of `tools/call`, or the
/// JSON-RPC error, with its code, that the request gets instead.
pub type Answered = Result<Value, (i64, String)>;

/// The tools a server serves: what a client learns of them, and what each
/// does when called. A server answers many calls at once, so a call that
/// waits holds up no other.
pub trait Serves: Send + Sync + 'static {
    /// The tools, as MCP lists them.
    fn tools(&self) -> Value;
    /// Calls the tool `params`, those of `tools/call`, name, with its
    /// arguments.
    fn call<'a>(&'a self, params: &'a Value)
    -> Pin<Box<dyn Future<Output = Answered> + Send + 'a>>;
}

/// The result of a call that answers with `text`, as an error when
/// `is_error`.
#[must_use]
pub fn answered(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

/// The result of a call that answers with `text`, and the picture `png`,
/// PNG in base64, if there is one.
pub(crate) fn answered_with_picture(text: &str, png: Option<&str>) -> Value {
    let mut content = vec![json!({ "type": "text", "text": text })];
    if let Some(png) = png {
        content.push(json!({ "type": "image", "data": png, "mimeType": "image/png" }));
    }
    json!({ "content": content, "isError": false })
}

/// How many servers this Cox started, which numbers their directories.
static STARTED: AtomicU64 = AtomicU64::new(0);

/// Starts the server of a chat working in `cwd`, an absolute path, which
/// the paths the agent gives are taken from, serving the tools named in
/// `only`, or all of them when `None`, but those of a project when the
/// chat is in none, and the one that makes a project of a chat when it is
/// in one, as `in_project` says.
///
/// # Errors
///
/// When the socket cannot be made.
pub fn start(
    cwd: PathBuf,
    only: Option<&[String]>,
    in_project: bool,
) -> anyhow::Result<ToolServer> {
    let only = Some(calls::for_chat(only, in_project));
    let directory = private_directory()?;
    let socket = directory.join("mcp");
    let listener = listen(&socket).inspect_err(|_| {
        let _ = fs::remove_dir_all(&directory);
    })?;
    let (sent, requests) = unbounded_channel();
    let asks = sent.clone();
    let (chats, chat_requests) = unbounded_channel();
    let served = Arc::new(Served {
        cwd,
        only,
        requests: sent,
        chats,
    });
    // The server stops when the chat lets go of the requests.
    let until = served.requests.clone();
    // What the server says is said in the span it starts in, its chat's:
    // its runtime runs on this one thread, which keeps what lasts where
    // this one was told to.
    let span = tracing::Span::current();
    let data = crate::storage::kept_on_this_thread();
    let trash = crate::trash::kept_on_this_thread();
    thread::spawn(move || {
        crate::storage::keep_on_this_thread(data);
        crate::trash::keep_on_this_thread(trash);
        let _entered = span.enter();
        if let Err(error) = serve(listener, served, until.closed()) {
            tracing::error!("the agent's tools stopped: {error:#}");
        }
        if let Err(error) = fs::remove_dir_all(&directory) {
            tracing::warn!("{} is left behind: {error}", directory.display());
        }
    });
    Ok(ToolServer {
        socket,
        requests,
        asks,
        chat_requests,
    })
}

/// Makes a new directory for a socket that only this user can enter, in
/// the folder of this run (see [`crate::scratch`]).
fn private_directory() -> anyhow::Result<PathBuf> {
    let scratch = crate::scratch::folder().context("cannot make the folder of this run")?;
    loop {
        let number = STARTED.fetch_add(1, Ordering::Relaxed);
        let directory = scratch.join(number.to_string());
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => return Ok(directory),
            // Left by an earlier Cox that had the same process id.
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| format!("cannot make {}", directory.display()));
            }
        }
    }
}

/// Listens on `socket`, a new path, for a server.
///
/// # Errors
///
/// When the socket cannot be made.
pub fn listen(socket: &Path) -> anyhow::Result<StdUnixListener> {
    StdUnixListener::bind(socket)
        .and_then(|listener| {
            listener.set_nonblocking(true)?;
            Ok(listener)
        })
        .with_context(|| format!("cannot listen on {}", socket.display()))
}

/// Serves `served` to the connections of `listener`, on this thread,
/// until `until` is done.
///
/// # Errors
///
/// When the runtime cannot start, or the listener fails.
pub fn serve(
    listener: StdUnixListener,
    served: Arc<dyn Serves>,
    until: impl Future<Output = ()>,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = UnixListener::from_std(listener)?;
        let mut until = pin!(until);
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (stream, _) = accepted?;
                    let served = served.clone();
                    tokio::spawn(async move {
                        if let Err(error) = talk(stream, served).await {
                            tracing::warn!("a connection of the tools ended: {error:#}");
                        }
                    });
                }
                () = &mut until => return Ok(()),
            }
        }
    })
}

/// What one chat's server serves: the tools named in `only`, or all of
/// them, working in `cwd`, asking the chat's stage through `requests`,
/// and the window through `chats`.
struct Served {
    cwd: PathBuf,
    only: Option<Vec<String>>,
    requests: UnboundedSender<ToolRequest>,
    chats: UnboundedSender<ChatRequest>,
}

impl Serves for Served {
    fn tools(&self) -> Value {
        calls::tools(self.only.as_deref())
    }

    fn call<'a>(
        &'a self,
        params: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Answered> + Send + 'a>> {
        Box::pin(calls::call(
            params,
            &self.cwd,
            self.only.as_deref(),
            &self.requests,
            &self.chats,
        ))
    }
}

/// Answers what the agent says on one connection, until it hangs up.
async fn talk(stream: UnixStream, served: Arc<dyn Serves>) -> anyhow::Result<()> {
    let (heard, mut said) = stream.into_split();
    // Answers go out as they are ready, in whatever order that is.
    let (answers, mut ready) = unbounded_channel::<Value>();
    let sending = tokio::spawn(async move {
        while let Some(answer) = ready.recv().await {
            let mut line = answer.to_string();
            tracing::debug!(target: WIRE, from = "cox", bytes = line.len(), "{}", in_brief(&answer));
            tracing::trace!(target: WIRE, from = "cox", "{line}");
            line.push('\n');
            said.write_all(line.as_bytes()).await?;
        }
        anyhow::Ok(())
    });
    // The requests not yet answered, by their ids, to cancel.
    let mut answering: HashMap<String, AbortHandle> = HashMap::new();
    let mut lines = BufReader::new(heard).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        tracing::debug!(target: WIRE, from = "agent", bytes = line.len(), "{}", brief(&line));
        tracing::trace!(target: WIRE, from = "agent", "{line}");
        let message = match serde_json::from_str::<Value>(&line) {
            Ok(message) => message,
            Err(error) => {
                let _ = answers.send(failure(&Value::Null, PARSE_ERROR, &error.to_string()));
                continue;
            }
        };
        if let Some(id) = cancelled(&message) {
            // Dropping the answer's future lets go of what it waits on.
            if let Some(answer) = answering.remove(&id.to_string()) {
                answer.abort();
            }
            continue;
        }
        let id = message.get("id").map(Value::to_string);
        let (served, answers) = (served.clone(), answers.clone());
        let answer = tokio::spawn(async move {
            if let Some(answer) = answer(&message, served.as_ref()).await {
                let _ = answers.send(answer);
            }
        });
        if let Some(id) = id {
            answering.retain(|_, answer| !answer.is_finished());
            answering.insert(id, answer.abort_handle());
        }
    }
    // What is still being answered goes unanswered: the agent hung up.
    for answer in answering.into_values() {
        answer.abort();
    }
    drop(answers);
    sending.await?
}

/// `line`, a message over the socket, in brief: each string value in it
/// longer than [`STRING_KEPT_WHOLE`] characters cut to its first
/// [`STRING_CUT_TO`] and how many bytes it had, so that a page written, a command's output or a
/// screenshot is not said whole. A line that is not JSON is cut as one
/// string.
#[must_use]
pub fn brief(line: &str) -> String {
    match serde_json::from_str::<Value>(line) {
        Ok(message) => in_brief(&message).to_string(),
        Err(_) => cut(line),
    }
}

/// `message` with its strings cut as [`brief`] cuts them. Cutting it
/// again changes nothing.
#[must_use]
pub fn in_brief(message: &Value) -> Value {
    match message {
        Value::String(text) => Value::String(cut(text)),
        Value::Array(items) => items.iter().map(in_brief).collect(),
        Value::Object(fields) => fields
            .iter()
            .map(|(name, value)| (name.clone(), in_brief(value)))
            .collect(),
        other => other.clone(),
    }
}

fn cut(text: &str) -> String {
    if text.chars().count() <= STRING_KEPT_WHOLE {
        return text.to_owned();
    }
    let kept: String = text.chars().take(STRING_CUT_TO).collect();
    format!("{kept}… ({} bytes)", text.len())
}

/// The id of the request `message` cancels, if it is the notice that does.
fn cancelled(message: &Value) -> Option<&Value> {
    if message.get("method")?.as_str()? != "notifications/cancelled" {
        return None;
    }
    message.get("params")?.get("requestId")
}

/// The answer to `message`, or `None` when it needs none: a
/// notification, or an answer to a request, which the server makes none
/// of.
async fn answer(message: &Value, served: &dyn Serves) -> Option<Value> {
    let id = message.get("id")?;
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let result = match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": served.tools() })),
        "tools/call" => served.call(params).await,
        _ => Err((METHOD_NOT_FOUND, format!("no method {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, error)) => failure(id, code, &error),
    })
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The server's half of starting a session: the version the client asks
/// for if the server speaks it, or the newest the server does.
fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|asked| PROTOCOL_VERSIONS.contains(asked))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "cox", "version": env!("CARGO_PKG_VERSION") },
    })
}

/// Runs `cox` as the bridge between the agent, on standard input and
/// output, and the chat's server on `socket`, until either hangs up.
///
/// # Errors
///
/// When the server cannot be reached, or the bridge cannot write to the
/// agent.
pub fn bridge(socket: &Path) -> anyhow::Result<()> {
    let server = StdUnixStream::connect(socket)
        .with_context(|| format!("cannot reach Cox at {}", socket.display()))?;
    let mut to_server = server.try_clone()?;
    thread::spawn(move || {
        let _ = pass(&mut io::stdin().lock(), &mut to_server);
        // The server hears that the agent is done, and hangs up too.
        let _ = to_server.shutdown(Shutdown::Write);
    });
    pass(&mut &server, &mut io::stdout().lock())
}

/// Passes on what `from` says to `to` as it comes, until `from` ends.
///
/// Not with `io::copy`, which on Linux has the kernel splice the agent's
/// pipe into the socket, where a line then waited for more to come.
fn pass(from: &mut impl Read, to: &mut impl Write) -> anyhow::Result<()> {
    let mut buffer = [0; 8192];
    loop {
        match from.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => {
                to.write_all(&buffer[..read])?;
                to.flush()?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
}

/// The socket to bridge to, if `args`, those after the program's name, ask
/// `cox` to run as the bridge.
#[must_use]
pub fn bridge_to(args: &[String]) -> Option<PathBuf> {
    match args {
        [option, socket] if option == OPTION => Some(PathBuf::from(socket)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(method: &str, params: &Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params })
    }

    /// The answer to `message`, for a request answered right away.
    fn answer_now(
        message: &Value,
        cwd: &Path,
        requests: &UnboundedSender<ToolRequest>,
    ) -> Option<Value> {
        let (chats, _chat_requests) = unbounded_channel();
        let served = Served {
            cwd: cwd.to_owned(),
            only: None,
            requests: requests.clone(),
            chats,
        };
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(answer(message, &served))
    }

    #[test]
    fn speaks_the_version_asked_for_or_its_newest() {
        let (sent, _requests) = unbounded_channel();
        let cwd = Path::new("/");
        let speaks = |asked: &str| {
            answer_now(
                &ask("initialize", &json!({ "protocolVersion": asked })),
                cwd,
                &sent,
            )
            .unwrap()["result"]["protocolVersion"]
                .clone()
        };
        assert_eq!(speaks("2025-03-26"), "2025-03-26");
        assert_eq!(speaks("2099-01-01"), PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn answers_no_notification_and_no_method_it_lacks() {
        let (sent, _requests) = unbounded_channel();
        let cwd = Path::new("/");
        let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert_eq!(answer_now(&initialized, cwd, &sent), None);
        let missing = answer_now(&ask("resources/list", &json!({})), cwd, &sent).unwrap();
        assert_eq!(missing["id"], 7);
        assert_eq!(missing["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn lists_and_calls_the_tools() {
        let (sent, _requests) = unbounded_channel();
        let cwd = Path::new("/");
        let listed = answer_now(&ask("tools/list", &json!({})), cwd, &sent).unwrap();
        // Cox's own, the Android emulator's, and the Mac's windows' on macOS.
        let tools = 21 + 6 + if cfg!(target_os = "macos") { 6 } else { 0 };
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), tools);
        let called = ask("tools/call", &json!({ "name": "no_such_tool" }));
        let called = answer_now(&called, cwd, &sent).unwrap();
        assert_eq!(called["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn a_message_in_brief_keeps_its_short_strings_and_cuts_its_long_ones() {
        let page = "a line of the page\n".repeat(1000);
        let message = json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "wiki_write", "arguments": { "path": "notes.md", "text": page } } });

        let said: Value = serde_json::from_str(&brief(&message.to_string())).unwrap();

        assert_eq!(said["method"], "tools/call");
        assert_eq!(said["params"]["arguments"]["path"], "notes.md");
        let text = said["params"]["arguments"]["text"].as_str().unwrap();
        assert!(text.starts_with("a line of the page\n"), "{text}");
        assert!(text.ends_with("… (19000 bytes)"), "{text}");
        assert_eq!(in_brief(&said), said);
        assert_eq!(brief(&"x".repeat(1000)).chars().count(), STRING_CUT_TO + 14);
    }

    #[test]
    fn runs_as_the_bridge_only_when_asked_to() {
        let args = |words: &[&str]| {
            words
                .iter()
                .map(|&word| word.to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            bridge_to(&args(&["--mcp", "/tmp/cox-1-0/mcp"])),
            Some(PathBuf::from("/tmp/cox-1-0/mcp"))
        );
        assert_eq!(bridge_to(&args(&["claude-code-acp"])), None);
        assert_eq!(bridge_to(&args(&["--mcp"])), None);
    }
}
