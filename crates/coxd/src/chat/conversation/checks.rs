//! A chat can have a [`Goal`](super::Goal): a command, which the chat works until it
//! passes, or words, which it works until an agent judges them met (see
//! [`crate::chat::judge`]). When a turn ends as the agent ends it, the
//! command runs, or the judge is asked; while the goal is not met, Cox
//! sends the agent what the command printed, or what the judge said, as
//! the next turn, and the agent goes on, until the goal is met, or the
//! chat has taken as many turns on the human's line as one line gets, or
//! the agent stops calling tools, which says it waits for the human.
//!
//! The triggers of the folder the chat works in that are due as the turn
//! ends (see [`crate::trigger`]) run after the goal, the same way, each a
//! [`Due`] that the host hands the conversation ahead of the turn's end:
//! the first that fails stops them, and the agent is told what it found.
//! The conversation shows them, with the goal, on one line of chips (see
//! [`Entry::Checks`]).
//!
//! The triggers that run as something else happens in the chat, as its
//! goal being met, the host hands the conversation as it happens (see
//! [`Conversation::run_event`]), and they run once the chat waits for the
//! human, all of them, on a line of their own (see [`Entry::Event`]):
//! what they find stays there, and the agent is not told it. A trigger
//! `by: this` tells the chat's agent what it says, as the next turn, in
//! the human's place, once those that run with it went.

use std::collections::VecDeque;
use std::path::PathBuf;

use cox_core::goal;

use super::{
    Check, CheckState, Conversation, Entry, GOAL_TURNS, OUTPUT_TAIL, Proposal, Proposed, State,
    tail,
};
use crate::agent::Command;
use crate::agent::mcp::{Ended, Ran};
use crate::chat::judge::{self, Verdict};

/// How many turns in a row without a tool call stop a chat working toward
/// its goal: an agent that only talks waits for the human.
const IDLE_TURNS: usize = 2;

/// The milestone of a chat whose goal is met, as the card that asks
/// whether to keep what it learned says it.
const GOAL_MET: &str = "The goal is met.";

/// The event of the goal being met, as a trigger's `on` names it.
const GOAL_MET_EVENT: &str = "goal.met";

/// A trigger due as a turn ends, which runs after the chat's goal (see
/// [`crate::trigger`]): what it goes by, and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Due {
    pub name: String,
    /// Its file, which tells it from another of the same name.
    pub file: std::path::PathBuf,
    pub does: Does,
}

/// What a trigger due does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Does {
    /// Runs the command, as the goal's would, passing when it exits 0.
    Run(String),
    /// Asks an agent of its own what the trigger says, which ends its
    /// reply with `PASS`, or `FAIL: ` and why, as a judge's.
    Ask(String),
    /// Tells the chat's agent what the trigger says, as the next turn,
    /// once what runs with it went: once for a line of the human's, as a
    /// turn ends.
    Tell(String),
}

/// What happened in a chat, besides a turn's end, whose triggers run:
/// the event, as a trigger's `on` names it, as `goal.met`, and what the
/// agent of a trigger is told of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Happened {
    pub on: String,
    pub about: String,
}

/// What runs now, as the turn ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum At {
    Goal,
    Trigger(Due),
}

/// What runs as a turn ended, or as something else happened, while it
/// does: as the chat checks, and as the agent asks the human in the
/// middle of it, which it goes on with once they answered.
#[derive(Debug)]
pub(super) struct Checks {
    /// What runs now.
    at: At,
    /// The triggers due that have not run yet.
    rest: VecDeque<Due>,
    /// The entry that shows what runs, and the place in it of what runs
    /// now.
    marks: (usize, usize),
    /// What happened whose triggers run, or `None` for those of a turn's
    /// end.
    event: Option<Happened>,
    /// What the triggers `by: this` that went tell the agent once the
    /// rest went, by their files.
    tells: Vec<(PathBuf, String)>,
    /// What came back of what runs now while the agent asked the human,
    /// which the chat takes once they answered.
    held: Option<Came>,
}

/// What came back of what runs as a turn ended, or as something else
/// happened, as the host hands it to the conversation.
#[derive(Debug)]
pub enum Came {
    /// How the command of the goal, or of the trigger that runs, went, as
    /// [`Conversation::checked`] takes it.
    Ran(Result<Ran, String>),
    /// What the judge of the goal said, as [`Conversation::judged`] takes
    /// it.
    Judged(Result<String, String>),
    /// What the agent of the trigger that runs said, as
    /// [`Conversation::found`] takes it.
    Found(Result<String, String>),
}

/// Whether the goal was met as the turn ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Met {
    #[default]
    No,
    /// It was, as what runs as the turn ended runs.
    Now,
    /// It was, once all ran, and the host has not run its triggers yet.
    Untaken,
}

impl Conversation {
    /// Hands the conversation the triggers due as the turn that runs
    /// ends, which run after the goal, in order, if it ends as the agent
    /// ends it.
    pub fn set_due(&mut self, due: Vec<Due>) {
        self.due = due;
    }

    /// Runs `at`, then `rest`, shown on a line of chips of their own, as
    /// `happened`, or as the turn ended.
    pub(super) fn run_checks(&mut self, at: At, rest: VecDeque<Due>, happened: Option<Happened>) {
        let first = match &at {
            At::Goal => "goal",
            At::Trigger(due) => due.name.as_str(),
        };
        let marks = std::iter::once(first)
            .chain(rest.iter().map(|due| due.name.as_str()))
            .map(|name| Check {
                name: name.to_owned(),
                state: CheckState::Waiting,
                said: None,
            })
            .collect();
        self.push(match &happened {
            Some(happened) => Entry::Event {
                on: happened.on.clone(),
                checks: marks,
            },
            None => Entry::Checks(marks),
        });
        self.checks = Some(Checks {
            at,
            rest,
            marks: (self.entries.len() - 1, 0),
            event: happened,
            tells: Vec::new(),
            held: None,
        });
        self.state = State::Checking;
        self.mark(CheckState::Running);
    }

    /// Has `due`, the triggers that run on what `happened`, run as soon as
    /// the chat waits for the human, which the host starts them with (see
    /// [`Self::run_waiting`]).
    pub fn run_event(&mut self, happened: Happened, due: Vec<Due>) {
        if due.is_empty() {
            return;
        }
        self.waiting.push_back((happened, due));
    }

