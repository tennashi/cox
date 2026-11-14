//! What goes on in a folder, as the stage's Activity page shows it, at
//! `cox://api/activity` (see [`crate::stage::plugins`]), in two tiers:
//! what waits for the human, the chats of the folder that ask them
//! something, or whose turn ended while they were not looking; and what
//! happened without them, the schedules that ran there, the pages of the
//! wiki written from there, and the sources kept there, the last first.
//! The home, where Cox lands, tells of every folder.
//!
//! The window tells this module how its chats stand as they change (see
//! [`tell`]), since the stage's pages are served apart from it; the bar
//! counts what waits on the Activity's button (see [`waiting`]).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

/// How many of what happened the page is told of, the last first.
const HAPPENED: usize = 50;

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

/// Keeps how the window's chats stand now, for the pages to read.
pub fn tell(told: Told) {
    if let Ok(mut now) = TOLD.lock() {
        *now = Some(told);
    }
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

/// Whether `chat` waits for the human and is of the folder `root`, or
/// of any, when `root` is the home of `told`.
fn waits_in(told: &Told, chat: &Chat, root: &Path) -> bool {
    (chat.asking || chat.unread) && (root == told.home || chat.cwd == root || chat.project == root)
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
/// now, the human's `wiki`, the folder's own, the sources at `raw`, and
/// `triggers`, as [`coxd::trigger::told`] tells them, of which the
/// schedules ran.
#[must_use]
pub fn told(root: &Path, wiki: &Path, raw: &Path, triggers: &Value) -> Value {
    told_of(&now(), root, wiki, raw, triggers)
}

/// The project of `folder`, by its name, as `told` says it.
fn name_of(told: &Told, folder: &Path) -> Option<String> {
    told.projects
        .iter()
        .find(|(project, _)| project == folder)
        .map(|(_, name)| name.clone())
}

/// The chats of the folder `root` that wait for the human, or of every
/// folder in the home, each with where it is, as JSON.
fn waiting_in(told: &Told, root: &Path) -> Vec<Value> {
    told.chats
        .iter()
        .filter(|chat| waits_in(told, chat, root))
        .map(|chat| {
            json!({
                "key": chat.key,
                "name": chat.name,
                "why": if chat.asking { "asks you" } else { "finished a turn" },
                "goal": chat.goal,
                "sketch": chat.cwd != chat.project,
                "where": if chat.project == told.home {
                    Some("~".to_owned())
                } else {
                    name_of(told, &chat.project)
                },
            })
        })
        .collect()
}

/// What goes on in the folder `root`, as [`told`] tells it, the chats
/// standing as `told` says.
fn told_of(told: &Told, root: &Path, wiki: &Path, raw: &Path, triggers: &Value) -> Value {
    let home = root == told.home;
    let waiting = waiting_in(told, root);
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
                    source["chat"].as_str().map(|chat| format!(" by {chat}")).unwrap_or_default(),
                    project
                        .filter(|_| home)
                        .map(|project| format!(" in {project}"))
                        .unwrap_or_default()
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
        }
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
        let json = told_of(
            &told,
            &root,
            &root.join("wiki"),
            &root.join("raw"),
            &json!({}),
        );
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
        let json = told_of(
            &told,
            &home,
            &home.join("wiki"),
            &home.join("raw"),
            &json!({}),
        );
        let keys: Vec<_> = json["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| chat["key"].as_u64().unwrap())
            .collect();
        assert_eq!(keys, [1, 3, 4]);
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
        let json = told_of(&told, &root, &wiki, &raw, &schedules);
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
        let json = told_of(&told, &home, &wiki, &raw, &schedules);
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
