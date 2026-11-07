//! The human's wiki: what the agents learned, kept as Markdown pages by
//! topic, for the agents of every project to find again. A page is a
//! file, as `gpui-layout.md` or `rust/errors.md`, in the wiki's folder,
//! which `settings.json` names, as `{"wiki": "~/notes/cox"}`, an Obsidian
//! vault or a Git repository of the human's, or else `wiki` where Cox
//! keeps what lasts (see [`crate::settings::wiki`]).
//!
//! The agents read and write it with Cox's tools (see
//! [`crate::agent::mcp`]), and every page they write goes through Cox,
//! which adds where it was used: a line under the page's "Used in", of
//! the day, the project, the chat, the commit the chat's folder was at,
//! and the paths of the code the page is of, so that a page says where it
//! came from without the agent writing it. Cox keeps `index.md`, the
//! page that lists the others, unless the human wrote their own. The
//! stage's Knowledge plugin shows the pages to the human (see [`told`]).
//!
//! A project has a wiki of its own too, for what holds only of its work
//! or its people, in `.cox/wiki` in its folder, where its repository can
//! keep it: only the chats of that project find its pages, read them, or
//! write them, the project's before the human's. A page of the human's
//! wiki that an earlier Cox kept in one project says so in its
//! properties, the front matter Obsidian keeps between `---` lines at the
//! page's top, as `only: <project>`, and only the chats of that project
//! see it either. The human sees every page, in the plugin as in the
//! folder.
//!
//! The agents write the pages in English, whatever language their chat
//! speaks, so that a page is found by the same words from every chat;
//! the human reads them in a language of their own, as Cox translates
//! them (see [`crate::translate`]).

pub mod told;

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The page that lists the others.
pub const INDEX: &str = "index";

/// The folder of a project that holds its own wiki.
pub const PROJECT_WIKI: &str = ".cox/wiki";

/// The heading under which a page says where it was used, which Cox
/// writes.
const USED_IN: &str = "## Used in";

/// The property that keeps a page in one project, as `only: cox`.
const ONLY: &str = "only";

/// The first line of an index Cox keeps, which tells it from one the
/// human wrote, which Cox leaves be.
const KEPT_INDEX: &str = "<!-- Cox keeps this index as pages are written. -->";

/// The most pages a search answers.
const PAGES_FOUND: usize = 10;

/// The most lines a search answers of each page.
const LINES_FOUND: usize = 3;

/// The most characters of a line a search answers.
const LINE_LENGTH: usize = 200;

/// The most characters of what a page is about, as the index lists it.
const ABOUT_LENGTH: usize = 120;

/// Pages are written one at a time, whichever chat writes them, so that
/// two writing the same page at once keep both of their "Used in" lines.
static WRITING: Mutex<()> = Mutex::new(());

/// Where a page was used, as Cox notes it under the page's "Used in".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Used {
    /// The day, as `2026-11-05`.
    pub day: String,
    /// The project, by its name.
    pub project: String,
    /// The chat, by its name.
    pub chat: String,
    /// The number the chat is kept by, if it is kept.
    pub kept: Option<String>,
    /// The commit the chat's folder was at, short, if it is in a Git
    /// repository with one.
    pub commit: Option<String>,
    /// The paths of the code the page is of, from the chat's folder.
    pub paths: Vec<String>,
}

impl Used {
    /// The line Cox notes it as, after the day: the same for the same
    /// chat at the same commit of the same code, whatever the day.
    fn what(&self) -> String {
        let mut what = format!("in {}, chat \"{}\"", self.project, self.chat);
        if let Some(kept) = &self.kept {
            let _ = write!(what, " (#{kept})");
        }
        if let Some(commit) = &self.commit {
            let _ = write!(what, ", at {commit}");
        }
        if !self.paths.is_empty() {
            let _ = write!(what, ": {}", self.paths.join(", "));
        }
        what
    }

    /// The line Cox notes it as.
    #[must_use]
    pub fn line(&self) -> String {
        format!("- {} {}", self.day, self.what())
    }
}

