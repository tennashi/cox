//! An agent reaching its chat's tools through `cox --mcp`, as it starts
//! the MCP server Cox hands it.

use std::io::{BufRead as _, BufReader, Lines, Write as _};
use std::path::{Path, PathBuf};
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
    let server = cox::agent::mcp::start(cwd.clone(), None, true).unwrap();
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
    let server = cox::agent::mcp::start(cwd.clone(), None, true).unwrap();
    let mut agent = Agent::start(&server);
    let mut requests = server.requests;

    // A command answers once the chat says how it went.
    agent.call(2, "run_command", &json!({ "command": "make" }));
    let Some(ToolRequest::RunCommand {
        command,
        cwd: working,
        answer,
        ..
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
    let server = cox::agent::mcp::start(cwd.clone(), only.as_deref(), true).unwrap();
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

#[test]
fn a_chat_in_a_project_gets_no_tool_to_make_one_of_it() {
    let cwd = scratch("in-project");
    let server = cox::agent::mcp::start(cwd.clone(), None, true).unwrap();
    let mut agent = Agent::start(&server);

    let listed = agent.ask(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"propose_project"), "{names:?}");
    assert!(names.contains(&"propose_name"), "{names:?}");
    agent.call(3, "propose_project", &json!({ "name": "anything" }));
    let refused = agent.hear();
    assert_eq!(refused["error"]["message"], "no tool propose_project");
    drop(agent);
    std::fs::remove_dir_all(cwd).unwrap();
}

#[test]
fn a_chat_in_no_project_gets_no_tool_to_name_its_project() {
    let cwd = scratch("no-project");
    let server = cox::agent::mcp::start(cwd.clone(), None, false).unwrap();
    let mut agent = Agent::start(&server);

    let listed = agent.ask(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"propose_name"), "{names:?}");
    // It gets the one that makes a project of it, and the rest.
    assert!(names.contains(&"propose_project"), "{names:?}");
    assert!(names.contains(&"show_markdown"), "{names:?}");
    agent.call(3, "propose_name", &json!({ "name": "anything" }));
    let refused = agent.hear();
    assert_eq!(refused["error"]["message"], "no tool propose_name");
    drop(agent);
    std::fs::remove_dir_all(cwd).unwrap();
}

/// Starts the server of a chat of the project `project`, in `cwd`, that
/// answers where it is as the chat `chat` of that project, and the
/// agent's end of the bridge to it.
fn chat_in(cwd: &std::path::Path, project: &str, chat: &str) -> (Agent, ToolServer) {
    let folder = cwd.to_owned();
    let mut server = cox::agent::mcp::start(cwd.to_owned(), None, true).unwrap();
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
                    folder: Some(folder.clone()),
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
    // A page is read by the `cox` built, as the tools' server runs it.
    cox::raw::page::read_with(PathBuf::from(env!("CARGO_BIN_EXE_cox")));
    // The wiki is in the home the settings set.
    let home = data.join("home");
    std::fs::write(
        data.join("settings.json"),
        json!({ "home": home }).to_string(),
    )
    .unwrap();
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
    let page = home.join("wiki/gpui/scroll.md");
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

    a_project_keeps_its_own_wiki(&mut writer, &mut reader, &one, &home);
    sources_are_kept_apart_from_the_wiki(&mut writer, &mut reader, &home);
    a_project_keeps_its_own_sources(&mut writer, &mut reader, &one, &home);
    drop((writer, reader));
    for folder in [data, one, other] {
        std::fs::remove_dir_all(folder).unwrap();
    }
}

/// A page the writer keeps in its project's own wiki, in `one`, is not
/// the reader's, nor in the human's wiki, in `home`.
fn a_project_keeps_its_own_wiki(writer: &mut Agent, reader: &mut Agent, one: &Path, home: &Path) {
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
    let customers = one.join(".cox/wiki/customers.md");
    assert!(
        written.starts_with(&format!(
            "{} is written, in this project's wiki, used ",
            customers.display()
        )),
        "{written}"
    );
    assert!(!home.join("wiki/customers.md").exists());
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
    assert!(
        found.contains("In this project's wiki:\n\ncustomers: Customers"),
        "{found}"
    );
    // The project's page is read before the human's, unless the human's
    // is asked for.
    writer.call(11, "wiki_read", &json!({ "page": "customers" }));
    let read = writer.hear();
    let read = read["result"]["content"][0]["text"].as_str().unwrap();
    assert!(read.starts_with("# Customers\n"), "{read}");
    writer.call(
        12,
        "wiki_read",
        &json!({ "page": "customers", "wiki": "human" }),
    );
    assert_eq!(writer.hear()["result"]["isError"], true);
}

