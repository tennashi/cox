//! How a chat's agent is set up, when the chat that opened it said: the
//! context it is told after the project's instructions, which of Cox's
//! own tools it gets, whether it gets the tools that drive the stage, and
//! which of its permission requests Cox answers in the human's place. A
//! chat the human opens has no setup: its agent gets every tool, and the
//! human answers every request.
//!
//! The policy answers the agent's permission requests by what they ask
//! for: a request that matches a pattern in `allow` is allowed once, one
//! that matches a pattern in `reject` is rejected once, and the human is
//! asked about the rest. A pattern matches the request's title, the
//! agent's, as it shows in the conversation, where a `*` matches any run
//! of characters and the rest matches as written. A pattern that starts
//! with the name of one of ACP's kinds of tool and a `:`, as `edit:*` or
//! `execute:cargo test*`, matches only a request of that kind, whose
//! title matches the rest, so that a policy can say what a call does
//! however the agent titles it. A policy can also keep the calls that
//! change files to a folder: one it allows is allowed only when every
//! file the agent says it works on is in the folder.

use std::path::{Component, Path, PathBuf};

use cox_acp::ToolKind;

use serde::{Deserialize, Serialize};

use crate::chat::sketch::Sketch;

/// How a chat's agent is set up, as the chat that opened it said.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    /// What the agent is told after the project's instructions, ahead of
    /// the first line, if anything.
    pub context: Option<String>,
    /// The tools of Cox's own the agent gets, by name, or `None` for all
    /// of them.
    pub tools: Option<Vec<String>>,
    /// Whether the agent gets the tools that drive the stage.
    pub stage: StageUse,
    /// What answers the agent's permission requests in the human's place.
    pub policy: Policy,
    /// The folder the agent works in, in place of the project's, when the
    /// chat is a sketch: a folder of its own (see
    /// [`crate::chat::sketch`]).
    pub folder: Option<PathBuf>,
    /// What the chat started from, when it is a sketch: not a chat that a
    /// sketch opened, which works in its folder too. A Cox from before
    /// sketches were named so kept it as `trial`.
    #[serde(alias = "trial")]
    pub sketch: Option<Sketch>,
}

/// Whether the agent of a chat set up as `setup`, or not set up, gets
/// the tool that writes the wiki, to keep what the chat learned there.
#[must_use]
pub fn keeps_in_wiki(setup: Option<&Setup>) -> bool {
    setup.is_none_or(|setup| setup.gives("wiki_write"))
}

/// What an agent does with the stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageUse {
    /// The agent gets the tools that read and drive the stage's pages.
    #[default]
    Drive,
    /// The agent gets no tools for the stage: it only shows the human
    /// things there, with Cox's own tools.
    Look,
}

/// What answers permission requests in the human's place, by what they
/// ask for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    /// Patterns of the requests to allow.
    pub allow: Vec<String>,
    /// Patterns of the requests to reject. A request both match is
    /// rejected.
    pub reject: Vec<String>,
    /// The folder that the calls changing files, which `allow` allows, are
    /// kept to, if any: such a call that works on a file elsewhere, or
    /// does not say which files it works on, is asked of the human.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub within: Option<PathBuf>,
}

/// A permission request, as a policy reads it: what the agent said of the
/// call it asks to make.
#[derive(Clone, Copy, Debug)]
pub struct Asked<'a> {
    /// The call's title, empty when the agent gave none.
    pub title: &'a str,
    /// What kind of tool the call is, if the agent said.
    pub kind: Option<ToolKind>,
    /// The files the call works on, as the agent said.
    pub locations: &'a [PathBuf],
}

impl<'a> Asked<'a> {
    /// A request that says only its title.
    #[must_use]
    pub fn titled(title: &'a str) -> Self {
        Self {
            title,
            kind: None,
            locations: &[],
        }
    }
}

/// What a policy says of a permission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Reject,
}

impl Setup {
    /// The setup `json` keeps, as [`Setup::to_json`] wrote it.
    ///
    /// # Errors
    ///
    /// When `json` is not a setup.
    pub fn from_json(json: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(json)?)
    }

    /// The setup as JSON, to keep.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Whether the agent gets the tool of Cox's own named `tool`.
    #[must_use]
    pub fn gives(&self, tool: &str) -> bool {
        self.tools
            .as_ref()
            .is_none_or(|tools| tools.iter().any(|given| given == tool))
    }

    /// What the agent gets of its context ahead of the first line, after
    /// the project's instructions, or `None` when it was given none.
    #[must_use]
    pub fn prompt(&self) -> Option<String> {
        let context = self.context.as_deref().map(str::trim)?;
        (!context.is_empty()).then(|| {
            format!(
                "You were set up with this context for the whole session. Keep to \
                 it.\n\n{context}\n"
            )
        })
    }
}

