//! The agent's replies as Markdown: the blocks a reply is made of, and the
//! styles of their text, for the window to draw. Plain Rust, without GPUI.
//!
//! Text keeps the line breaks the agent wrote, rather than joining lines
//! into one as Markdown does: a reply reads as the agent laid it out, and
//! lines of Japanese do not gain spaces between them. Web addresses in the
//! text are links even where the agent did not mark them as such, as in
//! what a command printed.

use std::ops::Range;

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
    /// Where the text links to: an address on the web. Links elsewhere,
    /// such as to a file of the repository, have nowhere to open and stay
    /// plain text.
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

    /// Makes the web addresses in the text that link nowhere yet links to
    /// themselves.
    fn link_addresses(&mut self) {
        for address in addresses(&self.text) {
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

    /// The text with its web addresses linked, and no other style.
    #[must_use]
    pub fn plain(text: &str) -> Self {
        let mut plain = Self::default();
        plain.push(text, &Style::default());
        plain.link_addresses();
        plain
    }
}

/// Whether `url` is an address on the web, which the stage can open.
fn is_on_the_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
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
        text.link_addresses();
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
                if is_on_the_web(&dest_url) {
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
                    cell.link_addresses();
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
    fn links_only_to_the_web() {
        let [
            Block {
                kind: Kind::Paragraph(text),
                ..
            },
        ] = &parse("[PR](https://github.com/tennashi/cox/pull/1) and [file](src/main.rs)")[..]
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
                (" and file", None),
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
