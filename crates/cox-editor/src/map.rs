//! A map of a Rust project, for the human to see where things are before
//! they read them: the crates of its workspace, and the modules in each,
//! as the `mod` lines declare them, each with the first paragraph of what
//! its file says of itself (its `//!` lines), how long it is, and, for a
//! crate, which of the workspace's crates it depends on. All of it comes
//! from the files as they are, so the map is as the project is now, with
//! nothing written for it.
//!
//! What changed in the project (see [`crate::changes`]) is laid over the
//! map by file: a module changed when its file did, or a file of a module
//! in it, and a crate when a file in its folder did.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::changes::Diff;

/// How deep the modules of a module go, at most: deeper is a cycle of
/// links, not a project.
const DEEPEST: usize = 16;

/// A project, as its crates and modules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map {
    /// The project's folder, where its `Cargo.toml` is.
    pub root: PathBuf,
    /// The crates of the workspace, in order of their folders; the one
    /// crate of a project that is no workspace.
    pub crates: Vec<Crate>,
}

/// A crate of the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crate {
    /// Its name, as its `Cargo.toml` gives it.
    pub name: String,
    /// Its folder, an absolute path.
    pub folder: PathBuf,
    /// The crates of the workspace it depends on, by name, in order.
    pub uses: Vec<String>,
    /// The files the crate starts from: `src/lib.rs`, then `src/main.rs`,
    /// those that are there, each with the modules it declares.
    pub entries: Vec<Module>,
}

/// A module of a crate, or a file a crate starts from: a file, and the
/// modules it declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Module {
    /// Its name: the name after `mod`, or the file's name for a file a
    /// crate starts from.
    pub name: String,
    /// Its file, an absolute path.
    pub path: PathBuf,
    /// The first paragraph of what the file says of itself, in its `//!`
    /// lines, or nothing when it says nothing.
    pub said: String,
    /// How many lines the file has.
    pub lines: usize,
    /// The modules it declares, in the order it declares them, that have
    /// a file.
    pub modules: Vec<Module>,
}

/// What changed in a part of the map: in how many files, and how many
/// lines they gained and lost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    pub files: usize,
    pub added: usize,
    pub removed: usize,
}

/// The lines each file that changed gained and lost, by its absolute
/// path, as [`Diff`] tells it. A file that was removed counts where it
/// was.
#[must_use]
pub fn changed_files(diff: &Diff) -> HashMap<PathBuf, (usize, usize)> {
    diff.files
        .iter()
        .map(|file| (diff.top.join(&file.path), (file.added, file.removed)))
        .collect()
}

impl Changed {
    fn add(&mut self, (added, removed): (usize, usize)) {
        self.files += 1;
        self.added += added;
        self.removed += removed;
    }

    /// Whether anything changed.
    #[must_use]
    pub fn any(self) -> bool {
        self.files > 0
    }
}

impl Crate {
    /// The first paragraph of what the crate says of itself: what its
    /// first file that says anything says.
    #[must_use]
    pub fn said(&self) -> &str {
        self.entries
            .iter()
            .map(|entry| entry.said.as_str())
            .find(|said| !said.is_empty())
            .unwrap_or("")
    }

    /// How many lines the crate's files have, in all.
    #[must_use]
    pub fn lines(&self) -> usize {
        self.entries.iter().map(Module::lines_in_all).sum()
    }

    /// The modules the crate's files declare, in order, each once.
    #[must_use]
    pub fn modules(&self) -> Vec<&Module> {
        let mut seen = Vec::new();
        for entry in &self.entries {
            for module in &entry.modules {
                if !seen.iter().any(|known: &&Module| known.name == module.name) {
                    seen.push(module);
                }
            }
        }
        seen
    }

    /// What changed in the crate's folder, among `files`, the files that
    /// changed with the lines each gained and lost.
    #[must_use]
    pub fn changed(&self, files: &HashMap<PathBuf, (usize, usize)>) -> Changed {
        let mut changed = Changed::default();
        for (path, &counts) in files {
            if path.starts_with(&self.folder) {
                changed.add(counts);
            }
        }
        changed
    }
}

