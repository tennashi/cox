//! The map of the chat's project, as a page of the stage draws it: the
//! picture of the project in tiers that an agent wrote in its
//! `.cox/map.json` (see [`cox_editor::map`]), with what Cox knows laid
//! over its boxes: what changed, as the Changes tab shows it, and which
//! boxes hold paths the project no longer has. The Map page reads it as
//! JSON from `cox://api/map` (see [`crate::stage::plugins`]), and the
//! agent's `show_map` tool opens the page at a tier or a box and hears
//! what is there.

use std::path::Path;

use cox_editor::changes;
use cox_editor::map::{self, MAP_FILE};

/// The map of the project in `root`, as JSON, for a page of the stage to
/// draw it (see [`crate::stage::plugins`]): the project's name and
/// folder; `file`, where the map is, or null when the project has none;
/// `error`, why it could not be read, if it could not; `since`, what the
/// changes are since, when the project is in a repository, and how many
/// files changed; `map`, the file as it is (see [`Map`]); and `facts`,
/// what Cox knows of each box by id (see [`Facts`]).
#[must_use]
pub fn told(root: &Path) -> serde_json::Value {
    let project = root.file_name().map_or_else(
        || root.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut json = serde_json::json!({
        "project": project,
        "root": root.display().to_string(),
        "file": serde_json::Value::Null,
        "error": serde_json::Value::Null,
        "since": serde_json::Value::Null,
        "changed_files": serde_json::Value::Null,
        "map": serde_json::Value::Null,
        "facts": {},
    });
    let map = match map::read(root) {
        Ok(Some(map)) => map,
        Ok(None) => return json,
        Err(error) => {
            json["file"] = serde_json::Value::String(root.join(MAP_FILE).display().to_string());
            json["error"] = serde_json::Value::String(error);
            return json;
        }
    };
    json["file"] = serde_json::Value::String(root.join(MAP_FILE).display().to_string());
    let diff = changes::read(root).ok();
    if let Some(diff) = &diff {
        json["since"] = serde_json::Value::String(diff.since.said());
        json["changed_files"] = serde_json::json!(diff.files.len());
    }
    let changed = diff
        .as_ref()
        .map(changes::changed_files)
        .unwrap_or_default();
    let facts: serde_json::Map<String, serde_json::Value> = map
        .parts
        .iter()
        .map(|(id, part)| {
            (
                id.clone(),
                serde_json::to_value(map::facts(root, part, &changed)).unwrap_or_default(),
            )
        })
        .collect();
    json["facts"] = serde_json::Value::Object(facts);
    json["map"] = serde_json::to_value(&map).unwrap_or_default();
    json
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
        let json = told(&root);
        assert_eq!(json["file"], serde_json::Value::Null);
        assert_eq!(json["map"], serde_json::Value::Null);
        assert_eq!(json["error"], serde_json::Value::Null);

        std::fs::create_dir_all(root.join(".cox")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join(MAP_FILE), A_MAP).unwrap();
        let json = told(&root);
        assert!(json["file"].as_str().unwrap().ends_with(".cox/map.json"));
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

        std::fs::write(root.join(MAP_FILE), r#"{ "tiers": 3 }"#).unwrap();
        let json = told(&root);
        assert!(json["error"].as_str().unwrap().contains("not a map"));
        assert_eq!(json["map"], serde_json::Value::Null);
    }
}
