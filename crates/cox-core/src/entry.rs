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
    /// The human, done looking at what the agent made since their last
    /// line, asked it to merge it, as the line after this says, which
    /// closes that work.
    Merge,
    /// The agent's reply, as much of it as has arrived.
    Reply(String),
    /// What the agent thought on its way to the reply, as much of it as
    /// has arrived.
    Thought(String),
    /// The plan the agent works to in a turn, as it last sent it.
    Plan(Vec<Step>),
    /// How the steps of the plan differ from those of the plan the agent
    /// first sent after the human's last line, which the human left it
    /// to: the steps it `added`, as what it decided on its own, and those
    /// it `removed`, as the plan stands now.
    PlanChanged {
        added: Vec<String>,
        removed: Vec<String>,
    },
    /// A picture or a video shown in the conversation, as a screenshot of
    /// what the agent made, for the human to see on the Mac or the phone:
    /// the copy Cox keeps of it, by the name of its `file`, of the `kind`
    /// it is, shown `from` the path it was at, with its `caption`, if it
    /// has one, and, for a picture, its `size` in pixels, when Cox could
    /// read it.
    Media {
        file: String,
        kind: MediaKind,
        from: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        caption: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<(u32, u32)>,
    },
    /// A file the agent showed the human on the stage, as a Markdown
    /// file to read or code at the lines it talks about, for the human
    /// to read in the conversation too, on the Mac or on the phone, where
    /// there is no stage: the copy Cox keeps of it as it was then, by the
    /// name of its `file`, shown from `path`, as the agent named it, and
    /// whether it is `markdown`, or else code at `lines`, if it named
    /// them, the first and the last, from 1.
    Shown {
        file: String,
        path: String,
        markdown: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<(usize, usize)>,
    },
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
        /// What the triggers that run as the agent asks gathered for the
        /// human to answer with, each as it went: what it said, the first
        /// line its headline.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        gathered: Vec<Check>,
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
    /// What runs as a turn ends, as the agent ends it: the chat's goal,
    /// then the triggers of the folder it works in, in order, each with
    /// how it went.
    Checks(Vec<Check>),
    /// What runs as something else happens in the chat, as its goal
    /// being met: the triggers of the folder it works in that run on it,
    /// `on`, as a trigger's `on` names it, as `goal.met`, each with how
    /// it went.
    Event { on: String, checks: Vec<Check> },
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
/// conversation, a tab of code or Markdown, a terminal, or a page, or a
/// region of the window, as a picture with what is in it.
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
    /// A picture of it, as a PNG file, by its absolute path: a region of
    /// the window the human chose, which `text` then says what is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<std::path::PathBuf>,
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

impl Entry {
    /// What ran, as a turn ended or as something else happened, if it is
    /// that.
    #[must_use]
    pub fn checks(&self) -> Option<&[Check]> {
        match self {
            Self::Checks(checks) | Self::Event { checks, .. } => Some(checks),
            _ => None,
        }
    }

    /// What ran, to change how it went, if it is that.
    pub fn checks_mut(&mut self) -> Option<&mut Vec<Check>> {
        match self {
            Self::Checks(checks) | Self::Event { checks, .. } => Some(checks),
            _ => None,
        }
    }
}

/// What a picture or a video in the conversation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
}

/// One of what runs as a turn ends, or as something else happens: the
/// chat's goal, or a trigger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// What it goes by: `goal`, or the trigger's name.
    pub name: String,
    pub state: CheckState,
    /// What it said, once it ran: what its command printed, at the end,
    /// or its agent's reply, or why it could not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
}

/// How one of what runs as a turn ends went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    /// Its turn has not come, and does not, once one before it failed.
    Waiting,
    Running,
    /// It passed: the goal is met, or the trigger found nothing.
    Passed,
    /// It failed, and the agent is told why: the goal is not met, or the
    /// trigger found something.
    Failed,
    /// It could not run, as when its agent did not start.
    Broken,
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