    /// Starts the triggers of what happened first that wait, if any do
    /// and the chat waits for the human; answers whether it did.
    pub fn run_waiting(&mut self) -> bool {
        if self.state != State::Idle {
            return false;
        }
        let Some((happened, mut due)) = self.waiting.pop_front() else {
            return false;
        };
        // A trigger by: this tells the agent once for a line.
        due.retain(|due| !matches!(due.does, Does::Tell(_)) || !self.told_line.contains(&due.file));
        let mut rest = VecDeque::from(due);
        let Some(first) = rest.pop_front() else {
            return self.run_waiting();
        };
        self.run_checks(At::Trigger(first), rest, Some(happened));
        true
    }

    /// What happened whose triggers run now, while they do, as against
    /// those of a turn's end.
    #[must_use]
    pub fn event(&self) -> Option<&Happened> {
        self.checks.as_ref()?.event.as_ref()
    }

    /// Whether the goal was met since this was last asked, for the host
    /// to run the triggers of it.
    pub fn take_goal_met(&mut self) -> bool {
        let met = self.met == Met::Untaken;
        if met {
            self.met = Met::No;
        }
        met
    }

    /// What the trigger `by: this` that runs now tells the agent, while
    /// one runs.
    #[must_use]
    pub fn telling(&self) -> Option<&str> {
        match self.trigger()?.does {
            Does::Tell(ref text) => Some(text),
            _ => None,
        }
    }

    /// Takes what the trigger `by: this` that runs now tells the agent, to
    /// tell it once the rest went, and returns the next turn to send, if
    /// that was the last.
    pub fn tell(&mut self) -> Option<Command> {
        self.taking()?;
        let due = self.trigger()?.clone();
        let Does::Tell(text) = due.does else {
            return None;
        };
        self.checks.as_mut()?.tells.push((due.file, text));
        self.say("Told the chat's agent, once the rest went.");
        self.next(CheckState::Passed)
    }

    /// Marks what runs now as `state`, on the line of chips.
    fn mark(&mut self, state: CheckState) {
        let Some((entry, at)) = self.check_at() else {
            return;
        };
        if let Some(marks) = self.entries.get_mut(entry).and_then(Entry::checks_mut)
            && let Some(mark) = marks.get_mut(at)
        {
            mark.state = state;
            self.changed.insert(entry);
        }
    }

    /// Keeps what what runs now `said`, on its chip, for the human to read
    /// there.
    fn say(&mut self, said: &str) {
        let Some((entry, at)) = self.check_at() else {
            return;
        };
        if let Some(marks) = self.entries.get_mut(entry).and_then(Entry::checks_mut)
            && let Some(mark) = marks.get_mut(at)
        {
            mark.said = Some(tail(said, OUTPUT_TAIL)).filter(|said| !said.is_empty());
            self.changed.insert(entry);
        }
    }

    /// What runs now passed: the next runs, or, when none is left, the
    /// chat waits for the human, as [`Self::next`] says.
    fn passed(&mut self) -> Option<Command> {
        self.next(CheckState::Passed)
    }

    /// What runs now went as `state` says, and what runs on: the next, or,
    /// when none is left, the chat waits for the human, asking whether to
    /// keep what was learned if the goal was met, unless the triggers `by:
    /// this` that went tell the agent what they say, which returns the
    /// turn to send.
    fn next(&mut self, state: CheckState) -> Option<Command> {
        self.mark(state);
        let checks = self.checks.as_mut()?;
        checks.marks.1 += 1;
        if let Some(due) = checks.rest.pop_front() {
            checks.at = At::Trigger(due);
            self.mark(CheckState::Running);
            return None;
        }
        let Checks { event, tells, .. } = self.checks.take()?;
        self.state = State::Idle;
        if self.met == Met::Now {
            self.ask_to_keep_here(GOAL_MET);
            // Its triggers run once for a line, as the turns they start
            // meet it again.
            self.met = if self.raised_line.insert(GOAL_MET_EVENT) {
                Met::Untaken
            } else {
                Met::No
            };
        }
        if tells.is_empty() {
            return None;
        }
        if event.is_none() && self.turns >= GOAL_TURNS {
            self.push(Entry::Note(format!(
                "The triggers by: this tell the agent nothing more: the chat took \
                 {GOAL_TURNS} turns, as many as one line gets; send a line to go on."
            )));
            return None;
        }
        let (files, texts): (Vec<PathBuf>, Vec<String>) = tells.into_iter().unzip();
        self.told_line.extend(files);
        let prompt = texts.join("\n\n");
        if event.is_none() {
            self.turns += 1;
        }
        self.push(Entry::Cox(prompt.clone()));
        self.state = State::Busy;
        Some(Command::Prompt(prompt.into()))
    }

    /// Stops what runs as the turn ended, marking what runs now as
    /// `state`; what has not run yet stays waiting. The chat waits for the
    /// human, unless it goes on, as [`Self::go_on`] says; while the agent
    /// asks them, it does once they answered.
    pub(super) fn stop_checks(&mut self, state: CheckState) {
        self.mark(state);
        self.checks = None;
        if self.state == State::Asking {
            self.before_asking = State::Idle;
        } else {
            self.state = State::Idle;
        }
    }

    /// What runs now could not run: as a turn ends, what was to run after
    /// it does not, and the chat waits for the human; as something else
    /// happened, the rest runs.
    fn broken(&mut self) -> Option<Command> {
        if self.event().is_some() {
            return self.next(CheckState::Broken);
        }
        self.stop_checks(CheckState::Broken);
        None
    }

    /// The name of what runs now as the turn ended, `goal` or a
    /// trigger's, as the line of chips shows it, while something runs.
    #[must_use]
    pub fn running_check(&self) -> Option<&str> {
        if self.state != State::Checking {
            return None;
        }
        self.entries.iter().rev().find_map(|entry| {
            entry
                .checks()?
                .iter()
                .find(|check| check.state == CheckState::Running)
                .map(|check| check.name.as_str())
        })
    }

    /// Where the chip of what runs now is, as the turn ended: the entry
    /// of the line of chips, and its place in it, while something runs.
    #[must_use]
    pub fn check_at(&self) -> Option<(usize, usize)> {
        Some(self.checks.as_ref()?.marks)
    }

    /// Whether what runs as the turn ended, or as something else
    /// happened, runs: as the chat checks, and as the agent asks the
    /// human in the middle of it.
    #[must_use]
    pub fn runs_checks(&self) -> bool {
        self.checks.is_some()
    }

