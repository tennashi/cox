//! The map of a project: a picture of it in tiers, for the human to see
//! where things are before they read them. Not what the files say, which
//! a tree of them shows, but what the parts of the project mean, which
//! an agent writes: a tier is rows of boxes; a box opens a tier of its
//! own, down to boxes that hold paths of the code, where the human reads
//! on; and boxes of other tiers can stay around a tier that is open,
//! above and below it. What the tiers are, and what goes in each, is
//! the project's to decide, in its chats; this module only reads the
//! file. The map is a file of the project, [`MAP_FILE`], so that it is
//! there whenever it is opened, written by an agent once and kept by the
//! agents that change the code, in the change that moves it.
//!
//! Over it goes what the project is now: what changed, as
//! [`crate::changes`] tells it, falls on the boxes that hold the files,
//! and up on the boxes that open their tiers, and a box whose paths are
//! gone is stale, for an agent to bring the map up to date (see
//! [`all_facts`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a project keeps its map, under its folder.
pub const MAP_FILE: &str = ".cox/map.json";

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
    /// The orders in which the boxes talk.
    #[serde(default)]
    pub flows: Vec<Flow>,
}

/// An order in which boxes talk: steps from one box to another, written
/// between the boxes that hold the code, which the tab rolls up to the
/// boxes of the tier it shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub id: String,
    pub name: String,
    /// One line on what the flow is.
    #[serde(default)]
    pub says: String,
    /// The steps, in order.
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// A step of a flow: what passes from one box to another.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub says: String,
    /// Where the step is made, a path from the project's folder, with a
    /// line as `path:12`.
    #[serde(default)]
    pub at: Option<String>,
    /// Whether the step is one back: the answer to an earlier step, as
    /// a reply is to a request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub back: bool,
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
    /// The boxes of the tiers above that stay around this one.
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

/// The boxes around an open tier: the user's side above it, and the
/// code's side below. Maps written when the frame stood beside the tier
/// name them `left` and `right`, which still read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    #[serde(default, alias = "left")]
    pub above: Vec<String>,
    #[serde(default, alias = "right")]
    pub below: Vec<String>,
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

/// What changed in a box: in how many of the files it holds, and those
/// the boxes of its tier hold, all the way down, and how many lines they
/// gained and lost.
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

/// What Cox knows of each box of `map`, by id, for the project in
/// `root`, among `changed`, the lines each file that changed gained and
/// lost, by its absolute path (see [`crate::changes::changed_files`]):
/// what a box holds, and what changed in it, counting the files its own
/// holds have and those the boxes of the tier it opens have, all the way
/// down, each once.
#[must_use]
pub fn all_facts<S: std::hash::BuildHasher>(
    root: &Path,
    map: &Map,
    changed: &HashMap<PathBuf, (usize, usize), S>,
) -> BTreeMap<String, Facts> {
    let own: BTreeMap<&str, (Facts, HashSet<PathBuf>)> = map
        .parts
        .iter()
        .map(|(id, part)| (id.as_str(), own_facts(root, part, changed)))
        .collect();
    let mut rolled: HashMap<&str, HashSet<PathBuf>> = HashMap::new();
    for id in map.parts.keys() {
        roll(id, map, &own, &mut rolled, &mut Vec::new());
    }
    own.into_iter()
        .map(|(id, (mut facts, _))| {
            let files = rolled.remove(id).unwrap_or_default();
            facts.changed = counted(&files, changed);
            (id.to_owned(), facts)
        })
        .collect()
}

/// The files that changed in the box `id`, and in the boxes of the tier
/// it opens, all the way down, kept in `rolled` once found; `path` is the
/// boxes above on the way, which a box in its own tier is not counted
/// into again.
fn roll<'a>(
    id: &'a str,
    map: &'a Map,
    own: &BTreeMap<&str, (Facts, HashSet<PathBuf>)>,
    rolled: &mut HashMap<&'a str, HashSet<PathBuf>>,
    path: &mut Vec<&'a str>,
) -> HashSet<PathBuf> {
    if let Some(files) = rolled.get(id) {
        return files.clone();
    }
    if path.contains(&id) {
        return HashSet::new();
    }
    let mut files: HashSet<PathBuf> = own
        .get(id)
        .map(|(_, files)| files.clone())
        .unwrap_or_default();
    let inside: Vec<&'a str> = map
        .parts
        .get(id)
        .and_then(|part| part.tier.as_deref())
        .and_then(|tier| map.tier(tier))
        .map(|tier| tier.parts().collect())
        .unwrap_or_default();
    if !inside.is_empty() {
        path.push(id);
        for within in inside {
            files.extend(roll(within, map, own, rolled, path));
        }
        path.pop();
    }
    rolled.insert(id, files.clone());
    files
}

/// What Cox knows of the box `part` of the project in `root`, among
/// `changed`, as [`all_facts`] tells it, but of the box's own holds
/// alone, with nothing of its tier.
#[must_use]
pub fn facts<S: std::hash::BuildHasher>(
    root: &Path,
    part: &Part,
    changed: &HashMap<PathBuf, (usize, usize), S>,
) -> Facts {
    own_facts(root, part, changed).0
}

