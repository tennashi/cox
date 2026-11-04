//! The syntax of a text, as tree-sitter parses it in its language, and
//! what each stretch of each line is, to draw it so.
//!
//! Code in one language can hold code in another, as a Markdown code block
//! holds Rust: the syntax is in layers, the text's language first, then
//! each stretch of code in another, as the language's injections query
//! finds it, parsed on its own.
//!
//! As the text changes, the syntax is edited at once, which keeps it in
//! step with the text, though stale within what changed, and then parsed
//! again, which takes what did not change from the syntax before.

use std::cmp::Reverse;
use std::ops::Range;

use tree_sitter::{
    InputEdit, Node, Parser, QueryCursor, StreamingIterator as _, TextProvider, Tree,
};

use crate::buffer::{Buffer, Edit, Point};
use crate::language::{Capture, Highlight, Language};

/// How deep code within code goes, as Rust in a Rust macro within
/// Markdown does.
const MOST_DEPTH: usize = 3;

/// The syntax of a text.
#[derive(Clone, Debug)]
pub struct Syntax {
    /// The text's language's first, then the injected ones, each after the
    /// layer it is within.
    layers: Vec<Layer>,
}

#[derive(Clone, Debug)]
struct Layer {
    language: &'static Language,
    tree: Tree,
    /// The stretches of the text the layer was parsed in, or none for the
    /// first, which is parsed in all of it.
    ranges: Vec<tree_sitter::Range>,
    /// How many layers it is within.
    depth: usize,
}

impl Layer {
    /// Whether the layer is in any of the bytes of `range`.
    fn covers(&self, range: &Range<usize>) -> bool {
        self.ranges.is_empty()
            || self
                .ranges
                .iter()
                .any(|within| within.start_byte < range.end && range.start < within.end_byte)
    }
}

impl Syntax {
    /// Parses `buffer` in `language`, with what did not change from `old`,
    /// the syntax of the text before, edited as it changed since, if any.
    ///
    /// `None` if tree-sitter cannot parse the language, which the tests
    /// rule out.
    #[must_use]
    pub fn parse(language: &'static Language, buffer: &Buffer, old: Option<&Self>) -> Option<Self> {
        let mut parser = Parser::new();
        let old = old
            .and_then(|old| old.layers.first())
            .filter(|layer| layer.language == language)
            .map(|layer| &layer.tree);
        let tree = parse(&mut parser, language, buffer, &[], old)?;
        let mut layers = vec![Layer {
            language,
            tree,
            ranges: Vec::new(),
            depth: 0,
        }];
        // Each layer's injections, as layers of their own, which can have
        // injections too.
        let mut at = 0;
        while let Some(layer) = layers.get(at) {
            if layer.depth < MOST_DEPTH {
                let depth = layer.depth + 1;
                for (language, ranges) in injections(layer, buffer) {
                    if let Some(tree) = parse(&mut parser, language, buffer, &ranges, None) {
                        layers.push(Layer {
                            language,
                            tree,
                            ranges,
                            depth,
                        });
                    }
                }
            }
            at += 1;
        }
        Some(Self { layers })
    }

    /// The text changed as `edit` says: keeps the syntax in step with it
    /// until it is parsed again.
    pub fn edit(&mut self, edit: &Edit) {
        let edit = InputEdit {
            start_byte: edit.start,
            old_end_byte: edit.old_end,
            new_end_byte: edit.new_end,
            start_position: ts_point(edit.start_point),
            old_end_position: ts_point(edit.old_end_point),
            new_end_position: ts_point(edit.new_end_point),
        };
        for layer in &mut self.layers {
            layer.tree.edit(&edit);
            for range in &mut layer.ranges {
                edit.edit_range(range);
            }
        }
    }

