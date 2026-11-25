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
    /// The human wrote the line before it, and what they handed with it,
    /// on a step of the agent's plan, from the Activity page: the step's
    /// thread there is of the line, and of what the agent replied to it.
    OnStep(OnStep),
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
    /// How the plan the agent sent differs from the one it sent before,
    /// in this turn or one before: a receipt of the change, written where
    /// it came, which tells why how far the plan has gone moved as it did.
    PlanRevised(Revision),
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
    /// A map of the project the agent showed the human on the stage, as
    /// `show_map` does, for the human to open where it showed it, on the
    /// Mac or on the phone: the `map`, by its name, unless the project's
    /// first, open `at`, the ids of the nodes it opened from the top
    /// down, and last of the node or the function chosen, if any.
    MapShown {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        map: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        at: Vec<String>,
    },
    /// A tool call, running until it has a result.
    ToolCall {
        id: String,
        title: String,
        /// Whether the call runs a command, as the agent's terminal tool
        /// does, whose title is then the command.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        command: bool,
        /// What the agent said a command is for, as the text the call
        /// showed while it ran, before what it printed took its place.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        about: Option<String>,
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
    /// The commits the agent made in a turn, as it ended, in the
    /// repositories of the folder the chat works in, newest first, each of
    /// which Cox marked in Git with the turn.
    Commits(Vec<Committed>),
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
    /// The agent asks the human `questions`, as Claude's `AskUserQuestion`
    /// does, for them to answer by choosing among options, or in their own
    /// words. Cox never answers it in their place.
    Question {
        /// What the agent says as it asks: the question itself, when it
        /// asks one.
        message: String,
        #[serde(default)]
        questions: Vec<Question>,
        /// How the human answered, once they did.
        #[serde(default)]
        answer: Option<QuestionAnswer>,
        /// When they answered, in milliseconds since the epoch: none for
        /// an answer an older Cox kept without it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answered_at: Option<i64>,
    },
    /// The agent asks the human to log in, one of the ways of `options`.
    Login {
        options: Vec<LoginOption>,
        /// Why the way the human chose last failed, when the agent asks
        /// again after one did.
        failed: Option<String>,
        /// The way chosen, once the human chose one.
        chosen: Option<usize>,
    },
    /// Something the agent proposed, for the human to answer. It is
    /// written as the kind of it is (see [`Written`]), so it stays last.
    #[serde(untagged)]
    Proposal(Proposal),
}

/// Something the agent proposed, for the human to carry out, change
/// first, as a goal, or let be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Written", into = "Written")]
pub struct Proposal {
    pub proposed: Proposed,
    /// Why the agent proposes it, if it said.
    pub why: Option<String>,
    /// What the human did with it, once they did.
    pub answer: Option<Answer>,
}

/// What the agent proposed, of each kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proposed {
    /// A goal for the chat, as the input that sets it reads: `$ ` and a
    /// command, or words.
    Goal(String),
    /// A name for the chat's project, which is its folder's.
    Name(String),
    /// What the chat's project is, in a sentence or two, for each chat's
    /// agent to be told of the project.
    About(String),
    /// An icon for the chat's project, in place of the one it has.
    Icon {
        /// The icon, as the rail would draw it, by the name of the copy
        /// Cox keeps of it, as of a picture shown (see [`Entry::Media`]).
        file: String,
        /// The path of the picture the agent made it from, as it gave it.
        from: String,
    },
    /// A project to make of a chat in none, named `name`, the chat moving
    /// into it.
    Project {
        /// The project's name, which its folder is named.
        name: String,
        /// The folder of the sketch from nothing whose files go into the
        /// project, as its directory, if the agent named one.
        sketch: Option<std::path::PathBuf>,
        /// The sketch's name, as the human's list shows it, if the agent
        /// named one.
        sketch_name: Option<String>,
    },
}

