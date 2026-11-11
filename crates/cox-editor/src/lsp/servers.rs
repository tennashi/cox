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
//! A server may also say where Cox gets its program from, its `source`,
//! and at which `version`:
//!
//! ```json
//! "rust-analyzer": {
//!   "source": {
//!     "github": "rust-lang/rust-analyzer",
//!     "asset": "rust-analyzer-{target}.gz",
//!     "program": "rust-analyzer-{target}"
//!   },
//!   "version": "2026-11-09"
//! },
//! "gopls": { "source": { "go": "golang.org/x/tools/gopls" }, "version": "v0.24.0" }
//! ```
//!
//! From GitHub, Cox downloads the `asset` of the release tagged with the
//! version, and runs the `program` in it; in both, `{version}` stands for
//! the version, `{target}` for the platform as Rust names it
//! (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, ...), and `{os}`
//! and `{arch}` for it as Cox does (`darwin`, `linux`; `aarch64`,
//! `x86_64`). With Go, Cox installs the package at the version, as `go
//! install` does. The program then runs in place of the command's own,
//! found on `PATH`, which still runs where the source has no program for
//! the platform. A file that gives a server of Cox's a `command` and no
//! `source` runs its command, not what Cox gets.
//!
//! Where a server is got from is Cox's to say, and the human's: a
//! project's `servers.json` names no `source`, only the `version` of one
//! that Cox, or the human, gets from somewhere. Each is said again by the
//! environment, over all the files: `COX_SERVER_RUST_ANALYZER_VERSION`
//! the version of `rust-analyzer`, and `COX_SERVER_RUST_ANALYZER_SOURCE`
//! its source, as JSON, the server's name upper-cased with `_` for what is
//! not a letter or a digit.
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
    /// Where Cox gets its program from, if it does.
    pub source: Option<Source>,
    /// The version of its program Cox gets, if it does.
    pub version: Option<String>,
}

/// Where Cox gets a server's program from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A release on GitHub.
    GitHub {
        /// The repository, as `rust-lang/rust-analyzer`.
        repository: String,
        /// The release's asset to download, by its name.
        asset: String,
        /// The program, by its path in the asset once unpacked.
        program: String,
    },
    /// A Go package, installed as `go install` does.
    Go {
        /// The package, as `golang.org/x/tools/gopls`.
        package: String,
    },
}

/// A source as a file of servers writes it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceEntry {
    github: Option<String>,
    asset: Option<String>,
    program: Option<String>,
    go: Option<String>,
}

impl SourceEntry {
    /// The source `self` writes, for the server `name`.
    fn source(self, name: &str) -> Result<Source, String> {
        let given = |what: &str, field: Option<String>| {
            field
                .filter(|given| !given.is_empty())
                .ok_or_else(|| format!("the server {name} is got from GitHub with no {what}"))
        };
        match (self.github, self.go) {
            (Some(repository), None) => Ok(Source::GitHub {
                repository: given("repository", Some(repository))?,
                asset: given("asset", self.asset)?,
                program: given("program", self.program)?,
            }),
            (None, Some(package)) if self.asset.is_none() && self.program.is_none() => {
                if package.is_empty() {
                    return Err(format!("the server {name} is got from Go with no package"));
                }
                Ok(Source::Go { package })
            }
            _ => Err(format!(
                "the source of the server {name} is to be either github, with its asset and \
                 program, or go"
            )),
        }
    }
}

/// The platform `platform`, as Cox names them (`darwin-aarch64`), as Rust
/// names it (`aarch64-apple-darwin`), if Cox knows it.
fn target(platform: &str) -> Option<&'static str> {
    Some(match platform {
        "darwin-aarch64" => "aarch64-apple-darwin",
        "darwin-x86_64" => "x86_64-apple-darwin",
        "linux-aarch64" => "aarch64-unknown-linux-gnu",
        "linux-x86_64" => "x86_64-unknown-linux-gnu",
        _ => return None,
    })
}

/// `template` with `{version}`, `{target}`, `{os}` and `{arch}` in it
/// standing for `version` and `platform`, if Cox knows what each it has
/// stands for.
fn filled(template: &str, version: &str, platform: &str) -> Option<String> {
    let (os, arch) = platform.split_once('-')?;
    let mut filled = template
        .replace("{version}", version)
        .replace("{os}", os)
        .replace("{arch}", arch);
    if filled.contains("{target}") {
        filled = filled.replace("{target}", target(platform)?);
    }
    Some(filled)
}

