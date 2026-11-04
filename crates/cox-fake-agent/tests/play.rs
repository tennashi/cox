//! Cox's side of ACP, talking to `cox-fake-agent` as it plays scenarios.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

/// The agent, playing a scenario, as Cox talks to it.
struct Agent {
    child: Child,
    input: ChildStdin,
    heard: Receiver<Value>,
    asked: u64,
}

impl Agent {
    /// Starts the agent on `scenario`, written to a file named `name`.
    fn play(name: &str, scenario: &Value) -> Self {
        let path = scratch(name);
        std::fs::write(&path, scenario.to_string()).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_cox-fake-agent"))
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output: ChildStdout = child.stdout.take().unwrap();
        let (sent, heard) = channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if sent.send(serde_json::from_str(&line).unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            heard,
            asked: 0,
        }
    }

    /// Starts the agent and a session with it, handing it `servers`.
    fn session(name: &str, scenario: &Value, servers: &Value) -> Self {
        let mut agent = Self::play(name, scenario);
        agent.ask("initialize", &json!({ "protocolVersion": 1 }));
        let (_, opened) = agent.ask("session/new", &json!({ "cwd": "/", "mcpServers": servers }));
        assert_eq!(opened["result"]["sessionId"], "fake-1");
        agent
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.input, "{message}").unwrap();
        self.input.flush().unwrap();
    }

    /// Asks `method`, and answers with what the agent sent until its
    /// answer, and the answer.
    fn ask(&mut self, method: &str, params: &Value) -> (Vec<Value>, Value) {
        self.asked += 1;
        let id = self.asked;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        self.until(|message| message["id"] == id && message.get("method").is_none())
    }

    /// Sends `text` as a prompt, without waiting.
    fn prompt(&mut self, text: &str) -> u64 {
        self.asked += 1;
        let id = self.asked;
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "session/prompt",
            "params": { "sessionId": "fake-1", "prompt": [{ "type": "text", "text": text }] },
        }));
        id
    }

    /// What the agent sent until `last`, and `last`.
    fn until(&mut self, last: impl Fn(&Value) -> bool) -> (Vec<Value>, Value) {
        let mut before = Vec::new();
        loop {
            let message = self
                .heard
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("the agent went quiet after {before:#?}"));
            if last(&message) {
                return (before, message);
            }
            before.push(message);
        }
    }

    /// Takes a turn on `text`, and answers its updates and its answer.
    fn turn(&mut self, text: &str) -> (Vec<Value>, Value) {
        let id = self.prompt(text);
        self.until(|message| message["id"] == id && message.get("method").is_none())
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("cox-fake-agent-{}-{name}.json", std::process::id()))
}

/// The updates among `sent`.
fn updates(sent: &[Value]) -> Vec<&Value> {
    sent.iter()
        .filter(|message| message["method"] == "session/update")
        .map(|message| &message["params"]["update"])
        .collect()
}

/// What the agent replied among `sent`.
fn replied(sent: &[Value]) -> String {
    updates(sent)
        .into_iter()
        .filter(|update| update["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|update| update["content"]["text"].as_str())
        .collect()
}

#[test]
fn a_turn_takes_its_steps_in_order() {
    let scenario = json!({ "turns": [{ "steps": [
        { "think": "Read first." },
        { "tool": { "id": "r1", "title": "Read src/main.rs", "kind": "read", "status": "completed" } },
        { "say": "It prints hello." },
        { "update": { "sessionUpdate": "session_info_update", "title": "Greeting" } },
    ] }] });
    let mut agent = Agent::session("order", &scenario, &json!([]));

    let (sent, answer) = agent.turn("what does it print?");

    assert_eq!(answer["result"]["stopReason"], "end_turn");
    let updates = updates(&sent);
    assert_eq!(updates[0]["sessionUpdate"], "agent_thought_chunk");
    assert_eq!(updates[0]["content"]["text"], "Read first.");
    assert_eq!(updates[1]["sessionUpdate"], "tool_call");
    assert_eq!(updates[1]["toolCallId"], "r1");
    assert_eq!(updates[1]["title"], "Read src/main.rs");
    assert_eq!(replied(&sent), "It prints hello.");
    assert_eq!(updates.last().unwrap()["title"], "Greeting");
}

#[test]
fn each_line_takes_the_first_turn_left_that_answers_it() {
    let scenario = json!({ "turns": [
        { "when": "judge", "steps": [{ "say": "PASS" }] },
        { "steps": [{ "say": "First." }], "stop": "max_tokens" },
        { "steps": [{ "say": "Second." }] },
    ] });
    let mut agent = Agent::session("when", &scenario, &json!([]));

    let (sent, answer) = agent.turn("hello");
    assert_eq!(replied(&sent), "First.");
    assert_eq!(answer["result"]["stopReason"], "max_tokens");
    let (sent, _) = agent.turn("You judge whether the goal is met");
    assert_eq!(replied(&sent), "PASS");
    let (sent, _) = agent.turn("and then?");
    assert_eq!(replied(&sent), "Second.");
    let (sent, answer) = agent.turn("and then?");
    assert_eq!(replied(&sent), "The scenario has no turn for this.");
    assert_eq!(answer["result"]["stopReason"], "end_turn");
}

#[test]
fn an_ask_goes_on_as_cox_answers() {
    let ask = json!({ "ask": {
        "toolCall": { "toolCallId": "e1", "title": "Edit src/main.rs" },
        "options": [
            { "optionId": "allow", "name": "Yes", "kind": "allow_once" },
            { "optionId": "reject", "name": "No", "kind": "reject_once" },
        ],
        "then": { "allow": [{ "say": "Edited." }], "reject": [{ "say": "Left as it was." }] },
    } });
    let scenario = json!({ "turns": [{ "steps": [ask] }, { "steps": [ask] }] });
    let mut agent = Agent::session("ask", &scenario, &json!([]));

    for (chosen, said) in [("reject", "Left as it was."), ("allow", "Edited.")] {
        let id = agent.prompt("edit it");
        let (_, request) = agent.until(|message| message["method"] == "session/request_permission");
        assert_eq!(request["params"]["toolCall"]["title"], "Edit src/main.rs");
        assert_eq!(request["params"]["sessionId"], "fake-1");
        agent.send(&json!({
            "jsonrpc": "2.0", "id": request["id"],
            "result": { "outcome": { "outcome": "selected", "optionId": chosen } },
        }));
        let (sent, answer) = agent.until(|message| message["id"] == id);
        assert_eq!(replied(&sent), said);
        assert_eq!(answer["result"]["stopReason"], "end_turn");
    }
}

#[test]
fn cancelling_ends_the_turn() {
    let scenario = json!({ "turns": [
        { "steps": [{ "say": "Working." }, { "sleep": 60_000 }, { "say": "Never said." }] },
        { "steps": [{ "say": "Still here." }] },
    ] });
    let mut agent = Agent::session("cancel", &scenario, &json!([]));

    let id = agent.prompt("go");
    agent.until(|message| message["params"]["update"]["content"]["text"] == "Working.");
    agent.send(&json!({
        "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": "fake-1" },
    }));
    let (sent, answer) = agent.until(|message| message["id"] == id);

    assert_eq!(answer["result"]["stopReason"], "cancelled");
    assert_eq!(replied(&sent), "");
    let (sent, _) = agent.turn("again");
    assert_eq!(replied(&sent), "Still here.");
}

#[test]
fn a_call_reaches_the_server_cox_handed_over() {
    // An MCP server that answers each call with the arguments it got.
    let server = r#"while read -r line; do
  id=$(printf '%s' "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"t","version":"0"}}}\n' "$id" ;;
    *'"method":"tools/call"'*)
      given=$(printf '%s' "$line" | sed 's/.*"arguments":\({[^}]*}\).*/\1/' | sed 's/"/\\"/g')
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"Chat 3 is open as chat 7, given %s"}]}}\n' "$id" "$given" ;;
  esac