impl Proposed {
    /// The goal, of a goal proposed, as its input reads, if it reads as
    /// one.
    #[must_use]
    pub fn goal(&self) -> Option<crate::goal::Goal> {
        match self {
            Self::Goal(goal) => crate::goal::Goal::parse(goal),
            _ => None,
        }
    }
}

/// A [`Proposal`] as Cox writes it, each kind under a name of its own,
/// which chats on disk and the phone's page read it by.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Written {
    Proposal {
        goal: String,
        why: Option<String>,
        answer: Option<Answer>,
    },
    NameProposal {
        name: String,
        why: Option<String>,
        answer: Option<Answer>,
    },
    AboutProposal {
        about: String,
        why: Option<String>,
        answer: Option<Answer>,
    },
    IconProposal {
        file: String,
        from: String,
        why: Option<String>,
        answer: Option<Answer>,
    },
    ProjectProposal {
        name: String,
        why: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sketch: Option<std::path::PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sketch_name: Option<String>,
        answer: Option<Answer>,
    },
}

impl From<Written> for Proposal {
    fn from(written: Written) -> Self {
        let (proposed, why, answer) = match written {
            Written::Proposal { goal, why, answer } => (Proposed::Goal(goal), why, answer),
            Written::NameProposal { name, why, answer } => (Proposed::Name(name), why, answer),
            Written::AboutProposal { about, why, answer } => (Proposed::About(about), why, answer),
            Written::IconProposal {
                file,
                from,
                why,
                answer,
            } => (Proposed::Icon { file, from }, why, answer),
            Written::ProjectProposal {
                name,
                why,
                sketch,
                sketch_name,
                answer,
            } => (
                Proposed::Project {
                    name,
                    sketch,
                    sketch_name,
                },
                why,
                answer,
            ),
        };
        Self {
            proposed,
            why,
            answer,
        }
    }
}

impl From<Proposal> for Written {
    fn from(
        Proposal {
            proposed,
            why,
            answer,
        }: Proposal,
    ) -> Self {
        match proposed {
            Proposed::Goal(goal) => Self::Proposal { goal, why, answer },
            Proposed::Name(name) => Self::NameProposal { name, why, answer },
            Proposed::About(about) => Self::AboutProposal { about, why, answer },
            Proposed::Icon { file, from } => Self::IconProposal {
                file,
                from,
                why,
                answer,
            },
            Proposed::Project {
                name,
                sketch,
                sketch_name,
            } => Self::ProjectProposal {
                name,
                why,
                sketch,
                sketch_name,
                answer,
            },
        }
    }
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

/// A commit the agent made in a turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Committed {
    /// The folder the chat worked in, whose changes show the commit.
    pub folder: std::path::PathBuf,
    /// The directory of the folder's project the commit is in, by name, or
    /// none for the repository the folder is in itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Its full name.
    pub id: String,
    /// The first line of its message.
    pub subject: String,
    /// How many lines its files gained, and lost, in all.
    pub added: usize,
    pub removed: usize,
    /// The commit it amended, by its full name, if it was made by amending
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amended: Option<String>,
}

impl Committed {
    /// Its short name, as people read it.
    #[must_use]
    pub fn short(&self) -> &str {
        self.id.get(..7).unwrap_or(&self.id)
    }

    /// The short name of the commit it amended, if it amended one.
    #[must_use]
    pub fn amended_short(&self) -> Option<&str> {
        self.amended
            .as_deref()
            .map(|amended| amended.get(..7).unwrap_or(amended))
    }
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

/// What the human did with something the agent proposed.
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

/// One question the agent asks the human (see [`Entry::Question`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// A short label for it, as "Approach", if the agent gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// The question, when the message the agent asks with does not say it,
    /// as when it asks several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// What the human chooses among: none for a question they answer in
    /// their own words alone.
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    /// Whether the human may choose more than one option.
    #[serde(default)]
    pub multiple: bool,
    /// Whether the human may answer in their own words, in place of an
    /// option, or beside the options they chose.
    #[serde(default)]
    pub other: bool,
}