impl Module {
    /// How many lines the module's file and those of the modules in it
    /// have, in all.
    #[must_use]
    pub fn lines_in_all(&self) -> usize {
        self.lines + self.modules.iter().map(Self::lines_in_all).sum::<usize>()
    }

    /// How many modules are in it, at every depth.
    #[must_use]
    pub fn modules_in_all(&self) -> usize {
        self.modules.len() + self.modules.iter().map(Self::modules_in_all).sum::<usize>()
    }

    /// What changed in the module's file, and in those of the modules in
    /// it, among `files`.
    #[must_use]
    pub fn changed(&self, files: &HashMap<PathBuf, (usize, usize)>) -> Changed {
        let mut changed = Changed::default();
        self.count_changed(files, &mut changed);
        changed
    }

    fn count_changed(&self, files: &HashMap<PathBuf, (usize, usize)>, changed: &mut Changed) {
        if let Some(&counts) = files.get(&self.path) {
            changed.add(counts);
        }
        for module in &self.modules {
            module.count_changed(files, changed);
        }
    }
}

/// The map of the project in `root`, an absolute path, where its
/// `Cargo.toml` is.
///
/// # Errors
///
/// When there is no `Cargo.toml` in `root`, or it cannot be read.
pub fn read(root: &Path) -> Result<Map, String> {
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|error| format!("cannot read {}: {error}", manifest.display()))?;
    let top = Manifest::parse(&text);
    let mut folders: Vec<PathBuf> = Vec::new();
    if top.package.is_some() {
        folders.push(root.to_owned());
    }
    for member in &top.members {
        folders.extend(members(root, member));
    }
    if folders.is_empty() {
        return Err(format!(
            "{} names no package and no workspace members",
            manifest.display()
        ));
    }
    let manifests: Vec<(PathBuf, Manifest)> = folders
        .into_iter()
        .filter_map(|folder| {
            let text = std::fs::read_to_string(folder.join("Cargo.toml")).ok()?;
            let manifest = Manifest::parse(&text);
            manifest.package.as_ref()?;
            Some((folder, manifest))
        })
        .collect();
    let names: Vec<String> = manifests
        .iter()
        .filter_map(|(_, manifest)| manifest.package.clone())
        .collect();
    let crates = manifests
        .into_iter()
        .map(|(folder, manifest)| {
            let name = manifest.package.unwrap_or_default();
            let uses = names
                .iter()
                .filter(|known| **known != name && manifest.dependencies.contains(known))
                .cloned()
                .collect();
            let entries = ["lib.rs", "main.rs"]
                .into_iter()
                .map(|file| folder.join("src").join(file))
                .filter(|path| path.is_file())
                .map(|path| module(path, 0))
                .collect();
            Crate {
                name,
                folder,
                uses,
                entries,
            }
        })
        .collect();
    Ok(Map {
        root: root.to_owned(),
        crates,
    })
}

/// The folders `member`, a line of a workspace's `members`, names under
/// `root`: the folders in a folder for one that ends in `/*`, those with a
/// `Cargo.toml`, in order of their names.
fn members(root: &Path, member: &str) -> Vec<PathBuf> {
    if let Some(parent) = member.strip_suffix("/*") {
        let Ok(entries) = std::fs::read_dir(root.join(parent)) else {
            return Vec::new();
        };
        let mut folders: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.join("Cargo.toml").is_file())
            .collect();
        folders.sort();
        folders
    } else {
        let folder = root.join(member);
        if folder.join("Cargo.toml").is_file() {
            Vec::from([folder])
        } else {
            Vec::new()
        }
    }
}

