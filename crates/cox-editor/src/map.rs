//! The map of a project: a picture of it in tiers, for the human to see
//! where things are before they read them. Not what the files say, which
//! a tree of them shows, but what the parts of the project mean, which
//! an agent writes: the top tier is the few things the project is among,
//! such as its user, the program, and what the program talks to; a box of
//! a tier opens a tier of its own, down to boxes that hold paths of the
//! code, where the human reads on. The map is a file of the project,
//! [`MAP_FILE`], so that it is there whenever it is opened, written by an
//! agent once and kept by the agents that change the code, in the change
//! that moves it.
//!
//! Over it goes what the project is now: what changed, as
//! [`crate::changes`] tells it, falls on the boxes that hold the files,
//! and a box whose paths are gone is stale, for an agent to bring the
//! map up to date (see [`facts`]).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a project keeps its map, under its folder.
pub const MAP_FILE: &str = ".cox/map.json";

/// How the map's file is written, as the agent is told it when it is to
/// write one, or to keep one.
pub const HOW_WRITTEN: &str = "\
The map is `.cox/map.json` in the project's folder, a JSON object with `tiers`, `parts` and \
`edges`. `tiers` is a list, the top tier first, each `{\"id\", \"name\", \"says\", \"rows\", \
\"frame\"}`: `says` is one line on what the tier shows; `rows` are the lines of boxes, each \
`{\"name\", \"parts\": [ids], \"under\": id}`, where `under` hangs the row under a box of a row \
above, as what that box holds; `frame` is `{\"left\": [ids], \"right\": [ids]}`, boxes of the \
tiers above that stay beside the tier when it is open, for the lines out of it to reach, the \
user's side left and the code's side right. `parts` is an object of the boxes by id, each \
`{\"name\", \"says\", \"each\", \"many\", \"holds\", \"tier\", \"kinds\", \"note\"}`: `says` is \
one line for a reader who does not know the project; `each` says how many there are and of what \
(\"one per chat\"), with `many: true` when there are several; `holds` are the paths of the \
code it is, from the project's folder, a file or a folder, a file with a line as `path:12`; \
`tier` is the id of the tier that opens inside it; `kinds` names the kinds of a box that is \
not one thing; `note` is a remark, such as where the code does not match the map. `edges` is a \
list of `{\"from\", \"to\", \"says\", \"dir\"}` between boxes, by id, `dir` being `to` (an \
arrow into `to`, the default), `both` or `none`; a tier draws the edges among its boxes and \
those to the boxes in its frame. Keep each tier small, five or six boxes, and every box \
either opening a tier or holding paths, so that the human can go from the top down to the \
code. Boxes may be in more than one tier. Write the map once, in the project's language, and \
change it as you move the code it holds.";

/// A project's map, as its file has it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Map {
    /// The tiers, the top one first.
    #[serde(default)]
    pub tiers: Vec<Tier>,
    /// The boxes, by id.
    #[serde(default)]
    pub parts: BTreeMap<String, Part>,
    /// The lines between boxes.
    #[serde(default)]
    pub edges: Vec<Edge>,
}

/// One tier of the map: a picture at one depth.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    pub id: String,
    pub name: String,
    /// One line on what the tier shows.
    #[serde(default)]
    pub says: String,
    /// The lines of boxes, from the top.
    #[serde(default)]
    pub rows: Vec<Row>,
    /// The boxes of the tiers above that stay beside this one.
    #[serde(default)]
    pub frame: Frame,
}

/// A line of boxes in a tier.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// What the line is, if it says.
    #[serde(default)]
    pub name: Option<String>,
    /// The box of a row above that this row hangs under, as what it holds.
    #[serde(default)]
    pub under: Option<String>,
    /// The boxes, by id, left to right.
    #[serde(default)]
    pub parts: Vec<String>,
}

/// The boxes beside an open tier.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    #[serde(default)]
    pub left: Vec<String>,
    #[serde(default)]
    pub right: Vec<String>,
}

