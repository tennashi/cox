//! What Cox tells a chat's agent in the human's place at the turns of a
//! sketch: as it starts, as a sketch an agent opened is open, and before
//! it is discarded; and as the human asks it to keep what the chat
//! learned in their wiki (see [`crate::wiki`]), and to merge what it made,
//! and as it asks an agent to draw a project's icon (see
//! [`crate::project_icon`]), and as it tells an agent the reports of the
//! subagents it ran and did not wait for. Each is the human's to say,
//! since how they work with sketches is theirs: a file of their workflow's folder (see
//! [`crate::settings::workflow`]), as `sketch/start.md`, or else Cox's own,
//! in `workflow/` beside this file. A file says where Cox puts what it
//! knows then with a name in braces, as `{folder}`, and a name a text
//! does not know stays as it is.
//!
//! How a sketch ends is the human's too: the ways the menu of the right
//! button offers, and what the sketch's agent goes on with unasked, which
//! their workflow's `sketch.json` says (see [`Sketch`]), or else Cox's own,
//! `workflow/sketch.json`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agent::mcp::Waited;
use crate::chat::sketch::Sketch as From;

/// A text Cox says at a turn of the human's workflow.
pub struct Text {
    /// Where the human keeps their own, in their workflow's folder.
    name: &'static str,
    /// Cox's own, for a human who keeps none.
    own: &'static str,
}

/// The first line of a sketch opened beside a chat, which its agent is
/// told: that it works in `{folder}`, which is `{from}`, to try `{what}`.
pub const SKETCH_START: Text = Text {
    name: "sketch/start.md",
    own: include_str!("workflow/sketch/start.md"),
};

/// What the agent that ran a subagent in a sketch with `agent` is told of it:
/// that it works in `{folder}`, which is `{from}`.
pub const SKETCH_OPENED: Text = Text {
    name: "sketch/opened.md",
    own: include_str!("workflow/sketch/opened.md"),
};

/// What a sketch's agent is asked before the sketch is discarded, with
/// `{left_out}`, the files too big to keep, when there are any.
pub const SKETCH_DISCARD: Text = Text {
    name: "sketch/discard.md",
    own: include_str!("workflow/sketch/discard.md"),
};

/// What a chat's agent is asked when the human asks it to keep what the
/// chat learned in their wiki.
pub const WIKI_SAVE: Text = Text {
    name: "wiki/save.md",
    own: include_str!("workflow/wiki/save.md"),
};

/// What a chat's agent is asked when the human, done looking at what it
/// made, asks it to merge it.
pub const CLOSE_MERGE: Text = Text {
    name: "close/merge.md",
    own: include_str!("workflow/close/merge.md"),
};

/// What the agent that draws a project's icon is asked (see
/// [`crate::project_icon::generate`]), with `{name}`, the project's name.
pub const ICON_GENERATE: Text = Text {
    name: "icon/generate.md",
    own: include_str!("workflow/icon/generate.md"),
};

/// What the agent that ran subagents is told, in a line of its own, of
/// those it did not wait for that came to something: `{reports}`, each as
/// [`SUBAGENT_ENDED`] or [`SUBAGENT_ASKS`] says.
pub const SUBAGENT_REPORTS: Text = Text {
    name: "subagent/reports.md",
    own: include_str!("workflow/subagent/reports.md"),
};

/// A subagent's report, in [`SUBAGENT_REPORTS`], once it ended its turn:
/// its chat's `{name}`, and what it last `{said}`. Its `{number}` is there
/// too, for a text the human keeps, though Cox's own leaves it out: the
/// human sees a chat only by its name.
pub const SUBAGENT_ENDED: Text = Text {
    name: "subagent/ended.md",
    own: include_str!("workflow/subagent/ended.md"),
};

/// A subagent's report, in [`SUBAGENT_REPORTS`], once it ended its turn
/// while what it runs in the `{background}` goes on, beside what
/// [`SUBAGENT_ENDED`] says.
pub const SUBAGENT_BACKGROUND: Text = Text {
    name: "subagent/background.md",
    own: include_str!("workflow/subagent/background.md"),
};

