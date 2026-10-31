//! A project's roles: what a chat in the project can be opened as. A
//! role is a file `.cox/roles/<name>.md` in the project's folder. TOML
//! between `---` lines at its top says how the role's agent runs, and
//! the Markdown under it is what the agent is told of its role, after
//! the project's instructions. Every part of the front matter can be
//! left out, and so can the front matter as a whole.
//!
//! ```markdown
//! ---
//! agent = "npx -y @agentclientprotocol/claude-agent-acp"
//! env = { CLAUDE_CODE_THINKING_DISPLAY = "summarized" }
//! tools = ["show_markdown", "show_file"]
//! stage = "look"
//! [permissions]
//! allow = ["Read *", "grep *", "cargo *"]
//! reject = ["Edit *", "Write *"]
//! ---
//! # Reviewer
//!
//! Read the change and say what you think of it. Change nothing.
//! ```
//!
//! The policy under `permissions` answers the agent's permission
//! requests in the human's place: a request whose title matches a
//! pattern in `allow` is allowed once, one that matches a pattern in
//! `reject` is rejected once, and the human is asked about the rest. A
//! `*` in a pattern matches any run of characters; the rest matches as
//! written. The title is the agent's, as it shows in the conversation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::Deserialize;

/// The directory in a project's folder that holds its roles.
pub const ROLES_DIR: &str = ".cox/roles";

/// A role, as its file says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    /// The name of its file, without the extension.
    pub name: String,
    /// The file it was read from.
    pub path: PathBuf,
    /// The agent to run, the program and its arguments, in place of the
    /// one Cox was given, if the role names one.
    pub command: Option<Vec<String>>,
    /// What is set in the agent's environment.
    pub env: BTreeMap<String, String>,
    /// The tools of Cox's own the agent gets, by name, or `None` for all
    /// of them.
    pub tools: Option<Vec<String>>,
    /// Whether the agent gets the tools that drive the stage.
    pub stage: StageUse,
    /// What answers the agent's permission requests in the human's place.
    pub policy: Policy,
    /// What the agent is told of the role: the Markdown under the front
    /// matter.
    pub text: String,
}

/// What the agent of a role does with the stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageUse {
    /// The agent gets the tools that read and drive the stage's pages.
    #[default]
    Drive,
    /// The agent gets no tools for the stage: it only shows the human
    /// things there, with Cox's own tools.
    Look,
}

/// What a role answers permission requests with, by their titles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
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

/// The front matter of a role's file.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Front {
    agent: Option<String>,
    env: BTreeMap<String, String>,
    tools: Option<Vec<String>>,
    stage: StageUse,
    permissions: Policy,
}

/// The line that opens and closes the front matter.
const FENCE: &str = "---";

impl Role {
    /// The names of the roles in `folder`, in order, or none when it has
    /// no roles.
    #[must_use]
    pub fn list(folder: &Path) -> Vec<String> {
        let directory = folder.join(ROLES_DIR);
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                log::error!("cannot read {}: {error}", directory.display());
                return Vec::new();
            }
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .filter(|path| path.is_file())
            .filter_map(|path| Some(path.file_stem()?.to_str()?.to_owned()))
            .collect();
        names.sort();
        names
    }

    /// The role `name` of `folder`, as its file says it.
    ///
    /// # Errors
    ///
    /// When there is no such role, or its file cannot be read or does
    /// not parse.
    pub fn read(folder: &Path, name: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !name.is_empty() && !name.contains(['/', '\\']),
            "{name:?} is no name of a role"
        );
        let path = folder.join(ROLES_DIR).join(format!("{name}.md"));
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read the role {name} from {}", path.display()))?;
        Self::parse(name, path, &text)
    }

    /// The role `name` that `text`, read from `path`, says.
    fn parse(name: &str, path: PathBuf, text: &str) -> anyhow::Result<Self> {
        let (front, text) = split_front(text)
            .with_context(|| format!("the role {name} in {} is malformed", path.display()))?;
        let front: Front = match front {
            Some(front) => toml::from_str(front).with_context(|| {
                format!(
                    "the front matter of the role {name} in {} is malformed",
                    path.display()
                )
            })?,
            None => Front::default(),
        };
        let command: Option<Vec<String>> = front
            .agent
            .map(|agent| agent.split_whitespace().map(str::to_owned).collect())
            .filter(|command: &Vec<String>| !command.is_empty());
        Ok(Self {
            name: name.to_owned(),
            path,
            command,
            env: front.env,
            tools: front.tools,
            stage: front.stage,
            policy: front.permissions,
            text: text.to_owned(),
        })
    }

    /// What the agent gets of the role ahead of the first line the human
    /// sends, after the project's instructions, or `None` when the role
    /// says nothing to tell.
    #[must_use]
    pub fn prompt(&self) -> Option<String> {
        let text = self.text.trim();
        (!text.is_empty()).then(|| {
            format!(
                "Your role in this project, from {}. Keep to it for the whole session.\n\n{text}\n",
                self.path.display()
            )
        })
    }
}

