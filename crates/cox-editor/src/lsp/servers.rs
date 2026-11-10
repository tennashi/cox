//! The language servers Cox knows, and which knows a language: those built
//! into Cox, in `servers.json` beside this file, under those the human
//! names in a `servers.json` of their own, under those a project names in
//! its own `servers.json`, all written the same way.
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
//! A project's server of a language goes before the human's, and theirs
//! before Cox's. A server of Cox's the project names is the project's,
//! with what it gives in place of Cox's, as `rust-analyzer` above, which
//! checks the code as it is saved; named as `null`, as `gopls` above, it
//! is not started at all. A server of the human's goes the same way.
//!
//! A server may also say where its program is downloaded from, at which
//! version, for each platform, as Cox names them (`darwin-aarch64`,
//! `linux-x86_64`, ...): the archive, its SHA-256, and where the program
//! is in it:
//!
//! ```json
//! "rust-analyzer": {
//!   "download": {
//!     "version": "2026-11-09",
//!     "platforms": {
//!       "darwin-aarch64": {
//!         "archive": "https://github.com/rust-lang/rust-analyzer/releases/download/2026-11-09/rust-analyzer-aarch64-apple-darwin.gz",
//!         "sha256": "e43a6500...",
//!         "program": "rust-analyzer-aarch64-apple-darwin"
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! The program then runs from where it was downloaded, in place of the
//! command's own, found on `PATH`, which still runs on a platform the
//! download does not name. A project that gives a server of Cox's a
//! `command` and no `download` runs its command, not what Cox downloads.
//!
//! Cox downloads its `rust-analyzer` from its releases on GitHub, which
//! run on their own, unlike that of `rustup`, which needs the rest of the
//! toolchain. It builds what it needs of the project, such as its
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
    /// Where its program is downloaded from, if it is.
    pub download: Option<Download>,
}

/// Where a server's program is downloaded from, at one version.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Download {
    /// The version, which the folder it is kept in is named after, so that
    /// each version is kept apart from the others.
    pub version: String,
    /// The archive of each platform, as Cox names them, such as
    /// `darwin-aarch64`.
    pub platforms: BTreeMap<String, Archive>,
}

/// The archive of a server's program, for one platform.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    /// Its URL.
    pub archive: String,
    /// Its SHA-256, in hexadecimal.
    pub sha256: String,
    /// The program, by its path in the archive.
    pub program: String,
}

impl Server {
    /// Whether the server knows `language`, as the protocol names it.
    #[must_use]
    pub fn knows(&self, language: &str) -> bool {
        self.languages.iter().any(|known| known == language)
    }

    /// Where its program is downloaded from on `platform`, as Cox names
    /// them, such as `darwin-aarch64`, if it is: the version, and the
    /// archive.
    #[must_use]
    pub fn download_for(&self, platform: &str) -> Option<(&str, &Archive)> {
        let download = self.download.as_ref()?;
        let archive = download.platforms.get(platform)?;
        Some((&download.version, archive))
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
    download: Option<Download>,
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

/// The file of a project, in its folder, that names servers of its own.
const PROJECT_SERVERS: &str = "servers.json";

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

    /// The servers of the project in `root`: those its `servers.json`
    /// names, over those the human's own, `user`, names, if given, over
    /// Cox's; and why each file that cannot be read, or does not say
    /// servers as it is to, is left out.
    #[must_use]
    pub fn of_project(user: Option<&Path>, root: &Path) -> (Self, Vec<String>) {
        let project = root.join(PROJECT_SERVERS);
        Self::of_files(user.into_iter().chain([project.as_path()]))
    }

    /// The servers each of `files`, of servers, names, over those of the
    /// files before it, over Cox's; and why each file that cannot be read,
    /// or does not say servers as it is to, is left out. A file that is not
    /// there names none.
    #[must_use]
    pub fn of_files<'a>(files: impl IntoIterator<Item = &'a Path>) -> (Self, Vec<String>) {
        let mut servers = Self::built_in();
        let mut left_out = Vec::new();
        for path in files {
            let read = match std::fs::read_to_string(path) {
                Ok(text) => Self::over(&servers.servers, &text),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => Err(error.to_string()),
            };
            match read {
                Ok(read) => servers = read,
                Err(error) => left_out.push(format!("{}: {error}", path.display())),
            }
        }
        (servers, left_out)
    }

