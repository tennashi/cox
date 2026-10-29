//! An agent reaching its chat's tools through `cox --mcp`, as it starts
//! the MCP server Cox hands it.

use std::io::{BufRead as _, BufReader, Lines, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use cox::agent::mcp::{Ended, Ran, ToolRequest, ToolServer};
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("cox-bridge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::canonicalize(directory).unwrap()
}

/// The agent's end of the bridge to a chat's server.
struct Agent {
    bridge: Child,
    to_cox: ChildStdin,
    from_cox: Lines<BufReader<ChildStdout>>,
}

impl Agent {
    /// Starts the bridge to `server`, as the agent does, and starts the
    /// session.
    fn start(server: &ToolServer) -> Self {
        let mut bridge = Command::new(env!("CARGO_BIN_EXE_cox"))
            .args([cox::agent::mcp::OPTION, server.socket.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let to_cox = bridge.stdin.take().unwrap();
        let from_cox = BufReader::new(bridge.stdout.take().unwrap()).lines();
        let mut agent = Self {
            bridge,
            to_cox,
            from_cox,
        };
        let initialized = agent.ask(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": { "name": "test", "version": "0" } },
        }));
        assert_eq!(initialized["result"]["serverInfo"]["name"], "cox");
        agent
    }

    fn say(&mut self, message: &Value) {
        writeln!(self.to_cox, "{message}").unwrap();
    }

    fn hear(&mut self) -> Value {
        serde_json::from_str(&self.from_cox.next().unwrap().unwrap()).unwrap()
    }

    fn ask(&mut self, message: &Value) -> Value {
        self.say(message);
        self.hear()
    }

    /// Calls the tool `name` with `arguments` as the request `id`, and
    /// waits for no answer.
    fn call(&mut self, id: u64, name: &str, arguments: &Value) {
        self.say(&json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        }));
    }
}

#[test]
fn the_agent_shows_a_file_on_its_chats_stage() {
    let cwd = scratch("show");
    std::fs::write(cwd.join("plan.md"), "# Plan\n").unwrap();
    let server = cox::agent::mcp::start(cwd.clone()).unwrap();
    let mut agent = Agent::start(&server);
    let ToolServer {
        socket,
        mut requests,
    } = server;

    agent.call(2, "show_markdown", &json!({ "path": "plan.md" }));
    let shown = agent.hear();
    assert_eq!(shown["id"], 2);
    assert_eq!(shown["result"]["isError"], false);
    match requests.try_recv().unwrap() {
        ToolRequest::ShowMarkdown { path, text } => {
            assert_eq!(path, cwd.join("plan.md"));
            assert_eq!(text, "# Plan\n");
        }
        request => panic!("asked {request:?}"),
    }

    // The chat closes: the bridge ends, and the socket goes.
    drop(requests);
    let deadline = Instant::now() + Duration::from_secs(10);
    while agent.bridge.try_wait().unwrap().is_none() || socket.exists() {
        assert!(Instant::now() < deadline, "the bridge outlived its chat");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn the_agent_runs_a_command_on_its_chats_stage_and_can_cancel_it() {
    let cwd = scratch("run");
    let server = cox::agent::mcp::start(cwd.clone()).unwrap();
    let mut agent = Agent::start(&server);
    let mut requests = server.requests;

    // A command answers once the chat says how it went.
    agent.call(2, "run_command", &json!({ "command": "make" }));
    let Some(ToolRequest::RunCommand {
        command,
        cwd: working,
        answer,
    }) = requests.blocking_recv()
    else {
        panic!("asked to run no command");
    };
    assert_eq!(command, "make");
    assert_eq!(working, cwd);
    // The server answers what else the agent asks while it runs.
    let pinged = agent.ask(&json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }));
    assert_eq!(pinged["id"], 3);
    answer
        .send(Ok(Ran {
            output: "built".to_owned(),
            ended: Ended::Exited(0),
        }))
        .unwrap();
    let ran = agent.hear();
    assert_eq!(ran["id"], 2);
    assert_eq!(
        ran["result"]["content"][0]["text"],
        "built\n\n[exit code 0]"
    );

    // The agent cancels a command: the chat hears it waits no longer.
    agent.call(4, "run_command", &json!({ "command": "sleep 60" }));
    let Some(ToolRequest::RunCommand { answer, .. }) = requests.blocking_recv() else {
        panic!("asked to run no command");
    };
    agent.say(&json!({
        "jsonrpc": "2.0", "method": "notifications/cancelled",
        "params": { "requestId": 4, "reason": "the human stopped the turn" },
    }));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !answer.is_closed() {
        assert!(Instant::now() < deadline, "the command was waited for");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(agent);
    std::fs::remove_dir_all(cwd).unwrap();
}
