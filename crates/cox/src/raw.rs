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

/// Who kept a source, and when.
#[derive(Clone, Copy, Debug)]
pub struct Kept<'a> {
    /// The day, as `2026-11-06`.
    pub day: &'a str,
    /// The chat, by its name.
    pub chat: &'a str,
    /// The number the chat is kept by, if it is kept.
    pub chat_id: Option<&'a str>,
    /// The chat's project, by its name, if it is in one.
    pub project: Option<&'a str>,
}

/// Keeps `source` in the folder at `root`, under `topic`, named after
/// the day it is kept and its title, its properties saying where it was
/// read, and, as `kept` says, the day, the chat that kept it, and its
/// project, each apart, for a tool such as Obsidian's to sort them by. One kept the same day under the same
/// title gets a name of its own, as `...-2`, since none is written again.
/// Answers the source's name, as `rust-news/2026-11-06-rust-1-92`.
///
/// # Errors
///
/// When `topic` is no folder's name, `source` says nothing or nowhere
/// where it was read, or it cannot be written.
pub fn keep(root: &Path, topic: &str, source: Source, kept: Kept) -> Result<String, String> {
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
    let day = kept.day;
    let mut properties = format!(
        "source: {}\nkept: {day}\nchat: {}\n",
        quoted(from),
        quoted(kept.chat)
    );
    if let Some(chat_id) = kept.chat_id {
        // A number, as Obsidian sorts it, when it is one.
        let chat_id = if !chat_id.is_empty() && chat_id.bytes().all(|byte| byte.is_ascii_digit()) {
            chat_id.to_owned()
        } else {
            quoted(chat_id)
        };
        let _ = writeln!(properties, "chat_id: {chat_id}");
    }
    if let Some(project) = kept.project {
        let _ = writeln!(properties, "project: {}", quoted(project));
    }
    let page = format!("---\n{properties}---\n\n# {title}\n\n{text}\n");
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

/// The sources in the folder at `root`, as the stage's Knowledge page
/// lists them, at `cox://api/raw` (see [`crate::stage::plugins`]): the
/// last kept first, each with its topic, its title, and, as its
/// properties say, where it was read, the day it was kept, and the chat
/// and project that kept it.
#[must_use]
pub fn told(root: &Path) -> serde_json::Value {
    let mut names = of(root, None);
    names.sort_by(|a, b| newest_first(a, b));
    let sources: Vec<serde_json::Value> = names
        .iter()
        .filter_map(|name| {
            let page = fs::read_to_string(wiki::file(root, name).ok()?).ok()?;
            let mut told = properties(&page);
            let (_, text) = wiki::front(&page);
            let title = text
                .lines()
                .find_map(|line| line.strip_prefix("# "))
                .map_or_else(|| name.clone(), |title| title.trim().to_owned());
            told["name"] = name.clone().into();
            told["topic"] = name.rsplit_once('/').map_or("", |(topic, _)| topic).into();
            told["title"] = title.into();
            Some(told)
        })
        .collect();
    serde_json::json!({
        "root": root.display().to_string(),
        "sources": sources,
    })
}

/// The source `name` in the folder at `root`, as the Knowledge page
/// reads it, at `cox://api/raw/<name>`: as a page of the wiki is told
/// (see [`wiki::told::page_in`]), translated as `translated` does, with
/// where it was read, the day it was kept, and the chat and project that
/// kept it; or `None` when there is no such source.
#[must_use]
pub fn told_source(
    root: &Path,
    name: &str,
    translated: impl FnOnce(&str) -> Option<crate::translate::Translated>,
) -> Option<serde_json::Value> {
    let page = fs::read_to_string(wiki::file(root, name).ok()?).ok()?;
    let mut told = wiki::told::page_in(root, name, translated)?;
    for (key, value) in properties(&page).as_object().into_iter().flatten() {
        told[key] = value.clone();
    }
    Some(told)
}

/// What the properties of the source `page` say of it: where it was read
/// (`source`), the day (`kept`), the chat (`chat`) and the project
/// (`project`), each `null` when they do not say it.
fn properties(page: &str) -> serde_json::Value {
    let (lines, _) = wiki::front(page);
    let value = |key: &str| {
        lines
            .iter()
            .find_map(|line| wiki::property(line, key))
            .map(unquoted)
    };
    serde_json::json!({
        "source": value("source"),
        "kept": value("kept"),
        "chat": value("chat"),
        "project": value("project"),
    })
}

/// `value` as YAML reads it, unquoted, as [`quoted`] quotes it.
fn unquoted(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return value
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .unwrap_or(value)
            .to_owned();
    };
    let mut plain = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => plain.push('\n'),
                Some(escaped) => plain.push(escaped),
                None => {}
            }
        } else {
            plain.push(c);
        }
    }
    plain
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

    fn by(day: &str) -> Kept<'_> {
        Kept {
            day,
            chat: "rust-news 11/06 09:00",
            chat_id: Some("12"),
            project: Some("notes"),
        }
    }

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
        let name = keep(&root, "rust-news", source, by("2026-11-06")).unwrap();
        assert_eq!(name, "rust-news/2026-11-06-announcing-rust-1-92-0");
        let kept = read(&root, &name).unwrap();
        assert_eq!(
            kept,
            "---\n\
             source: \"https://blog.rust-lang.org/2026/11/06/Rust-1.92.0/\"\n\
             kept: 2026-11-06\n\
             chat: \"rust-news 11/06 09:00\"\n\
             chat_id: 12\n\
             project: \"notes\"\n\
             ---\n\n\
             # Announcing Rust 1.92.0\n\n\
             Rust 1.92.0 stabilizes never type fallback.\n"
        );
        // One kept in no project says none.
        let home = Kept {
            project: None,
            ..by("2026-11-06")
        };
        let again = keep(&root, "rust-news", release("Updated.\n"), home).unwrap();
        assert_eq!(again, "rust-news/2026-11-06-announcing-rust-1-92-0-2");
        assert_eq!(read(&root, &name).unwrap(), kept);
        assert!(!read(&root, &again).unwrap().contains("project:"));
        assert!(keep(&root, "../out", source, by("2026-11-06")).is_err());
        assert!(keep(&root, "rust-news", release(" \n"), by("2026-11-06")).is_err());
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
            keep(&root, topic, source, by(day)).unwrap()
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

    #[test]
    fn the_sources_are_told_the_last_kept_first_with_where_each_was_read() {
        let root = scratch("told");
        let old = keep(&root, "rust-news", release("Old.\n"), by("2026-11-05")).unwrap();
        let new = keep(&root, "rust-news", release("New.\n"), by("2026-11-06")).unwrap();
        let told = told(&root);
        let sources = told["sources"].as_array().unwrap();
        assert_eq!(sources[0]["name"], new.as_str());
        assert_eq!(sources[1]["name"], old.as_str());
        assert_eq!(sources[0]["topic"], "rust-news");
        assert_eq!(sources[0]["title"], "Announcing Rust 1.92.0");
        assert_eq!(
            sources[0]["source"],
            "https://blog.rust-lang.org/2026/11/06/Rust-1.92.0/"
        );
        assert_eq!(sources[0]["kept"], "2026-11-06");
        assert_eq!(sources[0]["chat"], "rust-news 11/06 09:00");
        assert_eq!(sources[0]["project"], "notes");

        let page = told_source(&root, &new, |_| None).unwrap();
        assert_eq!(page["chat"], "rust-news 11/06 09:00");
        assert!(page["html"].as_str().unwrap().contains("New."));
        assert_eq!(told_source(&root, "rust-news/none", |_| None), None);
    }

    #[test]
    fn a_quoted_property_is_read_as_it_was_written() {
        assert_eq!(
            unquoted(&quoted("say \"hi\"\\ there\n")),
            "say \"hi\"\\ there\n"
        );
        assert_eq!(unquoted("'plain'"), "plain");
        assert_eq!(unquoted("2026-11-06"), "2026-11-06");
    }
}
