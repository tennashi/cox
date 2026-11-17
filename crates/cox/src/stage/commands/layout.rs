//! A command an agent ran, laid out for the human to read on the Commands
//! page: a long line of shell, as an agent writes it, broken into rows at
//! its `&&`, `||` and `;`, and at its `|` when the pipeline is long, the
//! bodies of its loops and `if`s indented, what it feeds a program as a
//! heredoc or a script folded behind a count of its lines, and colored as
//! the code tabs color shell. Only what the page shows: the command run
//! is the agent's, as it gave it.
//!
//! The command is parsed with tree-sitter's grammar of bash, as the code
//! tabs parse it (see [`cox_editor::syntax`]), and laid out from its
//! tree, as [`Row`]s, each [`Piece`]s of the command's text.

use std::ops::Range;
use std::path::Path;

use cox_editor::buffer::Buffer;
use cox_editor::language::Language;
use cox_editor::syntax::Syntax;
use serde_json::{Value, json};
use tree_sitter::{Node, Parser};

/// How many characters a pipeline or a loop is on one row at most.
const WIDTH: usize = 80;
/// How many characters a script given inline, as `python3 -c '…'`, shows
/// at most before it folds.
const SCRIPT: usize = 60;

/// A row of a command laid out.
#[derive(Debug)]
struct Row {
    /// How many blocks it is within, as a loop's body is within the loop.
    depth: usize,
    /// What joins it to the command of the row before, as `&&`, `||` or
    /// `|`, if anything does but a new line or a `;`.
    joint: Option<String>,
    /// The bytes of the command the row shows, in order: what between
    /// them is not shown is blank, or shows on another row.
    ranges: Vec<Range<usize>>,
}

/// What a row shows of a command.
#[derive(Debug, PartialEq, Eq)]
enum Piece {
    /// Some of the command, as it is.
    Code(Range<usize>),
    /// A space between two pieces of the command a row shows, as between
    /// a statement's body and its redirects.
    Space,
    /// Text the command gives a program, of lines or long, folded.
    Fold(Fold),
}

/// Text a command gives a program, as a heredoc's body or the script of
/// `python3 -c`, shown folded.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fold {
    /// The bytes of the command it takes, which the row does not show.
    hides: Range<usize>,
    /// The bytes of the text itself, within them, without quotes or the
    /// heredoc's last line.
    text: Range<usize>,
    /// The language the program reads it in, if Cox knows it.
    language: Option<&'static str>,
}

/// The command `line` laid out, as JSON for the page: its rows, each with
/// its `depth`, its `joint`, and its `code`, the stretches of text each
/// with what the syntax says it is, as the code tabs color it (see
/// [`cox_editor::language::Highlight::name`]), or `quiet` for what sets
/// up the environment, which the page dims, or null; and in place of a
/// stretch, a `fold` with the `language` of the text and its `lines`, as
/// stretches likewise. A path within `home` shows from `~`, and within
/// `folder`, the chat's, from it, unless the command goes elsewhere first.
#[must_use]
pub fn told(line: &str, folder: &Path, home: Option<&Path>) -> Value {
    let paths = Paths::new(line, folder, home);
    let Some(Laid {
        rows: layout,
        folds,
        quiet,
    }) = lay_out(line)
    else {
        return json!([{
            "depth": 0,
            "joint": null,
            "code": stretches(line, 0..line.len(), &colors(line, Some("bash"), &[]), &paths),
        }]);
    };
    let colors = colors(line, Some("bash"), &quiet);
    let rows: Vec<Value> = layout
        .iter()
        .map(|row| {
            let code: Vec<Value> = pieces(row, &folds, line)
                .into_iter()
                .flat_map(|piece| match piece {
                    Piece::Code(range) => stretches(line, range, &colors, &paths),
                    Piece::Space => vec![json!([" ", null])],
                    Piece::Fold(fold) => vec![folded(line, &fold, &paths)],
                })
                .collect();
            json!({"depth": row.depth, "joint": row.joint, "code": code})
        })
        .collect();
    json!(rows)
}

