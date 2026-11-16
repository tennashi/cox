//! What goes on in a folder, as the stage's Activity page shows it, at
//! `cox://api/activity` (see [`crate::stage::plugins`]): the human's
//! inbox, where they look first when they open Cox or have a moment. It
//! tells, in tiers, what is asked of them, the chats of the folder that
//! ask them something, with what they ask, or whose turn ended while they
//! were not looking; what is at work, the chats whose agents take a turn,
//! or work in the background once it ended, each with what it does now,
//! or runs in the background, its subagents under it; what was decided,
//! the questions agents asked there that the human answered, each with
//! what they chose and why, the last first; and what happened without
//! them, the schedules that ran there, the pages of the wiki written from
//! there, and the sources kept there, the last first. The home, where Cox
//! lands, tells of every folder.
//!
//! The window tells this module how its chats stand as they change (see
//! [`tell`]), since the stage's pages are served apart from it; the bar
//! counts what waits on the Activity's button (see [`waiting`]). What a
//! chat does now, and what it asks, come from what the window told the
//! Commands page of its conversation (see [`commands::now`]). What was
//! decided is read from the chats as coxd keeps them (see
//! [`Store::answered`]), so it holds for chats the window never showed,
//! and lasts as long as their conversations do: read again once a chat
//! stops asking, as when the human answers, and once one goes.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use cox_core::entry::{Entry, QuestionAnswer};
use cox_core::store::{ChatId, KeptAnswer, Store};
use cox_core::tasks::{Status, Tree};
use jiff::tz::TimeZone;
use serde_json::{Value, json};

use crate::stage::commands;

/// How many of what happened the page is told of, the last first.
const HAPPENED: usize = 50;
/// How many of what was decided the page is told of, the last first.
const DECIDED: usize = 20;
/// How many times what was decided is read again after a chat stopped
/// asking: coxd may tell the window so before it keeps the answer.
const READ_AGAIN: u8 = 2;

/// A chat as the Activity page is told of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chat {
    /// The number the window goes by for it this run, which a click on
    /// it, as `cox://chat/<key>`, shows.
    pub key: u64,
    pub name: String,
    /// Where it works.
    pub cwd: PathBuf,
    /// The folder of its project, which a sketch's differs from.
    pub project: PathBuf,
    /// Whether it asks the human something.
    pub asking: bool,
    /// Whether its turn ended while the human was not looking.
    pub unread: bool,
    /// Its goal, as the human set it, if it has one.
    pub goal: Option<String>,
    /// Whether its agent takes a turn, or stops one, or works in the
    /// background once its turn ended.
    pub working: bool,
    /// What its agent runs in the background, once its turn ended.
    pub background: Vec<coxd::daemon::wire::Background>,
    /// The key of the chat whose agent opened it, if one did.
    pub parent: Option<u64>,
    /// How coxd keeps it, which what it decided is read by.
    pub kept: Option<ChatId>,
}

/// A question an agent asked in a chat, which the human answered: what
/// the Decided tier lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decided {
    pub chat: ChatId,
    /// The name of the chat, as coxd keeps it.
    pub name: String,
    /// The folder of the chat's project, or `None` for Cox's own.
    pub project: Option<PathBuf>,
    /// The folder the chat works in, when it is not its project's.
    pub folder: Option<PathBuf>,
    /// When it was asked, as `2026-11-16 09:30`.
    pub when: String,
    /// The question's short label, as "Approach", if the agent gave one.
    pub header: Option<String>,
    pub question: String,
    /// The labels of the options the human chose, or their own words when
    /// they chose none.
    pub choice: String,
    /// What the agent said choosing those options means, which is why
    /// the human chose them.
    pub why: Option<String>,
    /// The human's own words beside the options they chose.
    pub other: Option<String>,
}

