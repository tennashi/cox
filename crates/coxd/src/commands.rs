//! What the agents of the chats run: what each chat's agent does now,
//! as the Activity page tells it on the chat's line (see [`Now`]), and a
//! command an agent ran as a page shows it, short (see [`called`]), or
//! laid out to read (see [`laid_out`]), with the last lines of what it
//! printed (see [`printed`]).
//!
//! What a chat's agent does now is read from its conversation, by
//! whoever holds it, the window or coxd (see [`Chat::of`]): what the agent
//! said before the commands it ran last, the one of them that runs yet,
//! and what it asks the human.

use std::path::{Path, PathBuf};

use cox_core::entry::{Content, Entry};
use serde_json::Value;

mod layout;

/// What a command an agent ran goes by where it shows short, as a chip of
/// the chat or a row of the Activity page: what the agent said it is for,
/// or, when it said nothing, the gist of the command (see
/// [`layout::gist`]).
#[must_use]
pub fn called(line: &str, about: Option<&str>) -> String {
    about.map_or_else(|| layout::gist(line), str::to_owned)
}

/// The row of the command `line` that says most of what it does, on one
/// line (see [`layout::gist`]).
#[must_use]
pub fn gist(line: &str) -> String {
    layout::gist(line)
}

/// The command `line`, run in `folder`, laid out to read (see
/// [`layout::told`]).
#[must_use]
pub fn laid_out(line: &str, folder: &Path) -> Value {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    layout::told(line, folder, home.as_deref())
}

/// The last lines of what a command that shows `content` printed, as a
/// page tells it.
#[must_use]
pub fn printed(content: &[Content]) -> String {
    last_lines(&text_of(content))
}

/// How many lines of what a command printed a page is told of, the last.
const LINES: usize = 14;
/// How many characters of what the agent said a line tells.
const SAID: usize = 160;

/// What the agent of a chat does now, as its conversation tells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chat {
    /// Its key.
    pub key: u64,
    /// The first line of what the agent said before the commands it ran
    /// last one after another, with nothing said between, if it said
    /// anything since the turn started.
    pub said: Option<String>,
    /// The last of those commands that runs yet, if one does, as the
    /// agent gave it, with what it said it is for, if it said.
    pub running: Option<(String, Option<String>)>,
    /// What its agent asks the human, while it asks a question (see
    /// [`asked`]).
    pub asks: Option<String>,
}