    /// What the stretches of `line` of `buffer` are, within its first
    /// `most` bytes, in order: as bytes of the line, with the stretches
    /// that are nothing in particular left out.
    #[must_use]
    pub fn highlights(
        &self,
        buffer: &Buffer,
        line: usize,
        most: usize,
    ) -> Vec<(Range<usize>, Highlight)> {
        let start = buffer.line_start(line);
        let line = start..start + buffer.line(line).len().min(most);
        if line.is_empty() {
            return Vec::new();
        }
        // What each byte is: later layers, which are within earlier ones,
        // win, as do smaller stretches within larger ones and, for the
        // same stretch, later patterns.
        let mut bytes = vec![None; line.len()];
        let mut cursor = QueryCursor::new();
        for layer in self.layers.iter().filter(|layer| layer.covers(&line)) {
            let query = &layer.language.highlights;
            cursor.set_byte_range(line.clone());
            let mut captures = cursor.captures(query, layer.tree.root_node(), text(buffer));
            let mut found = Vec::new();
            while let Some((found_match, index)) = captures.next() {
                let capture = found_match.captures()[*index];
                let what = layer.language.captures[capture.index as usize];
                if what != Capture::Ignored {
                    found.push((capture.node.byte_range(), what));
                }
            }
            // Stable, so the same stretch keeps the order of the patterns.
            found.sort_by_key(|(range, _)| (range.start, Reverse(range.end)));
            for (range, what) in found {
                let from = range.start.clamp(line.start, line.end) - line.start;
                let to = range.end.clamp(line.start, line.end) - line.start;
                let highlight = match what {
                    Capture::Highlight(highlight) => Some(highlight),
                    Capture::Plain | Capture::Ignored => None,
                };
                bytes[from..to].fill(highlight);
            }
        }
        let mut spans: Vec<(Range<usize>, Highlight)> = Vec::new();
        for (at, highlight) in bytes.into_iter().enumerate() {
            let Some(highlight) = highlight else {
                continue;
            };
            match spans.last_mut() {
                Some((range, last)) if range.end == at && *last == highlight => range.end += 1,
                _ => spans.push((at..at + 1, highlight)),
            }
        }
        spans
    }

    /// The names called on `line` of `buffer`, as the bytes of the line
    /// each is at, in order, as its language's tags query finds them,
    /// through code within code, as calls within a Rust macro's arguments;
    /// `None` when no language of the line has a tags query, which says
    /// nothing either way.
    #[must_use]
    pub fn calls(&self, buffer: &Buffer, line: usize) -> Option<Vec<Range<usize>>> {
        let start = buffer.line_start(line);
        let range = start..start + buffer.line(line).len();
        let mut found = None;
        let mut cursor = QueryCursor::new();
        for layer in self.layers.iter().filter(|layer| layer.covers(&range)) {
            let Some(tags) = &layer.language.tags else {
                continue;
            };
            let found: &mut Vec<Range<usize>> = found.get_or_insert_with(Vec::new);
            cursor.set_byte_range(range.clone());
            let mut matches = cursor.matches(&tags.query, layer.tree.root_node(), text(buffer));
            while let Some(found_match) = matches.next() {
                let captures = found_match.captures();
                if !captures.iter().any(|capture| capture.index == tags.call) {
                    continue;
                }
                for capture in captures.iter().filter(|capture| capture.index == tags.name) {
                    let name = capture.node.byte_range();
                    if range.start <= name.start && name.end <= range.end {
                        found.push(name.start - start..name.end - start);
                    }
                }
            }
        }
        if let Some(found) = &mut found {
            found.sort_by_key(|name| (name.start, name.end));
            found.dedup();
        }
        found
    }

    /// The lines that open what `line` of `buffer` is within, outermost
    /// first, as a function's `fn` line and its impl's, through code
    /// within code, before the line itself: the first line of each
    /// definition around it, as its language's tags query finds them
    /// (`definition.` and its kind, and `reference.implementation`), or,
    /// for a language with none, of each node around it of a kind its
    /// [`Language::scopes`] names.
    #[must_use]
    pub fn within(&self, buffer: &Buffer, line: usize) -> Vec<usize> {
        let start = buffer.line_start(line);
        let said = buffer.line(line);
        let lead = said.len() - said.trim_start().len();
        let range = start + lead..start + said.trim_end().len().max(lead);
        let around = |node: Node<'_>| {
            let row = node.start_position().row;
            (row < line && node.start_byte() <= range.start && range.end <= node.end_byte())
                .then_some(row)
        };
        let mut found = Vec::new();
        let mut cursor = QueryCursor::new();
        for layer in self.layers.iter().filter(|layer| layer.covers(&range)) {
            let mut opens = Vec::new();
            if let Some(tags) = &layer.language.tags {
                cursor.set_byte_range(range.clone());
                let mut matches = cursor.matches(&tags.query, layer.tree.root_node(), text(buffer));
                while let Some(found_match) = matches.next() {
                    opens.extend(
                        found_match
                            .captures()
                            .iter()
                            .filter(|capture| tags.scopes.contains(&capture.index))
                            .filter_map(|capture| around(capture.node)),
                    );
                }
            } else {
                let scopes = layer.language.scopes();
                let mut node = layer
                    .tree
                    .root_node()
                    .descendant_for_byte_range(range.start, range.end);
                while let Some(at) = node {
                    if scopes.contains(&at.kind()) {
                        opens.extend(around(at));
                    }
                    node = at.parent();
                }
            }
            opens.sort_unstable();
            opens.dedup();
            found.extend(opens);
        }
        found.dedup();
        found
    }
}