/// The front matter of `text`, if it has one, and the rest.
///
/// # Errors
///
/// When the front matter opens and does not close.
fn split_front(text: &str) -> anyhow::Result<(Option<&str>, &str)> {
    let Some(opened) = text.strip_prefix(FENCE) else {
        return Ok((None, text));
    };
    let Some(opened) = opened
        .strip_prefix('\n')
        .or_else(|| opened.strip_prefix("\r\n"))
    else {
        // A line that only starts with `---` is text.
        return Ok((None, text));
    };
    let mut from = 0;
    for line in opened.split_inclusive('\n') {
        if line.trim_end() == FENCE {
            return Ok((Some(&opened[..from]), &opened[from + line.len()..]));
        }
        from += line.len();
    }
    anyhow::bail!("the front matter opens with `---` and does not close")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("cox-role-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join(ROLES_DIR)).unwrap();
        folder
    }

    const REVIEWER: &str = "---
agent = \"npx -y @agentclientprotocol/claude-agent-acp\"
env = { CLAUDE_CODE_THINKING_DISPLAY = \"summarized\" }
tools = [\"show_markdown\", \"show_file\"]
stage = \"look\"
[permissions]
allow = [\"Read *\", \"grep *\"]
reject = [\"Edit *\", \"Write *\"]
---
# Reviewer

Change nothing.
";

    #[test]
    fn reads_a_role_from_its_file() {
        let folder = scratch("read");
        let path = folder.join(ROLES_DIR).join("reviewer.md");
        std::fs::write(&path, REVIEWER).unwrap();

        let role = Role::read(&folder, "reviewer").unwrap();
        assert_eq!(
            role,
            Role {
                name: "reviewer".to_owned(),
                path: path.clone(),
                command: Some(
                    ["npx", "-y", "@agentclientprotocol/claude-agent-acp"]
                        .map(str::to_owned)
                        .to_vec()
                ),
                env: [(
                    "CLAUDE_CODE_THINKING_DISPLAY".to_owned(),
                    "summarized".to_owned()
                )]
                .into_iter()
                .collect(),
                tools: Some(["show_markdown", "show_file"].map(str::to_owned).to_vec()),
                stage: StageUse::Look,
                policy: Policy {
                    allow: ["Read *", "grep *"].map(str::to_owned).to_vec(),
                    reject: ["Edit *", "Write *"].map(str::to_owned).to_vec(),
                },
                text: "# Reviewer\n\nChange nothing.\n".to_owned(),
            }
        );
        let prompt = role.prompt().unwrap();
        assert!(prompt.starts_with(&format!(
            "Your role in this project, from {}.",
            path.display()
        )));
        assert!(prompt.ends_with("\n\n# Reviewer\n\nChange nothing.\n"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_role_can_leave_out_the_front_matter_or_parts_of_it() {
        let text = Role::parse("plain", "plain.md".into(), "Only text.\n").unwrap();
        assert_eq!(text.command, None);
        assert_eq!(text.env, BTreeMap::new());
        assert_eq!(text.tools, None);
        assert_eq!(text.stage, StageUse::Drive);
        assert_eq!(text.policy, Policy::default());
        assert_eq!(text.text, "Only text.\n");

        let some = Role::parse("some", "some.md".into(), "---\nstage = \"look\"\n---\n").unwrap();
        assert_eq!(some.stage, StageUse::Look);
        assert_eq!(some.text, "");
        assert_eq!(some.prompt(), None);

        let empty = Role::parse("empty", "empty.md".into(), "").unwrap();
        assert_eq!(empty.prompt(), None);

        // A line that only starts with `---` is text.
        let rule = Role::parse("rule", "rule.md".into(), "--- not a fence\n").unwrap();
        assert_eq!(rule.text, "--- not a fence\n");
    }

    #[test]
    fn a_malformed_role_is_an_error() {
        let unclosed = Role::parse("a", "a.md".into(), "---\nstage = \"look\"\n").unwrap_err();
        assert!(
            format!("{unclosed:#}").contains("does not close"),
            "{unclosed:#}"
        );

        let unknown = Role::parse("b", "b.md".into(), "---\ncolour = \"red\"\n---\n").unwrap_err();
        assert!(
            format!("{unknown:#}").contains("front matter"),
            "{unknown:#}"
        );

        let stage = Role::parse("c", "c.md".into(), "---\nstage = \"fly\"\n---\n");
        assert!(stage.is_err());

        let folder = scratch("missing");
        assert!(Role::read(&folder, "nobody").is_err());
        assert!(Role::read(&folder, "../nobody").is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn lists_the_roles_of_a_folder_in_order() {
        let folder = scratch("list");
        let roles = folder.join(ROLES_DIR);
        std::fs::write(roles.join("reviewer.md"), "").unwrap();
        std::fs::write(roles.join("builder.md"), "").unwrap();
        std::fs::write(roles.join("notes.txt"), "").unwrap();
        std::fs::create_dir(roles.join("old.md")).unwrap();
        assert_eq!(Role::list(&folder), ["builder", "reviewer"]);

        let none = scratch("none");
        std::fs::remove_dir_all(none.join(ROLES_DIR)).unwrap();
        assert_eq!(Role::list(&none), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&folder);
        let _ = std::fs::remove_dir_all(&none);
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
