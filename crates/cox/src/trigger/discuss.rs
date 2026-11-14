//! A trigger talked through beside its card, and tried before it is
//! saved, as the stage's Triggers tab asks, the window running both, with
//! the agent a new chat talks to: no chat, as the judge of a goal is not.
//!
//! The agent a card is talked through with reads the project and the
//! trigger's file, and changes nothing: it proposes the trigger whole, in
//! a fenced block of its reply, which the page lays on the card as a
//! draft, for the human to save or not (see [`say`]). A trigger is tried
//! on what a chat in its folder said last as its turn ended, as it would
//! run then, whatever changed (see [`try_on`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use cox_acp::{PermissionAnswer, StopReason};
use cox_core::entry::CheckState;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::card::Draft;
use super::{By, Scope};
use crate::agent::mcp::Ended;
use crate::agent::setup::{Policy, Setup, StageUse};
use crate::agent::{Agent, AgentEvent, Command, Start};
use crate::chat::judge::{ALLOWED, Verdict};
use crate::daemon::chat::GATHER_FOR;
use crate::terminal::run_unseen;
use coxd::agents::Agents;

/// The agents Cox knows, once the window has read them.
static AGENTS: OnceLock<Agents> = OnceLock::new();

/// The talks going on, by the card they are of.
static TALKS: OnceLock<Mutex<HashMap<String, Talk>>> = OnceLock::new();

/// How each card's trigger was last tried, by the card.
static TRIES: OnceLock<Mutex<HashMap<String, Tried>>> = OnceLock::new();

/// How much of what a shell printed a try keeps, from its end.
const PRINTED: usize = 4000;

/// Has the cards talk with, and try their triggers by, one of `agents`.
pub fn start(agents: Agents) {
    let _ = AGENTS.set(agents);
}

/// A talk about a card's trigger.
struct Talk {
    /// What the human says next, as its agent is to be asked it.
    lines: Sender<String>,
    told: Arc<Mutex<Told>>,
}

/// How a talk stands, as the page shows it.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Told {
    /// What was said, the first first.
    pub said: Vec<Said>,
    /// Whether the agent is at its reply.
    pub working: bool,
    /// The agent, by the name the human knows it by.
    pub agent: String,
    /// Why the talk ended, if it did.
    pub error: Option<String>,
}

/// What one side of a talk said.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Said {
    /// `you`, or `agent`.
    pub who: &'static str,
    /// What it said, without the trigger it proposes.
    pub text: String,
    /// The card it proposes, if it proposes one.
    pub draft: Option<Draft>,
    /// Why what it proposes is no trigger, if it is none.
    pub unread: Option<String>,
}

/// A line the human says beside a card.
#[derive(Debug, serde::Deserialize)]
pub struct Line {
    /// The card, as the page tells it apart.
    pub id: String,
    /// The folder the card is shown in, where the agent works.
    pub folder: PathBuf,
    /// The trigger's file, if it has one yet.
    #[serde(default)]
    pub file: Option<PathBuf>,
    /// What the card says now.
    pub draft: Draft,
    /// What the trigger said when it last ran, if it ran.
    #[serde(default)]
    pub ran: Option<String>,
    /// How it went when it was tried on the card last, if it was.
    #[serde(default)]
    pub tried: Option<Tried>,
    /// What the human says.
    pub line: String,
}

