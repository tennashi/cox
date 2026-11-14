//! An agent reaching its chat's tools through `cox --mcp`, as it starts
//! the MCP server Cox hands it.

use coxd::testing::scratch;
use coxd::testing::waits;

use std::io::{BufRead as _, BufReader, Lines, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use coxd::agent::mcp::{ChatRequest, Ended, Ran, ToolRequest, ToolServer, Whereabouts};
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    std::fs::canonicalize(scratch::made(&format!("bridge-{name}"))).unwrap()
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
            .args([coxd::agent::mcp::OPTION, server.socket.to_str().unwrap()])
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
    let server = coxd::agent::mcp::start(cwd.clone(), None, true).unwrap();
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
    waits::until("the bridge's end with its chat", || {
        agent.bridge.try_wait().unwrap().is_some() && !socket.exists()
    });
}

#[test]
fn the_agent_runs_a_command_on_its_chats_stage_and_can_cancel_it() {
    let cwd = scratch("run");
    let server = coxd::agent::mcp::start(cwd.clone(), None, true).unwrap();
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
    waits::until("the end of the wait for the command", || answer.is_closed());
    drop(agent);
}

#[test]
fn the_agent_gets_only_the_tools_its_setup_names() {
    let cwd = scratch("only");
    std::fs::write(cwd.join("plan.md"), "# Plan\n").unwrap();
    let only = Some(vec!["show_markdown".to_owned()]);
    let server = coxd::agent::mcp::start(cwd.clone(), only.as_deref(), true).unwrap();
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
}

#[test]
fn a_chat_in_a_project_gets_no_tool_to_make_one_of_it() {
    let cwd = scratch("in-project");
    let server = coxd::agent::mcp::start(cwd.clone(), None, true).unwrap();
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
}

#[test]
fn a_chat_in_no_project_gets_no_tool_to_name_its_project() {
    let cwd = scratch("no-project");
    let server = coxd::agent::mcp::start(cwd.clone(), None, false).unwrap();
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
}