/// An option of a [`Question`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    /// What choosing it means, if the agent said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// How the human answered the questions of an [`Entry::Question`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionAnswer {
    /// What they picked of each question, in the order of the questions.
    Given(Vec<Picked>),
    /// They chose not to answer.
    Declined,
}

/// What the human picked of one [`Question`]: the options at `chosen`,
/// at most one where the question takes one, and their own words, if
/// they wrote any. Neither, for a question they skipped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picked {
    #[serde(default)]
    pub chosen: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
}

impl Question {
    /// Whether `picked` answers this question as it may be answered: each
    /// option chosen is one of its own, chosen once, one at most unless
    /// it takes more, and words only where it takes them.
    #[must_use]
    pub fn takes(&self, picked: &Picked) -> bool {
        let mut chosen = picked.chosen.clone();
        chosen.sort_unstable();
        chosen.dedup();
        chosen.len() == picked.chosen.len()
            && chosen.iter().all(|&index| index < self.options.len())
            && (self.multiple || chosen.len() <= 1)
            && (self.other || picked.other.is_none())
    }

    /// What `picked` says of this question, as a line reads it: the
    /// labels of the options chosen, then the human's own words, if any.
    #[must_use]
    pub fn said(&self, picked: &Picked) -> String {
        let mut said: Vec<&str> = picked
            .chosen
            .iter()
            .filter_map(|&index| self.options.get(index))
            .map(|choice| choice.label.as_str())
            .collect();
        if let Some(other) = &picked.other {
            said.push(other);
        }
        said.join(", ")
    }
}

impl QuestionAnswer {
    /// How the human answered `questions`, as a line reads it.
    #[must_use]
    pub fn said(&self, questions: &[Question]) -> String {
        let Self::Given(picked) = self else {
            return "declined".to_owned();
        };
        let said: Vec<String> = questions
            .iter()
            .zip(picked)
            .map(|(question, picked)| question.said(picked))
            .map(|said| {
                if said.is_empty() {
                    "skipped".to_owned()
                } else {
                    said
                }
            })
            .collect();
        if said.is_empty() {
            "answered".to_owned()
        } else {
            said.join("; ")
        }
    }
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
    /// When coxd first saw it in its state, in milliseconds since the
    /// epoch: when it was done, once it is. None for a step an older Cox
    /// kept, and one that was so in a plan an older Cox kept before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    /// When coxd first saw it under way, in milliseconds since the epoch,
    /// kept once it is done, for when the work on it started: None for a
    /// step not started, one done without coxd seeing it under way, and
    /// one an older Cox kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<i64>,
    /// What coxd knows it by in its chat, from 1, which it keeps as the
    /// agent rewords it, for what is said of it to stay with it. None
    /// for a step an older Cox kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
}

/// How far a plan has gone: how many of its steps are done, of how many
/// it has. Every place that tells how far a plan went counts it so, and
/// the pages of the stage are told it rather than count it again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepsDone {
    pub done: usize,
    pub of: usize,
}

impl StepsDone {
    /// How far the plan of `steps` has gone.
    #[must_use]
    pub fn count(steps: &[Step]) -> Self {
        Self {
            done: steps
                .iter()
                .filter(|step| step.state == StepState::Completed)
                .count(),
            of: steps.len(),
        }
    }

    /// Whether the plan has steps, and every one of them is done.
    #[must_use]
    pub fn finished(self) -> bool {
        self.of > 0 && self.done == self.of
    }

    /// How many of its steps are not done.
    #[must_use]
    pub fn left(self) -> usize {
        self.of - self.done
    }

    /// How much of the plan is done, in whole percents, rounded down, as
    /// 62 for 5 of 8: none for a plan with no steps.
    #[must_use]
    pub fn percent(self) -> Option<usize> {
        (self.of > 0).then(|| self.done * 100 / self.of)
    }