/// Has the agent of the card `line.id` told what the human says, starting
/// it first, and answers how the talk stands.
#[must_use]
pub fn say(line: &Line) -> Value {
    let talks = TALKS.get_or_init(Mutex::default);
    let mut talks = talks.lock().unwrap_or_else(PoisonError::into_inner);
    let ended = talks.get(&line.id).is_some_and(|talk| {
        talk.told
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .error
            .is_some()
    });
    if ended {
        talks.remove(&line.id);
    }
    let talk = talks.entry(line.id.clone()).or_insert_with(|| {
        let (lines, heard) = channel();
        let told = Arc::new(Mutex::new(Told::default()));
        let card = Card {
            folder: line.folder.clone(),
            file: line.file.clone(),
            name: line.draft.name.clone(),
            scope: line.draft.scope.unwrap_or(Scope::Project),
        };
        let context = context(line.file.as_deref(), line.ran.as_deref());
        let shared = Arc::clone(&told);
        std::thread::spawn(move || talk(&card, &context, &heard, &shared));
        Talk { lines, told }
    });
    let prompt = prompt(line);
    {
        let mut told = talk.told.lock().unwrap_or_else(PoisonError::into_inner);
        told.said.push(Said {
            who: "you",
            text: line.line.trim().to_owned(),
            draft: None,
            unread: None,
        });
        told.working = true;
    }
    if talk.lines.send(prompt).is_err() {
        talk.told
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .error = Some("the agent closed".to_owned());
    }
    json!(*talk.told.lock().unwrap_or_else(PoisonError::into_inner))
}

/// How the talk of the card `id` stands, if it has one.
#[must_use]
pub fn told(id: &str) -> Value {
    let talks = TALKS.get_or_init(Mutex::default);
    let talks = talks.lock().unwrap_or_else(PoisonError::into_inner);
    talks.get(id).map_or_else(
        || json!(Told::default()),
        |talk| json!(*talk.told.lock().unwrap_or_else(PoisonError::into_inner)),
    )
}

/// Ends the talk of the card `id`, and forgets how it was tried, as the
/// card closes; its agent stops once it has replied.
pub fn end(id: &str) {
    if let Some(talks) = TALKS.get() {
        talks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }
    if let Some(tries) = TRIES.get() {
        tries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
    }
}

/// The card a talk is about.
struct Card {
    folder: PathBuf,
    file: Option<PathBuf>,
    name: String,
    scope: Scope,
}

/// What the agent of a talk is told it is, of the trigger of `file`,
/// which said `ran` when it last ran.
fn context(file: Option<&Path>, ran: Option<&str>) -> String {
    let file = file.map_or_else(
        || "It is new: it has no file yet.".to_owned(),
        |file| format!("Its file is {}.", file.display()),
    );
    let ran = ran.map_or_else(String::new, |ran| {
        format!("\n\nWhen it last ran, it said:\n\n{}\n", ran.trim())
    });
    format!("{STANCE}\n\n{file}{ran}")
}

/// What the agent of a talk is told it is.
const STANCE: &str = "You help the human write one trigger of Cox, on its card, beside \
    which you talk with them. A trigger is a Markdown file whose properties, between `---` \
    lines at its top, say when it runs and who runs it, and whose rest is what is done.\n\n\
    - `on: turn.end` runs it as a chat's turn ends, and what it finds the chat's agent is \
    told, as the next turn. `on: goal.met`, `on: wiki.written` (or `on: {wiki.written: \
    {page: \"research/*\"}}` for some pages), `on: sketch.end`, `on: chat.open` and \
    `on: plan.changed` run it as a chat's goal is met, its agent writes a page of the wiki, \
    a sketch beside it ends, it opens, or its agent changes a step of the plan it started \
    the human's last line with, and what it finds stays on the chat's line, untold. \
    `on: permission.asked` runs it as a chat's agent asks the human's permission, to gather \
    what the human needs to answer: its first line is the headline the human reads first, \
    and the human is asked once the triggers have answered, or a minute went. \
    `on: {schedule: {cron: \"0 9 * * *\"}}` \
    opens a chat at the times the cron says, in the time of the place Cox runs in, the rest \
    being that chat's first line, where `{name}`, `{now}` and `{last}` stand for the \
    trigger's name, when it runs and when it ran last; `project: notes` has that chat \
    work in the project `notes` of Cox's home, rather than in the home.\n\
    - `when:` narrows the events of a chat: `changed: [\"**/*.rs\", \"!target/**\"]` runs it only when \
    files the globs match changed since it last ran, those that start `!` left out, and \
    `sketch: skip` or `sketch: only` keeps it out of sketches, or in them alone.\n\
    - `by: shell` runs the rest as a command in the chat's folder, which passes when it \
    exits 0. `by: this` tells the chat's own agent the rest, as the next turn, in the \
    human's place: on turn.end, once for a line of the human's. `by: new` asks an agent of its own the rest, which ends its reply with `PASS`, \
    or `FAIL: ` and what the chat has to act on; `allow` and `reject` list the patterns of \
    its permission requests Cox answers in the human's place, as `read:*` or \
    `execute:cargo test*`, and with none it may read, search and run the usual tests. A \
    schedule is always `by: new`.\n\n\
    Read what you need of the project, and the trigger's file, but change nothing: only the \
    human changes the card, and saves it. When you propose a change, say in a sentence what \
    you changed, then end your reply with the whole file as it is to be, in a fenced block \
    that opens with ```trigger, which Cox lays on the card for the human to see and save. \
    When you change nothing, write no such block. Answer in the language the human writes \
    in.";

