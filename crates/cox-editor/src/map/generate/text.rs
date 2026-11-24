//! What the map shows of a symbol's documentation: its first sentence,
//! and its signature.

use crate::scip::SymbolInformation;

/// The most letters of a sentence or a signature the map keeps.
const LONGEST: usize = 200;

/// The first sentence of a symbol's documentation, without the blocks of
/// code some indexers put its signature in.
pub(super) fn doc_of(info: Option<&SymbolInformation>) -> String {
    let mut parts = Vec::new();
    for text in info.into_iter().flat_map(|info| &info.documentation) {
        let text = without_code(text);
        let text = text.trim();
        // A rule some indexers put between the signature and the rest.
        let dashes = text.chars().take_while(|&c| c == '-').count();
        let text = if dashes >= 3 {
            text[dashes..].trim()
        } else {
            text
        };
        if !text.is_empty() {
            parts.push(text.to_owned());
        }
    }
    let text = parts
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let chars: Vec<char> = text.chars().collect();
    let end = (1..chars.len())
        .find(|&at| {
            matches!(chars[at], '.' | ':' | '。')
                && chars.get(at + 1).is_none_or(|next| next.is_whitespace())
        })
        .map_or(chars.len(), |at| at + 1);
    chars[..end].iter().take(LONGEST).collect()
}

/// `text` without its blocks of code, from ```` ``` ```` to ```` ``` ````.
fn without_code(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let Some(length) = rest[start + 3..].find("```") else {
            break;
        };
        out.push_str(&rest[..start]);
        rest = &rest[start + 3 + length + 3..];
    }
    out.push_str(rest);
    out
}

/// A symbol's signature: as the index has it, in its own field or in a
/// block of code in its documentation, or else its definition's line.
pub(super) fn sig_of(info: Option<&SymbolInformation>, lines: &[String], line: usize) -> String {
    let signature = info
        .and_then(|info| info.signature_documentation.as_ref())
        .map(|signature| signature.text.as_str())
        .filter(|text| !text.is_empty());
    let from_documentation = || {
        info.into_iter()
            .flat_map(|info| &info.documentation)
            .find_map(|text| first_code(text))
            .filter(|code| !code.is_empty())
    };
    let text = signature
        .or_else(from_documentation)
        .or_else(|| lines.get(line).map(String::as_str))
        .unwrap_or("");
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(LONGEST)
        .collect()
}

/// What the first block of code in `text` holds, as ```` ```rust ````
/// and a line, up to ```` ``` ````.
fn first_code(text: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(found) = text[from..].find("```") {
        let start = from + found + 3;
        let info = text[start..]
            .char_indices()
            .find(|&(_, c)| !(c.is_alphanumeric() || c == '_'))
            .map_or(text.len() - start, |(at, _)| at);
        let body = start + info;
        if text[body..].starts_with('\n') {
            let body = body + 1;
            if let Some(end) = text[body..].find("```") {
                return Some(&text[body..body + end]);
            }
            return None;
        }
        from = from + found + 1;
    }
    None
}
