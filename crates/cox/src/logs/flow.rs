//! The flow of one tool call through Cox, read from what Cox kept of the
//! run it was made in (see [`crate::logs`]), for the human to see how the
//! parts of Cox pass it on before they read the code. The agent says over
//! ACP that it calls a tool; `cox --mcp` passes its `tools/call` on to the
//! chat's tool server, on a thread of its own; the tool asks the stage,
//! on the window's thread, to show something, and the stage carries it
//! out; the server answers the agent; and the agent says the call is
//! done. Each step comes with where in Cox's code it is taken: the file
//! and line the log line was said at.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::Timestamp;
use serde_json::Value;

use crate::agent::mcp::{self, ASKED};

/// How long after a tool asks the stage its carrying the request out
/// still counts as this call's.
const CARRIED_WITHIN: Duration = Duration::from_secs(10);

/// How many characters of what was said a step shows at most.
const SAID: usize = 120;

/// Where a step of the flow is taken: a part of Cox, or the agent, at work
/// in a process or thread of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    /// The agent's process, and the `cox --mcp` it runs to reach Cox.
    Agent,
    /// Cox's thread for the agent, which speaks ACP with it.
    Acp,
    /// The chat's server of Cox's own tools, on a thread of its own.
    Tools,
    /// The window's thread, where the stage and the conversation are.
    Window,
}

impl Lane {
    pub const ALL: [Self; 4] = [Self::Agent, Self::Acp, Self::Tools, Self::Window];

    /// Its name, as the heading of its lane.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Acp => "agent thread",
            Self::Tools => "tools thread",
            Self::Window => "window",
        }
    }

    /// What it is, in a few words under the name.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::Agent => "the agent's process, and cox --mcp",
            Self::Acp => "Cox speaks ACP with the agent",
            Self::Tools => "the chat's MCP server, in Cox",
            Self::Window => "the stage and the conversation",
        }
    }
}

/// Where in Cox's code a step is taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// The file, as Cox was built: from the workspace, as
    /// `crates/cox/src/stage.rs`.
    pub file: PathBuf,
    pub line: u64,
}

/// One step of the flow: something said or done, in a lane, that the
/// step before, in its lane, led to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// How long after the first step it was taken.
    pub after: Duration,
    pub from: Lane,
    pub to: Lane,
    /// What was said or done, in a few words.
    pub said: String,
    pub source: Option<Source>,
}

/// The flow of one tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    /// The call's id, as the agent gave it over ACP.
    pub call: String,
    /// What the agent called the call, if it said.
    pub title: Option<String>,
    /// The file of the run the call was made in.
    pub run: PathBuf,
    /// The steps, in order.
    pub steps: Vec<Step>,
}

/// A line Cox kept, as far as the flow reads it.
struct Kept {
    at: Timestamp,
    target: String,
    /// Whose line it is, for a line sent over a wire: `cox` or `agent`.
    from: Option<String>,
    message: String,
    /// The id of the chat it was said in, when it was said in one.
    chat: Option<String>,
    source: Option<Source>,
}

/// A step as it is gathered, before the steps are put in order.
struct Gathered {
    at: Timestamp,
    from: Lane,
    to: Lane,
    said: String,
    source: Option<Source>,
}

/// Reads the flow of the tool call `call` from the files of the runs in
/// `logs`, newest first: from the newest run that names the call.
///
/// # Errors
///
/// When the files cannot be read, or none of them names the call.
pub fn read(logs: &Path, call: &str) -> Result<Flow, String> {
    for run in runs(logs)? {
        let file =
            File::open(&run).map_err(|error| format!("cannot read {}: {error}", run.display()))?;
        let lines = BufReader::new(file).lines().map_while(Result::ok);
        if let Some(flow) = of_run(lines, run.clone(), call) {
            return Ok(flow);
        }
    }
    Err(format!(
        "no run Cox kept in {} names the call {call}",
        logs.display()
    ))
}

/// The files of the runs in `logs`, newest first: their names start with
/// the time the run started.
fn runs(logs: &Path) -> Result<Vec<PathBuf>, String> {
    let mut runs: Vec<PathBuf> = fs::read_dir(logs)
        .map_err(|error| format!("cannot list {}: {error}", logs.display()))?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    runs.sort();
    runs.reverse();
    Ok(runs)
}

