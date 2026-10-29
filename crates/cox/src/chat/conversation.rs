//! What the window shows of a chat, and what the human's input means,
//! without GPUI: the conversation, as the agent's events change it, and
//! the commands the human's lines turn into (see [`crate::agent`]).

use std::collections::BTreeSet;

use cox_acp::PermissionAnswer;
pub use cox_core::entry::{Content, Entry, ToolCallResult};

use crate::agent::{AgentEvent, Command, Opened};

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
    /// The human asked the agent to stop the turn, and it has not ended
    /// yet.
    Stopping,
    /// The connection has closed; nothing more can be sent.
    Closed,
}

/// The conversation the window shows.
#[derive(Debug)]
pub struct Conversation {
    entries: Vec<Entry>,
    state: State,
    /// The indexes of the entries added or changed since they were last
    /// taken, to keep.
    changed: BTreeSet<usize>,
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
            changed: BTreeSet::new(),
        }
    }

    /// A conversation that opens with `note`, a note from Cox.
    #[must_use]
    pub fn noting(note: String) -> Self {
        let mut conversation = Self::new();
        conversation.push(Entry::Note(note));
        conversation
    }

    /// The conversation of a chat that has closed, which showed `entries`,
    /// for the human to read.
    #[must_use]
    pub fn closed(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            state: State::Closed,
            changed: BTreeSet::new(),
        }
    }

    /// Opens the conversation of a chat that has closed again, for the
    /// human to go on with it once its agent starts again.
    pub fn reopen(&mut self) {
        if self.state == State::Closed {
            self.state = State::Idle;
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

    /// The indexes of the entries added or changed since this was last
    /// called, in order.
    pub fn take_changed(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.changed).into_iter().collect()
    }

    fn push(&mut self, entry: Entry) {
        self.changed.insert(self.entries.len());
        self.entries.push(entry);
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
                self.push(Entry::Human(input.to_owned()));
                self.state = State::Busy;
                Some(Command::Prompt(input.to_owned()))
            }
            State::Asking => {
                let number: usize = input.trim().parse().ok()?;
                self.choose(number.checked_sub(1)?)
            }
            State::Busy | State::Stopping | State::Closed => None,
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
        self.changed.insert(self.entries.len() - 1);
        self.state = State::Busy;
        Some(Command::Answer(PermissionAnswer::Select(index)))
    }

    /// Asks the agent to stop the turn it is taking, if one is running and
    /// the human has not asked already, and returns the command to send. The
    /// conversation stays: once the turn ends, the human can send the next
    /// line.
    pub fn cancel(&mut self) -> Option<Command> {
        match self.state {
            State::Busy | State::Asking => {
                self.state = State::Stopping;
                Some(Command::Cancel)
            }
            State::Idle | State::Stopping | State::Closed => None,
        }
    }

    /// The index of the entry of tool call `id`, if it has one.
    fn tool_call_entry(&self, id: &str) -> Option<usize> {
        self.entries.iter().rposition(
            |entry| matches!(entry, Entry::ToolCall { id: shown_id, .. } if shown_id == id),
        )
    }

    /// Shows news of a tool call: in place of the call's entry once it has
    /// one, so a call shows as running until it ends. What a permission
    /// request for the call shows already, the call does not show again.
    fn show_tool_call(
        &mut self,
        id: String,
        title: Option<String>,
        content: Option<Vec<Content>>,
        result: Option<ToolCallResult>,
    ) {
        let Some(index) = self.tool_call_entry(&id) else {
            self.push(Entry::ToolCall {
                id,
                title: title.unwrap_or_else(|| "a tool call".to_owned()),
                content: content.unwrap_or_default(),
                result,
            });
            return;
        };
        let content = content.map(|content| {
            let asked = self.entries[index + 1..].iter().any(|entry| {
                matches!(entry, Entry::Permission { id: asked_id, content: asked, .. }
                    if *asked_id == id && *asked == content)
            });
            if asked { Vec::new() } else { content }
        });
        self.changed.insert(index);
        if let Entry::ToolCall {
            title: shown_title,
            content: shown_content,
            result: shown_result,
            ..
        } = &mut self.entries[index]
        {
            if let Some(title) = title {
                *shown_title = title;
            }
            if let Some(content) = content {
                *shown_content = content;
            }
            *shown_result = result;
        }
    }

    /// Shows a permission request with what its tool call is about to do:
    /// what the request tells, or else what the call shows, which then
    /// shows only here.
    fn ask(
        &mut self,
        id: String,
        title: Option<String>,
        content: Option<Vec<Content>>,
        options: Vec<String>,
    ) {
        let call = self.tool_call_entry(&id);
        let shown = call.and_then(|index| match &mut self.entries[index] {
            Entry::ToolCall { content, .. } => Some(content),
            _ => None,
        });
        let content = match shown {
            Some(shown) => {
                let content = content.unwrap_or_else(|| shown.clone());
                if *shown == content {
                    shown.clear();
                    self.changed.extend(call);
                }
                content
            }
            None => content.unwrap_or_default(),
        };
        self.push(Entry::Permission {
            id,
            title: title.unwrap_or_else(|| "a tool call".to_owned()),
            content,
            options,
            chosen: None,
        });
    }

    /// Shows what the agent side sent.
    pub fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Opened { how, .. } => match how {
                Opened::New => {}
                Opened::Resumed => {
                    self.push(Entry::Note(
                        "[resumed: the agent goes on from here]".to_owned(),
                    ));
                }
                Opened::Retold { instructions } => {
                    self.push(Entry::Note(
                        "[the agent could not resume its session: a new one is told the conversation so far]"
                            .to_owned(),
                    ));
                    if let Some(path) = instructions {
                        self.push(Entry::Note(format!(
                            "[instructions: {}]",
                            crate::ui::paths::shortened(&path)
                        )));
                    }
                }
            },
            AgentEvent::ReplyText(text) => {
                if let Some(Entry::Reply(reply)) = self.entries.last_mut() {
                    reply.push_str(&text);
                    self.changed.insert(self.entries.len() - 1);
                } else {
                    self.push(Entry::Reply(text));
                }
            }
            AgentEvent::ToolCall {
                id,
                title,
                content,
                result,
            } => self.show_tool_call(id, title, content, result),
            AgentEvent::PermissionRequest {
                id,
                title,
                content,
                options,
            } => {
                self.ask(id, title, content, options);
                // Asked before the agent heard it is to stop: the agent
                // side answers it as cancelled, without the human.
                if self.state != State::Stopping {
                    self.state = State::Asking;
                }
            }
            AgentEvent::TurnEnded(reason) => {
                self.push(Entry::Note(format!("[{reason:?}]")));
                self.state = State::Idle;
            }
            AgentEvent::TurnFailed(error) => {
                self.push(Entry::Note(format!("[error: {error}]")));
                self.state = State::Idle;
            }
            AgentEvent::Closed(error) => {
                let note = match error {
                    Some(error) => format!("[closed: {error}]"),
                    None => "[closed]".to_owned(),
                };
                self.push(Entry::Note(note));
                self.state = State::Closed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use cox_acp::StopReason;

    use super::*;

    #[test]
    fn a_closed_conversation_reopens_to_go_on() {
        let mut conversation = Conversation::closed(vec![
            Entry::Human("hello".to_owned()),
            Entry::Reply("echo: hello".to_owned()),
        ]);
        assert_eq!(conversation.submit("again"), None);

        conversation.reopen();
        assert_eq!(
            conversation.submit("again"),
            Some(Command::Prompt("again".to_owned()))
        );
        conversation.apply(AgentEvent::Opened {
            id: "session-1".to_owned(),
            how: Opened::Resumed,
        });
        assert_eq!(conversation.take_changed(), [2, 3]);
        assert_eq!(
            conversation.entries()[2..],
            [
                Entry::Human("again".to_owned()),
                Entry::Note("[resumed: the agent goes on from here]".to_owned()),
            ]
        );
    }

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
    fn shows_a_tool_call_as_running_until_it_ends() {
        let mut conversation = Conversation::new();
        conversation.submit("tool");
        let tool_call = |title: Option<&str>, result| AgentEvent::ToolCall {
            id: "call-1".to_owned(),
            title: title.map(str::to_owned),
            content: None,
            result,
        };

        conversation.apply(tool_call(Some("ls"), None));
        assert_eq!(
            conversation.entries()[1],
            Entry::ToolCall {
                id: "call-1".to_owned(),
                title: "ls".to_owned(),
                content: Vec::new(),
                result: None,
            }
        );

        conversation.apply(AgentEvent::ReplyText("listing".to_owned()));
        conversation.apply(tool_call(None, Some(ToolCallResult::Completed)));
        assert_eq!(
            conversation.entries()[1..],
            [
                Entry::ToolCall {
                    id: "call-1".to_owned(),
                    title: "ls".to_owned(),
                    content: Vec::new(),
                    result: Some(ToolCallResult::Completed),
                },
                Entry::Reply("listing".to_owned()),
            ]
        );
    }

    /// News of tool call `call-1`, showing `content` when it is given.
    fn tool_call(content: Option<Vec<Content>>, result: Option<ToolCallResult>) -> AgentEvent {
        AgentEvent::ToolCall {
            id: "call-1".to_owned(),
            title: Some("cargo xtask ci".to_owned()),
            content,
            result,
        }
    }

    /// What tool call `call-1` and the permission requests for it show.
    fn shown(conversation: &Conversation) -> Vec<(&'static str, Vec<Content>)> {
        conversation
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                Entry::ToolCall { content, .. } => Some(("call", content.clone())),
                Entry::Permission { content, .. } => Some(("permission", content.clone())),
                _ => None,
            })
            .collect()
    }

    fn text(text: &str) -> Content {
        Content::Text(text.to_owned())
    }

    fn permission(content: Option<Vec<Content>>) -> AgentEvent {
        AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("cargo xtask ci".to_owned()),
            content,
            options: vec!["Allow once".to_owned()],
        }
    }

    #[test]
    fn replaces_what_a_tool_call_shows_when_it_changes() {
        let mut conversation = Conversation::new();
        conversation.submit("tool");

        conversation.apply(tool_call(Some(vec![text("Run the checks")]), None));
        conversation.apply(tool_call(None, None));
        assert_eq!(
            shown(&conversation),
            [("call", vec![text("Run the checks")])]
        );

        conversation.apply(tool_call(
            Some(vec![text("error: 1 test failed")]),
            Some(ToolCallResult::Failed),
        ));
        assert_eq!(
            shown(&conversation),
            [("call", vec![text("error: 1 test failed")])]
        );
    }

    #[test]
    fn shows_what_a_permission_request_asks_for_only_there() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        let change = || {
            vec![Content::Diff {
                path: "/work/x.txt".to_owned(),
                lines: cox_core::diff::lines(Some("one\n"), "1\n"),
            }]
        };

        conversation.apply(tool_call(Some(change()), None));
        conversation.apply(permission(Some(change())));
        assert_eq!(
            shown(&conversation),
            [("call", Vec::new()), ("permission", change())]
        );

        conversation.choose(0);
        // Told again, the change still shows only where it was asked for.
        conversation.apply(tool_call(Some(change()), None));
        assert_eq!(
            shown(&conversation),
            [("call", Vec::new()), ("permission", change())]
        );
        conversation.apply(tool_call(
            Some(vec![text("done")]),
            Some(ToolCallResult::Completed),
        ));
        assert_eq!(
            shown(&conversation),
            [("call", vec![text("done")]), ("permission", change())]
        );
    }

    #[test]
    fn a_permission_request_that_tells_nothing_shows_what_its_call_shows() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");

        conversation.apply(tool_call(Some(vec![text("Run the checks")]), None));
        conversation.apply(permission(None));

        assert_eq!(
            shown(&conversation),
            [
                ("call", Vec::new()),
                ("permission", vec![text("Run the checks")])
            ]
        );
    }

    #[test]
    fn answers_a_permission_request_with_the_number_entered() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        conversation.apply(AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("touch x.txt".to_owned()),
            content: None,
            options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
        });

        assert_eq!(conversation.submit("3"), None);
        assert_eq!(
            conversation.submit("2"),
            Some(Command::Answer(PermissionAnswer::Select(1)))
        );
        assert_eq!(conversation.state(), State::Busy);
    }

    #[test]
    fn stops_a_turn_and_takes_the_next_line_once_it_has_ended() {
        let mut conversation = Conversation::new();
        assert_eq!(conversation.cancel(), None);
        conversation.submit("wait");
        conversation.apply(AgentEvent::ReplyText("waiting".to_owned()));

        assert_eq!(conversation.cancel(), Some(Command::Cancel));
        assert_eq!(conversation.state(), State::Stopping);
        assert_eq!(conversation.cancel(), None);
        assert_eq!(conversation.submit("again"), None);

        conversation.apply(AgentEvent::TurnEnded(StopReason::Cancelled));
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.submit("again"),
            Some(Command::Prompt("again".to_owned()))
        );
        assert_eq!(
            conversation.entries(),
            [
                Entry::Human("wait".to_owned()),
                Entry::Reply("waiting".to_owned()),
                Entry::Note("[Cancelled]".to_owned()),
                Entry::Human("again".to_owned()),
            ]
        );
    }

    #[test]
    fn stops_a_turn_that_waits_for_permission() {
        let mut conversation = Conversation::new();
        conversation.submit("permission");
        let request = || AgentEvent::PermissionRequest {
            id: "call-1".to_owned(),
            title: Some("touch x.txt".to_owned()),
            content: None,
            options: vec!["Allow once".to_owned(), "Reject once".to_owned()],
        };
        conversation.apply(request());

        assert_eq!(conversation.cancel(), Some(Command::Cancel));
        assert_eq!(conversation.choose(0), None);

        // A request the agent made before it heard it is to stop.
        conversation.apply(request());
        assert_eq!(conversation.state(), State::Stopping);
        assert_eq!(conversation.submit("1"), None);
    }

    #[test]
    fn tells_which_entries_changed_to_keep_them() {
        let mut conversation = Conversation::noting("[instructions: ~/cox/COX.md]".to_owned());
        assert_eq!(conversation.take_changed(), [0]);
        assert_eq!(conversation.take_changed(), Vec::<usize>::new());

        conversation.submit("hello");
        conversation.apply(AgentEvent::ReplyText("echo".to_owned()));
        assert_eq!(conversation.take_changed(), [1, 2]);
        conversation.apply(AgentEvent::ReplyText(": hello".to_owned()));
        assert_eq!(conversation.take_changed(), [2]);

        conversation.apply(tool_call(Some(vec![text("Run the checks")]), None));
        conversation.apply(AgentEvent::ReplyText("Running".to_owned()));
        assert_eq!(conversation.take_changed(), [3, 4]);
        // The request shows what the call showed, which the call then does
        // not.
        conversation.apply(permission(None));
        assert_eq!(conversation.take_changed(), [3, 5]);
        conversation.choose(0);
        assert_eq!(conversation.take_changed(), [5]);
        conversation.apply(tool_call(None, Some(ToolCallResult::Completed)));
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));
        assert_eq!(conversation.take_changed(), [3, 6]);
    }

    #[test]
    fn a_closed_chats_conversation_is_for_reading() {
        let mut conversation = Conversation::closed(vec![
            Entry::Human("hello".to_owned()),
            Entry::Reply("echo: hello".to_owned()),
        ]);

        assert_eq!(conversation.state(), State::Closed);
        assert_eq!(conversation.submit("again"), None);
        assert_eq!(conversation.cancel(), None);
        assert_eq!(conversation.entries().len(), 2);
        assert_eq!(conversation.take_changed(), Vec::<usize>::new());
    }
}