    /// How much of the plan is done, as "62%", or "–" for one with no
    /// steps.
    #[must_use]
    pub fn percent_text(self) -> String {
        self.percent()
            .map_or_else(|| "–".to_owned(), |percent| format!("{percent}%"))
    }
}

impl std::fmt::Display for StepsDone {
    /// As "2/3".
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.done, self.of)
    }
}

/// How a plan differs from the one before it, as [`Entry::PlanRevised`]
/// keeps it: how far each had gone, and the steps that tell why it moved
/// as it did. A step is the one before it was when coxd knows it by the
/// same id (see [`Step::id`]), as it does when it reads the same, or
/// alike enough to be the step reworded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// How far the plan before had gone.
    pub before: StepsDone,
    /// How far the plan has gone now.
    pub after: StepsDone,
    /// The steps the plan gained, as they read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    /// The steps it lost, as they read before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// The steps the agent reworded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reworded: Vec<Reworded>,
    /// The steps done before that are not now, as they read now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undone: Vec<String>,
    /// How many times the plan changed so between `before` and `after`:
    /// once, unless the turn kept as many receipts as it keeps, when its
    /// last takes in each change after it.
    #[serde(default = "once", skip_serializing_if = "is_once")]
    pub changes: usize,
}

/// A step of a plan the agent reworded: as it read `from`, and reads
/// now, `to`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reworded {
    pub from: String,
    pub to: String,
}

fn once() -> usize {
    1
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes the field by reference"
)]
fn is_once(changes: &usize) -> bool {
    *changes == 1
}

impl Revision {
    /// How `after` differs from `before`, each step of `after` the one of
    /// `before` that `matched` names by its place there, if any: empty
    /// when no step came, went, was reworded or was undone, as when steps
    /// only went further, or moved.
    #[must_use]
    pub fn between(before: &[Step], after: &[Step], matched: &[Option<usize>]) -> Self {
        let mut revision = Self {
            before: StepsDone::count(before),
            after: StepsDone::count(after),
            added: Vec::new(),
            removed: Vec::new(),
            reworded: Vec::new(),
            undone: Vec::new(),
            changes: 1,
        };
        let mut kept = vec![false; before.len()];
        for (step, matched) in after.iter().zip(matched) {
            let Some(was) = matched.and_then(|at| before.get(at)) else {
                revision.added.push(step.text.clone());
                continue;
            };
            kept[matched.unwrap_or_default()] = true;
            if was.text != step.text {
                revision.reworded.push(Reworded {
                    from: was.text.clone(),
                    to: step.text.clone(),
                });
            }
            if was.state == StepState::Completed && step.state != StepState::Completed {
                revision.undone.push(step.text.clone());
            }
        }
        revision.removed = before
            .iter()
            .zip(kept)
            .filter(|(_, kept)| !kept)
            .map(|(step, _)| step.text.clone())
            .collect();
        revision
    }