/// Parses `buffer` in `language`, only in `ranges` if any, with what did
/// not change from `old`, if any.
fn parse(
    parser: &mut Parser,
    language: &'static Language,
    buffer: &Buffer,
    ranges: &[tree_sitter::Range],
    old: Option<&Tree>,
) -> Option<Tree> {
    parser.set_language(&language.grammar).ok()?;
    parser.set_included_ranges(ranges).ok()?;
    parser.parse_with_options(&mut |byte, _| buffer.chunk_at(byte), old, None)
}

/// The languages injected in `layer`, each with the stretches of the text
/// it is in.
fn injections(layer: &Layer, buffer: &Buffer) -> Vec<(&'static Language, Vec<tree_sitter::Range>)> {
    let Some(query) = &layer.language.injections else {
        return Vec::new();
    };
    let content = query.capture_index_for_name("injection.content");
    let named = query.capture_index_for_name("injection.language");
    let mut injected = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, layer.tree.root_node(), text(buffer));
    while let Some(found) = matches.next() {
        let properties = query.property_settings(found.pattern_index);
        let mut name = properties
            .iter()
            .find(|property| &*property.key == "injection.language")
            .and_then(|property| property.value.as_deref())
            .map(str::to_owned);
        let children = properties
            .iter()
            .any(|property| &*property.key == "injection.include-children");
        let mut ranges = Vec::new();
        for capture in found.captures() {
            if Some(capture.index) == named {
                name = Some(buffer.chunks(capture.node.byte_range()).collect());
            } else if Some(capture.index) == content {
                ranges.extend(ranges_of(capture.node, children));
            }
        }
        if let Some(language) = name.as_deref().and_then(Language::named)
            && !ranges.is_empty()
        {
            injected.push((language, ranges));
        }
    }
    injected
}

/// The stretches of `node`: all of it with its children, or else what is
/// between them, the named ones, as a Markdown paragraph's text leaves out
/// the `>` of the quote it is in.
fn ranges_of(node: Node<'_>, children: bool) -> Vec<tree_sitter::Range> {
    if children {
        return vec![node.range()];
    }
    let mut ranges = Vec::new();
    let (mut start_byte, mut start_point) = (node.start_byte(), node.start_position());
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        ranges.push(tree_sitter::Range {
            start_byte,
            end_byte: child.start_byte(),
            start_point,
            end_point: child.start_position(),
        });
        (start_byte, start_point) = (child.end_byte(), child.end_position());
    }
    ranges.push(tree_sitter::Range {
        start_byte,
        end_byte: node.end_byte(),
        start_point,
        end_point: node.end_position(),
    });
    ranges.retain(|range| range.start_byte < range.end_byte);
    ranges
}

/// The text of a node of `buffer`, for queries to match.
fn text(buffer: &Buffer) -> impl TextProvider<&[u8]> {
    |node: Node<'_>| buffer.chunks(node.byte_range()).map(str::as_bytes)
}

