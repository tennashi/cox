//! An agent reaching its chat's tools through `cox --mcp`, as it starts
//! the MCP server Cox hands it.

use std::io::{BufRead as _, BufReader, Lines, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use cox::agent::mcp::{ChatRequest, Ended, Ran, ToolRequest, ToolServer, Whereabouts};
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
    let server = cox::agent::mcp::start(cwd.clone(), None).unwrap();
    let mut agent = Agent::start(&server);
    let ToolServer {
        socket,
        mut requests,
        ..
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
    let server = cox::agent::mcp::start(cwd.clone(), None).unwrap();
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

#[test]
fn the_agent_gets_only_the_tools_its_setup_names() {
    let cwd = scratch("only");
    std::fs::write(cwd.join("plan.md"), "# Plan\n").unwrap();
    let only = Some(vec!["show_markdown".to_owned()]);
    let server = cox::agent::mcp::start(cwd.clone(), only).unwrap();
    let mut agent = Agent::start(&server);
    let mut requests = server.requests;

    let listed = agent.ask(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["show_markdown"]);

    // A tool the agent does not get is no tool to it.
    agent.call(3, "run_command", &json!({ "command": "make" }));
    let refused = agent.hear();
    assert_eq!(refused["error"]["message"], "no tool run_command");
    assert!(requests.try_recv().is_err());
    // The one it gets works.
    agent.call(4, "show_markdown", &json!({ "path": "plan.md" }));
    assert_eq!(agent.hear()["result"]["isError"], false);
    assert!(matches!(
        requests.try_recv().unwrap(),
        ToolRequest::ShowMarkdown { .. }
    ));
    drop(agent);
    std::fs::remove_dir_all(cwd).unwrap();
}

/// Starts the server of a chat of the project `project`, in `cwd`, that
/// answers where it is as the chat `chat` of that project, and the
/// agent's end of the bridge to it.
fn chat_in(cwd: &std::path::Path, project: &str, chat: &str) -> (Agent, ToolServer) {
    let mut server = cox::agent::mcp::start(cwd.to_owned(), None).unwrap();
    let agent = Agent::start(&server);
    let mut chat_requests = std::mem::replace(
        &mut server.chat_requests,
        tokio::sync::mpsc::unbounded_channel().1,
    );
    let (project, chat) = (project.to_owned(), chat.to_owned());
    std::thread::spawn(move || {
        while let Some(request) = chat_requests.blocking_recv() {
            if let ChatRequest::Whereabouts { answer } = request {
                let _ = answer.send(Whereabouts {
                    project: project.clone(),
                    chat: chat.clone(),
                    kept: Some("7".to_owned()),
                });
            }
        }
    });
    (agent, server)
}

#[test]
fn a_page_one_project_writes_in_the_wiki_is_found_from_another_unless_kept() {
    let data = scratch("wiki-data");
    cox::storage::keep_in(&data);
    let one = scratch("wiki-one");
    let other = scratch("wiki-other");
    let (mut writer, _one) = chat_in(&one, "one", "Fix scroll");
    let (mut reader, _other) = chat_in(&other, "other", "Layout");

    writer.call(
        2,
        "wiki_write",
        &json!({
            "page": "gpui/scroll",
            "text": "# Scrolling in GPUI\n\nA flex child needs `min_h_0` to scroll.\n",
            "paths": ["src/view.rs"],
        }),
    );
    let written = writer.hear();
    assert_eq!(written["result"]["isError"], false, "{written}");
    let page = data.join("wiki/gpui/scroll.md");
    assert!(
        written["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{} is written", page.display())),
        "{written}"
    );

    reader.call(3, "wiki_search", &json!({ "query": "scroll min_h_0" }));
    let found = reader.hear();
    let found = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(found.contains("gpui/scroll: Scrolling in GPUI"), "{found}");
    assert!(found.contains("`min_h_0`"), "{found}");

    reader.call(4, "wiki_read", &json!({ "page": "gpui/scroll" }));
    let read = reader.hear();
    let read = read["result"]["content"][0]["text"].as_str().unwrap();
    assert!(read.starts_with("# Scrolling in GPUI\n"), "{read}");
    assert!(
        read.ends_with(" in one, chat \"Fix scroll\" (#7): src/view.rs\n"),
        "{read}"
    );
    reader.call(5, "wiki_read", &json!({}));
    let index = reader.hear();
    let index = index["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        index.contains("- [[gpui/scroll|Scrolling in GPUI]]: A flex child"),
        "{index}"
    );

    // A page the writer keeps in its project is not the reader's.
    writer.call(
        6,
        "wiki_write",
        &json!({
            "page": "customers",
            "text": "# Customers\n\nAcme scrolls a lot.\n",
            "keep_in_project": true,
        }),
    );
    let written = writer.hear();
    let written = written["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        written.ends_with(" It is kept in one, for its chats alone."),
        "{written}"
    );
    reader.call(7, "wiki_search", &json!({ "query": "scrolls" }));
    let found = reader.hear();
    let found = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(found.starts_with("No page of the wiki"), "{found}");
    reader.call(8, "wiki_read", &json!({ "page": "customers" }));
    assert_eq!(reader.hear()["result"]["isError"], true);
    reader.call(9, "wiki_read", &json!({}));
    let index = reader.hear();
    let index = index["result"]["content"][0]["text"].as_str().unwrap();
    assert!(!index.contains("customers"), "{index}");
    writer.call(10, "wiki_search", &json!({ "query": "scrolls" }));
    let found = writer.hear();
    let found = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(found.contains("customers: Customers"), "{found}");
    drop((writer, reader));
    for folder in [data, one, other] {
        std::fs::remove_dir_all(folder).unwrap();
    }
}
