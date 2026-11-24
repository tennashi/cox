//! The indexers Cox runs to read a project's code for its map, one for
//! each language it knows: which files at the top of a repository say
//! the language is there, as `Cargo.toml` does Rust's, how the indexer
//! runs, and how the human gets what it needs when that is missing. Each
//! writes a SCIP index (see [`cox_editor::scip`]). Another language is
//! another row of [`known`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use cox_editor::lsp::servers::{Server, Servers};

use crate::agents::check;
use crate::agents::download;
use crate::storage::Kept;

/// An indexer, and what says its language is in a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indexer {
    /// What it goes by, which the indexes it writes are kept by.
    pub name: String,
    /// The language it reads, as the human calls it.
    pub language: String,
    /// The files at a repository's top, any of which says the language is
    /// there.
    pub markers: Vec<String>,
    /// The files at a repository's top, any of which says another
    /// indexer reads it instead, as `tsconfig.json` says of the code of
    /// `package.json`.
    pub unless: Vec<String>,
    pub program: Program,
    /// Its arguments, after the program's own, run in a copy of the
    /// commit: `{out}` stands for the file it writes the index to,
    /// `{config}` for the file of [`Indexer::config`], and `{name}` for
    /// the project's name (see [`project_name`]).
    pub args: Vec<String>,
    /// What `{config}` holds, if it is used.
    pub config: Option<String>,
    /// The programs it runs besides its own, which have to be on `PATH`,
    /// each with how to get it.
    pub needs: Vec<(String, String)>,
    /// What the checkout has that the commit does not, which the indexer
    /// reads in the copy when the checkout has it, as the packages
    /// `npm install` put in `node_modules`, with what the index may lack
    /// when it does not.
    pub borrows: Option<(String, String)>,
}

/// Where an indexer's program comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    /// A program on `PATH`, or at a path, as it is, as an indexer the
    /// human installs would be, such as `scip-clang`, or a test's fake
    /// one: its version is what it says to `--version`.
    Path(String),
    /// The program of the language server of that name, got as the
    /// stage's language servers get it (see [`server`]): it indexes too.
    Server(String),
    /// A package of npm, at a version, run with `npx`.
    Npx { package: String, version: String },
    /// A Go package, at a version, installed with `go install`.
    Go {
        package: String,
        version: String,
        /// Its program, once installed.
        program: String,
    },
}

/// What an indexer runs, once found, and its version, which the indexes
/// it writes are kept by, so that another version writes them again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub runs: Runs,
    pub version: String,
}

/// What an indexer runs: a command, or a program got first, when it has
/// to be downloaded or installed, which only the indexing does, as it can
/// take a while.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Runs {
    /// The program and its arguments before the indexer's.
    Command(Vec<String>),
    /// The program of the language server, got as the stage gets it.
    Server(Box<Server>),
    /// The program of the Go package, installed with `go install`.
    Go {
        name: String,
        package: String,
        version: String,
        program: String,
    },
}

/// The indexers Cox knows. Each is pinned to a version, so that the
/// indexes of a commit stay what they were, and rust-analyzer is the one
/// the stage downloads for the code it shows.
#[must_use]
#[expect(clippy::too_many_lines, reason = "a table, a row for each language")]
pub fn known() -> Vec<Indexer> {
    let strings = |items: &[&str]| items.iter().map(|&item| item.to_owned()).collect();
    let node = (
        "npx".to_owned(),
        "install Node.js, which has npx: https://nodejs.org".to_owned(),
    );
    // Not needed for the code's own types, whose map is the same without
    // them, but for what the code calls on its packages' types.
    let modules = Some((
        "node_modules".to_owned(),
        "node_modules is not installed, so what the code calls through its packages' types may \
         be left out: install them, as with `npm install`, for Cox to index the code again"
            .to_owned(),
    ));
    let typescript = Program::Npx {
        package: "@sourcegraph/scip-typescript".to_owned(),
        version: "0.6.2".to_owned(),
    };
    vec![
        Indexer {
            name: "rust-analyzer".to_owned(),
            language: "Rust".to_owned(),
            markers: strings(&["Cargo.toml"]),
            unless: Vec::new(),
            program: Program::Server("rust-analyzer".to_owned()),
            args: strings(&[
                "scip",
                ".",
                "--output",
                "{out}",
                "--config-path",
                "{config}",
            ]),
            // What runs only to build the code, build scripts and
            // procedural macros, it skips: it reads the code as written.
            config: Some(
                r#"{"cargo":{"buildScripts":{"enable":false}},"procMacro":{"enable":false}}"#
                    .to_owned(),
            ),
            needs: vec![(
                "cargo".to_owned(),
                "install Rust with rustup: https://rustup.rs".to_owned(),
            )],
            borrows: None,
        },
        Indexer {
            name: "scip-typescript".to_owned(),
            language: "TypeScript".to_owned(),
            markers: strings(&["tsconfig.json"]),
            unless: Vec::new(),
            program: typescript.clone(),
            args: strings(&["index", "--output", "{out}"]),
            config: None,
            needs: vec![node.clone()],
            borrows: modules.clone(),
        },
        Indexer {
            name: "scip-typescript-js".to_owned(),
            language: "JavaScript".to_owned(),
            markers: strings(&["package.json", "jsconfig.json"]),
            unless: strings(&["tsconfig.json"]),
            program: typescript,
            args: strings(&["index", "--infer-tsconfig", "--output", "{out}"]),
            config: None,
            needs: vec![node.clone()],
            borrows: modules,
        },
        Indexer {
            name: "scip-python".to_owned(),
            language: "Python".to_owned(),
            markers: strings(&["pyproject.toml", "setup.py", "setup.cfg"]),
            unless: Vec::new(),
            program: Program::Npx {
                package: "@sourcegraph/scip-python".to_owned(),
                version: "0.6.8".to_owned(),
            },
            // The version, which it would read from Git, is none of the
            // map's: the copy it indexes is no repository.
            args: strings(&[
                "index",
                ".",
                "--project-name={name}",
                "--project-version=0",
                "--output",
                "{out}",
            ]),
            config: None,
            needs: vec![node],
            borrows: None,
        },
        Indexer {
            name: "scip-go".to_owned(),
            language: "Go".to_owned(),
            markers: strings(&["go.mod"]),
            unless: Vec::new(),
            program: Program::Go {
                package: "github.com/scip-code/scip-go/cmd/scip-go".to_owned(),
                version: "v0.2.10".to_owned(),
                program: "scip-go".to_owned(),
            },
            args: strings(&["--output={out}"]),
            config: None,
            needs: vec![(
                "go".to_owned(),
                "install Go: https://go.dev/doc/install".to_owned(),
            )],
            borrows: None,
        },
    ]
}