/// What Cox knows of the box `part` from its own holds, and the files
/// among `changed` that they have, each once.
fn own_facts<S: std::hash::BuildHasher>(
    root: &Path,
    part: &Part,
    changed: &HashMap<PathBuf, (usize, usize), S>,
) -> (Facts, HashSet<PathBuf>) {
    let mut facts = Facts::default();
    let mut files: HashSet<PathBuf> = HashSet::new();
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
                files.insert(file.clone());
                added += more;
                removed += less;
            }
        }
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
    facts.changed = counted(&files, changed);
    (facts, files)
}

/// What changed in `files`, among `changed`.
fn counted<S: std::hash::BuildHasher>(
    files: &HashSet<PathBuf>,
    changed: &HashMap<PathBuf, (usize, usize), S>,
) -> Changed {
    let lines = files.iter().filter_map(|file| changed.get(file));
    Changed {
        files: files.len(),
        added: lines.clone().map(|(added, _)| added).sum(),
        removed: lines.map(|(_, removed)| removed).sum(),
    }
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
          "frame": { "above": ["user"], "below": [] } }
      ],
      "parts": {
        "user": { "name": "User", "says": "Who runs it." },
        "app": { "name": "App", "says": "The program.", "tier": "app", "holds": ["src"] },
        "main": { "name": "main.rs", "says": "Where it starts.", "holds": ["src/main.rs:2"] },
        "gone": { "name": "Gone", "says": "Was here.", "holds": ["src/gone.rs"] }
      },
      "edges": [{ "from": "user", "to": "app", "says": "runs", "dir": "both" }],
      "flows": [{ "id": "run", "name": "A run", "says": "From the user to main.",
        "steps": [{ "from": "user", "to": "main", "says": "starts", "at": "src/main.rs:2" },
                  { "from": "main", "to": "user", "says": "greets", "back": true }] }]
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
        assert_eq!(map.tier("app").unwrap().frame.above, ["user"]);
        assert_eq!(map.edges[0].dir.as_deref(), Some("both"));
        assert_eq!(map.flows[0].steps[0].to, "main");
        assert_eq!(map.flows[0].steps[0].at.as_deref(), Some("src/main.rs:2"));
        assert!(!map.flows[0].steps[0].back);
        assert!(map.flows[0].steps[1].back);

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
    fn a_frame_written_as_left_and_right_still_reads() {
        let frame: Frame =
            serde_json::from_str(r#"{ "left": ["user"], "right": ["os"] }"#).unwrap();
        assert_eq!(frame.above, ["user"]);
        assert_eq!(frame.below, ["os"]);
        assert_eq!(
            serde_json::to_string(&frame).unwrap(),
            r#"{"above":["user"],"below":["os"]}"#
        );
    }

    #[test]
    fn a_line_is_split_from_a_path() {
        assert_eq!(split_line("src/a.rs:12"), ("src/a.rs", Some(12)));
        assert_eq!(split_line("src/a.rs"), ("src/a.rs", None));
        assert_eq!(split_line("src/a:b.rs"), ("src/a:b.rs", None));
    }
    #[test]
    fn what_changed_rolls_up_to_the_box_that_opens_the_tier() {
        let root = scratch("rollup");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "\n").unwrap();
        let map: Map = serde_json::from_str(
            r#"{
              "tiers": [
                { "id": "whole", "name": "Whole", "rows": [{ "parts": ["app", "folder", "loop"] }] },
                { "id": "app", "name": "App", "rows": [{ "parts": ["main", "lib"] }] },
                { "id": "loop", "name": "Loop", "rows": [{ "parts": ["loop", "main"] }] }
              ],
              "parts": {
                "app": { "name": "App", "tier": "app" },
                "folder": { "name": "Folder", "tier": "app", "holds": ["src"] },
                "loop": { "name": "Loop", "tier": "loop" },
                "main": { "name": "main", "holds": ["src/main.rs"] },
                "lib": { "name": "lib", "holds": ["src/lib.rs", "src/lib.rs:1"] }
              }
            }"#,
        )
        .unwrap();
        let mut changed = HashMap::new();
        changed.insert(root.join("src/main.rs"), (3, 1));
        changed.insert(root.join("src/lib.rs"), (2, 0));
        let all = all_facts(&root, &map, &changed);
        // A box with no holds of its own lights from its tier.
        assert_eq!(
            all["app"].changed,
            Changed {
                files: 2,
                added: 5,
                removed: 1
            }
        );
        assert_eq!(all["app"].holds, Vec::<Hold>::new());
        // A file is counted once, under a folder and in the tier alike,
        // and under two holds of one box.
        assert_eq!(all["folder"].changed.files, 2);
        assert_eq!(
            all["lib"].changed,
            Changed {
                files: 1,
                added: 2,
                removed: 0
            }
        );
        assert_eq!(all["lib"].holds[1].added, 2);
        // A box in its own tier does not go round for ever.
        assert_eq!(all["loop"].changed.files, 1);
        assert_eq!(facts(&root, &map.parts["app"], &changed).changed.files, 0);
    }
}
