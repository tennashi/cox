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
//! A box too big to read on one level opens on a level of boxes of its
//! own, which a person places, as the project does, and owns no code
//! itself: the boxes inside it own it. Where it is drawn closed, it
//! stands for them, its lines for theirs.
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
//!       "lines": [{ "from": "user", "to": { "box": "server", "implements": "Handler" } }]
//!     }
//!   },
//!   "agreed": [{ "id": "unowned:app/util", "why": "Helpers every box uses." }]
//! }
//! ```
//!
//! A line's end at a box may be a rule over the code, which picks the
//! types inside the box the line goes on to as the box opens: those that
//! implement a trait or an interface, or subclass a class, or those whose
//! code uses a name. The names are the code's, and the framework's,
//! which the person who knows it writes: no type id kept by hand, to go
//! stale as the code moves.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, Deserializer};
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
    /// Where its lines come from, when it, or a box inside it, owns code.
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

/// Where the lines of a box that owns code, or holds boxes that do, come
/// from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lines {
    /// From the code: a line to each box its code and that box's meet in.
    #[default]
    Generated,
    /// From the vocabulary, as for the code of a window, which talks to
    /// the rest over a socket rather than in code. The lines of the boxes
    /// inside it to those outside it are then a person's too, where it is
    /// drawn closed; inside it, they come from the code.
    Hand,
}

/// The top level of the map.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    /// Where the node opened sits, as its edge: `[x, y, width, height]`.
    #[serde(rename = "box")]
    pub area: [i32; 4],
    /// The nodes of the top level around it, by the side they are on and
    /// how far along it, for a level the top level opens on.
    #[serde(default)]
    pub outer: BTreeMap<String, Outer>,
    /// The nodes of the levels above around it, by where each sits, for
    /// a level further down: those the code's lines go on to. With none,
    /// here or in `outer`, the map places around the box every box above
    /// that the code of its parts meets; and it places each end of the
    /// level's lines that is placed nowhere in it.
    #[serde(default)]
    pub at: BTreeMap<String, [i32; 2]>,
    /// The edges of the nodes above that it is inside of, by node, as
    /// `box` is written.
    #[serde(default)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// A line between two nodes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "LineText")]
pub struct Line {
    pub from: String,
    pub to: String,
    /// Whether it goes both ways, as a request and its answer.
    pub both: bool,
    /// The sides of `from` and `to` it leaves and meets, unless the
    /// nearest.
    pub sides: Option<[Side; 2]>,
    /// The rules its ends at `from` and at `to` are, if they are: what
    /// it goes on to inside the box there as the box opens.
    pub rules: [Option<Rule>; 2],
}

/// Of the types inside a box, those a line to the box goes on to as it
/// opens, by a rule over the facts every index of the code has, the same
/// for every language: what a language server tells as a type's
/// implementations, and as a name's references.
///
/// The name is the code's: a type's, or a function's, as it is declared,
/// and as much of the module it is in, before it, as tells it from
/// others of its name, as `Render`, `tui/Renderer`, `hono-base.ts/Hono`
/// or `httpx._transports.base/BaseTransport`, whichever of `/`, `.` and
/// `:` parts them. rust-analyzer names the trait an `impl` block is of
/// by its name alone, which the name's last part is matched against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// The types that implement the trait or the interface, or subclass
    /// the class, of the name: its implementations.
    Implements(String),
    /// The types whose code, in their declarations or their functions,
    /// names it: its references.
    Uses(String),
}

impl Rule {
    /// The name it picks by.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Rule::Implements(name) | Rule::Uses(name) => name,
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rule::Implements(name) => write!(f, "implements {name}"),
            Rule::Uses(name) => write!(f, "uses {name}"),
        }
    }
}

/// A line, as its file has it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineText {
    from: EndText,
    to: EndText,
    #[serde(default)]
    both: bool,
    #[serde(default)]
    sides: Option<[Side; 2]>,
}

