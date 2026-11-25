//! The vocabulary of a map: the one part of it a person keeps, as
//! `maps/<name>.json`, and the agents keep with them. It holds what the
//! code cannot say: the boxes the project is told in, by name, and the
//! people and systems around it; where each box sits, from the top level
//! down as far as a person places them; the lines between boxes that no
//! code backs, as between processes; which code each box owns; and the
//! drift that was talked through and agreed on. Everything below the
//! boxes that own code is generated from the code's index (see
//! [`super::generate`]).
//!
//! A map is told from one viewpoint, as "using it" or "building it": a
//! project with several keeps a file for each.
//!
//! ```json
//! {
//!   "says": "What runs when someone uses the app",
//!   "nodes": {
//!     "app": { "name": "App" },
//!     "user": { "name": "User", "icon": "person", "kind": "outside" },
//!     "server": { "name": "Server", "icon": "server", "claims": ["app/server"] },
//!     "store": { "name": "Store", "icon": "store", "claims": ["app/store"] }
//!   },
//!   "top": { "center": "app", "left": ["user"], "lines": [{ "from": "user", "to": "app" }] },
//!   "levels": {
//!     "app": {
//!       "box": [-360, -260, 800, 520],
//!       "outer": { "user": ["left", 0] },
//!       "parts": { "server": [-300, -100], "store": [100, -100] },
//!       "lines": [{ "from": "user", "to": "server", "both": true }]
//!     }
//!   },
//!   "agreed": [{ "id": "unowned:app/util", "why": "Helpers every box uses." }]
//! }
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// A map's vocabulary, as its file has it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vocabulary {
    /// One line on what the map shows, as against the project's others.
    #[serde(default)]
    pub says: String,
    /// The boxes, by id, and the people and systems around them.
    pub nodes: BTreeMap<String, Node>,
    /// The top level: the project in the middle, and what is around it.
    pub top: Top,
    /// The levels a person placed, by the node each is inside of.
    #[serde(default)]
    pub levels: BTreeMap<String, Level>,
    /// The drift talked through and left as it is, each with why.
    #[serde(default)]
    pub agreed: Vec<Agreed>,
}

/// A box of the map, or a person or a system around the project.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub name: String,
    /// The icon drawn on it, by name, as `person` or `server`.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub kind: Kind,
    /// What it is to the code, drawn under its name, as `cox --daemon`;
    /// its first claim, unless given.
    #[serde(default)]
    pub code: Option<String>,
    /// Whether there are several of it, as one for each chat.
    #[serde(default)]
    pub many: bool,
    /// The modules it owns, as the index names them, as `cox/daemon` or
    /// `httpx._client`: a module belongs to the box with the longest
    /// claim it is in, or is.
    #[serde(default)]
    pub claims: Vec<String>,
    /// The files it owns, from the folder the map's [`super::MAPS_FOLDER`]
    /// is in, the project's or a repository's, or folders of them, for
    /// code whose smallest module holds several files, as a package of
    /// Go does: a file belongs to the box with the longest of these it is
    /// or is in, before any claim of its module.
    #[serde(default)]
    pub files: Vec<String>,
    /// Whether it opens on the types of the code it owns. A box that
    /// stays closed still owns its code, for the lines to it.
    #[serde(default = "opens")]
    pub open: bool,
    /// Where its lines come from, when it owns code.
    #[serde(default)]
    pub lines: Lines,
}

fn opens() -> bool {
    true
}

/// What a node is to the project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A part of the project.
    #[default]
    Part,
    /// A person or a system the project works with.
    Outside,
}

/// Where the lines of a box that owns code come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lines {
    /// From the code: a line to each box its code and that box's meet in.
    #[default]
    Generated,
    /// From the vocabulary, as for the code of a window, which talks to
    /// the rest over a socket rather than in code.
    Hand,
}

/// The top level of the map.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Top {
    /// The node in the middle, the project, which opens on the level
    /// inside it.
    pub center: String,
    /// The nodes in a column on its left, from the top.
    #[serde(default)]
    pub left: Vec<String>,
    /// The nodes in a column on its right, from the top.
    #[serde(default)]
    pub right: Vec<String>,
    #[serde(default)]
    pub lines: Vec<Line>,
}

/// A level inside a node, as a person placed it, in the coordinates of
/// the picture, `y` down.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    /// Where the node opened sits, as its edge: `[x, y, width, height]`.
    #[serde(rename = "box")]
    pub area: [i32; 4],
    /// The nodes of the top level around it, by the side they are on and
    /// how far along it, for a level the top level opens on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outer: BTreeMap<String, Outer>,
    /// The nodes of the levels above around it, by where each sits, for
    /// a level further down.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub at: BTreeMap<String, [i32; 2]>,
    /// The edges of the nodes above that it is inside of, by node, as
    /// `box` is written.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub frames: BTreeMap<String, [i32; 4]>,
    /// The nodes inside it, by where each sits: `[x, y]`.
    pub parts: BTreeMap<String, [i32; 2]>,
    /// The lines between them, and to those around, that no code backs.
    #[serde(default)]
    pub lines: Vec<Line>,
}

