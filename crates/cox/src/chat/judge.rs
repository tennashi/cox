//! The agent that judges whether a chat's goal in words is met: Cox
//! starts it when a turn of the chat ends, as the agent ends it, and asks
//! it, each time, whether the goal is met now, given what the chat last
//! said. It is an agent of the same command as the chat's, working in the
//! chat's folder, set up by Cox (see [`crate::agent::setup`]): told to
//! judge and to end its reply with a verdict, given no tools of Cox's own
//! and none for the stage, and allowed to read and to run what reads, with
//! every other permission request refused in the human's place. It is not
//! a chat: it is not in the list, and the human does not talk to it. What
//! it says reaches the chat as the verdict, with the judge's words.
//!
//! The judge lives as long as the chat's goal does: each judgement is a
//! turn of the same session, so it remembers what it looked at before.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use cox_acp::{PermissionAnswer, StopReason};
use tokio::sync::mpsc::UnboundedSender;

use crate::agent::instructions::Instructions;
use crate::agent::setup::{Policy, Setup, StageUse};
use crate::agent::{Agent, AgentEvent, AgentThread, Command, Start};

/// What starts an agent with no chat, working in a directory, as a
/// [`Start`] says: the judge of a goal.
pub type Starts = Rc<dyn Fn(PathBuf, Start) -> Agent>;

/// The file in a project's folder that replaces [`STANCE`] for the
/// project's judges, when there is one.
pub const STANCE_FILE: &str = ".cox/goal.md";

/// What the judge is told it is, ahead of the goal, unless the project's
/// [`STANCE_FILE`] says otherwise.
const STANCE: &str = "You judge whether another chat's work meets a goal. The chat works in \
    this same folder; you read what it did, and run what reads or tests it, but change \
    nothing: you may not edit or write files, and you may not run what changes the \
    project. Be strict about the goal as written, and no stricter. Each time you are \
    asked, say briefly what you looked at and what you found.";

/// How the judge ends its reply, so that Cox reads the verdict, and the
/// command that decides the goal, when the judge finds one.
const PROTOCOL: &str = "End every reply with one line of its own: `PASS` when the goal is \
    met, or `FAIL: ` and why it is not, in one sentence, which the working chat is told; or \
    `BLOCKED: ` and why, in one sentence, which the human is told, when the goal cannot be \
    judged as written, however the work goes on: when it asks what you may not run or \
    reach, as a deployment or a message sent out; when it is of how the work is done, not \
    of what it makes; or when it is more than one chat's work can meet, as a whole product \
    in place of its next step. \
    When one shell command, run in this folder, would decide the goal as written by \
    itself, passing exactly when the goal is met, now and as the work goes on, put a line \
    `COMMAND: ` and that command just before the verdict, which Cox offers the human to \
    judge the goal by in your place; you have run it, or would stake the verdict on it. \
    Leave the line out when part of the goal is a matter of judgement, or no command \
    decides it.";

/// What starts the line of a judge's reply that names the command that
/// decides the goal.
const COMMAND: &str = "COMMAND:";

/// The permission requests the judge's agent may go on with: reading and
/// searching, by ACP's kinds of tool, and running what reads or tests,
/// commands whose titles, as agents title them, are the command.
pub(crate) const ALLOWED: &[&str] = &[
    "read:*",
    "search:*",
    "execute:ls*",
    "execute:cat *",
    "execute:head *",
    "execute:tail *",
    "execute:find *",
    "execute:rg *",
    "execute:grep *",
    "execute:git status*",
    "execute:git diff*",
    "execute:git log*",
    "execute:git show*",
    "execute:cargo check*",
    "execute:cargo test*",
    "execute:cargo clippy*",
    "execute:cargo xtask ci*",
    "execute:npm test*",
    "execute:pytest*",
    "execute:make test*",
    "execute:go test*",
];

/// How a judge's reply judges the goal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Met, with what the judge said of it on the verdict's line, if
    /// anything.
    Met(String),
    /// Not met, with why, as the judge said.
    NotMet(String),
    /// Not to be judged as written, however the work goes on, with why,
    /// as the judge said: for the human to change the goal.
    Blocked(String),
}

