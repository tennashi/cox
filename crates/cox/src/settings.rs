//! What the human sets of Cox itself, on its settings screen (see
//! [`view`]), which lasts across runs in `settings.json`, where Cox keeps
//! what lasts (see [`crate::storage`]), as `{"home": "/home/me/Cox"}`.
//!
//! That is Cox's home: the folder chats in no project work in, which the
//! rail shows as `~`. `--home` on the command line goes ahead of it for
//! the run, and without either, the home is `~/cox`. The file also says
//! where the human keeps how they work with Cox, whatever the project, as
//! `{"workflow": "~/dotfiles/cox"}` (see [`workflow`]), and where their
//! wiki is, as `{"wiki": "~/notes/cox"}` (see [`wiki`]), and what language
//! they read it, and what they select, in, as `{"translation":
//! {"language": "Japanese"}}` (see [`translation`]), and the agent a new
//! chat talks to when the command line names none, by its name in
//! `agents.json`, as `{"agent": "Claude"}` (see [`agent`]), which
//! the human writes in it themselves.

pub mod view;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde_json::{Map, Value};

use crate::storage::Kept;

/// The settings as `settings.json` keeps them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Saved {
    /// Where chats in no project work, if the human chose it.
    pub home: Option<PathBuf>,
    /// The folder of the human's workflow, if they named one.
    pub workflow: Option<PathBuf>,
    /// The folder of the human's wiki, if they named one.
    pub wiki: Option<PathBuf>,
    /// How the human reads the wiki, and what they select, in a language
    /// of their own, if they named one.
    pub translation: Option<Translation>,
    /// The agent a new chat talks to when the command line names none,
    /// as when Cox is opened from the Finder, the Dock or a launcher, if
    /// they named one.
    pub agent: Option<String>,
}

/// How the human reads the wiki, which the agents write in English, and
/// what they select, in a language of their own, as `settings.json` says
/// it: `{"language": "Japanese", "command": ["plamo-translate", "--from",
/// "{from}", "--to", "{to}"]}`, or `{"language": "Japanese", "agent":
/// "Codex"}` (see [`crate::translate`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Translation {
    /// The language, as the human names it, which English goes into, and
    /// the rest into English, as the agent translating is told it, and
    /// the command gets it for `{from}` or `{to}`.
    pub language: String,
    /// The command that translates, reading the text on its standard
    /// input and writing the translation on its standard output, with
    /// `{from}` and `{to}` in its arguments standing for the languages;
    /// or `None` for an agent.
    pub command: Option<Vec<String>>,
    /// The agent that translates, when no command does, by the name the
    /// human knows it by among the agents Cox knows; or `None` for the
    /// one a new chat talks to.
    pub agent: Option<String>,
}

impl Translation {
    /// The translation `value`, as `settings.json` holds it.
    ///
    /// # Errors
    ///
    /// What is wrong with `value`, for the human to read.
    fn of(value: &Value) -> Result<Self, &'static str> {
        let language = value
            .get("language")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|language| !language.is_empty())
            .ok_or("it names no \"language\"")?;
        let command = match value.get("command") {
            None | Some(Value::Null) => None,
            Some(Value::Array(words)) => {
                let words: Option<Vec<String>> = words
                    .iter()
                    .map(|word| word.as_str().map(str::to_owned))
                    .collect();
                match words {
                    Some(words) if !words.is_empty() => Some(words),
                    _ => return Err("its \"command\" is not a list of words"),
                }
            }
            Some(_) => return Err("its \"command\" is not a list of words"),
        };
        let agent = match value.get("agent") {
            None | Some(Value::Null) => None,
            Some(Value::String(agent)) if !agent.trim().is_empty() => Some(agent.trim().to_owned()),
            Some(_) => return Err("its \"agent\" is not the name of an agent"),
        };
        Ok(Self {
            language: language.to_owned(),
            command,
            agent,
        })
    }
}

impl Saved {
    /// The settings this Cox keeps, or none when there are none, or they
    /// cannot be read, which is said.
    #[must_use]
    pub fn read() -> Self {
        crate::storage::path(Kept::Settings)
            .map_or_else(Self::default, |file| Self::read_from(&file))
    }