/// What a chat reads is kept apart from the wiki, in the home's `raw`
/// when the chat says it is for every project, for any chat to find
/// again.
fn sources_are_kept_apart_from_the_wiki(writer: &mut Agent, reader: &mut Agent, home: &Path) {
    writer.call(
        11,
        "raw_keep",
        &json!({
            "topic": "gpui-news",
            "from": "https://docs.rs/gpui/latest/gpui/",
            "title": "gpui docs",
            "text": "Overflow scrolls only inside a bounded height.\n",
            "keep_in_project": false,
        }),
    );
    let kept = writer.hear();
    let kept = kept["result"]["content"][0]["text"].as_str().unwrap();
    let name = kept
        .strip_prefix("The source is kept as ")
        .and_then(|rest| rest.split(", in ").next())
        .unwrap_or_else(|| panic!("{kept}"));
    assert!(
        name.starts_with("gpui-news/") && name.ends_with("-gpui-docs"),
        "{kept}"
    );
    assert!(home.join(format!("raw/{name}.md")).is_file(), "{kept}");
    reader.call(12, "wiki_search", &json!({ "query": "bounded height" }));
    let found = reader.hear();
    let found = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(found.starts_with("No page of the wiki"), "{found}");
    reader.call(13, "raw_search", &json!({ "query": "bounded height" }));
    let found = reader.hear();
    let found = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(found.contains(&format!("{name}: gpui docs")), "{found}");
    reader.call(14, "raw_read", &json!({ "topic": "gpui-news" }));
    let listed = reader.hear();
    let listed = listed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(listed.ends_with(&format!("first:\n{name}")), "{listed}");
    reader.call(15, "raw_read", &json!({ "source": name }));
    let read = reader.hear();
    let read = read["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        read.contains("chat: \"Fix scroll\"\nchat_id: 7\nproject: \"one\"\n"),
        "{read}"
    );
    // Marked as the source's, not the human's.
    assert!(
        read.starts_with(&format!(
            "What the source {name} says, lines 1 to 11 of 11,"
        )),
        "{read}"
    );
    assert!(
        read.contains("# gpui docs\n\nOverflow scrolls only inside a bounded height.\n--- source "),
        "{read}"
    );
    reader.call(16, "raw_read", &json!({ "source": name, "from_line": 9 }));
    let read = reader.hear();
    let read = read["result"]["content"][0]["text"].as_str().unwrap();
    assert!(read.contains(", lines 9 to 11 of 11,"), "{read}");
    assert!(!read.contains("chat_id"), "{read}");
    // Cox reads only a web page itself, and only of the web.
    writer.call(
        17,
        "raw_keep",
        &json!({ "topic": "gpui-news", "from": "cargo doc --open" }),
    );
    let refused = writer.hear();
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    writer.call(
        18,
        "raw_keep",
        &json!({ "topic": "gpui-news", "from": "http://127.0.0.1:9/admin" }),
    );
    let refused = writer.hear();
    let refused = refused["result"]["content"][0]["text"].as_str().unwrap();
    assert!(refused.contains("not on the web"), "{refused}");
}

/// What a chat of a project reads is kept in the project's own `raw`,
/// in `one`, unless it says otherwise: its chats find it, the project's
/// before the human's, and the reader, of another project, does not.
fn a_project_keeps_its_own_sources(
    writer: &mut Agent,
    reader: &mut Agent,
    one: &Path,
    home: &Path,
) {
    writer.call(
        19,
        "raw_keep",
        &json!({
            "topic": "sketches",
            "from": "chat 7",
            "title": "Acme mock",
            "text": "Acme wants a plain list, not cards.\n",
        }),
    );
    let kept = writer.hear();
    let kept = kept["result"]["content"][0]["text"].as_str().unwrap();
    let name = kept
        .strip_prefix("The source is kept as ")
        .and_then(|rest| rest.split(", in ").next())
        .unwrap_or_else(|| panic!("{kept}"));
    assert!(kept.contains(", in this project's sources, in "), "{kept}");
    assert!(one.join(format!("raw/{name}.md")).is_file(), "{kept}");
    assert!(!home.join(format!("raw/{name}.md")).exists(), "{kept}");
    reader.call(20, "raw_search", &json!({ "query": "plain list" }));
    let found = reader.hear();
    let found = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(found.starts_with("No source kept"), "{found}");
    reader.call(21, "raw_read", &json!({ "source": name }));
    assert_eq!(reader.hear()["result"]["isError"], true);
    writer.call(22, "raw_search", &json!({ "query": "plain list" }));
    let found = writer.hear();
    let found = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        found.contains(&format!("In this project's sources:\n\n{name}: Acme mock")),
        "{found}"
    );
    writer.call(23, "raw_read", &json!({}));
    let listed = writer.hear();
    let listed = listed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        listed.contains(&format!(
            "In this project's sources, the last kept first:\n{name}"
        )),
        "{listed}"
    );
    assert!(
        listed.contains("In the human's sources, the last kept first:\ngpui-news/"),
        "{listed}"
    );
    writer.call(24, "raw_read", &json!({ "source": name }));
    let read = writer.hear();
    let read = read["result"]["content"][0]["text"].as_str().unwrap();
    assert!(read.contains("Acme wants a plain list"), "{read}");
}