/// The row of the command `line` that says most of what it does, on one
/// line: the first that neither goes to a folder, as `cd`, nor only sets
/// up the environment, without what joins it to the row before, and its
/// folds as `…`. The Commands page picks its gist of a command as so.
#[must_use]
pub fn gist(line: &str) -> String {
    let first_line = || line.lines().next().unwrap_or_default().trim().to_owned();
    let Some(laid) = lay_out(line) else {
        return first_line();
    };
    let quiet = |range: &Range<usize>| {
        laid.quiet
            .iter()
            .any(|quiet| quiet.start <= range.start && range.end <= quiet.end)
    };
    let shown: Vec<(String, bool)> = laid
        .rows
        .iter()
        .map(|row| {
            let pieces = pieces(row, &laid.folds, line);
            let mut text = String::new();
            for piece in &pieces {
                match piece {
                    Piece::Code(range) => text.push_str(&line[range.clone()]),
                    Piece::Space => text.push(' '),
                    Piece::Fold(_) => text.push('…'),
                }
            }
            let sets_up = pieces.iter().all(|piece| match piece {
                Piece::Code(range) => quiet(range),
                Piece::Space | Piece::Fold(_) => true,
            });
            let goes = text == "cd" || text.starts_with("cd ");
            (text, sets_up || goes)
        })
        .collect();
    shown
        .iter()
        .find(|(_, set_up)| !set_up)
        .or_else(|| shown.first())
        .map_or_else(first_line, |(text, _)| text.clone())
}

/// A command laid out, with what it folds and the bytes that set up the
/// environment, which show quiet.
struct Laid {
    rows: Vec<Row>,
    folds: Vec<Fold>,
    quiet: Vec<Range<usize>>,
}

/// `line` parsed and laid out; `None` when tree-sitter does not parse it
/// as bash, as a line in zsh's own syntax can be.
fn lay_out(line: &str) -> Option<Laid> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(line, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }
    let mut folds = Vec::new();
    let mut quiet = Vec::new();
    find(root, line, &mut folds, &mut quiet);
    let mut layout = Vec::new();
    block(root, 0, &mut layout, line);
    Some(Laid {
        rows: layout,
        folds,
        quiet,
    })
}