/// What the agent of a talk is asked of `line`: what the card says now,
/// how it went when it was tried, and what the human says.
fn prompt(line: &Line) -> String {
    let tried = line.tried.as_ref().map_or_else(String::new, |tried| {
        let how = match tried.state {
            CheckState::Passed => "it passed",
            CheckState::Failed => "it found something",
            CheckState::Broken => "it could not run",
            CheckState::Waiting | CheckState::Running => "it is still running",
        };
        let said = tried
            .said
            .as_deref()
            .map_or_else(String::new, |said| format!(", saying:\n\n{}", said.trim()));
        format!("Tried on the last turn, {how}{said}\n\n")
    });
    format!(
        "The card reads now:\n\n```trigger\n{}```\n\n{tried}The human says: {}",
        line.draft.written(),
        line.line.trim()
    )
}

/// Talks the trigger of `card` through, the agent told `context`, asking
/// it each line `lines` brings, as `told` keeps, until the card closes.
fn talk(card: &Card, context: &str, lines: &Receiver<String>, told: &Mutex<Told>) {
    let fail = |error: String| {
        let mut told = told.lock().unwrap_or_else(PoisonError::into_inner);
        told.working = false;
        told.error = Some(error);
    };
    let Some(agents) = AGENTS.get() else {
        return fail("Cox knows no agent to talk with".to_owned());
    };
    let known = agents.of(None);
    told.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .agent
        .clone_from(&known.name);
    let setup = Setup {
        context: Some(context.to_owned()),
        tools: Some(Vec::new()),
        stage: StageUse::Look,
        policy: allowed(&[], &[]),
        folder: None,
        sketch: None,
    };
    let start = Start::New {
        instructions: None,
        setup: Some(setup),
    };
    let mut agent = crate::agent::spawn_known(known, card.folder.clone(), Vec::new(), start);
    while let Ok(prompt) = lines.recv() {
        told.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .said
            .push(Said {
                who: "agent",
                text: String::new(),
                draft: None,
                unread: None,
            });
        let replied = ask(&mut agent, prompt, |so_far| {
            if let Some(said) = told
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .said
                .last_mut()
            {
                said.text = shown(so_far);
            }
        });
        let reply = match replied {
            Ok(reply) => reply,
            Err(error) => return fail(error),
        };
        let mut told = told.lock().unwrap_or_else(PoisonError::into_inner);
        told.working = false;
        let Some(said) = told.said.last_mut() else {
            continue;
        };
        said.text = shown(&reply);
        if let Some(proposed) = proposed(&reply) {
            match card.draft_of(&proposed) {
                Ok(draft) => said.draft = Some(draft),
                Err(why) => said.unread = Some(why),
            }
        }
    }
}

impl Card {
    /// The card the file `text` makes, named and kept as this one is.
    fn draft_of(&self, text: &str) -> Result<Draft, String> {
        let file = self.file.clone().unwrap_or_else(|| {
            self.folder
                .join(super::PROJECT_TRIGGERS)
                .join(format!("{}.md", self.name))
        });
        let name = if self.name.trim().is_empty() {
            "trigger"
        } else {
            self.name.trim()
        };
        let trigger = super::Trigger::parse(name, &file, self.scope, text)?;
        let mut draft = Draft::of(&trigger);
        draft.name.clone_from(&self.name);
        Ok(draft)
    }
}

