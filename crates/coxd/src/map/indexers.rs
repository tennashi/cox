//! The indexers Cox runs to read a project's code for its map, one for
//! each language it knows: which files at the top of a repository say
//! the language is there, as `Cargo.toml` does Rust's, how the indexer
//! runs, and how the human gets what it needs when that is missing. Each
//! writes a SCIP index (see [`cox_editor::scip`]). Another language is
//! another row of [`known`].
//!
//! A repository of several packages, each in a folder of its own, may
//! have those files in the packages' folders only, and at its top what
//! names the folders: its members, as a `go.work` names Go's modules (see
//! [`Members`]). The indexer then reads the members, in one run at the
//! top, so that the paths in its index are from the top all the same.

use std::collections::{BTreeSet, HashMap};
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
    /// The files at a repository's top, or in a member's folder, any of
    /// which says another indexer reads it instead, as `tsconfig.json`
    /// says of the code of `package.json`.
    pub unless: Vec<String>,
    /// What at a repository's top names its members, whose folders the
    /// indexer reads when the top has none of the markers and theirs do.
    pub members: Option<Members>,
    pub program: Program,
    /// Its arguments, after the program's own, run at the top of a copy
    /// of the commit: `{out}` stands for the file it writes the index
    /// to, `{config}` for the file of [`Indexer::config`], and `{name}`
    /// for the project's name (see [`project_name`]). One with `{member}`
    /// in it is given once for each member read, by its folder from the
    /// top, as `apps/web`, and left out when the top itself is read.
    pub args: Vec<String>,
    /// What `{config}` holds, if it is used, where `{built}` stands for a
    /// folder of the indexer's own, kept from one index to the next, for
    /// what it builds to read the code.
    pub config: Option<String>,
    /// The programs it runs besides its own, which have to be on `PATH`,
    /// each with how to get it.
    pub needs: Vec<(String, String)>,
    /// What the checkout has that the commit does not, at its top and in
    /// its members' folders, which the indexer reads in the copy when
    /// the checkout has it, as the packages `npm install` put in
    /// `node_modules`, with what the index may lack when it does not.
    pub borrows: Option<(String, String)>,
}

/// What at a repository's top names its members: the folders that hold
/// the code of a repository of several packages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Members {
    /// The modules a `go.work` uses.
    GoWork,
    /// The packages of a workspace of npm, Yarn or Bun, which the
    /// `workspaces` of `package.json` name, or of pnpm, which the
    /// `packages` of `pnpm-workspace.yaml` do: each by a pattern, as
    /// `apps/*`, or left out by one after `!`.
    Packages,
}

