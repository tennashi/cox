//! What Cox tells a chat's agent in the human's place at the turns of a
//! trial: as it starts, as a trial an agent opened is open, and before
//! it is discarded; and as the human asks it to keep what the chat
//! learned in their wiki (see [`crate::wiki`]). Each is the human's to say, since how they work with
//! trials is theirs: a file of their workflow's folder (see
//! [`crate::settings::workflow`]), as `trial/start.md`, or else Cox's own,
//! in `workflow/` beside this file. A file says where Cox puts what it
//! knows then with a name in braces, as `{folder}`, and a name a text
//! does not know stays as it is.
//!
//! How a trial ends is the human's too: the ways the menu of the right
//! button offers, and what the trial's agent goes on with unasked, which
//! their workflow's `trial.json` says (see [`Trial`]), or else Cox's own,
//! `workflow/trial.json`.

use std::path::Path;

use serde::Deserialize;

use crate::chat::trial::Trial as From;

/// A text Cox says at a turn of the human's workflow.
pub struct Text {
    /// Where the human keeps their own, in their workflow's folder.
    name: &'static str,
    /// Cox's own, for a human who keeps none.
    own: &'static str,
}

/// The first line of a trial opened beside a chat, which its agent is
/// told: that it works in `{folder}`, which is `{from}`, to try `{what}`.
pub const TRIAL_START: Text = Text {
    name: "trial/start.md",
    own: include_str!("workflow/trial/start.md"),
};

/// What the agent that opened a trial with `open_chat` is told of it:
/// that it works in `{folder}`, which is `{from}`.
pub const TRIAL_OPENED: Text = Text {
    name: "trial/opened.md",
    own: include_str!("workflow/trial/opened.md"),
};

/// What a trial's agent is asked before the trial is discarded, with
/// `{left_out}`, the files too big to keep, when there are any.
pub const TRIAL_DISCARD: Text = Text {
    name: "trial/discard.md",
    own: include_str!("workflow/trial/discard.md"),
};

/// What a chat's agent is asked when the human asks it to keep what the
/// chat learned in their wiki.
pub const WIKI_SAVE: Text = Text {
    name: "wiki/save.md",
    own: include_str!("workflow/wiki/save.md"),
};

/// The texts Cox has of its own, by where the human keeps theirs.
const OWN: [&Text; 4] = [&TRIAL_START, &TRIAL_OPENED, &TRIAL_DISCARD, &WIKI_SAVE];

/// The text the human keeps as `name` in their workflow, or else Cox's
/// own of that name, with `values` in place of the names they stand for;
/// `None` when neither has it.
#[must_use]
pub fn say(name: &str, values: &[(&str, &str)]) -> Option<String> {
    say_in(crate::settings::workflow().as_deref(), name, values)
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

/// How the human's trials go: what their agent goes on with unasked, and
/// the ways they end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    /// The patterns of the permission requests a trial's agent goes on
    /// with unasked, on top of what reads and tests, when they change the
    /// trial's own files only.
    pub allow: Vec<String>,
    /// The ways a trial ends, in the order the menu offers them.
    pub endings: Vec<Ending>,
}

/// A way a trial ends, as the menu of the right button offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ending {
    /// What the menu calls it.
    pub name: String,
    /// The trials it ends, those from the project or from nothing, or
    /// either when `None`.
    pub from: Option<From>,
    /// The text the trial's agent is asked first, to sum the trial up, by
    /// where the human keeps it, as `trial/discard.md`.
    pub sum_up: Option<String>,
    /// Whether what the trial changed is kept apart (see
    /// [`crate::chat::trial::discard`]) before its folder goes.
    pub keep: bool,
    /// Whether the trial's folder is opened as a project, in place of
    /// going, for a trial from nothing.
    pub open: bool,
    /// The text the agent of the chat the trial was opened beside is told
    /// once the trial ends, with `{kept}`, where what the trial changed is
    /// kept, and `{summary}`, what the trial's agent said last.
    pub then: Option<String>,
    /// The goal the chat works toward once it goes on in the project, as
    /// "Set goal…" takes it.
    pub goal: Option<String>,
}

impl Ending {
    /// Whether it ends a trial that started from `from`.
    #[must_use]
    pub fn ends(&self, from: From) -> bool {
        self.from.is_none_or(|ends| ends == from) && (!self.open || from == From::Empty)
    }
}

/// `trial.json` as it is written.
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

/// Cox's own `trial.json`, for a human who keeps none.
const OWN_TRIAL: &str = include_str!("workflow/trial.json");

/// How the human's trials go, as their workflow's `trial.json` says, or
/// else as Cox's own does.
#[must_use]
pub fn trial() -> Trial {
    trial_in(crate::settings::workflow().as_deref())
}