/// Finds the folds within `node`, outermost first, and what sets up the
/// environment: a command's leading assignments, and `export`.
fn find(node: Node, line: &str, folds: &mut Vec<Fold>, quiet: &mut Vec<Range<usize>>) {
    if let Some(fold) = fold_of(node, line) {
        folds.push(fold);
        return;
    }
    match node.kind() {
        "variable_assignment" if node.parent().is_some_and(|p| p.kind() == "command") => {
            quiet.push(node.byte_range());
        }
        "declaration_command" if node.child(0).is_some_and(|c| c.kind() == "export") => {
            quiet.push(node.byte_range());
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find(child, line, folds, quiet);
    }
}

/// `node` as a fold, if it is one: a heredoc's body, a quoted string of
/// lines, or a long script given inline to a program that runs one.
fn fold_of(node: Node, line: &str) -> Option<Fold> {
    match node.kind() {
        "heredoc_body" => {
            let end = node
                .next_sibling()
                .filter(|next| next.kind() == "heredoc_end")
                .map_or(node.end_byte(), |end| end.end_byte());
            let text = trim_newlines(line, node.byte_range());
            Some(Fold {
                hides: node.start_byte()..end,
                text,
                language: heredoc_language(node, line),
            })
        }
        "string" | "raw_string" | "ansi_c_string" => {
            let range = node.byte_range();
            let text = &line[range.clone()];
            let script = script_of(node, line);
            let long = script.is_some() && text.chars().count() > SCRIPT;
            if !text.contains('\n') && !long {
                return None;
            }
            // A message given as `"$(cat <<'EOF' … EOF)"`, as an agent
            // writes a commit's, reads as the heredoc's body.
            if let Some(body) = heredoc_within(node) {
                return Some(Fold {
                    hides: range,
                    text: trim_newlines(line, body.byte_range()),
                    language: None,
                });
            }
            let open = if node.kind() == "ansi_c_string" { 2 } else { 1 };
            let inner = (range.start + open)..range.end.saturating_sub(1).max(range.start + open);
            Some(Fold {
                hides: range,
                text: trim_newlines(line, inner),
                language: script.and_then(|script| script.language),
            })
        }
        _ => None,
    }
}

/// The body of the heredoc `string` is all of, as `"$(cat <<'EOF' …
/// EOF\n)"` is, if it is one.
fn heredoc_within(string: Node) -> Option<Node> {
    let mut cursor = string.walk();
    let inner: Vec<Node> = string.named_children(&mut cursor).collect();
    let [substitution] = inner.as_slice() else {
        return None;
    };
    let statement = substitution.named_child(0)?;
    (substitution.kind() == "command_substitution"
        && substitution.named_child_count() == 1
        && statement.kind() == "redirected_statement")
        .then_some(())?;
    let mut cursor = statement.walk();
    let redirect = statement
        .children(&mut cursor)
        .find(|child| child.kind() == "heredoc_redirect")?;
    let mut cursor = redirect.walk();
    redirect
        .children(&mut cursor)
        .find(|child| child.kind() == "heredoc_body")
}

/// `range` of `line` without the line breaks at its edges, as a script
/// in quotes that starts on the line after them has.
fn trim_newlines(line: &str, range: Range<usize>) -> Range<usize> {
    let text = &line[range.clone()];
    let start = range.start + (text.len() - text.trim_start_matches(['\n', '\r']).len());
    let end = range.end - (text.len() - text.trim_end_matches(['\n', '\r']).len());
    start..end.max(start)
}

/// A program that runs a script it is given.
#[derive(Clone, Copy)]
struct Interpreter {
    /// The language Cox colors the script in, if it knows it.
    language: Option<&'static str>,
}

/// The program named `name`, if it runs a script it is given.
fn interpreter(name: &str) -> Option<Interpreter> {
    let name = name.rsplit('/').next().unwrap_or(name);
    let language = match name {
        "sh" | "bash" | "zsh" => Some("bash"),
        "node" | "deno" | "bun" => Some("javascript"),
        "ruby" | "perl" | "osascript" | "awk" | "jq" => None,
        _ if name.starts_with("python") => Some("python"),
        _ => return None,
    };
    Some(Interpreter { language })
}

/// The command `node` is an argument of, if it is one.
fn command_of(node: Node) -> Option<Node> {
    node.parent().filter(|parent| parent.kind() == "command")
}

/// The name of the program `command` runs, as written.
fn program<'a>(command: Node, line: &'a str) -> Option<&'a str> {
    let name = command.child_by_field_name("name")?;
    Some(&line[name.byte_range()])
}

/// The program that runs `string` as its script, when it is the argument
/// of `-c` or `-e` to one.
fn script_of(string: Node, line: &str) -> Option<Interpreter> {
    let command = command_of(string)?;
    let runs = interpreter(program(command, line)?)?;
    let flag = string.prev_named_sibling()?;
    let flag = &line[flag.byte_range()];
    (flag == "-c" || flag == "-e").then_some(runs)
}

/// The language of a heredoc's `body`: that of the program it is given
/// to, or of the file it is written to, as `cat > notes.md <<'EOF'`.
fn heredoc_language(body: Node, line: &str) -> Option<&'static str> {
    let statement = ancestors(body).find(|node| node.kind() == "redirected_statement")?;
    let mut cursor = statement.walk();
    let written = statement
        .children(&mut cursor)
        .filter(|child| child.kind() == "file_redirect")
        .find_map(|redirect| {
            let to = redirect.child_by_field_name("destination")?;
            let path = line[to.byte_range()].trim_matches(['"', '\'']);
            Language::of_path(Path::new(path)).map(Language::name)
        });
    if written.is_some() {
        return written;
    }
    let body = statement.child_by_field_name("body")?;
    let command = last_command(body)?;
    interpreter(program(command, line)?)?.language
}

