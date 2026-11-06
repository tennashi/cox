//! What the agents read, kept as they read it: a web page, a paper, a
//! release note or a command's output, as the sources of research the
//! human has an agent do, as on a schedule (see [`crate::schedule`]).
//! What is made of them, as a report, or a page of the wiki (see
//! [`crate::wiki`]), is for whoever makes it, which reads them again
//! from here and says which it was made from.
//!
//! Each source is a Markdown file in the folder `settings.json` names,
//! as `{"raw": "~/notes/raw"}`, or else `raw` in the home (see
//! [`crate::settings::raw`]), in a folder of its topic, as the research
//! it is of, named after the day it was kept and its title, as
//! `rust-news/2026-11-06-announcing-rust-1-92-0.md`. Its properties, the
//! front matter between `---` lines at its top, as Obsidian keeps them,
//! say where it was read, the day, and the chat that kept it. A source
//! is never written again: what changed is another source, kept beside
//! it.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::Path;

use crate::wiki::{self, Found};

/// The folder in the home that keeps the sources, unless the settings
/// name another.
pub const RAW: &str = "raw";

/// The most characters of a source's name, past its day.
const NAME_LENGTH: usize = 60;

/// The most sources a list answers.
const LISTED: usize = 50;

/// The most sources a search answers.
const FOUND: usize = 10;

/// The most lines a search answers of each source.
const LINES_FOUND: usize = 3;

/// The most characters of a line a search answers.
const LINE_LENGTH: usize = 200;

/// A source to keep: what an agent read, as it read it.
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    /// Where it was read: its URL, or the command or the file it came
    /// from.
    pub from: &'a str,
    /// Its title, as the source names itself.
    pub title: &'a str,
    /// What it says, as Markdown.
    pub text: &'a str,
}

/// Keeps `source` in the folder at `root`, under `topic`, named after
/// `day` and its title, its properties saying where it was read, the day,
/// and `by`, the chat that kept it. One kept the same day under the same
/// title gets a name of its own, as `...-2`, since none is written again.
/// Answers the source's name, as `rust-news/2026-11-06-rust-1-92`.
///
/// # Errors
///
/// When `topic` is no folder's name, `source` says nothing or nowhere
/// where it was read, or it cannot be written.
pub fn keep(
    root: &Path,
    topic: &str,
    source: Source,
    by: &str,
    day: &str,
) -> Result<String, String> {
    let topic = topic.trim().trim_matches('/');
    wiki::file(root, topic)
        .map_err(|_| format!("`{topic}` is no topic: a topic is words and `/`, as `rust-news`"))?;
    let from = source.from.trim();
    if from.is_empty() {
        return Err("no source: say where it was read, its URL or its command, as `from`".into());
    }
    let text = source.text.trim();
    if text.is_empty() {
        return Err("no text: give what the source says, as it was read, as `text`".into());
    }
    let title = source.title.trim();
    let title = if title.is_empty() { from } else { title };
    let page = format!(
        "---\nsource: {}\nkept: {day}\nby: {}\n---\n\n# {title}\n\n{text}\n",
        quoted(from),
        quoted(by)
    );
    let folder = root.join(topic);
    fs::create_dir_all(&folder)
        .map_err(|error| format!("cannot make {}: {error}", folder.display()))?;
    let stem = format!("{topic}/{day}-{}", slug(title));
    for count in 1.. {
        let name = if count == 1 {
            stem.clone()
        } else {
            format!("{stem}-{count}")
        };
        let file = wiki::file(root, &name)?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
        {
            Ok(mut kept) => {
                return kept
                    .write_all(page.as_bytes())
                    .map(|()| name)
                    .map_err(|error| format!("cannot write {}: {error}", file.display()));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("cannot write {}: {error}", file.display())),
        }
    }
    unreachable!("a name of its own is found")
}

/// The text of the source `name` in the folder at `root`.
///
/// # Errors
///
/// When there is no such source, or it cannot be read.
pub fn read(root: &Path, name: &str) -> Result<String, String> {
    let file = wiki::file(root, name)?;
    fs::read_to_string(&file).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => format!("no source is kept as `{name}`"),
        _ => format!("cannot read {}: {error}", file.display()),
    })
}

/// The sources in the folder at `root`, or of `topic` in it, by name,
/// the last kept first, the first few.
#[must_use]
pub fn list(root: &Path, topic: Option<&str>) -> Vec<String> {
    let mut names = of(root, topic);
    names.sort_by(|a, b| newest_first(a, b));
    names.truncate(LISTED);
    names
}

/// The sources in the folder at `root`, or of `topic` in it, that have
/// every word of `query`, in their names or their text, whatever the
/// case: those that have them most first, each with the first few of its
/// lines that have one.
#[must_use]
pub fn search(root: &Path, query: &str, topic: Option<&str>) -> Vec<Found> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(usize, Found)> = of(root, topic)
        .into_iter()
        .filter_map(|name| {
            let page = fs::read_to_string(wiki::file(root, &name).ok()?).ok()?;
            let lower = page.to_lowercase();
            let named = name.to_lowercase();
            if !words
                .iter()
                .all(|word| lower.contains(word) || named.contains(word))
            {
                return None;
            }
            let count = words
                .iter()
                .map(|word| {
                    lower.matches(word.as_str()).count() + named.matches(word.as_str()).count()
                })
                .sum();
            let (_, text) = wiki::front(&page);
            let title = text
                .lines()
                .find_map(|line| line.strip_prefix("# "))
                .map_or_else(|| name.clone(), |title| title.trim().to_owned());
            let lines = text
                .lines()
                .filter(|line| !line.starts_with("# "))
                .filter(|line| {
                    let line = line.to_lowercase();
                    words.iter().any(|word| line.contains(word))
                })
                .take(LINES_FOUND)
                .map(|line| line.trim().chars().take(LINE_LENGTH).collect())
                .collect();
            Some((count, Found { name, title, lines }))
        })
        .collect();
    found.sort_by(|(one, a), (other, b)| {
        other.cmp(one).then_with(|| newest_first(&a.name, &b.name))
    });
    found
        .into_iter()
        .take(FOUND)
        .map(|(_, found)| found)
        .collect()
}

