//! The agent's side of ACP, as a scenario plays it: it answers
//! `initialize` and `session/new`, with the scenario's settings, sets them
//! as `session/set_config_option` asks, takes a turn of the scenario for
//! each `session/prompt`, and ends the turn early when Cox cancels it.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead as _, Write};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::mcp::Server;
use crate::scenario::{Ask, Call, Scenario, Step, fill, put_in};

/// How long the agent takes between the words it says.
const WORD: Duration = Duration::from_millis(30);

/// JSON-RPC's code for a method the agent does not know.
const METHOD_NOT_FOUND: i64 = -32601;

/// JSON-RPC's code for a setting or value the scenario does not have.
const INVALID_PARAMS: i64 = -32602;

/// JSON-RPC's code for a turn the scenario fails.
const INTERNAL_ERROR: i64 = -32603;

/// How a turn's steps went.
enum Went {
    On,
    Cancelled,
    Failed(String),
    /// Cox went, and the agent goes with it.
    Gone,
}

pub struct Agent<W: Write> {
    scenario: Scenario,
    /// The turns of the scenario taken so far.
    taken: Vec<bool>,
    output: W,
    input: Receiver<Value>,
    /// What Cox sent while the agent waited for something else.
    held: VecDeque<Value>,
    /// The MCP servers `session/new` handed the agent, by their names.
    servers: HashMap<String, Value>,
    started: HashMap<String, Server>,
    /// What the last call answered.
    result: String,
    /// The folder the session works in, as Cox gave it.
    cwd: String,
    sessions: u64,
    asked: u64,
    calls: u64,
}

/// Plays `scenario` on standard input and output until Cox goes.
pub fn play(scenario: Scenario) {
    let (sent, input) = channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<Value>(&line) {
                Ok(message) => {
                    if sent.send(message).is_err() {
                        break;
                    }
                }
                Err(error) => eprintln!("cox-fake-agent: cannot read {line:?}: {error}"),
            }
        }
    });
    Agent::new(scenario, io::stdout(), input).run();
}

impl<W: Write> Agent<W> {
    pub fn new(scenario: Scenario, output: W, input: Receiver<Value>) -> Self {
        let taken = vec![false; scenario.turns.len()];
        Self {
            scenario,
            taken,
            output,
            input,
            held: VecDeque::new(),
            servers: HashMap::new(),
            started: HashMap::new(),
            result: String::new(),
            cwd: String::new(),
            sessions: 0,
            asked: 0,
            calls: 0,
        }
    }