    fn read_from(file: &Path) -> Self {
        match fs::read(file) {
            Ok(bytes) => Self::parse(&bytes).unwrap_or_else(|error| {
                tracing::error!("{} cannot be read: {error:#}", file.display());
                Self::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::error!("{} cannot be read: {error}", file.display());
                Self::default()
            }
        }
    }

    fn parse(bytes: &[u8]) -> anyhow::Result<Self> {
        let value: Value = serde_json::from_slice(bytes)?;
        let home = match value.get("home") {
            None | Some(Value::Null) => None,
            Some(Value::String(home)) => Some(PathBuf::from(home)),
            Some(_) => anyhow::bail!("\"home\" is not a path"),
        };
        // The human writes them in the file themselves, so what is wrong
        // with one is said, and the rest of the file still counts.
        let folder = |key: &str| match value.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::String(folder)) => {
                let user = std::env::var_os("HOME").map(PathBuf::from);
                folder_of(folder, user.as_deref())
                    .inspect_err(|why| tracing::error!("\"{key}\" names no folder: {why}"))
                    .ok()
            }
            Some(_) => {
                tracing::error!("\"{key}\" is not a path");
                None
            }
        };
        let workflow = folder("workflow");
        let wiki = folder("wiki");
        let translation = match value.get("translation") {
            None | Some(Value::Null) => None,
            Some(translation) => Translation::of(translation)
                .inspect_err(|why| tracing::error!("\"translation\" cannot be read: {why}"))
                .ok(),
        };
        let agent = match value.get("agent") {
            None | Some(Value::Null) => None,
            Some(Value::String(agent)) if !agent.trim().is_empty() => Some(agent.trim().to_owned()),
            Some(_) => {
                tracing::error!("\"agent\" is not the name of an agent");
                None
            }
        };
        Ok(Self {
            home,
            workflow,
            wiki,
            translation,
            agent,
        })
    }
}

/// The agent a new chat talks to when the command line names none, by
/// the name `agents.json` lists it by: what `settings.json` names, as
/// `{"agent": "Claude"}`, if it names one.
#[must_use]
pub fn agent() -> Option<String> {
    Saved::read().agent
}

/// The folder where the human keeps how they work with Cox, whatever the
/// project: what `settings.json` names, as a folder of their dotfiles, or
/// else `workflow` where Cox keeps what lasts. Its `COX.md` is what every
/// chat's agent is told first (see [`crate::agent::instructions`]).
#[must_use]
pub fn workflow() -> Option<PathBuf> {
    Saved::read()
        .workflow
        .or_else(|| crate::storage::path(Kept::Workflow))
}

/// The folder of the human's wiki, where the agents keep what they
/// learned (see [`crate::wiki`]): what `settings.json` names, as a vault
/// of their notes, or else `wiki` where Cox keeps what lasts.
#[must_use]
pub fn wiki() -> Option<PathBuf> {
    Saved::read()
        .wiki
        .or_else(|| crate::storage::path(Kept::Wiki))
}

/// How the human reads the wiki, and what they select, in a language of
/// their own, if `settings.json` says (see [`Translation`]).
#[must_use]
pub fn translation() -> Option<Translation> {
    Saved::read().translation
}

/// Keeps `home` as where chats in no project work, in this Cox's
/// `settings.json`.
///
/// # Errors
///
/// When Cox has no place to keep it, the path is not UTF-8, or the file
/// cannot be written.
pub fn keep_home(home: &Path) -> anyhow::Result<()> {
    let file =
        crate::storage::path(Kept::Settings).context("there is no place to keep the settings")?;
    keep_home_in(&file, home)
}

