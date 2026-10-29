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
use cox_core::tabs::Lines;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;

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
    /// What the agent's tools ask. Dropping it stops the server, and
    /// removes the socket.
    pub requests: UnboundedReceiver<ToolRequest>,
}

/// The versions of MCP the server speaks, newest first. Its tools work
/// the same in each.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const SHOW_MARKDOWN: &str = "show_markdown";
const SHOW_FILE: &str = "show_file";
const RUN_COMMAND: &str = "run_command";

/// The most of a command's output the agent gets: the end of it, where a
/// build or a test run says how it went.
const OUTPUT_LENGTH: usize = 30_000;

/// JSON-RPC's codes for what went wrong with a request.
const PARSE_ERROR: i64 = -32700;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// How many servers this Cox started, which numbers their directories.
static STARTED: AtomicU64 = AtomicU64::new(0);

/// Starts the server of a chat working in `cwd`, an absolute path, which
/// the paths the agent gives are taken from.
///
/// # Errors
///
/// When the socket cannot be made.
pub fn start(cwd: PathBuf) -> anyhow::Result<ToolServer> {
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
    thread::spawn(move || {
        let served = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| runtime.block_on(serve(listener, cwd, sent)));
        if let Err(error) = served {
            log::error!("the agent's tools stopped: {error:#}");
        }
        if let Err(error) = fs::remove_dir_all(&directory) {
            log::warn!("{} is left behind: {error}", directory.display());
        }
    });
    Ok(ToolServer { socket, requests })
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

/// Answers the bridge's connections until the chat lets go of `requests`.
async fn serve(
    listener: StdUnixListener,
    cwd: PathBuf,
    requests: UnboundedSender<ToolRequest>,
) -> anyhow::Result<()> {
    let listener = UnixListener::from_std(listener)?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let cwd = cwd.clone();
                let requests = requests.clone();
                tokio::spawn(async move {
                    if let Err(error) = talk(stream, &cwd, &requests).await {
                        log::warn!("a connection of the agent's tools ended: {error:#}");
                    }
                });
            }
            () = requests.closed() => return Ok(()),
        }
    }
}

/// Answers what the agent says on one connection, until it hangs up.
async fn talk(
    stream: UnixStream,
    cwd: &Path,
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
        let (cwd, requests, answers) = (cwd.to_owned(), requests.clone(), answers.clone());
        let answer = tokio::spawn(async move {
            if let Some(answer) = answer(&message, &cwd, &requests).await {
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
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Option<Value> {
    let id = message.get("id")?;
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let result = match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call(params, cwd, requests).await,
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

/// The tools, as the agent learns of them.
fn tools() -> Value {
    json!([
    {
        "name": SHOW_MARKDOWN,
        "description": "Shows the human a Markdown file, drawn as Markdown, on a tab of the \
            stage beside the conversation, where they read it while you talk. Use it for \
            what the human should read, such as a plan or notes you wrote. The tab follows \
            the file as it changes, so there is no need to show a file again after you \
            change it. A file shown already comes back to the front.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file, absolute or from the working directory.",
                },
            },
            "required": ["path"],
        },
    },
    {
        "name": SHOW_FILE,
        "description": "Shows the human a file as it is written, such as code, on a tab of \
            the stage beside the conversation, at the lines you name, which the tab marks \
            and scrolls to. Use it to point the human at the code you talk about, rather \
            than quoting it. The tab follows the file as it changes. A file shown already \
            comes back to the front, at the lines named now. For a plan or notes the human \
            should read as Markdown, use show_markdown instead.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file, absolute or from the working directory.",
                },
                "first_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The first line to show, counted from 1.",
                },
                "last_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The last line to show, counted from 1: the first line \
                        unless given.",
                },
            },
            "required": ["path"],
        },
    },
    {
        "name": RUN_COMMAND,
        "description": "Runs a shell command in a terminal on the stage beside the \
            conversation, where the human watches it run, and can type into it. Use it for \
            the commands the human should see, such as builds and tests. It runs in the \
            working directory with the human's shell (`$SHELL -c`), afresh each time, so a \
            `cd` does not carry over to the next. It waits until the command ends, and \
            answers what the terminal shows, cut to the end when long, and the exit code. \
            A command that waits for input waits for the human; one that never ends, such \
            as a server, holds you until the human stops it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command, as the shell reads it.",
                },
            },
            "required": ["command"],
        },
    },
    ])
}