/// The order of the sources named `a` and `b`, the last kept first, by
/// the day their names start with past their topic, and else by name.
fn newest_first(a: &str, b: &str) -> std::cmp::Ordering {
    let day = |name: &str| {
        let last = name.rsplit('/').next().unwrap_or(name);
        last.get(..10).unwrap_or(last).to_owned()
    };
    day(b).cmp(&day(a)).then_with(|| a.cmp(b))
}

/// The sources in the folder at `root`, or of `topic` in it, by name.
fn of(root: &Path, topic: Option<&str>) -> Vec<String> {
    match topic.map(|topic| topic.trim().trim_matches('/')) {
        Some(topic) if !topic.is_empty() => {
            if wiki::file(root, topic).is_err() {
                return Vec::new();
            }
            wiki::pages(&root.join(topic))
                .into_iter()
                .map(|name| format!("{topic}/{name}"))
                .collect()
        }
        _ => wiki::pages(root),
    }
}

/// `value` as YAML reads it, quoted.
fn quoted(value: &str) -> String {
    let mut quoted = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            c => quoted.push(c),
        }
    }
    let _ = write!(quoted, "\"");
    quoted
}

/// `title` as a part of a name: its letters and digits, lower case, each
/// run of the rest a `-`, cut short.
fn slug(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.chars().take(NAME_LENGTH).collect();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "source".to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("cox-raw-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    const BY: &str = "in notes, chat \"rust-news 11/06 09:00\" (#12)";

    fn release(text: &str) -> Source<'_> {
        Source {
            from: "https://blog.rust-lang.org/2026/11/06/Rust-1.92.0/",
            title: "Announcing Rust 1.92.0",
            text,
        }
    }

    #[test]
    fn a_source_is_kept_as_it_was_read_and_never_written_again() {
        let root = scratch("keep");
        let source = release("Rust 1.92.0 stabilizes never type fallback.\n");
        let name = keep(&root, "rust-news", source, BY, "2026-11-06").unwrap();
        assert_eq!(name, "rust-news/2026-11-06-announcing-rust-1-92-0");
        let kept = read(&root, &name).unwrap();
        assert_eq!(
            kept,
            "---\n\
             source: \"https://blog.rust-lang.org/2026/11/06/Rust-1.92.0/\"\n\
             kept: 2026-11-06\n\
             by: \"in notes, chat \\\"rust-news 11/06 09:00\\\" (#12)\"\n\
             ---\n\n\
             # Announcing Rust 1.92.0\n\n\
             Rust 1.92.0 stabilizes never type fallback.\n"
        );
        let again = keep(&root, "rust-news", release("Updated.\n"), BY, "2026-11-06").unwrap();
        assert_eq!(again, "rust-news/2026-11-06-announcing-rust-1-92-0-2");
        assert_eq!(read(&root, &name).unwrap(), kept);
        assert!(keep(&root, "../out", source, BY, "2026-11-06").is_err());
        assert!(keep(&root, "rust-news", release(" \n"), BY, "2026-11-06").is_err());
        assert!(read(&root, "rust-news/none").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_sources_are_listed_and_searched_by_topic() {
        let root = scratch("find");
        let day = |day: &str, title: &str, text: &str, topic: &str| {
            let source = Source {
                from: "https://example.com/",
                title,
                text,
            };
            keep(&root, topic, source, BY, day).unwrap()
        };
        let old = day(
            "2026-11-05",
            "Old",
            "Never type fallback, planned.",
            "rust-news",
        );
        let new = day(
            "2026-11-06",
            "New",
            "Never type fallback is stable.",
            "rust-news",
        );
        let other = day("2026-11-06", "Zig", "Never mind.", "zig-news");
        assert_eq!(list(&root, Some("rust-news")), [new.clone(), old.clone()]);
        assert_eq!(list(&root, None), [new.clone(), other.clone(), old.clone()]);
        assert_eq!(list(&root, Some("none")), Vec::<String>::new());
        let names = |found: Vec<Found>| -> Vec<String> {
            found.into_iter().map(|found| found.name).collect()
        };
        assert_eq!(
            names(search(&root, "NEVER fallback", None)),
            [new.clone(), old.clone()]
        );
        assert_eq!(names(search(&root, "never", Some("zig-news"))), [other]);
        let found = search(&root, "stable", None);
        assert_eq!(found[0].title, "New");
        assert_eq!(found[0].lines, ["Never type fallback is stable."]);
        assert_eq!(search(&root, " ", None), []);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_title_names_a_source() {
        assert_eq!(slug("Rust 1.92: ネバー型!"), "rust-1-92-ネバー型");
        assert_eq!(slug("!!!"), "source");
        assert_eq!(slug(&"a".repeat(100)).len(), NAME_LENGTH);
    }
}
