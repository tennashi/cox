//! What the human sets of Cox itself, on its settings screen (see
//! [`view`]), which lasts across runs in `settings.json`, where Cox keeps
//! what lasts (see [`crate::storage`]), as `{"home": "/home/me/Cox"}`.
//!
//! For now that is Cox's home: the folder chats in no project work in,
//! which the rail shows as `~`. `--home` on the command line goes ahead
//! of it for the run, and without either, the home is `~/Cox`.

pub mod view;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde_json::{Map, Value};

/// What the file is called, where Cox keeps what lasts.
const FILE: &str = "settings.json";

/// The settings as `settings.json` keeps them.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Saved {
    /// Where chats in no project work, if the human chose it.
    pub home: Option<PathBuf>,
}

impl Saved {
    /// The settings this Cox keeps, or none when there are none, or they
    /// cannot be read, which is said.
    #[must_use]
    pub fn read() -> Self {
        crate::storage::path(FILE).map_or_else(Self::default, |file| Self::read_from(&file))
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
        Ok(Self { home })
    }
}

/// Keeps `home` as where chats in no project work, in this Cox's
/// `settings.json`.
///
/// # Errors
///
/// When Cox has no place to keep it, the path is not UTF-8, or the file
/// cannot be written.
pub fn keep_home(home: &Path) -> anyhow::Result<()> {
    let file = crate::storage::path(FILE).context("there is no place to keep the settings")?;
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

    use super::{Saved, folder_of, keep_home_in};

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