/// A box of the map.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    /// One line on what it is, for a reader who does not know the project.
    #[serde(default)]
    pub says: String,
    /// How many there are, and of what: "one per chat".
    #[serde(default)]
    pub each: Option<String>,
    /// Whether there are several.
    #[serde(default)]
    pub many: bool,
    /// The paths of the code it is, from the project's folder: a file, a
    /// folder, or a file with a line, as `path:12`.
    #[serde(default)]
    pub holds: Vec<String>,
    /// The tier that opens inside it, by id.
    #[serde(default)]
    pub tier: Option<String>,
    /// The kinds of a box that is not one thing.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// A remark, such as where the code does not match the map.
    #[serde(default)]
    pub note: Option<String>,
}

/// A line between two boxes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub says: String,
    /// `to`, an arrow into `to`, unless given; `both`; or `none`.
    #[serde(default)]
    pub dir: Option<String>,
}

impl Map {
    /// The tier `id`, if the map has one.
    #[must_use]
    pub fn tier(&self, id: &str) -> Option<&Tier> {
        self.tiers.iter().find(|tier| tier.id == id)
    }

    /// The tier a box opens, if it opens one the map has.
    #[must_use]
    pub fn tier_of(&self, part: &str) -> Option<&Tier> {
        self.parts
            .get(part)
            .and_then(|part| part.tier.as_deref())
            .and_then(|id| self.tier(id))
    }
}

impl Tier {
    /// The boxes in the tier, by id, in the order of its rows.
    pub fn parts(&self) -> impl Iterator<Item = &str> {
        self.rows
            .iter()
            .flat_map(|row| row.parts.iter().map(String::as_str))
    }
}

/// The map of the project in `root`, if it has one.
///
/// # Errors
///
/// When the file cannot be read, or is not a map.
pub fn read(root: &Path) -> Result<Option<Map>, String> {
    let path = root.join(MAP_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {MAP_FILE}: {error}")),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| format!("{MAP_FILE} is not a map: {error}"))
}

/// A path a box holds, as the project has it now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Hold {
    /// As the map writes it.
    pub said: String,
    /// Where it is, absolute.
    pub path: PathBuf,
    /// The line, when the map names one.
    pub line: Option<usize>,
    /// Whether the project has it.
    pub exists: bool,
    /// How many lines a file has; none for a folder.
    pub lines: Option<usize>,
    /// How many lines changed in it, as the Changes tab shows: in the
    /// files of a folder together.
    pub added: usize,
    pub removed: usize,
}

/// What changed in a box: in how many of the files it holds, and how many
/// lines they gained and lost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Changed {
    pub files: usize,
    pub added: usize,
    pub removed: usize,
}

impl Changed {
    /// Whether anything changed.
    #[must_use]
    pub fn any(self) -> bool {
        self.files > 0
    }
}

/// What Cox knows of a box, over what the map says of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Facts {
    pub holds: Vec<Hold>,
    pub changed: Changed,
    /// The paths it holds that the project no longer has, as the map
    /// writes them.
    pub stale: Vec<String>,
}

/// What Cox knows of the box `part` of the project in `root`, among
/// `changed`, the lines each file that changed gained and lost, by its
/// absolute path (see [`crate::changes::changed_files`]).
#[must_use]
pub fn facts<S: std::hash::BuildHasher>(
    root: &Path,
    part: &Part,
    changed: &HashMap<PathBuf, (usize, usize), S>,
) -> Facts {
    let mut facts = Facts::default();
    for said in &part.holds {
        let (relative, line) = split_line(said);
        let path = root.join(relative);
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let exists = path.exists();
        let lines = path
            .is_file()
            .then(|| std::fs::read_to_string(&path).ok())
            .flatten()
            .map(|text| text.lines().count());
        let (mut added, mut removed) = (0, 0);
        for (file, (more, less)) in changed {
            if *file == path || (path.is_dir() && file.starts_with(&path)) {
                facts.changed.files += 1;
                added += more;
                removed += less;
            }
        }
        facts.changed.added += added;
        facts.changed.removed += removed;
        if !exists {
            facts.stale.push(said.clone());
        }
        facts.holds.push(Hold {
            said: said.clone(),
            path,
            line,
            exists,
            lines,
            added,
            removed,
        });
    }
    facts
}