impl Policy {
    /// What the policy says of the permission request `asked`, or `None`
    /// when it says nothing, and the human is to be asked.
    #[must_use]
    pub fn decide(&self, asked: Asked<'_>) -> Option<Decision> {
        if self.reject.iter().any(|pattern| answers(pattern, asked)) {
            Some(Decision::Reject)
        } else if self.allow.iter().any(|pattern| answers(pattern, asked)) && self.keeps(asked) {
            Some(Decision::Allow)
        } else {
            None
        }
    }

    /// Whether `asked` keeps to the policy's folder: it changes no files,
    /// as far as the agent said, or every file it says it works on is in
    /// the folder, or the policy keeps calls to none.
    fn keeps(&self, asked: Asked<'_>) -> bool {
        let Some(folder) = &self.within else {
            return true;
        };
        if !asked.kind.is_some_and(ToolKind::changes_files) {
            return true;
        }
        !asked.locations.is_empty() && asked.locations.iter().all(|path| inside(path, folder))
    }
}

/// Whether `pattern` matches `asked`: its title, and its kind when the
/// pattern names one.
fn answers(pattern: &str, asked: Asked<'_>) -> bool {
    let kinded = pattern
        .split_once(':')
        .and_then(|(name, rest)| Some((ToolKind::named(name)?, rest)));
    match kinded {
        Some((kind, rest)) => asked.kind == Some(kind) && matches(rest, asked.title),
        None => matches(pattern, asked.title),
    }
}

/// Whether `path` is in `folder`, as written: an absolute path under it
/// that does not climb out of it with `..`.
fn inside(path: &Path, folder: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|component| component == Component::ParentDir)
        && path.starts_with(folder)
}