/// What Cox gets of a server for one platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Got<'a> {
    /// From GitHub: the asset of the release `version` of `repository`,
    /// and the program in it.
    GitHub {
        repository: &'a str,
        version: &'a str,
        asset: String,
        program: String,
    },
    /// The Go package `package` at `version`, installed.
    Go {
        package: &'a str,
        version: &'a str,
        /// The program `go install` makes of it.
        program: String,
    },
}

impl Got<'_> {
    /// The version Cox gets.
    #[must_use]
    pub fn version(&self) -> &str {
        match self {
            Self::GitHub { version, .. } | Self::Go { version, .. } => version,
        }
    }
}

/// The program `go install` makes of `package`: its last element, or the
/// one before it when that is a major version, as `v2`.
fn go_program(package: &str) -> String {
    let mut elements = package.rsplit('/');
    let last = elements.next().unwrap_or(package);
    let major = last.len() > 1
        && last.starts_with('v')
        && last[1..].bytes().all(|byte| byte.is_ascii_digit());
    if major {
        elements.next().unwrap_or(last).to_owned()
    } else {
        last.to_owned()
    }
}

impl Server {
    /// Whether the server knows `language`, as the protocol names it.
    #[must_use]
    pub fn knows(&self, language: &str) -> bool {
        self.languages.iter().any(|known| known == language)
    }

    /// What Cox gets of the server on `platform`, as Cox names them, such
    /// as `darwin-aarch64`, if it gets anything there.
    #[must_use]
    pub fn got_on(&self, platform: &str) -> Option<Got<'_>> {
        let version = self.version.as_deref()?;
        match self.source.as_ref()? {
            Source::GitHub {
                repository,
                asset,
                program,
            } => Some(Got::GitHub {
                repository,
                version,
                asset: filled(asset, version, platform)?,
                program: filled(program, version, platform)?,
            }),
            Source::Go { package } => Some(Got::Go {
                package,
                version,
                program: go_program(package),
            }),
        }
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
    source: Option<SourceEntry>,
    version: Option<String>,
}

/// A file of servers: each by its name, or `null` for none.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    servers: BTreeMap<String, Option<Entry>>,
}

/// Who wrote a file of servers, which says what it may say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Writer {
    /// Cox, or the human: anything.
    Trusted,
    /// A project: anything but where a server is got from.
    Project,
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
    Servers::over(&[], include_str!("servers.json"), Writer::Trusted)
        .unwrap_or_else(|error| panic!("Cox's servers.json: {error}"))
});

/// The variable of the environment that says `what` of the server `name`,
/// as `COX_SERVER_RUST_ANALYZER_VERSION`.
#[must_use]
pub fn variable(name: &str, what: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("COX_SERVER_{name}_{what}")
}

impl Servers {
    /// Cox's own servers.
    #[must_use]
    pub fn built_in() -> Self {
        BUILT_IN.clone()
    }

    /// The servers of the project in `root`: those its `servers.json`
    /// names, over those the human's own, `user`, names, if given, over
    /// Cox's, as the environment says again; and why each file, or
    /// variable, that cannot be read, or does not say servers as it is
    /// to, is left out.
    #[must_use]
    pub fn of_project(user: Option<&Path>, root: &Path) -> (Self, Vec<String>) {
        let project = root.join(PROJECT_SERVERS);
        Self::layered(
            user.map(|user| (user, Writer::Trusted))
                .into_iter()
                .chain([(project.as_path(), Writer::Project)]),
            &|variable| std::env::var(variable).ok(),
        )
    }