/// Calls the tool `params` names, and answers what it did, or the error
/// the agent made in calling it.
async fn call(
    params: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let done = match name {
        SHOW_MARKDOWN => show_markdown(arguments, cwd, requests),
        SHOW_FILE => show_file(arguments, cwd, requests),
        RUN_COMMAND => run_command(arguments, cwd, requests).await,
        _ => return Err((INVALID_PARAMS, format!("no tool {name}"))),
    };
    let (text, is_error) = match done {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    }))
}

/// Reads the file `arguments` names, and asks the chat to show it.
fn show_markdown(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or("no path: give the file to show as `path`")?;
    let path = cwd.join(path);
    let read = fs::canonicalize(&path).and_then(|path| Ok((fs::read_to_string(&path)?, path)));
    let (text, path) = read.map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let shown = format!("{} is on the stage, for the human to read.", path.display());
    requests
        .send(ToolRequest::ShowMarkdown { path, text })
        .map_err(|_| "the chat has closed".to_owned())?;
    Ok(shown)
}

/// Reads the file `arguments` names, and asks the chat to show it at the
/// lines they name.
fn show_file(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or("no path: give the file to show as `path`")?;
    let line = |name: &str| match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(line) => line
            .as_u64()
            .and_then(|line| usize::try_from(line).ok())
            .filter(|&line| line > 0)
            .map(Some)
            .ok_or(format!("{name} is to be a line number, from 1")),
    };
    let lines = match (line("first_line")?, line("last_line")?) {
        (Some(first), last) => Some(Lines {
            first,
            last: last.unwrap_or(first),
        }),
        (None, Some(_)) => return Err("last_line needs a first_line".to_owned()),
        (None, None) => None,
    };
    let path = cwd.join(path);
    let read = fs::canonicalize(&path).and_then(|path| Ok((fs::read_to_string(&path)?, path)));
    let (text, path) = read.map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let count = text.split('\n').count();
    let lines = match lines {
        Some(lines) if lines.last < lines.first => {
            return Err("last_line comes before first_line".to_owned());
        }
        Some(lines) if lines.first > count => {
            return Err(format!("{} has {count} lines", path.display()));
        }
        Some(lines) => Some(Lines {
            last: lines.last.min(count),
            ..lines
        }),
        None => None,
    };
    let shown = match lines {
        Some(lines) => format!(
            "{} is on the stage at lines {}-{}, for the human to read.",
            path.display(),
            lines.first,
            lines.last,
        ),
        None => format!("{} is on the stage, for the human to read.", path.display()),
    };
    requests
        .send(ToolRequest::ShowFile { path, text, lines })
        .map_err(|_| "the chat has closed".to_owned())?;
    Ok(shown)
}

/// Asks the chat to run the command `arguments` holds, and waits for it to
/// end: answers what it printed and how it ended, an error unless it
/// exited with 0.
async fn run_command(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let command = arguments
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or("no command: give the command to run as `command`")?;
    let (answer, ran) = oneshot::channel();
    requests
        .send(ToolRequest::RunCommand {
            command: command.to_owned(),
            cwd: cwd.to_owned(),
            answer,
        })
        .map_err(|_| "the chat has closed".to_owned())?;
    let ran = ran
        .await
        .map_err(|_| "the chat closed while the command ran".to_owned())??;
    let told = told(&ran);
    if ran.ended == Ended::Exited(0) {
        Ok(told)
    } else {
        Err(told)
    }
}

