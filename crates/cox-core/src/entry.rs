//! What a chat's conversation shows, entry by entry: what the human sent,
//! what Cox sent the agent in their place, what the agent replied and did,
//! and the notes Cox adds. It is what Cox keeps of a chat for the human to
//! read again.

use serde::{Deserialize, Serialize};

use crate::diff::Line;

/// One thing shown in the conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    /// A line the human sent.
    Human(String),
    /// A line Cox sent the agent in the human's place, as the agent's
    /// next turn: what the chat's goal asks while it is not met.
    Cox(String),
    /// The agent's reply, as much of it as has arrived.
    Reply(String),
    /// What the agent thought on its way to the reply, as much of it as
    /// has arrived.
    Thought(String),
    /// The plan the agent works to in a turn, as it last sent it.
    Plan(Vec<Step>),
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
        /// The option chosen, once one is: by the human, or by Cox in
        /// their place, as the chat's setup says.
        chosen: Option<usize>,
        /// Whether Cox chose, as the chat's setup says, so the human was
        /// not asked.
        #[serde(default)]
        by_setup: bool,
    },
    /// A goal the agent proposed for the chat, for the human to set as it
    /// is, to change before they set it, or to let be.
    Proposal {
        /// The goal, as the input that sets it reads: `$ ` and a command,
        /// or words.
        goal: String,
        /// Why the agent proposes it, if it said.
        why: Option<String>,
        /// What the human did with it, once they did.
        answer: Option<Answer>,
    },
    /// A note from Cox: the instructions the agent gets, how a turn ended,
    /// an error, or the connection closing.
    Note(String),
}

/// What the human did with a goal the agent proposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    /// Set it as it was proposed.
    Set,
    /// Changed it, and set what they changed it to.
    Changed,
    /// Let it be.
    Dismissed,
}

/// A step of the plan the agent works to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// What the step is to do.
    pub text: String,
    pub state: StepState,
}

/// How far a step of the plan has gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    InProgress,
    Completed,
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