done"#;
    let servers = json!([{ "name": "cox", "command": "sh", "args": ["-c", server], "env": [] }]);
    let scenario = json!({ "turns": [{ "steps": [
        { "call": { "tool": "open_chat", "arguments": { "text": "review it" } } },
        { "call": { "tool": "wait_chat", "arguments": { "chat": "{result:number after as chat }" } } },
        { "say": "It said: {result}" },
    ] }] });
    let mut agent = Agent::session("call", &scenario, &servers);

    let (sent, answer) = agent.turn("get it reviewed");

    assert_eq!(answer["result"]["stopReason"], "end_turn");
    let updates = updates(&sent);
    assert_eq!(updates[0]["sessionUpdate"], "tool_call");
    assert_eq!(updates[0]["title"], "open_chat");
    assert_eq!(updates[0]["status"], "in_progress");
    assert_eq!(updates[1]["sessionUpdate"], "tool_call_update");
    assert_eq!(updates[1]["status"], "completed");
    assert_eq!(
        updates[1]["content"][0]["content"]["text"],
        r#"Chat 3 is open as chat 7, given {"text":"review it"}"#
    );
    assert_eq!(updates[2]["rawInput"], json!({ "chat": 7 }));
    assert_eq!(
        replied(&sent),
        r#"It said: Chat 3 is open as chat 7, given {"chat":7}"#
    );
}

#[test]
fn a_session_has_the_settings_of_the_scenario_and_sets_them() {
    let model = json!({
        "id": "model", "name": "Model", "category": "model", "type": "select",
        "currentValue": "large",
        "options": [
            { "group": "big", "name": "Big", "options": [{ "value": "large", "name": "Large" }] },
            { "group": "little", "name": "Little", "options": [{ "value": "small", "name": "Small" }] },
        ],
    });
    let scenario = json!({ "configOptions": [model], "turns": [] });
    let mut agent = Agent::play("settings", &scenario);
    agent.ask("initialize", &json!({ "protocolVersion": 1 }));
    let (_, opened) = agent.ask("session/new", &json!({ "cwd": "/", "mcpServers": [] }));
    assert_eq!(opened["result"]["configOptions"], json!([model]));

    let set = |value: &str| json!({ "sessionId": "fake-1", "configId": "model", "value": value });
    let (_, answer) = agent.ask("session/set_config_option", &set("small"));
    assert_eq!(
        answer["result"]["configOptions"][0]["currentValue"],
        "small"
    );
    let (_, refused) = agent.ask("session/set_config_option", &set("huge"));
    assert!(refused["error"]["message"].is_string());
    let (_, opened) = agent.ask("session/new", &json!({ "cwd": "/", "mcpServers": [] }));
    assert_eq!(
        opened["result"]["configOptions"][0]["currentValue"],
        "small"
    );
}

#[test]
fn a_scenario_it_cannot_read_stops_it() {
    let path = scratch("broken");
    std::fs::write(&path, r#"{ "turns": [{ "steps": [{ "shout": "x" }] }] }"#).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cox-fake-agent"))
        .arg(&path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("shout"));
}

#[test]
fn the_scenarios_that_come_with_it_read() {
    let scenarios = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios");
    for entry in std::fs::read_dir(scenarios).unwrap() {
        let path = entry.unwrap().path();
        let output = Command::new(env!("CARGO_BIN_EXE_cox-fake-agent"))
            .arg(&path)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
