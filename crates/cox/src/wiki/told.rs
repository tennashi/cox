//! The wiki as the stage's Knowledge plugin reads it, at `cox://api/wiki` and
//! `cox://api/wiki/<page>` (see [`crate::stage::plugins`]): each page
//! with its title, what it is about, the pages it links to and those that
//! link to it, where it was used, the project it is kept in, if it is
//! kept in one, and what Cox finds wrong with it, which
//! it marks without an agent: links to no page, pages no page links to,
//! pages with no title, and pages that say nowhere where they were used,
//! as those the human wrote themselves.
//!
//! A link is `[[name]]`, or `[[name|what it says]]`, as Obsidian writes
//! them, or a Markdown link to a page's file, as `[errors](rust/errors.md)`.
//! A name that is no page's names the page of that last part, as
//! `[[errors]]` does `rust/errors`, when only one has it, as Obsidian
//! finds it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, html};
use serde_json::{Value, json};

use super::{INDEX, about, file, front, only, pages, split, title};
use crate::translate::Translated;

/// What Cox finds wrong with a page.
const BROKEN: &str = "broken";
const ORPHAN: &str = "orphan";
const UNTITLED: &str = "untitled";
const UNUSED: &str = "unused";

/// The Markdown a page is read as.
pub(crate) fn options() -> Options {
    Options::ENABLE_WIKILINKS
        | Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
}

/// A page, as the plugin is told of it.
struct Told {
    title: Option<String>,
    about: Option<String>,
    /// What the page says, past its "Used in".
    body: String,
    /// The pages it links to, as they are, or as the link names them when
    /// no page is so named.
    links: BTreeSet<String>,
    /// The links that name no page.
    broken: BTreeSet<String>,
    /// Where it was used, each as its day and the rest of its line.
    used: Vec<(String, String)>,
    /// The project it is kept in, if it is kept in one.
    only: Option<String>,
}

/// The wiki at `root`: its folder, and its pages, but the index, by name.
#[must_use]
pub fn told(root: &Path) -> Value {
    let all = read_all(root);
    let linked_from = linked_from(&all);
    let pages: Vec<Value> = all
        .iter()
        .map(|(name, page)| page_json(name, page, &linked_from))
        .collect();
    json!({
        "root": root.display().to_string(),
        "index": root.join(format!("{INDEX}.md")).display().to_string(),
        "pages": pages,
    })
}

/// The page `name` of the wiki at `root`, as [`told`] tells it, with its
/// text as HTML, its links going to `#` and the page's name, and its own
/// HTML shown as text; or `None` when there is no such page.
#[must_use]
pub fn page(root: &Path, name: &str) -> Option<Value> {
    page_in(root, name, |_| None)
}

/// The page `name` of the wiki at `root`, as [`page`] tells it, and, as
/// `translated` translates its text, as `translation`: the language,
/// the translated text as HTML, as the page's own is, how many of its
/// blocks are translated, of how many, and why translating failed last,
/// if it did.
#[must_use]
pub fn page_in(
    root: &Path,
    name: &str,
    translated: impl FnOnce(&str) -> Option<Translated>,
) -> Option<Value> {
    let all = read_all(root);
    let page = all.get(name)?;
    let linked_from = linked_from(&all);
    let names: Vec<&str> = all.keys().map(String::as_str).collect();
    let mut told = page_json(name, page, &linked_from);
    told["html"] = Value::String(html_of(name, &page.body, &names));
    told["file"] = Value::String(file(root, name).ok()?.display().to_string());
    told["translation"] = translated(&page.body).map_or(Value::Null, |translated| {
        json!({
            "language": translated.language,
            "html": html_of(name, &translated.markdown, &names),
            "done": translated.done,
            "of": translated.of,
            "error": translated.error,
        })
    });
    Some(told)
}

/// Every page of the wiki at `root` but the index, read, by name.
fn read_all(root: &Path) -> BTreeMap<String, Told> {
    let names: Vec<String> = pages(root)
        .into_iter()
        .filter(|name| name != INDEX)
        .collect();
    let all: Vec<&str> = names.iter().map(String::as_str).collect();
    names
        .iter()
        .filter_map(|name| {
            let text = fs::read_to_string(file(root, name).ok()?).ok()?;
            Some((name.clone(), read(name, &text, &all)))
        })
        .collect()
}