impl Verdict {
    /// The verdict `reply` ends with: `PASS`, `FAIL` or `BLOCKED`, on its
    /// last line.
    /// A reply that ends with neither judges the goal not met, since the
    /// judge did not say it was.
    #[must_use]
    pub fn of(reply: &str) -> Self {
        let last = reply
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty());
        let Some(last) = last else {
            return Self::NotMet("it said nothing".to_owned());
        };
        let said = |rest: &str| {
            rest.trim_start_matches([':', ' ', '-', '`', '*', '_'])
                .trim()
                .to_owned()
        };
        let line = last.trim_matches(['`', '*', '_', ' ']);
        if let Some(rest) = strip_word(line, "PASS") {
            Self::Met(said(rest))
        } else if let Some(rest) = strip_word(line, "BLOCKED") {
            let why = said(rest);
            Self::Blocked(if why.is_empty() {
                "it said so, giving no reason".to_owned()
            } else {
                why
            })
        } else if let Some(rest) = strip_word(line, "FAIL") {
            let why = said(rest);
            Self::NotMet(if why.is_empty() {
                "it said so, giving no reason".to_owned()
            } else {
                why
            })
        } else {
            Self::NotMet("it gave no verdict".to_owned())
        }
    }
}

/// The command that `reply` says decides the goal, on a line of its own
/// that starts `COMMAND:`, if it names one: the last such line.
#[must_use]
pub fn command_of(reply: &str) -> Option<String> {
    reply.lines().rev().find_map(|line| {
        let line = line.trim().trim_start_matches(['`', '*', '_']);
        let command = line.strip_prefix(COMMAND)?;
        let command = command.trim().trim_matches(['`', '*', '_']).trim();
        let command = command.strip_prefix("$ ").unwrap_or(command).trim();
        (!command.is_empty()).then(|| command.to_owned())
    })
}

/// What follows `word` at the start of `line`, when `line` starts with
/// it, as a word of its own.
fn strip_word<'a>(line: &'a str, word: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(word)?;
    rest.chars()
        .next()
        .is_none_or(|next| !next.is_alphanumeric())
        .then_some(rest)
}

/// What the judges of the project in `folder` are told they are: what
/// its [`STANCE_FILE`] says, or [`STANCE`] when there is none.
#[must_use]
pub fn stance(folder: &Path) -> String {
    let path = folder.join(STANCE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => text.trim().to_owned(),
        Ok(_) => STANCE.to_owned(),
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::error!("cannot read {}: {error}", path.display());
            }
            STANCE.to_owned()
        }
    }
}

/// How the judge of `goal` is set up, told `stance` ahead of the goal.
#[must_use]
pub fn setup(goal: &str, stance: &str) -> Setup {
    Setup {
        context: Some(format!("{stance}\n\n{PROTOCOL}\n\nThe goal:\n\n{goal}\n")),
        tools: Some(Vec::new()),
        stage: StageUse::Look,
        policy: Policy {
            allow: ALLOWED.iter().map(|&title| title.to_owned()).collect(),
            reject: Vec::new(),
            within: None,
        },
        folder: None,
        sketch: None,
    }
}

/// What the judge is asked each time: whether `goal` is met now, given
/// what the chat last `said`.
#[must_use]
pub fn question(goal: &str, said: Option<&str>) -> String {
    let said = match said {
        Some(said) if !said.trim().is_empty() => {
            format!(
                "The chat working toward it last said:\n\n{}\n\n",
                said.trim()
            )
        }
        _ => "The chat working toward it said nothing this turn.\n\n".to_owned(),
    };
    format!(
        "Is the goal met now?\n\n{goal}\n\n{said}Look at the project as it is, and end with \
         PASS or FAIL: why."
    )
}

/// What the judge of a chat's goal said: a judgement, or that it closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Heard {
    /// The judge's reply to one question, or why it gave none.
    Judged(Result<String, String>),
    /// The judge's agent closed, as this says: the next judgement starts
    /// a judge anew, and the one under way, if any, is not coming.
    Closed(String),
}

/// The judge of a chat's goal: its agent, and what hears it.
pub struct Judge {
    commands: UnboundedSender<Command>,
    _thread: AgentThread,
    _hears: AbortOnDrop,
}