/// The last command of `node`, as of a list or a pipeline.
fn last_command(node: Node) -> Option<Node> {
    if node.kind() == "command" {
        return Some(node);
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    children.into_iter().rev().find_map(last_command)
}

/// `node`'s parents, innermost first.
fn ancestors(node: Node) -> impl Iterator<Item = Node> {
    std::iter::successors(node.parent(), Node::parent)
}

/// Whether `node` is short enough to show on one row as it is.
fn fits(node: Node, line: &str) -> bool {
    let text = &line[node.byte_range()];
    !text.contains('\n') && text.chars().count() <= WIDTH
}

/// Lays out `node`, a command of any kind, as rows at `depth`, the first
/// joined to the row before by `joint`.
fn rows(node: Node, depth: usize, joint: Option<String>, layout: &mut Vec<Row>, line: &str) {
    match node.kind() {
        // The commands of a chain show a row each, but within a block,
        // where a short chain, as `a && break`, reads as one.
        "list" if depth == 0 || !fits(node, line) => {
            let mut joint = joint;
            for (part, next) in parts(node, line) {
                rows(part, depth, joint.take().or(next), layout, line);
            }
        }
        // tree-sitter takes `a && b | c` as a pipeline of the chain
        // `a && b` into `c`: its commands show a row each all the same,
        // as those of a chain do.
        "pipeline" if !fits(node, line) || (depth == 0 && chains(node)) => {
            let mut joint = joint;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    rows(child, depth, joint.take(), layout, line);
                } else {
                    joint = Some(line[child.byte_range()].to_owned());
                }
            }
        }
        "redirected_statement" => match node.child_by_field_name("body") {
            Some(body) => {
                rows(body, depth, joint, layout, line);
                append(layout, body.end_byte()..node.end_byte());
            }
            None => row(layout, depth, joint, node.byte_range()),
        },
        "for_statement"
        | "c_style_for_statement"
        | "while_statement"
        | "if_statement"
        | "compound_statement"
        | "subshell"
        | "function_definition"
            if !fits(node, line) =>
        {
            let start = layout.len();
            let mut header = None;
            compound(node, depth, &mut header, layout, line);
            if let Some(header) = header {
                row(layout, depth, None, header);
            }
            if let Some(first) = layout.get_mut(start) {
                first.joint = joint;
            }
        }
        _ => row(layout, depth, joint, node.byte_range()),
    }
}

/// Whether `pipeline` starts with a chain, as `a && b | c` parses, and
/// `a && b 2>&1 | c`, its redirect taken as the chain's.
fn chains(pipeline: Node) -> bool {
    let first = pipeline.named_child(0);
    let first = match first {
        Some(first) if first.kind() == "redirected_statement" => first.child_by_field_name("body"),
        _ => first,
    };
    first.is_some_and(|first| first.kind() == "list")
}

/// The commands of `list`, as `a && b || c` has, each with the operator
/// before it, the first with none.
fn parts<'a>(list: Node<'a>, line: &str) -> Vec<(Node<'a>, Option<String>)> {
    let mut found = Vec::new();
    let mut joint = None;
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() == "list" {
            let mut inner = parts(child, line);
            if let Some(first) = inner.first_mut() {
                first.1 = first.1.take().or(joint.take());
            }
            found.extend(inner);
        } else if child.is_named() {
            found.push((child, joint.take()));
        } else {
            joint = Some(line[child.byte_range()].to_owned());
        }
    }
    found
}

/// Lays out the statements of `node`, as of the whole command or a
/// block's body, each a row of its own at `depth`.
fn block(node: Node, depth: usize, layout: &mut Vec<Row>, line: &str) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        statement(child, depth, layout, line);
    }
}

/// Lays out `child`, of a block: a statement as its rows, a `&` that
/// runs the one before in the background on the row of its end.
fn statement(child: Node, depth: usize, layout: &mut Vec<Row>, line: &str) {
    if child.is_named() {
        rows(child, depth, None, layout, line);
    } else if child.kind() == "&" {
        append(layout, child.byte_range());
    }
}