/// `reply` without the trigger it proposes, or the start of one.
fn shown(reply: &str) -> String {
    let Some(at) = reply.find("```trigger") else {
        return reply.trim().to_owned();
    };
    let after = &reply[at + "```trigger".len()..];
    let rest = after
        .find("\n```")
        .map_or("", |end| &after[end + "\n```".len()..]);
    format!("{}\n\n{}", reply[..at].trim(), rest.trim())
        .trim()
        .to_owned()
}

/// The trigger `reply` proposes, whole, in its fenced block, if it
/// proposes one.
fn proposed(reply: &str) -> Option<String> {
    let at = reply.find("```trigger")?;
    let after = &reply[at + "```trigger".len()..];
    let after = after.strip_prefix('\n').unwrap_or(after);
    let end = after.find("\n```")?;
    Some(after[..=end].to_owned())
}

/// What `agent` replies to `prompt`, as far as it has as it goes being
/// told `so_far`; its permission requests the policy does not answer
/// refused.
fn ask(agent: &mut Agent, prompt: String, mut so_far: impl FnMut(&str)) -> Result<String, String> {
    agent
        .commands
        .send(Command::Prompt(prompt.into()))
        .map_err(|_| "the agent closed".to_owned())?;
    let mut reply = String::new();
    loop {
        let Some(event) = agent.events.blocking_recv() else {
            return Err("the agent closed".to_owned());
        };
        match event {
            AgentEvent::ReplyText(text) => {
                reply.push_str(&text);
                so_far(&reply);
            }
            AgentEvent::PermissionRequest { .. } => {
                let _ = agent
                    .commands
                    .send(Command::Answer(PermissionAnswer::Cancel));
            }
            AgentEvent::LoginNeeded { .. } => {
                return Err(
                    "the agent asks to log in: log in from a chat of it, then try again".to_owned(),
                );
            }
            AgentEvent::TurnEnded(StopReason::EndTurn) => return Ok(reply),
            AgentEvent::TurnEnded(reason) => {
                return Err(format!("its turn ended early: {reason:?}"));
            }
            AgentEvent::TurnFailed(error) => return Err(format!("its turn failed: {error}")),
            AgentEvent::Closed(error) => {
                return Err(error.map_or_else(
                    || "the agent closed".to_owned(),
                    |error| format!("the agent closed: {error}"),
                ));
            }
            _ => {}
        }
    }
}

/// A policy that allows what `allow` names, or, with none, what the judge
/// of a goal may, and refuses what `reject` names.
fn allowed(allow: &[String], reject: &[String]) -> Policy {
    Policy {
        allow: if allow.is_empty() {
            ALLOWED.iter().map(|&title| title.to_owned()).collect()
        } else {
            allow.to_vec()
        },
        reject: reject.to_vec(),
        within: None,
    }
}

/// How a card's trigger went, tried.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tried {
    pub state: CheckState,
    #[serde(default)]
    pub said: Option<String>,
}

/// A card's trigger to try.
#[derive(Debug, serde::Deserialize)]
pub struct Try {
    pub id: String,
    pub folder: PathBuf,
    #[serde(default)]
    pub file: Option<PathBuf>,
    pub draft: Draft,
}

/// Tries the trigger of a card on what a chat in its folder said last as
/// its turn ended, as `last`, the file Cox keeps that in, has it, and
/// answers how it stands: running, at first.
#[must_use]
pub fn try_on(asked: Try, last: Option<PathBuf>) -> Value {
    let tries = TRIES.get_or_init(Mutex::default);
    let running = Tried {
        state: CheckState::Running,
        said: None,
    };
    tries
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(asked.id.clone(), running.clone());
    std::thread::spawn(move || {
        let went = tried_on(&asked, last.as_deref());
        tries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(asked.id, went);
    });
    json!(running)
}

