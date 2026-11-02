//! What the server answers, read from the JSON it says it in: what it
//! says of a name, where the name is defined and used, and the symbols a
//! file defines.

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

/// A symbol a file defines, such as a function or a type, as the server
/// lists them: its name and kind, what the server adds of it, such as a
/// signature, where it is, where its name is within that, and the
/// symbols within it, such as the methods of an `impl`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The kind's name, in lower case, as `function`, `struct` or `module`.
    pub kind: String,
    pub detail: Option<String>,
    /// The whole of it.
    pub range: Range<Position>,
    /// Its name, within `range`.
    pub selection: Range<Position>,
    pub children: Vec<Symbol>,
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

/// The symbols `value` says a file defines: in a tree, as a server that
/// supports that lists them, or else as a flat list, each with the file
/// it is in, which is then read as flat, leaving out those in another
/// file.
pub(super) fn symbols(value: &Value) -> Vec<Symbol> {
    let Value::Array(values) = value else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|value| {
            let name = value.get("name")?.as_str()?.to_owned();
            let kind = kind_name(value.get("kind").and_then(Value::as_u64).unwrap_or(0));
            let detail = value
                .get("detail")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|detail| !detail.is_empty())
                .map(str::to_owned);
            let (range, selection) = if let Some(location) = value.get("location") {
                // The flat form: a location, and nothing within it.
                let whole = range(location.get("range")?)?;
                (whole.clone(), whole)
            } else {
                let whole = range(value.get("range")?)?;
                let selection = value
                    .get("selectionRange")
                    .and_then(range)
                    .unwrap_or_else(|| whole.clone());
                (whole, selection)
            };
            Some(Symbol {
                name,
                kind,
                detail,
                range,
                selection,
                children: value.get("children").map(symbols).unwrap_or_default(),
            })
        })
        .collect()
}

/// The name of the kind of symbol the protocol numbers `kind`.
fn kind_name(kind: u64) -> String {
    let name = match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type parameter",
        _ => "symbol",
    };
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::{Hover, Location, hover, locations, symbols};
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

    #[test]
    fn reads_symbols_as_a_tree_or_flat() {
        let at = |line: u32, from: u32, to: u32| {
            json!({
                "start": { "line": line, "character": from },
                "end": { "line": line, "character": to },
            })
        };
        let tree = json!([
            {
                "name": "Stage", "kind": 23, "detail": "pub struct Stage",
                "range": at(10, 0, 40), "selectionRange": at(10, 11, 16),
                "children": [
                    { "name": "cwd", "kind": 8, "range": at(12, 4, 16), "selectionRange": at(12, 4, 7) },
                ],
            },
            { "name": "new", "kind": 12, "detail": "  ", "range": at(50, 0, 1) },
            { "kind": 12, "range": at(60, 0, 1) },
        ]);
        let read = symbols(&tree);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].name, "Stage");
        assert_eq!(read[0].kind, "struct");
        assert_eq!(read[0].detail.as_deref(), Some("pub struct Stage"));
        assert_eq!(read[0].selection.start.line, 10);
        assert_eq!(read[0].selection.start.character, 11);
        assert_eq!(read[0].children[0].name, "cwd");
        assert_eq!(read[0].children[0].kind, "field");
        assert_eq!(read[1].name, "new");
        assert_eq!(read[1].kind, "function");
        assert_eq!(read[1].detail, None);
        // Without a name of its own, the selection is the whole.
        assert_eq!(read[1].selection, read[1].range);

        let flat = json!([
            {
                "name": "main", "kind": 12,
                "location": { "uri": "file:///src/main.rs", "range": at(0, 0, 12) },
            },
            { "name": "odd", "kind": 99, "location": { "uri": "file:///src/main.rs", "range": at(3, 0, 1) } },
        ]);
        let read = symbols(&flat);
        assert_eq!(read[0].name, "main");
        assert_eq!(read[0].range.start.line, 0);
        assert_eq!(read[0].children, Vec::<super::Symbol>::new());
        assert_eq!(read[1].kind, "symbol");
        assert_eq!(symbols(&Value::Null), []);
    }
}
