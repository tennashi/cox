//! Cox's own tools for a chat's agent, as an MCP server that answers inside
//! Cox, where the tools reach what the chat holds, such as its stage.
//!
//! The agent starts its MCP servers itself, as programs that speak MCP on
//! their standard input and output. So the server Cox hands the agent is
//! Cox itself, run as `cox --mcp <socket>`: a [`bridge`] that only carries
//! what the agent says to the chat's [`Server`], on a Unix socket, and the
//! answers back. Each chat has a socket of its own, in a directory that
//! only the human's user can enter, so that an agent reaches its own chat
//! and no other. The socket goes when the chat does.
//!
//! What goes over the socket is MCP as it goes over standard input and
//! output: JSON-RPC messages, one to a line.

use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use anyhow::Context as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The option that runs `cox` as the bridge, followed by the socket.
pub const OPTION: &str = "--mcp";

/// What the agent's tools ask of its chat.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// Show the human the Markdown file at `path`, an absolute path,
    /// which says `text`, on a tab of the stage.
    ShowMarkdown { path: PathBuf, text: String },
}

/// One chat's server, as the chat holds it.
pub struct Server {
    /// Where the bridge reaches the server.
    pub socket: PathBuf,
    /// What the agent's tools ask. Dropping it stops the server, and
    /// removes the socket.
    pub requests: UnboundedReceiver<Request>,
}

/// The versions of MCP the server speaks, newest first. Its tools work
/// the same in each.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const SHOW_MARKDOWN: &str = "show_markdown";

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
pub fn start(cwd: PathBuf) -> anyhow::Result<Server> {
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
    Ok(Server { socket, requests })
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
    requests: UnboundedSender<Request>,
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
    requests: &UnboundedSender<Request>,
) -> anyhow::Result<()> {
    let (heard, mut said) = stream.into_split();
    let mut lines = BufReader::new(heard).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(message) => answer(&message, cwd, requests),
            Err(error) => Some(failure(&Value::Null, PARSE_ERROR, &error.to_string())),
        };
        if let Some(answer) = answer {
            let mut line = answer.to_string();
            line.push('\n');
            said.write_all(line.as_bytes()).await?;
        }
    }
    Ok(())
}

/// The answer to `message`, or `None` when it needs none: a
/// notification, or an answer to a request, which the server makes none
/// of.
fn answer(message: &Value, cwd: &Path, requests: &UnboundedSender<Request>) -> Option<Value> {
    let id = message.get("id")?;
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let result = match method {
        "initialize" => Ok(initialize(params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => call(params, cwd, requests),
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
    json!([{
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
    }])
}

/// Calls the tool `params` names, and answers what it did, or the error
/// the agent made in calling it.
fn call(
    params: &Value,
    cwd: &Path,
    requests: &UnboundedSender<Request>,
) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    let done = match name {
        SHOW_MARKDOWN => show_markdown(arguments, cwd, requests),
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
    requests: &UnboundedSender<Request>,
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
        .send(Request::ShowMarkdown { path, text })
        .map_err(|_| "the chat has closed".to_owned())?;
    Ok(shown)
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
            answer(
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
        assert_eq!(answer(&initialized, cwd, &sent), None);
        let missing = answer(&ask("resources/list", &json!({})), cwd, &sent).unwrap();
        assert_eq!(missing["id"], 7);
        assert_eq!(missing["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn lists_the_tools() {
        let (sent, _requests) = unbounded_channel();
        let listed = answer(&ask("tools/list", &json!({})), Path::new("/"), &sent).unwrap();
        assert_eq!(listed["result"]["tools"][0]["name"], SHOW_MARKDOWN);
    }

    #[test]
    fn shows_a_file_from_the_working_directory() {
        let cwd = scratch("show");
        fs::write(cwd.join("plan.md"), "# Plan\n").unwrap();
        let (sent, mut requests) = unbounded_channel();

        let shown = answer(
            &ask(
                "tools/call",
                &json!({ "name": SHOW_MARKDOWN, "arguments": { "path": "plan.md" } }),
            ),
            &cwd,
            &sent,
        )
        .unwrap();

        assert_eq!(shown["result"]["isError"], false);
        assert_eq!(
            requests.try_recv().unwrap(),
            Request::ShowMarkdown {
                path: cwd.join("plan.md"),
                text: "# Plan\n".to_owned()
            }
        );
        fs::remove_dir_all(cwd).unwrap();
    }

    #[test]
    fn tells_the_agent_of_a_file_it_cannot_read() {
        let cwd = scratch("missing");
        let (sent, mut requests) = unbounded_channel();

        let shown = answer(
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
