//! An agent reaching its chat's tools through `cox --mcp`, as it starts
//! the MCP server Cox hands it.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cox::mcp::Request;
use serde_json::{Value, json};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("cox-bridge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::canonicalize(directory).unwrap()
}

#[test]
fn the_agent_shows_a_file_on_its_chats_stage() {
    let cwd = scratch("show");
    std::fs::write(cwd.join("plan.md"), "# Plan\n").unwrap();
    let server = cox::mcp::start(cwd.clone()).unwrap();
    let cox::mcp::Server {
        socket,
        mut requests,
    } = server;

    let mut bridge = Command::new(env!("CARGO_BIN_EXE_cox"))
        .args([cox::mcp::OPTION, socket.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut to_cox = bridge.stdin.take().unwrap();
    let mut from_cox = BufReader::new(bridge.stdout.take().unwrap()).lines();
    let mut ask = |message: Value| -> Value {
        writeln!(to_cox, "{message}").unwrap();
        serde_json::from_str(&from_cox.next().unwrap().unwrap()).unwrap()
    };

    let initialized = ask(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "test", "version": "0" } },
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "cox");
    let shown = ask(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "show_markdown", "arguments": { "path": "plan.md" } },
    }));
    assert_eq!(shown["id"], 2);
    assert_eq!(shown["result"]["isError"], false);
    assert_eq!(
        requests.try_recv().unwrap(),
        Request::ShowMarkdown {
            path: cwd.join("plan.md"),
            text: "# Plan\n".to_owned(),
        }
    );

    // The chat closes: the bridge ends, and the socket goes.
    drop(requests);
    let deadline = Instant::now() + Duration::from_secs(10);
    while bridge.try_wait().unwrap().is_none() || socket.exists() {
        assert!(Instant::now() < deadline, "the bridge outlived its chat");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::remove_dir_all(cwd).unwrap();
}