/// Keeps `home` in `file`, leaving the rest of what it holds as it was.
fn keep_home_in(file: &Path, home: &Path) -> anyhow::Result<()> {
    let mut settings = match fs::read(file) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(Value::Object(settings)) => settings,
            // What cannot be read goes, in place of losing the home too.
            _ => Map::new(),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", file.display())),
    };
    let home = home
        .to_str()
        .context("the folder's path is not UTF-8, which settings.json holds")?;
    settings.insert("home".to_owned(), Value::String(home.to_owned()));
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).with_context(|| format!("making {}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(&Value::Object(settings))?;
    text.push('\n');
    fs::write(file, text).with_context(|| format!("writing {}", file.display()))
}

/// The folder `text` names, as the human types it on the settings screen:
/// a whole path, or one from the home directory `user` that starts with
/// `~`. A folder that is not there yet is made when a chat first works in
/// it.
///
/// # Errors
///
/// What is wrong with `text`, for the human to read, when it names no
/// folder.
pub fn folder_of(text: &str, user: Option<&Path>) -> Result<PathBuf, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Give the folder's path.");
    }
    let path = match text.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            let user = user.ok_or("There is no home directory for ~ to stand for.")?;
            user.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(text),
    };
    if !path.is_absolute() {
        return Err("Give the whole path, from / or from ~.");
    }
    if path.exists() && !path.is_dir() {
        return Err("That is a file, not a folder.");
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Saved, Translation, folder_of, keep_home_in};

    #[test]
    fn the_home_is_read_and_kept_beside_what_else_the_file_holds() {
        let dir = std::env::temp_dir().join(format!("cox-settings-{}", std::process::id()));
        let file = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Saved::read_from(&file), Saved::default());

        keep_home_in(&file, Path::new("/work/one")).unwrap();
        assert_eq!(
            Saved::read_from(&file).home,
            Some(PathBuf::from("/work/one"))
        );

        std::fs::write(&file, r#"{"home": "/work/one", "later": 1}"#).unwrap();
        keep_home_in(&file, Path::new("/work/two")).unwrap();
        let kept: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(kept["home"], "/work/two");
        assert_eq!(kept["later"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_home_that_is_not_a_path_is_not_read() {
        assert!(Saved::parse(br#"{"home": 3}"#).is_err());
        assert_eq!(Saved::parse(b"{}").unwrap(), Saved::default());
    }

    #[test]
    fn the_workflow_is_a_folder_named_whole_or_from_the_home_directory() {
        let saved = Saved::parse(br#"{"workflow": "/srv/cox"}"#).unwrap();
        assert_eq!(saved.workflow, Some(PathBuf::from("/srv/cox")));
        if let Some(user) = std::env::var_os("HOME") {
            let saved = Saved::parse(br#"{"workflow": "~/dotfiles/cox"}"#).unwrap();
            assert_eq!(
                saved.workflow,
                Some(PathBuf::from(user).join("dotfiles/cox"))
            );
        }
        let saved = Saved::parse(br#"{"home": "/work", "workflow": "dotfiles"}"#).unwrap();
        assert_eq!(saved.home, Some(PathBuf::from("/work")));
        assert_eq!(saved.workflow, None);
        assert_eq!(Saved::parse(br#"{"workflow": 3}"#).unwrap().workflow, None);
    }

    #[test]
    fn the_agent_is_named() {
        let saved = Saved::parse(br#"{"agent": " Claude "}"#).unwrap();
        assert_eq!(saved.agent.as_deref(), Some("Claude"));
        for bytes in [
            br#"{"home": "/work", "agent": " "}"#.as_slice(),
            br#"{"home": "/work", "agent": ["npx", "agent"]}"#,
        ] {
            let saved = Saved::parse(bytes).unwrap();
            assert_eq!(saved.agent, None);
            assert_eq!(saved.home, Some(PathBuf::from("/work")));
        }
    }

    #[test]
    fn the_wiki_is_a_folder_as_the_workflow_is() {
        let saved = Saved::parse(br#"{"wiki": "/srv/notes", "workflow": "/srv/cox"}"#).unwrap();
        assert_eq!(saved.wiki, Some(PathBuf::from("/srv/notes")));
        assert_eq!(saved.workflow, Some(PathBuf::from("/srv/cox")));
        assert_eq!(Saved::parse(br#"{"wiki": "notes"}"#).unwrap().wiki, None);
    }

    #[test]
    fn the_wiki_is_read_in_a_language_by_a_command_or_an_agent() {
        let saved = Saved::parse(
            br#"{"translation": {"language": "Japanese",
                "command": ["plamo-translate", "--to", "{to}"]}}"#,
        )
        .unwrap();
        assert_eq!(
            saved.translation,
            Some(Translation {
                language: "Japanese".to_owned(),
                command: Some(vec![
                    "plamo-translate".to_owned(),
                    "--to".to_owned(),
                    "{to}".to_owned()
                ]),
                agent: None,
            })
        );
        let saved = Saved::parse(br#"{"translation": {"language": "Japanese"}}"#).unwrap();
        assert_eq!(saved.translation.clone().unwrap().command, None);
        assert_eq!(saved.translation.unwrap().agent, None);
        let saved = Saved::parse(br#"{"translation": {"language": "Japanese", "agent": "Codex"}}"#)
            .unwrap();
        assert_eq!(saved.translation.unwrap().agent.as_deref(), Some("Codex"));
        // What is wrong with it is said, and the rest of the file counts.
        for wrong in [
            br#"{"home": "/work", "translation": {}}"#.as_slice(),
            br#"{"home": "/work", "translation": {"language": " "}}"#,
            br#"{"home": "/work", "translation": {"language": "ja", "command": []}}"#,
            br#"{"home": "/work", "translation": {"language": "ja", "command": "tr"}}"#,
            br#"{"home": "/work", "translation": "Japanese"}"#,
            br#"{"home": "/work", "translation": {"language": "ja", "agent": 1}}"#,
        ] {
            let saved = Saved::parse(wrong).unwrap();
            assert_eq!(saved.translation, None);
            assert_eq!(saved.home, Some(PathBuf::from("/work")));
        }
    }

    #[test]
    fn a_folder_is_typed_whole_or_from_the_home_directory() {
        let user = Path::new("/home/me");
        assert_eq!(folder_of("~", Some(user)), Ok(PathBuf::from("/home/me")));
        assert_eq!(
            folder_of(" ~/Work ", Some(user)),
            Ok(PathBuf::from("/home/me/Work"))
        );
        assert_eq!(
            folder_of("/srv/cox", Some(user)),
            Ok(PathBuf::from("/srv/cox"))
        );
        assert!(folder_of("Work", Some(user)).is_err());
        assert!(folder_of("~other/Work", Some(user)).is_err());
        assert!(folder_of("", Some(user)).is_err());
        assert!(folder_of("~/Work", None).is_err());
    }
}