/// As [`trial`], with the human's workflow in `workflow`. What cannot be
/// read of theirs is said, and Cox's own counts in its place.
///
/// # Panics
///
/// Only were Cox's own `trial.json` not to read, which its tests check.
#[must_use]
pub fn trial_in(workflow: Option<&Path>) -> Trial {
    let path = workflow.map(|workflow| workflow.join("trial.json"));
    if let Some(text) = path.as_deref().and_then(human) {
        match parse(&text) {
            Ok(trial) => return trial,
            Err(error) => tracing::error!(
                "{} cannot be read, so Cox's own counts: {error}",
                path.unwrap_or_default().display()
            ),
        }
    }
    parse(OWN_TRIAL).expect("Cox's own trial.json reads")
}

fn parse(text: &str) -> serde_json::Result<Trial> {
    let written: Written = serde_json::from_str(text)?;
    Ok(Trial {
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
        self.say_in(crate::settings::workflow().as_deref(), values)
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
fn fill(text: &str, values: &[(&str, &str)]) -> String {
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
    fn cox_s_own_trials_open_from_nothing_and_are_discarded() {
        let own = trial_in(None);
        assert_eq!(own.allow, ["edit:*", "delete:*", "move:*"]);
        let names: Vec<_> = own
            .endings
            .iter()
            .map(|ending| ending.name.as_str())
            .collect();
        assert_eq!(names, ["Open as project", "Discard trial"]);
        let [open, discard] = &own.endings[..] else {
            unreachable!()
        };
        assert!(open.open && !open.ends(From::Project) && open.ends(From::Empty));
        assert!(discard.keep && discard.ends(From::Project) && discard.ends(From::Empty));
        assert_eq!(discard.sum_up.as_deref(), Some("trial/discard.md"));
    }

    #[test]
    fn the_human_s_trial_json_goes_ahead_of_cox_s() {
        let folder = std::env::temp_dir().join(format!("cox-workflow-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("trial")).unwrap();
        std::fs::write(
            folder.join("trial.json"),
            r#"{"endings": [{"name": "Rewrite in project", "sum_up": "trial/understand.md",
                "keep": "ref", "then": "trial/rewrite.md", "goal": "$ cargo test"}]}"#,
        )
        .unwrap();
        let trial = trial_in(Some(&folder));
        assert_eq!(trial.allow, Vec::<String>::new());
        assert_eq!(
            trial.endings,
            [Ending {
                name: "Rewrite in project".to_owned(),
                from: None,
                sum_up: Some("trial/understand.md".to_owned()),
                keep: true,
                open: false,
                then: Some("trial/rewrite.md".to_owned()),
                goal: Some("$ cargo test".to_owned()),
            }]
        );
        // A text of the human's own is theirs to keep; Cox has none of it.
        std::fs::write(folder.join("trial/rewrite.md"), "Rewrite from {kept}.\n").unwrap();
        assert_eq!(
            say_in(Some(&folder), "trial/rewrite.md", &[("kept", "refs/x")]).as_deref(),
            Some("Rewrite from refs/x.")
        );
        assert_eq!(say_in(Some(&folder), "trial/understand.md", &[]), None);
        assert!(say_in(Some(&folder), "trial/discard.md", &[]).is_some());

        // What cannot be read counts as Cox's own.
        std::fs::write(
            folder.join("trial.json"),
            r#"{"endings": [{"keep": "branch"}]}"#,
        )
        .unwrap();
        assert_eq!(trial_in(Some(&folder)), trial_in(None));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_human_s_text_goes_ahead_of_cox_s() {
        let folder = std::env::temp_dir().join(format!("cox-workflow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let values = [
            ("folder", "/t"),
            ("from", "nothing"),
            ("what", "a greeting"),
        ];

        let own = TRIAL_START.say_in(Some(&folder), &values);
        assert!(
            own.starts_with("You work in a trial now, in /t, nothing,"),
            "{own}"
        );
        assert!(
            own.ends_with("Go on with what you try: a greeting"),
            "{own}"
        );

        std::fs::create_dir_all(folder.join("trial")).unwrap();
        std::fs::write(
            folder.join("trial/start.md"),
            "Spike {what} in {folder}.\n\n",
        )
        .unwrap();
        assert_eq!(
            TRIAL_START.say_in(Some(&folder), &values),
            "Spike a greeting in /t."
        );
        // An empty file says nothing of the human's.
        std::fs::write(folder.join("trial/start.md"), "\n").unwrap();
        assert_eq!(TRIAL_START.say_in(Some(&folder), &values), own);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