/// The call's own lines, over ACP, as the flow reads them first: they
/// say which chat the call was made in, and when it started and ended.
struct Called {
    steps: Vec<Gathered>,
    title: Option<String>,
    /// What the agent said it calls the tool with, ACP's `rawInput`, as
    /// it last said it.
    input: Option<Value>,
    /// The id of the chat, when the lines were said in one.
    chat: Option<String>,
    started: Timestamp,
    /// When the agent last said of the call; `None` while it goes on.
    ended: Option<Timestamp>,
}

/// The flow of the tool call `call` in the run kept in `lines`, those of
/// the file `run`, if the run names the call.
fn of_run(lines: impl Iterator<Item = String>, run: PathBuf, call: &str) -> Option<Flow> {
    let targets = [cox_acp::WIRE, mcp::WIRE, mcp::ASKS, ASKED];
    let kept: Vec<Kept> = lines
        .filter(|line| targets.iter().any(|target| line.contains(target)))
        .filter_map(|line| parse(&line))
        .collect();
    let mut called = over_acp(&kept, call)?;
    let mut gathered = std::mem::take(&mut called.steps);
    let in_chat = |line: &Kept| called.chat.is_none() || line.chat == called.chat;
    let (calls, answered) = over_socket(&kept, &called, in_chat, &mut gathered);
    if let Some(called_at) = calls {
        let until = answered.or(called.ended);
        of_stage(&kept, called_at, until, in_chat, &mut gathered);
    }

    gathered.sort_by_key(|step| step.at);
    let first = gathered.first()?.at;
    let steps = gathered
        .into_iter()
        .map(|step| Step {
            after: step.at.duration_since(first).unsigned_abs(),
            from: step.from,
            to: step.to,
            said: step.said,
            source: step.source,
        })
        .collect();
    Some(Flow {
        call: call.to_owned(),
        title: called.title,
        run,
        steps,
    })
}

/// The steps of the call `call` over ACP, from the lines `kept`: what the
/// agent says of the call, its permission requests for it, and Cox's
/// answers to those.
fn over_acp(kept: &[Kept], call: &str) -> Option<Called> {
    let mut steps = Vec::new();
    let mut title = None;
    let mut input = None;
    let mut chat = None;
    let (mut started, mut ended) = (None, None);
    // The ids of the agent's permission requests for the call, which Cox
    // answers by id.
    let mut permissions = Vec::new();
    for line in kept.iter().filter(|line| line.target == cox_acp::WIRE) {
        let Ok(message) = serde_json::from_str::<Value>(&line.message) else {
            continue;
        };
        let step = match line.from.as_deref() {
            Some("agent") => acp_step(&message, call, &mut title, &mut input, &mut permissions),
            Some("cox") => permission_answered(&message, &permissions),
            _ => None,
        };
        if let Some((from, to, said)) = step {
            chat.get_or_insert_with(|| line.chat.clone());
            started.get_or_insert(line.at);
            ended = Some(line.at);
            steps.push(Gathered {
                at: line.at,
                from,
                to,
                said,
                source: line.source.clone(),
            });
        }
    }
    Some(Called {
        steps,
        title,
        input,
        chat: chat.flatten(),
        started: started?,
        ended,
    })
}

/// Gathers the steps of `called` over the socket from the lines `kept` in
/// its chat, into `gathered`: the `tools/call` within the call's time
/// whose arguments are what the agent said it calls the tool with, or any
/// when it did not say, and the server's answer. Answers when the tool was first called, if it was,
/// and when it was last answered.
fn over_socket(
    kept: &[Kept],
    called: &Called,
    in_chat: impl Fn(&Kept) -> bool,
    gathered: &mut Vec<Gathered>,
) -> (Option<Timestamp>, Option<Timestamp>) {
    let mut calls = Vec::new();
    let (mut first_called, mut answered) = (None, None);
    for line in kept
        .iter()
        .filter(|line| line.target == mcp::WIRE && in_chat(line))
    {
        let Ok(message) = serde_json::from_str::<Value>(&line.message) else {
            continue;
        };
        let in_time =
            line.at >= called.started && called.ended.is_none_or(|ended| line.at <= ended);
        match line.from.as_deref() {
            Some("agent") if in_time => {
                if let Some((id, said)) = tool_called(&message, called.input.as_ref()) {
                    calls.push(id);
                    first_called.get_or_insert(line.at);
                    gathered.push(Gathered {
                        at: line.at,
                        from: Lane::Agent,
                        to: Lane::Tools,
                        said,
                        source: line.source.clone(),
                    });
                }
            }
            Some("cox") if calls.contains(&message["id"]) => {
                answered = Some(line.at);
                gathered.push(Gathered {
                    at: line.at,
                    from: Lane::Tools,
                    to: Lane::Agent,
                    said: tool_answered(&message),
                    source: line.source.clone(),
                });
            }
            _ => {}
        }
    }
    (first_called, answered)
}