/// The day it is, as a "Used in" line says it.
#[must_use]
pub fn today() -> String {
    jiff::Timestamp::now().strftime("%Y-%m-%d").to_string()
}

/// The commit the Git repository of `folder` is at, short, if it is in
/// one with a commit.
#[must_use]
pub fn commit_of(folder: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(folder)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let commit = String::from_utf8(output.stdout).ok()?;
    let commit = commit.trim();
    (output.status.success() && !commit.is_empty()).then(|| commit.to_owned())
}

/// A page a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The page's name, as `rust/errors`.
    pub name: String,
    /// Its title, the text of its first heading, or its name.
    pub title: String,
    /// The lines that have what was searched for, the first few.
    pub lines: Vec<String>,
}

/// The file of the page `name` in the wiki at `root`.
///
/// # Errors
///
/// When `name` is no page's name: empty, or a path that leaves the wiki
/// or goes into a hidden folder, such as an Obsidian vault's own.
pub fn file(root: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim().trim_end_matches(".md");
    if name.is_empty() {
        return Err("no page: give the page's name, as `gpui-layout` or `rust/errors`".into());
    }
    let parts: Vec<&str> = name.split('/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || part.starts_with('.') || part.contains('\\'))
    {
        return Err(format!(
            "`{name}` is no page's name: a name is words and `/`, as `rust/errors`, with no \
             part empty or starting with a dot"
        ));
    }
    Ok(root.join(format!("{name}.md")))
}

/// The pages of the wiki at `root`, by name, in order: every Markdown
/// file in it, but in hidden folders.
#[must_use]
pub fn pages(root: &Path) -> Vec<String> {
    let mut pages = Vec::new();
    let mut folders = vec![PathBuf::new()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = fs::read_dir(root.join(&folder)) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let Some(name) = name.to_str().filter(|name| !name.starts_with('.')) else {
                continue;
            };
            let path = folder.join(name);
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => folders.push(path),
                Ok(_) => {
                    if let Some(page) = name.strip_suffix(".md") {
                        let page = folder.join(page);
                        if let Some(page) = page.to_str() {
                            pages.push(page.replace('\\', "/"));
                        }
                    }
                }
                Err(_) => {}
            }
        }
    }
    pages.sort();
    pages
}

/// `page` as its properties, the lines of the front matter between the
/// `---` lines at its top, as Obsidian keeps them, and the rest of it.
#[must_use]
pub fn front(page: &str) -> (Vec<&str>, &str) {
    let Some(rest) = page
        .strip_prefix("---\n")
        .or_else(|| page.strip_prefix("---\r\n"))
    else {
        return (Vec::new(), page);
    };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return (rest[..at].lines().collect(), &rest[at + line.len()..]);
        }
        at += line.len();
    }
    (Vec::new(), page)
}

/// The value of the property `line` sets, if it sets `key`.
#[must_use]
pub fn property<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (named, value) = line.split_once(':')?;
    (named.trim() == key).then(|| value.trim())
}

/// The project `page` is kept in, if its properties keep it in one.
#[must_use]
pub fn only(page: &str) -> Option<String> {
    let value = front(page)
        .0
        .into_iter()
        .find_map(|line| property(line, ONLY))?;
    let value = match value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        Some(quoted) => quoted.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => value
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .unwrap_or(value)
            .to_owned(),
    };
    (!value.is_empty()).then_some(value)
}

/// The property that keeps a page in `project`, quoted when YAML would
/// read it otherwise.
fn only_line(project: &str) -> String {
    let plain = !project.is_empty()
        && project.trim() == project
        && !project.contains(|c: char| ":#\"'\\[]{},&*!|>%@`".contains(c));
    if plain {
        format!("{ONLY}: {project}")
    } else {
        let quoted = project.replace('\\', "\\\\").replace('"', "\\\"");
        format!("{ONLY}: \"{quoted}\"")
    }
}