    /// The servers each of `files`, of servers the human writes, names,
    /// over those of the files before it, over Cox's, as the environment
    /// says again; and why each file, or variable, that cannot be read, or
    /// does not say servers as it is to, is left out. A file that is not
    /// there names none.
    #[must_use]
    pub fn of_files<'a>(files: impl IntoIterator<Item = &'a Path>) -> (Self, Vec<String>) {
        Self::layered(
            files.into_iter().map(|file| (file, Writer::Trusted)),
            &|variable| std::env::var(variable).ok(),
        )
    }

    /// The servers each of `files` names, as its writer may, over those of
    /// the files before it, over Cox's, as `environment`, which answers
    /// a variable's value, says again.
    fn layered<'a>(
        files: impl IntoIterator<Item = (&'a Path, Writer)>,
        environment: &dyn Fn(&str) -> Option<String>,
    ) -> (Self, Vec<String>) {
        let mut servers = Self::built_in();
        let mut left_out = Vec::new();
        for (path, writer) in files {
            let read = match std::fs::read_to_string(path) {
                Ok(text) => Self::over(&servers.servers, &text, writer),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => Err(error.to_string()),
            };
            match read {
                Ok(read) => servers = read,
                Err(error) => left_out.push(format!("{}: {error}", path.display())),
            }
        }
        left_out.extend(servers.said_again(environment));
        (servers, left_out)
    }

    /// The version and source of each server as `environment` says them
    /// again, if it does; and why each variable that does not say a source
    /// as it is to is left out.
    fn said_again(&mut self, environment: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
        let mut left_out = Vec::new();
        for server in &mut self.servers {
            let source = variable(&server.name, "SOURCE");
            if let Some(text) = environment(&source).filter(|text| !text.is_empty()) {
                match serde_json::from_str::<SourceEntry>(&text)
                    .map_err(|error| error.to_string())
                    .and_then(|entry| entry.source(&server.name))
                {
                    Ok(given) => server.source = Some(given),
                    Err(error) => left_out.push(format!("{source}: {error}")),
                }
            }
            if let Some(version) =
                environment(&variable(&server.name, "VERSION")).filter(|text| !text.is_empty())
            {
                server.version = Some(version);
            }
        }
        left_out
    }

    /// All of them, the first of a language going before the rest.
    #[must_use]
    pub fn all(&self) -> &[Server] {
        &self.servers
    }

    /// The servers `text`, a file of servers `writer` wrote, names, ahead
    /// of those of `under` it leaves be.
    fn over(under: &[Server], text: &str, writer: Writer) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|error| error.to_string())?;
        let named: Vec<String> = file.servers.keys().cloned().collect();
        let mut servers = Vec::new();
        for (name, entry) in file.servers {
            let Some(entry) = entry else {
                continue;
            };
            let base = under.iter().find(|server| server.name == name);
            let given =
                |what: &str, field: &Option<Vec<String>>, of_base: fn(&Server) -> &Vec<String>| {
                    field
                        .clone()
                        .or_else(|| base.map(|base| of_base(base).clone()))
                        .filter(|given| !given.is_empty())
                        .ok_or_else(|| format!("the server {name} has no {what}"))
                };
            if entry.version.as_deref() == Some("") {
                return Err(format!("the server {name} is got at no version"));
            }
            let source = match entry.source {
                Some(_) if writer == Writer::Project => {
                    return Err(format!(
                        "a project does not say where the server {name} is got from, only its \
                         version"
                    ));
                }
                Some(source) => Some(source.source(&name)?),
                // A command of the file's own is run as it is.
                None if entry.command.is_some() => None,
                None => base.and_then(|base| base.source.clone()),
            };
            servers.push(Server {
                command: given("command", &entry.command, |base| &base.command)?,
                languages: given("languages", &entry.languages, |base| &base.languages)?,
                markers: entry
                    .markers
                    .or_else(|| base.map(|base| base.markers.clone()))
                    .unwrap_or_default(),
                initialization_options: entry
                    .initialization_options
                    .or_else(|| base.map(|base| base.initialization_options.clone()))
                    .unwrap_or(Value::Null),
                settings: entry
                    .settings
                    .or_else(|| base.map(|base| base.settings.clone()))
                    .unwrap_or(Value::Null),
                source,
                version: entry
                    .version
                    .or_else(|| base.and_then(|base| base.version.clone())),
                name,
            });
        }
        servers.extend(
            under
                .iter()
                .filter(|server| !named.contains(&server.name))
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
            Writer::Project,
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
    fn gets_coxs_rust_analyzer_from_github_and_gopls_from_go() {
        let servers = Servers::built_in();
        let rust = servers.of("rust").unwrap();
        let Some(Got::GitHub {
            repository,
            version,
            asset,
            program,
        }) = rust.got_on("darwin-aarch64")
        else {
            panic!("{rust:?}");
        };
        assert_eq!(repository, "rust-lang/rust-analyzer");
        assert_eq!(version, rust.version.as_deref().unwrap());
        assert_eq!(asset, "rust-analyzer-aarch64-apple-darwin.gz");
        assert_eq!(program, "rust-analyzer-aarch64-apple-darwin");
        assert_eq!(rust.got_on("windows-x86_64"), None);
        let go = servers.of("go").unwrap();
        let Some(Got::Go {
            package, program, ..
        }) = go.got_on("windows-x86_64")
        else {
            panic!("{go:?}");
        };
        assert_eq!(package, "golang.org/x/tools/gopls");
        assert_eq!(program, "gopls");
        assert_eq!(servers.of("python").unwrap().got_on("linux-x86_64"), None);
    }

    #[test]
    fn fills_a_template_for_the_platform() {
        assert_eq!(
            filled("zls-{arch}-{os}-{version}.tar.xz", "0.15.0", "linux-x86_64"),
            Some("zls-x86_64-linux-0.15.0.tar.xz".to_owned())
        );
        assert_eq!(filled("ra-{target}.gz", "1", "freebsd-x86_64"), None);
        assert_eq!(go_program("golang.org/x/tools/gopls"), "gopls");
        assert_eq!(go_program("example.com/cmd/tool/v2"), "tool");
        assert_eq!(go_program("example.com/vet"), "vet");
    }

    #[test]
    fn a_project_says_the_version_and_not_the_source() {
        let over = |text: &str, writer| Servers::over(&Servers::built_in().servers, text, writer);
        let older = over(
            r#"{ "servers": { "rust-analyzer": { "version": "2026-10-01" } } }"#,
            Writer::Project,
        )
        .unwrap();
        let rust = older.of("rust").unwrap();
        assert_eq!(rust.version.as_deref(), Some("2026-10-01"));
        assert_eq!(rust.source, Servers::built_in().of("rust").unwrap().source);
        let elsewhere = r#"{ "servers": { "rust-analyzer": {
            "source": { "github": "someone/rust-analyzer", "asset": "ra.gz", "program": "ra" }
        } } }"#;
        assert!(
            over(elsewhere, Writer::Project)
                .unwrap_err()
                .contains("only its version")
        );
        let human = over(elsewhere, Writer::Trusted).unwrap();
        assert!(matches!(
            &human.of("rust").unwrap().source,
            Some(Source::GitHub { repository, .. }) if repository == "someone/rust-analyzer"
        ));
        // A command of its own runs as it is.
        let runs = over(
            r#"{ "servers": { "rust-analyzer": { "command": ["/opt/ra"] } } }"#,
            Writer::Project,
        )
        .unwrap();
        assert_eq!(runs.of("rust").unwrap().got_on("linux-x86_64"), None);
        // What leaves the command be leaves Cox's source be.
        let keeps = over(
            r#"{ "servers": { "rust-analyzer": { "settings": {} } } }"#,
            Writer::Project,
        )
        .unwrap();
        assert!(keeps.of("rust").unwrap().source.is_some());
    }

    #[test]
    fn a_source_is_github_with_its_asset_and_program_or_go() {
        let fails = |source: &str| {
            Servers::over(
                &[],
                &format!(
                    r#"{{ "servers": {{ "zls": {{ "command": ["zls"], "languages": ["zig"],
                        "source": {source}, "version": "0.15.0" }} }} }}"#
                ),
                Writer::Trusted,
            )
            .unwrap_err()
        };
        assert_eq!(
            fails(r#"{ "github": "zigtools/zls", "asset": "zls.tar.xz" }"#),
            "the server zls is got from GitHub with no program"
        );
        assert!(fails(r#"{ "go": "example.com/zls", "asset": "x" }"#).contains("either github"));
        assert!(fails("{}").contains("either github"));
        assert!(fails(r#"{ "gitlab": "zigtools/zls" }"#).contains("gitlab"));
        let no_version = Servers::over(
            &[],
            r#"{ "servers": { "zls": { "command": ["zls"], "languages": ["zig"], "version": "" } } }"#,
            Writer::Trusted,
        );
        assert_eq!(
            no_version.unwrap_err(),
            "the server zls is got at no version"
        );
    }

    #[test]
    fn the_environment_says_the_version_and_source_again() {
        assert_eq!(
            variable("rust-analyzer", "VERSION"),
            "COX_SERVER_RUST_ANALYZER_VERSION"
        );
        let environment = |variable: &str| match variable {
            "COX_SERVER_RUST_ANALYZER_VERSION" => Some("2026-09-01".to_owned()),
            "COX_SERVER_GOPLS_SOURCE" => Some(r#"{ "go": "example.com/gopls" }"#.to_owned()),
            "COX_SERVER_PYRIGHT_SOURCE" => Some(r#"{ "go": "" }"#.to_owned()),
            _ => None,
        };
        let (servers, left_out) = Servers::layered([], &environment);
        assert_eq!(
            servers.of("rust").unwrap().version.as_deref(),
            Some("2026-09-01")
        );
        assert_eq!(
            servers.of("go").unwrap().source,
            Some(Source::Go {
                package: "example.com/gopls".to_owned()
            })
        );
        assert_eq!(servers.of("python").unwrap().source, None);
        assert_eq!(
            left_out,
            ["COX_SERVER_PYRIGHT_SOURCE: the server pyright is got from Go with no package"]
        );
    }

    #[test]
    fn a_server_of_its_own_needs_a_command_and_languages() {
        let fails = |text: &str| Servers::over(&[], text, Writer::Trusted).unwrap_err();
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
