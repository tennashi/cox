//! The lines of a file of the project around one, for a plugin to show
//! the code a step of a flow is made at, as the Map does when the mouse
//! rests on a step: `cox://api/lines/<path>:<line>` (see
//! [`crate::stage::plugins`]). Each line comes as its stretches, with
//! what the syntax says each is, as the code tabs color them.

use std::ops::Range;
use std::path::Path;

use cox_editor::buffer::Buffer;
use cox_editor::language::{Highlight, Language};
use cox_editor::syntax::Syntax;

/// How many lines before and after the one asked for are told.
const AROUND: usize = 4;

/// The lines of the file at `path`, an absolute path, in the project in
/// `root`, around `line`, as JSON, or `None` for a path outside the
/// project: the file and the line, whether the file can be read
/// (`exists`), the number of the first line told (`from`), and the lines,
/// [`AROUND`] before and after the line at most, or none for a file that
/// cannot be read or a line it does not have. Each line is its stretches
/// in order, each the text and what the syntax says it is, by the name
/// of its [`Highlight`], or null for nothing in particular, as for all
/// of a file in no language tree-sitter parses.
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
    let buffer = text.as_deref().map(Buffer::new);
    let syntax = buffer.as_ref().and_then(|buffer| {
        Language::of_path(&path).and_then(|language| Syntax::parse(language, buffer, None))
    });
    let count = buffer.as_ref().map_or(0, |buffer| {
        // The rope counts a line past a last line break, which is no line.
        let lines = buffer.lines();
        if lines > 0 && buffer.line(lines - 1).is_empty() {
            lines - 1
        } else {
            lines
        }
    });
    let from = line.saturating_sub(AROUND).max(1);
    let to = line.saturating_add(AROUND).min(count);
    let lines: Vec<serde_json::Value> = match &buffer {
        Some(buffer) if line >= 1 && from <= to => (from..=to)
            .map(|n| {
                let text = buffer.line(n - 1);
                let spans = syntax
                    .as_ref()
                    .map(|syntax| syntax.highlights(buffer, n - 1, usize::MAX))
                    .unwrap_or_default();
                serde_json::json!(stretches(&text, &spans))
            })
            .collect(),
        _ => Vec::new(),
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

/// `text` as its stretches, each with the name of what it is, or none:
/// those `spans` say, as bytes of the text, and the plain ones between.
/// A span off a character's edge, which the syntax does not give, makes
/// the whole line plain.
fn stretches(
    text: &str,
    spans: &[(Range<usize>, Highlight)],
) -> Vec<(String, Option<&'static str>)> {
    let mut out = Vec::new();
    let mut at = 0;
    for (range, highlight) in spans {
        let (Some(plain), Some(lit)) = (text.get(at..range.start), text.get(range.clone())) else {
            return vec![(text.to_owned(), None)];
        };
        if !plain.is_empty() {
            out.push((plain.to_owned(), None));
        }
        if !lit.is_empty() {
            out.push((lit.to_owned(), Some(highlight.name())));
        }
        at = range.end;
    }
    match text.get(at..) {
        Some(rest) if !rest.is_empty() => out.push((rest.to_owned(), None)),
        Some(_) => {}
        None => return vec![(text.to_owned(), None)],
    }
    out
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

    /// The text of a line told, its stretches joined.
    fn text_of(line: &serde_json::Value) -> String {
        line.as_array()
            .unwrap()
            .iter()
            .map(|stretch| stretch[0].as_str().unwrap())
            .collect()
    }

    #[test]
    fn the_lines_around_one_are_told_within_the_project() {
        let root = scratch("around");
        let text = (1..=20)
            .map(|n| format!("fn f{n}() {{}}\n"))
            .collect::<Vec<_>>()
            .concat();
        std::fs::write(root.join("a.rs"), text).unwrap();
        let json = told(&root, &root.join("a.rs"), 10).unwrap();
        assert_eq!(json["name"], "a.rs");
        assert_eq!(json["from"], 6);
        assert_eq!(json["lines"].as_array().unwrap().len(), 9);
        assert_eq!(text_of(&json["lines"][4]), "fn f10() {}");
        assert_eq!(json["lines"][4][0], serde_json::json!(["fn", "keyword"]));
        assert!(
            json["lines"][4]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s[1] == "function")
        );
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

    #[test]
    fn a_file_in_no_language_is_told_plain() {
        let root = scratch("plain");
        std::fs::write(root.join("notes.txt"), "one\ntwo\n").unwrap();
        let json = told(&root, &root.join("notes.txt"), 2).unwrap();
        assert_eq!(json["from"], 1);
        assert_eq!(
            json["lines"],
            serde_json::json!([[["one", null]], [["two", null]]])
        );
    }

    #[test]
    fn stretches_keep_the_text_whole() {
        let spans = [(0..2, Highlight::Keyword), (3..7, Highlight::Function)];
        let got = stretches("fn main() {}", &spans);
        assert_eq!(
            got,
            vec![
                ("fn".to_owned(), Some("keyword")),
                (" ".to_owned(), None),
                ("main".to_owned(), Some("function")),
                ("() {}".to_owned(), None),
            ]
        );
        // A span off a character's edge makes the line plain.
        assert_eq!(
            stretches("éa", &[(1..2, Highlight::Keyword)]),
            vec![("éa".to_owned(), None)]
        );
    }
}