/// Whether a chat of `project` sees `page`: kept in no project, or in
/// that one.
fn seen(page: &str, project: &str) -> bool {
    only(page).is_none_or(|only| only == project)
}

/// The pages of the wiki at `root` that a chat of `project` does not
/// see.
fn unseen(root: &Path, project: &str) -> Vec<String> {
    pages(root)
        .into_iter()
        .filter(|name| {
            fs::read_to_string(root.join(format!("{name}.md")))
                .is_ok_and(|page| !seen(&page, project))
        })
        .collect()
}

/// The text of the page `name` in the wiki at `root`, as a chat of
/// `project` reads it: a page kept in another project is no page to it,
/// and the index lists none.
///
/// # Errors
///
/// When there is no such page, or it cannot be read.
pub fn read(root: &Path, name: &str, project: &str) -> Result<String, String> {
    let file = file(root, name)?;
    let no_page = || format!("the wiki has no page `{name}`");
    let text = fs::read_to_string(&file).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => no_page(),
        _ => format!("cannot read {}: {error}", file.display()),
    })?;
    if !seen(&text, project) {
        return Err(no_page());
    }
    if name.trim().trim_end_matches(".md") != INDEX {
        return Ok(text);
    }
    let unseen = unseen(root, project);
    let mut index = String::new();
    for line in text.split_inclusive('\n') {
        let links = |name: &String| {
            line.contains(&format!("[[{name}]]"))
                || line.contains(&format!("[[{name}|"))
                || line.contains(&format!("({name}.md)"))
        };
        if !unseen.iter().any(links) {
            index.push_str(line);
        }
    }
    Ok(index)
}

/// The pages of the wiki at `root`, but the index and those kept in
/// another project than `project`, that have every word of `query`, in
/// their names or their text, past their properties, whatever the case:
/// those that have them most first, each with the first few of its lines
/// that have one.
#[must_use]
pub fn search(root: &Path, query: &str, project: &str) -> Vec<Found> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(usize, Found)> = pages(root)
        .into_iter()
        // The index has every page's title, so it would be found for each.
        .filter(|name| name != INDEX)
        .filter_map(|name| {
            let page = fs::read_to_string(file(root, &name).ok()?).ok()?;
            if !seen(&page, project) {
                return None;
            }
            let (_, text) = front(&page);
            let lower = text.to_lowercase();
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
            let lines = text
                .lines()
                // The title is answered as the title.
                .filter(|line| !line.starts_with("# "))
                .filter(|line| {
                    let line = line.to_lowercase();
                    words.iter().any(|word| line.contains(word))
                })
                .take(LINES_FOUND)
                .map(|line| cut(line.trim(), LINE_LENGTH))
                .collect();
            Some((
                count,
                Found {
                    title: title(text).unwrap_or_else(|| name.clone()),
                    name,
                    lines,
                },
            ))
        })
        .collect();
    found.sort_by(|(one, a), (other, b)| other.cmp(one).then_with(|| a.name.cmp(&b.name)));
    found
        .into_iter()
        .take(PAGES_FOUND)
        .map(|(_, found)| found)
        .collect()
}