    /// Keeps `came`, what came back of what runs, while the agent asks
    /// the human in the middle of it, for the host to take once they
    /// answered (see [`Self::take_held`]).
    pub fn hold(&mut self, came: Came) {
        if let Some(checks) = &mut self.checks {
            checks.held = Some(came);
        }
    }

    /// What came back of what runs while the agent asked the human in the
    /// middle of it, once they answered, for the host to take as it would
    /// have then.
    pub fn take_held(&mut self) -> Option<Came> {
        self.taking()?;
        self.checks.as_mut()?.held.take()
    }

    /// What runs, while the chat takes what comes back of it: as it
    /// checks, not as the agent asks the human in the middle of it, when
    /// the host holds what comes (see [`Self::hold`]).
    fn taking(&self) -> Option<&Checks> {
        self.checks
            .as_ref()
            .filter(|_| self.state == State::Checking)
    }

    /// The trigger that runs now, if one does.
    #[must_use]
    pub fn trigger(&self) -> Option<&Due> {
        match &self.checks.as_ref()?.at {
            At::Trigger(due) => Some(due),
            At::Goal => None,
        }
    }

    /// The command that runs to see whether the goal is met, or of the
    /// trigger that runs, while one does.
    #[must_use]
    pub fn checking(&self) -> Option<&str> {
        match &self.checks.as_ref()?.at {
            At::Goal => self.goal.as_ref()?.command(),
            At::Trigger(Due {
                does: Does::Run(command),
                ..
            }) => Some(command),
            At::Trigger(_) => None,
        }
    }

    /// The words of the goal the judge is asked about, while it is.
    #[must_use]
    pub fn judging(&self) -> Option<&str> {
        match self.checks.as_ref()?.at {
            At::Goal => self.goal.as_ref()?.words(),
            At::Trigger(_) => None,
        }
    }

    /// The trigger whose agent is asked, while it is.
    #[must_use]
    pub fn asking(&self) -> Option<&Due> {
        self.trigger()
            .filter(|due| matches!(due.does, Does::Ask(_)))
    }

    /// Takes how the command of the goal, or of the trigger that runs,
    /// went, `ran`, and returns the next turn to send the agent, when it
    /// failed and the chat goes on. When it passed, what is due next
    /// runs, as [`Self::checking`] and the others say, or the chat waits
    /// for the human; it waits too when the command could not run, or the
    /// chat has taken as many turns as one line gets, or the agent made
    /// no tool call in its last turns.
    pub fn checked(&mut self, ran: Result<Ran, String>) -> Option<Command> {
        self.taking()?;
        let command = self.checking()?.to_owned();
        let trigger = self.trigger().map(|due| due.name.clone());
        let ran = match ran {
            Ok(ran) => ran,
            Err(error) => {
                self.say(&error);
                self.push(Entry::Note(match &trigger {
                    Some(name) => format!("The trigger `{name}` could not run: {error}"),
                    None => format!("The goal could not be checked: {error}"),
                }));
                return self.broken();
            }
        };
        self.say(&ran.output);
        let not_met = match ran.ended {
            Ended::Exited(0) => {
                if trigger.is_none() {
                    self.push(Entry::Note(format!("Goal met: `{command}` passed.")));
                    self.met = Met::Now;
                }
                return self.passed();
            }
            Ended::Closed => {
                self.push(Entry::Note(match &trigger {
                    Some(name) => format!(
                        "The trigger `{name}` did not run: the terminal of `{command}` was closed."
                    ),
                    None => {
                        format!("The goal was not checked: the terminal of `{command}` was closed.")
                    }
                }));
                return self.broken();
            }
            ended => match &trigger {
                Some(name) => format!("The trigger `{name}` did not pass: `{command}` {ended}."),
                None => format!("Goal not met: `{command}` {ended}."),
            },
        };
        if self.event().is_some() {
            return self.next(CheckState::Failed);
        }
        let output = tail(&ran.output, OUTPUT_TAIL);
        let go_on = if trigger.is_some() {
            "Fix what it found; the triggers run again as your turn ends."
        } else {
            "Go on until it passes."
        };
        let told = if output.is_empty() {
            format!("{go_on} It printed nothing.")
        } else {
            format!("{go_on} It printed, at the end:\n\n{output}")
        };
        self.stop_checks(CheckState::Failed);
        self.go_on(&not_met, &told)
    }

    /// Takes what the judge of the goal in words said, `judged`, or why
    /// it could not judge, and returns the next turn to send the agent,
    /// when the goal is not met and the chat goes on toward it; otherwise
    /// what is due next runs, or the chat waits for the human, as after
    /// [`Conversation::checked`].
    pub fn judged(&mut self, judged: Result<String, String>) -> Option<Command> {
        self.taking()?;
        self.judging()?;
        let said = match judged {
            Ok(said) => said,
            Err(error) => {
                self.say(&error);
                self.push(Entry::Note(format!(
                    "The goal could not be judged: {error}"
                )));
                self.stop_checks(CheckState::Broken);
                return None;
            }
        };
        self.say(&said);
        self.offer_command(&said);
        let why = match Verdict::of(&said) {
            Verdict::Met(said) => {
                self.push(Entry::Note(if said.is_empty() {
                    "Goal met, as judged.".to_owned()
                } else {
                    format!("Goal met, as judged: {said}")
                }));
                self.met = Met::Now;
                return self.passed();
            }
            Verdict::NotMet(why) => why,
            Verdict::Blocked(why) => {
                self.push(Entry::Note(format!(
                    "The goal cannot be judged as written: {why} Change it to what this chat's \
                     work can meet and the judge can check, then send a line to go on."
                )));
                self.stop_checks(CheckState::Broken);
                return None;
            }
        };
        let not_met = format!("Goal not met, as judged: {why}");
        let told = format!(
            "Go on until it is met. The judge said:\n\n{}",
            tail(&said, OUTPUT_TAIL)
        );
        self.stop_checks(CheckState::Failed);
        self.go_on(&not_met, &told)
    }

