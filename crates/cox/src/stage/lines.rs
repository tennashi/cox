//! The lines of a file of the project around one, for a plugin to show
//! the code a step of a flow is made at, as the Map does when the mouse
//! rests on a step: `cox://api/lines/<path>:<line>` (see
//! [`crate::stage::plugins`]).

use std::path::Path;

/// How many lines before and after the one asked for are told.
const AROUND: usize = 4;

/// The lines of the file at `path`, an absolute path, in the project in
/// `root`, around `line`, as JSON, or `None` for a path outside the
/// project: the file and the line, whether the file can be read
/// (`exists`), the number of the first line told (`from`), and the lines,
/// [`AROUND`] before and after the line at most, or none for a file that
/// cannot be read or a line it does not have.
#[must_use]
pub fn told(root: &Path, path: &Path, line: usize) -> Option<serde_json::Value> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    if !path.starts_with(&root) {
        return None;
    }
    let name = path
        .strip_prefix(&root)
        .map_or_else(|_| path.display().to_string(), |p| p.display().to_string());
    let text = std::fs::read_to_string(&path).ok();
    let all: Vec<&str> = text
        .as_deref()
        .map(|text| text.lines().collect())
        .unwrap_or_default();
    let from = line.saturating_sub(AROUND).max(1);
    let to = line.saturating_add(AROUND).min(all.len());
    let lines: Vec<&str> = if line >= 1 && from <= to {
        all[from - 1..to].to_vec()
    } else {
        Vec::new()
    };
    Some(serde_json::json!({
        "path": path.display().to_string(),
        "name": name,
        "line": line,
        "exists": text.is_some(),
        "from": from,
        "lines": lines,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cox-lines-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_lines_around_one_are_told_within_the_project() {
        let root = scratch("around");
        let text = (1..=20)
            .map(|n| format!("line {n}\n"))
            .collect::<Vec<_>>()
            .concat();
        std::fs::write(root.join("a.rs"), text).unwrap();
        let json = told(&root, &root.join("a.rs"), 10).unwrap();
        assert_eq!(json["name"], "a.rs");
        assert_eq!(json["from"], 6);
        assert_eq!(json["lines"].as_array().unwrap().len(), 9);
        assert_eq!(json["lines"][4], "line 10");
        let json = told(&root, &root.join("a.rs"), 2).unwrap();
        assert_eq!(json["from"], 1);
        assert_eq!(json["lines"].as_array().unwrap().len(), 6);
        let json = told(&root, &root.join("a.rs"), 20).unwrap();
        assert_eq!(json["from"], 16);
        assert_eq!(json["lines"].as_array().unwrap().len(), 5);
        let json = told(&root, &root.join("a.rs"), 40).unwrap();
        assert_eq!(json["lines"], serde_json::json!([]));
        let json = told(&root, &root.join("none.rs"), 1).unwrap();
        assert_eq!(json["exists"], false);
        assert_eq!(json["lines"], serde_json::json!([]));
        assert_eq!(told(&root, Path::new("/etc/hostname"), 1), None);
    }
}