/// Writes `text` as the page `name` in the wiki at `root`, noting under
/// its "Used in" that it was used as `used` says, after where it was used
/// before; and lists it in the index, when Cox keeps it. Answers the
/// page's file.
///
/// The page is kept in `used`'s project when `keep` is `Some(true)`, in
/// none when it is `Some(false)`, and as it was otherwise. What `text`
/// says under a "Used in" of its own is Cox's to write, so it is left
/// out, as the project it keeps the page in is; its other properties are
/// the page's, or, when it has none, those the page had.
///
/// # Errors
///
/// When `name` is no page's name, or that of a page kept in another
/// project, `text` says nothing, or the page cannot be written.
pub fn write(
    root: &Path,
    name: &str,
    text: &str,
    used: &Used,
    keep: Option<bool>,
) -> Result<PathBuf, String> {
    let file = file(root, name)?;
    let (given, text) = front(text);
    let (body, _) = split(text);
    let body = body.trim();
    if body.is_empty() {
        return Err("no text: give the whole page as `text`, from its `# ` title".into());
    }
    let _writing = WRITING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let was = fs::read_to_string(&file).unwrap_or_default();
    if !seen(&was, &used.project) {
        return Err(format!(
            "the wiki has a page `{name}` that another project keeps to itself: name this \
             one otherwise"
        ));
    }
    let (had, was_text) = front(&was);
    let mut properties: Vec<String> = if given.is_empty() { had } else { given }
        .into_iter()
        .filter(|line| property(line, ONLY).is_none())
        .map(str::to_owned)
        .collect();
    let kept = match keep {
        Some(true) => Some(used.project.clone()),
        Some(false) => None,
        None => only(&was),
    };
    if let Some(project) = kept {
        properties.push(only_line(&project));
    }
    let (_, mut lines) = split(was_text);
    let what = used.what();
    // A line a day: the same use again moves to the end, as of today.
    lines.retain(|line| {
        let after_day = line
            .strip_prefix("- ")
            .and_then(|line| line.split_once(' '))
            .map(|(_, what)| what);
        after_day != Some(what.as_str())
    });
    lines.push(used.line());
    let mut page = String::new();
    if !properties.is_empty() {
        let _ = write!(page, "---\n{}\n---\n\n", properties.join("\n"));
    }
    let _ = write!(page, "{body}\n\n{USED_IN}\n\n");
    for line in &lines {
        page.push_str(line);
        page.push('\n');
    }
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)
            .map_err(|error| format!("cannot make {}: {error}", folder.display()))?;
    }
    fs::write(&file, page).map_err(|error| format!("cannot write {}: {error}", file.display()))?;
    if let Err(error) = keep_index(root) {
        tracing::warn!("the wiki's index is not kept: {error}");
    }
    Ok(file)
}

/// `text` as the page's own text, and the lines under its "Used in".
fn split(text: &str) -> (&str, Vec<String>) {
    let at = text
        .match_indices(USED_IN)
        .find(|(at, _)| *at == 0 || text[..*at].ends_with('\n'))
        .map(|(at, _)| at);
    match at {
        Some(at) => {
            let lines = text[at + USED_IN.len()..]
                .lines()
                .map(str::trim)
                .take_while(|line| !line.starts_with('#'))
                .filter(|line| line.starts_with("- "))
                .map(str::to_owned)
                .collect();
            (&text[..at], lines)
        }
        None => (text, Vec::new()),
    }
}

/// Writes the index of the wiki at `root`, a line for each page, with
/// its title and what it is about, unless the human wrote an index of
/// their own.
fn keep_index(root: &Path) -> io::Result<()> {
    let index = root.join(format!("{INDEX}.md"));
    match fs::read_to_string(&index) {
        Ok(text) if !text.starts_with(KEPT_INDEX) => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut text = format!("{KEPT_INDEX}\n# Index\n\n");
    for name in pages(root).into_iter().filter(|name| name != INDEX) {
        let Ok(page) = fs::read_to_string(root.join(format!("{name}.md"))) else {
            continue;
        };
        let title = title(&page).unwrap_or_else(|| name.clone());
        let _ = write!(text, "- [[{name}|{title}]]");
        if let Some(about) = about(&page) {
            let _ = write!(text, ": {about}");
        }
        text.push('\n');
    }
    fs::write(index, text)
}

/// The text of the first heading of `page`, if it has one.
fn title(page: &str) -> Option<String> {
    front(page)
        .1
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())
}

/// What `page` is about: its first line of text, past its title, cut
/// short.
fn about(page: &str) -> Option<String> {
    let (body, _) = split(front(page).1);
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("<!--"))
        .map(|line| cut(line, ABOUT_LENGTH))
}