/// The page `name`, which says `text`, among the pages `all`.
fn read(name: &str, text: &str, all: &[&str]) -> Told {
    let (body, noted) = split(front(text).1);
    let mut links = BTreeSet::new();
    let mut broken = BTreeSet::new();
    for event in Parser::new_ext(body, options()) {
        let Event::Start(Tag::Link {
            link_type,
            dest_url,
            ..
        }) = event
        else {
            continue;
        };
        let Some(named) = named(name, link_type, &dest_url) else {
            continue;
        };
        if let Some(page) = resolved(&named, all) {
            links.insert(page.to_owned());
        } else {
            broken.insert(named.clone());
            links.insert(named);
        }
    }
    let used = noted
        .iter()
        .filter_map(|line| {
            let (day, what) = line.strip_prefix("- ")?.split_once(' ')?;
            Some((day.to_owned(), what.to_owned()))
        })
        .collect();
    Told {
        title: title(body),
        about: about(text),
        body: body.trim_end().to_owned(),
        links,
        broken,
        used,
        only: only(text),
    }
}

/// The name of the page a link of the page `from` names, if it is a
/// link to a page: a wiki link, or a link to a Markdown file of the wiki,
/// from the page's folder.
fn named(from: &str, link_type: LinkType, url: &str) -> Option<String> {
    let url = url.split('#').next().unwrap_or(url).trim();
    if url.is_empty() {
        return None;
    }
    let markdown = Path::new(url)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    // The extension is ".md" in some case, three bytes.
    let stem = if markdown { &url[..url.len() - 3] } else { url };
    match link_type {
        LinkType::WikiLink { .. } => Some(stem.to_owned()),
        _ if url.contains("://") || url.starts_with('/') || !markdown => None,
        _ => {
            let mut parts: Vec<&str> = from.split('/').collect();
            parts.pop();
            for part in stem.split('/') {
                match part {
                    "." | "" => {}
                    ".." => {
                        parts.pop()?;
                    }
                    part => parts.push(part),
                }
            }
            Some(parts.join("/"))
        }
    }
}

/// The page of `all` that `named` names: the page of that name, or else
/// the only one whose last part it is.
fn resolved<'a>(named: &str, all: &[&'a str]) -> Option<&'a str> {
    if let Some(page) = all.iter().find(|page| **page == named) {
        return Some(page);
    }
    let mut ending = all
        .iter()
        .filter(|page| page.rsplit('/').next() == Some(named));
    match (ending.next(), ending.next()) {
        (Some(page), None) => Some(page),
        _ => None,
    }
}

/// The pages that link to each page, by its name, the page itself left
/// out.
fn linked_from(all: &BTreeMap<String, Told>) -> BTreeMap<&str, Vec<&str>> {
    let mut from: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, page) in all {
        for link in &page.links {
            if link != name {
                from.entry(link.as_str()).or_default().push(name.as_str());
            }
        }
    }
    from
}

/// The page `name`, as the plugin reads it, without its text.
fn page_json(name: &str, page: &Told, linked_from: &BTreeMap<&str, Vec<&str>>) -> Value {
    let from = linked_from.get(name).cloned().unwrap_or_default();
    let mut marks = Vec::new();
    if !page.broken.is_empty() {
        marks.push(BROKEN);
    }
    if from.is_empty() {
        marks.push(ORPHAN);
    }
    if page.title.is_none() {
        marks.push(UNTITLED);
    }
    if page.used.is_empty() {
        marks.push(UNUSED);
    }
    json!({
        "name": name,
        "title": page.title.clone().unwrap_or_else(|| name.to_owned()),
        "about": page.about,
        "links": page.links,
        "broken": page.broken,
        "linked_from": from,
        "used": page
            .used
            .iter()
            .map(|(day, what)| json!({ "day": day, "what": what }))
            .collect::<Vec<_>>(),
        "only": page.only,
        "marks": marks,
    })
}

