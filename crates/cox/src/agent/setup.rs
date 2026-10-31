//! How a chat's agent is set up, when the chat that opened it said: the
//! context it is told after the project's instructions, which of Cox's
//! own tools it gets, whether it gets the tools that drive the stage, and
//! which of its permission requests Cox answers in the human's place. A
//! chat the human opens has no setup: its agent gets every tool, and the
//! human answers every request.
//!
//! The policy answers the agent's permission requests by their titles: a
//! request whose title matches a pattern in `allow` is allowed once, one
//! that matches a pattern in `reject` is rejected once, and the human is
//! asked about the rest. A `*` in a pattern matches any run of
//! characters; the rest matches as written. The title is the agent's, as
//! it shows in the conversation.

use serde::{Deserialize, Serialize};

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

/// What answers permission requests in the human's place, by their
/// titles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    /// Patterns of the titles to allow.
    pub allow: Vec<String>,
    /// Patterns of the titles to reject. A title both match is rejected.
    pub reject: Vec<String>,
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

    /// What the agent gets of its context ahead of the first line, after
    /// the project's instructions, or `None` when it was given none.
    #[must_use]
    pub fn prompt(&self) -> Option<String> {
        let context = self.context.as_deref().map(str::trim)?;
        (!context.is_empty()).then(|| {
            format!(
                "The chat that opened this one set it up with this context. Keep to it for the \
                 whole session.\n\n{context}\n"
            )
        })
    }
}

impl Policy {
    /// What the policy says of a permission request titled `title`, or
    /// `None` when it says nothing, and the human is to be asked.
    #[must_use]
    pub fn decide(&self, title: &str) -> Option<Decision> {
        if self.reject.iter().any(|pattern| matches(pattern, title)) {
            Some(Decision::Reject)
        } else if self.allow.iter().any(|pattern| matches(pattern, title)) {
            Some(Decision::Allow)
        } else {
            None
        }
    }
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
                allow: ["Read *", "grep *"].map(str::to_owned).to_vec(),
                reject: ["Edit *", "Write *"].map(str::to_owned).to_vec(),
            },
        };
        let json = setup.to_json();
        assert_eq!(Setup::from_json(&json).unwrap(), setup);
        assert!(json.contains("\"stage\":\"look\""), "{json}");

        // Every part can be left out.
        assert_eq!(Setup::from_json("{}").unwrap(), Setup::default());
        assert_eq!(
            Setup::from_json(r#"{"policy":{"reject":["Edit *"]}}"#)
                .unwrap()
                .policy
                .reject,
            ["Edit *"]
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
        assert!(prompt.starts_with("The chat that opened this one set it up"));
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
        };
        assert_eq!(policy.decide("Read src/main.rs"), Some(Decision::Allow));
        assert_eq!(policy.decide("cargo xtask ci"), Some(Decision::Allow));
        assert_eq!(policy.decide("git push"), Some(Decision::Allow));
        assert_eq!(policy.decide("git push origin main"), Some(Decision::Allow));
        assert_eq!(policy.decide("Write lib.rs"), Some(Decision::Allow));
        assert_eq!(policy.decide("ls"), Some(Decision::Allow));
        assert_eq!(policy.decide("Edit src/main.rs"), Some(Decision::Reject));
        // Rejected before allowed.
        assert_eq!(
            policy.decide("cargo publish --dry-run"),
            Some(Decision::Reject)
        );
        assert_eq!(policy.decide("ls -l"), None);
        assert_eq!(policy.decide("Write x.txt"), None);
        assert_eq!(policy.decide("Readme"), None);
        assert_eq!(Policy::default().decide("anything"), None);
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