    /// Answers what Cox sends until it goes.
    pub fn run(&mut self) {
        while let Some(message) = self.next() {
            let id = message.get("id").cloned();
            let method = message["method"].as_str().unwrap_or_default();
            match (method, id) {
                ("initialize", Some(id)) => self.answer(
                    &id,
                    &json!({ "protocolVersion": 1, "agentCapabilities": {} }),
                ),
                ("session/new", Some(id)) => {
                    message["params"]["cwd"]
                        .as_str()
                        .unwrap_or_default()
                        .clone_into(&mut self.cwd);
                    self.servers = message["params"]["mcpServers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|server| {
                            Some((server["name"].as_str()?.to_owned(), server.clone()))
                        })
                        .collect();
                    self.sessions += 1;
                    let session = format!("fake-{}", self.sessions);
                    let mut answer = json!({ "sessionId": session });
                    if !self.scenario.config_options.is_empty() {
                        answer["configOptions"] = Value::from(self.scenario.config_options.clone());
                    }
                    self.answer(&id, &answer);
                }
                ("session/set_config_option", Some(id)) => self.set(&id, &message["params"]),
                ("session/prompt", Some(id)) => {
                    if !self.prompt(&id, &message["params"]) {
                        return;
                    }
                }
                ("", _) | (_, None) => {}
                (method, Some(id)) => {
                    let error = format!("cox-fake-agent knows no {method}");
                    self.fail(&id, METHOD_NOT_FOUND, &error);
                }
            }
        }
    }

    /// Sets the setting `params` names to the value it says, and answers
    /// `id` with all the settings, or with an error for a setting or value
    /// the scenario does not have.
    fn set(&mut self, id: &Value, params: &Value) {
        let (setting, value) = (&params["configId"], &params["value"]);
        let option = self
            .scenario
            .config_options
            .iter_mut()
            .find(|option| option["id"] == *setting);
        let Some(option) = option else {
            self.fail(
                id,
                INVALID_PARAMS,
                &format!("there is no setting {setting}"),
            );
            return;
        };
        let offered = option["options"]
            .as_array()
            .into_iter()
            .flatten()
            // Grouped, a choice is in a group's options.
            .flat_map(|choice| {
                choice["options"]
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![choice.clone()])
            })
            .any(|choice| choice["value"] == *value);
        if !offered {
            self.fail(id, INVALID_PARAMS, &format!("{setting} cannot be {value}"));
            return;
        }
        option["currentValue"] = value.clone();
        let options = Value::from(self.scenario.config_options.clone());
        self.answer(id, &json!({ "configOptions": options }));
    }

    /// Takes the turn that answers the prompt `params`, and answers `id`
    /// as it ends. Answers whether Cox is still there.
    fn prompt(&mut self, id: &Value, params: &Value) -> bool {
        let session = params["sessionId"].clone();
        let text = params["prompt"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let turn = (0..self.scenario.turns.len())
            .find(|&turn| !self.taken[turn] && self.scenario.turns[turn].answers(&text));
        let Some(turn) = turn else {
            self.say(&session, "The scenario has no turn for this.");
            self.answer(id, &json!({ "stopReason": "end_turn" }));
            return true;
        };
        self.taken[turn] = true;
        let steps = std::mem::take(&mut self.scenario.turns[turn].steps);
        let went = self.steps(&session, &steps);
        self.scenario.turns[turn].steps = steps;
        match went {
            Went::On => {
                let stop = self.scenario.turns[turn].stop.clone();
                self.answer(id, &json!({ "stopReason": stop }));
            }
            Went::Cancelled => self.answer(id, &json!({ "stopReason": "cancelled" })),
            Went::Failed(error) => self.fail(id, INTERNAL_ERROR, &error),
            Went::Gone => return false,
        }
        true
    }

    fn steps(&mut self, session: &Value, steps: &[Step]) -> Went {
        for step in steps {
            if self.cancelled() {
                return Went::Cancelled;
            }
            let went = self.step(session, step);
            if !matches!(went, Went::On) {
                return went;
            }
        }
        Went::On
    }

    fn step(&mut self, session: &Value, step: &Step) -> Went {
        let result = self.result.clone();
        let cwd = self.cwd.clone();
        let text = |text: &String| put_in(text, &cwd, &result);
        match step {
            Step::Say(said) => return self.say(session, &text(said)),
            Step::Think(thought) => self.update(
                session,
                &json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": text(thought) } }),
            ),
            Step::Tool(fields) => {
                self.update(session, &tool("tool_call", fields, &cwd, &result));
            }
            Step::ToolUpdate(fields) => {
                self.update(session, &tool("tool_call_update", fields, &cwd, &result));
            }
            Step::Update(update) => self.update(session, &fill(update, &cwd, &result)),
            Step::Ask(ask) => return self.ask(session, ask),
            Step::Call(call) => self.call(session, call),
            Step::Sleep(milliseconds) => return self.pause(Duration::from_millis(*milliseconds)),
            Step::Stderr(line) => eprintln!("{}", text(line)),
            Step::Fail(error) => return Went::Failed(text(error)),
        }
        Went::On
    }

    /// Says `text` a word at a time.
    fn say(&mut self, session: &Value, text: &str) -> Went {
        let words: Vec<&str> = text.split_inclusive(' ').collect();
        for (number, word) in words.iter().enumerate() {
            if number > 0 {
                let went = self.pause(WORD);
                if !matches!(went, Went::On) {
                    return went;
                }
            }
            self.update(
                session,
                &json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": word } }),
            );
        }
        Went::On
    }

    fn ask(&mut self, session: &Value, ask: &Ask) -> Went {
        self.asked += 1;
        let id = Value::from(format!("fake-ask-{}", self.asked));
        let mut params = match fill(&Value::Object(ask.request.clone()), &self.cwd, &self.result) {
            Value::Object(params) => params,
            _ => Map::new(),
        };
        params.insert("sessionId".to_owned(), session.clone());
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": params }));
        // What Cox chose, by its option's id, or `cancelled`.
        let chosen = loop {
            let answered = self
                .held
                .iter()
                .position(|message| message.get("method").is_none() && message["id"] == id);
            let message = match answered {
                Some(at) => self.held.remove(at).expect("found above"),
                None => match self.input.recv() {
                    Ok(message) => message,
                    Err(_) => return Went::Gone,
                },
            };
            if message.get("method").is_some() || message["id"] != id {
                // A cancel comes with Cox's answer to this request too.
                if message["method"] != "session/cancel" {
                    self.held.push_back(message);
                }
                continue;
            }
            let outcome = &message["result"]["outcome"];
            break match outcome["outcome"].as_str() {
                Some("selected") => outcome["optionId"].as_str().unwrap_or_default().to_owned(),
                _ => "cancelled".to_owned(),
            };
        };
        let went = match ask.then.get(&chosen) {
            Some(steps) => self.steps(session, steps),
            None => Went::On,
        };
        match went {
            Went::On if chosen == "cancelled" => Went::Cancelled,
            went => went,
        }
    }

    /// Calls a tool, showing the call as a real agent does, and keeps what
    /// it answered for `{result}`.
    fn call(&mut self, session: &Value, call: &Call) {
        self.calls += 1;
        let id = format!("fake-call-{}", self.calls);
        let arguments = fill(&call.arguments, &self.cwd, &self.result);
        self.update(
            session,
            &json!({
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "title": call.title.as_ref().unwrap_or(&call.tool),
                "kind": "other",
                "status": "in_progress",
                "rawInput": arguments,
            }),
        );
        let answered = self
            .server(&call.server)
            .and_then(|server| server.call(&call.tool, &arguments));
        let (status, text) = match answered {
            Ok(Ok(text)) => ("completed", text),
            Ok(Err(text)) | Err(text) => ("failed", text),
        };
        self.update(
            session,
            &json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": status,
                "content": [{ "type": "content", "content": { "type": "text", "text": text } }],
            }),
        );
        self.result = text;
    }

    /// The server Cox named `name`, started the first time it is called.
    fn server(&mut self, name: &str) -> Result<&mut Server, String> {
        if !self.started.contains_key(name) {
            let server = self
                .servers
                .get(name)
                .ok_or_else(|| format!("Cox handed the agent no server {name}"))?;
            let started = Server::start(server)?;
            self.started.insert(name.to_owned(), started);
        }
        Ok(self.started.get_mut(name).expect("started above"))
    }

    /// Waits `time`, unless Cox cancels the turn or goes first.
    fn pause(&mut self, time: Duration) -> Went {
        let until = Instant::now() + time;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Went::On;
            }
            match self.input.recv_timeout(left) {
                Err(RecvTimeoutError::Timeout) => return Went::On,
                Err(RecvTimeoutError::Disconnected) => return Went::Gone,
                Ok(message) if message["method"] == "session/cancel" => return Went::Cancelled,
                Ok(message) => self.held.push_back(message),
            }
        }
    }

    /// Whether Cox has cancelled the turn, among what it sent so far.
    fn cancelled(&mut self) -> bool {
        while let Ok(message) = self.input.try_recv() {
            self.held.push_back(message);
        }
        let at = self
            .held
            .iter()
            .position(|message| message["method"] == "session/cancel");
        at.and_then(|at| self.held.remove(at)).is_some()
    }

    /// The next of what Cox sent, held or new, or `None` once it went.
    fn next(&mut self) -> Option<Value> {
        self.held.pop_front().or_else(|| self.input.recv().ok())
    }

    fn update(&mut self, session: &Value, update: &Value) {
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": session, "update": update },
        }));
    }

    fn answer(&mut self, id: &Value, result: &Value) {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    fn fail(&mut self, id: &Value, code: i64, message: &str) {
        self.send(
            &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        );
    }

    fn send(&mut self, message: &Value) {
        // Cox gone, the agent learns it from its input.
        let _ = writeln!(self.output, "{message}").and_then(|()| self.output.flush());
    }
}

/// ACP's update `kind` of a tool call, from the step's `fields`, with
/// `toolCallId` as `id`.
fn tool(kind: &str, fields: &Map<String, Value>, cwd: &str, result: &str) -> Value {
    let mut update = Map::new();
    update.insert("sessionUpdate".to_owned(), Value::from(kind));
    for (key, value) in fields {
        let key = if key == "id" { "toolCallId" } else { key };
        update.insert(key.to_owned(), fill(value, cwd, result));
    }
    Value::Object(update)
}