/// `body`, the text of the page `name` among `all`, as HTML: its links
/// to pages going to `#` and the page's name, for the plugin to open
/// them, and any HTML of its own shown as text, so that what an agent
/// wrote runs nothing on the stage.
fn html_of(name: &str, body: &str, all: &[&str]) -> String {
    let events = Parser::new_ext(body, options()).map(|event| match event {
        Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            let dest_url = match named(name, link_type, &dest_url) {
                Some(named) => {
                    let page = resolved(&named, all).map_or(named.as_str(), |page| page);
                    format!("#{page}").into()
                }
                None => dest_url,
            };
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            })
        }
        event => event,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("cox-wiki-told-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("rust")).unwrap();
        directory
    }

    #[test]
    fn the_pages_say_what_they_link_and_what_is_wrong_with_them() {
        let root = scratch("marks");
        fs::write(
            root.join("gpui.md"),
            "# GPUI\n\nSee [[taffy]], [[errors|the errors]] and [[gone]].\n\n\
             `[[not a link]]`\n\n## Used in\n\n- 2026-11-05 in cox, chat \"x\"\n",
        )
        .unwrap();
        fs::write(
            root.join("taffy.md"),
            "# Taffy\n\nBack to [GPUI](gpui.md).\n",
        )
        .unwrap();
        fs::write(root.join("rust/errors.md"), "No title.\n").unwrap();
        fs::write(root.join("index.md"), "- [[gpui]]\n- [[lonely]]\n").unwrap();
        fs::write(
            root.join("lonely.md"),
            "---\nonly: acme\n---\n\n# Lonely\n\nOf acme alone.\n",
        )
        .unwrap();

        let told = told(&root);
        let pages = told["pages"].as_array().unwrap();
        let of = |name: &str| pages.iter().find(|page| page["name"] == name).unwrap();
        let names: Vec<&str> = pages
            .iter()
            .map(|page| page["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["gpui", "lonely", "rust/errors", "taffy"]);
        assert_eq!(of("gpui")["links"], json!(["gone", "rust/errors", "taffy"]));
        assert_eq!(of("gpui")["broken"], json!(["gone"]));
        assert_eq!(of("gpui")["linked_from"], json!(["taffy"]));
        assert_eq!(of("gpui")["marks"], json!([BROKEN]));
        assert_eq!(
            of("gpui")["used"],
            json!([{ "day": "2026-11-05", "what": "in cox, chat \"x\"" }])
        );
        // The index links to no page as far as the marks go.
        assert_eq!(of("lonely")["marks"], json!([ORPHAN, UNUSED]));
        assert_eq!(of("lonely")["title"], "Lonely");
        assert_eq!(of("lonely")["about"], "Of acme alone.");
        assert_eq!(of("lonely")["only"], "acme");
        assert_eq!(of("gpui")["only"], Value::Null);
        assert_eq!(of("rust/errors")["title"], "rust/errors");
        assert_eq!(of("rust/errors")["marks"], json!([UNTITLED, UNUSED]));
        assert_eq!(of("rust/errors")["linked_from"], json!(["gpui"]));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_page_is_read_in_the_humans_language_with_its_links_as_they_were() {
        let root = scratch("translated");
        fs::write(root.join("gpui.md"), "# GPUI\n\nSee [[errors]].\n").unwrap();
        fs::write(root.join("rust/errors.md"), "# Errors\n").unwrap();
        let page = page_in(&root, "gpui", |body| {
            assert_eq!(body, "# GPUI\n\nSee [[errors]].");
            Some(Translated {
                language: "Japanese".to_owned(),
                markdown: "# GPUI\n\n[[errors]] を見る。".to_owned(),
                done: 1,
                of: 2,
                error: None,
            })
        })
        .unwrap();
        let translation = &page["translation"];
        assert_eq!(translation["language"], "Japanese");
        assert_eq!(
            translation["html"],
            "<h1>GPUI</h1>\n<p><a href=\"#rust/errors\">errors</a> を見る。</p>\n"
        );
        assert_eq!(
            (translation["done"].clone(), translation["of"].clone()),
            (json!(1), json!(2))
        );
        assert_eq!(translation["error"], Value::Null);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_page_is_html_whose_links_open_pages_and_whose_own_html_is_text() {
        let root = scratch("html");
        fs::write(
            root.join("gpui.md"),
            "---\ntags: [ui]\n---\n# GPUI\n\n[[errors|Errors]], [Taffy](taffy.md), [web](https://x.dev) \
             <script>alert(1)</script>\n\n## Used in\n\n- 2026-11-05 in cox\n",
        )
        .unwrap();
        fs::write(root.join("rust/errors.md"), "# Errors\n").unwrap();
        let page = page(&root, "gpui").unwrap();
        let html = page["html"].as_str().unwrap();
        assert!(
            html.contains("<a href=\"#rust/errors\">Errors</a>"),
            "{html}"
        );
        assert!(html.contains("<a href=\"#taffy\">Taffy</a>"), "{html}");
        assert!(html.contains("<a href=\"https://x.dev\">web</a>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
        assert!(!html.contains("Used in"), "{html}");
        assert!(html.starts_with("<h1>GPUI</h1>"), "{html}");
        assert_eq!(page["file"], root.join("gpui.md").display().to_string());
        assert_eq!(page["translation"], Value::Null);
        assert_eq!(super::page(&root, "nothing"), None);
        let _ = fs::remove_dir_all(&root);
    }
}