    /// Whether it tells no step that came, went, was reworded or undone,
    /// as one that takes in changes that undid each other.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.reworded.is_empty()
            && self.undone.is_empty()
    }

    /// Whether less of the plan is done after than before, as the
    /// percents of [`Self::headline`] read.
    #[must_use]
    pub fn dropped(&self) -> bool {
        matches!(
            (self.before.percent(), self.after.percent()),
            (Some(before), Some(after)) if after < before
        )
    }

    /// What it tells, in a line: the steps that came and went, how many
    /// were reworded and undone, and how much of the plan was done before
    /// and is now, as "+3 steps · 80% → 62%".
    #[must_use]
    pub fn headline(&self) -> String {
        let counted = |count: usize, one: &str, more: &str| {
            format!("{count} {}", if count == 1 { one } else { more })
        };
        let mut told: Vec<String> = Vec::new();
        if !self.added.is_empty() {
            told.push(format!("+{}", counted(self.added.len(), "step", "steps")));
        }
        if !self.removed.is_empty() {
            told.push(format!("−{}", counted(self.removed.len(), "step", "steps")));
        }
        if !self.reworded.is_empty() {
            told.push(format!("{} reworded", self.reworded.len()));
        }
        if !self.undone.is_empty() {
            told.push(format!("{} undone", self.undone.len()));
        }
        if told.is_empty() {
            told.push("back as it was".to_owned());
        }
        format!("{} · {}", told.join(", "), self.progress())
    }

    /// How much of the plan was done before and is now, and over how
    /// many changes, once more than one, as "80% → 62%".
    #[must_use]
    pub fn progress(&self) -> String {
        let over = if self.changes > 1 {
            format!(" · over {} changes", self.changes)
        } else {
            String::new()
        };
        format!(
            "{} → {}{over}",
            self.before.percent_text(),
            self.after.percent_text()
        )
    }

    /// What it tells past how the plan stands against the one the human
    /// left the agent to, as [`Entry::PlanChanged`] tells it with the
    /// steps `added` and `removed` there, when the same change wrote
    /// both: each step reworded and undone, and the steps that came and
    /// went that are not so there, as one back in the human's plan, or
    /// one the agent had added and took out again.
    #[must_use]
    pub fn past_drift(&self, added: &[String], removed: &[String]) -> Self {
        Self {
            added: self
                .added
                .iter()
                .filter(|step| !added.contains(step))
                .cloned()
                .collect(),
            removed: self
                .removed
                .iter()
                .filter(|step| !removed.contains(step))
                .cloned()
                .collect(),
            ..self.clone()
        }
    }
}

/// A step of the agent's plan a line of the human's is on: the step the
/// chat knows by `id` (see [`Step::id`]), as it read as they wrote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnStep {
    pub id: u64,
    pub text: String,
}

