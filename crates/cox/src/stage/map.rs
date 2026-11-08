//! A map of the chat's project, as a page of the stage draws it: a
//! picture of the project in tiers that an agent wrote in a file of
//! `.cox/maps` (see [`cox_editor::map`]), with what Cox knows laid
//! over its boxes: what changed, as the Changes tab shows it, and which
//! boxes hold paths the project no longer has. The Map page reads it as
//! JSON from `cox://api/map/<name>` (see [`crate::stage::plugins`]), and the
//! agent's `show_map` tool opens the page at a tier or a box and hears
//! what is there.

use std::path::Path;

use cox_editor::changes;
use cox_editor::map;

/// The map of the project in `root` named `name`, or its first unless
/// named, as JSON, for a page of the stage to draw it (see
/// [`crate::stage::plugins`]): the project's name and folder; `maps`, the
/// project's maps, each with its name, its file and what it says it
/// shows, for the page to go to another; `name`, the map's, or null when
/// the project has none; `file`, where the map is, or null when the
/// project has none; `error`, why it could not be read, if it could not;
/// `since`, what the changes are since, when the project is in a
/// repository, and how many files changed; `map`, the file as it is (see
/// [`map::Map`]); and `facts`, what Cox knows of each box by id (see
/// [`map::Facts`]), what changed in the boxes of its tier counted into
/// it.
#[must_use]
pub fn told(root: &Path, name: Option<&str>) -> serde_json::Value {
    let project = root.file_name().map_or_else(
        || root.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let maps = map::list(root);
    let mut json = serde_json::json!({
        "project": project,
        "root": root.display().to_string(),
        "maps": maps,
        "name": serde_json::Value::Null,
        "file": serde_json::Value::Null,
        "error": serde_json::Value::Null,
        "since": serde_json::Value::Null,
        "changed_files": serde_json::Value::Null,
        "map": serde_json::Value::Null,
        "facts": {},
    });
    let named = match name {
        None => maps.first(),
        Some(name) => maps.iter().find(|named| named.name == name),
    };
    if let Some(named) = named {
        json["name"] = serde_json::Value::String(named.name.clone());
        json["file"] = serde_json::Value::String(root.join(&named.file).display().to_string());
    }
    let map = match map::read(root, name) {
        Ok(Some((_, map))) => map,
        Ok(None) => return json,
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
    let changed = diff
        .as_ref()
        .map(changes::changed_files)
        .unwrap_or_default();
    let facts: serde_json::Map<String, serde_json::Value> = map::all_facts(root, &map, &changed)
        .into_iter()
        .map(|(id, facts)| (id, serde_json::to_value(facts).unwrap_or_default()))
        .collect();
    json["facts"] = serde_json::Value::Object(facts);
    json["map"] = serde_json::to_value(&map).unwrap_or_default();
    json
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cox_editor::map::MAPS_FOLDER;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("cox-stage-map-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::canonicalize(directory).unwrap()
    }

    const A_MAP: &str = r#"{
      "tiers": [
        { "id": "whole", "name": "Whole", "rows": [{ "parts": ["user", "app"] }] },
        { "id": "app", "name": "The app", "rows": [{ "parts": ["main", "gone"] }] }
      ],
      "parts": {
        "user": { "name": "User" },
        "app": { "name": "App", "tier": "app", "holds": ["src"] },
        "main": { "name": "main.rs", "holds": ["src/main.rs:2"] },
        "gone": { "name": "Gone", "holds": ["src/gone.rs"] }
      },
      "edges": [{ "from": "user", "to": "app", "says": "runs", "dir": "both" }]
    }"#;

    #[test]
    fn the_map_is_told_as_json_with_the_facts_laid_over() {
        let root = scratch("told");
        let json = told(&root, None);
        assert_eq!(json["file"], serde_json::Value::Null);
        assert_eq!(json["maps"], serde_json::json!([]));
        assert_eq!(json["map"], serde_json::Value::Null);
        assert_eq!(json["error"], serde_json::Value::Null);

        std::fs::create_dir_all(root.join(MAPS_FOLDER)).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join(MAPS_FOLDER).join("map.json"), A_MAP).unwrap();
        let json = told(&root, None);
        assert!(
            json["file"]
                .as_str()
                .unwrap()
                .ends_with(".cox/maps/map.json")
        );
        assert_eq!(json["name"], "map");
        assert_eq!(json["map"]["tiers"][0]["id"], "whole");
        assert_eq!(json["map"]["parts"]["app"]["tier"], "app");
        assert_eq!(json["map"]["edges"][0]["says"], "runs");
        assert_eq!(json["facts"]["main"]["holds"][0]["lines"], 1);
        assert_eq!(json["facts"]["main"]["holds"][0]["line"], 2);
        assert_eq!(
            json["facts"]["gone"]["stale"],
            serde_json::json!(["src/gone.rs"])
        );
        assert_eq!(json["facts"]["user"]["holds"], serde_json::json!([]));

        std::fs::write(root.join(MAPS_FOLDER).join("map.json"), r#"{ "tiers": 3 }"#).unwrap();
        let json = told(&root, None);
        assert!(json["error"].as_str().unwrap().contains("not a map"));
        assert_eq!(json["map"], serde_json::Value::Null);
    }

    #[test]
    fn a_map_is_told_by_its_name_with_the_others() {
        let root = scratch("named");
        std::fs::create_dir_all(root.join(MAPS_FOLDER)).unwrap();
        std::fs::write(root.join(MAPS_FOLDER).join("map.json"), A_MAP).unwrap();
        std::fs::write(
            root.join(MAPS_FOLDER).join("runtime.json"),
            r#"{ "says": "Where it runs.",
                 "tiers": [{ "id": "hosts", "name": "Hosts", "rows": [{ "parts": ["box"] }] }],
                 "parts": { "box": { "name": "A box" } } }"#,
        )
        .unwrap();

        let json = told(&root, Some("runtime"));
        assert_eq!(json["name"], "runtime");
        assert!(
            json["file"]
                .as_str()
                .unwrap()
                .ends_with(".cox/maps/runtime.json")
        );
        assert_eq!(json["map"]["says"], "Where it runs.");
        assert_eq!(json["map"]["tiers"][0]["id"], "hosts");
        assert_eq!(json["facts"]["box"]["holds"], serde_json::json!([]));
        assert_eq!(json["maps"][0]["name"], "map");
        assert_eq!(json["maps"][1]["name"], "runtime");
        assert_eq!(json["maps"][1]["says"], "Where it runs.");
        assert_eq!(json["maps"][1]["file"], ".cox/maps/runtime.json");
        assert_eq!(told(&root, None)["name"], "map");

        let json = told(&root, Some("gone"));
        assert_eq!(json["name"], serde_json::Value::Null);
        assert_eq!(json["map"], serde_json::Value::Null);
        assert!(json["error"].as_str().unwrap().contains("no map `gone`"));
        assert_eq!(json["maps"][1]["name"], "runtime");
    }
}