/// What an indexer reads of a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reads<'a> {
    pub indexer: &'a Indexer,
    /// The members it reads, by their folders from the repository's top,
    /// as `apps/web`, or none when it reads the top itself.
    pub members: Vec<String>,
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
pub fn known() -> Vec<Indexer> {
    let strings = |items: &[&str]| items.iter().map(|&item| item.to_owned()).collect();
    let node = (
        "npx".to_owned(),
        "install Node.js, which has npx: https://nodejs.org".to_owned(),
    );
    let typescript = Indexer {
        name: "scip-typescript".to_owned(),
        language: "TypeScript".to_owned(),
        markers: strings(&["tsconfig.json"]),
        unless: Vec::new(),
        members: Some(Members::Packages),
        program: Program::Npx {
            package: "@sourcegraph/scip-typescript".to_owned(),
            version: "0.6.2".to_owned(),
        },
        args: strings(&["index", "--output", "{out}", "./{member}"]),
        config: None,
        needs: vec![node.clone()],
        // Not needed for the code's own types, whose map is the same
        // without them, but for what the code calls on its packages'
        // types.
        borrows: Some((
            "node_modules".to_owned(),
            "node_modules is not installed, so what the code calls through its packages' types \
             may be left out: install them, as with `npm install`, for Cox to index the code again"
                .to_owned(),
        )),
    };
    vec![
        Indexer {
            name: "rust-analyzer".to_owned(),
            language: "Rust".to_owned(),
            markers: strings(&["Cargo.toml"]),
            unless: Vec::new(),
            // A workspace of Cargo has a `Cargo.toml` at its top.
            members: None,
            program: Program::Server("rust-analyzer".to_owned()),
            args: strings(&[
                "scip",
                ".",
                "--output",
                "{out}",
                "--config-path",
                "{config}",
            ]),
            // `scip` runs the build scripts and builds the procedural
            // macros, whatever its config says, as what they make is in
            // the code it reads: into a target folder kept across
            // commits, so that each builds only what changed.
            config: Some(r#"{"cargo":{"targetDir":"{built}"}}"#.to_owned()),
            needs: vec![(
                "cargo".to_owned(),
                "install Rust with rustup: https://rustup.rs".to_owned(),
            )],
            borrows: None,
        },
        typescript.clone(),
        Indexer {
            name: "scip-typescript-js".to_owned(),
            language: "JavaScript".to_owned(),
            markers: strings(&["package.json", "jsconfig.json"]),
            unless: strings(&["tsconfig.json"]),
            args: strings(&["index", "--infer-tsconfig", "--output", "{out}"]),
            ..typescript
        },
        Indexer {
            name: "scip-python".to_owned(),
            language: "Python".to_owned(),
            markers: strings(&["pyproject.toml", "setup.py", "setup.cfg"]),
            unless: Vec::new(),
            members: None,
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
            members: Some(Members::GoWork),
            program: Program::Go {
                package: "github.com/scip-code/scip-go/cmd/scip-go".to_owned(),
                version: "v0.2.10".to_owned(),
                program: "scip-go".to_owned(),
            },
            // With no `go.mod` at the top, it reads the packages its
            // patterns name, each by its own module.
            args: strings(&["--output={out}", "./{member}/..."]),
            config: None,
            needs: vec![(
                "go".to_owned(),
                "install Go: https://go.dev/doc/install".to_owned(),
            )],
            borrows: None,
        },
    ]
}

/// What each of `indexers` that reads a repository reads of it: of the
/// commit whose files are `files`, by their paths from its top, and
/// whose files have the text `text` gives.
#[must_use]
pub fn of<'a>(
    indexers: &'a [Indexer],
    files: &[String],
    text: &dyn Fn(&str) -> Option<String>,
) -> Vec<Reads<'a>> {
    let at_top = |names: &[String]| names.iter().any(|name| files.contains(name));
    let named: Vec<(Members, Named)> = [Members::GoWork, Members::Packages]
        .into_iter()
        .filter(|members| {
            indexers
                .iter()
                .any(|indexer| indexer.members == Some(*members))
        })
        .filter_map(|members| Some((members, members.named(files, text)?)))
        .collect();
    indexers
        .iter()
        .filter_map(|indexer| {
            let named = named
                .iter()
                .find(|(members, _)| indexer.members == Some(*members))
                .map(|(_, named)| named);
            let among = |names: &[String]| named.map_or_default(|named| named.with(files, names));
            if at_top(&indexer.unless) || !among(&indexer.unless).is_empty() {
                return None;
            }
            let members = if at_top(&indexer.markers) {
                Vec::new()
            } else {
                Some(among(&indexer.markers)).filter(|members| !members.is_empty())?
            };
            Some(Reads { indexer, members })
        })
        .collect()
}

/// The folders a repository's top names as its members: those the
/// patterns `named` match, but for those `left_out` do.
struct Named {
    named: globset::GlobSet,
    left_out: globset::GlobSet,
}

impl Members {
    /// The folders the top of a commit names as its members, if it names
    /// any: of the commit whose files are `files`, with the text `text`
    /// gives.
    fn named(self, files: &[String], text: &dyn Fn(&str) -> Option<String>) -> Option<Named> {
        let read = |file: &str| files.iter().any(|at| at == file).then(|| text(file))?;
        let patterns = match self {
            Self::GoWork => used(&read("go.work")?)
                .iter()
                .map(|folder| globset::escape(folder))
                .collect(),
            Self::Packages => read("package.json")
                .map(|text| workspaces(&text))
                .filter(|patterns| !patterns.is_empty())
                .or_else(|| Some(packages(&read("pnpm-workspace.yaml")?)))?,
        };
        let set = |left_out: bool| {
            let mut set = globset::GlobSetBuilder::new();
            for pattern in &patterns {
                let (pattern, not) = match pattern.strip_prefix('!') {
                    Some(pattern) => (pattern, true),
                    None => (pattern.as_str(), false),
                };
                let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
                let glob = globset::GlobBuilder::new(pattern.trim_end_matches('/'))
                    .literal_separator(true)
                    .build();
                if let Ok(glob) = glob
                    && not == left_out
                {
                    set.add(glob);
                }
            }
            set.build().unwrap_or_else(|_| globset::GlobSet::empty())
        };
        Some(Named {
            named: set(false),
            left_out: set(true),
        })
    }
}