/// How alike `one` and `other` read, from 0 to 1: the Dice coefficient of
/// the pairs of letters in each of their words, which holds for words
/// written without spaces between them, as Japanese is, too.
#[must_use]
pub fn alike(one: &str, other: &str) -> f64 {
    let pairs = |text: &str| -> Vec<(char, char)> {
        let mut pairs: Vec<(char, char)> = text
            .split(|c: char| !c.is_alphanumeric())
            .flat_map(|word| {
                let letters: Vec<char> = word.chars().flat_map(char::to_lowercase).collect();
                match letters.as_slice() {
                    [] => Vec::new(),
                    [only] => vec![(*only, ' ')],
                    _ => letters.windows(2).map(|pair| (pair[0], pair[1])).collect(),
                }
            })
            .collect();
        pairs.sort_unstable();
        pairs
    };
    let (one, other) = (pairs(one), pairs(other));
    if one.is_empty() || other.is_empty() {
        return 0.0;
    }
    let (mut both, mut at) = (0_u32, 0);
    for pair in &one {
        while at < other.len() && other[at] < *pair {
            at += 1;
        }
        if at < other.len() && other[at] == *pair {
            both += 1;
            at += 1;
        }
    }
    let all = u32::try_from(one.len() + other.len()).unwrap_or(u32::MAX);
    f64::from(2 * both) / f64::from(all)
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

/// The files the agent changed in `entries`, by the paths it named them
/// by, each once, in the order it first changed them: those of the
/// changes its tool calls that completed show, or that the permission
/// asked for one showed.
#[must_use]
pub fn written(entries: &[Entry]) -> Vec<String> {
    let completed: std::collections::HashSet<&str> = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::ToolCall {
                id,
                result: Some(ToolCallResult::Completed),
                ..
            } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut written: Vec<String> = Vec::new();
    for entry in entries {
        let (Entry::ToolCall { id, content, .. } | Entry::Permission { id, content, .. }) = entry
        else {
            continue;
        };
        if !completed.contains(id.as_str()) {
            continue;
        }
        for content in content {
            if let Content::Diff { path, .. } = content
                && !written.contains(path)
            {
                written.push(path.clone());
            }
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(path: &str) -> Content {
        Content::Diff {
            path: path.to_owned(),
            lines: Vec::new(),
        }
    }

    fn step(text: &str, state: StepState) -> Step {
        Step {
            text: text.to_owned(),
            state,
            since: None,
            started: None,
            id: None,
        }
    }

    #[test]
    fn how_far_a_plan_went_counts_its_steps_done_of_all() {
        let plan = [
            step("Read", StepState::Completed),
            step("Fix", StepState::InProgress),
            step("Test", StepState::Pending),
        ];
        let done = StepsDone::count(&plan);
        assert_eq!(done, StepsDone { done: 1, of: 3 });
        assert_eq!(done.to_string(), "1/3");
        assert_eq!(done.left(), 2);
        assert!(!done.finished());
        assert!(StepsDone::count(&plan[..1]).finished());
        // A plan with no steps is not finished, and has none left.
        let none = StepsDone::count(&[]);
        assert_eq!((none.to_string().as_str(), none.left()), ("0/0", 0));
        assert!(!none.finished());
        assert_eq!(
            serde_json::to_value(done).unwrap(),
            serde_json::json!({ "done": 1, "of": 3 })
        );
    }

    #[test]
    fn a_revision_tells_the_steps_that_came_went_were_reworded_or_undone() {
        let before = [
            step("Read the code", StepState::Completed),
            step("Fix", StepState::Completed),
            step("Test", StepState::Completed),
            step("Lint", StepState::Completed),
            step("Commit", StepState::Pending),
        ];
        let after = [
            step("Read all the code", StepState::Completed),
            step("Fix", StepState::InProgress),
            step("Test", StepState::Completed),
            step("Lint", StepState::Completed),
            step("Write the docs", StepState::Pending),
            step("Push", StepState::Pending),
            step("Open a pull request", StepState::Pending),
        ];
        let matched = [Some(0), Some(1), Some(2), Some(3), None, None, None];
        let revision = Revision::between(&before, &after, &matched);
        assert_eq!(
            revision,
            Revision {
                before: StepsDone { done: 4, of: 5 },
                after: StepsDone { done: 3, of: 7 },
                added: vec![
                    "Write the docs".to_owned(),
                    "Push".to_owned(),
                    "Open a pull request".to_owned(),
                ],
                removed: vec!["Commit".to_owned()],
                reworded: vec![Reworded {
                    from: "Read the code".to_owned(),
                    to: "Read all the code".to_owned(),
                }],
                undone: vec!["Fix".to_owned()],
                changes: 1,
            }
        );
        assert!(revision.dropped());
        assert_eq!(
            revision.headline(),
            "+3 steps, −1 step, 1 reworded, 1 undone · 80% → 42%"
        );
        // Kept as it reads, with no list it has nothing in.
        let kept = serde_json::to_value(Entry::PlanRevised(Revision {
            reworded: Vec::new(),
            undone: Vec::new(),
            ..revision
        }))
        .unwrap();
        assert_eq!(
            kept,
            serde_json::json!({ "plan_revised": {
                "before": { "done": 4, "of": 5 },
                "after": { "done": 3, "of": 7 },
                "added": ["Write the docs", "Push", "Open a pull request"],
                "removed": ["Commit"],
            } })
        );
        assert_eq!(
            serde_json::from_value::<Entry>(kept.clone())
                .map(|entry| serde_json::to_value(entry).unwrap())
                .unwrap(),
            kept
        );
    }

    #[test]
    fn past_the_drift_a_revision_tells_what_the_drift_does_not() {
        let revision = Revision {
            before: StepsDone { done: 2, of: 4 },
            after: StepsDone { done: 1, of: 4 },
            added: vec!["Push".to_owned(), "Lint".to_owned()],
            removed: vec!["Try it".to_owned(), "Commit".to_owned()],
            reworded: vec![Reworded {
                from: "Write the docs".to_owned(),
                to: "Write the docs for --shout".to_owned(),
            }],
            undone: vec!["Test".to_owned()],
            changes: 1,
        };
        // The agent added "Push" on its own, and "Lint" is back as the
        // human planned it; "Commit" was the human's, "Try it" the
        // agent's own.
        let past = revision.past_drift(
            &["Push".to_owned(), "Write the docs for --shout".to_owned()],
            &["Commit".to_owned(), "Write the docs".to_owned()],
        );
        assert_eq!(
            past,
            Revision {
                added: vec!["Lint".to_owned()],
                removed: vec!["Try it".to_owned()],
                ..revision.clone()
            }
        );
        assert_eq!(past.progress(), "50% → 25%");
    }

    #[test]
    fn a_plan_whose_steps_only_went_further_or_moved_is_not_revised() {
        let before = [
            step("Read", StepState::InProgress),
            step("Fix", StepState::Pending),
        ];
        let after = [
            step("Fix", StepState::InProgress),
            step("Read", StepState::Completed),
        ];
        assert!(Revision::between(&before, &after, &[Some(1), Some(0)]).is_empty());
        // Taking steps out raises how much is done, which is no drop.
        let fewer = Revision::between(&before, &after[1..], &[Some(0)]);
        assert!(!fewer.dropped());
        assert_eq!(fewer.headline(), "−1 step · 0% → 100%");
        let empty = Revision {
            changes: 4,
            ..Revision::between(&[], &before, &[None, None])
        };
        assert_eq!(empty.headline(), "+2 steps · – → 0% · over 4 changes");
    }

    #[test]
    fn the_files_written_are_those_of_the_calls_that_completed() {
        let entries = [
            // Asked for first, its change shown on the permission only.
            Entry::Permission {
                id: "1".to_owned(),
                title: "Edit".to_owned(),
                content: vec![diff("/work/a.rs")],
                options: Vec::new(),
                chosen: Some(0),
                by_setup: false,
                gathered: Vec::new(),
            },
            Entry::ToolCall {
                id: "1".to_owned(),
                title: "Edit".to_owned(),
                command: false,
                about: None,
                content: Vec::new(),
                result: Some(ToolCallResult::Completed),
            },
            Entry::ToolCall {
                id: "2".to_owned(),
                title: "Edit".to_owned(),
                command: false,
                about: None,
                content: vec![diff("/work/b.rs"), diff("/work/a.rs")],
                result: Some(ToolCallResult::Completed),
            },
            Entry::ToolCall {
                id: "3".to_owned(),
                title: "Edit".to_owned(),
                command: false,
                about: None,
                content: vec![diff("/work/failed.rs")],
                result: Some(ToolCallResult::Failed),
            },
            Entry::ToolCall {
                id: "4".to_owned(),
                title: "Edit".to_owned(),
                command: false,
                about: None,
                content: vec![diff("/work/running.rs")],
                result: None,
            },
        ];
        assert_eq!(written(&entries), ["/work/a.rs", "/work/b.rs"]);
    }

    #[test]
    fn a_question_reads_as_written_and_as_kept_with_less() {
        let entry = Entry::Question {
            message: "Which?".to_owned(),
            questions: vec![Question {
                header: Some("Pick".to_owned()),
                text: None,
                options: vec![QuestionOption {
                    label: "A".to_owned(),
                    description: Some("the first".to_owned()),
                }],
                multiple: false,
                other: true,
            }],
            answer: Some(QuestionAnswer::Given(vec![Picked {
                chosen: vec![0],
                other: None,
            }])),
            answered_at: Some(1_700_000_000_000),
        };
        let written = serde_json::to_string(&entry).unwrap();
        assert_eq!(
            written,
            r#"{"question":{"message":"Which?","questions":[{"header":"Pick","options":[{"label":"A","description":"the first"}],"multiple":false,"other":true}],"answer":{"given":[{"chosen":[0]}]},"answered_at":1700000000000}}"#
        );
        assert_eq!(serde_json::from_str::<Entry>(&written).unwrap(), entry);
        let declined: Entry =
            serde_json::from_str(r#"{"question":{"message":"Which?","answer":"declined"}}"#)
                .unwrap();
        assert_eq!(
            declined,
            Entry::Question {
                message: "Which?".to_owned(),
                questions: Vec::new(),
                answer: Some(QuestionAnswer::Declined),
                answered_at: None,
            }
        );
    }

    #[test]
    fn a_map_shown_is_written_with_what_it_says_alone() {
        let entry = Entry::MapShown {
            map: Some("use".to_owned()),
            at: vec!["cox".to_owned(), "cox/coxd".to_owned()],
        };
        let written = serde_json::to_string(&entry).unwrap();
        assert_eq!(
            written,
            r#"{"map_shown":{"map":"use","at":["cox","cox/coxd"]}}"#
        );
        assert_eq!(serde_json::from_str::<Entry>(&written).unwrap(), entry);
        assert_eq!(
            serde_json::from_str::<Entry>(r#"{"map_shown":{}}"#).unwrap(),
            Entry::MapShown {
                map: None,
                at: Vec::new()
            },
            "the project's first map, at its top"
        );
    }

    /// What Cox has written of each proposal, as chats on disk and the
    /// phone's page hold it.
    const PROPOSALS: [&str; 8] = [
        r#"{"proposal":{"goal":"$ make","why":"make builds it","answer":null}}"#,
        r#"{"proposal":{"goal":"The greeting names the user.","why":null,"answer":"changed"}}"#,
        r#"{"name_proposal":{"name":"shop","why":"It sells things.","answer":"set"}}"#,
        r#"{"name_proposal":{"name":"market","why":null,"answer":null}}"#,
        r#"{"about_proposal":{"about":"A shop that sells tea.","why":null,"answer":"dismissed"}}"#,
        r#"{"icon_proposal":{"file":"icon-1.png","from":"logo.svg","why":"The teal of the shop.","answer":null}}"#,
        r#"{"project_proposal":{"name":"market","why":null,"answer":null}}"#,
        r#"{"project_proposal":{"name":"shop","why":"We agreed.","sketch":"/home/sketches/tea","sketch_name":"tea shop","answer":"set"}}"#,
    ];

    #[test]
    fn a_proposal_reads_and_writes_as_cox_has_written_it() {
        let proposal = |proposed, why: Option<&str>, answer| {
            Entry::Proposal(Proposal {
                proposed,
                why: why.map(str::to_owned),
                answer,
            })
        };
        let read = [
            proposal(
                Proposed::Goal("$ make".to_owned()),
                Some("make builds it"),
                None,
            ),
            proposal(
                Proposed::Goal("The greeting names the user.".to_owned()),
                None,
                Some(Answer::Changed),
            ),
            proposal(
                Proposed::Name("shop".to_owned()),
                Some("It sells things."),
                Some(Answer::Set),
            ),
            proposal(Proposed::Name("market".to_owned()), None, None),
            proposal(
                Proposed::About("A shop that sells tea.".to_owned()),
                None,
                Some(Answer::Dismissed),
            ),
            proposal(
                Proposed::Icon {
                    file: "icon-1.png".to_owned(),
                    from: "logo.svg".to_owned(),
                },
                Some("The teal of the shop."),
                None,
            ),
            proposal(
                Proposed::Project {
                    name: "market".to_owned(),
                    sketch: None,
                    sketch_name: None,
                },
                None,
                None,
            ),
            proposal(
                Proposed::Project {
                    name: "shop".to_owned(),
                    sketch: Some("/home/sketches/tea".into()),
                    sketch_name: Some("tea shop".to_owned()),
                },
                Some("We agreed."),
                Some(Answer::Set),
            ),
        ];
        for (written, read) in PROPOSALS.into_iter().zip(read) {
            let entry: Entry = serde_json::from_str(written).unwrap();
            assert_eq!(entry, read, "{written}");
            assert_eq!(serde_json::to_string(&entry).unwrap(), written);
        }
    }
}