/// Gathers into `gathered` what the tool asked of the stage from `called`
/// until `until`, from the lines `kept` in its chat, and the stage
/// carrying each request out, when it did within [`CARRIED_WITHIN`].
fn of_stage(
    kept: &[Kept],
    called: Timestamp,
    until: Option<Timestamp>,
    in_chat: impl Fn(&Kept) -> bool,
    gathered: &mut Vec<Gathered>,
) {
    let asks = kept.iter().filter(|line| {
        line.target == mcp::ASKS
            && in_chat(line)
            && line.at >= called
            && until.is_none_or(|until| line.at <= until)
    });
    for ask in asks {
        gathered.push(Gathered {
            at: ask.at,
            from: Lane::Tools,
            to: Lane::Window,
            said: ask.message.clone(),
            source: ask.source.clone(),
        });
        let asked = ask
            .message
            .strip_prefix("asks the stage to ")
            .unwrap_or(&ask.message);
        let carried = kept.iter().find(|line| {
            line.target == ASKED
                && in_chat(line)
                && line.at >= ask.at
                && within(ask.at, line.at, CARRIED_WITHIN)
                && line.message.ends_with(asked)
        });
        if let Some(carried) = carried {
            gathered.push(Gathered {
                at: carried.at,
                from: Lane::Window,
                to: Lane::Window,
                said: carried.message.clone(),
                source: carried.source.clone(),
            });
        }
    }
}

/// Whether `later` is within `limit` after `earlier`.
fn within(earlier: Timestamp, later: Timestamp, limit: Duration) -> bool {
    later.duration_since(earlier).unsigned_abs() <= limit
}

/// `line`, one Cox kept, as far as the flow reads it.
fn parse(line: &str) -> Option<Kept> {
    let kept: Value = serde_json::from_str(line).ok()?;
    if said_again_whole(&kept) {
        return None;
    }
    let at: Timestamp = kept["timestamp"].as_str()?.parse().ok()?;
    let fields = &kept["fields"];
    let chat = kept["spans"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|span| span["name"] == "chat")
        .map(|span| span["id"].to_string());
    let source = match (kept["filename"].as_str(), kept["line_number"].as_u64()) {
        (Some(file), Some(line)) => Some(Source {
            file: PathBuf::from(file),
            line,
        }),
        _ => None,
    };
    Some(Kept {
        at,
        target: kept["target"].as_str()?.to_owned(),
        from: fields["from"].as_str().map(str::to_owned),
        message: fields["message"].as_str()?.to_owned(),
        chat,
        source,
    })
}

/// Whether `kept` is the whole of a message over the tools' socket,
/// which is said in brief at `debug` too.
fn said_again_whole(kept: &Value) -> bool {
    kept["target"] == mcp::WIRE && kept["level"] == "TRACE"
}

/// The step `message`, which the agent sent over ACP, is of the call
/// `call`, if it is about it: what it says of the call, or a permission
/// request for it, whose id goes to `permissions`. The call's title, when
/// the agent gives it, goes to `title`, and what it calls the tool with,
/// its `rawInput`, to `input`, which the agent may say again once the
/// input is whole.
fn acp_step(
    message: &Value,
    call: &str,
    title: &mut Option<String>,
    input: &mut Option<Value>,
    permissions: &mut Vec<Value>,
) -> Option<(Lane, Lane, String)> {
    let params = &message["params"];
    let update = &params["update"];
    if update["toolCallId"] == call {
        if let Some(given) = update["title"].as_str() {
            title.get_or_insert_with(|| given.to_owned());
        }
        if !update["rawInput"].is_null() {
            *input = Some(update["rawInput"].clone());
        }
        let status = update["status"].as_str();
        let said = match (update["sessionUpdate"].as_str(), status) {
            (Some("tool_call"), Some(status)) => {
                format!(
                    "the agent says it calls `{}`, {status}",
                    named(title.as_deref())
                )
            }
            (Some("tool_call"), None) => {
                format!("the agent says it calls `{}`", named(title.as_deref()))
            }
            (_, Some(status)) => format!("the agent says the call is {status}"),
            (_, None) => "the agent says more of the call".to_owned(),
        };
        return Some((Lane::Agent, Lane::Acp, said));
    }
    if message["method"] == "session/request_permission" && params["toolCall"]["toolCallId"] == call
    {
        permissions.push(message["id"].clone());
        let what = params["toolCall"]["title"]
            .as_str()
            .map_or_else(|| named(title.as_deref()), str::to_owned);
        return Some((
            Lane::Agent,
            Lane::Acp,
            format!("the agent asks permission for `{what}`"),
        ));
    }
    None
}