/// How the trigger of the card `id` went when it was last tried, if it
/// was.
#[must_use]
pub fn tried(id: &str) -> Value {
    let tries = TRIES.get_or_init(Mutex::default);
    json!(tries.lock().unwrap_or_else(PoisonError::into_inner).get(id))
}

/// How the trigger of `tried` goes on the last turn `last` keeps.
fn tried_on(tried: &Try, last: Option<&Path>) -> Tried {
    let broken = |said: String| Tried {
        state: CheckState::Broken,
        said: Some(said),
    };
    let file = tried.file.clone().unwrap_or_else(|| {
        tried
            .folder
            .join(super::PROJECT_TRIGGERS)
            .join(format!("{}.md", tried.draft.name.trim()))
    });
    let trigger = match tried.draft.trigger(&file) {
        Ok(trigger) => trigger,
        Err(why) => return broken(why),
    };
    if trigger.cron().is_some() {
        return broken("A schedule runs at its times, on no turn.".to_owned());
    }
    match &trigger.by {
        By::Shell => run(&trigger.text, &tried.folder),
        // There is no chat to try it in: what it tells is all it does.
        By::This => Tried {
            state: CheckState::Passed,
            said: Some(format!(
                "It tells the chat's agent, in your place:\n\n{}",
                trigger.text.trim()
            )),
        },
        By::New { allow, reject, .. } => {
            let Some(said) = super::card::last_turn(last, &tried.folder) else {
                return broken("No turn has ended in this folder yet, to try it on.".to_owned());
            };
            let Some(agents) = AGENTS.get() else {
                return broken("Cox knows no agent to try it with.".to_owned());
            };
            let mut setup = super::setup(&trigger.text, allow, reject);
            setup.policy = allowed(allow, reject);
            let start = Start::New {
                instructions: None,
                setup: Some(setup),
            };
            let mut agent =
                crate::agent::spawn_known(agents.of(None), tried.folder.clone(), Vec::new(), start);
            match ask(&mut agent, super::question(said.as_deref()), |_| {}) {
                Ok(reply) => Tried {
                    state: match Verdict::of(&reply) {
                        Verdict::Met(_) => CheckState::Passed,
                        Verdict::NotMet(_) | Verdict::Blocked(_) => CheckState::Failed,
                    },
                    said: Some(reply.trim().to_owned()),
                },
                Err(error) => broken(error),
            }
        }
    }
}

/// How the command `text` goes in `folder`, run as a chat runs it when
/// no window is there to show it: what its terminal shows, the end of
/// it, and whether it exited 0. One that takes longer than a chat waits
/// on a trigger is stopped as a chat stops it, and fails.
fn run(text: &str, folder: &Path) -> Tried {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return cannot_run(&error.to_string()),
    };
    let (answer, ran) = oneshot::channel();
    let ((), ran) = runtime.block_on(async {
        tokio::join!(
            run_unseen(text.to_owned(), folder, answer),
            tokio::time::timeout(GATHER_FOR, ran),
        )
    });
    let ran = match ran {
        Ok(Ok(Ok(ran))) => ran,
        Ok(Ok(Err(error))) => return cannot_run(&error),
        Ok(Err(_)) => return cannot_run("its terminal went away"),
        Err(_) => {
            return Tried {
                state: CheckState::Failed,
                said: Some(format!(
                    "It took longer than {} seconds, so it was stopped.",
                    GATHER_FOR.as_secs()
                )),
            };
        }
    };
    let printed = ran.output.trim();
    let start = printed.len().saturating_sub(PRINTED);
    let start = (start..=printed.len())
        .find(|&at| printed.is_char_boundary(at))
        .unwrap_or(printed.len());
    let ended = match ran.ended {
        Ended::Exited(code) => format!("it exited {code}"),
        Ended::Signal(_) | Ended::Unknown | Ended::Closed => "it was stopped".to_owned(),
    };
    Tried {
        state: if ran.ended == Ended::Exited(0) {
            CheckState::Passed
        } else {
            CheckState::Failed
        },
        said: Some(
            format!("{}\n\n{ended}", &printed[start..])
                .trim()
                .to_owned(),
        ),
    }
}