/// The indexers of `indexers` that read a repository whose top holds
/// the files `top`, by their names.
#[must_use]
pub fn of<'a>(indexers: &'a [Indexer], top: &[String]) -> Vec<&'a Indexer> {
    let has = |names: &[String]| names.iter().any(|name| top.contains(name));
    indexers
        .iter()
        .filter(|indexer| has(&indexer.markers) && !has(&indexer.unless))
        .collect()
}

/// What `indexer` runs, found on `path`, or among the language servers
/// of the project in `root`, as the stage finds them, or how to get what
/// it needs, when that is not there.
///
/// # Errors
///
/// What is missing, and how to get it.
pub fn find(
    indexer: &Indexer,
    root: &Path,
    path: Option<&std::ffi::OsStr>,
) -> Result<Found, String> {
    let on_path = |program: &str| check::find(program, path, Path::new("/"));
    for (program, how) in &indexer.needs {
        if on_path(program).is_none() {
            return Err(format!("{program} is not on Cox's PATH: {how}"));
        }
    }
    match &indexer.program {
        Program::Path(program) => {
            let found = on_path(program).ok_or_else(|| {
                format!("{program} is not on Cox's PATH: install {}", indexer.name)
            })?;
            Ok(Found {
                version: version_of(&found),
                runs: Runs::Command(vec![found.to_string_lossy().into_owned()]),
            })
        }
        Program::Npx { package, version } => Ok(Found {
            runs: Runs::Command(vec![
                "npx".to_owned(),
                "-y".to_owned(),
                format!("{package}@{version}"),
            ]),
            version: version.clone(),
        }),
        Program::Go {
            package,
            version,
            program,
        } => Ok(Found {
            runs: Runs::Go {
                name: indexer.name.clone(),
                package: package.clone(),
                version: version.clone(),
                program: program.clone(),
            },
            version: version.clone(),
        }),
        Program::Server(name) => {
            let user = crate::storage::path(Kept::Servers);
            let (servers, _) = Servers::of_project(user.as_deref(), root);
            let server = servers
                .all()
                .iter()
                .find(|server| server.name == *name)
                .cloned()
                .ok_or_else(|| format!("the language server {name} is turned off"))?;
            if server.got_on(&download::platform()).is_some() {
                return Ok(Found {
                    version: server.version.clone().unwrap_or_default(),
                    runs: Runs::Server(Box::new(server)),
                });
            }
            let program = server.command.first().cloned().unwrap_or_default();
            let found = on_path(&program)
                .ok_or_else(|| format!("{program} is not on Cox's PATH: install {name}"))?;
            Ok(Found {
                version: version_of(&found),
                runs: Runs::Command(vec![found.to_string_lossy().into_owned()]),
            })
        }
    }
}

impl Found {
    /// The command to run, its program got first if it has to be.
    ///
    /// # Errors
    ///
    /// When the program cannot be downloaded or installed.
    pub fn got(&self) -> anyhow::Result<Vec<String>> {
        let told = |note: String| tracing::info!("{note}");
        let program = match &self.runs {
            Runs::Command(command) => return Ok(command.clone()),
            Runs::Server(server) => download::server_program((**server).clone())?
                .command
                .first()
                .cloned()
                .unwrap_or_default(),
            Runs::Go {
                name,
                package,
                version,
                program,
            } => download::go_installed(Kept::IndexerDownloads, name, package, version, &told)?
                .join(program)
                .to_string_lossy()
                .into_owned(),
        };
        Ok(vec![program])
    }
}