/// What the agent is told of a command that ran: its output, or the end
/// of it, and how it ended.
fn told(ran: &Ran) -> String {
    let output = ran.output.trim_end();
    let output = match output.char_indices().rev().nth(OUTPUT_LENGTH - 1) {
        // Cut at a line, past the cut.
        Some((cut, _)) => {
            let kept = &output[cut..];
            let kept = kept.split_once('\n').map_or(kept, |(_, rest)| rest);
            let cut = output[..output.len() - kept.len()].lines().count();
            format!("[the first {cut} lines are cut]\n{kept}")
        }
        None => output.to_owned(),
    };
    if output.is_empty() {
        format!("[no output, {}]", ran.ended)
    } else {
        format!("{output}\n\n[{}]", ran.ended)
    }
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
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(answer(message, cwd, requests))
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("cox-mcp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::canonicalize(directory).unwrap()
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
    fn lists_the_tools() {
        let (sent, _requests) = unbounded_channel();
        let listed = answer_now(&ask("tools/list", &json!({})), Path::new("/"), &sent).unwrap();
        assert_eq!(listed["result"]["tools"][0]["name"], SHOW_MARKDOWN);
        assert_eq!(listed["result"]["tools"][1]["name"], SHOW_FILE);
        assert_eq!(listed["result"]["tools"][2]["name"], RUN_COMMAND);
    }

    #[test]
    fn shows_a_file_from_the_working_directory() {
        let cwd = scratch("show");
        fs::write(cwd.join("plan.md"), "# Plan\n").unwrap();
        let (sent, mut requests) = unbounded_channel();

        let shown = answer_now(
            &ask(
                "tools/call",
                &json!({ "name": SHOW_MARKDOWN, "arguments": { "path": "plan.md" } }),
            ),
            &cwd,
            &sent,
        )
        .unwrap();

        assert_eq!(shown["result"]["isError"], false);
        match requests.try_recv().unwrap() {
            ToolRequest::ShowMarkdown { path, text } => {
                assert_eq!(path, cwd.join("plan.md"));
                assert_eq!(text, "# Plan\n");
            }
            request => panic!("asked {request:?}"),
        }
        fs::remove_dir_all(cwd).unwrap();
    }

    /// Calls `show_file` in `cwd` with `arguments`, and answers what it
    /// answers, with what the chat is asked.
    fn show_file(cwd: &Path, arguments: &Value) -> (Value, Option<ToolRequest>) {
        let (sent, mut requests) = unbounded_channel();
        let shown = answer_now(
            &ask(
                "tools/call",
                &json!({ "name": SHOW_FILE, "arguments": arguments }),
            ),
            cwd,
            &sent,
        )
        .unwrap();
        (shown["result"].clone(), requests.try_recv().ok())
    }

    #[test]
    fn shows_a_file_at_the_lines_named() {
        let cwd = scratch("show-file");
        fs::write(cwd.join("main.rs"), "fn main() {\n}\n").unwrap();

        let (shown, asked) = show_file(
            &cwd,
            &json!({ "path": "main.rs", "first_line": 2, "last_line": 9 }),
        );

        assert_eq!(shown["isError"], false);
        assert_eq!(
            shown["content"][0]["text"],
            format!(
                "{} is on the stage at lines 2-3, for the human to read.",
                cwd.join("main.rs").display()
            )
        );
        match asked {
            Some(ToolRequest::ShowFile { path, text, lines }) => {
                assert_eq!(path, cwd.join("main.rs"));
                assert_eq!(text, "fn main() {\n}\n");
                assert_eq!(lines, Some(Lines { first: 2, last: 3 }));
            }
            request => panic!("asked {request:?}"),
        }

        let (_, asked) = show_file(&cwd, &json!({ "path": "main.rs", "first_line": 1 }));
        assert!(
            matches!(
                asked,
                Some(ToolRequest::ShowFile {
                    lines: Some(Lines { first: 1, last: 1 }),
                    ..
                })
            ),
            "{asked:?}"
        );
        fs::remove_dir_all(cwd).unwrap();
    }

    #[test]
    fn tells_the_agent_of_lines_that_are_not_there() {
        let cwd = scratch("show-file-lines");
        fs::write(cwd.join("main.rs"), "fn main() {}\n").unwrap();
        let fails = |arguments: Value| {
            let (shown, asked) = show_file(&cwd, &arguments);
            assert_eq!(shown["isError"], true);
            assert!(asked.is_none());
            shown["content"][0]["text"].as_str().unwrap().to_owned()
        };

        assert!(fails(json!({ "path": "main.rs", "first_line": 5 })).ends_with("has 2 lines"));
        assert_eq!(
            fails(json!({ "path": "main.rs", "first_line": 2, "last_line": 1 })),
            "last_line comes before first_line"
        );
        assert_eq!(
            fails(json!({ "path": "main.rs", "first_line": 0 })),
            "first_line is to be a line number, from 1"
        );
        assert!(fails(json!({ "path": "gone.rs" })).starts_with("cannot read"));
        fs::remove_dir_all(cwd).unwrap();
    }

    #[test]
    fn tells_the_agent_of_a_file_it_cannot_read() {
        let cwd = scratch("missing");
        let (sent, mut requests) = unbounded_channel();

        let shown = answer_now(
            &ask(
                "tools/call",
                &json!({ "name": SHOW_MARKDOWN, "arguments": { "path": "missing.md" } }),
            ),
            &cwd,
            &sent,
        )
        .unwrap();

        assert_eq!(shown["result"]["isError"], true);
        let text = shown["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("cannot read"), "{text}");
        assert!(requests.try_recv().is_err());
        fs::remove_dir_all(cwd).unwrap();
    }

    /// Calls `run_command` with `arguments`, and answers the request the
    /// chat gets with `ran`.
    async fn run(arguments: Value, ran: Result<Ran, String>) -> Value {
        let (sent, mut requests) = unbounded_channel();
        let chat = tokio::spawn(async move {
            match requests.recv().await {
                Some(ToolRequest::RunCommand {
                    command,
                    cwd,
                    answer,
                }) => {
                    assert_eq!(cwd, Path::new("/work"));
                    answer.send(ran).unwrap();
                    Some(command)
                }
                _ => None,
            }
        });
        let message = ask(
            "tools/call",
            &json!({ "name": RUN_COMMAND, "arguments": arguments }),
        );
        let answered = answer(&message, Path::new("/work"), &sent).await.unwrap();
        drop(sent);
        let asked = chat.await.unwrap();
        assert_eq!(
            asked.as_deref(),
            message["params"]["arguments"]["command"].as_str()
        );
        answered["result"].clone()
    }

    #[tokio::test]
    async fn answers_what_a_command_printed_and_how_it_ended() {
        let ran = Ran {
            output: "ok\n\n".to_owned(),
            ended: Ended::Exited(0),
        };
        let result = run(json!({ "command": "true" }), Ok(ran)).await;
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["text"], "ok\n\n[exit code 0]");

        let ran = Ran {
            output: "error: no\n".to_owned(),
            ended: Ended::Exited(2),
        };
        let result = run(json!({ "command": "false" }), Ok(ran)).await;
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "error: no\n\n[exit code 2]");
    }

    #[tokio::test]
    async fn tells_the_agent_a_command_did_not_start() {
        let result = run(json!({ "command": "true" }), Err("no pty".to_owned())).await;
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "no pty");
    }

    #[tokio::test]
    async fn asks_for_a_command() {
        let (sent, mut requests) = unbounded_channel();
        let message = ask(
            "tools/call",
            &json!({ "name": RUN_COMMAND, "arguments": { "command": " " } }),
        );
        let answered = answer(&message, Path::new("/"), &sent).await.unwrap();
        assert_eq!(answered["result"]["isError"], true);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn keeps_the_end_of_a_long_output() {
        let output = (0..10_000).fold(String::new(), |output, line| {
            output + &format!("line {line}\n")
        });
        let told = told(&Ran {
            output,
            ended: Ended::Signal(2),
        });
        assert!(told.len() < OUTPUT_LENGTH + 100, "{}", told.len());
        let (cut, rest) = told.split_once('\n').unwrap();
        let first: usize = rest.lines().next().unwrap()["line ".len()..]
            .parse()
            .unwrap();
        assert_eq!(cut, format!("[the first {first} lines are cut]"));
        assert!(told.ends_with("line 9999\n\n[ended by signal 2]"), "{told}");
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
