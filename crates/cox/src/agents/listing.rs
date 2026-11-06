//! `agents.json` as the settings screen changes it: each agent's object
//! whole, so that what Cox does not know of it stays, written back as the
//! human would write it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use serde_json::{Map, Value};

/// The agents `agents.json` lists, in its order.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Listing {
    agents: Vec<Map<String, Value>>,
}

impl Listing {
    /// `agents.json` as it is now, with no agents when there is none.
    ///
    /// # Errors
    ///
    /// When Cox has no place for it, or it cannot be read as a list of
    /// agents.
    pub fn read() -> anyhow::Result<Self> {
        Self::read_from(&file()?)
    }

    fn read_from(file: &Path) -> anyhow::Result<Self> {
        let bytes = match fs::read(file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", file.display()));
            }
        };
        let listed: Vec<Value> = serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not a list of agents", file.display()))?;
        let agents = listed
            .into_iter()
            .map(|agent| match agent {
                Value::Object(agent) => Ok(agent),
                _ => bail!("{} lists something that is not an agent", file.display()),
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Self { agents })
    }

    /// Each agent's name and command, as the file has them, an agent with
    /// none having them empty.
    #[must_use]
    pub fn agents(&self) -> Vec<(String, Vec<String>)> {
        self.agents
            .iter()
            .map(|agent| {
                let name = agent
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let command = agent
                    .get("command")
                    .and_then(Value::as_array)
                    .map(|words| {
                        words
                            .iter()
                            .filter_map(|word| word.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                (name, command)
            })
            .collect()
    }

    /// Lists the agent named `was`, or with none, a new one at the end, as
    /// `name`, running `command`.
    ///
    /// # Errors
    ///
    /// When there is no agent named `was`, or another agent is named
    /// `name` already.
    pub fn set(&mut self, was: Option<&str>, name: &str, command: &[String]) -> anyhow::Result<()> {
        let at = was.map(|was| self.position(was).context("agents.json lists it no more"));
        let at = at.transpose()?;
        if self.position(name).is_some_and(|other| Some(other) != at) {
            bail!("Another agent is named {name} already.");
        }
        let at = at.unwrap_or_else(|| {
            self.agents.push(Map::new());
            self.agents.len() - 1
        });
        let agent = &mut self.agents[at];
        agent.insert("name".to_owned(), Value::String(name.to_owned()));
        agent.insert(
            "command".to_owned(),
            Value::Array(command.iter().cloned().map(Value::String).collect()),
        );
        Ok(())
    }

    /// Takes the agent named `name` out of the list, if it has one.
    pub fn remove(&mut self, name: &str) {
        if let Some(at) = self.position(name) {
            self.agents.remove(at);
        }
    }

    /// Keeps the list as `agents.json`.
    ///
    /// # Errors
    ///
    /// When Cox has no place for it, or it cannot be written.
    pub fn keep(&self) -> anyhow::Result<()> {
        self.keep_in(&file()?)
    }

    fn keep_in(&self, file: &Path) -> anyhow::Result<()> {
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).with_context(|| format!("making {}", parent.display()))?;
        }
        let listed: Vec<Value> = self.agents.iter().cloned().map(Value::Object).collect();
        let mut text = serde_json::to_string_pretty(&listed)?;
        text.push('\n');
        fs::write(file, text).with_context(|| format!("writing {}", file.display()))
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.agents
            .iter()
            .position(|agent| agent.get("name").and_then(Value::as_str) == Some(name))
    }
}

/// Where `agents.json` is.
fn file() -> anyhow::Result<PathBuf> {
    crate::storage::path(crate::storage::Kept::Agents)
        .context("there is no place to keep the agents")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|&word| word.to_owned()).collect()
    }

    #[test]
    fn the_listing_changes_keeping_what_cox_does_not_know() {
        let folder = std::env::temp_dir().join(format!("cox-listing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let file = folder.join("agents.json");
        fs::create_dir_all(&folder).unwrap();
        assert_eq!(Listing::read_from(&file).unwrap(), Listing::default());

        fs::write(
            &file,
            r#"[{"name": "Codex", "command": ["codex-acp"], "note": "mine"}]"#,
        )
        .unwrap();
        let mut listing = Listing::read_from(&file).unwrap();
        listing
            .set(None, "Claude", &words(&["npx", "-y", "claude-agent-acp"]))
            .unwrap();
        listing
            .set(Some("Codex"), "Codex CLI", &words(&["X=1", "codex-acp"]))
            .unwrap();
        assert!(listing.set(Some("Gone"), "Gone", &[]).is_err());
        assert!(listing.set(None, "Claude", &[]).is_err());
        listing.keep_in(&file).unwrap();

        let listing = Listing::read_from(&file).unwrap();
        assert_eq!(
            listing.agents(),
            [
                ("Codex CLI".to_owned(), words(&["X=1", "codex-acp"])),
                (
                    "Claude".to_owned(),
                    words(&["npx", "-y", "claude-agent-acp"])
                ),
            ]
        );
        assert_eq!(listing.agents[0]["note"], "mine");

        let mut listing = listing;
        listing.remove("Codex CLI");
        assert_eq!(listing.agents().len(), 1);

        fs::write(&file, r#"{"name": "Codex"}"#).unwrap();
        assert!(Listing::read_from(&file).is_err());
        let _ = fs::remove_dir_all(&folder);
    }
}
