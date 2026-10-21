//! What the window shows, and what the human's input means, without GPUI.

use cox_acp::{PermissionAnswer, StopReason, ToolCallResult};

/// What the agent side tells the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// A piece of the agent's reply.
    ReplyText(String),
    /// A tool call the agent made has ended.
    ToolCall {
        title: Option<String>,
        result: ToolCallResult,
    },
    /// The agent asks permission for a tool call; the turn waits for an
    /// [`Command::Answer`].
    PermissionRequest {
        title: Option<String>,
        options: Vec<String>,
    },
    /// The turn ended.
    TurnEnded(StopReason),
    /// The agent answered the prompt with an error.
    TurnFailed(String),
    /// The connection to the agent closed, with the error that closed it, if
    /// any.
    Closed(Option<String>),
}

/// What the window asks of the agent side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Prompt(String),
    Answer(PermissionAnswer),
}

/// One thing shown in the conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A line the human sent.
    Human(String),
    /// The agent's reply, as much of it as has arrived.
    Reply(String),
    ToolCall {
        title: String,
        result: ToolCallResult,
    },
    Permission {
        title: String,
        options: Vec<String>,
        /// The option the human chose, once they have.
        chosen: Option<usize>,
    },
    /// A note from Cox: how a turn ended, an error, or the connection closing.
    Note(String),
}

/// Whose move it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Waiting for the human to send a line.
    Idle,
    /// A turn is running.
    Busy,
    /// The agent waits for the human to choose one of the options of the
    /// last [`Entry::Permission`].
    Asking,
    /// The connection has closed; nothing more can be sent.
    Closed,
}

/// The conversation the window shows.
#[derive(Debug)]
pub struct Conversation {
    entries: Vec<Entry>,
    state: State,
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

impl Conversation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            state: State::Idle,
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// Takes what the human entered, and returns what to ask of the agent,
    /// if anything: a prompt when idle, or the answer to a permission request
    /// when the input is the number of one of its options.
    pub fn submit(&mut self, input: &str) -> Option<Command> {
        match self.state {
            State::Idle => {
                if input.trim().is_empty() {
                    return None;
                }
                self.entries.push(Entry::Human(input.to_owned()));
                self.state = State::Busy;
                Some(Command::Prompt(input.to_owned()))
            }
            State::Asking => {
                let number: usize = input.trim().parse().ok()?;
                self.choose(number.checked_sub(1)?)
            }
            State::Busy | State::Closed => None,
        }
    }

    /// Chooses the option at `index` of the permission request the agent
    /// waits on, and returns the answer to send.
    pub fn choose(&mut self, index: usize) -> Option<Command> {
        if self.state != State::Asking {
            return None;
        }
        let Some(Entry::Permission {
            options, chosen, ..
        }) = self.entries.last_mut()
        else {
            return None;
        };
        if index >= options.len() {
            return None;
        }
        *chosen = Some(index);
        self.state = State::Busy;
        Some(Command::Answer(PermissionAnswer::Select(index)))
    }

    /// Shows what the agent side sent.
    pub fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::ReplyText(text) => {
                if let Some(Entry::Reply(reply)) = self.entries.last_mut() {
                    reply.push_str(&text);
                } else {
                    self.entries.push(Entry::Reply(text));
                }
            }
            AgentEvent::ToolCall { title, result } => self.entries.push(Entry::ToolCall {
                title: title.unwrap_or_else(|| "a tool call".to_owned()),
                result,
            }),
            AgentEvent::PermissionRequest { title, options } => {
                self.entries.push(Entry::Permission {
                    title: title.unwrap_or_else(|| "a tool call".to_owned()),
                    options,
                    chosen: None,
                });
                self.state = State::Asking;
            }
            AgentEvent::TurnEnded(reason) => {
                self.entries.push(Entry::Note(format!("[{reason:?}]")));
                self.state = State::Idle;
            }
            AgentEvent::TurnFailed(error) => {
                self.entries.push(Entry::Note(format!("[error: {error}]")));
                self.state = State::Idle;
            }
            AgentEvent::Closed(error) => {
                let note = match error {
                    Some(error) => format!("[closed: {error}]"),
                    None => "[closed]".to_owned(),
                };
                self.entries.push(Entry::Note(note));
                self.state = State::Closed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_a_line_when_idle_and_waits_for_the_turn() {
        let mut conversation = Conversation::new();

        assert_eq!(
            conversation.submit("hello"),
            Some(Command::Prompt("hello".to_owned()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.submit("again"), None);
    }

    #[test]
    fn joins_reply_pieces_and_returns_to_idle_when_the_turn_ends() {
        let mut conversation = Conversation::new();
        conversation.submit("hello");

        conversation.apply(AgentEvent::ReplyText("echo: ".to_owned()));
        conversation.apply(AgentEvent::ReplyText("hello".to_owned()));
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));

        assert_eq!(
            conversation.entries(),
            [
                Entry::Human("hello".to_owned()),
                Entry::Reply("echo: hello".to_owned()),
                Entry::Note("[EndTurn]".to_owned()),
            ]
        );
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn answers_a_permission_request_with_the_number_entered() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        conversation.apply(AgentEvent::PermissionRequest {
            title: Some("touch x.txt".to_owned()),
            options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
        });

        assert_eq!(conversation.submit("3"), None);
        assert_eq!(
            conversation.submit("2"),
            Some(Command::Answer(PermissionAnswer::Select(1)))
        );
        assert_eq!(conversation.state(), State::Busy);
    }
}