/// A subagent's report, in [`SUBAGENT_REPORTS`], once what it ran in the
/// background, as [`SUBAGENT_BACKGROUND`] told, ended, with what
/// [`SUBAGENT_ENDED`] says.
pub const SUBAGENT_SETTLED: Text = Text {
    name: "subagent/settled.md",
    own: include_str!("workflow/subagent/settled.md"),
};

/// A subagent's report, in [`SUBAGENT_REPORTS`], as it waits for the human
/// to answer what it `{asking}`, beside what [`SUBAGENT_ENDED`] says.
pub const SUBAGENT_ASKS: Text = Text {
    name: "subagent/asks.md",
    own: include_str!("workflow/subagent/asks.md"),
};

/// How much of what a subagent last said its report in
/// [`SUBAGENT_REPORTS`] holds, in characters, from the start.
pub const REPORT_LENGTH: usize = 2000;

/// The texts Cox has of its own, by where the human keeps theirs.
const OWN: [&Text; 11] = [
    &SKETCH_START,
    &SKETCH_OPENED,
    &SKETCH_DISCARD,
    &WIKI_SAVE,
    &CLOSE_MERGE,
    &ICON_GENERATE,
    &SUBAGENT_REPORTS,
    &SUBAGENT_ENDED,
    &SUBAGENT_BACKGROUND,
    &SUBAGENT_SETTLED,
    &SUBAGENT_ASKS,
];

/// The report of the subagent in the chat `number`, named `name`, that
/// stands as `stands`, as [`SUBAGENT_REPORTS`] holds it: what it last said
/// cut to [`REPORT_LENGTH`], and what it runs in the background, if
/// anything runs, or, as `settled` says, that what the agent was told ran
/// there ended.
#[must_use]
pub fn subagent_report(number: u64, name: &str, stands: &Waited, settled: bool) -> String {
    let said = match stands.said.as_deref() {
        None => "(It said nothing.)".to_owned(),
        Some(said) => match said.char_indices().nth(REPORT_LENGTH) {
            Some((end, _)) => format!("{}\u{2026}", said[..end].trim_end()),
            None => said.to_owned(),
        },
    };
    let number = number.to_string();
    let mut values = vec![("number", number.as_str()), ("name", name), ("said", &said)];
    let background = crate::daemon::wire::told(&stands.background);
    match stands.asking.as_deref() {
        Some(asking) => {
            values.push(("asking", asking));
            SUBAGENT_ASKS.say(&values)
        }
        None if !stands.background.is_empty() => {
            values.push(("background", &background));
            SUBAGENT_BACKGROUND.say(&values)
        }
        None if settled => SUBAGENT_SETTLED.say(&values),
        None => SUBAGENT_ENDED.say(&values),
    }
}

/// The text the human keeps as `name` in their workflow, or else Cox's
/// own of that name, with `values` in place of the names they stand for;
/// `None` when neither has it.
#[must_use]
pub fn say(name: &str, values: &[(&str, &str)]) -> Option<String> {
    say_in(Some(crate::settings::workflow().as_path()), name, values)
}

/// As [`say`], with the human's workflow in `workflow`.
#[must_use]
pub fn say_in(workflow: Option<&Path>, name: &str, values: &[(&str, &str)]) -> Option<String> {
    if let Some(text) = OWN.iter().find(|text| text.name == name) {
        return Some(text.say_in(workflow, values));
    }
    let text = workflow.and_then(|workflow| human(&workflow.join(name)));
    if text.is_none() {
        tracing::error!("the workflow has no {name}");
    }
    text.map(|text| fill(&text, values).trim_end().to_owned())
}

/// What the human wrote in the file at `path`, unless there is no such
/// file, or it says nothing.
fn human(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => Some(text),
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::error!("cannot read {}: {error}", path.display());
            None
        }
    }
}

/// How the human's sketches go: what their agent goes on with unasked, and
/// the ways they end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sketch {
    /// The patterns of the permission requests a sketch's agent goes on
    /// with unasked, on top of what reads and tests, when they change the
    /// sketch's own files only.
    pub allow: Vec<String>,
    /// The ways a sketch ends, in the order the menu offers them.
    pub endings: Vec<Ending>,
}