impl Named {
    /// The members among `files` whose folders hold a file named one of
    /// `names`, by their folders, in order: none of them the top, nor in
    /// `node_modules`.
    fn with(&self, files: &[String], names: &[String]) -> Vec<String> {
        let members: BTreeSet<&str> = files
            .iter()
            .filter_map(|file| file.rsplit_once('/'))
            .filter(|(_, name)| names.iter().any(|of| of == name))
            .map(|(folder, _)| folder)
            .filter(|folder| !folder.split('/').any(|part| part == "node_modules"))
            .filter(|folder| self.named.is_match(folder) && !self.left_out.is_match(folder))
            .collect();
        members.into_iter().map(str::to_owned).collect()
    }
}

/// The folders the text of a `go.work` uses, from its own, but for its
/// own and those outside it.
fn used(text: &str) -> Vec<String> {
    let mut used = Vec::new();
    let mut listing = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        let folder = if listing {
            listing = line != ")";
            line
        } else {
            let Some(rest) = line.strip_prefix("use").map(str::trim) else {
                continue;
            };
            listing = rest == "(";
            // Another directive that starts so, as `used`, is none.
            if !line.starts_with("use ") && !line.starts_with("use\t") {
                continue;
            }
            rest
        };
        let folder = folder.trim_matches(|c| c == '"' || c == '`');
        let folder = folder.strip_prefix("./").unwrap_or(folder);
        let folder = folder.trim_end_matches('/');
        let inside = !folder.starts_with('/') && !folder.split('/').any(|part| part == "..");
        if inside && !["", ".", "(", ")"].contains(&folder) {
            used.push(folder.to_owned());
        }
    }
    used
}

/// The patterns the `workspaces` of the text of a `package.json` name
/// its packages by: a list, or the `packages` of a table.
fn workspaces(text: &str) -> Vec<String> {
    let Ok(package) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let workspaces = &package["workspaces"];
    let patterns = workspaces
        .as_array()
        .or_else(|| workspaces["packages"].as_array());
    patterns
        .into_iter()
        .flatten()
        .filter_map(|pattern| pattern.as_str().map(str::to_owned))
        .collect()
}