impl Decided {
    /// What was decided by `kept`, each of its questions the human
    /// answered, its time as it reads in `zone`.
    fn of(kept: &KeptAnswer, zone: &TimeZone) -> Vec<Self> {
        let Entry::Question {
            message,
            questions,
            answer: Some(QuestionAnswer::Given(picked)),
            ..
        } = &kept.entry
        else {
            return Vec::new();
        };
        let when = jiff::Timestamp::from_millisecond(kept.at).map_or_default(|at| {
            at.to_zoned(zone.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        });
        let message = message.trim();
        questions
            .iter()
            .zip(picked)
            .filter_map(|(question, picked)| {
                let chosen: Vec<_> = picked
                    .chosen
                    .iter()
                    .filter_map(|&index| question.options.get(index))
                    .collect();
                let mut other = picked
                    .other
                    .clone()
                    .filter(|other| !other.trim().is_empty());
                let choice = if chosen.is_empty() {
                    other.take()?
                } else {
                    chosen
                        .iter()
                        .map(|option| option.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let why: Vec<&str> = chosen
                    .iter()
                    .filter_map(|option| option.description.as_deref())
                    .collect();
                let asked = question
                    .text
                    .clone()
                    .or_else(|| {
                        (questions.len() == 1 && !message.is_empty()).then(|| message.to_owned())
                    })
                    .or_else(|| question.header.clone())
                    .unwrap_or_else(|| message.to_owned());
                Some(Self {
                    chat: kept.chat,
                    name: kept.name.clone(),
                    project: kept.project.clone(),
                    folder: kept.folder.clone(),
                    when: when.clone(),
                    header: question.header.clone(),
                    question: asked,
                    choice,
                    why: (!why.is_empty()).then(|| why.join("; ")),
                    other,
                })
            })
            .collect()
    }
}

/// How the window's chats stand, and its projects, by folder and name,
/// with the home, the folder of Cox's own chats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Told {
    pub chats: Vec<Chat>,
    pub projects: Vec<(PathBuf, String)>,
    pub home: PathBuf,
}

/// How the chats stood when the window last told.
static TOLD: Mutex<Option<Told>> = Mutex::new(None);

/// What was decided, as last read, and how many times to read it again
/// before that holds.
struct Read {
    decided: Option<Vec<Decided>>,
    again: u8,
}

static READ: Mutex<Read> = Mutex::new(Read {
    decided: None,
    again: 0,
});

/// Keeps how the window's chats stand now, for the pages to read, and
/// has what was decided read again if a chat stopped asking, or went.
pub fn tell(told: Told) {
    let Ok(mut now) = TOLD.lock() else {
        return;
    };
    let changed = now.as_ref().is_some_and(|was| decided_since(was, &told));
    *now = Some(told);
    drop(now);
    if changed && let Ok(mut read) = READ.lock() {
        read.again = READ_AGAIN;
    }
}

/// Whether what was decided may have changed from when the chats stood
/// as `was` to how they stand `now`: a chat stopped asking, as when the
/// human answered it, or went, and with it what it decided.
fn decided_since(was: &Told, now: &Told) -> bool {
    was.chats.iter().any(|chat| {
        let still = now.chats.iter().find(|now| now.key == chat.key);
        still.is_none_or(|still| chat.asking && !still.asking)
    })
}

/// What was decided in every chat coxd keeps, the last asked first, read
/// again once what the window told says it changed.
fn decided() -> Vec<Decided> {
    let Ok(mut read) = READ.lock() else {
        return Vec::new();
    };
    if read.decided.is_none() || read.again > 0 {
        read.again = read.again.saturating_sub(1);
        let zone = TimeZone::system();
        let answered = coxd::storage::path(coxd::storage::Kept::Chats)
            .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
            .and_then(|path| Store::read(&path)?.answered());
        match answered {
            Ok(answered) => {
                read.decided = Some(
                    answered
                        .iter()
                        .flat_map(|kept| Decided::of(kept, &zone))
                        .collect(),
                );
            }
            Err(error) => tracing::warn!("what the chats decided is not read: {error:#}"),
        }
    }
    read.decided.clone().unwrap_or_default()
}

/// How the chats stood when the window last told, or none of them.
fn now() -> Told {
    TOLD.lock()
        .ok()
        .and_then(|now| now.clone())
        .unwrap_or_default()
}

/// The window's projects, by folder and name, as it last told.
#[must_use]
pub fn projects() -> Vec<(PathBuf, String)> {
    now().projects
}

/// The home, the folder of Cox's own chats, as the window last told, if it
/// told.
#[must_use]
pub fn home() -> Option<PathBuf> {
    Some(now().home).filter(|home| !home.as_os_str().is_empty())
}

/// Whether `chat` is of the folder `root`, or of any, when `root` is the
/// home of `told`.
fn of_folder(told: &Told, chat: &Chat, root: &Path) -> bool {
    root == told.home || chat.cwd == root || chat.project == root
}

/// Whether `chat` waits for the human and is of the folder `root`, or
/// of any, when `root` is the home of `told`.
fn waits_in(told: &Told, chat: &Chat, root: &Path) -> bool {
    (chat.asking || chat.unread) && of_folder(told, chat, root)
}

/// How many chats of the folder `root` wait for the human, for the bar.
#[must_use]
pub fn waiting(root: &Path) -> usize {
    let told = now();
    told.chats
        .iter()
        .filter(|chat| waits_in(&told, chat, root))
        .count()
}

/// What goes on in the folder `root`, as JSON, from how the chats stand
/// now, what they do and ask, as the Commands page was told, what they
/// decided, the human's `wiki`, the folder's own, the sources at `raw`,
/// and `triggers`, as [`coxd::trigger::told`] tells them, of which the
/// schedules ran.
#[must_use]
pub fn told(root: &Path, wiki: &Path, raw: &Path, triggers: &Value) -> Value {
    told_of(
        &now(),
        root,
        &decided(),
        &trees(),
        &commands::now,
        &Places {
            wiki,
            raw,
            triggers,
        },
    )
}

/// The tasks of each chat whose agent said any, as coxd keeps them.
fn trees() -> Vec<(ChatId, Tree)> {
    let trees = coxd::storage::path(coxd::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("no place keeps the chats"))
        .and_then(|path| Store::read(&path)?.trees());
    trees.unwrap_or_else(|error| {
        tracing::warn!("the tasks of the chats are not read: {error:#}");
        Vec::new()
    })
}

/// Where what happened in a folder is read from: the human's wiki, their
/// sources, and the triggers, as [`coxd::trigger::told`] tells them.
struct Places<'a> {
    wiki: &'a Path,
    raw: &'a Path,
    triggers: &'a Value,
}

/// The project of `folder`, by its name, as `told` says it.
fn name_of(told: &Told, folder: &Path) -> Option<String> {
    told.projects
        .iter()
        .find(|(project, _)| project == folder)
        .map(|(_, name)| name.clone())
}

/// Where `project` is, as the home tells it: by its name, or `~` for
/// the home's own.
fn where_of(told: &Told, project: &Path) -> Option<String> {
    if project == told.home {
        Some("~".to_owned())
    } else {
        name_of(told, project)
    }
}

/// The chats of the folder `root` that wait for the human, or of every
/// folder in the home, each with where it is, and what it asks, as
/// `now` says, as JSON.
fn waiting_in(told: &Told, root: &Path, now: &dyn Fn(u64) -> Option<commands::Now>) -> Vec<Value> {
    told.chats
        .iter()
        .filter(|chat| waits_in(told, chat, root))
        .map(|chat| {
            json!({
                "key": chat.key,
                "name": chat.name,
                "why": if chat.asking { "asks you" } else { "finished a turn" },
                "asks": chat.asking.then(|| now(chat.key).and_then(|now| now.asks)).flatten(),
                "goal": chat.goal,
                "sketch": chat.cwd != chat.project,
                "where": where_of(told, &chat.project),
            })
        })
        .collect()
}

/// The chats of the folder `root` at work, or that planned tasks not all
/// done, or of every folder in the home, each with what it does now, as
/// `now` says, and its tasks, of `trees`, and after each the subagents
/// at work it opened that no task of it was handed to, as JSON.
fn at_work(
    told: &Told,
    root: &Path,
    trees: &[(ChatId, Tree)],
    now: &dyn Fn(u64) -> Option<commands::Now>,
) -> Vec<Value> {
    let tree_of = |chat: &Chat| {
        chat.kept
            .and_then(|kept| trees.iter().find(|(id, _)| *id == kept))
            .map(|(_, tree)| tree)
    };
    let working: Vec<&Chat> = told
        .chats
        .iter()
        .filter(|chat| {
            (chat.working || tree_of(chat).is_some_and(|tree| !tree.is_done()))
                && of_folder(told, chat, root)
        })
        .collect();
    let handed: Vec<ChatId> = working
        .iter()
        .filter_map(|chat| tree_of(chat))
        .flat_map(|tree| tree.tasks().iter().filter_map(|task| task.chat))
        .collect();
    let mut listed = Vec::new();
    let mut under: Vec<(&Chat, usize)> = working
        .iter()
        .rev()
        .filter(|chat| {
            !chat
                .parent
                .is_some_and(|parent| working.iter().any(|other| other.key == parent))
        })
        .map(|chat| (*chat, 0))
        .collect();
    while let Some((chat, depth)) = under.pop() {
        let does = now(chat.key).unwrap_or_default();
        listed.push(json!({
            "key": chat.key,
            "name": chat.name,
            "goal": chat.goal,
            "depth": depth,
            "working": chat.working,
            "sketch": chat.cwd != chat.project,
            "where": where_of(told, &chat.project),
            "said": does.said,
            "running": does.running,
            "background": chat
                .background
                .iter()
                .map(|background| {
                    json!({
                        "what": background.what(),
                        "command": background.is_command(),
                    })
                })
                .collect::<Vec<_>>(),
            "tasks": tree_of(chat).map_or_else(Vec::new, |tree| tasks(told, tree, now)),
        }));
        under.extend(
            working
                .iter()
                .rev()
                .filter(|other| {
                    other.parent == Some(chat.key)
                        && !other.kept.is_some_and(|kept| handed.contains(&kept))
                })
                .map(|other| (*other, depth + 1)),
        );
    }
    listed
}

/// The tasks of `tree`, as JSON, each with the chat it was handed to, by
/// its key and name when the window lists it, and what it does now, as
/// `now` says, while it is at work.
fn tasks(told: &Told, tree: &Tree, now: &dyn Fn(u64) -> Option<commands::Now>) -> Vec<Value> {
    tree.tasks()
        .iter()
        .map(|task| {
            let (status, note) = match &task.status {
                Status::Pending => ("pending", None),
                Status::InProgress => ("in_progress", None),
                Status::Done { result } => ("done", result.as_deref()),
                Status::Blocked { blocker } => ("blocked", Some(blocker.as_str())),
            };
            let chat = task.chat.map(|kept| {
                match told.chats.iter().find(|chat| chat.kept == Some(kept)) {
                    Some(chat) => json!({
                        "key": chat.key,
                        "name": chat.name,
                        "working": chat.working,
                        "said": chat.working.then(|| now(chat.key).and_then(|now| now.said)).flatten(),
                    }),
                    None => json!({ "key": null, "name": format!("chat {kept}"), "working": false }),
                }
            });
            json!({
                "id": task.id.as_str(),
                "title": task.title,
                "detail": task.detail,
                "depth": task.depth(tree),
                "status": status,
                "note": note,
                "chat": chat,
            })
        })
        .collect()
}

/// What was decided in the folder `root`, or in any in the home, the
/// last [`DECIDED`] of `decided`, as JSON: each chat by its key when the
/// window lists it, and by its name there.
fn decided_in(told: &Told, root: &Path, decided: &[Decided]) -> Vec<Value> {
    decided
        .iter()
        .filter(|decided| {
            let project = decided.project.as_deref().unwrap_or(&told.home);
            root == told.home || project == root || decided.folder.as_deref() == Some(root)
        })
        .take(DECIDED)
        .map(|decided| {
            let listed = told
                .chats
                .iter()
                .find(|chat| chat.kept == Some(decided.chat));
            json!({
                "when": decided.when,
                "header": decided.header,
                "question": decided.question,
                "choice": decided.choice,
                "why": decided.why,
                "other": decided.other,
                "chat": listed.map_or(decided.name.as_str(), |chat| chat.name.as_str()),
                "key": listed.map(|chat| chat.key),
                "where": where_of(told, decided.project.as_deref().unwrap_or(&told.home)),
            })
        })
        .collect()
}

/// What goes on in the folder `root`, as [`told`] tells it, the chats
/// standing as `told` says, doing and asking as `now` says, having
/// `decided` what they did, and what happened read from `places`.
fn told_of(
    told: &Told,
    root: &Path,
    decided: &[Decided],
    trees: &[(ChatId, Tree)],
    now: &dyn Fn(u64) -> Option<commands::Now>,
    places: &Places<'_>,
) -> Value {
    let Places {
        wiki,
        raw,
        triggers,
    } = *places;
    let home = root == told.home;
    let waiting = waiting_in(told, root, now);
    let working = at_work(told, root, trees, now);
    let decided = decided_in(told, root, decided);
    let name = name_of(told, root);
    let mut happened = Vec::new();
    for folder in triggers["folders"].as_array().into_iter().flatten() {
        let place = folder["name"].as_str().unwrap_or("~");
        for trigger in folder["triggers"].as_array().into_iter().flatten() {
            let schedule = &trigger["schedule"];
            let Some(last) = schedule["last"].as_str() else {
                continue;
            };
            let every = schedule["every"].as_str().unwrap_or_default();
            happened.push(json!({
                "when": last,
                "kind": "schedule",
                "title": trigger["name"],
                "says": if home {
                    format!("ran in {place}, {every}")
                } else {
                    format!("ran, {every}")
                },
                "href": "cox://triggers/",
            }));
        }
    }
    let written =
        |root: &Path, shelf: &str, of: &dyn Fn(&str) -> bool, happened: &mut Vec<Value>| {
            let told = coxd::wiki::told::told(root);
            for page in told["pages"].as_array().into_iter().flatten() {
                let name = page["name"].as_str().unwrap_or_default();
                for used in page["used"].as_array().into_iter().flatten() {
                    let what = used["what"].as_str().unwrap_or_default();
                    if of(what) {
                        happened.push(json!({
                            "when": used["day"],
                            "kind": "wiki",
                            "title": page["title"],
                            "says": what,
                            "href": format!("cox://wiki/#{shelf}{name}"),
                        }));
                    }
                }
            }
        };
    if home {
        written(wiki, "", &|_: &str| true, &mut happened);
    } else if let Some(name) = &name {
        let ours = format!("in {name}, chat ");
        written(
            wiki,
            "",
            &|what: &str| what.starts_with(&ours),
            &mut happened,
        );
    }
    if !home {
        written(
            &root.join(coxd::wiki::PROJECT_WIKI),
            "project:",
            &|_: &str| true,
            &mut happened,
        );
    }
    kept_sources(root, raw, home, name.as_deref(), &mut happened);
    happened.sort_by(|a, b| {
        let when = |event: &Value| event["when"].as_str().unwrap_or_default().to_owned();
        when(b).cmp(&when(a))
    });
    happened.truncate(HAPPENED);
    json!({
        "folder": root,
        "home": home,
        "waiting": waiting,
        "working": working,
        "decided": decided,
        "happened": happened,
    })
}

/// Pushes onto `happened` the sources kept of the folder `root`, the
/// project of `name`, if it is one: those of the human's sources at
/// `raw` the project kept, or all of them in the home, and those of the
/// project's own.
fn kept_sources(
    root: &Path,
    raw: &Path,
    home: bool,
    name: Option<&str>,
    happened: &mut Vec<Value>,
) {
    let mut sources: Vec<(Value, &str)> = coxd::raw::told(raw)["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|source| (source.clone(), "raw:"))
        .collect();
    if !home {
        sources.extend(
            coxd::raw::told(&root.join(coxd::raw::PROJECT_RAW))["sources"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|source| (source.clone(), "project-raw:")),
        );
    }
    for (source, shelf) in &sources {
        let project = source["project"].as_str();
        let kept_here = home || *shelf == "project-raw:" || (project.is_some() && name == project);
        if kept_here {
            happened.push(json!({
                "when": source["kept"],
                "kind": "source",
                "title": source["title"],
                "says": format!(
                    "kept in {}{}{}",
                    source["topic"].as_str().unwrap_or_default(),
                    source["chat"].as_str().map_or_default(|chat| format!(" by {chat}")),
                    project
                        .filter(|_| home)
                        .map_or_default(|project| format!(" in {project}"))
                ),
                "href": format!("cox://wiki/#{shelf}{}", source["name"].as_str().unwrap_or_default()),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coxd::testing::TempDir;

    fn chat(key: u64, cwd: &Path, project: &Path, asking: bool, unread: bool) -> Chat {
        Chat {
            key,
            name: format!("chat {key}"),
            cwd: cwd.to_owned(),
            project: project.to_owned(),
            asking,
            unread,
            goal: None,
            working: false,
            background: Vec::new(),
            parent: None,
            kept: None,
        }
    }

    /// What goes on in the folder `root`, nothing decided, doing or
    /// asking anything, and what happened read from `wiki`, `raw` and
    /// `triggers`.
    fn told_at(told: &Told, root: &Path, wiki: &Path, raw: &Path, triggers: &Value) -> Value {
        let places = Places {
            wiki,
            raw,
            triggers,
        };
        told_of(told, root, &[], &[], &|_| None, &places)
    }

    /// What goes on in the folder `root`, where nothing happened.
    fn told_in(told: &Told, root: &Path, at: &Path) -> Value {
        told_at(told, root, &at.join("wiki"), &at.join("raw"), &json!({}))
    }

    #[test]
    fn what_waits_is_of_the_folder_and_its_sketches() {
        let root = TempDir::new();
        let other = root.join("other");
        let sketch = root.join("sketch");
        let told = Told {
            chats: vec![
                chat(1, &root, &root, true, false),
                chat(2, &root, &root, false, false),
                chat(3, &sketch, &root, false, true),
                chat(4, &other, &other, true, false),
            ],
            projects: vec![(root.to_path_buf(), "app".to_owned())],
            home: root.join("home"),
        };
        let json = told_in(&told, &root, &root);
        let waiting = json["waiting"].as_array().unwrap();
        assert_eq!(waiting.len(), 2);
        assert_eq!(waiting[0]["key"], 1);
        assert_eq!(waiting[0]["why"], "asks you");
        assert_eq!(waiting[1]["key"], 3);
        assert_eq!(waiting[1]["why"], "finished a turn");
        assert_eq!(waiting[1]["sketch"], true);
        assert_eq!(waiting[1]["where"], "app");

        // The home tells of what waits in every folder.
        let home = told.home.clone();
        let json = told_in(&told, &home, &home);
        let keys: Vec<_> = json["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| chat["key"].as_u64().unwrap())
            .collect();
        assert_eq!(keys, [1, 3, 4]);
    }

    #[test]
    fn a_question_answered_is_decided_with_why_and_a_declined_one_is_not() {
        use cox_core::entry::{Picked, Question, QuestionOption};

        let option = |label: &str, description: Option<&str>| QuestionOption {
            label: label.to_owned(),
            description: description.map(str::to_owned),
        };
        let question = Question {
            header: Some("Approach".to_owned()),
            text: None,
            options: vec![
                option("Derive", Some("One source of truth")),
                option("Record", Some("A table of its own")),
            ],
            multiple: false,
            other: true,
        };
        let words = Question {
            header: Some("Name".to_owned()),
            text: Some("What is it called?".to_owned()),
            options: Vec::new(),
            multiple: false,
            other: true,
        };
        let kept = |answer| KeptAnswer {
            chat: serde_json::from_value(json!(7)).unwrap(),
            name: "plan".to_owned(),
            project: Some(PathBuf::from("/src/app")),
            folder: None,
            at: 1_763_280_000_000,
            entry: Entry::Question {
                message: "Which way do we go?".to_owned(),
                questions: vec![question.clone(), words.clone()],
                answer: Some(answer),
                answered_at: None,
            },
        };
        let given = QuestionAnswer::Given(vec![
            Picked {
                chosen: vec![0],
                other: Some("but keep it cheap".to_owned()),
            },
            Picked {
                chosen: Vec::new(),
                other: Some("inbox".to_owned()),
            },
        ]);

        let decided = Decided::of(&kept(given), &TimeZone::UTC);

        assert_eq!(decided.len(), 2);
        assert_eq!(decided[0].when, "2025-11-16 08:00");
        assert_eq!(decided[0].header.as_deref(), Some("Approach"));
        // Of several questions, one without words of its own is asked by
        // its label, the message being for them all.
        assert_eq!(decided[0].question, "Approach");
        assert_eq!(decided[0].choice, "Derive");
        assert_eq!(decided[0].why.as_deref(), Some("One source of truth"));
        assert_eq!(decided[0].other.as_deref(), Some("but keep it cheap"));
        assert_eq!(decided[1].question, "What is it called?");
        assert_eq!(decided[1].choice, "inbox");
        assert_eq!(decided[1].why, None);
        assert_eq!(decided[1].other, None);
        assert_eq!(
            Decided::of(&kept(QuestionAnswer::Declined), &TimeZone::UTC),
            []
        );
        let skipped = QuestionAnswer::Given(vec![Picked::default(), Picked::default()]);
        assert_eq!(Decided::of(&kept(skipped), &TimeZone::UTC), []);
    }

    #[test]
    fn the_inbox_tells_what_is_asked_at_work_and_decided_in_the_folder() {
        let root = TempDir::new();
        let other = root.join("other");
        let home = root.join("home");
        let kept = |number: i64| -> ChatId { serde_json::from_value(json!(number)).unwrap() };
        let mut asking = chat(1, &root, &root, true, false);
        asking.kept = Some(kept(1));
        let mut parent = chat(2, &root, &root, false, false);
        parent.working = true;
        parent.goal = Some("ship it".to_owned());
        let mut subagent = chat(3, &root, &root, false, false);
        subagent.working = true;
        subagent.parent = Some(2);
        let mut elsewhere = chat(4, &other, &other, false, false);
        elsewhere.working = true;
        let mut orphan = chat(5, &root, &root, false, false);
        orphan.working = true;
        orphan.parent = Some(9);
        orphan.background = vec![coxd::daemon::wire::Background {
            kind: coxd::daemon::wire::BackgroundKind::Command,
            name: "sleep 120".to_owned(),
            description: None,
        }];
        let told = Told {
            chats: vec![asking, subagent, parent, elsewhere, orphan],
            projects: vec![
                (root.to_path_buf(), "app".to_owned()),
                (other.clone(), "other".to_owned()),
            ],
            home: home.clone(),
        };
        let decided = |chat: i64, project: &Path, question: &str| Decided {
            chat: kept(chat),
            name: format!("kept {chat}"),
            project: Some(project.to_owned()),
            folder: None,
            when: "2026-11-16 09:00".to_owned(),
            header: None,
            question: question.to_owned(),
            choice: "Derive".to_owned(),
            why: Some("One source of truth".to_owned()),
            other: None,
        };
        let decisions = [
            decided(1, &root, "Which way?"),
            decided(8, &other, "Which name?"),
            decided(6, &root, "Which store?"),
        ];
        let now = |key: u64| {
            Some(commands::Now {
                said: Some(format!("step of {key}")),
                running: (key == 3).then(|| "sleep 30 && echo done".to_owned()),
                asks: (key == 1).then(|| "Which way?".to_owned()),
            })
        };
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &decisions, &[], &now, &places);

        assert_eq!(json["waiting"][0]["asks"], "Which way?");
        let working: Vec<(u64, u64)> = json["working"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| {
                (
                    chat["key"].as_u64().unwrap(),
                    chat["depth"].as_u64().unwrap(),
                )
            })
            .collect();
        // A subagent comes under the chat that opened it, and one whose
        // opener is not at work comes on its own.
        assert_eq!(working, [(2, 0), (3, 1), (5, 0)]);
        assert_eq!(json["working"][0]["goal"], "ship it");
        assert_eq!(json["working"][0]["said"], "step of 2");
        assert_eq!(json["working"][1]["running"], "sleep 30 && echo done");
        // A chat at work in the background tells what runs there.
        assert_eq!(
            json["working"][2]["background"],
            json!([{"what": "sleep 120", "command": true}])
        );
        let decided = json["decided"].as_array().unwrap();
        assert_eq!(decided.len(), 2);
        // A chat the window lists is linked, by the name it has there.
        assert_eq!(decided[0]["key"], 1);
        assert_eq!(decided[0]["chat"], "chat 1");
        assert_eq!(decided[0]["why"], "One source of truth");
        assert_eq!(decided[1]["key"], Value::Null);
        assert_eq!(decided[1]["chat"], "kept 6");

        // The home tells of every folder.
        let json = told_of(&told, &home, &decisions, &[], &now, &places);
        assert_eq!(json["working"].as_array().unwrap().len(), 4);
        let questions: Vec<&str> = json["decided"]
            .as_array()
            .unwrap()
            .iter()
            .map(|decided| decided["question"].as_str().unwrap())
            .collect();
        assert_eq!(questions, ["Which way?", "Which name?", "Which store?"]);
        assert_eq!(json["decided"][1]["where"], "other");
    }

    #[test]
    fn a_chat_that_planned_tasks_shows_them_each_with_the_chat_it_was_handed_to() {
        use cox_core::tasks::{Said, TaskId};
        let root = TempDir::new();
        let kept = |number: i64| -> ChatId { serde_json::from_value(json!(number)).unwrap() };
        let listed = |key: u64, working: bool, parent: Option<u64>| {
            let mut listed = chat(key, &root, &root, false, false);
            listed.working = working;
            listed.parent = parent;
            listed.kept = Some(kept(i64::try_from(key).unwrap()));
            listed
        };
        let told = Told {
            chats: vec![
                listed(2, true, None),
                listed(3, true, Some(2)),
                listed(4, true, Some(2)),
                listed(5, false, None),
                listed(6, false, None),
            ],
            projects: Vec::new(),
            home: root.join("home"),
        };
        let id = |id: &str| TaskId::try_from(id.to_owned()).unwrap();
        let tree = |tasks: &[(&str, Option<Status>)]| {
            let mut tree = Tree::default();
            tree.apply(
                tasks
                    .iter()
                    .map(|(task, status)| Said {
                        id: Some(id(task)),
                        title: Some(format!("Do {task}")),
                        status: status.clone(),
                        ..Said::default()
                    })
                    .collect(),
                1,
            )
            .unwrap();
            tree
        };
        let mut handed = tree(&[
            (
                "store",
                Some(Status::Done {
                    result: Some("kept".to_owned()),
                }),
            ),
            ("ui", None),
            (
                "docs",
                Some(Status::Blocked {
                    blocker: "no window".to_owned(),
                }),
            ),
        ]);
        handed.hand(&id("ui"), kept(3), 2).unwrap();
        let trees = [
            (kept(2), handed),
            (kept(5), tree(&[("later", None)])),
            (
                kept(6),
                tree(&[("over", Some(Status::Done { result: None }))]),
            ),
        ];
        let now = |key: u64| {
            Some(commands::Now {
                said: Some(format!("step of {key}")),
                running: None,
                asks: None,
            })
        };
        let places = Places {
            wiki: &root.join("wiki"),
            raw: &root.join("raw"),
            triggers: &json!({}),
        };

        let json = told_of(&told, &root, &[], &trees, &now, &places);

        let rows: Vec<(u64, u64)> = json["working"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| {
                (
                    chat["key"].as_u64().unwrap(),
                    chat["depth"].as_u64().unwrap(),
                )
            })
            .collect();
        // The subagent handed a task shows in it, not again under the
        // chat; a chat whose plan is not done shows though it waits, and
        // one whose plan is done does not.
        assert_eq!(rows, [(2, 0), (4, 1), (5, 0)]);
        let tasks = &json["working"][0]["tasks"];
        assert_eq!(tasks[0]["status"], "done");
        assert_eq!(tasks[0]["note"], "kept");
        assert_eq!(tasks[1]["status"], "in_progress");
        assert_eq!(
            tasks[1]["chat"],
            json!({ "key": 3, "name": "chat 3", "working": true, "said": "step of 3" })
        );
        assert_eq!(tasks[2]["status"], "blocked");
        assert_eq!(tasks[2]["note"], "no window");
        assert_eq!(json["working"][2]["working"], false);
        assert_eq!(json["working"][2]["tasks"][0]["status"], "pending");
    }

    #[test]
    fn what_was_decided_is_read_again_once_a_chat_stops_asking_or_goes() {
        let root = PathBuf::from("/p");
        let told = |chats: Vec<Chat>| Told {
            chats,
            projects: Vec::new(),
            home: PathBuf::from("/home"),
        };
        let asking = told(vec![
            chat(1, &root, &root, true, false),
            chat(2, &root, &root, false, false),
        ]);
        let answered = told(vec![
            chat(1, &root, &root, false, true),
            chat(2, &root, &root, false, false),
        ]);
        let gone = told(vec![chat(1, &root, &root, false, true)]);

        assert!(decided_since(&asking, &answered));
        assert!(!decided_since(&answered, &asking));
        assert!(!decided_since(&answered, &answered));
        assert!(decided_since(&answered, &gone));
    }

    #[test]
    fn what_happened_there_comes_the_last_first() {
        let root = TempDir::new();
        // The human's wiki, apart from the project's own, in `root`.
        let wiki = root.join("human/wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        std::fs::write(
            wiki.join("errors.md"),
            "# Errors\n\nUse anyhow.\n\n## Used in\n\n\
             - 2026-11-05 in app, chat \"one\"\n\
             - 2026-11-06 in other, chat \"two\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("wiki")).unwrap();
        std::fs::write(
            root.join("wiki/people.md"),
            "# People\n\nAsk Ann.\n\n## Used in\n\n- 2026-11-04 in app, chat \"three\"\n",
        )
        .unwrap();
        // The human's sources, apart from the project's own, in `root`.
        let raw = root.join("human/raw");
        let source = coxd::raw::Source {
            from: "https://example.com/",
            title: "News",
            text: "What changed.",
        };
        let kept = |project| coxd::raw::Kept {
            day: "2026-11-07",
            chat: "news",
            chat_id: None,
            project,
        };
        coxd::raw::keep(&raw, "news", source, kept(Some("app"))).unwrap();
        coxd::raw::keep(&raw, "elsewhere", source, kept(Some("other"))).unwrap();
        let schedules = json!({
            "folders": [{"folder": root.path(), "name": null, "triggers": [
                {"name": "triage", "schedule": {"every": "Every 6 hours", "last": "2026-11-07 09:00"}},
                {"name": "never", "schedule": {"every": "Daily 09:00", "last": null}},
                {"name": "ci"},
            ]}],
        });
        let home = root.join("home");
        let told = Told {
            chats: Vec::new(),
            projects: vec![
                (home.clone(), "~".to_owned()),
                (root.to_path_buf(), "app".to_owned()),
            ],
            home: home.clone(),
        };
        let json = told_at(&told, &root, &wiki, &raw, &schedules);
        let kinds: Vec<(&str, &str)> = json["happened"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["kind"].as_str().unwrap(),
                    event["when"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("schedule", "2026-11-07 09:00"),
                ("source", "2026-11-07"),
                ("wiki", "2026-11-05"),
                ("wiki", "2026-11-04"),
            ]
        );
        assert_eq!(json["happened"][2]["href"], "cox://wiki/#errors");
        assert_eq!(json["happened"][3]["href"], "cox://wiki/#project:people");

        // The home tells of what happened in every folder: the wiki's
        // pages written from any, and the sources kept in any, but not
        // the project's own wiki, which only the project shows.
        let schedules = json!({
            "folders": [
                {"folder": home, "name": null, "triggers": []},
                {"folder": root.path(), "name": "app", "triggers": [
                    {"name": "triage", "schedule": {"every": "Every 6 hours", "last": "2026-11-07 09:00"}},
                ]},
            ],
        });
        let json = told_at(&told, &home, &wiki, &raw, &schedules);
        let said: Vec<(&str, &str)> = json["happened"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["kind"].as_str().unwrap(),
                    event["says"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                ("schedule", "ran in app, Every 6 hours"),
                ("source", "kept in elsewhere by news in other"),
                ("source", "kept in news by news in app"),
                ("wiki", "in other, chat \"two\""),
                ("wiki", "in app, chat \"one\""),
            ]
        );
    }
}
