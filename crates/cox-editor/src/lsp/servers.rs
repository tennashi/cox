//! The language servers Cox knows, and which knows a language: those built
//! into Cox, in `servers.json` beside this file, under those a project
//! names in its own `.cox/servers.json`, written the same way.
//!
//! The file names each server, with the command that starts it, the
//! languages it knows, as the protocol names them (see
//! [`super::languages`]), the files a folder that is a project of them
//! has, the options it is started with, and its settings:
//!
//! ```json
//! {
//!   "servers": {
//!     "pyright": {
//!       "command": ["pyright-langserver", "--stdio"],
//!       "languages": ["python"],
//!       "markers": ["pyproject.toml"],
//!       "initializationOptions": {},
//!       "settings": { "python": { "analysis": { "typeCheckingMode": "basic" } } }
//!     },
//!     "rust-analyzer": { "settings": { "rust-analyzer": { "checkOnSave": true } } },
//!     "gopls": null
//!   }
//! }
//! ```
//!
//! A project's server of a language goes before Cox's. A server of Cox's
//! the project names is the project's, with what it gives in place of
//! Cox's, as `rust-analyzer` above, which checks the code as it is saved;
//! named as `null`, as `gopls` above, it is not started at all.
//!
//! Cox's `rust-analyzer` builds what it needs of the project, such as its
//! procedural macros, in a directory of its own, `target/rust-analyzer`,
//! so that it does not hold up the builds of the project while it does,
//! and does not check the project as it is saved, which only finds
//! errors, which the editor does not show yet.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::Value;

/// A language server, as Cox starts it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Server {
    /// What the server goes by, as its file names it.
    pub name: String,
    /// The program, found on `PATH`, and its arguments.
    pub command: Vec<String>,
    /// The languages it knows, as the protocol names them.
    pub languages: Vec<String>,
    /// The files a folder that is a project of its languages has, such
    /// as `Cargo.toml` for Rust.
    pub markers: Vec<String>,
    /// What it is told when it starts, as its `initializationOptions`.
    pub initialization_options: Value,
    /// Its settings, by section, as it asks for them
    /// (`workspace/configuration`).
    pub settings: Value,
}

impl Server {
    /// Whether the server knows `language`, as the protocol names it.
    #[must_use]
    pub fn knows(&self, language: &str) -> bool {
        self.languages.iter().any(|known| known == language)
    }

    /// The settings of `section`, as `rust-analyzer` or `python.analysis`,
    /// or all of them for none, as the server asks for them: null for
    /// settings the server was not given.
    #[must_use]
    pub fn settings_of(&self, section: Option<&str>) -> Value {
        let Some(section) = section.filter(|section| !section.is_empty()) else {
            return self.settings.clone();
        };
        section
            .split('.')
            .try_fold(&self.settings, |settings, key| settings.get(key))
            .cloned()
            .unwrap_or(Value::Null)
    }
}

/// A server as a file of servers names it: all of it, for a server of its
/// own, or what it changes of one of Cox's.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Entry {
    command: Option<Vec<String>>,
    languages: Option<Vec<String>>,
    markers: Option<Vec<String>>,
    initialization_options: Option<Value>,
    settings: Option<Value>,
}

/// A file of servers: each by its name, or `null` for none.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    servers: BTreeMap<String, Option<Entry>>,
}

/// The servers Cox knows, the first of a language going before the rest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Servers {
    servers: Vec<Server>,
}

/// Cox's own servers.
static BUILT_IN: LazyLock<Servers> = LazyLock::new(|| {
    Servers::over(&[], include_str!("servers.json"))
        .unwrap_or_else(|error| panic!("Cox's servers.json: {error}"))
});

impl Servers {
    /// Cox's own servers.
    #[must_use]
    pub fn built_in() -> Self {
        BUILT_IN.clone()
    }

