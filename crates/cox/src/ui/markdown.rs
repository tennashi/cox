//! The agent's replies as Markdown: the blocks a reply is made of, and the
//! styles of their text, for the window to draw. Plain Rust, without GPUI.
//!
//! Text keeps the line breaks the agent wrote, rather than joining lines
//! into one as Markdown does: a reply reads as the agent laid it out, and
//! lines of Japanese do not gain spaces between them. Web addresses and
//! paths of files in the text are links even where the agent did not mark
//! them as such, as in what a command printed, or a place in the code the
//! agent names, such as `src/main.rs:12`.

use std::ops::Range;

use cox_core::tabs::Lines;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// One block of a reply, such as a paragraph or a code block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    /// How many lists the block is in.
    pub depth: usize,
    /// Whether the block is quoted.
    pub quoted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph(Text),
    /// A heading, at level 1 to 6.
    Heading(u8, Text),
    /// The first paragraph of an item in a list, after its marker: a
    /// bullet, the item's number, or a box checked or not.
    Item {
        marker: String,
        text: Text,
    },
    /// Code as it was written, with no styles but its links.
    Code(Text),
    /// A line across the reply.
    Rule,
    /// A table's rows of cells, its head first.
    Table(Vec<Vec<Text>>),
}

/// Text, in runs that each have one style and together cover it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub text: String,
    pub runs: Vec<(Range<usize>, Style)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "text can be bold, slanted, struck through and code all at once"
)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    /// Code within a line.
    pub code: bool,
    /// Where the text links to: an address on the web, or a file, by its
    /// path as written, with the lines to show after it if any (see
    /// [`file_link`]). Links elsewhere, such as to an email address, have
    /// nowhere to open and stay plain text.
    pub link: Option<String>,
}

impl Text {
    fn push(&mut self, text: &str, style: &Style) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        match self.runs.last_mut() {
            Some((range, last)) if last == style => range.end = end,
            _ => self.runs.push((start..end, style.clone())),
        }
    }

    /// Makes the web addresses and the paths of files in the text that
    /// link nowhere yet links to themselves.
    fn link_bare(&mut self) {
        let addresses = addresses(&self.text);
        let paths = paths(&self.text).into_iter().filter(|path| {
            !addresses
                .iter()
                .any(|address| address.start < path.end && path.start < address.end)
        });
        let mut bare: Vec<_> = paths.chain(addresses.iter().cloned()).collect();
        bare.sort_by_key(|range| range.start);
        for address in bare {
            let url = self.text[address.clone()].to_owned();
            let mut runs = Vec::with_capacity(self.runs.len() + 2);
            for (range, style) in self.runs.drain(..) {
                let inside = range.start.max(address.start)..range.end.min(address.end);
                if inside.is_empty() || style.link.is_some() {
                    runs.push((range, style));
                    continue;
                }
                if range.start < inside.start {
                    runs.push((range.start..inside.start, style.clone()));
                }
                runs.push((
                    inside.clone(),
                    Style {
                        link: Some(url.clone()),
                        ..style.clone()
                    },
                ));
                if inside.end < range.end {
                    runs.push((inside.end..range.end, style));
                }
            }
            self.runs = runs;
        }
    }

    /// The text with its web addresses and paths of files linked, and no
    /// other style.
    #[must_use]
    pub fn plain(text: &str) -> Self {
        let mut plain = Self::default();
        plain.push(text, &Style::default());
        plain.link_bare();
        plain
    }
}

/// Whether `url` is an address on the web, which the stage can open.
#[must_use]
pub fn is_on_the_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// A link to a file, and to lines of it if it names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileLink<'a> {
    /// The file's path, as written: absolute, or from where the link is.
    pub path: &'a str,
    pub lines: Option<Lines>,
}

/// The file `link` links to, if it links to one: a path, or a `file://`
/// address, maybe with lines after it, as in `src/main.rs:12`,
/// `src/main.rs:12-20`, `src/main.rs:12:5` (at a column of line 12) or
/// `src/main.rs#L12-L20`.
#[must_use]
pub fn file_link(link: &str) -> Option<FileLink<'_>> {
    if link.is_empty() || link.starts_with('#') || is_on_the_web(link) {
        return None;
    }
    let link = link.strip_prefix("file://").unwrap_or(link);
    if let Some((path, anchor)) = link.split_once('#') {
        return (!path.is_empty()).then(|| FileLink {
            path,
            lines: anchor_lines(anchor),
        });
    }
    let (path, lines) = match link.find(':') {
        Some(colon) => {
            let (lines, len) = suffix_lines(&link[colon..])?;
            if colon + len != link.len() {
                // A scheme, as in `mailto:`, or something after the lines.
                return None;
            }
            (&link[..colon], Some(lines))
        }
        None => (link, None),
    };
    (!path.is_empty()).then_some(FileLink { path, lines })
}

