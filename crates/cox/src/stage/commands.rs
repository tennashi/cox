//! What the agents of a folder's chats run, as the stage's Commands page
//! shows them, at `cox://api/commands` (see [`crate::stage::plugins`]):
//! for each chat, the commands of its last steps, each step under what the
//! agent said before it ran them, and each command with how it stands, and
//! how long it ran, as the window saw it; a command that runs long, as a
//! test or a build, with what it printed, once the agent tells, as it does
//! when the command ends, so the human sees what the agent waits on.
//!
//! The window tells this module of a chat's conversation as it changes
//! (see [`tell`]), since the stage's pages are served apart from it; the
//! Activity page reads from it what each chat at work does now, and what
//! each chat that asks the human asks (see [`now`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cox_core::entry::{Content, Entry, ToolCallResult};
use serde_json::{Value, json};

/// How long a command runs before the page shows it big, with what it
/// printed.
const LONG: Duration = Duration::from_secs(5);
/// How many steps of a chat the page is told of, the last.
const STEPS: usize = 6;
/// How many lines of what a command printed the page is told of, the
/// last.
const LINES: usize = 14;
/// How many characters of what the agent said a step is headed by.
const SAID: usize = 160;

/// A chat, as the window tells of it.
#[derive(Clone, Debug)]
pub struct Chat {
    /// The number the window goes by for it this run, which a click on
    /// it, as `cox://chat/<key>`, shows.
    pub key: u64,
    pub name: String,
    /// Where it works.
    pub cwd: PathBuf,
    /// The folder of its project, which a sketch's differs from.
    pub project: PathBuf,
    /// Whether its agent takes a turn.
    pub working: bool,
    /// The steps of its conversation that ran commands (see [`steps`]).
    pub steps: Vec<Step>,
    /// What its agent asks the human, while it asks a question (see
    /// [`asked`]).
    pub asks: Option<String>,
}

/// Commands the agent ran one after another, with nothing said between.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The first line of what the agent said before it ran them, if it
    /// said anything since the turn started.
    pub said: Option<String>,
    pub commands: Vec<Command>,
}

/// A command the agent ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// The id of its tool call.
    pub id: String,
    /// The command, as the agent gave it.
    pub line: String,
    /// What the call shows: what the agent said the command is for while
    /// it runs, and what it printed once it ended.
    pub shows: String,
    /// How it ended, or `None` while it runs.
    pub result: Option<ToolCallResult>,
}

/// The steps of `entries` that ran commands, the last [`STEPS`].
#[must_use]
pub fn steps(entries: &[Entry]) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let mut said: Option<String> = None;
    let mut fresh = true;
    for entry in entries {
        match entry {
            Entry::Human(_) | Entry::Cox(_) => {
                said = None;
                fresh = true;
            }
            Entry::Reply(text) => {
                if let Some(line) = first_line(text) {
                    said = Some(line);
                    fresh = true;
                }
            }
            Entry::ToolCall {
                id,
                title,
                command: true,
                content,
                result,
            } => {
                if fresh || steps.is_empty() {
                    steps.push(Step {
                        said: said.clone(),
                        commands: Vec::new(),
                    });
                    fresh = false;
                }
                if let Some(step) = steps.last_mut() {
                    step.commands.push(Command {
                        id: id.clone(),
                        line: title.clone(),
                        shows: text_of(content),
                        result: *result,
                    });
                }
            }
            _ => {}
        }
    }
    let skip = steps.len().saturating_sub(STEPS);
    steps.split_off(skip)
}

/// What the last question of `entries` asks, while it is not answered:
/// the first line of what the agent said as it asked, or of its first
/// question, or that question's label.
#[must_use]
pub fn asked(entries: &[Entry]) -> Option<String> {
    let Some(Entry::Question {
        message,
        questions,
        answer: None,
    }) = entries
        .iter()
        .rev()
        .find(|entry| matches!(entry, Entry::Question { .. }))
    else {
        return None;
    };
    first_line(message).or_else(|| {
        let question = questions.first()?;
        question
            .text
            .as_deref()
            .and_then(first_line)
            .or_else(|| question.header.clone())
    })
}