/// The module whose file is at `path`, `depth` modules under the file its
/// crate starts from, with the modules it declares.
fn module(path: PathBuf, depth: usize) -> Module {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let modules = if depth < DEEPEST {
        declared_in(&path, &text)
            .into_iter()
            .map(|(name, path)| Module {
                name: name.clone(),
                ..module(path, depth + 1)
            })
            .collect()
    } else {
        Vec::new()
    };
    Module {
        name,
        path,
        said: said(&text),
        lines: text.lines().count(),
        modules,
    }
}

/// The modules the file at `path`, which says `text`, declares, by name
/// and the file of each, those that have one: `name.rs` or `name/mod.rs`
/// beside the file, for a file a crate starts from or a `mod.rs`, and in
/// the folder of the file's name otherwise.
fn declared_in(path: &Path, text: &str) -> Vec<(String, PathBuf)> {
    let parent = path.parent().unwrap_or(Path::new(""));
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let folder = if matches!(file, "lib.rs" | "main.rs" | "mod.rs") {
        parent.to_owned()
    } else {
        parent.join(path.file_stem().unwrap_or_default())
    };
    text.lines()
        .filter_map(declared)
        .filter_map(|name| {
            let flat = folder.join(format!("{name}.rs"));
            let nested = folder.join(name).join("mod.rs");
            let path = [flat, nested].into_iter().find(|path| path.is_file())?;
            Some((name.to_owned(), path))
        })
        .collect()
}

/// The name of the module `line` declares, as `mod name;` with or without
/// a visibility ahead, if it declares one.
fn declared(line: &str) -> Option<&str> {
    let line = line.trim();
    let rest = match line.strip_prefix("pub") {
        Some(rest) => {
            let rest = rest.trim_start();
            match rest.strip_prefix('(') {
                Some(inside) => inside.split_once(')')?.1.trim_start(),
                None => rest,
            }
        }
        None => line,
    };
    let name = rest
        .strip_prefix("mod ")?
        .trim()
        .strip_suffix(';')?
        .trim_end();
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(name)
}

/// The first paragraph of the `//!` lines at the top of `text`, on one
/// line, without the brackets of its links.
fn said(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(doc) = line.strip_prefix("//!") else {
            if lines.is_empty() && (line.is_empty() || line.starts_with("#!")) {
                continue;
            }
            break;
        };
        let doc = doc.trim();
        if doc.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        lines.push(doc);
    }
    lines
        .join(" ")
        .chars()
        .filter(|c| !matches!(c, '[' | ']'))
        .collect()
}

/// What a `Cargo.toml` says that the map needs.
#[derive(Debug, Default, PartialEq, Eq)]
struct Manifest {
    /// The package's name, if the file names a package.
    package: Option<String>,
    /// The workspace's members, as the file lists them.
    members: Vec<String>,
    /// The names of its dependencies, of every target.
    dependencies: Vec<String>,
}

impl Manifest {
    /// Reads `text`, a `Cargo.toml`, line by line: enough of TOML for the
    /// tables the map needs.
    fn parse(text: &str) -> Self {
        let mut manifest = Self::default();
        let mut table = String::new();
        let mut in_members = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if in_members {
                manifest.members.extend(quoted(line));
                if line.contains(']') {
                    in_members = false;
                }
                continue;
            }
            if let Some(header) = line.strip_prefix('[') {
                header.trim_end_matches(']').trim().clone_into(&mut table);
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().split('.').next().unwrap_or("").trim();
            let value = value.trim();
            match table.as_str() {
                "package" if key == "name" => {
                    manifest.package = quoted(value).into_iter().next();
                }
                "workspace" if key == "members" => {
                    manifest.members.extend(quoted(value));
                    in_members = !value.contains(']');
                }
                table if is_dependencies(table) => manifest.dependencies.push(key.to_owned()),
                _ => {}
            }
        }
        manifest
    }
}

/// Whether `table` lists dependencies that are built in: `dependencies`,
/// or those of a target, not the workspace's, nor those of development or
/// of the build.
fn is_dependencies(table: &str) -> bool {
    table == "dependencies" || (table.starts_with("target.") && table.ends_with(".dependencies"))
}