/// `text`, cut to `length` characters, with an ellipsis when cut.
fn cut(text: &str, length: usize) -> String {
    match text.char_indices().nth(length) {
        Some((end, _)) => format!("{}\u{2026}", text[..end].trim_end()),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("cox-wiki-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    fn used(project: &str, chat: &str, day: &str) -> Used {
        Used {
            day: day.to_owned(),
            project: project.to_owned(),
            chat: chat.to_owned(),
            kept: Some("3".to_owned()),
            commit: Some("9955f39".to_owned()),
            paths: vec!["src/view.rs".to_owned()],
        }
    }

    #[test]
    fn a_page_says_where_it_was_used_as_cox_notes_it() {
        let root = scratch("used");
        let page = "# GPUI layout\n\nA flex child needs `min_h_0` to scroll.\n";
        write(
            &root,
            "gpui-layout",
            page,
            &used("cox", "Fix scroll", "2026-11-04"),
            None,
        )
        .unwrap();
        // The agent's own "Used in" is left out, and the same use again
        // moves to the end, as of the day.
        let again = "# GPUI layout\n\nA flex child needs `min_h_0` to scroll, in a column.\n\n\
                     ## Used in\n\n- made up\n";
        write(
            &root,
            "gpui-layout",
            again,
            &used("notes", "Layout", "2026-11-05"),
            None,
        )
        .unwrap();
        write(
            &root,
            "gpui-layout",
            again,
            &used("cox", "Fix scroll", "2026-11-05"),
            None,
        )
        .unwrap();
        assert_eq!(
            read(&root, "gpui-layout", "cox").unwrap(),
            "# GPUI layout\n\nA flex child needs `min_h_0` to scroll, in a column.\n\n\
             ## Used in\n\n\
             - 2026-11-05 in notes, chat \"Layout\" (#3), at 9955f39: src/view.rs\n\
             - 2026-11-05 in cox, chat \"Fix scroll\" (#3), at 9955f39: src/view.rs\n"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_index_lists_the_pages_unless_the_human_wrote_it() {
        let root = scratch("index");
        let day = "2026-11-05";
        write(
            &root,
            "rust/errors",
            "# Errors\n\nUse anyhow at the top.\n",
            &used("a", "c", day),
            None,
        )
        .unwrap();
        write(
            &root,
            "acp",
            "# ACP\n\nSessions resume.\n",
            &used("b", "c", day),
            None,
        )
        .unwrap();
        assert_eq!(
            read(&root, INDEX, "a").unwrap(),
            format!(
                "{KEPT_INDEX}\n# Index\n\n- [[acp|ACP]]: Sessions resume.\n\
                 - [[rust/errors|Errors]]: Use anyhow at the top.\n"
            )
        );
        fs::write(root.join("index.md"), "# Mine\n").unwrap();
        write(
            &root,
            "more",
            "# More\n\nText.\n",
            &used("b", "c", day),
            None,
        )
        .unwrap();
        assert_eq!(read(&root, INDEX, "a").unwrap(), "# Mine\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_search_finds_the_pages_with_every_word() {
        let root = scratch("search");
        let day = "2026-11-05";
        write(
            &root,
            "gpui",
            "# GPUI\n\nLayout is taffy.\nTaffy is slow in debug.\n",
            &used("a", "c", day),
            None,
        )
        .unwrap();
        write(
            &root,
            "taffy",
            "# Taffy\n\nFlexbox.\n",
            &used("a", "c", day),
            None,
        )
        .unwrap();
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::write(root.join(".obsidian/taffy.md"), "taffy").unwrap();
        let found = search(&root, "TAFFY", "a");
        let names: Vec<&str> = found.iter().map(|found| found.name.as_str()).collect();
        assert_eq!(names, ["gpui", "taffy"]);
        assert_eq!(found[0].title, "GPUI");
        // The title is answered apart, and not as a line found.
        assert_eq!(found[1].title, "Taffy");
        assert_eq!(found[1].lines, [] as [String; 0]);
        assert_eq!(
            found[0].lines,
            ["Layout is taffy.", "Taffy is slow in debug."]
        );
        let names: Vec<String> = search(&root, "taffy debug", "a")
            .into_iter()
            .map(|found| found.name)
            .collect();
        assert_eq!(names, ["gpui"]);
        assert_eq!(search(&root, "  ", "a"), []);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_page_kept_in_a_project_is_only_that_projects() {
        let root = scratch("only");
        let day = "2026-11-05";
        let page =
            "---\ntags: [billing]\nonly: made-up\n---\n\n# Billing\n\nAcme pays by the seat.\n";
        write(&root, "billing", page, &used("acme", "c", day), Some(true)).unwrap();
        write(
            &root,
            "seats",
            "# Seats\n\nA seat is a login.\n",
            &used("b", "c", day),
            None,
        )
        .unwrap();
        let text = read(&root, "billing", "acme").unwrap();
        // The page is kept where it was written, whatever the agent wrote.
        assert!(
            text.starts_with("---\ntags: [billing]\nonly: acme\n---\n\n# Billing\n"),
            "{text}"
        );
        assert_eq!(only(&text).as_deref(), Some("acme"));
        assert_eq!(title(&text).as_deref(), Some("Billing"));
        assert_eq!(about(&text).as_deref(), Some("Acme pays by the seat."));
        let names = |project| -> Vec<String> {
            search(&root, "seat", project)
                .into_iter()
                .map(|found| found.name)
                .collect()
        };
        assert_eq!(names("acme"), ["seats", "billing"]);
        assert_eq!(names("b"), ["seats"]);
        // Its properties are not searched.
        assert_eq!(search(&root, "tags", "acme"), []);
        assert!(read(&root, "billing", "b").is_err());
        let index = read(&root, INDEX, "b").unwrap();
        assert!(!index.contains("billing"), "{index}");
        assert!(
            read(&root, INDEX, "acme")
                .unwrap()
                .contains("[[billing|Billing]]")
        );
        assert!(
            write(
                &root,
                "billing",
                "# Mine\n\nText.\n",
                &used("b", "c", day),
                None
            )
            .is_err()
        );
        // Written again, it stays kept, with its properties, until it is
        // let out.
        write(
            &root,
            "billing",
            "# Billing\n\nBy the seat.\n",
            &used("acme", "c", day),
            None,
        )
        .unwrap();
        let text = read(&root, "billing", "acme").unwrap();
        assert!(
            text.starts_with("---\ntags: [billing]\nonly: acme\n---\n"),
            "{text}"
        );
        write(
            &root,
            "billing",
            "# Billing\n\nBy the seat.\n",
            &used("acme", "c", day),
            Some(false),
        )
        .unwrap();
        let text = read(&root, "billing", "b").unwrap();
        assert!(
            text.starts_with("---\ntags: [billing]\n---\n\n# Billing\n"),
            "{text}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_is_kept_in_a_line_yaml_reads() {
        assert_eq!(only_line("cox"), "only: cox");
        assert_eq!(only_line("my app"), "only: my app");
        assert_eq!(only_line("a: \"b\""), r#"only: "a: \"b\"""#);
        for project in ["cox", "my app", "a: \"b\"", "c:\\d", "'x'"] {
            let page = format!("---\n{}\n---\n# T\n", only_line(project));
            assert_eq!(only(&page).as_deref(), Some(project), "{page}");
        }
        assert_eq!(only("# T\n\nonly: cox\n"), None);
    }

    #[test]
    fn a_name_stays_in_the_wiki() {
        let root = Path::new("/wiki");
        assert_eq!(
            file(root, "rust/errors"),
            Ok(PathBuf::from("/wiki/rust/errors.md"))
        );
        assert_eq!(file(root, "acp.md"), Ok(PathBuf::from("/wiki/acp.md")));
        assert!(file(root, "").is_err());
        assert!(file(root, "../x").is_err());
        assert!(file(root, "/x").is_err());
        assert!(file(root, ".obsidian/x").is_err());
        assert!(file(root, "a//b").is_err());
    }
}