/// The first line of `text` that says something, cut to [`SAID`]
/// characters.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let mut cut: String = line.chars().take(SAID).collect();
    if cut.len() < line.len() {
        cut.push('…');
    }
    Some(cut)
}

/// The text of `content`, its pieces a line apart.
fn text_of(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(|content| match content {
            Content::Text(text) => Some(text.as_str()),
            Content::Diff { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// When the window saw a command start, and end, by its chat and its
/// id.
#[derive(Clone, Copy, Debug)]
struct Seen {
    /// When the window first saw it run, if it saw it run.
    started: Option<Instant>,
    ended: Option<Instant>,
}

/// What the window told, by chat, and when it saw each command start
/// and end.
#[derive(Default)]
struct Told {
    chats: HashMap<u64, Chat>,
    seen: HashMap<(u64, String), Seen>,
}

impl Told {
    /// Keeps `chat` as it is now, `now`, in place of what was told of it.
    fn tell(&mut self, chat: Chat, now: Instant) {
        let key = chat.key;
        self.seen.retain(|(chat_key, id), _| {
            *chat_key != key
                || chat
                    .steps
                    .iter()
                    .any(|step| step.commands.iter().any(|command| command.id == *id))
        });
        for command in chat.steps.iter().flat_map(|step| &step.commands) {
            let seen = self.seen.entry((key, command.id.clone())).or_insert(Seen {
                started: command.result.is_none().then_some(now),
                ended: None,
            });
            if command.result.is_some() && seen.ended.is_none() {
                seen.ended = Some(now);
            }
        }
        self.chats.insert(key, chat);
    }

    /// Forgets the chats the window no longer has, all but `keys`.
    fn keep(&mut self, keys: &[u64]) {
        self.chats.retain(|key, _| keys.contains(key));
        self.seen.retain(|(key, _), _| keys.contains(key));
    }
}

/// What the window told.
static TOLD: Mutex<Option<Told>> = Mutex::new(None);

/// Keeps `chat` as it is now, for the pages to read.
pub fn tell(chat: Chat) {
    if let Ok(mut told) = TOLD.lock() {
        told.get_or_insert_with(Told::default)
            .tell(chat, Instant::now());
    }
}

/// Forgets the chats the window no longer has, all but `keys`.
pub fn keep(keys: &[u64]) {
    if let Ok(mut told) = TOLD.lock() {
        told.get_or_insert_with(Told::default).keep(keys);
    }
}

/// What the agent of a chat does now, and what it asks, as the window
/// last told of its conversation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Now {
    /// What the agent said before its last step of commands.
    pub said: Option<String>,
    /// The last command of that step still running, if one is.
    pub running: Option<String>,
    /// What the agent asks the human (see [`asked`]).
    pub asks: Option<String>,
}

impl Chat {
    fn now(&self) -> Now {
        let last = self.steps.last();
        Now {
            said: last.and_then(|step| step.said.clone()),
            running: last.and_then(|step| {
                step.commands
                    .iter()
                    .rev()
                    .find(|command| command.result.is_none())
                    .map(|command| command.line.clone())
            }),
            asks: self.asks.clone(),
        }
    }
}

/// What the agent of the chat `key` does now, and what it asks, if the
/// window told of its conversation.
#[must_use]
pub fn now(key: u64) -> Option<Now> {
    let told = TOLD.lock().ok()?;
    told.as_ref()?.chats.get(&key).map(Chat::now)
}

/// What the page at `cox://api/commands` reads, for the stage of the
/// folder `root`: the folder, and the chats that work there, or whose
/// project's folder it is, that ran commands, those whose agents take a
/// turn first.
#[must_use]
pub fn told(root: &Path) -> Value {
    let told = TOLD.lock();
    let empty = Told::default();
    let told = told
        .as_ref()
        .ok()
        .and_then(|told| told.as_ref())
        .unwrap_or(&empty);
    told_of(told, root, Instant::now())
}

fn told_of(told: &Told, root: &Path, now: Instant) -> Value {
    let mut chats: Vec<&Chat> = told
        .chats
        .values()
        .filter(|chat| (chat.cwd == root || chat.project == root) && !chat.steps.is_empty())
        .collect();
    chats.sort_by_key(|chat| (!chat.working, std::cmp::Reverse(chat.key)));
    let chats: Vec<Value> = chats
        .into_iter()
        .map(|chat| {
            let steps: Vec<Value> = chat
                .steps
                .iter()
                .map(|step| {
                    let commands: Vec<Value> = step
                        .commands
                        .iter()
                        .map(|command| {
                            let seen = told.seen.get(&(chat.key, command.id.clone()));
                            command_of(command, seen, now)
                        })
                        .collect();
                    json!({
                        "id": step.commands.first().map(|command| &command.id),
                        "said": step.said,
                        "commands": commands,
                    })
                })
                .collect();
            json!({
                "key": chat.key,
                "name": chat.name,
                "working": chat.working,
                "sketch": chat.cwd != root,
                "steps": steps,
            })
        })
        .collect();
    json!({"folder": root.display().to_string(), "chats": chats})
}

/// How the page is told of `command`, which the window saw start and end
/// as `seen` says, at `now`.
fn command_of(command: &Command, seen: Option<&Seen>, now: Instant) -> Value {
    let ran = seen.and_then(|seen| {
        let started = seen.started?;
        Some(seen.ended.unwrap_or(now).saturating_duration_since(started))
    });
    let state = match command.result {
        None => "running",
        Some(ToolCallResult::Completed) => "done",
        Some(ToolCallResult::Failed) => "failed",
        Some(ToolCallResult::Cancelled) => "cancelled",
    };
    let long = ran.is_some_and(|ran| ran >= LONG);
    let printed = (command.result.is_some()
        && (long || command.result == Some(ToolCallResult::Failed)))
    .then(|| last_lines(&command.shows));
    json!({
        "line": command.line,
        "state": state,
        "why": command.result.is_none().then(|| first_line(&command.shows)).flatten(),
        "secs": ran.map(|ran| ran.as_secs()),
        "long": long,
        "printed": printed,
    })
}

/// The last [`LINES`] lines of `text`, out of the fence of Markdown
/// code an agent may put what a command printed in.
fn last_lines(text: &str) -> String {
    let mut lines: Vec<&str> = text.trim_end().lines().collect();
    if lines.first().is_some_and(|line| line.starts_with("```"))
        && lines.last().is_some_and(|line| line.trim() == "```")
    {
        lines.pop();
        lines.remove(0);
    }
    lines[lines.len().saturating_sub(LINES)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, line: &str, shows: &str, result: Option<ToolCallResult>) -> Entry {
        Entry::ToolCall {
            id: id.to_owned(),
            title: line.to_owned(),
            command: true,
            content: vec![Content::Text(shows.to_owned())],
            result,
        }
    }

    #[test]
    fn steps_part_where_the_agent_says_something() {
        let entries = [
            Entry::Human("fix it".to_owned()),
            call("1", "grep -n x", "", Some(ToolCallResult::Completed)),
            Entry::Reply("Found it.\n\nNow I test.".to_owned()),
            call("2", "cargo test", "Runs the tests", None),
            Entry::ToolCall {
                id: "3".to_owned(),
                title: "Read a.rs".to_owned(),
                command: false,
                content: Vec::new(),
                result: Some(ToolCallResult::Completed),
            },
            call("4", "cargo fmt", "", None),
        ];

        let steps = steps(&entries);

        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].said, None);
        assert_eq!(steps[0].commands[0].line, "grep -n x");
        assert_eq!(steps[1].said.as_deref(), Some("Found it."));
        let lines: Vec<&str> = steps[1].commands.iter().map(|c| c.line.as_str()).collect();
        assert_eq!(lines, ["cargo test", "cargo fmt"]);
    }

    #[test]
    fn a_command_that_ran_long_shows_what_it_printed() {
        let root = PathBuf::from("/p");
        let chat = |result: Option<ToolCallResult>| Chat {
            key: 1,
            name: "fix".to_owned(),
            cwd: root.clone(),
            project: root.clone(),
            working: true,
            asks: None,
            steps: vec![Step {
                said: None,
                commands: vec![
                    Command {
                        id: "1".to_owned(),
                        line: "cargo test".to_owned(),
                        shows: if result.is_some() {
                            "ok\ntest result: ok"
                        } else {
                            "Runs the tests"
                        }
                        .to_owned(),
                        result,
                    },
                    Command {
                        id: "2".to_owned(),
                        line: "ls".to_owned(),
                        shows: "a\nb".to_owned(),
                        result: Some(ToolCallResult::Completed),
                    },
                ],
            }],
        };
        let start = Instant::now();
        let mut told = Told::default();

        told.tell(chat(None), start);
        let running = told_of(&told, &root, start + Duration::from_secs(7));
        told.tell(
            chat(Some(ToolCallResult::Completed)),
            start + Duration::from_secs(9),
        );
        let ended = told_of(&told, &root, start + Duration::from_secs(30));

        let test = &running["chats"][0]["steps"][0]["commands"][0];
        assert_eq!(test["state"], "running");
        assert_eq!(test["why"], "Runs the tests");
        assert_eq!(test["long"], true);
        assert_eq!(test["secs"], 7);
        let test = &ended["chats"][0]["steps"][0]["commands"][0];
        assert_eq!(test["state"], "done");
        assert_eq!(test["secs"], 9);
        assert_eq!(test["printed"], "ok\ntest result: ok");
        let ls = &ended["chats"][0]["steps"][0]["commands"][1];
        assert_eq!(ls["long"], false);
        assert_eq!(ls["secs"], Value::Null);
        assert_eq!(ls["printed"], Value::Null);
    }

    #[test]
    fn a_chat_does_its_last_step_and_asks_its_last_question_unanswered() {
        use cox_core::entry::{Question, QuestionAnswer};

        let question = |message: &str, text: Option<&str>, answer| Entry::Question {
            message: message.to_owned(),
            questions: vec![Question {
                header: Some("Approach".to_owned()),
                text: text.map(str::to_owned),
                ..Question::default()
            }],
            answer,
        };
        let entries = vec![
            call("1", "ls", "", Some(ToolCallResult::Completed)),
            Entry::Reply("Now I wait.\nThen I report.".to_owned()),
            call("2", "sleep 30 && echo done", "Waits", None),
            call("3", "date", "", Some(ToolCallResult::Completed)),
        ];
        let chat = Chat {
            key: 1,
            name: "wait".to_owned(),
            cwd: PathBuf::from("/p"),
            project: PathBuf::from("/p"),
            working: true,
            asks: asked(&entries),
            steps: steps(&entries),
        };

        assert_eq!(
            chat.now(),
            Now {
                said: Some("Now I wait.".to_owned()),
                running: Some("sleep 30 && echo done".to_owned()),
                asks: None,
            }
        );
        assert_eq!(
            asked(&[question("", Some("Which one?"), None)]).as_deref(),
            Some("Which one?")
        );
        assert_eq!(
            asked(&[question("Which way?\n\nTell me.", None, None)]).as_deref(),
            Some("Which way?")
        );
        assert_eq!(
            asked(&[question("", None, None)]).as_deref(),
            Some("Approach")
        );
        assert_eq!(
            asked(&[question("Which way?", None, Some(QuestionAnswer::Declined))]),
            None
        );
    }

    #[test]
    fn what_a_command_printed_comes_out_of_its_fence() {
        assert_eq!(last_lines("```console\n1\n2\n```"), "1\n2");
        assert_eq!(last_lines("1\n2\n"), "1\n2");
    }

    #[test]
    fn a_chat_of_another_folder_is_not_told() {
        let mut told = Told::default();
        told.tell(
            Chat {
                key: 1,
                name: "x".to_owned(),
                cwd: PathBuf::from("/q"),
                project: PathBuf::from("/q"),
                working: false,
                asks: None,
                steps: steps(&[call("1", "ls", "", None)]),
            },
            Instant::now(),
        );

        assert_eq!(
            told_of(&told, Path::new("/p"), Instant::now())["chats"],
            json!([])
        );
    }
}