/// Where a node of the top level sits around a level: the side of its
/// box, and how far along that side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Outer(pub Side, pub i32);

/// A side of a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// A line between two nodes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub from: String,
    pub to: String,
    /// Whether it goes both ways, as a request and its answer.
    #[serde(default)]
    pub both: bool,
    /// The sides of `from` and `to` it leaves and meets, unless the
    /// nearest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sides: Option<[Side; 2]>,
}

/// A drift talked through and left as it is.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agreed {
    /// The drift's id, as the generated map gives it.
    pub id: String,
    pub why: String,
}

/// Why a vocabulary does not read: each problem, with where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub problems: Vec<String>,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.problems.join("\n"))
    }
}

impl std::error::Error for Error {}

impl Node {
    /// Whether its lines come from the code: it owns code, and does not
    /// keep its lines by hand.
    #[must_use]
    pub fn generates_lines(&self) -> bool {
        (!self.claims.is_empty() || !self.files.is_empty()) && self.lines == Lines::Generated
    }
}

impl Vocabulary {
    /// Reads a vocabulary from its file's text.
    ///
    /// # Errors
    ///
    /// When the text is not one, as JSON, or names a node it does not
    /// have, or two nodes own the same code, or a line is drawn by hand
    /// between two boxes whose lines come from the code: every problem,
    /// each with where it is.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let vocabulary: Self = serde_json::from_str(text).map_err(|error| Error {
            problems: vec![error.to_string()],
        })?;
        let problems = vocabulary.problems();
        if problems.is_empty() {
            Ok(vocabulary)
        } else {
            Err(Error { problems })
        }
    }

    /// What does not hold together in it, each with where.
    fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let known = |id: &str, at: &str, problems: &mut Vec<String>| {
            if !self.nodes.contains_key(id) {
                problems.push(format!("{at}: no node `{id}`"));
            }
        };
        let mut claimed: BTreeMap<(&str, &str), &str> = BTreeMap::new();
        for (id, node) in &self.nodes {
            if id.is_empty() || id.contains(['/', '|']) {
                problems.push(format!(
                    "nodes.{id}: a node's id is not empty and has no `/` or `|`"
                ));
            }
            if node.name.trim().is_empty() {
                problems.push(format!("nodes.{id}: no name"));
            }
            for (field, owned) in [("claims", &node.claims), ("files", &node.files)] {
                if node.kind == Kind::Outside && !owned.is_empty() {
                    problems.push(format!(
                        "nodes.{id}.{field}: a node outside the project owns none of its code"
                    ));
                }
                for item in owned {
                    if item.is_empty() {
                        problems.push(format!("nodes.{id}.{field}: an empty one"));
                    } else if let Some(other) = claimed.insert((field, item), id) {
                        problems.push(format!(
                            "nodes.{id}.{field}: `{item}` is `{other}`'s already"
                        ));
                    }
                }
            }
        }
        known(&self.top.center, "top.center", &mut problems);
        for id in self.top.left.iter().chain(&self.top.right) {
            known(id, "top", &mut problems);
        }
        self.check_lines("top.lines", &self.top.lines, &mut problems);
        let mut inside: BTreeMap<&str, &str> = BTreeMap::new();
        for (opened, level) in &self.levels {
            let at = format!("levels.{opened}");
            known(opened, &at, &mut problems);
            if !level.outer.is_empty() && !level.at.is_empty() {
                problems.push(format!(
                    "{at}: places those around it by `outer` or by `at`, not both"
                ));
            }
            for (field, ids) in [
                ("outer", level.outer.keys().collect::<Vec<_>>()),
                ("at", level.at.keys().collect()),
                ("frames", level.frames.keys().collect()),
                ("parts", level.parts.keys().collect()),
            ] {
                for id in ids {
                    known(id, &format!("{at}.{field}"), &mut problems);
                }
            }
            for part in level.parts.keys() {
                if part == opened {
                    problems.push(format!("{at}.parts: `{part}` is inside itself"));
                } else if let Some(other) = inside.insert(part, opened) {
                    problems.push(format!("{at}.parts: `{part}` is inside `{other}` already"));
                }
            }
            self.check_lines(&format!("{at}.lines"), &level.lines, &mut problems);
        }
        let mut agreed = BTreeSet::new();
        for (at, agreement) in self.agreed.iter().enumerate() {
            if agreement.id.is_empty() {
                problems.push(format!("agreed[{at}]: no id"));
            } else if !agreed.insert(agreement.id.as_str()) {
                problems.push(format!("agreed[{at}]: `{}` again", agreement.id));
            }
        }
        problems
    }

    fn check_lines(&self, at: &str, lines: &[Line], problems: &mut Vec<String>) {
        for (index, line) in lines.iter().enumerate() {
            let mut both_generate = true;
            for end in [&line.from, &line.to] {
                match self.nodes.get(end) {
                    None => problems.push(format!("{at}[{index}]: no node `{end}`")),
                    Some(node) => both_generate &= node.generates_lines(),
                }
            }
            if both_generate {
                problems.push(format!(
                    "{at}[{index}]: the lines between `{}` and `{}` come from the code; \
                     drop this one, or keep one's lines by hand with `\"lines\": \"hand\"`",
                    line.from, line.to
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A_VOCABULARY: &str = r#"{
      "says": "What runs when someone uses it",
      "nodes": {
        "app": { "name": "App" },
        "user": { "name": "User", "icon": "person", "kind": "outside" },
        "server": { "name": "Server", "claims": ["app/server"], "many": true },
        "store": { "name": "Store", "claims": ["app/store"], "files": ["db/"], "open": false },
        "window": { "name": "Window", "claims": ["app/ui"], "lines": "hand" }
      },
      "top": { "center": "app", "left": ["user"], "lines": [{ "from": "user", "to": "app" }] },
      "levels": {
        "app": {
          "box": [-360, -260, 800, 520],
          "outer": { "user": ["left", -20] },
          "parts": { "server": [-300, -100], "store": [100, -100], "window": [0, 100] },
          "lines": [{ "from": "window", "to": "server", "both": true, "sides": ["top", "bottom"] }]
        }
      },
      "agreed": [{ "id": "unowned:app/util", "why": "Every box uses it." }]
    }"#;

    #[test]
    fn a_vocabulary_reads_from_its_file() {
        let vocabulary = Vocabulary::parse(A_VOCABULARY).unwrap();
        assert_eq!(vocabulary.says, "What runs when someone uses it");
        assert_eq!(vocabulary.nodes["user"].kind, Kind::Outside);
        assert_eq!(vocabulary.nodes["app"].kind, Kind::Part);
        assert!(vocabulary.nodes["server"].open);
        assert!(vocabulary.nodes["server"].many);
        assert!(!vocabulary.nodes["store"].open);
        assert!(vocabulary.nodes["server"].generates_lines());
        assert!(!vocabulary.nodes["window"].generates_lines());
        assert!(!vocabulary.nodes["app"].generates_lines());
        assert_eq!(vocabulary.top.center, "app");
        let level = &vocabulary.levels["app"];
        assert_eq!(level.area, [-360, -260, 800, 520]);
        assert_eq!(level.outer["user"], Outer(Side::Left, -20));
        assert_eq!(level.parts["store"], [100, -100]);
        assert_eq!(level.lines[0].sides, Some([Side::Top, Side::Bottom]));
        assert!(level.lines[0].both);
        assert_eq!(vocabulary.agreed[0].id, "unowned:app/util");
    }

    #[test]
    fn a_vocabulary_that_does_not_hold_together_tells_each_problem() {
        let text = A_VOCABULARY
            .replace(
                r#""parts": { "server""#,
                r#""at": { "app": [0, 0] }, "parts": { "nobody": [0, 0], "server""#,
            )
            .replace(r#""claims": ["app/store"]"#, r#""claims": ["app/server"]"#)
            .replace(
                r#""kind": "outside" }"#,
                r#""kind": "outside", "files": ["x"] }"#,
            )
            .replace(
                r#""from": "window", "to": "server""#,
                r#""from": "store", "to": "server""#,
            )
            .replace(r#""center": "app""#, r#""center": "ap""#)
            .replace(
                r#""agreed": ["#,
                r#""agreed": [{ "id": "unowned:app/util", "why": "" }, "#,
            );
        let error = Vocabulary::parse(&text).unwrap_err();
        assert_eq!(
            error.problems,
            [
                "nodes.store.claims: `app/server` is `server`'s already",
                "nodes.user.files: a node outside the project owns none of its code",
                "top.center: no node `ap`",
                "levels.app: places those around it by `outer` or by `at`, not both",
                "levels.app.parts: no node `nobody`",
                "levels.app.lines[0]: the lines between `store` and `server` come from the code; \
                 drop this one, or keep one's lines by hand with `\"lines\": \"hand\"`",
                "agreed[1]: `unowned:app/util` again",
            ]
        );
    }

    #[test]
    fn coxs_own_map_reads() {
        // Cox's map of itself, which its agents keep as they move its code.
        let text = include_str!("../../../../maps/use.json");
        let vocabulary = Vocabulary::parse(text)
            .unwrap_or_else(|error| panic!("maps/use.json does not read: {error}"));
        assert_eq!(vocabulary.top.center, "cox");
    }

    #[test]
    fn a_file_of_another_shape_says_what_it_does_not_know() {
        let error = Vocabulary::parse(r#"{ "tiers": [], "nodes": {} }"#).unwrap_err();
        assert!(error.to_string().starts_with("unknown field `tiers`"));
        let error = Vocabulary::parse(r#"{ "nodes": {}, "top": { "center": "x" }"#).unwrap_err();
        assert!(error.to_string().contains("line 1"));
    }
}
