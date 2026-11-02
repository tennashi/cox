//! A scenario made from what a chat's agent did in a run that Cox kept:
//! the lines Cox and the agent sent each other, and what the agent wrote
//! to its standard error, as Cox's `src/logs.rs` keeps them. Played, the
//! scenario sends Cox what the agent sent, turn by turn, and as fast, to
//! make what happened in the run happen again, as on another machine.
//!
//! The agent's calls of Cox's own tools are not among those lines, so the
//! scenario does not make them.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// Where the lines of a run say the agent sent or got something.
const WIRE: &str = "cox_acp::wire";
const STDERR: &str = "cox_acp::stderr";

/// The longest a scenario waits between two steps, however long the run
/// waited.
const LONGEST_WAIT: u64 = 10_000;

/// The scenario of the chat `chat` in `run`, the text of a run's file, or
/// of its only chat when `chat` is `None`. A goal's judge is the chat
/// `judge`.
pub fn scenario(run: &str, chat: Option<&str>) -> Result<Value, String> {
    let mut chats: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for line in run.lines().filter(|line| !line.trim().is_empty()) {
        let said: Value = serde_json::from_str(line).map_err(|error| format!("{error}: {line}"))?;
        if said["target"] != WIRE && said["target"] != STDERR {
            continue;
        }
        if let Some(of) = of(&said) {
            chats.entry(of).or_default().push(said);
        }
    }
    let named: Vec<&String> = chats.keys().collect();
    let chosen = match chat {
        Some(chat) => chats
            .get(chat)
            .ok_or_else(|| format!("the run has no chat {chat}, only {}", listed(&named)))?,
        None if chats.len() == 1 => chats.values().next().expect("one chat"),
        None if chats.is_empty() => return Err("the run has no agent's lines".to_owned()),
        None => {
            return Err(format!(
                "the run has chats {}: say which with --chat",
                listed(&named)
            ));
        }
    };
    Ok(json!({ "turns": turns(chosen) }))
}

/// The chat `said` was said in: the number it is kept by, or `judge`.
fn of(said: &Value) -> Option<String> {
    let spans = said["spans"].as_array()?;
    spans
        .iter()
        .rev()
        .find_map(|span| match span["name"].as_str()? {
            "chat" => Some(match &span["id"] {
                Value::String(id) => id.clone(),
                id => id.to_string(),
            }),
            "judge" => Some("judge".to_owned()),
            _ => None,
        })
}

