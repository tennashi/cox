//! The map of the chat's project: its crates, and the modules in each,
//! as boxes, each with the first paragraph of what its file says of
//! itself, for the human to see where things are before they read them
//! (see [`cox_editor::map`]). The map is open at a crate, and at the
//! modules in it from the crate down, or at the crates; a box with
//! modules in it opens the map at that box, and a file opens on a tab of
//! its own. What changed in the project, as the Changes tab shows it, is
//! laid over the map: a box with a file that changed in it is marked,
//! with how many lines changed. Each box has to do with the others: the
//! boxes it uses, and those that use it, from what the files name (see
//! [`cox_editor::map::Module::names`]), or, for crates, from their
//! manifests.
//!
//! The stage's Map page draws it, from what [`told`] says of it as JSON
//! at `cox://api/map/<crate>/<module>…` (see [`crate::stage::plugins`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cox_editor::changes;
use cox_editor::map::{self, Changed, Crate, Module};

use crate::ui::paths::shortened;

/// What a box of the map shows.
enum Shown<'a> {
    Crate(&'a Crate),
    /// A file: one a crate starts from, or that of the module the map is
    /// open at.
    File(&'a Module),
    /// A module in the crate or module the map is open at.
    Module(&'a Module),
}

/// Where the map is open, as far as the map has it, and what it shows
/// there.
struct Place<'a> {
    /// How many of the names the map is open at the map has, from the
    /// first.
    depth: usize,
    /// The crate the map is open in, unless it is open at the crates.
    krate: Option<&'a Crate>,
    boxes: Vec<Shown<'a>>,
}

impl<'a> Place<'a> {
    /// Where `map` is open `at`, and what it shows there: the crates, or
    /// the files and modules of the crate or module the names lead to.
    fn of(map: &'a map::Map, at: &[String]) -> Self {
        let place = map.place(at);
        let boxes = match place.krate {
            None => map.crates.iter().map(Shown::Crate).collect(),
            Some(_) => place
                .files
                .into_iter()
                .map(Shown::File)
                .chain(place.modules.into_iter().map(Shown::Module))
                .collect(),
        };
        Self {
            depth: place.depth,
            krate: place.krate,
            boxes,
        }
    }
}

impl Shown<'_> {
    /// The name the box shows.
    fn label(&self) -> String {
        match self {
            Shown::Crate(krate) => krate.name.clone(),
            Shown::File(module) => module.path.file_name().map_or_else(
                || module.name.clone(),
                |name| name.to_string_lossy().into_owned(),
            ),
            Shown::Module(module) => module.name.clone(),
        }
    }

    /// What the files of the box name (see [`Module::names`]).
    fn names(&self) -> Vec<&[String]> {
        match self {
            Shown::Crate(krate) => krate
                .entries
                .iter()
                .flat_map(Module::names_in_all)
                .collect(),
            Shown::File(module) => module.names.iter().map(Vec::as_slice).collect(),
            Shown::Module(module) => module.names_in_all().collect(),
        }
    }

    /// Whether `name` leads into the box, in crate `krate` when the box
    /// is in one. A file a crate starts from holds what is at the top of
    /// the crate and in none of its modules, whichever file declares
    /// them: `crate::` in `main.rs` means `lib.rs` as much as itself.
    fn holds(&self, krate: Option<&Crate>, name: &[String]) -> bool {
        match (self, krate) {
            (Shown::Crate(this), _) => name.first() == Some(&this.name),
            (Shown::File(module), Some(krate)) if module.at.is_empty() => match name {
                [first] => *first == krate.name,
                [first, one] => {
                    *first == krate.name && !krate.modules().iter().any(|m| m.name == *one)
                }
                _ => false,
            },
            (Shown::File(module), Some(krate)) => module.file_is_named(&krate.name, name),
            (Shown::Module(module), Some(krate)) => module.is_named(&krate.name, name),
            (Shown::File(_) | Shown::Module(_), None) => false,
        }
    }

    /// Whether the box uses `other`: a crate by its manifest, and the
    /// rest by what their files name.
    fn uses(&self, other: &Self, krate: Option<&Crate>) -> bool {
        match (self, other) {
            (Shown::Crate(this), Shown::Crate(other)) => this.uses.contains(&other.name),
            _ => self.names().iter().any(|name| other.holds(krate, name)),
        }
    }
}

impl Shown<'_> {
    /// What the box is, in a word: `crate`, `file` or `module`.
    fn kind(&self) -> &'static str {
        match self {
            Shown::Crate(_) => "crate",
            Shown::File(_) => "file",
            Shown::Module(_) => "module",
        }
    }

    /// What changed in what the box shows, among `files`.
    fn changed(&self, files: Option<&HashMap<PathBuf, (usize, usize)>>) -> Changed {
        let Some(files) = files else {
            return Changed::default();
        };
        match self {
            Shown::Crate(krate) => krate.changed(files),
            Shown::File(module) | Shown::Module(module) => module.changed(files),
        }
    }
}