/// Lays out `node`, a loop, an `if`, a block in braces or a subshell
/// too long for a row: what opens it on a row, as `for f in *; do`,
/// what it runs indented below, and what closes it, as `done`, on a row
/// of its own. `header` is what of a row was seen and not laid out yet.
fn compound(
    node: Node,
    depth: usize,
    header: &mut Option<Range<usize>>,
    layout: &mut Vec<Row>,
    line: &str,
) {
    let mut cursor = node.walk();
    let mut within = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "do_group" | "elif_clause" | "else_clause" => {
                compound(child, depth, header, layout, line);
                within = false;
            }
            "compound_statement" if node.kind() == "function_definition" => {
                compound(child, depth, header, layout, line);
            }
            "elif" | "else" => {
                if let Some(open) = header.take() {
                    row(layout, depth, None, open);
                }
                *header = Some(child.byte_range());
                if child.kind() == "else" {
                    row(layout, depth, None, child.byte_range());
                    *header = None;
                    within = true;
                }
            }
            "(" | ")" if node.kind() == "function_definition" => {
                let start = header
                    .as_ref()
                    .map_or(child.start_byte(), |open| open.start);
                *header = Some(start..child.end_byte());
            }
            "then" | "do" | "{" | "(" => {
                let start = header.take().map_or(child.start_byte(), |open| open.start);
                row(layout, depth, None, start..child.end_byte());
                within = true;
            }
            "done" | "fi" | "}" | ")" => {
                row(layout, depth, None, child.byte_range());
                within = false;
            }
            _ if within => statement(child, depth + 1, layout, line),
            _ => {
                let start = header
                    .as_ref()
                    .map_or(child.start_byte(), |open| open.start);
                *header = Some(start..child.end_byte());
            }
        }
    }
}

/// Adds a row of `range` at `depth`, joined by `joint`.
fn row(layout: &mut Vec<Row>, depth: usize, joint: Option<String>, range: Range<usize>) {
    layout.push(Row {
        depth,
        joint,
        ranges: vec![range],
    });
}

/// Adds `range` to the end of the last row, as a statement's redirects
/// after its body's last row.
fn append(layout: &mut [Row], range: Range<usize>) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = layout.last_mut() {
        last.ranges.push(range);
    }
}

/// What `row` shows: its ranges, less the folds within them, each fold
/// in its place, and the blanks at the edges of what is left trimmed.
fn pieces(row: &Row, folds: &[Fold], line: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let code = |range: Range<usize>, pieces: &mut Vec<Piece>| {
        let range = trim(line, range);
        if !range.is_empty() {
            // Two ranges of a row are a space apart, as a statement's
            // body and its redirects are.
            if let Some(Piece::Code(last)) = pieces.last()
                && last.end < range.start
            {
                pieces.push(Piece::Space);
            }
            pieces.push(Piece::Code(range));
        }
    };
    for range in &row.ranges {
        let mut at = range.start;
        for fold in folds
            .iter()
            .filter(|fold| range.start <= fold.hides.start && fold.hides.end <= range.end)
        {
            code(at..fold.hides.start, &mut pieces);
            pieces.push(Piece::Fold(fold.clone()));
            at = fold.hides.end;
        }
        code(at..range.end, &mut pieces);
    }
    pieces
}

/// `range` of `line` without the blanks at its edges.
fn trim(line: &str, range: Range<usize>) -> Range<usize> {
    let text = &line[range.clone()];
    let start = range.start + (text.len() - text.trim_start().len());
    let end = range.end - (text.len() - text.trim_end().len());
    start..end.max(start)
}

/// What each byte of `text` is, by the syntax of `language`, with the
/// bytes of `quiet` quiet.
fn colors(text: &str, language: Option<&str>, quiet: &[Range<usize>]) -> Vec<Option<&'static str>> {
    let mut bytes = vec![None; text.len()];
    let language = language.and_then(Language::named);
    let buffer = Buffer::new(text);
    if let Some(syntax) = language.and_then(|language| Syntax::parse(language, &buffer, None)) {
        for line in 0..buffer.lines() {
            let start = buffer.line_start(line);
            for (range, highlight) in syntax.highlights(&buffer, line, usize::MAX) {
                let range =
                    (start + range.start).min(text.len())..(start + range.end).min(text.len());
                bytes[range].fill(Some(highlight.name()));
            }
        }
    }
    for range in quiet {
        bytes[range.clone()].fill(Some("quiet"));
    }
    bytes
}