fn listed(names: &[&String]) -> String {
    names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The turns of a chat whose agent's lines are `lines`.
fn turns(lines: &[Value]) -> Vec<Value> {
    let mut turns = Vec::new();
    // The turn under way: the id of its prompt, and its steps so far.
    let mut current: Option<(Value, Vec<Value>)> = None;
    // When the last step, or the prompt, came, in milliseconds of the day.
    let mut last: Option<u64> = None;
    for said in lines {
        let at = said["timestamp"].as_str().and_then(milliseconds);
        let text = said["fields"]["message"].as_str().unwrap_or_default();
        if said["target"] == STDERR {
            if let Some((_, steps)) = &mut current {
                wait(steps, last, at);
                steps.push(json!({ "stderr": text }));
                last = at.or(last);
            }
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let method = message["method"].as_str();
        if said["fields"]["from"] == "cox" {
            if method == Some("session/prompt") {
                current = Some((message["id"].clone(), Vec::new()));
                last = at;
            } else if method.is_none() && current.is_some() {
                // Cox answered the agent, as the human chose an option:
                // the agent waits from then, not from its asking, which
                // the human takes their time over again as it plays.
                last = at;
            }
            continue;
        }
        let Some((prompt, steps)) = &mut current else {
            continue;
        };
        match method {
            Some("session/update") => {
                wait(steps, last, at);
                steps.push(json!({ "update": message["params"]["update"] }));
                last = at.or(last);
            }
            Some("session/request_permission") => {
                wait(steps, last, at);
                let mut asked: Map<String, Value> =
                    message["params"].as_object().cloned().unwrap_or_default();
                asked.remove("sessionId");
                steps.push(json!({ "ask": asked }));
                last = at.or(last);
            }
            None if message["id"] == *prompt => {
                wait(steps, last, at);
                let mut turn = json!({ "steps": std::mem::take(steps) });
                if let Some(error) = message.get("error") {
                    let error = error["message"].as_str().unwrap_or("the turn failed");
                    turn["steps"]
                        .as_array_mut()
                        .expect("steps")
                        .push(json!({ "fail": error }));
                } else if let Some(stop) = message["result"]["stopReason"].as_str() {
                    turn["stop"] = Value::from(stop);
                }
                turns.push(turn);
                current = None;
            }
            _ => {}
        }
    }
    turns
}

/// Waits in `steps` as long as the run waited from `last` to `at`, when
/// that was long enough to tell.
fn wait(steps: &mut Vec<Value>, last: Option<u64>, at: Option<u64>) {
    let (Some(last), Some(at)) = (last, at) else {
        return;
    };
    // Across midnight, the day starts again.
    let waited = (at + DAY - last) % DAY;
    if waited >= 20 {
        steps.push(json!({ "sleep": waited.min(LONGEST_WAIT) }));
    }
}

const DAY: u64 = 24 * 60 * 60 * 1000;

/// The milliseconds of the day of `timestamp`, as RFC 3339 writes it in
/// UTC, as `2026-11-01T16:35:57.123456Z`.
fn milliseconds(timestamp: &str) -> Option<u64> {
    let time = timestamp.split_once('T')?.1.strip_suffix('Z')?;
    let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut parts = clock.split(':').map(str::parse::<u64>);
    let (hours, minutes, seconds) = (
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    );
    let fraction: String = fraction.chars().chain("000".chars()).take(3).collect();
    Some(((hours * 60 + minutes) * 60 + seconds) * 1000 + fraction.parse::<u64>().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line of a run's file.
    fn line(at: &str, target: &str, from: Option<&str>, message: &Value, chat: &str) -> String {
        let span = if chat == "judge" {
            json!({ "name": "judge", "cwd": "/p" })
        } else {
            json!({ "name": "chat", "id": chat, "cwd": "/p" })
        };
        let mut fields = json!({ "message": message.as_str().map_or_else(|| message.to_string(), str::to_owned) });
        if let Some(from) = from {
            fields["from"] = Value::from(from);
        }
        json!({
            "timestamp": format!("2026-11-01T{at}Z"),
            "level": "DEBUG",
            "fields": fields,
            "target": target,
            "spans": [span, { "name": "connection" }],
        })
        .to_string()
    }

    fn run() -> String {
        let update = |text: &str| {
            json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": "s",
                "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } } } })
        };
        [
            line("10:00:00.000", "cox::logs", None, &json!("Cox starts"), "1"),
            line(
                "10:00:01.000",
                WIRE,
                Some("cox"),
                &json!({ "jsonrpc": "2.0", "id": "a", "method": "initialize" }),
                "1",
            ),
            line(
                "10:00:01.100",
                WIRE,
                Some("agent"),
                &json!({ "jsonrpc": "2.0", "id": "a", "result": {} }),
                "1",
            ),
            line(
                "10:00:02.000",
                WIRE,
                Some("cox"),
                &json!({ "jsonrpc": "2.0", "id": "p1", "method": "session/prompt" }),
                "1",
            ),
            line("10:00:02.500", STDERR, None, &json!("thinking hard"), "1"),
            line("10:00:02.510", WIRE, Some("agent"), &update("Hello"), "1"),
            line(
                "10:00:03.000",
                WIRE,
                Some("agent"),
                &json!({ "jsonrpc": "2.0", "id": 9, "method": "session/request_permission",
                "params": { "sessionId": "s", "toolCall": { "toolCallId": "e" }, "options": [] } }),
                "1",
            ),
            line(
                "10:00:03.700",
                WIRE,
                Some("cox"),
                &json!({ "jsonrpc": "2.0", "id": 9, "result": { "outcome": { "outcome": "cancelled" } } }),
                "1",
            ),
            line(
                "10:00:04.000",
                WIRE,
                Some("agent"),
                &json!({ "jsonrpc": "2.0", "id": "p1", "result": { "stopReason": "cancelled" } }),
                "1",
            ),
            line(
                "10:00:05.000",
                WIRE,
                Some("cox"),
                &json!({ "jsonrpc": "2.0", "id": "j1", "method": "session/prompt" }),
                "judge",
            ),
            line(
                "10:00:05.000",
                WIRE,
                Some("agent"),
                &json!({ "jsonrpc": "2.0", "id": "j1", "error": { "message": "no model" } }),
                "judge",
            ),
        ]
        .join("\n")
    }

    #[test]
    fn a_chats_turns_are_made_from_what_its_agent_sent() {
        let scenario = scenario(&run(), Some("1")).unwrap();

        assert_eq!(
            scenario,
            json!({ "turns": [{
                "stop": "cancelled",
                "steps": [
                    { "sleep": 500 },
                    { "stderr": "thinking hard" },
                    { "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Hello" } } },
                    { "sleep": 490 },
                    { "ask": { "toolCall": { "toolCallId": "e" }, "options": [] } },
                    { "sleep": 300 },
                ],
            }] })
        );
    }

    #[test]
    fn a_turn_the_agent_failed_fails() {
        let scenario = scenario(&run(), Some("judge")).unwrap();

        assert_eq!(
            scenario,
            json!({ "turns": [{ "steps": [{ "fail": "no model" }] }] })
        );
    }

    #[test]
    fn a_run_of_several_chats_needs_one_named() {
        assert_eq!(
            scenario(&run(), None),
            Err("the run has chats 1, judge: say which with --chat".to_owned())
        );
        assert_eq!(
            scenario(&run(), Some("2")),
            Err("the run has no chat 2, only 1, judge".to_owned())
        );
    }

    #[test]
    fn the_time_of_day_reads_from_a_timestamp() {
        assert_eq!(milliseconds("2026-11-01T01:02:03.456789Z"), Some(3_723_456));
        assert_eq!(milliseconds("2026-11-01T01:02:03Z"), Some(3_723_000));
        assert_eq!(milliseconds("nonsense"), None);
    }
}
