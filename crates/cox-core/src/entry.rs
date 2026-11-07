//! What a chat's conversation shows, entry by entry: what the human sent,
//! what Cox sent the agent in their place, what the agent replied and did,
//! and the notes Cox adds. It is what Cox keeps of a chat for the human to
//! read again.

use serde::{Deserialize, Serialize};

use crate::diff::Line;
use crate::store::ChatId;

/// One thing shown in the conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    /// A line the human sent.
    Human(String),
    /// What the human handed the chat with the line before it, from
    /// elsewhere on the screen, as text selected in a tab.
    Attached(Vec<Attachment>),
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
    /// Cox asks the human whether to keep what a chat learned in the
    /// wiki, at a milestone of it, as when its goal is met, or a sketch
    /// ended.
    Keep {
        /// The milestone, as the card says it.
        why: String,
        /// The chat whose agent is to keep it, when it is another than
        /// this one, as the sketch that ended beside it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chat: Option<ChatId>,
        /// Whether the human chose to keep it, once they chose.
        #[serde(default)]
        kept: Option<bool>,
    },
    /// A note from Cox: the instructions the agent gets, how a turn ended,
    /// an error, or the connection closing.
    Note(String),
    /// The human switched the chat from the agent named `from` to the one
    /// named `to`, which goes on from the conversation so far.
    Switched { from: String, to: String },
    /// The agent asks the human to log in, one of the ways of `options`.
    Login {
        options: Vec<LoginOption>,
        /// Why the way the human chose last failed, when the agent asks
        /// again after one did.
        failed: Option<String>,
        /// The way chosen, once the human chose one.
        chosen: Option<usize>,
    },
}

/// Something on the screen the human hands a chat with their next line,
/// for its agent to read with it: text they selected, in the
/// conversation, a tab of code or Markdown, a terminal, or a page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// What the chip that shows it names it by, as where it is from:
    /// `src/main.rs:12-30`, or a page's title.
    pub label: String,
    /// Where it is from, as the agent is told: a file's `file:` URI, with
    /// the lines as `#L12-30`, a page's address, or `cox:` and what in
    /// Cox shows it, as `cox://terminal`.
    pub uri: String,
    /// What it says.
    pub text: String,
}

/// A way the agent offers the human to log in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// For a way the human carries out in a terminal, running the agent's
    /// command, what to add to it; `None` for one the agent carries out
    /// itself.
    pub terminal: Option<TerminalLogin>,
}

/// What to add to the agent's command, run in a terminal for the human to
/// log in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalLogin {
    pub args: Vec<String>,
    /// Over what the command sets of its environment.
    pub env: std::collections::BTreeMap<String, String>,
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