/// The step `message`, which Cox sent over ACP, is, if it answers one of
/// the `permissions` asked for the call.
fn permission_answered(message: &Value, permissions: &[Value]) -> Option<(Lane, Lane, String)> {
    if message["id"].is_null() || !permissions.contains(&message["id"]) {
        return None;
    }
    let outcome = &message["result"]["outcome"];
    let said = match outcome["optionId"].as_str() {
        Some(option) => format!("Cox answers it: {option}"),
        None => format!(
            "Cox answers it: {}",
            outcome["outcome"].as_str().unwrap_or("nothing")
        ),
    };
    Some((Lane::Acp, Lane::Agent, said))
}

/// What the agent called the call, or "a tool" until it says.
fn named(title: Option<&str>) -> String {
    title.unwrap_or("a tool").to_owned()
}

/// The id and the step of `message`, if it calls a tool over the socket
/// with `input`, what the agent said it calls the tool with: any tool,
/// with anything, when it did not say. No arguments are taken as an
/// empty object, as MCP takes them.
fn tool_called(message: &Value, input: Option<&Value>) -> Option<(Value, String)> {
    if message["method"] != "tools/call" {
        return None;
    }
    let name = message["params"]["name"].as_str()?;
    let arguments = &message["params"]["arguments"];
    let none = Value::Object(serde_json::Map::new());
    let given = if arguments.is_null() {
        &none
    } else {
        arguments
    };
    if input.is_some_and(|input| mcp::in_brief(input) != mcp::in_brief(given)) {
        return None;
    }
    let said = if arguments.is_null() {
        format!("`cox --mcp` passes on `tools/call` {name}")
    } else {
        format!(
            "`cox --mcp` passes on `tools/call` {name} {}",
            cut(&arguments.to_string())
        )
    };
    Some((message["id"].clone(), said))
}

/// The step `message` is, the server's answer to a tool call.
fn tool_answered(message: &Value) -> String {
    if let Some(error) = message["error"]["message"].as_str() {
        return format!("the server answers an error: {}", cut(error));
    }
    let result = &message["result"];
    let text = result["content"][0]["text"].as_str().unwrap_or("");
    if result["isError"] == true {
        format!("the tool answers an error: {}", cut(text))
    } else {
        format!("the tool answers: {}", cut(text))
    }
}