/// A way a sketch ends, as the menu of the right button offers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ending {
    /// What the menu calls it.
    pub name: String,
    /// The sketches it ends, those from the project or from nothing, or
    /// either when `None`.
    pub from: Option<From>,
    /// The text the sketch's agent is asked first, to sum the sketch up, by
    /// where the human keeps it, as `sketch/discard.md`.
    pub sum_up: Option<String>,
    /// Whether what the sketch changed is kept apart (see
    /// [`crate::chat::sketch::discard`]) before its folder goes.
    pub keep: bool,
    /// Whether a project is made for what the sketch tried, for a sketch
    /// from nothing, what it made kept apart, as a discarded sketch's is.
    pub open: bool,
    /// The text the agent of the chat the sketch was opened beside is told
    /// once the sketch ends, with `{kept}`, where what the sketch changed is
    /// kept, `{summary}`, what the sketch's agent said last, and `{source}`,
    /// the source that keeps what the sketch did (see
    /// [`crate::chat::sketch::SOURCES`]).
    pub then: Option<String>,
    /// The goal the chat works toward once it goes on in the project, as
    /// "Set goal…" takes it.
    pub goal: Option<String>,
}

impl Ending {
    /// Whether it ends a sketch that started from `from`.
    #[must_use]
    pub fn ends(&self, from: From) -> bool {
        self.from.is_none_or(|ends| ends == from) && (!self.open || from == From::Empty)
    }
}