    /// Takes what the agent of the trigger that runs said, `said`, or why
    /// it could not say, and returns the next turn to send the chat's
    /// agent, when the trigger found something and the chat goes on;
    /// otherwise what is due next runs, or the chat waits for the human,
    /// as after [`Conversation::checked`].
    pub fn found(&mut self, said: Result<String, String>) -> Option<Command> {
        self.taking()?;
        let name = self.asking()?.name.clone();
        let said = match said {
            Ok(said) => said,
            Err(error) => {
                self.say(&error);
                self.push(Entry::Note(format!(
                    "The trigger `{name}` could not run: {error}"
                )));
                return self.broken();
            }
        };
        self.say(&said);
        let why = match Verdict::of(&said) {
            Verdict::Met(_) => return self.passed(),
            // What happened besides a turn's end, the agent is not told.
            Verdict::NotMet(_) | Verdict::Blocked(_) if self.event().is_some() => {
                return self.next(CheckState::Failed);
            }
            Verdict::NotMet(why) | Verdict::Blocked(why) => why,
        };
        let found = format!("The trigger `{name}` found something: {why}");
        let told = format!(
            "Act on what it said; the triggers run again as your turn ends. It said:\n\n{}",
            tail(&said, OUTPUT_TAIL)
        );
        self.stop_checks(CheckState::Failed);
        self.go_on(&found, &told)
    }

    /// Proposes the command the judge `said` decides the goal, if it named
    /// one, for the human to judge the goal by in the judge's place: once
    /// in the chat, as a proposal of the agent's would be.
    fn offer_command(&mut self, said: &str) {
        let Some(command) = judge::command_of(said) else {
            return;
        };
        let goal = Proposed::Goal(goal::Goal::Command(command).input());
        let offered = self.entries.iter().any(
            |entry| matches!(entry, Entry::Proposal(Proposal { proposed, .. }) if *proposed == goal),
        );
        if !offered {
            self.propose(
                goal,
                Some(
                    "The judge says this command passes just when the goal is met, so it \
                     can decide the goal in the judge's place."
                        .to_owned(),
                ),
            );
        }
    }

    /// Goes on toward the goal, `not_met` as the check or the judge said:
    /// returns the next turn, telling the agent so and `told`, unless the
    /// chat has taken as many turns as one line gets, or the agent made no
    /// tool call in its last turns, when the chat notes so and waits for
    /// the human.
    fn go_on(&mut self, not_met: &str, told: &str) -> Option<Command> {
        if self.turns >= GOAL_TURNS {
            self.push(Entry::Note(format!(
                "{not_met} The chat took {GOAL_TURNS} turns toward it, as many as one line gets; send a line to go on."
            )));
            return None;
        }
        if self.turns_without_tools() >= IDLE_TURNS {
            self.push(Entry::Note(format!(
                "{not_met} The agent made no tool call in its last {IDLE_TURNS} turns, so it may wait for you."
            )));
            return None;
        }
        let prompt = format!("{not_met} {told}");
        self.turns += 1;
        if let Some(goal) = &mut self.goal {
            goal.turns += 1;
        }
        self.push(Entry::Cox(prompt.clone()));
        self.state = State::Busy;
        Some(Command::Prompt(prompt.into()))
    }

    /// How many of the last turns made no tool call, counting back from
    /// the last turn to the first that made one.
    fn turns_without_tools(&self) -> usize {
        let mut turns = 0;
        let mut called = false;
        for entry in self.entries.iter().rev() {
            match entry {
                Entry::ToolCall { .. } => called = true,
                Entry::Human(_) | Entry::Cox(_) => {
                    if called {
                        break;
                    }
                    turns += 1;
                }
                _ => {}
            }
        }
        turns
    }
}

#[cfg(test)]
mod tests {
    use cox_acp::{PermissionAnswer, StopReason};

    use super::super::tests::step;
    use super::super::{Answer, Goal, StepState, ToolCallResult};
    use super::*;
    use crate::agent::{AgentEvent, Prompt};

    /// How the goal's command went: it exited with `code`, printing
    /// `output`.
    #[expect(clippy::unnecessary_wraps, reason = "what `checked` takes")]
    fn ran(code: i32, output: &str) -> Result<Ran, String> {
        Ok(Ran {
            output: output.to_owned(),
            ended: Ended::Exited(code),
        })
    }

    /// The triggers `ci`, which runs `cargo test`, and `simplify`, which
    /// asks its agent, due as the turn ends.
    fn ci_and_simplify() -> Vec<Due> {
        vec![
            Due {
                name: "ci".to_owned(),
                file: "/work/triggers/ci.md".into(),
                does: Does::Run("cargo test".to_owned()),
            },
            Due {
                name: "simplify".to_owned(),
                file: "/home/me/cox/triggers/simplify.md".into(),
                does: Does::Ask("Say what could be simpler.".to_owned()),
            },
        ]
    }