/// The first line of `text`, up to [`SAID`] characters.
fn cut(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    let mut cut: String = line.chars().take(SAID).collect();
    if cut.len() < line.len() || text.lines().nth(1).is_some() {
        cut.push('…');
    }
    cut
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A line as Cox keeps it, said at `at` seconds into the run.
    fn line(at: f64, target: &str, from: Option<&str>, message: &str, file: &str) -> String {
        let mut fields = json!({ "message": message });
        if let Some(from) = from {
            fields["from"] = json!(from);
        }
        json!({
            "timestamp": format!("2026-11-02T08:00:{at:09.6}Z"),
            "level": "DEBUG",
            "fields": fields,
            "target": target,
            "filename": file,
            "line_number": 42,
            "spans": [{ "cwd": "/work", "id": "3", "name": "chat" }],
        })
        .to_string()
    }

    fn acp(at: f64, from: &str, message: &Value) -> String {
        line(
            at,
            cox_acp::WIRE,
            Some(from),
            &message.to_string(),
            "crates/cox-acp/src/lib.rs",
        )
    }

    fn wire(at: f64, from: &str, message: &Value) -> String {
        line(
            at,
            mcp::WIRE,
            Some(from),
            &message.to_string(),
            "crates/cox/src/agent/mcp.rs",
        )
    }

    fn update(id: &str, fields: Value) -> Value {
        let mut update = fields;
        update["toolCallId"] = json!(id);
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": { "sessionId": "s", "update": update },
        })
    }

    fn a_run() -> Vec<String> {
        vec![
            acp(
                1.0,
                "agent",
                &update(
                    "call-1",
                    json!({ "sessionUpdate": "tool_call", "title": "Show the map",
                        "status": "pending", "rawInput": {} }),
                ),
            ),
            // The input, once it is whole.
            acp(
                1.0005,
                "agent",
                &update(
                    "call-1",
                    json!({ "sessionUpdate": "tool_call_update", "rawInput": { "at": ["cox"] } }),
                ),
            ),
            // Another call, at the same time, which is not this one's.
            wire(
                1.001,
                "agent",
                &json!({ "jsonrpc": "2.0", "id": 8, "method": "tools/call",
                    "params": { "name": "show_file", "arguments": { "path": "a.rs" } } }),
            ),
            wire(
                1.002,
                "agent",
                &json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                    "params": { "name": "show_map", "arguments": { "at": ["cox"] } } }),
            ),
            line(
                1.003,
                mcp::ASKS,
                None,
                "asks the stage to show the map at cox",
                "crates/cox/src/agent/mcp/calls.rs",
            ),
            wire(
                1.004,
                "cox",
                &json!({ "jsonrpc": "2.0", "id": 9, "result": {
                    "content": [{ "type": "text", "text": "The map is on the stage:\nstage: 7 lines" }],
                    "isError": false } }),
            ),
            wire(
                1.005,
                "cox",
                &json!({ "jsonrpc": "2.0", "id": 8, "result": { "content": [], "isError": false } }),
            ),
            line(
                1.010,
                ASKED,
                None,
                "the stage carries it out: show the map at cox",
                "crates/cox/src/stage/asked.rs",
            ),
            acp(
                1.020,
                "agent",
                &update(
                    "call-1",
                    json!({ "sessionUpdate": "tool_call_update", "status": "completed" }),
                ),
            ),
            line(
                2.0,
                "cox::other",
                None,
                "something else",
                "crates/cox/src/app.rs",
            ),
        ]
    }

    #[test]
    fn follows_a_call_from_the_agent_to_the_stage_and_back() {
        let flow = of_run(a_run().into_iter(), PathBuf::from("run.jsonl"), "call-1").unwrap();

        assert_eq!(flow.title.as_deref(), Some("Show the map"));
        let steps: Vec<(u128, Lane, Lane, &str)> = flow
            .steps
            .iter()
            .map(|step| {
                (
                    step.after.as_millis(),
                    step.from,
                    step.to,
                    step.said.as_str(),
                )
            })
            .collect();
        assert_eq!(
            steps,
            [
                (
                    0,
                    Lane::Agent,
                    Lane::Acp,
                    "the agent says it calls `Show the map`, pending"
                ),
                (0, Lane::Agent, Lane::Acp, "the agent says more of the call"),
                (
                    2,
                    Lane::Agent,
                    Lane::Tools,
                    r#"`cox --mcp` passes on `tools/call` show_map {"at":["cox"]}"#
                ),
                (
                    3,
                    Lane::Tools,
                    Lane::Window,
                    "asks the stage to show the map at cox"
                ),
                (
                    4,
                    Lane::Tools,
                    Lane::Agent,
                    "the tool answers: The map is on the stage:…"
                ),
                (
                    10,
                    Lane::Window,
                    Lane::Window,
                    "the stage carries it out: show the map at cox"
                ),
                (
                    20,
                    Lane::Agent,
                    Lane::Acp,
                    "the agent says the call is completed"
                ),
            ]
        );
        assert_eq!(
            flow.steps[3].source,
            Some(Source {
                file: PathBuf::from("crates/cox/src/agent/mcp/calls.rs"),
                line: 42
            })
        );
    }

    #[test]
    fn a_call_that_says_no_input_takes_the_tool_calls_in_its_time() {
        let lines = vec![
            acp(
                1.0,
                "agent",
                &update(
                    "call-3",
                    json!({ "sessionUpdate": "tool_call", "title": "Show the file" }),
                ),
            ),
            wire(
                1.001,
                "agent",
                &json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                    "params": { "name": "show_file" } }),
            ),
            acp(
                1.002,
                "agent",
                &update(
                    "call-3",
                    json!({ "sessionUpdate": "tool_call_update", "status": "completed" }),
                ),
            ),
            // After the call, so not its.
            wire(
                1.003,
                "agent",
                &json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                    "params": { "name": "show_map" } }),
            ),
        ];
        let flow = of_run(lines.into_iter(), PathBuf::from("run.jsonl"), "call-3").unwrap();
        let said: Vec<&str> = flow.steps.iter().map(|step| step.said.as_str()).collect();
        assert_eq!(
            said,
            [
                "the agent says it calls `Show the file`",
                "`cox --mcp` passes on `tools/call` show_file",
                "the agent says the call is completed",
            ]
        );
    }

    #[test]
    fn a_call_said_in_brief_over_the_socket_is_still_the_calls() {
        let page = "a line of the page\n".repeat(1000);
        let called = json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": { "name": "wiki_write", "arguments": { "path": "notes.md", "text": page } } });
        let whole = wire(1.001, "agent", &called).replace(r#""DEBUG""#, r#""TRACE""#);
        let lines = vec![
            acp(
                1.0,
                "agent",
                &update(
                    "call-4",
                    json!({ "sessionUpdate": "tool_call", "title": "Write notes.md",
                        "rawInput": { "path": "notes.md", "text": page } }),
                ),
            ),
            whole,
            wire(1.001, "agent", &mcp::in_brief(&called)),
            acp(
                1.002,
                "agent",
                &update(
                    "call-4",
                    json!({ "sessionUpdate": "tool_call_update", "status": "completed" }),
                ),
            ),
        ];

        let flow = of_run(lines.into_iter(), PathBuf::from("run.jsonl"), "call-4").unwrap();

        let said: Vec<&str> = flow.steps.iter().map(|step| step.said.as_str()).collect();
        assert_eq!(said.len(), 3, "{said:?}");
        assert!(
            said[1].starts_with("`cox --mcp` passes on `tools/call` wiki_write "),
            "{}",
            said[1]
        );
    }

    #[test]
    fn a_permission_request_and_its_answer_are_steps() {
        let lines = vec![
            acp(
                1.0,
                "agent",
                &update(
                    "call-2",
                    json!({ "sessionUpdate": "tool_call", "title": "Edit src/main.rs" }),
                ),
            ),
            acp(
                1.1,
                "agent",
                &json!({ "jsonrpc": "2.0", "id": 5, "method": "session/request_permission",
                    "params": { "sessionId": "s",
                        "toolCall": { "toolCallId": "call-2", "title": "Edit src/main.rs" },
                        "options": [] } }),
            ),
            acp(
                3.0,
                "cox",
                &json!({ "jsonrpc": "2.0", "id": 5,
                    "result": { "outcome": { "outcome": "selected", "optionId": "allow" } } }),
            ),
        ];

        let flow = of_run(lines.into_iter(), PathBuf::from("run.jsonl"), "call-2").unwrap();

        let said: Vec<&str> = flow.steps.iter().map(|step| step.said.as_str()).collect();
        assert_eq!(
            said,
            [
                "the agent says it calls `Edit src/main.rs`",
                "the agent asks permission for `Edit src/main.rs`",
                "Cox answers it: allow",
            ]
        );
        assert_eq!(flow.steps[2].from, Lane::Acp);
        assert_eq!(flow.steps[2].after, Duration::from_secs(2));
    }

    #[test]
    fn a_run_that_does_not_name_the_call_has_no_flow() {
        assert!(of_run(a_run().into_iter(), PathBuf::from("run.jsonl"), "call-9").is_none());
    }

    #[test]
    fn reads_the_newest_run_that_names_the_call() {
        let directory = std::env::temp_dir().join(format!("cox-flow-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("20261101T100000Z-1.jsonl"),
            a_run().join("\n"),
        )
        .unwrap();
        fs::write(directory.join("20261101T110000Z-2.jsonl"), "").unwrap();

        let flow = read(&directory, "call-1").unwrap();
        assert_eq!(flow.run, directory.join("20261101T100000Z-1.jsonl"));
        assert_eq!(flow.steps.len(), 7);
        assert!(read(&directory, "call-9").is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn what_is_said_is_cut_to_its_first_line() {
        assert_eq!(cut("one\ntwo"), "one…");
        assert_eq!(cut("one"), "one");
        let long = "x".repeat(SAID + 1);
        assert_eq!(cut(&long).chars().count(), SAID + 1);
    }
}