/// How long what a program said to `--version` is taken for its
/// version: a program on `PATH` may be a proxy of rustup's, as
/// `rust-analyzer` is, which runs another toolchain's once `rustup
/// update` changed it, while it stays as it was, so nothing of the file
/// tells it changed.
const VERSION_LASTS: std::time::Duration = std::time::Duration::from_mins(1);

/// The first line `program` says to `--version`, or `unknown`: asked
/// again once [`VERSION_LASTS`] passed, and not while another thread
/// waits for it.
fn version_of(program: &PathBuf) -> String {
    type Said = HashMap<PathBuf, (Instant, String)>;
    static SAID: Mutex<Option<Said>> = Mutex::new(None);
    let said = |said: &mut Option<Said>| {
        said.get_or_insert_default()
            .get(program)
            .filter(|(when, _)| when.elapsed() < VERSION_LASTS)
            .map(|(_, version)| version.clone())
    };
    if let Some(version) = said(&mut SAID.lock().unwrap_or_else(PoisonError::into_inner)) {
        return version;
    }
    let version = asked_version(program);
    SAID.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_default()
        .insert(program.clone(), (Instant::now(), version.clone()));
    version
}

/// The first line `program` says to `--version`, or `unknown`.
fn asked_version(program: &PathBuf) -> String {
    let said = crate::process::run_with_deadline(
        std::process::Command::new(program).arg("--version"),
        None,
        std::time::Duration::from_secs(10),
        |_| {},
    );
    said.ok()
        .filter(crate::process::Output::success)
        .and_then(|said| {
            String::from_utf8_lossy(&said.stdout)
                .lines()
                .next()
                .map(|line| line.trim().to_owned())
        })
        .filter(|line| !line.is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The project's name, which `{name}` stands for, as its symbols are
/// named by it, the same in every checkout: as its `pyproject.toml`
/// names it, or else the name of the folder of the repository's own
/// checkout, which its worktrees share.
#[must_use]
pub fn project_name(snapshot: &Path, checkout: &Path) -> String {
    let named = std::fs::read_to_string(snapshot.join("pyproject.toml"))
        .ok()
        .and_then(|text| named_in(&text));
    named.unwrap_or_else(|| {
        let common = cox_git::git::run(
            checkout,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .ok()
        .map(PathBuf::from);
        let own = common.as_deref().and_then(Path::parent).unwrap_or(checkout);
        own.file_name().map_or_else(
            || "project".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    })
}

/// The name in the `[project]` table of the text of a `pyproject.toml`.
fn named_in(text: &str) -> Option<String> {
    let mut in_project = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_project = line == "[project]";
            continue;
        }
        if !in_project {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "name"
        {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            return (!value.is_empty()).then(|| value.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(top: &[&str]) -> Vec<String> {
        let indexers = known();
        let top: Vec<String> = top.iter().map(|&name| name.to_owned()).collect();
        of(&indexers, &top)
            .into_iter()
            .map(|indexer| indexer.name.clone())
            .collect()
    }

    #[test]
    fn the_files_at_a_repositorys_top_say_which_indexers_read_it() {
        assert_eq!(names(&["Cargo.toml", "README.md"]), ["rust-analyzer"]);
        assert_eq!(
            names(&["package.json", "tsconfig.json"]),
            ["scip-typescript"]
        );
        assert_eq!(names(&["package.json"]), ["scip-typescript-js"]);
        assert_eq!(names(&["pyproject.toml"]), ["scip-python"]);
        assert_eq!(names(&["go.mod", "go.sum"]), ["scip-go"]);
        assert_eq!(
            names(&["Cargo.toml", "go.mod"]),
            ["rust-analyzer", "scip-go"]
        );
        assert_eq!(names(&["pom.xml"]), Vec::<String>::new());
    }

    #[test]
    fn what_an_indexer_needs_is_told_with_how_to_get_it() {
        let go = known()
            .into_iter()
            .find(|indexer| indexer.name == "scip-go");
        let missing = find(&go.unwrap(), Path::new("/"), Some("/nowhere".as_ref())).unwrap_err();
        assert!(missing.contains("go is not on Cox's PATH"), "{missing}");
        assert!(missing.contains("https://go.dev"), "{missing}");

        let typescript = known()
            .into_iter()
            .find(|indexer| indexer.name == "scip-typescript")
            .unwrap();
        let missing = find(&typescript, Path::new("/"), Some("/nowhere".as_ref())).unwrap_err();
        assert!(missing.contains("npx is not on Cox's PATH"), "{missing}");
    }

    #[test]
    fn a_python_project_is_named_by_its_pyproject() {
        let text = "[build-system]\nname = \"hatch\"\n\n[project]\nname = \"httpx\"\n";
        assert_eq!(named_in(text).as_deref(), Some("httpx"));
        assert_eq!(named_in("[tool.x]\nname = 'y'\n"), None);
    }
}