    /// How the line of chips of what ran as the turn ended stands, by
    /// name.
    fn marks(conversation: &Conversation) -> Vec<(String, CheckState)> {
        conversation
            .entries()
            .iter()
            .rev()
            .find_map(|entry| {
                Some(
                    entry
                        .checks()?
                        .iter()
                        .map(|check| (check.name.clone(), check.state))
                        .collect(),
                )
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_trigger_by_this_tells_the_agent_once_for_a_line() {
        let commit = || Due {
            name: "commit".to_owned(),
            file: "/p/triggers/commit.md".into(),
            does: Does::Tell("Write the commit message.".to_owned()),
        };
        let mut conversation = Conversation::new();
        conversation.submit("fix");
        conversation.set_due(vec![commit(), ci_and_simplify().remove(0)]);
        take_turn(&mut conversation, true);
        assert_eq!(conversation.telling(), Some("Write the commit message."));
        // It tells once the rest went.
        assert_eq!(conversation.tell(), None);
        assert_eq!(conversation.checking(), Some("cargo test"));
        assert_eq!(
            conversation.checked(ran(0, "ok")),
            Some(Command::Prompt(
                "Write the commit message.".to_owned().into()
            ))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Cox("Write the commit message.".to_owned()))
        );
        // The turn it starts does not tell it again.
        conversation.set_due(vec![commit()]);
        take_turn(&mut conversation, true);
        assert_eq!(conversation.state(), State::Idle);
        // The next line does.
        conversation.submit("and the docs");
        conversation.set_due(vec![commit()]);
        take_turn(&mut conversation, true);
        assert_eq!(conversation.telling(), Some("Write the commit message."));
    }

    #[test]
    fn the_goal_met_again_on_a_line_runs_its_triggers_once() {
        let commit = Due {
            name: "commit".to_owned(),
            file: "/p/triggers/commit.md".into(),
            does: Does::Tell("Write the commit message.".to_owned()),
        };
        let happened = Happened {
            on: "goal.met".to_owned(),
            about: "The chat's goal is met.".to_owned(),
        };
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("fix");
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert!(conversation.take_goal_met());
        conversation.run_event(happened.clone(), vec![commit.clone()]);
        assert!(conversation.run_waiting());
        assert_eq!(
            conversation.tell(),
            Some(Command::Prompt(
                "Write the commit message.".to_owned().into()
            ))
        );
        // The turn it starts meets the goal again, which runs nothing.
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert!(!conversation.take_goal_met());
        conversation.run_event(happened, vec![commit]);
        assert!(!conversation.run_waiting());
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn what_happened_runs_its_triggers_once_the_chat_waits_untold() {
        let happened = || Happened {
            on: "goal.met".to_owned(),
            about: "The chat's goal is met.".to_owned(),
        };
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("fix");
        take_turn(&mut conversation, true);
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.state(), State::Idle);
        assert!(conversation.take_goal_met());
        assert!(!conversation.take_goal_met());

        conversation.run_event(happened(), ci_and_simplify());
        assert!(conversation.run_waiting());
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.event(), Some(&happened()));
        assert!(matches!(
            conversation.entries().last(),
            Some(Entry::Event { on, .. }) if on == "goal.met"
        ));
        // What a trigger finds stays on its chip, and the rest still run.
        assert_eq!(conversation.checked(ran(1, "a test failed")), None);
        assert_eq!(
            conversation.asking().map(|due| due.name.as_str()),
            Some("simplify")
        );
        assert_eq!(conversation.found(Ok("FAIL: it repeats".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(conversation.event(), None);
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Failed),
                ("simplify".to_owned(), CheckState::Failed),
            ]
        );

        // While the agent is at work, they wait for its turn to end.
        conversation.submit("more");
        conversation.run_event(happened(), ci_and_simplify());
        assert!(!conversation.run_waiting());
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert_eq!(conversation.state(), State::Idle);
        assert!(conversation.run_waiting());
        assert_eq!(conversation.state(), State::Checking);
    }

    #[test]
    fn the_triggers_run_after_the_goal_until_one_fails() {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), Some("make"));
        assert_eq!(conversation.running_check(), Some("goal"));

        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), Some("cargo test"));
        assert_eq!(
            conversation.trigger().map(|due| due.name.as_str()),
            Some("ci")
        );
        assert_eq!(conversation.running_check(), Some("ci"));
        // The goal is met, but the human is asked to keep what was learned
        // only once the triggers pass too.
        assert!(
            !conversation
                .entries()
                .iter()
                .any(|entry| matches!(entry, Entry::Keep { .. }))
        );

        assert_eq!(conversation.checked(ran(0, "test result: ok")), None);
        assert_eq!(conversation.checking(), None);
        assert_eq!(
            conversation.asking().map(|due| due.name.as_str()),
            Some("simplify")
        );
        // Its agent is told what the turn said, not what ran after.
        assert_eq!(conversation.turn_told(), Some("Done."));

        let said = "Read the diff.\nFAIL: parse repeats read";
        let told = "The trigger `simplify` found something: parse repeats read \
                    Act on what it said; the triggers run again as your turn ends. \
                    It said:\n\nRead the diff.\nFAIL: parse repeats read";
        assert_eq!(
            conversation.found(Ok(said.to_owned())),
            Some(Command::Prompt(told.to_owned().into()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(
            marks(&conversation),
            [
                ("goal".to_owned(), CheckState::Passed),
                ("ci".to_owned(), CheckState::Passed),
                ("simplify".to_owned(), CheckState::Failed),
            ]
        );

        // The next turn runs them all again, and once they pass, the human
        // is asked whether to keep what the chat learned.
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        conversation.checked(ran(0, "ok"));
        assert_eq!(conversation.found(Ok("Fine.\nPASS".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert!(matches!(
            conversation.entries().last(),
            Some(Entry::Keep { .. })
        ));
    }

    #[test]
    fn a_chat_with_no_goal_runs_its_triggers() {
        let mut conversation = Conversation::new();
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);
        assert_eq!(conversation.checking(), Some("cargo test"));
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Running),
                ("simplify".to_owned(), CheckState::Waiting),
            ]
        );

        let told = "The trigger `ci` did not pass: `cargo test` exit code 101. Fix what it \
                    found; the triggers run again as your turn ends. It printed, at the \
                    end:\n\n1 failed";
        assert_eq!(
            conversation.checked(ran(101, "1 failed")),
            Some(Command::Prompt(told.to_owned().into()))
        );
        // The trigger after the one that failed did not run.
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Failed),
                ("simplify".to_owned(), CheckState::Waiting),
            ]
        );

        // A turn with nothing due ends as it did.
        take_turn(&mut conversation, true);
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn a_trigger_that_cannot_run_waits_for_the_human() {
        let mut conversation = Conversation::new();
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, ""));
        assert_eq!(conversation.found(Err("its agent closed".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some("The trigger `simplify` could not run: its agent closed")
        );
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Passed),
                ("simplify".to_owned(), CheckState::Broken),
            ]
        );
    }

    #[test]
    fn stopping_while_a_trigger_runs_gives_the_rest_up() {
        let mut conversation = Conversation::new();
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);

        assert_eq!(conversation.cancel(), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(
            marks(&conversation),
            [
                ("ci".to_owned(), CheckState::Waiting),
                ("simplify".to_owned(), CheckState::Waiting),
            ]
        );
    }

    /// A conversation working until `make` passes, whose agent takes a
    /// turn on "fix", with a tool call when `calling`, which ends.
    fn toward_make(calling: bool) -> Conversation {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        assert_eq!(
            conversation.entries(),
            [Entry::Note("Goal: until `make` passes.".to_owned())]
        );
        conversation.submit("fix");
        take_turn(&mut conversation, calling);
        conversation
    }

    /// The agent takes a turn, with a tool call when `calling`, and ends
    /// it as it does.
    fn take_turn(conversation: &mut Conversation, calling: bool) {
        if calling {
            let id = format!("call-{}", conversation.entries().len());
            conversation.apply(AgentEvent::ToolCall {
                id,
                title: Some("Edit x.txt".to_owned()),
                content: None,
                result: Some(ToolCallResult::Completed),
            });
        }
        conversation.apply(AgentEvent::ReplyText("Done.".to_owned()));
        conversation.apply(AgentEvent::TurnEnded(StopReason::EndTurn));
    }

    #[test]
    fn the_human_is_asked_to_keep_in_the_wiki_once_for_a_line_of_theirs() {
        let mut conversation = toward_make(true);
        conversation.checked(ran(0, "ok"));
        let asked = conversation.entries().len() - 1;
        assert_eq!(conversation.answer_keep(asked, true), Some(None));
        // The turn that keeps it meets the goal again, which asks nothing.
        conversation.told("Keep it.");
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("Goal met: `make` passed.".to_owned()))
        );
        // The human's next line does.
        conversation.submit("more");
        take_turn(&mut conversation, true);
        conversation.checked(ran(0, "ok"));
        assert!(matches!(
            conversation.entries().last(),
            Some(Entry::Keep { kept: None, .. })
        ));
    }

    #[test]
    fn a_chat_whose_agent_cannot_keep_in_the_wiki_is_not_asked_to() {
        let mut conversation = toward_make(true).with_keeping(false);
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("Goal met: `make` passed.".to_owned()))
        );
    }

    #[test]
    fn works_toward_its_goal_until_the_command_passes() {
        let mut conversation = toward_make(true);
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), Some("make"));
        assert_eq!(conversation.submit("again"), None);
        assert_eq!(conversation.goal().map(Goal::turns), Some(1));
        conversation.take_changed();

        let prompt = "Goal not met: `make` exit code 2. Go on until it passes. \
                      It printed, at the end:\n\nerror: x is not defined";
        assert_eq!(
            conversation.checked(ran(2, "error: x is not defined\n\n")),
            Some(Command::Prompt(prompt.to_owned().into()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.checking(), None);
        assert_eq!(conversation.goal().map(Goal::turns), Some(2));
        // The goal's chip shows that it failed, with what it printed.
        assert_eq!(conversation.take_changed(), [4, 5]);
        assert_eq!(
            conversation.entries()[4],
            Entry::Checks(vec![Check {
                name: "goal".to_owned(),
                state: CheckState::Failed,
                said: Some("error: x is not defined".to_owned()),
            }])
        );
        assert_eq!(conversation.entries()[5], Entry::Cox(prompt.to_owned()));

        take_turn(&mut conversation, true);
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.state(), State::Idle);
        // The human is asked whether to keep what the chat learned.
        let entries = conversation.entries();
        assert_eq!(
            entries[entries.len() - 2..],
            [
                Entry::Note("Goal met: `make` passed.".to_owned()),
                Entry::Keep {
                    why: GOAL_MET.to_owned(),
                    chat: None,
                    kept: None,
                },
            ]
        );
        let asked = entries.len() - 1;
        assert_eq!(conversation.answer_keep(asked, false), Some(None));
        assert_eq!(conversation.answer_keep(asked, true), None);
        assert_eq!(
            conversation.entries()[asked],
            Entry::Keep {
                why: GOAL_MET.to_owned(),
                chat: None,
                kept: Some(false),
            }
        );
        // The goal stays, for the human's next line.
        assert_eq!(conversation.goal().and_then(Goal::command), Some("make"));
        conversation.submit("more");
        assert_eq!(conversation.goal().map(Goal::turns), Some(1));
    }

    #[test]
    fn each_turn_toward_the_goal_plans_anew() {
        let mut conversation = toward_make(true);
        let first = vec![step("Fix", StepState::Completed)];
        let then = vec![step("Fix more", StepState::InProgress)];
        conversation.checked(ran(1, ""));
        conversation.apply(AgentEvent::Plan(then.clone()));
        assert_eq!(conversation.last_plan(), Some(&then[..]));

        conversation.apply(AgentEvent::Plan(first.clone()));
        let plans: Vec<_> = conversation
            .entries()
            .iter()
            .filter(|entry| matches!(entry, Entry::Plan(_)))
            .collect();
        assert_eq!(plans, [&Entry::Plan(first)]);
    }

    #[test]
    fn tells_the_agent_the_end_of_a_long_output() {
        let mut conversation = toward_make(true);
        let output = format!("{}{}", "x".repeat(100), "y".repeat(OUTPUT_TAIL));

        let Some(Command::Prompt(Prompt { text: prompt, .. })) =
            conversation.checked(ran(1, &output))
        else {
            panic!("the agent goes on");
        };
        assert!(prompt.ends_with(&format!("[…]{}", "y".repeat(OUTPUT_TAIL))));
        assert!(!prompt.contains("xy"), "{prompt}");
    }

    #[test]
    fn stops_toward_the_goal_when_the_agent_only_talks() {
        let mut conversation = toward_make(false);
        assert!(conversation.checked(ran(1, "")).is_some());

        take_turn(&mut conversation, false);
        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some(
                "Goal not met: `make` exit code 1. The agent made no tool call in its last 2 turns, so it may wait for you."
            )
        );
    }

    #[test]
    fn stops_after_as_many_turns_toward_the_goal_as_one_line_gets() {
        let mut conversation = toward_make(true);
        for turn in 1..GOAL_TURNS {
            assert_eq!(conversation.goal().map(Goal::turns), Some(turn));
            assert!(conversation.checked(ran(1, "")).is_some());
            take_turn(&mut conversation, true);
        }

        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some(
                "Goal not met: `make` exit code 1. The chat took 10 turns toward it, as many as one line gets; send a line to go on."
            )
        );
        // The human's line starts the count again.
        conversation.submit("go on");
        take_turn(&mut conversation, true);
        assert!(conversation.checked(ran(1, "")).is_some());
    }

    #[test]
    fn a_turn_that_is_stopped_or_fails_is_not_checked() {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("wait");
        conversation.cancel();
        conversation.apply(AgentEvent::TurnEnded(StopReason::Cancelled));
        assert_eq!(conversation.state(), State::Idle);

        conversation.submit("fail");
        conversation.apply(AgentEvent::TurnFailed("lost".to_owned()));
        assert_eq!(conversation.state(), State::Idle);

        conversation.submit("long");
        conversation.apply(AgentEvent::TurnEnded(StopReason::MaxTokens));
        assert_eq!(conversation.state(), State::Idle);
    }

    #[test]
    fn stopping_while_the_goal_is_checked_gives_the_check_up() {
        let mut conversation = toward_make(true);

        assert_eq!(conversation.cancel(), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("Stopped.".to_owned()))
        );
        // What the command answers once it is stopped changes nothing.
        let entries = conversation.entries().len();
        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(conversation.entries().len(), entries);
        assert_eq!(
            conversation.submit("again"),
            Some(Command::Prompt("again".to_owned().into()))
        );
    }

    #[test]
    fn changing_the_goal_while_it_is_checked_gives_the_check_up() {
        let mut conversation = toward_make(true);
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        assert_eq!(conversation.state(), State::Checking);

        conversation.set_goal(None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(conversation.goal(), None);
        assert_eq!(
            conversation.entries().last(),
            Some(&Entry::Note("The goal is cleared.".to_owned()))
        );
        assert_eq!(conversation.checked(ran(1, "")), None);
        // With no goal, a turn ends as it did.
        conversation.submit("fix");
        take_turn(&mut conversation, true);
        assert_eq!(conversation.state(), State::Idle);
    }

    /// The agent goes on by itself after its turn ended, and asks the
    /// human's permission.
    fn ask_after_the_turn(conversation: &mut Conversation) {
        conversation.apply(AgentEvent::WentOn);
        conversation.apply(AgentEvent::PermissionRequest {
            id: "call-9".to_owned(),
            title: Some("touch y.txt".to_owned()),
            content: None,
            options: vec!["Allow".to_owned(), "Reject".to_owned()],
        });
    }

    #[test]
    fn what_runs_as_the_turn_ended_goes_on_once_the_human_answers_the_agent() {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Command("make".to_owned())));
        conversation.submit("fix");
        conversation.set_due(ci_and_simplify());
        take_turn(&mut conversation, true);

        ask_after_the_turn(&mut conversation);
        assert_eq!(conversation.state(), State::Asking);
        assert_eq!(
            conversation.choose(0),
            Some(Command::Answer(PermissionAnswer::Select(0)))
        );
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), Some("make"));

        // `make` ends once the human answered: the goal is met, and the
        // triggers run after it.
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.checking(), Some("cargo test"));
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.found(Ok("PASS".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            marks(&conversation),
            [
                ("goal".to_owned(), CheckState::Passed),
                ("ci".to_owned(), CheckState::Passed),
                ("simplify".to_owned(), CheckState::Passed),
            ]
        );
        assert!(matches!(
            conversation.entries().last(),
            Some(Entry::Keep { .. })
        ));
    }

    #[test]
    fn a_goal_judged_as_the_agent_asked_goes_on_once_the_human_answers() {
        let mut conversation = toward_words(true);
        ask_after_the_turn(&mut conversation);
        conversation.choose(1);

        assert_eq!(conversation.judging(), Some("The greeting names the user."));
        let Some(Command::Prompt(Prompt { text: prompt, .. })) =
            conversation.judged(Ok("FAIL: it says hello".to_owned()))
        else {
            panic!("the agent goes on toward the goal");
        };
        assert!(
            prompt.starts_with("Goal not met, as judged: it says hello"),
            "{prompt}"
        );
        assert_eq!(conversation.state(), State::Busy);
    }

    #[test]
    fn stopping_the_agent_that_asks_as_the_goal_is_checked_gives_the_check_up() {
        let mut conversation = toward_make(true);
        ask_after_the_turn(&mut conversation);

        assert_eq!(conversation.cancel(), Some(Command::Cancel));
        assert_eq!(conversation.state(), State::Stopping);
        conversation.apply(AgentEvent::TurnEnded(StopReason::Cancelled));
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            marks(&conversation),
            [("goal".to_owned(), CheckState::Waiting)]
        );
        assert_eq!(conversation.checked(ran(1, "")), None);
        assert_eq!(
            conversation.submit("again"),
            Some(Command::Prompt("again".to_owned().into()))
        );
    }

    #[test]
    fn changing_the_goal_as_the_agent_asks_gives_the_check_up() {
        let mut conversation = toward_make(true);
        ask_after_the_turn(&mut conversation);

        conversation.set_goal(None);
        assert_eq!(conversation.state(), State::Asking);
        assert_eq!(conversation.checking(), None);
        assert_eq!(
            marks(&conversation),
            [("goal".to_owned(), CheckState::Waiting)]
        );
        // Once answered, the chat waits for the human.
        assert_eq!(conversation.whole().before_asking, State::Idle);
    }

    #[test]
    fn what_comes_back_as_the_agent_asks_is_taken_once_the_human_answers() {
        let mut conversation = toward_make(true);
        ask_after_the_turn(&mut conversation);
        // While the human is asked, the chat takes nothing that comes back.
        let entries = conversation.entries().len();
        assert_eq!(conversation.checked(ran(0, "ok")), None);
        assert_eq!(conversation.entries().len(), entries);
        conversation.hold(Came::Ran(ran(1, "error")));
        assert_eq!(conversation.state(), State::Asking);
        assert!(conversation.take_held().is_none());

        conversation.choose(0);
        let Some(Came::Ran(ran)) = conversation.take_held() else {
            panic!("what `make` answered is kept");
        };
        assert!(conversation.checked(ran).is_some());
        assert_eq!(conversation.state(), State::Busy);
        assert!(conversation.take_held().is_none());
    }

    #[test]
    fn a_goal_that_cannot_be_checked_waits_for_the_human() {
        let mut conversation = toward_make(true);
        assert_eq!(
            conversation.checked(Err("the chat has no terminal to run it in".to_owned())),
            None
        );
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some("The goal could not be checked: the chat has no terminal to run it in")
        );

        conversation.submit("fix");
        take_turn(&mut conversation, true);
        let closed = Ok(Ran {
            output: String::new(),
            ended: Ended::Closed,
        });
        assert_eq!(conversation.checked(closed), None);
        assert_eq!(
            conversation.last_said(),
            Some("The goal was not checked: the terminal of `make` was closed.")
        );
    }

    /// A conversation working until the greeting names the user, as an
    /// agent judges, whose agent takes a turn on "fix", with a tool call
    /// when `calling`, which ends.
    fn toward_words(calling: bool) -> Conversation {
        let mut conversation = Conversation::new();
        conversation.set_goal(Some(goal::Goal::Words(
            "The greeting names the user.".to_owned(),
        )));
        assert_eq!(
            conversation.entries(),
            [Entry::Note("Goal: The greeting names the user.".to_owned())]
        );
        conversation.submit("fix");
        take_turn(&mut conversation, calling);
        conversation
    }

    #[test]
    fn a_goal_the_judge_cannot_judge_as_written_waits_for_the_human() {
        let mut conversation = toward_words(true);
        conversation.take_changed();

        let said = "It asks for Slack.\nBLOCKED: I cannot see whether Slack got a message.";
        assert_eq!(conversation.judged(Ok(said.to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert!(conversation.entries().iter().any(|entry| matches!(entry,
        Entry::Note(note) if note.starts_with(
            "The goal cannot be judged as written: I cannot see whether Slack got a message."
        ))));
        // The agent is not told to go on, and the goal stays for the human
        // to change.
        assert!(
            !conversation
                .entries()
                .iter()
                .any(|entry| matches!(entry, Entry::Cox(_)))
        );
        assert!(conversation.goal().is_some());
    }

    #[test]
    fn works_toward_a_goal_in_words_until_the_judge_says_it_is_met() {
        let mut conversation = toward_words(true);
        assert_eq!(conversation.state(), State::Checking);
        assert_eq!(conversation.checking(), None);
        assert_eq!(conversation.judging(), Some("The greeting names the user."));
        assert_eq!(conversation.submit("again"), None);
        conversation.take_changed();

        let said = "I read src/main.rs.\nFAIL: the greeting still says hello";
        let prompt = "Goal not met, as judged: the greeting still says hello \
                      Go on until it is met. The judge said:\n\n\
                      I read src/main.rs.\nFAIL: the greeting still says hello";
        assert_eq!(
            conversation.judged(Ok(said.to_owned())),
            Some(Command::Prompt(prompt.to_owned().into()))
        );
        assert_eq!(conversation.state(), State::Busy);
        assert_eq!(conversation.judging(), None);
        assert_eq!(conversation.goal().map(Goal::turns), Some(2));
        assert_eq!(conversation.take_changed(), [4, 5]);
        assert_eq!(conversation.entries()[5], Entry::Cox(prompt.to_owned()));

        take_turn(&mut conversation, true);
        assert_eq!(
            conversation.judged(Ok("I read it again.\n\n**PASS**\n".to_owned())),
            None
        );
        assert_eq!(conversation.state(), State::Idle);
        let entries = conversation.entries();
        assert_eq!(
            entries[entries.len() - 2],
            Entry::Note("Goal met, as judged.".to_owned())
        );
        assert!(matches!(entries.last(), Some(Entry::Keep { .. })));
        // The goal stays, for the human's next line.
        assert_eq!(
            conversation.goal().and_then(Goal::words),
            Some("The greeting names the user.")
        );
        conversation.submit("more");
        assert_eq!(conversation.goal().map(Goal::turns), Some(1));
    }

    #[test]
    fn the_command_the_judge_finds_is_offered_once() {
        let mut conversation = toward_words(true);
        let said = "I ran it.\nCOMMAND: cargo test greeting\nFAIL: the test fails";
        assert!(conversation.judged(Ok(said.to_owned())).is_some());
        let offered = conversation
            .entries()
            .iter()
            .position(|entry| matches!(entry, Entry::Proposal(_)))
            .expect("the command is offered");
        // Offered ahead of the next turn it gives.
        assert!(matches!(conversation.entries()[offered + 1], Entry::Cox(_)));
        assert_eq!(
            conversation.proposed(offered),
            Some(&Proposed::Goal("$ cargo test greeting".to_owned()))
        );

        take_turn(&mut conversation, true);
        conversation.judged(Ok("COMMAND: cargo test greeting\nPASS".to_owned()));
        let offers = conversation
            .entries()
            .iter()
            .filter(|entry| matches!(entry, Entry::Proposal(_)))
            .count();
        assert_eq!(offers, 1);

        // Set, it is the goal, which a command checks from then on.
        conversation.answer_proposal(offered, Answer::Set);
        assert_eq!(conversation.proposed(offered), None);
        conversation.set_goal(goal::Goal::parse("$ cargo test greeting"));
        assert_eq!(
            conversation.goal().and_then(Goal::command),
            Some("cargo test greeting")
        );
    }

    #[test]
    fn the_judge_may_say_why_the_goal_is_met() {
        let mut conversation = toward_words(true);
        assert_eq!(
            conversation.judged(Ok("PASS: main.rs greets by name".to_owned())),
            None
        );
        assert_eq!(
            conversation.last_said(),
            Some("Goal met, as judged: main.rs greets by name")
        );
    }

    #[test]
    fn a_judge_that_gives_no_verdict_has_not_met_the_goal() {
        let mut conversation = toward_words(true);
        let prompt = conversation
            .judged(Ok("I am not sure what you mean.".to_owned()))
            .expect("the chat goes on");
        let Command::Prompt(Prompt { text: prompt, .. }) = prompt else {
            panic!("a prompt");
        };
        assert!(
            prompt
                .starts_with("Goal not met, as judged: it gave no verdict Go on until it is met."),
            "{prompt}"
        );
        assert!(prompt.ends_with("I am not sure what you mean."), "{prompt}");
    }

    #[test]
    fn a_goal_that_cannot_be_judged_waits_for_the_human() {
        let mut conversation = toward_words(true);
        assert_eq!(
            conversation.judged(Err("the chat has no agent to judge it".to_owned())),
            None
        );
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some("The goal could not be judged: the chat has no agent to judge it")
        );
        // What a judge answers once the chat waits changes nothing.
        let entries = conversation.entries().len();
        assert_eq!(conversation.judged(Ok("PASS".to_owned())), None);
        assert_eq!(conversation.entries().len(), entries);
    }

    #[test]
    fn a_goal_in_words_stops_when_the_agent_makes_no_tool_call() {
        let mut conversation = toward_words(false);
        assert_eq!(
            conversation.judged(Ok("FAIL: not yet".to_owned())),
            Some(Command::Prompt(
                "Goal not met, as judged: not yet Go on until it is met. The judge said:\n\nFAIL: not yet"
                    .to_owned()
                    .into()
            ))
        );
        take_turn(&mut conversation, false);
        assert_eq!(conversation.judged(Ok("FAIL: not yet".to_owned())), None);
        assert_eq!(conversation.state(), State::Idle);
        assert_eq!(
            conversation.last_said(),
            Some(
                "Goal not met, as judged: not yet The agent made no tool call in its last 2 turns, so it may wait for you."
            )
        );
    }

    #[test]
    fn a_closed_conversation_keeps_the_goal_it_had() {
        let mut conversation = Conversation::closed(vec![Entry::Human("fix".to_owned())])
            .with_goal(Some(goal::Goal::Command("make".to_owned())));
        assert_eq!(conversation.goal().and_then(Goal::command), Some("make"));

        conversation.reopen();
        conversation.submit("again");
        take_turn(&mut conversation, true);
        assert_eq!(conversation.checking(), Some("make"));
    }
}
