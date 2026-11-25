//! The maps of a project: pictures of it, for the human to see where
//! things are before they read them, and to talk through with the agents
//! where what the project's parts mean and its code disagree. A project
//! can have several, each from a viewpoint of its own, as one of what
//! runs when someone uses it and one of how it is built: each is a file
//! of the project, in [`MAPS_FOLDER`], named after the map, so that it is
//! there whenever it is opened, kept by the agents that change the code,
//! in the change that moves it. A repository of the project keeps maps
//! of its own the same way, in its own [`MAPS_FOLDER`], to version them
//! with its code: the project has them too, each named after the
//! repository and the map (see [`list`]).
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
    /// The map's name: its file's, without `.json`, after its
    /// repository's name and `/` when a repository of the project keeps
    /// it, as `cox/use`.
    pub name: String,
    /// Its file, from the project's folder.
    pub file: String,
    /// The folder the files it claims are from, from the project's, with
    /// a `/` after it: its repository's, as `directories/cox/`, or empty
    /// for the project's own (see [`generate::generate`]).
    #[serde(skip)]
    pub files_from: String,
    /// What the map says it shows, if its file says and reads.
    pub says: String,
}

/// The maps of the project in `root`: the files of its own
/// [`MAPS_FOLDER`], by name, then those of each of its `repositories`,
/// each by its name and its folder from `root`, as `("cox",
/// "directories/cox")`, named after the repository and the map, as
/// `cox/use`. A file's name has no `/`, so the names of the project's
/// own maps and those of its repositories' are never the same; and a
/// chat that works in a checkout of a repository, whose folder is the
/// root, has its maps by their own names.
#[must_use]
pub fn list(root: &Path, repositories: &[(String, String)]) -> Vec<Named> {
    let mut maps = in_folder(root, "", "");
    for (repository, folder) in repositories {
        maps.extend(in_folder(
            root,
            &format!("{}/", folder.trim_end_matches('/')),
            &format!("{repository}/"),
        ));
    }
    maps
}

/// The maps in [`MAPS_FOLDER`] in the folder `within` of `root`, from
/// it with a `/` after, or empty for `root` itself, by name, each named
/// after `qualified` and its file.
fn in_folder(root: &Path, within: &str, qualified: &str) -> Vec<Named> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(within).join(MAPS_FOLDER))
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
            let file = format!("{within}{MAPS_FOLDER}/{name}.json");
            let says = said(&root.join(&file));
            Named {
                name: format!("{qualified}{name}"),
                file,
                files_from: within.to_owned(),
                says,
            }
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
        assert_eq!(list(&root, &[]), []);
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

        let listed: Vec<String> = list(&root, &[])
            .into_iter()
            .map(|named| named.name)
            .collect();
        // A name with `:` in it is one too: the Map page's hash encodes it.
        assert_eq!(listed, ["a:b", "build", "use", "wrong"]);
        let named = &list(&root, &[])[2];
        assert_eq!(named.file, "maps/use.json");
        assert_eq!(named.files_from, "");
        assert_eq!(named.says, "What runs.");
        // A file that does not read says nothing.
        assert_eq!(list(&root, &[])[3].says, "");
    }

    #[test]
    fn a_repositorys_maps_are_the_projects_by_its_name() {
        let root = Folder::new();
        let write = |file: &str, says: &str| {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!(r#"{{ "says": "{says}" }}"#)).unwrap();
        };
        write("maps/use.json", "The project's.");
        write("directories/app/maps/use.json", "The app's.");
        write("directories/app/maps/build.json", "How the app is built.");
        write("directories/web/maps/use.json", "The web's.");
        // A repository the project is not told of has none of its maps.
        write("directories/other/maps/use.json", "Not told.");
        let repositories = [
            ("app".to_owned(), "directories/app".to_owned()),
            ("web".to_owned(), "directories/web/".to_owned()),
        ];

        let listed: Vec<(String, String, String, String)> = list(&root, &repositories)
            .into_iter()
            .map(|named| (named.name, named.file, named.files_from, named.says))
            .collect();
        let named = |name: &str, file: &str, from: &str, says: &str| {
            (
                name.to_owned(),
                file.to_owned(),
                from.to_owned(),
                says.to_owned(),
            )
        };
        // The project's own first, then each repository's, by name: two of
        // the same name are both there.
        assert_eq!(
            listed,
            [
                named("use", "maps/use.json", "", "The project's."),
                named(
                    "app/build",
                    "directories/app/maps/build.json",
                    "directories/app/",
                    "How the app is built."
                ),
                named(
                    "app/use",
                    "directories/app/maps/use.json",
                    "directories/app/",
                    "The app's."
                ),
                named(
                    "web/use",
                    "directories/web/maps/use.json",
                    "directories/web/",
                    "The web's."
                ),
            ]
        );
        // A chat working in the repository's checkout has them by their
        // own names, as ever.
        let app: Vec<String> = list(&root.join("directories/app"), &[])
            .into_iter()
            .map(|named| named.name)
            .collect();
        assert_eq!(app, ["build", "use"]);
    }
}
