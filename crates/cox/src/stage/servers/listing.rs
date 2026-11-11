//! The human's `servers.json` as the settings screen changes it: each
//! server's object whole, so that what the screen does not show of it, as
//! its settings, stays, written back as the human would write it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use serde_json::{Map, Value};

/// The servers the human's `servers.json` names, in its order.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Listing {
    /// Each server by its name: an object, or `null` for one of Cox's
    /// turned off.
    servers: Map<String, Value>,
}

/// What the human's `servers.json` says of a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// Not named: as Cox has it, if Cox has it.
    No,
    /// Named as `null`: not started.
    Off,
    /// Named, with its command, if it gives one.
    Given(Option<Vec<String>>),
}

impl Listing {
    /// The human's `servers.json` as it is now, with no servers when
    /// there is none.
    ///
    /// # Errors
    ///
    /// When Cox has no place for it, or it cannot be read as servers.
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
        let Value::Object(mut whole) = serde_json::from_slice(&bytes)
            .with_context(|| format!("{} is not JSON", file.display()))?
        else {
            bail!("{} is not an object of servers", file.display());
        };
        match whole.remove("servers") {
            None => Ok(Self::default()),
            Some(Value::Object(servers)) => Ok(Self { servers }),
            Some(_) => bail!("the servers of {} are not an object", file.display()),
        }
    }

    /// What the file says of the server named `name`.
    #[must_use]
    pub fn named(&self, name: &str) -> Named {
        match self.servers.get(name) {
            None => Named::No,
            Some(Value::Null) => Named::Off,
            Some(server) => Named::Given(server.get("command").and_then(Value::as_array).map(
                |words| {
                    words
                        .iter()
                        .filter_map(|word| word.as_str().map(str::to_owned))
                        .collect()
                },
            )),
        }
    }

    /// The names of the servers the file names, in its order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.servers.keys().cloned().collect()
    }

    /// Has the server named `name` run `command`, keeping what else the
    /// file gives of it, and, with `languages`, know them: one of Cox's
    /// then runs the command in place of what Cox downloads.
    pub fn set(&mut self, name: &str, command: &[String], languages: Option<&[String]>) {
        let server = self
            .servers
            .entry(name.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if !server.is_object() {
            *server = Value::Object(Map::new());
        }
        let words =
            |words: &[String]| Value::Array(words.iter().cloned().map(Value::String).collect());
        if let Value::Object(server) = server {
            server.insert("command".to_owned(), words(command));
            if let Some(languages) = languages {
                server.insert("languages".to_owned(), words(languages));
            }
        }
    }

    /// Has the server named `name`, one of Cox's, run Cox's command
    /// again, keeping what else the file gives of it, as its settings.
    pub fn unset_command(&mut self, name: &str) {
        let Some(Value::Object(server)) = self.servers.get_mut(name) else {
            return;
        };
        server.remove("command");
        if server.is_empty() {
            self.servers.shift_remove(name);
        }
    }

    /// Turns off the server named `name`, one of Cox's, naming it as
    /// `null`.
    pub fn turn_off(&mut self, name: &str) {
        self.servers.insert(name.to_owned(), Value::Null);
    }

    /// Leaves the server named `name` out of the file: one of Cox's is as
    /// Cox has it again, and one of the human's is gone.
    pub fn forget(&mut self, name: &str) {
        self.servers.shift_remove(name);
    }

    /// Keeps the servers as the human's `servers.json`.
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
        let whole = serde_json::json!({ "servers": self.servers });
        let mut text = serde_json::to_string_pretty(&whole)?;
        text.push('\n');
        fs::write(file, text).with_context(|| format!("writing {}", file.display()))
    }
}

/// Where the human's `servers.json` is.
///
/// # Errors
///
/// When Cox has no place for it.
pub fn file() -> anyhow::Result<PathBuf> {
    crate::storage::path(crate::storage::Kept::Servers)
        .context("there is no place to keep the language servers")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|&word| word.to_owned()).collect()
    }

    #[test]
    fn the_listing_changes_keeping_what_the_screen_does_not_show() {
        let folder =
            std::env::temp_dir().join(format!("cox-servers-listing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let file = folder.join("servers.json");
        fs::create_dir_all(&folder).unwrap();
        assert_eq!(Listing::read_from(&file).unwrap(), Listing::default());
        fs::write(
            &file,
            r#"{"servers": {"rust-analyzer": {"settings": {"rust-analyzer": {}}}}}"#,
        )
        .unwrap();
        let mut listing = Listing::read_from(&file).unwrap();
        assert_eq!(listing.named("rust-analyzer"), Named::Given(None));
        listing.set("rust-analyzer", &words(&["rust-analyzer"]), None);
        listing.set("zls", &words(&["zls"]), Some(&words(&["zig"])));
        listing.turn_off("gopls");
        listing.keep_in(&file).unwrap();

        let mut listing = Listing::read_from(&file).unwrap();
        assert_eq!(
            listing.named("rust-analyzer"),
            Named::Given(Some(words(&["rust-analyzer"])))
        );
        assert_eq!(
            listing.servers["rust-analyzer"]["settings"],
            serde_json::json!({"rust-analyzer": {}})
        );
        assert_eq!(
            listing.servers["zls"]["languages"],
            serde_json::json!(["zig"])
        );
        assert_eq!(listing.named("gopls"), Named::Off);
        assert_eq!(listing.named("clangd"), Named::No);
        assert_eq!(listing.names(), words(&["rust-analyzer", "zls", "gopls"]));
        listing.forget("gopls");
        assert_eq!(listing.named("gopls"), Named::No);
        listing.unset_command("rust-analyzer");
        assert_eq!(listing.named("rust-analyzer"), Named::Given(None));

        // What Cox's editor reads of it is what the listing says.
        listing.keep_in(&file).unwrap();
        let (servers, left_out) = cox_editor::lsp::Servers::of_files([file.as_path()]);
        assert!(left_out.is_empty(), "{left_out:?}");
        assert_eq!(servers.of("zig").unwrap().command, ["zls"]);
        // Cox's command again, it is downloaded as Cox has it.
        assert!(servers.of("rust").unwrap().source.is_some());

        fs::write(&file, r#"{"servers": []}"#).unwrap();
        assert!(Listing::read_from(&file).is_err());
        let _ = fs::remove_dir_all(&folder);
    }
}
