//! What a scenario says the agent does, read from JSON:
//!
//! ```json
//! {
//!   "turns": [
//!     { "steps": [{ "say": "Hello." }] },
//!     { "when": "edit", "steps": [{ "think": "..." }, { "tool": { ... } }] }
//!   ]
//! }
//! ```
//!
//! Each line sent to the agent takes the first turn not taken yet whose
//! `when`, if it has one, is in what the line says, and the agent takes its
//! steps one after another, then ends the turn with its `stop`, `end_turn`
//! unless it says otherwise. A step is an object with one key, which says
//! what it is (see [`Step`]).

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub turns: Vec<Turn>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    /// Text the line sent has to have for the turn to answer it.
    pub when: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// The turn's `stopReason`.
    #[serde(default = "end_turn")]
    pub stop: String,
}

fn end_turn() -> String {
    "end_turn".to_owned()
}

/// One thing the agent does in a turn. In the text of a step, `{cwd}`
/// stands for the folder the session works in, as Cox gave it, `{result}`
/// for what the last [`Step::Call`] answered, and a value that is
/// just `"{result:number}"` for the first number in it, or
/// `"{result:number after <text>}"` for the first number after `<text>`,
/// as `"{result:number after as chat }"` stands for the chat `open_chat`
/// opened, to give `wait_chat`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// Replies with the text, a word at a time.
    Say(String),
    /// Thinks the text.
    Think(String),
    /// Starts a tool call: the fields of ACP's `tool_call` update, with
    /// `toolCallId` as `id`.
    Tool(Map<String, Value>),
    /// Updates a tool call: the fields of ACP's `tool_call_update`, with
    /// `toolCallId` as `id`.
    ToolUpdate(Map<String, Value>),
    /// Sends ACP's `session/update` with this `update`, as it is.
    Update(Value),
    /// Asks permission, and goes on as Cox answers.
    Ask(Ask),
    /// Calls a tool of an MCP server Cox handed the agent.
    Call(Call),
    /// Waits this many milliseconds.
    Sleep(u64),
    /// Writes the line to standard error.
    Stderr(String),
    /// Ends the turn with an error saying this.
    Fail(String),
}

/// ACP's `session/request_permission`: its `toolCall` and `options`, and
/// in `then`, the steps to take for each option Cox may choose, by its
/// `optionId`, or `cancelled`. When Cox cancels, the turn ends once those
/// steps are taken.
#[derive(Debug, Deserialize)]
pub struct Ask {
    #[serde(default)]
    pub then: HashMap<String, Vec<Step>>,
    #[serde(flatten)]
    pub request: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    /// The name Cox gave the server, `cox` for Cox's own tools.
    #[serde(default = "cox")]
    pub server: String,
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
}

fn cox() -> String {
    "cox".to_owned()
}

impl Turn {
    /// Whether the turn answers a line that says `text`.
    #[must_use]
    pub fn answers(&self, text: &str) -> bool {
        self.when.as_deref().is_none_or(|when| text.contains(when))
    }
}

/// How a value standing for a number in what the last call answered
/// starts.
const NUMBER: &str = "{result:number";

/// `value` with what `{cwd}`, `{result}` and `{result:number}` stand for
/// put in, from `cwd`, the folder the session works in, and `result`,
/// what the last call answered.
#[must_use]
pub fn fill(value: &Value, cwd: &str, result: &str) -> Value {
    match value {
        Value::String(text) if text.starts_with(NUMBER) && text.ends_with('}') => {
            let after = text[NUMBER.len()..text.len() - 1].strip_prefix(" after ");
            let rest = match after {
                Some(after) => result.find(after).map(|at| &result[at + after.len()..]),
                None => Some(result),
            };
            rest.and_then(number).map_or(Value::Null, Value::from)
        }
        Value::String(text) => Value::String(put_in(text, cwd, result)),
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| fill(item, cwd, result)).collect())
        }
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), fill(value, cwd, result)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// `text` with what `{cwd}` and `{result}` stand for put in.
#[must_use]
pub fn put_in(text: &str, cwd: &str, result: &str) -> String {
    text.replace("{cwd}", cwd).replace("{result}", result)
}

/// The first number in `text`.
fn number(text: &str) -> Option<u64> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let digits: String = text[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_scenario_reads_from_json() {
        let scenario: Scenario = serde_json::from_value(json!({
            "turns": [
                { "steps": [{ "say": "Hi." }, { "sleep": 10 }] },
                { "when": "edit", "stop": "cancelled", "steps": [{
                    "ask": {
                        "toolCall": { "toolCallId": "e1", "title": "Edit a" },
                        "options": [{ "optionId": "allow", "name": "Yes", "kind": "allow_once" }],
                        "then": { "allow": [{ "say": "Done." }] }
                    }
                }] }
            ]
        }))
        .unwrap();
        assert!(scenario.turns[0].answers("anything"));
        assert_eq!(scenario.turns[0].stop, "end_turn");
        assert!(!scenario.turns[1].answers("read it"));
        assert!(scenario.turns[1].answers("please edit it"));
        let Step::Ask(ask) = &scenario.turns[1].steps[0] else {
            panic!("an ask");
        };
        assert_eq!(ask.request["toolCall"]["title"], "Edit a");
        assert!(!ask.request.contains_key("then"));
        assert_eq!(ask.then["allow"].len(), 1);
    }

    #[test]
    fn a_step_of_no_known_kind_is_an_error() {
        let read = serde_json::from_value::<Scenario>(
            json!({ "turns": [{ "steps": [{ "shout": "x" }] }] }),
        );
        assert!(read.is_err());
    }

    #[test]
    fn the_result_fills_in() {
        let opened = "Chat 3 is open under this chat as chat 12, working on your line.";
        let filled = fill(
            &json!({
                "chat": "{result:number after as chat }",
                "first": "{result:number}",
                "missing": "{result:number after nowhere}",
                "text": "It said: {result}",
                "path": "{cwd}/notes.md",
                "n": 3
            }),
            "/work",
            opened,
        );
        assert_eq!(
            filled,
            json!({
                "chat": 12,
                "first": 3,
                "missing": null,
                "text": format!("It said: {opened}"),
                "path": "/work/notes.md",
                "n": 3
            })
        );
    }
}
