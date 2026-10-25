//! What a chat's conversation shows, entry by entry: what the human sent,
//! what the agent replied and did, and the notes Cox adds. It is what Cox
//! keeps of a chat for the human to read again.

use serde::{Deserialize, Serialize};

use crate::diff::Line;

/// One thing shown in the conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    /// A line the human sent.
    Human(String),
    /// The agent's reply, as much of it as has arrived.
    Reply(String),
    /// A tool call, running until it has a result.
    ToolCall {
        id: String,
        title: String,
        /// What the call shows, less what a permission request for it shows
        /// already.
        content: Vec<Content>,
        result: Option<ToolCallResult>,
    },
    Permission {
        /// The id of the tool call the agent asks to run.
        id: String,
        title: String,
        /// What the call is about to do, for the human to read before
        /// choosing.
        content: Vec<Content>,
        options: Vec<String>,
        /// The option the human chose, once they have.
        chosen: Option<usize>,
    },
    /// A note from Cox: the instructions the agent gets, how a turn ended,
    /// an error, or the connection closing.
    Note(String),
}

/// Something a tool call or a permission request shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Content {
    /// Text, such as what a command printed.
    Text(String),
    /// A change to the file at `path`.
    Diff { path: String, lines: Vec<Line> },
}

/// How a tool call ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallResult {
    Completed,
    Failed,
    /// The turn was cancelled before the call ended.
    Cancelled,
}