/// The bytes of `range` of `text` as stretches of what they are, by
/// `colors`, with paths shortened as `paths` says.
fn stretches(
    text: &str,
    range: Range<usize>,
    colors: &[Option<&'static str>],
    paths: &Paths,
) -> Vec<Value> {
    let mut out: Vec<(String, Option<&'static str>)> = Vec::new();
    let mut at = range.start;
    while at < range.end {
        let what = colors[at];
        let mut end = at + 1;
        while end < range.end && (colors[end] == what || !text.is_char_boundary(end)) {
            end += 1;
        }
        let Some(piece) = text.get(at..end) else {
            break;
        };
        match out.last_mut() {
            Some((last, last_what)) if *last_what == what => last.push_str(piece),
            _ => out.push((piece.to_owned(), what)),
        }
        at = end;
    }
    out.into_iter()
        .map(|(text, what)| json!([paths.shorten(&text), what]))
        .collect()
}

/// `fold` of `line` as the page is told of it.
fn folded(line: &str, fold: &Fold, paths: &Paths) -> Value {
    let text = &line[fold.text.clone()];
    let colors = colors(text, fold.language, &[]);
    let mut lines = Vec::new();
    let mut start = 0;
    for piece in text.split('\n') {
        let end = start + piece.len();
        lines.push(json!(stretches(text, start..end, &colors, paths)));
        start = end + 1;
    }
    json!({"fold": {"language": fold.language, "lines": lines}})
}

/// How paths show: within the home folder from `~`, and within the
/// chat's folder from it, unless the command goes to another folder
/// first, which a path shown from the chat's would mislead about.
struct Paths {
    /// The chat's folder, and the home folder, each with what it shows
    /// as.
    shown: Vec<(String, &'static str)>,
}

impl Paths {
    fn new(line: &str, folder: &Path, home: Option<&Path>) -> Self {
        let folder = folder.display().to_string();
        let mut shown = Vec::new();
        let goes_elsewhere = line.split(['&', ';', '|', '\n', '(', '{']).any(|part| {
            let mut words = part.split_whitespace();
            words.next() == Some("cd")
                && words
                    .next()
                    .is_some_and(|to| to.trim_matches(['"', '\'']) != folder)
        });
        if !goes_elsewhere && folder.len() > 1 {
            shown.push((folder, "."));
        }
        if let Some(home) = home.map(|home| home.display().to_string())
            && home.len() > 1
        {
            shown.push((home, "~"));
        }
        Self { shown }
    }

    /// `text` with the paths in it shortened.
    fn shorten(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for (path, short) in &self.shown {
            let mut out = String::with_capacity(text.len());
            let mut rest = text.as_str();
            while let Some(at) = rest.find(path.as_str()) {
                let after = &rest[at + path.len()..];
                let before = rest[..at].chars().next_back();
                let starts = before.is_none_or(|c| !(c.is_alphanumeric() || c == '/' || c == '.'));
                let next = after.chars().next();
                out.push_str(&rest[..at]);
                match next {
                    Some('/') if starts => {
                        // `./a` reads as `a`.
                        if *short == "." {
                            rest = &after[1..];
                        } else {
                            out.push_str(short);
                            rest = after;
                        }
                    }
                    None | Some(' ' | '"' | '\'' | ';' | ')' | '&' | '|' | '\n' | ':')
                        if starts =>
                    {
                        out.push_str(short);
                        rest = after;
                    }
                    _ => {
                        out.push_str(path);
                        rest = after;
                    }
                }
            }
            out.push_str(rest);
            text = out;
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    /// `told`, as text: each row indented by its depth, after its joint,
    /// each fold as `[language, N lines]`, then its lines indented
    /// below the row with `> `.
    fn shown(rows: &Value) -> String {
        let mut out = String::new();
        for row in rows.as_array().unwrap() {
            let depth = usize::try_from(row["depth"].as_u64().unwrap()).unwrap();
            let indent = "  ".repeat(depth);
            out.push_str(&indent);
            if let Some(joint) = row["joint"].as_str() {
                out.push_str(joint);
                out.push(' ');
            }
            let mut folded = Vec::new();
            for piece in row["code"].as_array().unwrap() {
                if let Some(fold) = piece.get("fold") {
                    let lines = fold["lines"].as_array().unwrap();
                    let language = fold["language"].as_str().unwrap_or("text");
                    write!(out, "[{language}, {} lines]", lines.len()).unwrap();
                    folded.extend(lines.iter().map(|line| {
                        line.as_array()
                            .unwrap()
                            .iter()
                            .map(|stretch| stretch[0].as_str().unwrap())
                            .collect::<String>()
                    }));
                } else {
                    out.push_str(piece[0].as_str().unwrap());
                }
            }
            out.push('\n');
            for line in folded {
                writeln!(out, "{indent}  > {line}").unwrap();
            }
        }
        out
    }

    /// Real commands agents ran in this project, each with how it lays
    /// out, from `layout-samples.txt`: a sample is a line `%% <name>`, the
    /// command, a line `%% laid out`, and the layout, as [`shown`] tells
    /// it, for a chat in `/work/cox` with its home in `/Users/me`.
    fn samples() -> Vec<(String, String, String)> {
        let text = include_str!("layout-samples.txt");
        let mut samples = Vec::new();
        for sample in text.split("\n%% sample ").skip(1) {
            let (name, rest) = sample.split_once('\n').unwrap();
            let (command, laid) = rest.split_once("\n%% laid out\n").unwrap();
            samples.push((
                name.to_owned(),
                command.to_owned(),
                laid.trim_end().to_owned(),
            ));
        }
        samples
    }

    /// The stretches of row `row` of `rows`, as text and what each is.
    fn row(rows: &Value, row: usize) -> Vec<(String, Option<String>)> {
        rows[row]["code"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stretch| {
                let what = stretch[1].as_str().map(str::to_owned);
                (stretch[0].as_str().unwrap().to_owned(), what)
            })
            .collect()
    }

    #[test]
    fn an_environment_set_up_is_quiet_and_paths_show_short() {
        let rows = told(
            "cd /work/cox && CARGO_TARGET_DIR=/Users/me/t cargo test -p cox",
            Path::new("/work/cox"),
            Some(Path::new("/Users/me")),
        );

        let text =
            |row: &[(String, Option<String>)]| row.iter().map(|s| s.0.as_str()).collect::<String>();
        assert_eq!(text(&row(&rows, 0)), "cd .");
        let set_up = row(&rows, 1);
        assert_eq!(
            set_up[0],
            ("CARGO_TARGET_DIR=~/t".to_owned(), Some("quiet".to_owned()))
        );
        assert_eq!(text(&set_up), "CARGO_TARGET_DIR=~/t cargo test -p cox");
        assert!(
            set_up
                .iter()
                .any(|(text, what)| text == "cargo" && what.as_deref() == Some("function")),
            "{set_up:?}"
        );
        assert_eq!(rows[1]["joint"], "&&");
    }

    #[test]
    fn a_path_in_the_chat_s_folder_shows_whole_once_the_command_goes_elsewhere() {
        let rows = told(
            "cd /tmp && cat /work/cox/notes.md /work/cox-old/a",
            Path::new("/work/cox"),
            None,
        );

        let text: String = row(&rows, 1).into_iter().map(|s| s.0).collect();
        assert_eq!(text, "cat /work/cox/notes.md /work/cox-old/a");
        let rows = told(
            "cat /work/cox/notes.md /work/cox-old/a",
            Path::new("/work/cox"),
            None,
        );
        let text: String = row(&rows, 0).into_iter().map(|s| s.0).collect();
        assert_eq!(text, "cat notes.md /work/cox-old/a");
    }

    #[test]
    fn the_gist_of_a_command_is_what_it_does_past_its_set_up() {
        assert_eq!(
            gist("cd /work/cox && export CARGO_TARGET_DIR=/t && cargo test -p cox 2>&1 | tail -5"),
            "cargo test -p cox 2>&1"
        );
        assert_eq!(
            gist("cd cox && python3 - <<'EOF'\nimport pathlib\nprint(1)\nEOF\n"),
            "python3 - <<'EOF'…"
        );
        assert_eq!(gist("cd /tmp"), "cd /tmp");
        assert_eq!(
            gist("for n in $(seq 3); do echo $n; done"),
            "for n in $(seq 3); do echo $n; done"
        );
    }

    #[test]
    fn real_commands_lay_out_to_read() {
        let samples = samples();
        assert!(samples.len() >= 10, "{} samples", samples.len());
        let mut wrong = Vec::new();
        for (name, command, laid) in &samples {
            let rows = told(
                command,
                Path::new("/work/cox"),
                Some(Path::new("/Users/me")),
            );
            let got = shown(&rows);
            if got.trim_end() != laid {
                wrong.push(format!("%% sample {name}\n{command}\n%% laid out\n{got}"));
            }
        }
        assert!(
            wrong.is_empty(),
            "laid out otherwise:\n\n{}",
            wrong.join("\n")
        );
    }
}