/// The lines that `anchor`, after the `#` of a link, names, as in `L12` or
/// `L12-L20`.
fn anchor_lines(anchor: &str) -> Option<Lines> {
    let anchor = anchor.strip_prefix('L')?;
    let (first, last) = match anchor.split_once('-') {
        Some((first, last)) => (first, last.strip_prefix('L').unwrap_or(last)),
        None => (anchor, anchor),
    };
    lines(first.parse().ok()?, last.parse().ok()?)
}

/// The lines that the start of `text` names after a path, as in `:12`,
/// `:12-20` or `:12:5`, and how long that is.
fn suffix_lines(text: &str) -> Option<(Lines, usize)> {
    let digits = |text: &str| text.bytes().take_while(u8::is_ascii_digit).count();
    let rest = text.strip_prefix(':')?;
    let first_len = digits(rest);
    let first = rest[..first_len].parse().ok()?;
    let mut len = 1 + first_len;
    let mut last = first;
    let after = &text[len..];
    if let Some(more) = after.strip_prefix('-').or_else(|| after.strip_prefix(':')) {
        let more_len = digits(more);
        if more_len > 0 {
            if after.starts_with('-') {
                last = more[..more_len].parse().ok()?;
            }
            len += 1 + more_len;
        }
    }
    Some((lines(first, last)?, len))
}

/// Lines from `first` to `last`, as people number them.
fn lines(first: usize, last: usize) -> Option<Lines> {
    (first > 0).then(|| Lines {
        first,
        last: last.max(first),
    })
}