/// A line's end, as its file has it: a node's id, or a rule over the
/// types inside a box, `{"box", "implements"}` or `{"box", "uses"}`.
enum EndText {
    Node(String),
    Rule(RuleText),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleText {
    #[serde(rename = "box")]
    node: String,
    #[serde(default)]
    implements: Option<String>,
    #[serde(default)]
    uses: Option<String>,
}

impl<'de> Deserialize<'de> for EndText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = EndText;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a node's id, or a rule over the types inside a box, \
                     as `{\"box\": \"app\", \"implements\": \"Handler\"}`",
                )
            }

            fn visit_str<E: de::Error>(self, id: &str) -> Result<EndText, E> {
                Ok(EndText::Node(id.to_owned()))
            }

            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<EndText, A::Error> {
                RuleText::deserialize(de::value::MapAccessDeserializer::new(map)).map(EndText::Rule)
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

impl TryFrom<LineText> for Line {
    type Error = String;

    fn try_from(text: LineText) -> Result<Self, String> {
        let mut ends = [String::new(), String::new()];
        let mut rules = [None, None];
        for (at, end) in [text.from, text.to].into_iter().enumerate() {
            match end {
                EndText::Node(id) => ends[at] = id,
                EndText::Rule(rule) => {
                    let names = |name: &str| {
                        name.split(['/', '.', ':'])
                            .any(|part| !part.trim().is_empty())
                    };
                    rules[at] = Some(match (rule.implements, rule.uses) {
                        (Some(name), None) if names(&name) => Rule::Implements(name),
                        (None, Some(name)) if names(&name) => Rule::Uses(name),
                        (None, None) | (Some(_), Some(_)) => {
                            return Err(format!(
                                "the end at `{}` is a rule of one kind: \
                                 `implements` or `uses`",
                                rule.node
                            ));
                        }
                        _ => return Err(format!("the end at `{}` names nothing", rule.node)),
                    });
                    ends[at] = rule.node;
                }
            }
        }
        let [from, to] = ends;
        Ok(Line {
            from,
            to,
            both: text.both,
            sides: text.sides,
            rules,
        })
    }
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
    /// Whether it owns code: claims a module or a file.
    #[must_use]
    pub fn owns_code(&self) -> bool {
        !self.claims.is_empty() || !self.files.is_empty()
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

    /// The node each node a level places is inside of: the node the level
    /// is of.
    #[must_use]
    pub fn inside_of(&self) -> BTreeMap<&str, &str> {
        self.levels
            .iter()
            .flat_map(|(opened, level)| {
                level
                    .parts
                    .keys()
                    .map(move |part| (part.as_str(), opened.as_str()))
            })
            .collect()
    }

    /// Whether the lines of the node `id` come from the code: it does not
    /// keep its lines by hand, and owns code, or a box inside it does
    /// whose lines come from the code, for which it stands in the levels
    /// above, where it is drawn closed. Not the top's center, whose lines
    /// to what is around the project a person draws.
    #[must_use]
    pub fn generates(&self, id: &str) -> bool {
        id != self.top.center && self.generates_within(id, &mut BTreeSet::new())
    }

    fn generates_within<'v>(&'v self, id: &'v str, seen: &mut BTreeSet<&'v str>) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        if node.lines == Lines::Hand || !seen.insert(id) {
            return false;
        }
        node.owns_code()
            || self.levels.get(id).is_some_and(|level| {
                level
                    .parts
                    .keys()
                    .any(|part| self.generates_within(part, seen))
            })
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
            if self.nodes.get(opened).is_some_and(Node::owns_code) {
                problems.push(format!(
                    "{at}: `{opened}` owns code, so it opens on its types; \
                     give its code to the boxes inside it"
                ));
            }
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
        for &opened in inside.values().collect::<BTreeSet<_>>() {
            let mut at = opened;
            for _ in 0..inside.len() {
                match inside.get(at) {
                    Some(&outer) if outer == opened => {
                        problems.push(format!("levels.{opened}: `{opened}` is inside itself"));
                        break;
                    }
                    Some(&outer) => at = outer,
                    None => break,
                }
            }
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

    /// Whether the node `id` opens on types, or holds boxes that do, for
    /// a rule to pick of: a part that owns code and does not stay
    /// closed, or whose level places one.
    #[must_use]
    pub fn shows_types(&self, id: &str) -> bool {
        self.shows_types_within(id, &mut BTreeSet::new())
    }

    fn shows_types_within<'v>(&'v self, id: &'v str, seen: &mut BTreeSet<&'v str>) -> bool {
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        if node.kind == Kind::Outside || !seen.insert(id) {
            return false;
        }
        (node.owns_code() && node.open)
            || self.levels.get(id).is_some_and(|level| {
                level
                    .parts
                    .keys()
                    .any(|part| self.shows_types_within(part, seen))
            })
    }

    fn check_lines(&self, at: &str, lines: &[Line], problems: &mut Vec<String>) {
        for (index, line) in lines.iter().enumerate() {
            let mut both_generate = true;
            for (end, rule, field) in [
                (&line.from, &line.rules[0], "from"),
                (&line.to, &line.rules[1], "to"),
            ] {
                if !self.nodes.contains_key(end) {
                    problems.push(format!("{at}[{index}]: no node `{end}`"));
                    continue;
                }
                both_generate &= self.generates(end);
                if let Some(rule) = rule
                    && !self.shows_types(end)
                {
                    problems.push(format!(
                        "{at}[{index}].{field}: `{rule}` picks of the types inside `{end}`, \
                         which shows none: it owns no code, stays closed, or is outside"
                    ));
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
        assert!(vocabulary.generates("server"));
        assert!(!vocabulary.generates("window"));
        assert!(!vocabulary.generates("app"));
        assert!(!vocabulary.generates("user"));
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
                // With no center, `app` is a box whose level holds code.
                "top.lines[0]: the lines between `user` and `app` come from the code; \
                 drop this one, or keep one's lines by hand with `\"lines\": \"hand\"`",
                "levels.app: places those around it by `outer` or by `at`, not both",
                "levels.app.parts: no node `nobody`",
                "levels.app.lines[0]: the lines between `store` and `server` come from the code; \
                 drop this one, or keep one's lines by hand with `\"lines\": \"hand\"`",
                "agreed[1]: `unowned:app/util` again",
            ]
        );
    }

    /// A vocabulary whose box `group` opens on a level of its own, of
    /// `inner`, which owns code; `group` keeps its lines as `lines` says.
    fn nested(lines: &str) -> String {
        format!(
            r#"{{
              "nodes": {{
                "app": {{ "name": "App" }},
                "server": {{ "name": "Server", "claims": ["app/server"] }},
                "group": {{ "name": "Group", "lines": "{lines}" }},
                "inner": {{ "name": "Inner", "claims": ["app/inner"] }}
              }},
              "top": {{ "center": "app" }},
              "levels": {{
                "app": {{
                  "box": [0, 0, 800, 500],
                  "parts": {{ "server": [0, 0], "group": [300, 0] }},
                  "lines": [{{ "from": "server", "to": "group" }}]
                }},
                "group": {{
                  "box": [0, 0, 800, 500],
                  "at": {{ "server": [0, 0] }},
                  "parts": {{ "inner": [300, 0] }}
                }}
              }}
            }}"#
        )
    }

    #[test]
    fn a_box_that_opens_on_a_level_of_its_own_stands_for_the_boxes_inside_it() {
        let vocabulary = Vocabulary::parse(&nested("hand")).unwrap();
        assert_eq!(
            vocabulary.inside_of(),
            BTreeMap::from([("group", "app"), ("inner", "group"), ("server", "app")])
        );
        // Kept by hand, its lines are a person's, as to the server.
        assert!(!vocabulary.generates("group"));
        assert!(vocabulary.generates("inner"));
        // Otherwise they are the code's, of the box inside it.
        let error = Vocabulary::parse(&nested("generated")).unwrap_err();
        assert_eq!(
            error.problems,
            [
                "levels.app.lines[0]: the lines between `server` and `group` come from the code; \
              drop this one, or keep one's lines by hand with `\"lines\": \"hand\"`"
            ]
        );
        // A box that owns code opens on its types, not on a level of boxes.
        let text = nested("hand").replace(
            r#""lines": "hand" }"#,
            r#""lines": "hand", "claims": ["app/group"] }"#,
        );
        assert_eq!(
            Vocabulary::parse(&text).unwrap_err().problems,
            [
                "levels.group: `group` owns code, so it opens on its types; \
              give its code to the boxes inside it"
            ]
        );
        // Nor is a box inside itself, through another.
        let text = r#"{ "nodes": { "app": { "name": "App" }, "a": { "name": "A" }, "b": { "name": "B" } },
                        "top": { "center": "app" },
                        "levels": { "a": { "box": [0, 0, 9, 9], "parts": { "b": [0, 0] } },
                                    "b": { "box": [0, 0, 9, 9], "parts": { "a": [0, 0] } } } }"#;
        assert_eq!(
            Vocabulary::parse(text).unwrap_err().problems,
            [
                "levels.a: `a` is inside itself",
                "levels.b: `b` is inside itself"
            ]
        );
    }

    #[test]
    fn a_lines_end_at_a_box_may_be_a_rule_over_its_types() {
        let text = A_VOCABULARY.replace(
            r#""lines": [{ "from": "window", "to": "server", "both": true, "sides": ["top", "bottom"] }]"#,
            r#""lines": [{ "from": "user", "to": { "box": "server", "implements": "tui/Renderer" } },
                         { "from": { "box": "server", "uses": "useState" }, "to": "user", "both": true }]"#,
        );
        let vocabulary = Vocabulary::parse(&text).unwrap();
        let lines = &vocabulary.levels["app"].lines;
        assert_eq!(
            (lines[0].from.as_str(), lines[0].to.as_str()),
            ("user", "server")
        );
        assert_eq!(
            lines[0].rules,
            [None, Some(Rule::Implements("tui/Renderer".to_owned()))]
        );
        assert_eq!(
            lines[1].rules,
            [Some(Rule::Uses("useState".to_owned())), None]
        );
        assert!(lines[1].both);
        assert_eq!(
            lines[1].rules[0].as_ref().unwrap().to_string(),
            "uses useState"
        );

        // A rule is of one kind, and names something; its file says where
        // one is not.
        for (end, said) in [
            (
                r#"{ "box": "server", "implements": "A", "uses": "B" }"#,
                "the end at `server` is a rule of one kind: `implements` or `uses`",
            ),
            (
                r#"{ "box": "server" }"#,
                "the end at `server` is a rule of one kind: `implements` or `uses`",
            ),
            (
                r#"{ "box": "server", "implements": " / " }"#,
                "the end at `server` names nothing",
            ),
            (
                r#"{ "box": "server", "implement": "A" }"#,
                "unknown field `implement`",
            ),
            ("3", "a node's id, or a rule over the types inside a box"),
        ] {
            let text = A_VOCABULARY.replace(
                r#""to": "server", "both""#,
                &format!(r#""to": {end}, "both""#),
            );
            let error = Vocabulary::parse(&text).unwrap_err().to_string();
            assert!(error.contains(said), "{error}");
            assert!(error.contains("at line 16 column"), "{error}");
        }

        // Nor does a rule pick of a node that shows no types: one outside,
        // or a box that stays closed.
        let text = A_VOCABULARY.replace(
            r#""lines": [{ "from": "window", "to": "server", "both": true, "sides": ["top", "bottom"] }]"#,
            r#""lines": [{ "from": { "box": "user", "uses": "A" }, "to": "server" },
                         { "from": "user", "to": { "box": "store", "implements": "B" } }]"#,
        );
        assert_eq!(
            Vocabulary::parse(&text).unwrap_err().problems,
            [
                "levels.app.lines[0].from: `uses A` picks of the types inside `user`, \
                 which shows none: it owns no code, stays closed, or is outside",
                "levels.app.lines[1].to: `implements B` picks of the types inside `store`, \
                 which shows none: it owns no code, stays closed, or is outside",
            ]
        );
        // A box whose level holds boxes that open shows their types.
        assert!(
            Vocabulary::parse(&nested("hand"))
                .unwrap()
                .shows_types("group")
        );
    }

    #[test]
    fn coxs_own_maps_read() {
        // Cox's maps of itself, which its agents keep as they move its code.
        let maps = [
            ("use", include_str!("../../../../maps/use.json")),
            ("build", include_str!("../../../../maps/build.json")),
        ];
        for (name, text) in maps {
            let vocabulary = Vocabulary::parse(text)
                .unwrap_or_else(|error| panic!("maps/{name}.json does not read: {error}"));
            assert_eq!(vocabulary.top.center, "cox", "{name}");
        }
    }

    #[test]
    fn a_file_of_another_shape_says_what_it_does_not_know() {
        let error = Vocabulary::parse(r#"{ "tiers": [], "nodes": {} }"#).unwrap_err();
        assert!(error.to_string().starts_with("unknown field `tiers`"));
        let error = Vocabulary::parse(r#"{ "nodes": {}, "top": { "center": "x" }"#).unwrap_err();
        assert!(error.to_string().contains("line 1"));
    }
}