/// A task that stops when this is dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Judge {
    /// Starts the judge of `goal`, working in `cwd`, with `starts`, on the
    /// tokio `LocalSet` of the thread that calls it. What it says of each
    /// judgement goes to `heard`.
    pub fn start(
        starts: &Starts,
        cwd: PathBuf,
        goal: &str,
        heard: impl Fn(Heard) + 'static,
    ) -> Self {
        let setup = setup(goal, &stance(&cwd));
        Self::start_as(starts, cwd, setup, heard)
    }

    /// Starts an agent set up as `setup` says, working in `cwd`, which
    /// judges as the judge of a goal does, as a trigger's agent: what it
    /// says of each question goes to `heard`.
    pub fn start_as(
        starts: &Starts,
        cwd: PathBuf,
        setup: Setup,
        heard: impl Fn(Heard) + 'static,
    ) -> Self {
        let start = Start::New {
            instructions: Instructions::project(&cwd),
            setup: Some(setup),
        };
        let Agent {
            commands,
            mut events,
            thread,
            ..
        } = starts(cwd, start);
        let answers = commands.clone();
        let listens = tokio::task::spawn_local(async move {
            let mut reply = String::new();
            while let Some(event) = events.recv().await {
                let judged = match event {
                    AgentEvent::ReplyText(text) => {
                        reply.push_str(&text);
                        continue;
                    }
                    // The judge may not go on with what its setup does not
                    // allow, and there is no human to ask.
                    AgentEvent::PermissionRequest { .. } => {
                        let _ = answers.send(Command::Answer(PermissionAnswer::Cancel));
                        continue;
                    }
                    AgentEvent::TurnEnded(StopReason::EndTurn) => Ok(std::mem::take(&mut reply)),
                    AgentEvent::TurnEnded(StopReason::Cancelled) => {
                        reply.clear();
                        continue;
                    }
                    AgentEvent::TurnEnded(reason) => {
                        reply.clear();
                        Err(format!("its agent's turn ended early: {reason:?}"))
                    }
                    AgentEvent::TurnFailed(error) => {
                        reply.clear();
                        Err(format!("its agent's turn failed: {error}"))
                    }
                    AgentEvent::Closed(error) => {
                        let error = error.map_or_else(
                            || "its agent closed".to_owned(),
                            |error| format!("its agent closed: {error}"),
                        );
                        heard(Heard::Closed(error));
                        break;
                    }
                    _ => continue,
                };
                heard(Heard::Judged(judged));
            }
        });
        Self {
            commands,
            _thread: thread,
            _hears: AbortOnDrop(listens),
        }
    }

    /// Asks the judge whether `goal` is met now, given what the chat last
    /// `said`.
    pub fn ask(&self, goal: &str, said: Option<&str>) {
        let _ = self
            .commands
            .send(Command::Prompt(question(goal, said).into()));
    }

    /// Asks the agent `question`, as it is.
    pub fn put(&self, question: String) {
        let _ = self.commands.send(Command::Prompt(question.into()));
    }

    /// Stops the judgement under way, if one is.
    pub fn cancel(&self) {
        let _ = self.commands.send(Command::Cancel);
    }
}

#[cfg(test)]
mod tests {
    use cox_acp::ToolKind;

    use super::*;
    use crate::agent::setup::{Asked, Decision};

    #[test]
    fn the_verdict_is_the_replys_last_line() {
        assert_eq!(
            Verdict::of("I read main.rs.\n\nPASS\n"),
            Verdict::Met(String::new())
        );
        assert_eq!(
            Verdict::of("**PASS**: the greeting names the user."),
            Verdict::Met("the greeting names the user.".to_owned())
        );
        assert_eq!(
            Verdict::of("I read main.rs.\nFAIL: the greeting still says hello.\n\n"),
            Verdict::NotMet("the greeting still says hello.".to_owned())
        );
        assert_eq!(
            Verdict::of("`FAIL` - no tests"),
            Verdict::NotMet("no tests".to_owned())
        );
        assert_eq!(
            Verdict::of("FAIL"),
            Verdict::NotMet("it said so, giving no reason".to_owned())
        );
        // A word that only starts like one is no verdict.
        assert_eq!(
            Verdict::of("PASSWORD is set"),
            Verdict::NotMet("it gave no verdict".to_owned())
        );
        assert_eq!(
            Verdict::of("It looks fine to me."),
            Verdict::NotMet("it gave no verdict".to_owned())
        );
        assert_eq!(
            Verdict::of("  \n"),
            Verdict::NotMet("it said nothing".to_owned())
        );
        assert_eq!(
            Verdict::of("It asks for a deployment.\nBLOCKED: I cannot reach GCP."),
            Verdict::Blocked("I cannot reach GCP.".to_owned())
        );
        assert_eq!(
            Verdict::of("**BLOCKED**"),
            Verdict::Blocked("it said so, giving no reason".to_owned())
        );
    }