impl Chat {
    /// The chat `key`, whose conversation is `entries`, as it does now.
    #[must_use]
    pub fn of(key: u64, entries: &[Entry]) -> Self {
        let mut said: Option<String> = None;
        // Whether the next command starts another run of them.
        let mut fresh = true;
        let mut chat = Self {
            key,
            asks: asked(entries),
            ..Self::default()
        };
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
                    title,
                    command: true,
                    about,
                    result,
                    ..
                } => {
                    if std::mem::take(&mut fresh) {
                        chat.said.clone_from(&said);
                        chat.running = None;
                    }
                    if result.is_none() {
                        chat.running = Some((title.clone(), about.clone()));
                    }
                }
                _ => {}
            }
        }
        chat
    }

    /// What its agent does now, and what it asks, as the page is told it.
    #[must_use]
    pub fn now(&self) -> Now {
        Now {
            said: self.said.clone(),
            running: self.running.as_ref().map(|(line, about)| {
                let called = called(line, about.as_deref());
                (called, line.clone())
            }),
            asks: self.asks.clone(),
        }
    }
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
        ..
    }) = entries
        .iter()
        .rev()
        .find(|entry| matches!(entry, Entry::Question { .. }))
    else {
        return None;
    };
    // A question of several goes by its header, its own short name.
    let line = |question: &cox_core::entry::Question| {
        question
            .header
            .clone()
            .or_else(|| question.text.as_deref().and_then(first_line))
    };
    // Several questions come under a message that names none of them.
    if questions.len() > 1 {
        let lines: Vec<String> = questions.iter().filter_map(line).collect();
        if !lines.is_empty() {
            return Some(lines.join(" · "));
        }
    }
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
pub fn first_line(text: &str) -> Option<String> {
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

/// What the agent of a chat does now, and what it asks, as its
/// conversation told when it was last read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Now {
    /// What the agent said before the commands it ran last.
    pub said: Option<String>,
    /// What the last of those commands that runs yet goes by, if one
    /// does (see [`called`]), and the command, as the agent gave it.
    pub running: Option<(String, String)>,
    /// What the agent asks the human (see [`asked`]).
    pub asks: Option<String>,
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
    use cox_core::entry::ToolCallResult;

    fn call(id: &str, line: &str, shows: &str, result: Option<ToolCallResult>) -> Entry {
        Entry::ToolCall {
            id: id.to_owned(),
            title: line.to_owned(),
            command: true,
            about: None,
            content: vec![Content::Text(shows.to_owned())],
            result,
        }
    }

    #[test]
    fn a_chat_does_the_last_command_it_runs_of_those_it_ran_last() {
        let entries = [
            Entry::Human("fix it".to_owned()),
            call("1", "grep -n x", "", Some(ToolCallResult::Completed)),
            Entry::Reply("Found it.\n\nNow I test.".to_owned()),
            call("2", "cargo test", "Runs the tests", None),
            Entry::ToolCall {
                id: "3".to_owned(),
                title: "Read a.rs".to_owned(),
                command: false,
                about: None,
                content: Vec::new(),
                result: None,
            },
            Entry::Thought("hm".to_owned()),
            call("4", "cargo fmt", "", Some(ToolCallResult::Completed)),
        ];

        let chat = Chat::of(1, &entries);

        assert_eq!(
            chat.now(),
            Now {
                said: Some("Found it.".to_owned()),
                // The agent said nothing of what it is for: it goes by
                // its gist.
                running: Some(("cargo test".to_owned(), "cargo test".to_owned())),
                asks: None,
            }
        );
        // Once the agent said something again, the commands before it are
        // done with.
        let mut more = entries.to_vec();
        more.push(Entry::Reply("Now I lint.".to_owned()));
        more.push(call(
            "5",
            "cargo clippy",
            "",
            Some(ToolCallResult::Completed),
        ));
        let chat = Chat::of(1, &more);
        assert_eq!(chat.said.as_deref(), Some("Now I lint."));
        assert_eq!(chat.running, None);
        // A line of the human's starts the turn over.
        more.push(Entry::Human("again".to_owned()));
        more.push(call("6", "sleep 30 && echo done", "Waits", None));
        let chat = Chat::of(1, &more);
        assert_eq!(
            chat.now(),
            Now {
                said: None,
                running: Some(("sleep 30".to_owned(), "sleep 30 && echo done".to_owned())),
                asks: None,
            }
        );
    }

    #[test]
    fn a_chat_asks_its_last_question_unanswered() {
        use cox_core::entry::{Question, QuestionAnswer};

        let question = |message: &str, text: Option<&str>, answer| Entry::Question {
            message: message.to_owned(),
            questions: vec![Question {
                header: Some("Approach".to_owned()),
                text: text.map(str::to_owned),
                ..Question::default()
            }],
            answer,
            answered_at: None,
        };
        assert_eq!(
            Chat::of(1, &[question("", Some("Which one?"), None)])
                .asks
                .as_deref(),
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
        let Entry::Question {
            questions: mut both,
            ..
        } = question("", Some("Which one?"), None)
        else {
            unreachable!("a question");
        };
        both.push(cox_core::entry::Question {
            header: Some("Add".to_owned()),
            ..both[0].clone()
        });
        assert_eq!(
            asked(&[Entry::Question {
                message: "Please answer the following questions.".to_owned(),
                questions: both,
                answer: None,
                answered_at: None,
            }])
            .as_deref(),
            Some("Approach · Add")
        );
    }

    #[test]
    fn what_a_command_printed_comes_out_of_its_fence() {
        assert_eq!(last_lines("```console\n1\n2\n```"), "1\n2");
        assert_eq!(last_lines("1\n2\n"), "1\n2");
        assert_eq!(
            printed(&[Content::Text("```console\nok\n```".to_owned())]),
            "ok"
        );
    }
}
