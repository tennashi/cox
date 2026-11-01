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
//! else, and stops the command when the agent cancels the request.
//!
//! The tools themselves are in [`calls`]: what the agent learns of them,
//! and what each does when called, through what it asks of the chat: of
//! its stage, as a [`ToolRequest`], or of the window, as a
//! [`ChatRequest`], such as opening another chat.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use anyhow::Context as _;
use cox_core::goal::Goal;
use cox_core::tabs::Lines;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

use crate::agent::setup::Setup;
use crate::chat::trial::Trial;

mod calls;

/// The option that runs `cox` as the bridge, followed by the socket.
pub const OPTION: &str = "--mcp";

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
    /// Run `command` in a terminal on the stage, working in `cwd`, and
    /// answer how it went, or why it could not start, once it ends. The
    /// agent no longer waits for the answer once `answer` is closed: the
    /// command is to stop.
    RunCommand {
        command: String,
        cwd: PathBuf,
        answer: oneshot::Sender<Result<Ran, String>>,
    },
}

/// What the agent's tools ask of the window its chat is in: of the chat's
/// conversation, or past the chat itself.
#[derive(Debug)]
pub enum ChatRequest {
    /// Show the human `goal`, proposed for the chat, for them to set,
    /// change, or let be, with `why`, if the agent said; answer the goal
    /// the chat has now, if any.
    ProposeGoal {
        goal: Goal,
        why: Option<String>,
        answer: oneshot::Sender<Option<Goal>>,
    },
    /// Open a chat in the chat's project, under it in the list, whose
    /// agent is set up as `setup` says, works until `goal` is met if
    /// given, and gets `text` as its first line; answer how the chat goes
    /// by, or why it could not open.
    OpenChat {
        text: String,
        setup: Setup,
        goal: Option<Goal>,
        /// What the chat starts from when it is a trial, working in a
        /// folder of its own.
        trial: Option<Trial>,
        answer: oneshot::Sender<Result<OpenedChat, String>>,
    },
    /// Wait for the chat that goes by `chat` to end its turn, or to come
    /// to wait for the human, and answer how it stands then; or at once,
    /// when it is not at work; or why there is no such chat.
    WaitChat {
        chat: u64,
        answer: oneshot::Sender<Result<Waited, String>>,
    },
}

/// How a chat stands, as the agent that waited for it is told.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// The folder it works in, when it is a trial.
    pub folder: Option<PathBuf>,
}

/// How a command the agent ran went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    /// What its terminal shows, with what scrolled off.
    pub output: String,
    pub ended: Ended,
}

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
const INVALID_PARAMS: i64 = -32602;

/// How many servers this Cox started, which numbers their directories.
static STARTED: AtomicU64 = AtomicU64::new(0);

/// Starts the server of a chat working in `cwd`, an absolute path, which
/// the paths the agent gives are taken from, serving the tools named in
/// `only`, or all of them when `None`.
///
/// # Errors
///
/// When the socket cannot be made.
pub fn start(cwd: PathBuf, only: Option<Vec<String>>) -> anyhow::Result<ToolServer> {
    let directory = private_directory()?;
    let socket = directory.join("mcp");
    let listener = StdUnixListener::bind(&socket)
        .and_then(|listener| {
            listener.set_nonblocking(true)?;
            Ok(listener)
        })
        .inspect_err(|_| {
            let _ = fs::remove_dir_all(&directory);
        })
        .with_context(|| format!("cannot listen on {}", socket.display()))?;
    let (sent, requests) = unbounded_channel();
    let asks = sent.clone();
    let (chats, chat_requests) = unbounded_channel();
    thread::spawn(move || {
        let served = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| runtime.block_on(serve(listener, cwd, only, sent, chats)));
        if let Err(error) = served {
            log::error!("the agent's tools stopped: {error:#}");
        }
        if let Err(error) = fs::remove_dir_all(&directory) {
            log::warn!("{} is left behind: {error}", directory.display());
        }
    });
    Ok(ToolServer {
        socket,
        requests,
        asks,
        chat_requests,
    })
}

/// Makes a new directory for a socket that only this user can enter.
fn private_directory() -> anyhow::Result<PathBuf> {
    loop {
        let number = STARTED.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!("cox-{}-{number}", std::process::id()));
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

/// Answers the bridge's connections until the chat lets go of `requests`,
/// with the tools named in `only`, or all of them, asking the window
/// through `chats`.
async fn serve(
    listener: StdUnixListener,
    cwd: PathBuf,
    only: Option<Vec<String>>,
    requests: UnboundedSender<ToolRequest>,
    chats: UnboundedSender<ChatRequest>,
) -> anyhow::Result<()> {
    let listener = UnixListener::from_std(listener)?;
    let served = Served { cwd, only, chats };
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let served = served.clone();
                let requests = requests.clone();
                tokio::spawn(async move {
                    if let Err(error) = talk(stream, &served, &requests).await {
                        log::warn!("a connection of the agent's tools ended: {error:#}");
                    }
                });
            }
            () = requests.closed() => return Ok(()),
        }
    }
}

/// What one chat's server serves: the tools named in `only`, or all of
/// them, working in `cwd`, asking the window through `chats`.
#[derive(Clone)]
struct Served {
    cwd: PathBuf,
    only: Option<Vec<String>>,
    chats: UnboundedSender<ChatRequest>,
}

/// Answers what the agent says on one connection, until it hangs up.
async fn talk(
    stream: UnixStream,
    served: &Served,
    requests: &UnboundedSender<ToolRequest>,
) -> anyhow::Result<()> {
    let (heard, mut said) = stream.into_split();
    // Answers go out as they are ready, in whatever order that is.
    let (answers, mut ready) = unbounded_channel::<Value>();
    let sending = tokio::spawn(async move {
        while let Some(answer) = ready.recv().await {
            let mut line = answer.to_string();
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
        let (served, requests, answers) = (served.clone(), requests.clone(), answers.clone());
        let answer = tokio::spawn(async move {
            if let Some(answer) = answer(&message, &served, &requests).await {
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
async fn answer(
    message: &Value,
    served: &Served,
    requests: &UnboundedSender<ToolRequest>,
) -> Option<Value> {
    let id = message.get("id")?;
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let only = served.only.as_deref();
    let result = match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": calls::tools(only) })),
        "tools/call" => calls::call(params, &served.cwd, only, requests, &served.chats).await,
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
            chats,
        };
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(answer(message, &served, requests))
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
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 6);
        let called = ask("tools/call", &json!({ "name": "no_such_tool" }));
        let called = answer_now(&called, cwd, &sent).unwrap();
        assert_eq!(called["error"]["code"], INVALID_PARAMS);
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