/// The strings quoted in `line`, in order.
fn quoted(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::{File, How, Since};

    /// A folder of its own for a test, with the files `files`, each with
    /// its text.
    fn project(test: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cox-map-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        root
    }

    #[test]
    fn a_mod_line_declares_a_module_whatever_its_visibility() {
        assert_eq!(declared("mod app;"), Some("app"));
        assert_eq!(declared("pub mod app;"), Some("app"));
        assert_eq!(declared("pub(crate) mod app;"), Some("app"));
        assert_eq!(declared("  pub(in crate::a) mod b_2;"), Some("b_2"));
        assert_eq!(declared("mod tests {"), None);
        assert_eq!(declared("use crate::mod_a;"), None);
        assert_eq!(declared("// mod app;"), None);
        assert_eq!(declared("modules;"), None);
    }

    #[test]
    fn what_a_file_says_is_its_first_doc_paragraph_on_one_line() {
        let text = "#![allow(x)]\n\n//! The first\n//! paragraph, with a [`link`].\n//!\n//! The second.\nuse x;\n";
        assert_eq!(said(text), "The first paragraph, with a `link`.");
        assert_eq!(said("use x;\n//! Not at the top.\n"), "");
        assert_eq!(said(""), "");
    }

    #[test]
    fn a_manifest_gives_its_package_members_and_dependencies() {
        let text = r#"
[workspace]
resolver = "3"
members = [
    "crates/*",
    "xtask",
]

[package]
name = "top"

[dependencies]
anyhow.workspace = true
# a comment
serde = { version = "1", features = ["derive"] }
cox-core = { path = "crates/cox-core" }

[dev-dependencies]
tokio = "1"

[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"

[workspace.dependencies]
not-mine = "1"
"#;
        let manifest = Manifest::parse(text);
        assert_eq!(manifest.package.as_deref(), Some("top"));
        assert_eq!(manifest.members, ["crates/*", "xtask"]);
        assert_eq!(
            manifest.dependencies,
            ["anyhow", "serde", "cox-core", "objc2"]
        );
    }

    #[test]
    fn a_workspace_maps_to_its_crates_and_their_modules() {
        let root = project(
            "workspace",
            &[
                ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
                (
                    "crates/app/Cargo.toml",
                    "[package]\nname = \"app\"\n[dependencies]\ncore = { path = \"../core\" }\nserde = \"1\"\n",
                ),
                (
                    "crates/app/src/lib.rs",
                    "//! The app.\n//!\n//! More.\n\npub mod stage;\nmod ui;\nmod gone;\n",
                ),
                ("crates/app/src/main.rs", "fn main() {}\n"),
                (
                    "crates/app/src/stage.rs",
                    "//! The stage,\n//! beside the chat.\nmod tab;\npub(crate) mod code;\n",
                ),
                ("crates/app/src/stage/tab.rs", "//! A tab.\n"),
                ("crates/app/src/stage/code/mod.rs", "// no doc\n"),
                ("crates/app/src/ui.rs", ""),
                ("crates/core/Cargo.toml", "[package]\nname = \"core\"\n"),
                ("crates/core/src/lib.rs", "//! The core.\n"),
                ("crates/other/README.md", "not a crate"),
            ],
        );
        let map = read(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();

        let names: Vec<&str> = map.crates.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, ["app", "core"]);
        let app = &map.crates[0];
        assert_eq!(app.folder, root.join("crates/app"));
        assert_eq!(app.uses, ["core"]);
        assert_eq!(app.said(), "The app.");
        let entries: Vec<&str> = app.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(entries, ["lib.rs", "main.rs"]);
        let modules: Vec<&str> = app.modules().iter().map(|m| m.name.as_str()).collect();
        assert_eq!(modules, ["stage", "ui"]);
        let stage = app.modules()[0];
        assert_eq!(stage.path, root.join("crates/app/src/stage.rs"));
        assert_eq!(stage.said, "The stage, beside the chat.");
        assert_eq!(stage.lines, 4);
        let inner: Vec<(&str, PathBuf)> = stage
            .modules
            .iter()
            .map(|m| (m.name.as_str(), m.path.clone()))
            .collect();
        assert_eq!(
            inner,
            [
                ("tab", root.join("crates/app/src/stage/tab.rs")),
                ("code", root.join("crates/app/src/stage/code/mod.rs")),
            ]
        );
        assert_eq!(stage.modules_in_all(), 2);
        assert_eq!(stage.lines_in_all(), 4 + 1 + 1);
        // lib.rs and main.rs, stage.rs and the two in it, and ui.rs.
        assert_eq!(app.lines(), 7 + 1 + 6);
        assert_eq!(map.crates[1].uses, Vec::<String>::new());
        assert_eq!(map.crates[1].said(), "The core.");
    }

    #[test]
    fn a_project_that_is_no_workspace_is_one_crate() {
        let root = project(
            "single",
            &[
                ("Cargo.toml", "[package]\nname = \"one\"\n"),
                ("src/main.rs", "//! One.\nmod a;\n"),
                ("src/a.rs", "//! A.\n"),
            ],
        );
        let map = read(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(map.crates.len(), 1);
        assert_eq!(map.crates[0].name, "one");
        assert_eq!(map.crates[0].folder, root);
        assert_eq!(map.crates[0].modules()[0].name, "a");
    }

    #[test]
    fn a_folder_with_no_manifest_has_no_map() {
        let root = project("none", &[("README.md", "")]);
        let error = read(&root).unwrap_err();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(error.contains("Cargo.toml"), "{error}");
    }

    #[test]
    fn changes_lay_over_modules_and_crates_by_file() {
        let root = Path::new("/p");
        let tab = Module {
            name: "tab".to_owned(),
            path: root.join("crates/app/src/stage/tab.rs"),
            said: String::new(),
            lines: 1,
            modules: Vec::new(),
        };
        let stage = Module {
            name: "stage".to_owned(),
            path: root.join("crates/app/src/stage.rs"),
            said: String::new(),
            lines: 1,
            modules: Vec::from([tab]),
        };
        let app = Crate {
            name: "app".to_owned(),
            folder: root.join("crates/app"),
            uses: Vec::new(),
            entries: Vec::from([Module {
                name: "lib.rs".to_owned(),
                path: root.join("crates/app/src/lib.rs"),
                said: String::new(),
                lines: 1,
                modules: Vec::from([stage]),
            }]),
        };
        let files: HashMap<PathBuf, (usize, usize)> = [
            (root.join("crates/app/src/stage/tab.rs"), (3, 1)),
            (root.join("crates/app/Cargo.toml"), (1, 0)),
            (root.join("crates/core/src/lib.rs"), (5, 5)),
        ]
        .into_iter()
        .collect();
        let stage = &app.entries[0].modules[0];
        assert_eq!(
            stage.changed(&files),
            Changed {
                files: 1,
                added: 3,
                removed: 1
            }
        );
        assert_eq!(app.entries[0].changed(&files), stage.changed(&files));
        assert_eq!(
            app.changed(&files),
            Changed {
                files: 2,
                added: 4,
                removed: 1
            }
        );
        assert!(!Changed::default().any());
    }

    #[test]
    fn a_diff_is_laid_over_by_absolute_path() {
        let diff = Diff {
            top: PathBuf::from("/p"),
            since: Since::Head("abc".to_owned()),
            files: Vec::from([File {
                path: PathBuf::from("src/a.rs"),
                was: None,
                how: How::Changed,
                lines: Vec::new(),
                binary: false,
                added: 2,
                removed: 1,
            }]),
        };
        let files = changed_files(&diff);
        assert_eq!(files.get(Path::new("/p/src/a.rs")), Some(&(2, 1)));
    }
}