/// A path as the map writes it, and the line after it, if any: `a.rs:12`
/// is `a.rs` at line 12.
fn split_line(said: &str) -> (&str, Option<usize>) {
    match said.rsplit_once(':') {
        Some((path, line)) if !path.is_empty() => match line.parse() {
            Ok(line) => (path, Some(line)),
            Err(_) => (said, None),
        },
        _ => (said, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("cox-map-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::canonicalize(directory).unwrap()
    }

    const A_MAP: &str = r#"{
      "tiers": [
        { "id": "whole", "name": "Whole", "says": "The few things.", "rows": [{ "parts": ["user", "app"] }] },
        { "id": "app", "name": "The app", "rows": [{ "name": "inside", "parts": ["main", "gone"] }],
          "frame": { "left": ["user"], "right": [] } }
      ],
      "parts": {
        "user": { "name": "User", "says": "Who runs it." },
        "app": { "name": "App", "says": "The program.", "tier": "app", "holds": ["src"] },
        "main": { "name": "main.rs", "says": "Where it starts.", "holds": ["src/main.rs:2"] },
        "gone": { "name": "Gone", "says": "Was here.", "holds": ["src/gone.rs"] }
      },
      "edges": [{ "from": "user", "to": "app", "says": "runs", "dir": "both" }]
    }"#;

    #[test]
    fn the_map_is_read_from_the_projects_file() {
        let root = scratch("read");
        assert_eq!(read(&root), Ok(None));
        std::fs::create_dir_all(root.join(".cox")).unwrap();
        std::fs::write(root.join(MAP_FILE), A_MAP).unwrap();

        let map = read(&root).unwrap().unwrap();
        assert_eq!(map.tiers.len(), 2);
        assert_eq!(map.tier("app").unwrap().name, "The app");
        assert_eq!(map.tier_of("app").unwrap().id, "app");
        assert_eq!(map.tier_of("user"), None);
        assert_eq!(
            map.tier("app").unwrap().parts().collect::<Vec<_>>(),
            ["main", "gone"]
        );
        assert_eq!(map.parts["main"].holds, ["src/main.rs:2"]);
        assert_eq!(map.edges[0].dir.as_deref(), Some("both"));

        std::fs::write(root.join(MAP_FILE), "{ not json").unwrap();
        assert!(
            read(&root)
                .unwrap_err()
                .starts_with(".cox/map.json is not a map")
        );
    }

    #[test]
    fn the_facts_of_a_box_are_what_it_holds_as_the_project_has_it() {
        let root = scratch("facts");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {\n}\n").unwrap();
        let changed = HashMap::from([(root.join("src/main.rs"), (3, 1))]);

        let part = Part {
            holds: Vec::from(["src/main.rs:2".to_owned(), "src/gone.rs".to_owned()]),
            ..Part::default()
        };
        let facts = facts(&root, &part, &changed);
        assert_eq!(facts.holds[0].path, root.join("src/main.rs"));
        assert_eq!(facts.holds[0].line, Some(2));
        assert_eq!(facts.holds[0].lines, Some(2));
        assert!(facts.holds[0].exists);
        assert_eq!((facts.holds[0].added, facts.holds[0].removed), (3, 1));
        assert!(!facts.holds[1].exists);
        assert_eq!(facts.holds[1].line, None);
        assert_eq!(facts.stale, ["src/gone.rs"]);
        assert_eq!(
            facts.changed,
            Changed {
                files: 1,
                added: 3,
                removed: 1
            }
        );

        let folder = Part {
            holds: Vec::from(["src".to_owned()]),
            ..Part::default()
        };
        let facts = self::facts(&root, &folder, &changed);
        assert_eq!(facts.holds[0].lines, None);
        assert_eq!(facts.changed.files, 1);
        assert_eq!(facts.changed.added, 3);
    }

    #[test]
    fn a_line_is_split_from_a_path() {
        assert_eq!(split_line("src/a.rs:12"), ("src/a.rs", Some(12)));
        assert_eq!(split_line("src/a.rs"), ("src/a.rs", None));
        assert_eq!(split_line("src/a:b.rs"), ("src/a:b.rs", None));
    }
}