/// `sketch.json` as it is written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    allow: Option<Vec<String>>,
    endings: Vec<WrittenEnding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WrittenEnding {
    name: String,
    from: Option<From>,
    sum_up: Option<String>,
    keep: Option<Keep>,
    folder: Option<Folder>,
    then: Option<String>,
    goal: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Keep {
    Ref,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Folder {
    Remove,
    Open,
}

/// Cox's own `sketch.json`, for a human who keeps none.
const OWN_SKETCH: &str = include_str!("workflow/sketch.json");

/// How the human's sketches go, as their workflow's `sketch.json` says, or
/// else as Cox's own does.
#[must_use]
pub fn sketch() -> Sketch {
    sketch_in(Some(crate::settings::workflow().as_path()))
}

/// As [`sketch`], with the human's workflow in `workflow`. What cannot be
/// read of theirs is said, and Cox's own counts in its place.
///
/// # Panics
///
/// Only were Cox's own `sketch.json` not to read, which its tests check.
#[must_use]
pub fn sketch_in(workflow: Option<&Path>) -> Sketch {
    let path = workflow.map(|workflow| workflow.join("sketch.json"));
    if let Some(text) = path.as_deref().and_then(human) {
        match parse(&text) {
            Ok(sketch) => return sketch,
            Err(error) => tracing::error!(
                "{} cannot be read, so Cox's own counts: {error}",
                path.unwrap_or_default().display()
            ),
        }
    }
    parse(OWN_SKETCH).expect("Cox's own sketch.json reads")
}

fn parse(text: &str) -> serde_json::Result<Sketch> {
    let written: Written = serde_json::from_str(text)?;
    Ok(Sketch {
        allow: written.allow.unwrap_or_default(),
        endings: written
            .endings
            .into_iter()
            .map(|ending| Ending {
                name: ending.name,
                from: ending.from,
                sum_up: ending.sum_up,
                keep: matches!(ending.keep, Some(Keep::Ref)),
                open: ending.folder == Some(Folder::Open),
                then: ending.then,
                goal: ending.goal,
            })
            .collect(),
    })
}

impl Text {
    /// The text, the human's or else Cox's, with `values` in place of the
    /// names they stand for.
    #[must_use]
    pub fn say(&self, values: &[(&str, &str)]) -> String {
        self.say_in(Some(crate::settings::workflow().as_path()), values)
    }

    /// As [`Self::say`], with the human's workflow in `workflow`.
    #[must_use]
    pub fn say_in(&self, workflow: Option<&Path>, values: &[(&str, &str)]) -> String {
        let human = workflow.and_then(|workflow| human(&workflow.join(self.name)));
        fill(human.as_deref().unwrap_or(self.own), values)
            .trim_end()
            .to_owned()
    }
}

/// `text` with each `{name}` of `values` in place, in one pass, so that a
/// value that holds a name in braces stays as it is.
#[must_use]
pub fn fill(text: &str, values: &[(&str, &str)]) -> String {
    let mut filled = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        filled.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let name = &after[..close];
            values
                .iter()
                .find(|(known, _)| *known == name)
                .map(|(_, value)| (close, *value))
        });
        if let Some((close, value)) = value {
            filled.push_str(value);
            rest = &after[close + 1..];
        } else {
            filled.push('{');
            rest = after;
        }
    }
    filled.push_str(rest);
    filled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_text_is_filled_in_once() {
        assert_eq!(
            fill(
                "in {folder}, {from}: {what} {unknown} {",
                &[
                    ("folder", "/t/{what}"),
                    ("from", "a worktree"),
                    ("what", "x")
                ]
            ),
            "in /t/{what}, a worktree: x {unknown} {"
        );
    }

    #[test]
    fn cox_s_own_sketches_open_from_nothing_and_are_discarded() {
        let own = sketch_in(None);
        assert_eq!(own.allow, ["edit:*", "delete:*", "move:*"]);
        let names: Vec<_> = own
            .endings
            .iter()
            .map(|ending| ending.name.as_str())
            .collect();
        assert_eq!(names, ["Open as project", "Discard sketch"]);
        let [open, discard] = &own.endings[..] else {
            unreachable!()
        };
        assert!(open.open && !open.ends(From::Project) && open.ends(From::Empty));
        assert!(discard.keep && discard.ends(From::Project) && discard.ends(From::Empty));
        assert_eq!(discard.sum_up.as_deref(), Some("sketch/discard.md"));
    }

    #[test]
    fn the_human_s_sketch_json_goes_ahead_of_cox_s() {
        let folder = TempDir::new();
        std::fs::create_dir_all(folder.join("sketch")).unwrap();
        std::fs::write(
            folder.join("sketch.json"),
            r#"{"endings": [{"name": "Rewrite in project", "sum_up": "sketch/understand.md",
                "keep": "ref", "then": "sketch/rewrite.md", "goal": "$ cargo test"}]}"#,
        )
        .unwrap();
        let sketch = sketch_in(Some(&folder));
        assert_eq!(sketch.allow, Vec::<String>::new());
        assert_eq!(
            sketch.endings,
            [Ending {
                name: "Rewrite in project".to_owned(),
                from: None,
                sum_up: Some("sketch/understand.md".to_owned()),
                keep: true,
                open: false,
                then: Some("sketch/rewrite.md".to_owned()),
                goal: Some("$ cargo test".to_owned()),
            }]
        );
        // A text of the human's own is theirs to keep; Cox has none of it.
        std::fs::write(folder.join("sketch/rewrite.md"), "Rewrite from {kept}.\n").unwrap();
        assert_eq!(
            say_in(Some(&folder), "sketch/rewrite.md", &[("kept", "refs/x")]).as_deref(),
            Some("Rewrite from refs/x.")
        );
        assert_eq!(say_in(Some(&folder), "sketch/understand.md", &[]), None);
        assert!(say_in(Some(&folder), "sketch/discard.md", &[]).is_some());

        // What cannot be read counts as Cox's own.
        std::fs::write(
            folder.join("sketch.json"),
            r#"{"endings": [{"keep": "branch"}]}"#,
        )
        .unwrap();
        assert_eq!(sketch_in(Some(&folder)), sketch_in(None));
    }

    #[test]
    fn the_human_s_text_goes_ahead_of_cox_s() {
        let folder = TempDir::new();
        let values = [
            ("folder", "/t"),
            ("from", "nothing"),
            ("what", "a greeting"),
        ];

        let own = SKETCH_START.say_in(Some(&folder), &values);
        assert!(
            own.starts_with("You work in a sketch now, in /t, nothing,"),
            "{own}"
        );
        assert!(
            own.ends_with("Go on with what you try: a greeting"),
            "{own}"
        );

        std::fs::create_dir_all(folder.join("sketch")).unwrap();
        std::fs::write(
            folder.join("sketch/start.md"),
            "Spike {what} in {folder}.\n\n",
        )
        .unwrap();
        assert_eq!(
            SKETCH_START.say_in(Some(&folder), &values),
            "Spike a greeting in /t."
        );
        // An empty file says nothing of the human's.
        std::fs::write(folder.join("sketch/start.md"), "\n").unwrap();
        assert_eq!(SKETCH_START.say_in(Some(&folder), &values), own);
    }
}