    #[test]
    fn the_judge_may_name_the_command_that_decides_the_goal() {
        let reply = "I ran the test.\n\nCOMMAND: `cargo test greeting`\nPASS";
        assert_eq!(command_of(reply), Some("cargo test greeting".to_owned()));
        // The verdict is read past it.
        assert_eq!(Verdict::of(reply), Verdict::Met(String::new()));
        assert_eq!(
            command_of("**COMMAND:** $ make check\nFAIL: it fails"),
            Some("make check".to_owned())
        );
        assert_eq!(command_of("I read it.\nPASS"), None);
        assert_eq!(command_of("COMMAND:  \nPASS"), None);
        // Only a line of its own names one.
        assert_eq!(command_of("No COMMAND: decides it.\nPASS"), None);
    }

    #[test]
    fn the_judge_is_set_up_to_read_and_not_to_change() {
        let setup = setup("The greeting names the user.", STANCE);
        let context = setup.context.unwrap();
        assert!(context.starts_with("You judge whether"), "{context}");
        assert!(context.contains("`PASS`") && context.contains("`FAIL: `"));
        assert!(context.contains("`COMMAND: `"), "{context}");
        assert!(context.ends_with("The goal:\n\nThe greeting names the user.\n"));
        assert_eq!(setup.tools, Some(Vec::new()));
        assert_eq!(setup.stage, StageUse::Look);
        let decide = |title, kind| {
            setup.policy.decide(Asked {
                title,
                kind: Some(kind),
                locations: &[],
            })
        };
        assert_eq!(
            decide("Read src/main.rs", ToolKind::Read),
            Some(Decision::Allow)
        );
        assert_eq!(decide("Find *.rs", ToolKind::Search), Some(Decision::Allow));
        assert_eq!(
            decide("cargo test --workspace", ToolKind::Execute),
            Some(Decision::Allow)
        );
        assert_eq!(decide("Edit src/main.rs", ToolKind::Edit), None);
        assert_eq!(decide("cargo publish", ToolKind::Execute), None);
        // A command's title that is not a command's call.
        assert_eq!(decide("cargo test", ToolKind::Other), None);
    }

    #[test]
    fn the_projects_own_stance_replaces_the_default() {
        let folder = std::env::temp_dir().join(format!("cox-judge-stance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join(".cox")).unwrap();
        assert_eq!(stance(&folder), STANCE);

        std::fs::write(folder.join(STANCE_FILE), " \n").unwrap();
        assert_eq!(stance(&folder), STANCE);

        std::fs::write(
            folder.join(STANCE_FILE),
            "You review the change as a maintainer would.\n",
        )
        .unwrap();
        assert_eq!(
            stance(&folder),
            "You review the change as a maintainer would."
        );
        let setup = setup("The tests pass.", &stance(&folder));
        let context = setup.context.unwrap();
        assert!(context.starts_with("You review the change as a maintainer would.\n\n"));
        assert!(context.contains("`PASS`"), "{context}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_question_tells_the_goal_and_what_the_chat_said() {
        let asked = question("The tests pass.", Some("I fixed the test.\n"));
        assert!(asked.starts_with("Is the goal met now?\n\nThe tests pass.\n\n"));
        assert!(asked.contains("last said:\n\nI fixed the test.\n\n"));
        assert!(asked.ends_with("end with PASS or FAIL: why."));
        assert!(question("x", None).contains("said nothing this turn"));
        assert!(question("x", Some(" ")).contains("said nothing this turn"));
    }
}