/// Where paths of files are in `text`, with the lines after them, if any:
/// a path of letters, digits and `_-./~` whose file name has an extension,
/// and that has a `/` in it, or lines after it. So `src/main.rs` and
/// `main.rs:12` are paths, and `main.rs`, `e.g.` and `v0.2.2` are not.
#[must_use]
pub fn paths(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let in_path = |byte: u8| byte.is_ascii_alphanumeric() || b"_-./~".contains(&byte);
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if !in_path(bytes[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && in_path(bytes[at]) {
            at += 1;
        }
        // A sentence may end right after a path.
        let mut end = at;
        while end > start && matches!(bytes[end - 1], b'.' | b'-' | b'~') {
            end -= 1;
        }
        let lines = if end == at {
            suffix_lines(&text[at..]).map(|(_, len)| len)
        } else {
            None
        };
        // After a `:`, it is part of something else, as of `std::fs` or
        // an address.
        let after_colon = start > 0 && bytes[start - 1] == b':';
        if !after_colon && is_a_file(&text[start..end], lines.is_some()) {
            let end = end + lines.unwrap_or(0);
            found.push(start..end);
            at = end;
        }
    }
    found
}

/// Whether `path`, followed by lines if `with_lines`, reads as the path of
/// a file.
fn is_a_file(path: &str, with_lines: bool) -> bool {
    if path.starts_with("//") {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let extension_reads = extension.len() <= 10
        && extension.starts_with(|c: char| c.is_ascii_alphabetic())
        && extension.chars().all(|c| c.is_ascii_alphanumeric());
    !stem.is_empty() && extension_reads && (path.contains('/') || with_lines)
}

/// Where web addresses are in `text`: from `http://` or `https://` up to a
/// space, or a character that is not ASCII or not taken as part of an
/// address in prose, less the punctuation that ends a sentence and a
/// closing bracket that has no opening one in the address.
#[must_use]
pub fn addresses(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find("http").map(|at| from + at) {
        let rest = &text[start..];
        if !is_on_the_web(rest) || (start > 0 && text.as_bytes()[start - 1].is_ascii_alphanumeric())
        {
            from = start + "http".len();
            continue;
        }
        let mut end = start
            + rest
                .find(|c: char| !c.is_ascii_graphic() || "<>\"'`".contains(c))
                .unwrap_or(rest.len());
        loop {
            let address = &text[start..end];
            let trimmed = match address.chars().last() {
                Some('.' | ',' | ':' | ';' | '!' | '?' | '*' | '_' | '~') => true,
                Some(')') => address.matches('(').count() < address.matches(')').count(),
                Some(']') => address.matches('[').count() < address.matches(']').count(),
                _ => false,
            };
            if !trimmed {
                break;
            }
            end -= 1;
        }
        let scheme = if text[start..].starts_with("https://") {
            "https://".len()
        } else {
            "http://".len()
        };
        if end > start + scheme {
            found.push(start..end);
        }
        from = end.max(start + scheme);
    }
    found
}

/// The blocks of `markdown`.
#[must_use]
pub fn parse(markdown: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut blocks = Blocks::default();
    for event in Parser::new_ext(markdown, options) {
        blocks.take(event);
    }
    blocks.flush();
    blocks.done
}

/// What a block being read turns into once it ends.
enum Open {
    Paragraph,
    Heading(u8),
    Item(String),
}

/// The blocks read so far, and where the reading is.
#[derive(Default)]
struct Blocks {
    done: Vec<Block>,
    /// The paragraph, heading or item being read, and its text so far.
    open: Option<(Open, Text)>,
    /// The marker of the item that started last, until its first paragraph
    /// takes it.
    marker: Option<String>,
    /// For each list the reading is in, the number of its next item, or
    /// `None` when its items have bullets.
    lists: Vec<Option<u64>>,
    quotes: usize,
    /// The code block being read.
    code: Option<String>,
    /// The table being read: its rows, and the cell being read.
    table: Option<(Vec<Vec<Text>>, Option<Text>)>,
    /// The styles in effect, innermost last.
    styles: Vec<Style>,
}

impl Blocks {
    fn style(&self) -> Style {
        self.styles.last().cloned().unwrap_or_default()
    }

    fn push_style(&mut self, change: impl FnOnce(&mut Style)) {
        let mut style = self.style();
        change(&mut style);
        self.styles.push(style);
    }

    fn block(&mut self, kind: Kind) {
        self.done.push(Block {
            kind,
            depth: self.lists.len(),
            quoted: self.quotes > 0,
        });
    }

    /// Starts a block of text, taking the marker of an item that has just
    /// started.
    fn open(&mut self, open: Open) {
        self.flush();
        let open = match (open, self.marker.take()) {
            (Open::Paragraph, Some(marker)) => Open::Item(marker),
            (open, _) => open,
        };
        self.open = Some((open, Text::default()));
    }

    /// Ends the block of text being read, if any.
    fn flush(&mut self) {
        let Some((open, mut text)) = self.open.take() else {
            return;
        };
        text.link_bare();
        let kind = match open {
            Open::Paragraph => Kind::Paragraph(text),
            Open::Heading(level) => Kind::Heading(level, text),
            Open::Item(marker) => Kind::Item { marker, text },
        };
        self.block(kind);
    }

    /// Starts a block that is not text, after an item's marker if one is
    /// waiting, so that the marker still shows.
    fn start_other_block(&mut self) {
        self.flush();
        if let Some(marker) = self.marker.take() {
            self.done.push(Block {
                kind: Kind::Item {
                    marker,
                    text: Text::default(),
                },
                depth: self.lists.len(),
                quoted: self.quotes > 0,
            });
        }
    }

    /// Adds text where the reading is: to a code block, a table's cell, or
    /// a block of text, which starts if none has.
    fn text(&mut self, text: &str, style: &Style) {
        if let Some(code) = &mut self.code {
            code.push_str(text);
            return;
        }
        if let Some((_, cell)) = &mut self.table {
            if let Some(cell) = cell {
                cell.push(text, style);
            }
            return;
        }
        if self.open.is_none() {
            // The items of a tight list hold text with no paragraph.
            self.open(Open::Paragraph);
        }
        if let Some((_, open)) = &mut self.open {
            open.push(text, style);
        }
    }

    fn take(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                self.text(&text, &self.style());
            }
            Event::Code(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                let mut style = self.style();
                style.code = true;
                self.text(&text, &style);
            }
            Event::FootnoteReference(label) => self.text(&format!("[^{label}]"), &self.style()),
            Event::SoftBreak | Event::HardBreak => self.text("\n", &self.style()),
            Event::Rule => {
                self.start_other_block();
                self.block(Kind::Rule);
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "☑" } else { "☐" }.to_owned();
                match &mut self.open {
                    Some((Open::Item(shown), _)) => *shown = marker,
                    _ => self.marker = Some(marker),
                }
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => {
                if self.table.is_none() {
                    self.open(Open::Paragraph);
                }
            }
            Tag::Heading { level, .. } => self.open(Open::Heading(heading_level(level))),
            Tag::BlockQuote(_) => {
                self.start_other_block();
                self.quotes += 1;
            }
            Tag::CodeBlock(_) => {
                self.start_other_block();
                self.code = Some(String::new());
            }
            Tag::List(first) => {
                self.start_other_block();
                self.lists.push(first);
            }
            Tag::Item => {
                self.flush();
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}.");
                        *number += 1;
                        marker
                    }
                    _ => "•".to_owned(),
                };
                self.marker = Some(marker);
            }
            Tag::Table(_) => {
                self.start_other_block();
                self.table = Some((Vec::new(), None));
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some((rows, _)) = &mut self.table {
                    rows.push(Vec::new());
                }
            }
            Tag::TableCell => {
                if let Some((_, cell)) = &mut self.table {
                    *cell = Some(Text::default());
                }
            }
            Tag::Emphasis => self.push_style(|style| style.emphasis = true),
            Tag::Strong => self.push_style(|style| style.strong = true),
            Tag::Strikethrough => self.push_style(|style| style.strikethrough = true),
            Tag::Link { dest_url, .. } => self.push_style(|style| {
                if is_on_the_web(&dest_url) || file_link(&dest_url).is_some() {
                    style.link = Some(dest_url.into_string());
                }
            }),
            // The rest show as the text they hold, with no style of their
            // own.
            _ => self.push_style(|_| {}),
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::HtmlBlock | TagEnd::Heading(_) | TagEnd::Item => {
                if self.table.is_none() {
                    self.flush();
                }
                if tag == TagEnd::Item {
                    // An item that held nothing still shows its marker.
                    self.start_other_block();
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quotes = self.quotes.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                if let Some(mut code) = self.code.take() {
                    if code.ends_with('\n') {
                        code.pop();
                    }
                    self.block(Kind::Code(Text::plain(&code)));
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Table => {
                if let Some((rows, _)) = self.table.take() {
                    self.block(Kind::Table(rows));
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {}
            TagEnd::TableCell => {
                if let Some((rows, cell)) = &mut self.table
                    && let Some(mut cell) = cell.take()
                {
                    cell.link_bare();
                    if let Some(row) = rows.last_mut() {
                        row.push(cell);
                    }
                }
            }
            _ => {
                self.styles.pop();
            }
        }
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(kind: Kind) -> Block {
        Block {
            kind,
            depth: 0,
            quoted: false,
        }
    }

    fn plain(text: &str) -> Text {
        Text {
            text: text.to_owned(),
            runs: vec![(0..text.len(), Style::default())],
        }
    }

    #[test]
    fn reads_headings_paragraphs_and_styles() {
        let blocks = parse("# Title\n\nSome **bold** and `code`.");

        let mut text = Text::default();
        text.push("Some ", &Style::default());
        text.push(
            "bold",
            &Style {
                strong: true,
                ..Style::default()
            },
        );
        text.push(" and ", &Style::default());
        text.push(
            "code",
            &Style {
                code: true,
                ..Style::default()
            },
        );
        text.push(".", &Style::default());
        assert_eq!(
            blocks,
            [
                block(Kind::Heading(1, plain("Title"))),
                block(Kind::Paragraph(text))
            ]
        );
    }

    #[test]
    fn keeps_line_breaks_as_written() {
        assert_eq!(
            parse("一行目\n二行目"),
            [block(Kind::Paragraph(plain("一行目\n二行目")))]
        );
    }

    #[test]
    fn keeps_code_as_written() {
        assert_eq!(
            parse("```console\n$ cargo xtask ci\n  ok\n```"),
            [block(Kind::Code(plain("$ cargo xtask ci\n  ok")))]
        );
    }

    #[test]
    fn reads_lists_with_their_markers_and_depth() {
        let blocks = parse("1. one\n2. two\n   - inner\n- [x] done");

        let item = |marker: &str, text: &str, depth| Block {
            kind: Kind::Item {
                marker: marker.to_owned(),
                text: plain(text),
            },
            depth,
            quoted: false,
        };
        assert_eq!(
            blocks,
            [
                item("1.", "one", 1),
                item("2.", "two", 1),
                item("•", "inner", 2),
                item("☑", "done", 1),
            ]
        );
    }

    #[test]
    fn reads_tables_by_rows() {
        let blocks = parse("| a | b |\n|---|---|\n| 1 | 2 |");

        assert_eq!(
            blocks,
            [block(Kind::Table(vec![
                vec![plain("a"), plain("b")],
                vec![plain("1"), plain("2")],
            ]))]
        );
    }

    #[test]
    fn links_to_the_web_and_to_files() {
        let [
            Block {
                kind: Kind::Paragraph(text),
                ..
            },
        ] = &parse(
            "[PR](https://github.com/tennashi/cox/pull/1), [file](src/main.rs) \
             and [mail](mailto:a@example.com)",
        )[..]
        else {
            panic!("one paragraph");
        };

        let links: Vec<_> = text
            .runs
            .iter()
            .map(|(range, style)| (&text.text[range.clone()], style.link.as_deref()))
            .collect();
        assert_eq!(
            links,
            [
                ("PR", Some("https://github.com/tennashi/cox/pull/1")),
                (", ", None),
                ("file", Some("src/main.rs")),
                (" and mail", None),
            ]
        );
    }

    #[test]
    fn links_web_addresses_in_text_and_code() {
        let text = Text::plain("see https://example.com/a_(b)_c. and http://x.test/,");

        let links: Vec<_> = text
            .runs
            .iter()
            .filter_map(|(range, style)| Some((&text.text[range.clone()], style.link.as_deref()?)))
            .collect();
        assert_eq!(
            links,
            [
                ("https://example.com/a_(b)_c", "https://example.com/a_(b)_c"),
                ("http://x.test/", "http://x.test/"),
            ]
        );
    }

    #[test]
    fn links_paths_of_files_in_text_and_code() {
        let text = Text::plain(
            "see crates/cox/src/stage.rs:52を, main.rs:3-9, src/a.rs. \
             not main.rs, e.g. v0.2.2 std::fs and/or https://x.test/a.rs",
        );

        let links: Vec<_> = text
            .runs
            .iter()
            .filter_map(|(range, style)| Some((&text.text[range.clone()], style.link.as_deref()?)))
            .collect();
        assert_eq!(
            links,
            [
                ("crates/cox/src/stage.rs:52", "crates/cox/src/stage.rs:52"),
                ("main.rs:3-9", "main.rs:3-9"),
                ("src/a.rs", "src/a.rs"),
                ("https://x.test/a.rs", "https://x.test/a.rs"),
            ]
        );
    }

    #[test]
    fn reads_the_lines_a_link_to_a_file_names() {
        let link = |first, last| Some(Lines { first, last });
        let cases = [
            ("src/main.rs", None),
            ("src/main.rs:12", link(12, 12)),
            ("src/main.rs:12-20", link(12, 20)),
            ("src/main.rs:12:5", link(12, 12)),
            ("src/main.rs#L12-L20", link(12, 20)),
            ("file:///work/src/main.rs#L3", link(3, 3)),
            ("src/main.rs#intro", None),
        ];
        for (written, lines) in cases {
            let read = file_link(written).expect("a link to a file");
            assert_eq!(read.lines, lines, "{written}");
            assert!(read.path.ends_with("src/main.rs"), "{written}");
        }
        for not_a_file in ["", "#intro", "mailto:a@example.com", "https://x.test/"] {
            assert_eq!(file_link(not_a_file), None, "{not_a_file}");
        }
    }

    #[test]
    fn an_address_ends_where_japanese_starts() {
        let text = "https://github.com/tennashi/cox/pull/26を見て";

        let end = text.find('を').expect("the text has を");

        assert_eq!(addresses(text), [Range { start: 0, end }]);
        assert_eq!(
            addresses("https:// and xhttps://example.com"),
            Vec::<Range<usize>>::new()
        );
    }

    #[test]
    fn an_address_split_by_styles_links_as_a_whole() {
        let [
            Block {
                kind: Kind::Paragraph(text),
                ..
            },
        ] = &parse("at https://example.com/**a**")[..]
        else {
            panic!("one paragraph");
        };

        assert!(
            text.runs
                .iter()
                .filter(|(range, _)| range.start >= 3)
                .all(|(_, style)| style.link.as_deref() == Some("https://example.com/a")),
            "{text:?}"
        );
    }
}