/// The map of the project in `root`, open `at`, as JSON, for a page of
/// the stage to draw it (see [`crate::stage::plugins`]): the project's
/// name, the names the map is open at, as far as it has them, and the
/// one it has not when it has not them all; what changed since, when
/// the project is in a repository; and the boxes, each with its kind, its
/// name, its path (a file to open, or a crate's folder), what it says of
/// itself, its counts, what changed in it, and which of the other boxes
/// it uses, by their places among the boxes. When the project cannot be
/// mapped, `error` says why, and there are no boxes.
#[must_use]
pub fn told(root: &Path, at: &[String]) -> serde_json::Value {
    let project = root.file_name().map_or_else(
        || root.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut json = serde_json::json!({
        "project": project,
        "root": root.display().to_string(),
        "at": [],
        "missing": serde_json::Value::Null,
        "error": serde_json::Value::Null,
        "since": serde_json::Value::Null,
        "changed_files": serde_json::Value::Null,
        "boxes": [],
    });
    let read = match map::read(root) {
        Ok(map) if map.crates.is_empty() => Err(format!("No crate in {}", shortened(&map.root))),
        Ok(map) => Ok(map),
        Err(error) => Err(format!("Cannot map the project: {error}")),
    };
    let map = match read {
        Ok(map) => map,
        Err(error) => {
            json["error"] = serde_json::Value::String(error);
            return json;
        }
    };
    let diff = changes::read(root).ok();
    if let Some(diff) = &diff {
        json["since"] = serde_json::Value::String(diff.since.said());
        json["changed_files"] = serde_json::json!(diff.files.len());
    }
    let files = diff.as_ref().map(map::changed_files);
    let place = Place::of(&map, at);
    json["at"] = serde_json::json!(at[..place.depth]);
    if place.depth < at.len() {
        json["missing"] = serde_json::Value::String(at[place.depth].clone());
    }
    let boxes: Vec<serde_json::Value> = place
        .boxes
        .iter()
        .enumerate()
        .map(|(index, shown)| {
            let uses: Vec<usize> = place
                .boxes
                .iter()
                .enumerate()
                .filter(|(other, box_)| *other != index && shown.uses(box_, place.krate))
                .map(|(other, _)| other)
                .collect();
            let changed = shown.changed(files.as_ref());
            let (path, lines, modules, crates, opens) = match *shown {
                Shown::Crate(krate) => (&krate.folder, krate.lines(), None, &krate.uses[..], false),
                Shown::File(module) => (&module.path, module.lines, None, &[][..], true),
                Shown::Module(module) if module.modules.is_empty() => {
                    (&module.path, module.lines, None, &[][..], true)
                }
                Shown::Module(module) => (
                    &module.path,
                    module.lines_in_all(),
                    Some(module.modules_in_all()),
                    &[][..],
                    false,
                ),
            };
            let said = match *shown {
                Shown::Crate(krate) => krate.said(),
                Shown::File(module) | Shown::Module(module) => module.said.as_str(),
            };
            serde_json::json!({
                "kind": shown.kind(),
                "name": shown.label(),
                "path": path.display().to_string(),
                "opens": opens,
                "said": said,
                "lines": lines,
                "modules": modules,
                "crates": crates,
                "changed": changed.any().then(|| serde_json::json!({
                    "files": changed.files,
                    "added": changed.added,
                    "removed": changed.removed,
                })),
                "uses": uses,
            })
        })
        .collect();
    json["boxes"] = serde_json::Value::Array(boxes);
    json
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("cox-map-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::canonicalize(directory).unwrap()
    }

    #[test]
    fn the_map_is_told_as_json_with_what_each_box_uses() {
        let root = scratch("told");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"one\"\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "//! One.\nmod a;\nmod b;\npub fn f() {\n    crate::a::g();\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("src/a.rs"), "//! A.\npub fn g() {}\n").unwrap();
        std::fs::write(root.join("src/b.rs"), "mod c;\n").unwrap();
        std::fs::create_dir_all(root.join("src/b")).unwrap();
        std::fs::write(root.join("src/b/c.rs"), "use crate::a::g;\n").unwrap();

        let json = told(&root, &[]);
        assert_eq!(json["error"], serde_json::Value::Null);
        assert_eq!(json["at"], serde_json::json!([]));
        assert_eq!(json["boxes"][0]["kind"], "crate");
        assert_eq!(json["boxes"][0]["name"], "one");
        assert_eq!(json["boxes"][0]["said"], "One.");
        assert_eq!(json["boxes"][0]["opens"], false);
        assert_eq!(json["boxes"][0]["lines"], 10);

        let json = told(&root, &["one".to_owned()]);
        assert_eq!(json["at"], serde_json::json!(["one"]));
        let names: Vec<&str> = json["boxes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|box_| box_["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["lib.rs", "a", "b"]);
        assert_eq!(json["boxes"][0]["kind"], "file");
        assert_eq!(json["boxes"][0]["opens"], true);
        assert_eq!(json["boxes"][0]["uses"], serde_json::json!([1]));
        assert_eq!(json["boxes"][1]["kind"], "module");
        assert_eq!(json["boxes"][1]["uses"], serde_json::json!([]));
        assert_eq!(json["boxes"][2]["modules"], 1);
        assert_eq!(json["boxes"][2]["opens"], false);
        assert_eq!(json["boxes"][2]["uses"], serde_json::json!([1]));
        assert!(
            json["boxes"][1]["path"]
                .as_str()
                .unwrap()
                .ends_with("src/a.rs")
        );

        let json = told(&root, &["one".to_owned(), "nope".to_owned()]);
        assert_eq!(json["at"], serde_json::json!(["one"]));
        assert_eq!(json["missing"], "nope");
        assert_eq!(json["boxes"].as_array().unwrap().len(), 3);

        let json = told(&scratch("empty"), &[]);
        assert!(json["error"].as_str().unwrap().starts_with("Cannot map"));
        assert_eq!(json["boxes"], serde_json::json!([]));
    }
}
