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
//!
//! Each file also names what it uses, in its `use` lines and in the paths
//! in its code, as `crate::stage::Stage` or `cox_core::tabs`: the map
//! keeps those that lead into a crate of the project, resolved from the
//! crate's name down, so that for any two parts of the map it can tell
//! whether one uses the other.

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

/// Where a map is open, as far as the map has the names given, and what
/// is there (see [`Map::place`]).
#[derive(Debug, PartialEq, Eq)]
pub struct Place<'a> {
    /// How many of the names given the map has, from the first.
    pub depth: usize,
    /// The crate the names lead into; none at the crates.
    pub krate: Option<&'a Crate>,
    /// The files there: those the crate starts from, or that of the
    /// module the names lead to; none at the crates.
    pub files: Vec<&'a Module>,
    /// The modules there; none at the crates.
    pub modules: Vec<&'a Module>,
}

impl Map {
    /// Where the map is open `at`: the crate the first name is, and the
    /// modules in it the names after it are, from the crate down, as far
    /// as the map has them; the crates when it names none, or no crate
    /// of the map.
    #[must_use]
    pub fn place(&self, at: &[String]) -> Place<'_> {
        let Some(krate) = at
            .first()
            .and_then(|name| self.crates.iter().find(|krate| krate.name == *name))
        else {
            return Place {
                depth: 0,
                krate: None,
                files: Vec::new(),
                modules: Vec::new(),
            };
        };
        let mut depth = 1;
        let mut files: Vec<&Module> = krate.entries.iter().collect();
        let mut modules: Vec<&Module> = krate.modules();
        for name in at.iter().skip(1) {
            let Some(module) = modules.iter().find(|module| module.name == *name) else {
                break;
            };
            files = Vec::from([*module]);
            modules = module.modules.iter().collect();
            depth += 1;
        }
        Place {
            depth,
            krate: Some(krate),
            files,
            modules,
        }
    }
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
    /// Where it is in its crate: the names from the crate down to it, or
    /// none for a file a crate starts from.
    pub at: Vec<String>,
    /// The first paragraph of what the file says of itself, in its `//!`
    /// lines, or nothing when it says nothing.
    pub said: String,
    /// How many lines the file has.
    pub lines: usize,
    /// What the file names that is in a crate of the project, each once,
    /// in order: a path of the crate's name and the names after it, as
    /// `["cox", "stage", "Stage"]` for `crate::stage::Stage` in `cox`.
    pub names: Vec<Vec<String>>,
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

    /// What the module's file names, and the files of the modules in it.
    pub fn names_in_all(&self) -> impl Iterator<Item = &[String]> {
        let own = self.names.iter().map(Vec::as_slice);
        let within = self.modules.iter().flat_map(|module| {
            Box::new(module.names_in_all()) as Box<dyn Iterator<Item = &[String]>>
        });
        own.chain(within)
    }

    /// Whether `name`, a path from a crate's name down as [`Self::names`]
    /// has them, leads into the module of crate `krate`, or is it: into
    /// its file, or a module in it.
    #[must_use]
    pub fn is_named(&self, krate: &str, name: &[String]) -> bool {
        name.first().is_some_and(|first| first == krate) && name[1..].starts_with(&self.at)
    }

    /// Whether `name` leads into the module's own file, and not a module
    /// in it: it is the module, or one thing in it.
    #[must_use]
    pub fn file_is_named(&self, krate: &str, name: &[String]) -> bool {
        self.is_named(krate, name)
            && match name.get(1 + self.at.len()..) {
                None | Some([]) => true,
                Some([one]) => !self.modules.iter().any(|module| module.name == *one),
                Some(_) => false,
            }
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
                .map(|path| module(path, &name, Vec::new(), &names, 0))
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

/// The module whose file is at `path`, `at` in crate `krate` of the
/// project whose crates are `crates`, `depth` modules under the file its
/// crate starts from, with the modules it declares.
fn module(path: PathBuf, krate: &str, at: Vec<String>, crates: &[String], depth: usize) -> Module {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let modules = if depth < DEEPEST {
        declared_in(&path, &text)
            .into_iter()
            .map(|(name, path)| {
                let mut within = at.clone();
                within.push(name.clone());
                Module {
                    name,
                    ..module(path, krate, within, crates, depth + 1)
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let names = names_in(&text, krate, &at, crates);
    Module {
        name,
        path,
        at,
        said: said(&text),
        lines: text.lines().count(),
        names,
        modules,
    }
}

/// What `text`, the file `at` in crate `krate`, names that is in one of
/// `crates`: the paths of its `use` lines and of its code, resolved from
/// the crate's name down (see [`Module::names`]), each once, in order.
fn names_in(text: &str, krate: &str, at: &[String], crates: &[String]) -> Vec<Vec<String>> {
    let code = without_comments(text);
    let mut names: Vec<Vec<String>> = Vec::new();
    let paths = use_paths(&code).into_iter().chain(inline_paths(&code));
    for path in paths {
        if let Some(name) = resolved(&path, krate, at, crates)
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

/// `text` without its `//` comments, so that what they say is not taken
/// for code; `://`, as in an address, is not a comment.
fn without_comments(text: &str) -> String {
    let mut code = String::with_capacity(text.len());
    for line in text.lines() {
        let mut kept = line;
        let mut from = 0;
        while let Some(found) = kept[from..].find("//") {
            let index = from + found;
            if index > 0 && kept.as_bytes()[index - 1] == b':' {
                from = index + 2;
            } else {
                kept = &kept[..index];
                break;
            }
        }
        code.push_str(kept);
        code.push('\n');
    }
    code
}

/// The paths of the `use` lines in `code`, each in full: `use a::{b, c::d};`
/// is `a::b` and `a::c::d`. A `*` ends a path at the path before it.
fn use_paths(code: &str) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    let mut from = 0;
    while let Some(found) = code[from..].find("use ") {
        let index = from + found;
        let before = code[..index].chars().next_back();
        from = index + 4;
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Some(end) = code[from..].find(';') else {
            break;
        };
        let tree = &code[from..from + end];
        from += end + 1;
        let tokens: Vec<&str> = use_tokens(tree);
        let mut index = 0;
        use_tree(&tokens, &mut index, &[], &mut paths);
    }
    paths
}

/// The tokens of a `use` tree: names, `::`, `{`, `}`, `,`, `*` and `as`.
fn use_tokens(tree: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut rest = tree;
    while !rest.is_empty() {
        let next = rest.chars().next().unwrap_or(' ');
        if next.is_whitespace() {
            rest = rest.trim_start();
        } else if let Some(after) = rest.strip_prefix("::") {
            tokens.push("::");
            rest = after;
        } else if next.is_alphanumeric() || next == '_' {
            let len = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            tokens.push(&rest[..len]);
            rest = &rest[len..];
        } else {
            tokens.push(&rest[..next.len_utf8()]);
            rest = &rest[next.len_utf8()..];
        }
    }
    tokens
}

/// Reads one tree of `tokens` from `index`, under `prefix`, into `paths`,
/// up to a `,` or `}` of the tree around it, or the end.
fn use_tree(tokens: &[&str], index: &mut usize, prefix: &[String], paths: &mut Vec<Vec<String>>) {
    let mut path = prefix.to_vec();
    while *index < tokens.len() {
        let token = tokens[*index];
        *index += 1;
        match token {
            "::" => {}
            "{" => loop {
                use_tree(tokens, index, &path, paths);
                match tokens.get(*index - 1) {
                    Some(&",") => {}
                    _ => return,
                }
            },
            "," | "}" | "*" => break,
            "as" => {
                // The alias: not a name of the path's.
                *index += 1;
            }
            "self" if path.len() > prefix.len() || !prefix.is_empty() => {}
            name => path.push(name.to_owned()),
        }
    }
    if !path.is_empty() {
        paths.push(path);
    }
}

/// The paths of two or more names in `code`, as `crate::stage::Stage`
/// where it is written, whatever is before or after them.
fn inline_paths(code: &str) -> Vec<Vec<String>> {
    let mut paths = Vec::new();
    let bytes = code.as_bytes();
    let is_name = |index: usize| {
        bytes
            .get(index)
            .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
    };
    let mut index = 0;
    while index < bytes.len() {
        if !is_name(index) || (index > 0 && is_name(index - 1)) {
            index += 1;
            continue;
        }
        let mut path = Vec::new();
        loop {
            let start = index;
            while is_name(index) {
                index += 1;
            }
            path.push(code[start..index].to_owned());
            if bytes.get(index..index + 2) == Some(b"::") && is_name(index + 2) {
                index += 2;
            } else {
                break;
            }
        }
        if path.len() > 1 {
            paths.push(path);
        }
    }
    paths
}

/// `path`, as written in the file `at` in crate `krate`, from the name of
/// the crate it leads into down, if it leads into one of `crates`:
/// `crate::` is `krate`, `self::` is the file's module, `super::` the
/// module it is in, and a crate's name, with `_` for `-`, is that crate.
fn resolved(path: &[String], krate: &str, at: &[String], crates: &[String]) -> Option<Vec<String>> {
    let (first, rest) = path.split_first()?;
    let mut name = vec![krate.to_owned()];
    match first.as_str() {
        "crate" => {}
        "self" => name.extend(at.iter().cloned()),
        "super" => {
            if rest.is_empty() {
                return None;
            }
            name.extend(at.iter().take(at.len().checked_sub(1)?).cloned());
        }
        other => {
            let found = crates
                .iter()
                .find(|known| known.replace('-', "_") == *other)?;
            name = vec![found.clone()];
        }
    }
    name.extend(rest.iter().cloned());
    Some(name)
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
                (
                    "crates/app/src/stage/tab.rs",
                    "//! A tab, see [`crate::ui`].\nuse core::Thing;\nuse super::code;\nfn f() { crate::ui::draw(); }\n",
                ),
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
        assert_eq!(stage.at, ["stage"]);
        let tab = &stage.modules[0];
        assert_eq!(tab.at, ["stage", "tab"]);
        let names: Vec<Vec<&str>> = tab
            .names
            .iter()
            .map(|name| name.iter().map(String::as_str).collect())
            .collect();
        assert_eq!(
            names,
            [
                vec!["core", "Thing"],
                vec!["app", "stage", "code"],
                vec!["app", "ui", "draw"],
            ]
        );
        assert_eq!(stage.modules_in_all(), 2);
        assert_eq!(stage.lines_in_all(), 4 + 4 + 1);
        // lib.rs and main.rs, stage.rs and the two in it, and ui.rs.
        assert_eq!(app.lines(), 7 + 1 + 9);
        assert_eq!(map.crates[1].uses, Vec::<String>::new());
        assert_eq!(map.crates[1].said(), "The core.");

        let at =
            |names: &[&str]| -> Vec<String> { names.iter().map(|n| (*n).to_owned()).collect() };
        let crates = map.place(&at(&[]));
        assert_eq!((crates.depth, crates.krate), (0, None));
        let in_app = map.place(&at(&["app"]));
        assert_eq!(in_app.depth, 1);
        assert_eq!(in_app.krate.map(|k| k.name.as_str()), Some("app"));
        let files: Vec<&str> = in_app.files.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(files, ["lib.rs", "main.rs"]);
        let modules: Vec<&str> = in_app.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(modules, ["stage", "ui"]);
        let in_stage = map.place(&at(&["app", "stage", "nowhere"]));
        assert_eq!(in_stage.depth, 2);
        assert_eq!(in_stage.files[0].name, "stage");
        let modules: Vec<&str> = in_stage.modules.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(modules, ["tab", "code"]);
        assert_eq!(map.place(&at(&["nowhere"])).depth, 0);
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
    fn a_file_names_what_its_use_lines_and_its_code_lead_to() {
        let text = r#"
//! Docs name [`crate::ui`], which is not a use.
use std::path::{Path, PathBuf};
pub(crate) use crate::stage::{Stage, tab::{self, TabContent as Content}};
use super::*;
use cox_core::tabs::Lines;
use self::inner::Thing;

fn f(x: &crate::ui::Link) {
    let url = "https://example.com//not/a/comment";
    super::page::open(x); // a comment: crate::nowhere
    cox_browser::start();
}
"#;
        let crates: Vec<String> = ["cox", "cox-core", "cox-browser"].map(str::to_owned).into();
        let at: Vec<String> = ["stage", "links"].map(str::to_owned).into();
        let names: Vec<String> = names_in(text, "cox", &at, &crates)
            .iter()
            .map(|name| name.join("::"))
            .collect();
        assert_eq!(
            names,
            [
                "cox::stage::Stage",
                "cox::stage::tab",
                "cox::stage::tab::TabContent",
                "cox-core::tabs::Lines",
                "cox::stage::links::inner::Thing",
                "cox::stage",
                "cox::ui::Link",
                "cox::stage::page::open",
                "cox-browser::start",
            ]
        );
    }

    #[test]
    fn a_name_leads_into_a_module_or_its_own_file() {
        let name = |path: &str| -> Vec<String> { path.split("::").map(str::to_owned).collect() };
        let tab = Module {
            name: "tab".to_owned(),
            path: PathBuf::from("/p/src/stage/tab.rs"),
            at: name("stage::tab"),
            said: String::new(),
            lines: 1,
            names: Vec::new(),
            modules: Vec::new(),
        };
        let stage = Module {
            name: "stage".to_owned(),
            path: PathBuf::from("/p/src/stage.rs"),
            at: name("stage"),
            said: String::new(),
            lines: 1,
            names: Vec::new(),
            modules: Vec::from([tab]),
        };
        assert!(stage.is_named("cox", &name("cox::stage::Stage")));
        assert!(stage.is_named("cox", &name("cox::stage::tab::Tab")));
        assert!(stage.is_named("cox", &name("cox::stage")));
        assert!(!stage.is_named("cox", &name("cox::ui::stage")));
        assert!(!stage.is_named("other", &name("cox::stage::Stage")));
        assert!(stage.file_is_named("cox", &name("cox::stage::Stage")));
        assert!(!stage.file_is_named("cox", &name("cox::stage::tab")));
        assert!(!stage.file_is_named("cox", &name("cox::stage::tab::Tab")));
        let lib = Module {
            name: "lib.rs".to_owned(),
            path: PathBuf::from("/p/src/lib.rs"),
            at: Vec::new(),
            said: String::new(),
            lines: 1,
            names: Vec::new(),
            modules: Vec::from([stage]),
        };
        assert!(lib.is_named("cox", &name("cox::stage::Stage")));
        assert!(lib.file_is_named("cox", &name("cox::Top")));
        assert!(!lib.file_is_named("cox", &name("cox::stage::Stage")));
        assert!(lib.is_named("cox", &name("cox")));
        assert!(lib.file_is_named("cox", &name("cox")));
        let all: Vec<String> = lib.names_in_all().map(|name| name.join("::")).collect();
        assert_eq!(all, Vec::<String>::new());
    }

    #[test]
    fn changes_lay_over_modules_and_crates_by_file() {
        let root = Path::new("/p");
        let tab = Module {
            name: "tab".to_owned(),
            path: root.join("crates/app/src/stage/tab.rs"),
            at: vec!["stage".to_owned(), "tab".to_owned()],
            said: String::new(),
            lines: 1,
            names: Vec::new(),
            modules: Vec::new(),
        };
        let stage = Module {
            name: "stage".to_owned(),
            path: root.join("crates/app/src/stage.rs"),
            at: vec!["stage".to_owned()],
            said: String::new(),
            lines: 1,
            names: Vec::new(),
            modules: Vec::from([tab]),
        };
        let app = Crate {
            name: "app".to_owned(),
            folder: root.join("crates/app"),
            uses: Vec::new(),
            entries: Vec::from([Module {
                name: "lib.rs".to_owned(),
                path: root.join("crates/app/src/lib.rs"),
                at: Vec::new(),
                said: String::new(),
                lines: 1,
                names: Vec::new(),
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
