//! The maps of a project: pictures of it, for the human to see where
//! things are before they read them, and to talk through with the agents
//! where what the project's parts mean and its code disagree. A project
//! can have several, each from a viewpoint of its own, as one of what
//! runs when someone uses it and one of how it is built: each is a file
//! of the project, in [`MAPS_FOLDER`], named after the map, so that it is
//! there whenever it is opened, kept by the agents that change the code,
//! in the change that moves it.
//!
//! The file is the map's [`vocabulary`]: what the code cannot say, the
//! boxes and where they sit, and the code each box owns. [`generate`]
//! reads the rest from an index of the code, into the map as
//! [`generated`], with where the two disagree.

pub mod generate;
pub mod generated;
pub mod vocabulary;

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Where a project keeps its maps, under its folder: a file each, as
/// `maps/use.json` for the map named `use`.
pub const MAPS_FOLDER: &str = "maps";

/// A map of a project, by its name, before it is read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Named {
    /// The map's name: its file's, without `.json`.
    pub name: String,
    /// Its file, from the project's folder.
    pub file: String,
    /// What the map says it shows, if its file says and reads.
    pub says: String,
}

/// The maps of the project in `root`: the files of [`MAPS_FOLDER`], by
/// name.
#[must_use]
pub fn list(root: &Path) -> Vec<Named> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(MAPS_FOLDER))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let name = name.strip_suffix(".json")?;
            (!name.is_empty()).then(|| name.to_owned())
        })
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let file = format!("{MAPS_FOLDER}/{name}.json");
            let says = said(&root.join(&file));
            Named { name, file, says }
        })
        .collect()
}

/// What the map in `path` says it shows, or nothing when it says
/// nothing, or does not read.
fn said(path: &Path) -> String {
    #[derive(Deserialize)]
    struct Says {
        #[serde(default)]
        says: String,
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Says>(&text).ok())
        .map_or_default(|says| says.says)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Folder;

    #[test]
    fn a_project_has_maps_by_name() {
        let root = Folder::new();
        assert_eq!(list(&root), []);
        std::fs::create_dir_all(root.join(MAPS_FOLDER)).unwrap();
        std::fs::write(
            root.join(MAPS_FOLDER).join("use.json"),
            r#"{ "says": "What runs.", "nodes": {}, "top": { "center": "app" } }"#,
        )
        .unwrap();
        std::fs::write(
            root.join(MAPS_FOLDER).join("build.json"),
            r#"{ "says": "How it is built." }"#,
        )
        .unwrap();
        std::fs::write(root.join(MAPS_FOLDER).join("wrong.json"), "{").unwrap();
        // The one file an earlier Cox read is no map now, nor one in
        // `.cox/maps`, where Cox kept them before.
        std::fs::write(root.join("map.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join(".cox/maps")).unwrap();
        std::fs::write(root.join(".cox/maps/old.json"), "{}").unwrap();
        std::fs::write(root.join(MAPS_FOLDER).join("a:b.json"), "{}").unwrap();
        std::fs::write(root.join(MAPS_FOLDER).join("notes.md"), "").unwrap();

        let listed: Vec<String> = list(&root).into_iter().map(|named| named.name).collect();
        // A name with `:` in it is one too: the Map page's hash encodes it.
        assert_eq!(listed, ["a:b", "build", "use", "wrong"]);
        let named = &list(&root)[2];
        assert_eq!(named.file, "maps/use.json");
        assert_eq!(named.says, "What runs.");
        // A file that does not read says nothing.
        assert_eq!(list(&root)[3].says, "");
    }
}