    /// The servers of the project in `root`: those its
    /// `.cox/servers.json` names, over Cox's.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, or does not say servers as it is to,
    /// saying why.
    pub fn of_project(root: &Path) -> Result<Self, String> {
        let path = root.join(".cox/servers.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::over(&BUILT_IN.servers, &text)
                .map_err(|error| format!("{}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::built_in()),
            Err(error) => Err(format!("{}: {error}", path.display())),
        }
    }

    /// The servers `text`, a file of servers, names, ahead of those of
    /// `under` it leaves be.
    fn over(under: &[Server], text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let mut servers = Vec::new();
        for (name, entry) in &file.servers {
            let Some(entry) = entry else {
                continue;
            };
            let base = under.iter().find(|server| server.name == *name);
            let given =
                |what: &str, field: &Option<Vec<String>>, of_base: fn(&Server) -> &Vec<String>| {
                    field
                        .clone()
                        .or_else(|| base.map(|base| of_base(base).clone()))
                        .filter(|given| !given.is_empty())
                        .ok_or_else(|| format!("the server {name} has no {what}"))
                };
            servers.push(Server {
                name: name.clone(),
                command: given("command", &entry.command, |base| &base.command)?,
                languages: given("languages", &entry.languages, |base| &base.languages)?,
                markers: entry
                    .markers
                    .clone()
                    .or_else(|| base.map(|base| base.markers.clone()))
                    .unwrap_or_default(),
                initialization_options: entry
                    .initialization_options
                    .clone()
                    .or_else(|| base.map(|base| base.initialization_options.clone()))
                    .unwrap_or(Value::Null),
                settings: entry
                    .settings
                    .clone()
                    .or_else(|| base.map(|base| base.settings.clone()))
                    .unwrap_or(Value::Null),
            });
        }
        servers.extend(
            under
                .iter()
                .filter(|server| !file.servers.contains_key(&server.name))
                .cloned(),
        );
        Ok(Self { servers })
    }

    /// The server that knows `language`, as the protocol names it, if one
    /// does.
    #[must_use]
    pub fn of(&self, language: &str) -> Option<&Server> {
        self.servers.iter().find(|server| server.knows(language))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn knows_a_server_for_each_of_coxs_languages() {
        let servers = Servers::built_in();
        let name = |language: &str| servers.of(language).map(|server| server.name.as_str());
        assert_eq!(name("rust"), Some("rust-analyzer"));
        assert_eq!(name("python"), Some("pyright"));
        assert_eq!(name("typescriptreact"), Some("typescript-language-server"));
        assert_eq!(name("go"), Some("gopls"));
        assert_eq!(name("toml"), None);
        let rust = servers.of("rust").unwrap();
        assert_eq!(rust.command, ["rust-analyzer"]);
        assert_eq!(rust.markers, ["Cargo.toml"]);
        assert_eq!(rust.initialization_options["checkOnSave"], false);
    }

    #[test]
    fn a_project_names_servers_over_coxs() {
        let servers = Servers::over(
            &Servers::built_in().servers,
            r#"{ "servers": {
                "pylsp": { "command": ["pylsp"], "languages": ["python"] },
                "rust-analyzer": { "settings": { "rust-analyzer": { "checkOnSave": true } } },
                "gopls": null
            } }"#,
        )
        .unwrap();
        // Its own goes before Cox's of the same language.
        assert_eq!(servers.of("python").unwrap().command, ["pylsp"]);
        // What it gives of one of Cox's takes the place of Cox's, and the
        // rest stays.
        let rust = servers.of("rust").unwrap();
        assert_eq!(rust.command, ["rust-analyzer"]);
        assert_eq!(
            rust.settings_of(Some("rust-analyzer")),
            json!({ "checkOnSave": true })
        );
        assert_eq!(servers.of("go"), None);
        assert!(servers.of("typescript").is_some());
    }

    #[test]
    fn a_server_of_its_own_needs_a_command_and_languages() {
        let fails = |text: &str| Servers::over(&[], text).unwrap_err();
        assert_eq!(
            fails(r#"{ "servers": { "zls": { "languages": ["zig"] } } }"#),
            "the server zls has no command"
        );
        assert_eq!(
            fails(r#"{ "servers": { "zls": { "command": ["zls"], "languages": [] } } }"#),
            "the server zls has no languages"
        );
        assert!(fails(r#"{ "servers": { "zls": { "comand": ["zls"] } } }"#).contains("comand"));
    }

    #[test]
    fn a_project_with_no_file_has_coxs_servers() {
        let root = std::env::temp_dir().join(format!("cox-servers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".cox")).unwrap();
        assert_eq!(Servers::of_project(&root), Ok(Servers::built_in()));
        std::fs::write(root.join(".cox/servers.json"), "{").unwrap();
        assert!(
            Servers::of_project(&root)
                .unwrap_err()
                .contains("servers.json")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tells_the_settings_of_a_section() {
        let server = Server {
            settings: json!({ "python": { "analysis": { "typeCheckingMode": "basic" } } }),
            ..Server::default()
        };
        assert_eq!(
            server.settings_of(Some("python.analysis")),
            json!({ "typeCheckingMode": "basic" })
        );
        assert_eq!(server.settings_of(Some("python.linting")), Value::Null);
        assert_eq!(server.settings_of(None), server.settings);
    }
}