fn ts_point(point: Point) -> tree_sitter::Point {
    tree_sitter::Point::new(point.line, point.column)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stretches of `line` of `text`, in `language`, as the text they
    /// cover.
    fn highlights(language: &str, text: &str, line: usize) -> Vec<(String, Highlight)> {
        let buffer = Buffer::new(text);
        let syntax = Syntax::parse(Language::named(language).unwrap(), &buffer, None).unwrap();
        let line_text = buffer.line(line);
        syntax
            .highlights(&buffer, line, usize::MAX)
            .into_iter()
            .map(|(range, highlight)| (line_text[range].to_owned(), highlight))
            .collect()
    }

    fn has(found: &[(String, Highlight)], text: &str, highlight: Highlight) -> bool {
        found
            .iter()
            .any(|(found, what)| found == text && *what == highlight)
    }

    #[test]
    fn highlights_rust() {
        let text = "/// Runs.\nfn run(count: usize) -> String {\n    let s = \"a\\n\";\n}\n";
        let found = highlights("rust", text, 0);
        assert_eq!(found, [("/// Runs.".to_owned(), Highlight::Comment)]);
        let found = highlights("rust", text, 1);
        assert!(has(&found, "fn", Highlight::Keyword), "{found:?}");
        assert!(has(&found, "run", Highlight::Function), "{found:?}");
        assert!(has(&found, "usize", Highlight::TypeBuiltin), "{found:?}");
        assert!(has(&found, "String", Highlight::Type), "{found:?}");
        // Within the string, its escape.
        let found = highlights("rust", text, 2);
        assert!(has(&found, "\"a", Highlight::String), "{found:?}");
        assert!(has(&found, "\\n", Highlight::Escape), "{found:?}");
    }

    #[test]
    fn highlights_what_spans_lines_on_each() {
        let text = "/* one\ntwo\nthree */ fn x() {}\n";
        assert_eq!(
            highlights("rust", text, 1),
            [("two".to_owned(), Highlight::Comment)]
        );
        let found = highlights("rust", text, 2);
        assert!(has(&found, "three */", Highlight::Comment), "{found:?}");
        assert!(has(&found, "fn", Highlight::Keyword), "{found:?}");
    }

    #[test]
    fn highlights_rust_in_a_macro() {
        let found = highlights("rust", "fn x() { vec![run(1)]; }\n", 0);
        assert!(has(&found, "vec!", Highlight::Function), "{found:?}");
        assert!(has(&found, "run", Highlight::Function), "{found:?}");
    }

    #[test]
    fn highlights_markdown_with_its_inline_text_and_code_blocks() {
        let text = "# Title\n\nSome *words* and `code`.\n\n```rust\nfn x() {}\n```\n";
        let found = highlights("markdown", text, 0);
        assert!(has(&found, "#", Highlight::PunctuationSpecial), "{found:?}");
        assert!(has(&found, "Title", Highlight::Heading), "{found:?}");
        let found = highlights("markdown", text, 2);
        assert!(has(&found, "words", Highlight::Emphasis), "{found:?}");
        assert!(has(&found, "code", Highlight::Literal), "{found:?}");
        // The code is drawn as Rust, not as a code block.
        let found = highlights("markdown", text, 5);
        assert!(has(&found, "fn", Highlight::Keyword), "{found:?}");
        assert!(
            !found.iter().any(|(text, _)| text.contains("x()")),
            "{found:?}"
        );
    }

    #[test]
    fn highlights_toml() {
        let found = highlights("toml", "[package]\nname = \"cox\"\n", 1);
        assert_eq!(
            found,
            [
                ("name".to_owned(), Highlight::Property),
                ("=".to_owned(), Highlight::Operator),
                ("\"cox\"".to_owned(), Highlight::String),
            ]
        );
    }

    #[test]
    fn highlights_only_the_bytes_asked_for() {
        let found = highlights("rust", "let x = \"a long string\";\n", 0);
        assert!(has(&found, "\"a long string\"", Highlight::String));
        let buffer = Buffer::new("let x = \"a long string\";\n");
        let syntax = Syntax::parse(Language::named("rust").unwrap(), &buffer, None).unwrap();
        let spans = syntax.highlights(&buffer, 0, 12);
        assert!(spans.iter().all(|(range, _)| range.end <= 12), "{spans:?}");
        assert_eq!(syntax.highlights(&buffer, 1, 12).first(), None);
    }

    #[test]
    fn tells_the_names_called_on_a_line() {
        let calls = |language: &str, text: &str, line: usize| {
            let buffer = Buffer::new(text);
            let syntax = Syntax::parse(Language::named(language).unwrap(), &buffer, None).unwrap();
            let line_text = buffer.line(line).into_owned();
            syntax.calls(&buffer, line).map(|calls| {
                calls
                    .into_iter()
                    .map(|name| line_text[name].to_owned())
                    .collect::<Vec<_>>()
            })
        };
        let rust =
            "fn main() {\n    let x = run::<u8>(Store::new(), s.total());\n    vec![go(1)];\n}\n";
        assert_eq!(
            calls("rust", rust, 1),
            Some(vec!["run".into(), "new".into(), "total".into()])
        );
        // Within a macro's arguments too; the definition is no call.
        assert_eq!(
            calls("rust", rust, 2),
            Some(vec!["vec".into(), "go".into()])
        );
        assert_eq!(calls("rust", rust, 0), Some(Vec::new()));
        let python = "def main():\n    print(cart.total())\n";
        assert_eq!(
            calls("python", python, 1),
            Some(vec!["print".into(), "total".into()])
        );
        let typescript = "function main() {\n  return store.add(make(1));\n}\n";
        assert_eq!(
            calls("typescript", typescript, 1),
            Some(vec!["add".into(), "make".into()])
        );
        // A language with no tags query says nothing.
        assert_eq!(calls("toml", "a = 1\n", 0), None);
    }

    #[test]
    fn tells_what_a_line_is_within() {
        let text = "mod m {\n    impl S {\n        fn f(&self) {\n            let x = 1;\n            x\n        }\n    }\n}\nfn g() {}\n";
        let buffer = Buffer::new(text);
        let syntax = Syntax::parse(Language::named("rust").unwrap(), &buffer, None).unwrap();
        assert_eq!(syntax.within(&buffer, 4), vec![0, 1, 2]);
        // The line that opens a scope is not within it.
        assert_eq!(syntax.within(&buffer, 2), vec![0, 1]);
        assert_eq!(syntax.within(&buffer, 8), Vec::<usize>::new());
        // An empty line between functions is within the module.
        let buffer = Buffer::new("mod m {\n    fn a() {}\n\n    fn b() {}\n}\n");
        let syntax = Syntax::parse(Language::named("rust").unwrap(), &buffer, None).unwrap();
        assert_eq!(syntax.within(&buffer, 2), vec![0]);
        // Through code within code: Rust in a Markdown section's code block.
        let text = "# Top\n\n## Sub\n\n```rust\nfn x() {\n    1\n}\n```\n";
        let buffer = Buffer::new(text);
        let syntax = Syntax::parse(Language::named("markdown").unwrap(), &buffer, None).unwrap();
        assert_eq!(syntax.within(&buffer, 6), vec![0, 2, 5]);
    }

    #[test]
    fn tells_what_a_line_is_within_by_the_tags() {
        let within = |language: &str, text: &str, line: usize| {
            let buffer = Buffer::new(text);
            let syntax = Syntax::parse(Language::named(language).unwrap(), &buffer, None).unwrap();
            syntax.within(&buffer, line)
        };
        // The impls of a generic type and of a generic trait.
        let text = "impl<T> S<T> {\n    fn f() {\n        1\n    }\n}\n";
        assert_eq!(within("rust", text, 2), vec![0, 1]);
        let text = "impl From<u8> for S {\n    fn from(n: u8) -> Self {\n        S\n    }\n}\n";
        assert_eq!(within("rust", text, 2), vec![0, 1]);
        let text = "class Cart:\n    def total(self):\n        return 1\n";
        assert_eq!(within("python", text, 2), vec![0, 1]);
        let text =
            "namespace N {\n  enum E {\n    A,\n  }\n  const f = () => {\n    return 1;\n  };\n}\n";
        assert_eq!(within("typescript", text, 2), vec![0, 1]);
        assert_eq!(within("typescript", text, 5), vec![0, 4]);
    }

    #[test]
    fn follows_the_text_as_it_changes() {
        let rust = Language::named("rust").unwrap();
        let mut buffer = Buffer::new("fn one() {}\nfn two() {}\n");
        let mut syntax = Syntax::parse(rust, &buffer, None).unwrap();
        let edit = buffer
            .set_text("fn one() {}\n// A comment.\nfn two() {}\n")
            .unwrap();
        // Edited, the syntax stays in step: `fn` is a keyword on the line
        // it went to.
        syntax.edit(&edit);
        let keyword = |syntax: &Syntax, line| {
            syntax
                .highlights(&buffer, line, usize::MAX)
                .contains(&(0..2, Highlight::Keyword))
        };
        assert!(keyword(&syntax, 2));
        // Parsed again, it finds the comment.
        let syntax = Syntax::parse(rust, &buffer, Some(&syntax)).unwrap();
        assert_eq!(
            syntax.highlights(&buffer, 1, usize::MAX),
            [(0.."// A comment.".len(), Highlight::Comment)]
        );
        assert!(keyword(&syntax, 2));
    }
}
