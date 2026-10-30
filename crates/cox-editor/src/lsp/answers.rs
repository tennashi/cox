//! What the server answers about a name, read from the JSON it says it
//! in: what it says of the name, and where the name is defined and used.

use std::ops::Range;
use std::path::PathBuf;

use serde_json::Value;
use url::Url;

use super::Position;

/// What the server says of a name: its type and its documentation, as
/// Markdown, and the stretch of the line it is about, if it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub markdown: String,
    pub range: Option<Range<Position>>,
}

/// A stretch of a file, such as where a name is defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// The file, as an absolute path.
    pub path: PathBuf,
    pub range: Range<Position>,
}

/// The position `value` says.
fn position(value: &Value) -> Option<Position> {
    Some(Position {
        line: u32::try_from(value.get("line")?.as_u64()?).ok()?,
        character: u32::try_from(value.get("character")?.as_u64()?).ok()?,
    })
}

/// The stretch `value` says.
fn range(value: &Value) -> Option<Range<Position>> {
    Some(position(value.get("start")?)?..position(value.get("end")?)?)
}

/// The hover `value` says, if it says anything.
pub(super) fn hover(value: &Value) -> Option<Hover> {
    let markdown = markdown(value.get("contents")?);
    if markdown.trim().is_empty() {
        return None;
    }
    Some(Hover {
        markdown,
        range: value.get("range").and_then(range),
    })
}

/// The contents of a hover, as Markdown: markup, or a string of the older
/// form, or a list of them.
fn markdown(contents: &Value) -> String {
    match contents {
        Value::String(markdown) => markdown.clone(),
        Value::Array(contents) => contents
            .iter()
            .map(markdown)
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(contents) => {
            let value = contents
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match (
                contents.get("kind").and_then(Value::as_str),
                contents.get("language").and_then(Value::as_str),
            ) {
                (Some("markdown"), _) => value.to_owned(),
                (_, Some(language)) => format!("```{language}\n{value}\n```"),
                // Plain text, which Markdown draws as it is in a code block.
                _ => format!("```\n{value}\n```"),
            }
        }
        _ => String::new(),
    }
}

/// The locations `value` says: one, a list of them, or of links to them,
/// leaving out those not in a file.
pub(super) fn locations(value: &Value) -> Vec<Location> {
    let one = |value: &Value| {
        // A link goes to the name, within the whole of what it links to.
        let (uri, stretch) = match value.get("targetUri") {
            Some(uri) => (
                uri,
                value
                    .get("targetSelectionRange")
                    .or_else(|| value.get("targetRange"))?,
            ),
            None => (value.get("uri")?, value.get("range")?),
        };
        Some(Location {
            path: Url::parse(uri.as_str()?).ok()?.to_file_path().ok()?,
            range: range(stretch)?,
        })
    };
    match value {
        Value::Array(values) => values.iter().filter_map(one).collect(),
        Value::Object(_) => one(value).into_iter().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::{Hover, Location, hover, locations};
    use crate::lsp::Position;

    #[test]
    fn reads_hovers_of_every_form() {
        let markup = json!({
            "contents": { "kind": "markdown", "value": "```rust\nfn main()\n```" },
            "range": {
                "start": { "line": 1, "character": 3 },
                "end": { "line": 1, "character": 7 },
            },
        });
        assert_eq!(
            hover(&markup),
            Some(Hover {
                markdown: "```rust\nfn main()\n```".to_owned(),
                range: Some(
                    Position {
                        line: 1,
                        character: 3
                    }..Position {
                        line: 1,
                        character: 7
                    }
                ),
            })
        );
        let marked = json!({
            "contents": [{ "language": "rust", "value": "fn main()" }, "Runs."],
        });
        assert_eq!(
            hover(&marked).unwrap().markdown,
            "```rust\nfn main()\n```\n\nRuns."
        );
        assert_eq!(hover(&json!({ "contents": "" })), None);
        assert_eq!(hover(&Value::Null), None);
    }

    #[test]
    fn reads_locations_and_links_to_them() {
        let range = json!({
            "start": { "line": 4, "character": 7 },
            "end": { "line": 4, "character": 11 },
        });
        let whole = json!({
            "start": { "line": 3, "character": 0 },
            "end": { "line": 9, "character": 1 },
        });
        let expected = Location {
            path: PathBuf::from("/src/main.rs"),
            range: Position {
                line: 4,
                character: 7,
            }..Position {
                line: 4,
                character: 11,
            },
        };
        let location = json!({ "uri": "file:///src/main.rs", "range": range });
        assert_eq!(locations(&location), std::slice::from_ref(&expected));
        let links = json!([
            { "targetUri": "file:///src/main.rs", "targetRange": whole, "targetSelectionRange": range },
            { "targetUri": "untitled:Untitled-1", "targetRange": whole },
        ]);
        assert_eq!(locations(&links), [expected]);
        assert_eq!(locations(&Value::Null), []);
    }
}