/// How a command that could not run for `error` went.
fn cannot_run(error: &str) -> Tried {
    Tried {
        state: CheckState::Broken,
        said: Some(format!("cannot run it: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coxd::testing::TempDir;

    #[test]
    fn a_reply_proposes_the_trigger_in_its_fenced_block() {
        let reply = "I left tests out.\n\n```trigger\n---\non: turn.end\nby: shell\n---\n\ncargo test\n```\n";
        assert_eq!(shown(reply), "I left tests out.");
        assert_eq!(
            proposed(reply).as_deref(),
            Some("---\non: turn.end\nby: shell\n---\n\ncargo test\n")
        );
        // While it arrives, the block does not show.
        assert_eq!(shown("Here:\n\n```trigger\n---\non: tu"), "Here:");
        assert_eq!(proposed("Here:\n\n```trigger\n---\non: tu"), None);
        assert_eq!(proposed("Nothing to change."), None);

        let card = Card {
            folder: PathBuf::from("/p"),
            file: None,
            name: "ci".to_owned(),
            scope: Scope::Project,
        };
        let draft = card.draft_of(&proposed(reply).unwrap()).unwrap();
        assert_eq!(draft.name, "ci");
        assert_eq!(draft.by, "shell");
        assert_eq!(draft.text, "cargo test");
        assert!(card.draft_of("---\non: never\n---\nx").is_err());
    }

    #[test]
    fn a_shell_trigger_is_tried_in_its_folder() {
        let folder = TempDir::new();
        let tried = run("pwd -P", &folder);
        assert_eq!(
            tried.said,
            Some(format!("{}\n\nit exited 0", folder.display()))
        );
    }

    #[test]
    fn a_shell_trigger_is_tried_on_a_terminal_as_a_chat_runs_it() {
        let folder = TempDir::new();
        let tried = run("test -t 0 && test -t 1 && echo terminal", &folder);
        assert_eq!(tried.said.as_deref(), Some("terminal\n\nit exited 0"));
    }

    #[test]
    fn a_shell_trigger_that_exits_0_passes_saying_what_it_printed() {
        let folder = TempDir::new();
        let passed = run("echo hi", &folder);
        assert_eq!(passed.state, CheckState::Passed);
        assert_eq!(passed.said.as_deref(), Some("hi\n\nit exited 0"));
    }

    #[test]
    fn a_shell_trigger_that_exits_otherwise_fails_saying_what_it_printed() {
        let folder = TempDir::new();
        let failed = run("echo no >&2; exit 3", &folder);
        assert_eq!(failed.state, CheckState::Failed);
        assert_eq!(failed.said.as_deref(), Some("no\n\nit exited 3"));
    }

    #[test]
    fn the_agent_of_a_talk_is_told_the_card_and_how_it_went() {
        let line = Line {
            id: "1".to_owned(),
            folder: PathBuf::from("/p"),
            file: None,
            draft: Draft {
                name: "ci".to_owned(),
                on: "turn.end".to_owned(),
                by: "shell".to_owned(),
                text: "cargo test".to_owned(),
                ..Draft::default()
            },
            ran: None,
            tried: Some(Tried {
                state: CheckState::Failed,
                said: Some("1 failed".to_owned()),
            }),
            line: "Why?".to_owned(),
        };
        assert_eq!(
            prompt(&line),
            "The card reads now:\n\n```trigger\n---\non: turn.end\nby: shell\n---\n\ncargo test\n```\n\nTried on the last turn, it found something, saying:\n\n1 failed\n\nThe human says: Why?"
        );
    }

    #[test]
    fn the_agent_of_a_talk_is_told_the_trigger_s_file_and_what_it_last_said() {
        assert!(context(None, Some("FAIL: x")).ends_with(
            "It is new: it has no file yet.\n\nWhen it last ran, it said:\n\nFAIL: x\n"
        ));
        assert!(
            context(Some(Path::new("/p/ci.md")), None).ends_with("Its file is /p/ci.md."),
            "a trigger with a file, never run"
        );
    }
}