/// The patterns the `packages` of the text of a `pnpm-workspace.yaml`
/// name its packages by: a list, an item to a line.
fn packages(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|line| line.trim_end() != "packages:")
        .skip(1)
        .map(|line| line.split(" #").next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map_while(|line| line.strip_prefix('-'))
        .map(|pattern| {
            pattern
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_owned()
        })
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

    /// What the indexers Cox knows read of a commit of the files
    /// `files`, each with its text: each by its name, with its members.
    fn reads(files: &[(&str, &str)]) -> Vec<(String, Vec<String>)> {
        let indexers = known();
        let names: Vec<String> = files.iter().map(|(file, _)| (*file).to_owned()).collect();
        let text = |file: &str| {
            let (_, text) = files.iter().find(|(at, _)| *at == file)?;
            Some((*text).to_owned())
        };
        of(&indexers, &names, &text)
            .into_iter()
            .map(|reads| (reads.indexer.name.clone(), reads.members))
            .collect()
    }

    /// The names of the indexers that read the top of a commit of the
    /// files `top`.
    fn names(top: &[&str]) -> Vec<String> {
        let files: Vec<(&str, &str)> = top.iter().map(|file| (*file, "")).collect();
        let reads = reads(&files);
        assert!(reads.iter().all(|(_, members)| members.is_empty()));
        reads.into_iter().map(|(name, _)| name).collect()
    }

    fn read(name: &str, members: &[&str]) -> (String, Vec<String>) {
        let members = members.iter().map(|member| (*member).to_owned());
        (name.to_owned(), members.collect())
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
        // A file further down says nothing, with nothing at the top that
        // names its folder.
        assert_eq!(names(&["web/tsconfig.json", "tool/go.mod"]).len(), 0);
    }

    #[test]
    fn the_modules_a_go_work_uses_are_read_from_the_top() {
        let work = "go 1.26.0\n\nuse (\n\t./packages/protocol // shared\n\t./apps/oesd\n\t./gone\n\t../other\n)\nuse ./tools/kit/\n";
        let files = [
            ("go.work", work),
            ("apps/oesd/go.mod", ""),
            ("apps/oesd/main.go", ""),
            ("packages/protocol/go.mod", ""),
            ("tools/kit/go.mod", ""),
            // Not one the workspace uses.
            ("vendor/thing/go.mod", ""),
        ];
        assert_eq!(
            reads(&files),
            [read(
                "scip-go",
                &["apps/oesd", "packages/protocol", "tools/kit"]
            )]
        );

        // A module at the top is read as ever, with all under it.
        let mut at_top = files.to_vec();
        at_top.push(("go.mod", ""));
        assert_eq!(reads(&at_top), [read("scip-go", &[])]);
    }

    #[test]
    fn the_packages_of_a_workspace_are_read_by_their_own_tsconfigs() {
        let package = r#"{ "workspaces": ["apps/*", "packages/*", "tools", "!apps/old"] }"#;
        let files = [
            ("package.json", package),
            ("tsconfig.base.json", ""),
            ("apps/fleet/package.json", ""),
            ("apps/fleet/tsconfig.json", ""),
            ("apps/fleet/src/main.ts", ""),
            // A package with no TypeScript of its own.
            ("apps/oesd/package.json", ""),
            ("apps/old/tsconfig.json", ""),
            ("apps/fleet/node_modules/dep/tsconfig.json", ""),
            ("apps/fleet/fixtures/tsconfig.json", ""),
            ("packages/wire/tsconfig.json", ""),
            ("tools/tsconfig.json", ""),
            ("elsewhere/tsconfig.json", ""),
        ];
        let members = ["apps/fleet", "packages/wire", "tools"];
        assert_eq!(reads(&files), [read("scip-typescript", &members)]);

        // Bun's and Yarn's table, and pnpm's file, name them too.
        let table = r#"{ "workspaces": { "packages": ["apps/*", "packages/*", "tools"] } }"#;
        let mut bun = files.to_vec();
        bun[0] = ("package.json", table);
        bun.retain(|(file, _)| *file != "apps/old/tsconfig.json");
        assert_eq!(reads(&bun), [read("scip-typescript", &members)]);
        let mut pnpm = bun.clone();
        pnpm[0] = ("package.json", "{}");
        pnpm.push((
            "pnpm-workspace.yaml",
            "packages:\n  # all of them\n  - 'apps/*'\n  - \"packages/**\"\n  - tools\ncatalog:\n  - no\n",
        ));
        assert_eq!(reads(&pnpm), [read("scip-typescript", &members)]);

        // A `tsconfig.json` at the top reads all of it, as ever.
        let mut at_top = files.to_vec();
        at_top.push(("tsconfig.json", ""));
        assert_eq!(reads(&at_top), [read("scip-typescript", &[])]);
    }

    #[test]
    fn a_workspace_of_javascript_alone_is_read_from_its_top() {
        let files = [
            ("package.json", r#"{ "workspaces": ["packages/*"] }"#),
            ("packages/a/package.json", ""),
            ("packages/a/index.js", ""),
        ];
        assert_eq!(reads(&files), [read("scip-typescript-js", &[])]);
    }

    #[test]
    fn a_repository_of_go_and_typescript_packages_is_read_by_both() {
        let files = [
            ("package.json", r#"{ "workspaces": ["apps/*"] }"#),
            ("go.work", "use ./apps/oesd\n"),
            ("apps/oesd/go.mod", ""),
            ("apps/oesd/package.json", ""),
            ("apps/fleet/package.json", ""),
            ("apps/fleet/tsconfig.json", ""),
        ];
        assert_eq!(
            reads(&files),
            [
                read("scip-typescript", &["apps/fleet"]),
                read("scip-go", &["apps/oesd"])
            ]
        );
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