/// Starts the server of a chat of the project `project`, in `cwd`, that
/// answers where it is as the chat `chat` of that project, and the
/// agent's end of the bridge to it.
fn chat_in(cwd: &std::path::Path, project: &str, chat: &str) -> (Agent, ToolServer) {
    let folder = cwd.to_owned();
    let mut server = coxd::agent::mcp::start(cwd.to_owned(), None, true).unwrap();
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

/// Two chats, each of a project of its own: the writer, chat 7 "Fix
/// scroll" of the project "one", and the reader, of the project "other",
/// with a home of the test's own, where the human's wiki and sources are.
struct Wikis {
    writer: Agent,
    reader: Agent,
    /// The folder of the writer's project.
    one: PathBuf,
    /// The home, which holds the human's wiki and sources.
    home: PathBuf,
    _servers: (ToolServer, ToolServer),
}

impl Wikis {
    fn open(name: &str) -> Self {
        let data = scratch::data();
        // A page is read by the `cox` built, as the tools' server runs it.
        coxd::raw::page::read_with(PathBuf::from(env!("CARGO_BIN_EXE_cox")));
        let home = std::fs::canonicalize(&data).unwrap().join("home");
        std::fs::write(
            data.join("settings.json"),
            json!({ "home": home }).to_string(),
        )
        .unwrap();
        let one = scratch(&format!("{name}-one"));
        let other = scratch(&format!("{name}-other"));
        let (writer, writer_server) = chat_in(&one, "one", "Fix scroll");
        let (reader, reader_server) = chat_in(&other, "other", "Layout");
        Self {
            writer,
            reader,
            one,
            home,
            _servers: (writer_server, reader_server),
        }
    }
}

/// What the tool `name` answers `agent` called with `arguments`, in
/// words, and whether it says it failed.
fn told(agent: &mut Agent, name: &str, arguments: &Value) -> (String, bool) {
    agent.call(2, name, arguments);
    let answer = agent.hear();
    let text = answer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{answer}"))
        .to_owned();
    (text, answer["result"]["isError"] == true)
}

/// The page on scrolling in GPUI, made from `sources`.
fn scroll_page(sources: &[&str]) -> Value {
    json!({
        "page": "gpui/scroll",
        "text": "# Scrolling in GPUI\n\nA flex child needs `min_h_0` to scroll.\n",
        "paths": ["src/view.rs"],
        "sources": sources,
    })
}

/// Keeps what `arguments` say as a source, as `agent`, and answers the
/// name it is kept by, and what Cox said.
fn keep_source(agent: &mut Agent, arguments: &Value) -> (String, String) {
    let (kept, failed) = told(agent, "raw_keep", arguments);
    assert!(!failed, "{kept}");
    let name = kept
        .strip_prefix("The source is kept as ")
        .and_then(|rest| rest.split(", in ").next())
        .unwrap_or_else(|| panic!("{kept}"))
        .to_owned();
    (name, kept)
}

/// The gpui docs, kept as a source of the human's, for every project.
fn keep_the_gpui_docs(agent: &mut Agent) -> (String, String) {
    keep_source(
        agent,
        &json!({
            "topic": "gpui-news",
            "from": "https://docs.rs/gpui/latest/gpui/",
            "title": "gpui docs",
            "text": "Overflow scrolls only inside a bounded height.\n",
            "keep_in_project": false,
        }),
    )
}

#[test]
fn a_wiki_page_from_no_source_is_not_written() {
    let mut wikis = Wikis::open("wiki-no-source");
    let mut page = scroll_page(&[]);
    page.as_object_mut().unwrap().remove("sources");

    let (refused, _) = told(&mut wikis.writer, "wiki_write", &page);
    assert!(refused.starts_with("no sources: "), "{refused}");
    assert!(!wikis.home.join("wiki/gpui/scroll.md").exists());
}

#[test]
fn a_wiki_page_from_a_source_not_kept_is_not_written() {
    let mut wikis = Wikis::open("wiki-not-kept");

    let (refused, _) = told(
        &mut wikis.writer,
        "wiki_write",
        &scroll_page(&["gpui-news/nothing"]),
    );
    assert!(
        refused.starts_with("gpui-news/nothing is no source kept"),
        "{refused}"
    );
    assert!(!wikis.home.join("wiki/gpui/scroll.md").exists());
}

#[test]
fn a_page_one_project_writes_in_the_human_s_wiki_is_found_from_another() {
    let mut wikis = Wikis::open("wiki-found");

    let (written, failed) = told(&mut wikis.writer, "wiki_write", &scroll_page(&["chat 7"]));
    assert!(!failed, "{written}");
    let page = wikis.home.join("wiki/gpui/scroll.md");
    assert!(
        written.starts_with(&format!("{} is written", page.display())),
        "{written}"
    );
    let (found, _) = told(
        &mut wikis.reader,
        "wiki_search",
        &json!({ "query": "scroll min_h_0" }),
    );
    assert!(found.contains("gpui/scroll: Scrolling in GPUI"), "{found}");
    assert!(found.contains("`min_h_0`"), "{found}");
    let (read, _) = told(
        &mut wikis.reader,
        "wiki_read",
        &json!({ "page": "gpui/scroll" }),
    );
    assert!(read.starts_with("# Scrolling in GPUI\n"), "{read}");
    assert!(
        read.ends_with(" in one, chat \"Fix scroll\" (#7), from chat 7: src/view.rs\n"),
        "it says where it was made, and from what: {read}"
    );
    let (index, _) = told(&mut wikis.reader, "wiki_read", &json!({}));
    assert!(
        index.contains("- [[gpui/scroll|Scrolling in GPUI]]: A flex child"),
        "{index}"
    );
}

/// The page on customers, kept in the project's own wiki.
fn customers_page() -> Value {
    json!({
        "page": "customers",
        "text": "# Customers\n\nAcme scrolls a lot.\n",
        "keep_in_project": true,
        "sources": ["chat 7"],
    })
}

#[test]
fn a_page_kept_in_a_project_s_wiki_is_found_by_its_chats_alone() {
    let mut wikis = Wikis::open("wiki-project");

    let (written, _) = told(&mut wikis.writer, "wiki_write", &customers_page());
    let customers = wikis.one.join("wiki/customers.md");
    assert!(
        written.starts_with(&format!(
            "{} is written, in this project's wiki, used ",
            customers.display()
        )),
        "{written}"
    );
    assert!(!wikis.home.join("wiki/customers.md").exists());
    let (found, _) = told(
        &mut wikis.writer,
        "wiki_search",
        &json!({ "query": "scrolls" }),
    );
    assert!(
        found.contains("In this project's wiki:\n\ncustomers: Customers"),
        "{found}"
    );
    let (found, _) = told(
        &mut wikis.reader,
        "wiki_search",
        &json!({ "query": "scrolls" }),
    );
    assert!(found.starts_with("No page of the wiki"), "{found}");
    let (_, failed) = told(
        &mut wikis.reader,
        "wiki_read",
        &json!({ "page": "customers" }),
    );
    assert!(failed, "another project's page is not read");
    let (index, _) = told(&mut wikis.reader, "wiki_read", &json!({}));
    assert!(!index.contains("customers"), "{index}");
}

#[test]
fn a_project_s_page_is_read_before_the_human_s_unless_the_human_s_is_asked_for() {
    let mut wikis = Wikis::open("wiki-first");
    told(&mut wikis.writer, "wiki_write", &customers_page());

    let (read, _) = told(
        &mut wikis.writer,
        "wiki_read",
        &json!({ "page": "customers" }),
    );
    assert!(read.starts_with("# Customers\n"), "{read}");
    let (_, failed) = told(
        &mut wikis.writer,
        "wiki_read",
        &json!({ "page": "customers", "wiki": "human" }),
    );
    assert!(failed, "the human's wiki has no such page");
}

#[test]
fn a_source_kept_for_every_project_is_found_by_another_and_not_in_the_wiki() {
    let mut wikis = Wikis::open("raw-every");

    let (name, kept) = keep_the_gpui_docs(&mut wikis.writer);
    assert!(
        name.starts_with("gpui-news/") && name.ends_with("-gpui-docs"),
        "{kept}"
    );
    assert!(
        wikis.home.join(format!("raw/{name}.md")).is_file(),
        "{kept}"
    );
    let (found, _) = told(
        &mut wikis.reader,
        "wiki_search",
        &json!({ "query": "bounded height" }),
    );
    assert!(found.starts_with("No page of the wiki"), "{found}");
    let (found, _) = told(
        &mut wikis.reader,
        "raw_search",
        &json!({ "query": "bounded height" }),
    );
    assert!(found.contains(&format!("{name}: gpui docs")), "{found}");
    let (listed, _) = told(
        &mut wikis.reader,
        "raw_read",
        &json!({ "topic": "gpui-news" }),
    );
    assert!(listed.ends_with(&format!("first:\n{name}")), "{listed}");
}

#[test]
fn a_source_reads_as_the_source_s_saying_where_it_was_kept_from() {
    let mut wikis = Wikis::open("raw-read");
    let (name, _) = keep_the_gpui_docs(&mut wikis.writer);

    let (read, _) = told(&mut wikis.reader, "raw_read", &json!({ "source": name }));
    assert!(
        read.contains("chat: \"Fix scroll\"\nchat_id: 7\nproject: \"one\"\n"),
        "{read}"
    );
    assert!(
        read.starts_with(&format!(
            "What the source {name} says, lines 1 to 11 of 11,"
        )),
        "marked as the source's, not the human's: {read}"
    );
    assert!(
        read.contains("# gpui docs\n\nOverflow scrolls only inside a bounded height.\n--- source "),
        "{read}"
    );
}

#[test]
fn a_source_reads_from_the_line_asked_for() {
    let mut wikis = Wikis::open("raw-from-line");
    let (name, _) = keep_the_gpui_docs(&mut wikis.writer);

    let (read, _) = told(
        &mut wikis.reader,
        "raw_read",
        &json!({ "source": name, "from_line": 9 }),
    );
    assert!(read.contains(", lines 9 to 11 of 11,"), "{read}");
    assert!(!read.contains("chat_id"), "{read}");
}

#[test]
fn cox_keeps_as_a_source_only_a_web_page_of_the_web_it_reads_itself() {
    let mut wikis = Wikis::open("raw-web");

    let (_, failed) = told(
        &mut wikis.writer,
        "raw_keep",
        &json!({ "topic": "gpui-news", "from": "cargo doc --open" }),
    );
    assert!(failed, "no command is run for a source");
    let (refused, _) = told(
        &mut wikis.writer,
        "raw_keep",
        &json!({ "topic": "gpui-news", "from": "http://127.0.0.1:9/admin" }),
    );
    assert!(refused.contains("not on the web"), "{refused}");
}

/// The Acme mock, kept as a source of the project of `agent`'s chat.
fn keep_the_acme_mock(agent: &mut Agent) -> (String, String) {
    keep_source(
        agent,
        &json!({
            "topic": "sketches",
            "from": "chat 7",
            "title": "Acme mock",
            "text": "Acme wants a plain list, not cards.\n",
        }),
    )
}

#[test]
fn a_source_a_project_keeps_is_found_by_its_chats_first_and_not_another_s() {
    let mut wikis = Wikis::open("raw-project");
    keep_the_gpui_docs(&mut wikis.writer);

    let (name, kept) = keep_the_acme_mock(&mut wikis.writer);
    assert!(kept.contains(", in this project's sources, in "), "{kept}");
    assert!(wikis.one.join(format!("raw/{name}.md")).is_file(), "{kept}");
    assert!(
        !wikis.home.join(format!("raw/{name}.md")).exists(),
        "{kept}"
    );
    let (found, _) = told(
        &mut wikis.reader,
        "raw_search",
        &json!({ "query": "plain list" }),
    );
    assert!(found.starts_with("No source kept"), "{found}");
    let (_, failed) = told(&mut wikis.reader, "raw_read", &json!({ "source": name }));
    assert!(failed, "another project's source is not read");
    let (found, _) = told(
        &mut wikis.writer,
        "raw_search",
        &json!({ "query": "plain list" }),
    );
    assert!(
        found.contains(&format!("In this project's sources:\n\n{name}: Acme mock")),
        "{found}"
    );
    let (listed, _) = told(&mut wikis.writer, "raw_read", &json!({}));
    assert!(
        listed.contains(&format!(
            "In this project's sources, the last kept first:\n{name}"
        )),
        "{listed}"
    );
    assert!(
        listed.contains("In the human's sources, the last kept first:\ngpui-news/"),
        "the project's are listed before the human's: {listed}"
    );
    let (read, _) = told(&mut wikis.writer, "raw_read", &json!({ "source": name }));
    assert!(read.contains("Acme wants a plain list"), "{read}");
}

#[test]
fn a_page_made_from_a_project_s_source_names_it_and_another_project_cannot_make_it() {
    let mut wikis = Wikis::open("raw-page");
    let (name, _) = keep_the_acme_mock(&mut wikis.writer);
    let page = json!({
        "page": "acme",
        "text": "# Acme\n\nWants a plain list.\n",
        "keep_in_project": true,
        "sources": [name],
    });

    let (_, failed) = told(&mut wikis.reader, "wiki_write", &page);
    assert!(failed, "the reader has no such source to make it from");
    let (written, failed) = told(&mut wikis.writer, "wiki_write", &page);
    assert!(!failed, "{written}");
    let page = std::fs::read_to_string(wikis.one.join("wiki/acme.md")).unwrap();
    assert!(page.contains(&format!(", from {name}\n")), "{page}");
}