/// Whether `text` is what `pattern` says, where `*` stands for any run
/// of characters.
fn matches(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return text.is_empty();
    };
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        // No `*`: the whole text has to match.
        return rest.is_empty();
    };
    for part in middle {
        let Some(found) = rest.find(part) else {
            return false;
        };
        rest = &rest[found + part.len()..];
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setup_is_kept_as_json_and_read_back() {
        let setup = Setup {
            context: Some("Change nothing.".to_owned()),
            tools: Some(["show_markdown", "show_file"].map(str::to_owned).to_vec()),
            stage: StageUse::Look,
            policy: Policy {
                allow: ["read:*", "grep *"].map(str::to_owned).to_vec(),
                reject: ["Edit *", "Write *"].map(str::to_owned).to_vec(),
                within: Some("/sketches/greet-1".into()),
            },
            folder: Some("/sketches/greet-1".into()),
            sketch: Some(Sketch::Project),
        };
        let json = setup.to_json();
        assert_eq!(Setup::from_json(&json).unwrap(), setup);
        assert!(json.contains("\"stage\":\"look\""), "{json}");
        assert!(json.contains("\"sketch\":\"project\""), "{json}");

        // A Cox from before sketches were named so kept it as `trial`.
        assert_eq!(
            Setup::from_json(r#"{"trial":"empty"}"#).unwrap().sketch,
            Some(Sketch::Empty)
        );

        // Every part can be left out.
        assert_eq!(Setup::from_json("{}").unwrap(), Setup::default());
        assert_eq!(
            Setup::from_json(r#"{"policy":{"reject":["Edit *"]}}"#)
                .unwrap()
                .policy
                .reject,
            ["Edit *"]
        );
        // A policy kept before it could keep calls to a folder keeps them
        // to none.
        assert_eq!(
            Setup::from_json(r#"{"policy":{"allow":["Edit *"]}}"#)
                .unwrap()
                .policy
                .within,
            None
        );
        assert!(Setup::from_json("not json").is_err());
    }

    #[test]
    fn the_context_is_told_when_there_is_one() {
        let told = Setup {
            context: Some("Change nothing.\n".to_owned()),
            ..Setup::default()
        };
        let prompt = told.prompt().unwrap();
        assert!(prompt.starts_with("You were set up with this context"));
        assert!(prompt.ends_with("\n\nChange nothing.\n"));
        assert_eq!(Setup::default().prompt(), None);
        let blank = Setup {
            context: Some("  \n".to_owned()),
            ..Setup::default()
        };
        assert_eq!(blank.prompt(), None);
    }

    #[test]
    fn a_policy_decides_by_the_title() {
        let policy = Policy {
            allow: ["Read *", "cargo *", "git push*", "*.rs", "ls"]
                .map(str::to_owned)
                .to_vec(),
            reject: ["Edit *", "cargo publish*"].map(str::to_owned).to_vec(),
            within: None,
        };
        let decide = |title| policy.decide(Asked::titled(title));
        assert_eq!(decide("Read src/main.rs"), Some(Decision::Allow));
        assert_eq!(decide("cargo xtask ci"), Some(Decision::Allow));
        assert_eq!(decide("git push"), Some(Decision::Allow));
        assert_eq!(decide("git push origin main"), Some(Decision::Allow));
        assert_eq!(decide("Write lib.rs"), Some(Decision::Allow));
        assert_eq!(decide("ls"), Some(Decision::Allow));
        assert_eq!(decide("Edit src/main.rs"), Some(Decision::Reject));
        // Rejected before allowed.
        assert_eq!(decide("cargo publish --dry-run"), Some(Decision::Reject));
        assert_eq!(decide("ls -l"), None);
        assert_eq!(decide("Write x.txt"), None);
        assert_eq!(decide("Readme"), None);
        assert_eq!(Policy::default().decide(Asked::titled("anything")), None);
    }

    #[test]
    fn a_pattern_that_names_a_kind_matches_only_that_kind() {
        let policy = Policy {
            allow: ["read:*", "execute:cargo test*", "edit:*.md"]
                .map(str::to_owned)
                .to_vec(),
            reject: vec!["delete:*".to_owned()],
            within: None,
        };
        let decide = |title, kind| {
            policy.decide(Asked {
                title,
                kind,
                locations: &[],
            })
        };
        // Whatever the agent titles it.
        assert_eq!(
            decide("View notes", Some(ToolKind::Read)),
            Some(Decision::Allow)
        );
        assert_eq!(decide("", Some(ToolKind::Read)), Some(Decision::Allow));
        assert_eq!(
            decide("cargo test --workspace", Some(ToolKind::Execute)),
            Some(Decision::Allow)
        );
        assert_eq!(decide("cargo test", Some(ToolKind::Other)), None);
        assert_eq!(decide("cargo test", None), None);
        assert_eq!(decide("cargo build", Some(ToolKind::Execute)), None);
        assert_eq!(
            decide("Edit notes.md", Some(ToolKind::Edit)),
            Some(Decision::Allow)
        );
        assert_eq!(decide("Edit main.rs", Some(ToolKind::Edit)), None);
        assert_eq!(
            decide("Remove notes.md", Some(ToolKind::Delete)),
            Some(Decision::Reject)
        );
        // A name before a `:` that is no kind is part of the title.
        let titled = Policy {
            allow: vec!["Note: *".to_owned()],
            ..Policy::default()
        };
        assert_eq!(
            titled.decide(Asked::titled("Note: hi")),
            Some(Decision::Allow)
        );
    }

    #[test]
    fn a_policy_keeps_the_calls_that_change_files_to_its_folder() {
        let policy = Policy {
            allow: ["edit:*", "move:*", "read:*"].map(str::to_owned).to_vec(),
            reject: Vec::new(),
            within: Some("/sketches/scratch-1".into()),
        };
        let decide = |kind, locations: &[&str]| {
            let locations: Vec<PathBuf> = locations.iter().map(PathBuf::from).collect();
            policy.decide(Asked {
                title: "Change it",
                kind: Some(kind),
                locations: &locations,
            })
        };
        assert_eq!(
            decide(ToolKind::Edit, &["/sketches/scratch-1/notes.md"]),
            Some(Decision::Allow)
        );
        assert_eq!(
            decide(
                ToolKind::Move,
                &["/sketches/scratch-1/a.md", "/sketches/scratch-1/b/a.md"]
            ),
            Some(Decision::Allow)
        );
        // Elsewhere, partly elsewhere, or nowhere it says.
        assert_eq!(decide(ToolKind::Edit, &["/home/me/notes.md"]), None);
        assert_eq!(
            decide(
                ToolKind::Move,
                &["/sketches/scratch-1/a.md", "/home/me/a.md"]
            ),
            None
        );
        assert_eq!(decide(ToolKind::Edit, &[]), None);
        assert_eq!(
            decide(ToolKind::Edit, &["/sketches/scratch-1/../x.md"]),
            None
        );
        assert_eq!(decide(ToolKind::Edit, &["/sketches/scratch-10/x.md"]), None);
        assert_eq!(decide(ToolKind::Edit, &["notes.md"]), None);
        // A call that changes no files goes anywhere.
        assert_eq!(
            decide(ToolKind::Read, &["/home/me/notes.md"]),
            Some(Decision::Allow)
        );
    }

    #[test]
    fn a_star_stands_for_any_run_of_characters() {
        assert!(matches("*", ""));
        assert!(matches("*", "anything at all"));
        assert!(matches("a*b*c", "abc"));
        assert!(matches("a*b*c", "a--b--c"));
        assert!(!matches("a*b*c", "a--c--b"));
        assert!(matches("**", "x"));
        assert!(!matches("", "x"));
        assert!(matches("", ""));
    }
}