    /// All of them, the first of a language going before the rest.
    #[must_use]
    pub fn all(&self) -> &[Server] {
        &self.servers
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
            if let Some(download) = &entry.download {
                if download.version.is_empty() {
                    return Err(format!("the server {name} is downloaded at no version"));
                }
                if download
                    .platforms
                    .values()
                    .any(|archive| archive.program.is_empty())
                {
                    return Err(format!("the server {name} is downloaded with no program"));
                }
            }
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
                // A command of the project's own is run as it is.
                download: match (&entry.download, &entry.command) {
                    (Some(download), _) => Some(download.clone()),
                    (None, Some(_)) => None,
                    (None, None) => base.and_then(|base| base.download.clone()),
                },
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
    fn downloads_coxs_rust_analyzer_on_each_platform_it_names() {
        let servers = Servers::built_in();
        let rust = servers.of("rust").unwrap();
        let (version, archive) = rust.download_for("darwin-aarch64").unwrap();
        assert!(archive.archive.contains(version));
        assert_eq!(archive.program, "rust-analyzer-aarch64-apple-darwin");
        assert_eq!(archive.sha256.len(), 64);
        assert!(rust.download_for("windows-x86_64").is_none());
        assert!(servers.of("go").unwrap().download.is_none());
    }

    #[test]
    fn a_project_downloads_a_version_of_its_own_or_runs_its_command() {
        let over = |text: &str| Servers::over(&Servers::built_in().servers, text).unwrap();
        let downloads = over(
            r#"{ "servers": { "rust-analyzer": { "download": {
                "version": "1.90.0",
                "platforms": { "linux-x86_64": {
                    "archive": "https://example.com/ra-1.90.0.tar.gz",
                    "sha256": "00",
                    "program": "ra/bin/rust-analyzer"
                } }
            } } } }"#,
        );
        let rust = downloads.of("rust").unwrap();
        let (version, archive) = rust.download_for("linux-x86_64").unwrap();
        assert_eq!(version, "1.90.0");
        assert_eq!(archive.program, "ra/bin/rust-analyzer");
        // In place of all of Cox's.
        assert!(rust.download_for("darwin-aarch64").is_none());
        assert_eq!(rust.command, ["rust-analyzer"]);
        // A command of its own runs as it is.
        let runs = over(r#"{ "servers": { "rust-analyzer": { "command": ["/opt/ra"] } } }"#);
        assert_eq!(runs.of("rust").unwrap().download, None);
        // What leaves the command be leaves Cox's download be.
        let keeps = over(r#"{ "servers": { "rust-analyzer": { "settings": {} } } }"#);
        assert!(keeps.of("rust").unwrap().download.is_some());
    }

    #[test]
    fn a_download_needs_a_version_and_a_program() {
        let fails = |download: &str| {
            Servers::over(
                &[],
                &format!(
                    r#"{{ "servers": {{ "zls": {{ "command": ["zls"], "languages": ["zig"],
                        "download": {download} }} }} }}"#
                ),
            )
            .unwrap_err()
        };
        assert_eq!(
            fails(r#"{ "version": "", "platforms": {} }"#),
            "the server zls is downloaded at no version"
        );
        assert_eq!(
            fails(
                r#"{ "version": "0.15", "platforms": { "linux-x86_64":
                    { "archive": "https://example.com/zls.tar.gz", "sha256": "00", "program": "" } } }"#
            ),
            "the server zls is downloaded with no program"
        );
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
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            Servers::of_project(None, &root),
            (Servers::built_in(), Vec::new())
        );
        std::fs::write(root.join(PROJECT_SERVERS), "{").unwrap();
        let (servers, left_out) = Servers::of_project(None, &root);
        assert_eq!(servers, Servers::built_in());
        assert!(left_out[0].contains("servers.json"), "{left_out:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_project_names_servers_over_the_humans_over_coxs() {
        let root = std::env::temp_dir().join(format!("cox-servers-user-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let user = root.join("user.json");
        std::fs::write(
            &user,
            r#"{ "servers": {
                "gopls": { "command": ["go", "run", "golang.org/x/tools/gopls@v0.20.0"] },
                "zls": { "command": ["zls"], "languages": ["zig"] },
                "pyright": null
            } }"#,
        )
        .unwrap();
        std::fs::write(
            root.join(PROJECT_SERVERS),
            r#"{ "servers": { "zls": { "command": ["/opt/zls"] } } }"#,
        )
        .unwrap();
        let (servers, left_out) = Servers::of_project(Some(&user), &root);
        assert!(left_out.is_empty(), "{left_out:?}");
        assert_eq!(servers.of("go").unwrap().command[0], "go");
        assert_eq!(servers.of("zig").unwrap().command, ["/opt/zls"]);
        assert_eq!(servers.of("python"), None);
        assert!(servers.of("rust").is_some());
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
