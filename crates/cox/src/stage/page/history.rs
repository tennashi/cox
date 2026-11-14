//! The pages the stage visited, as a browser's history: each page's URL,
//! title, how many times it was visited and when last, for the address
//! line to offer as the human types (see [`History::offered`]) and to
//! fill in (see [`History::fill`]).
//!
//! One history serves every stage of the window, read from Cox's state
//! the first time it is wanted and written back a moment after it
//! changes (see [`visited`]).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext as _, Global, Task};
use serde::{Deserialize, Serialize};

use crate::storage;

/// The most pages the history keeps: past it, those visited least lately
/// go.
pub const MOST: usize = 2000;

/// How long after the history changes it is written, so that pages
/// loaded one after another are written once.
const WRITE_AFTER: Duration = Duration::from_secs(2);

const DAY: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visited {
    pub url: String,
    /// The page's title, empty until it had one.
    pub title: String,
    pub visits: u32,
    /// When it was last visited, in seconds since 1970.
    pub last: u64,
}

/// How well a page fits what is typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Fit {
    /// Each word typed starts a word of its title or URL.
    Words,
    /// Its URL starts with what is typed, with or without its scheme and
    /// `www.`.
    Start,
}

/// What the address line fills in after what is typed: the rest of a
/// page's URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fill {
    /// What goes after what is typed.
    pub rest: String,
    /// The page's URL, which Enter opens.
    pub url: String,
}

/// The pages visited, in no order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct History {
    pages: Vec<Visited>,
}

impl History {
    /// The history kept at `file`: empty when there is none, or it cannot
    /// be read.
    #[must_use]
    pub fn read(file: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(file) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::warn!("cannot read the history at {}: {error}", file.display());
            Self::default()
        })
    }

    /// Keeps the history at `file`.
    ///
    /// # Errors
    ///
    /// When it cannot be written.
    pub fn write(&self, file: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        storage::write_whole(file, text)
    }

    #[must_use]
    pub fn pages(&self) -> &[Visited] {
        &self.pages
    }

    /// The page at `url`, titled `title`, was visited at `now`, in seconds
    /// since 1970. Only pages of the web, over `http:` or `https:`, are
    /// kept. Answers whether the history changed.
    pub fn visit(&mut self, url: &str, title: &str, now: u64) -> bool {
        if !of_the_web(url) {
            return false;
        }
        match self.pages.iter_mut().find(|page| page.url == url) {
            Some(page) => {
                page.visits = page.visits.saturating_add(1);
                page.last = now;
                if !title.is_empty() {
                    title.clone_into(&mut page.title);
                }
            }
            None => self.pages.push(Visited {
                url: url.to_owned(),
                title: title.to_owned(),
                visits: 1,
                last: now,
            }),
        }
        while self.pages.len() > MOST {
            let oldest = self
                .pages
                .iter()
                .enumerate()
                .min_by_key(|(_, page)| page.last)
                .map(|(place, _)| place);
            if let Some(oldest) = oldest {
                self.pages.swap_remove(oldest);
            }
        }
        true
    }

    /// The page at `url`, if visited, is titled `title` now. Answers
    /// whether the history changed.
    pub fn title(&mut self, url: &str, title: &str) -> bool {
        match self.pages.iter_mut().find(|page| page.url == url) {
            Some(page) if !title.is_empty() && page.title != title => {
                title.clone_into(&mut page.title);
                true
            }
            _ => false,
        }
    }

    /// Up to `most` pages that fit `typed`, the best first: those whose
    /// URL starts with it (see [`rest_of`]), then those with a word of the
    /// title or URL starting with each word of it; among them, those
    /// visited more and lately first (see [`frecency`]), then the shorter.
    #[must_use]
    pub fn offered(&self, typed: &str, now: u64, most: usize) -> Vec<&Visited> {
        let typed = typed.trim();
        if typed.is_empty() {
            return Vec::new();
        }
        let mut fitting: Vec<(Fit, u64, &Visited)> = self
            .pages
            .iter()
            .filter_map(|page| Some((fit(page, typed)?, frecency(page, now), page)))
            .collect();
        fitting.sort_by(|(fit, score, page), (other_fit, other_score, other)| {
            other_fit
                .cmp(fit)
                .then(other_score.cmp(score))
                .then(page.url.len().cmp(&other.url.len()))
                .then(page.url.cmp(&other.url))
        });
        fitting
            .into_iter()
            .take(most)
            .map(|(_, _, page)| page)
            .collect()
    }

    /// What to fill in after `typed`, typed at the end of the address
    /// line: the rest of the URL of the best page offered for it, if its
    /// URL starts with it. Words, or a line starting with `?`, which is
    /// searched for, get none.
    #[must_use]
    pub fn fill(&self, typed: &str, now: u64) -> Option<Fill> {
        if typed.is_empty() || typed.starts_with('?') || typed.contains(char::is_whitespace) {
            return None;
        }
        let best = self.offered(typed, now, 1).into_iter().next()?;
        let rest = rest_of(&best.url, typed)?;
        (!rest.is_empty()).then(|| Fill {
            rest: rest.to_owned(),
            url: best.url.clone(),
        })
    }
}

/// Whether `url` is of a page of the web, which the history keeps: not
/// Cox's own (`cox:`), a file, nor a page made of its address
/// (`about:`, `data:`) or the browser's.
fn of_the_web(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// What of `url` comes after `typed`, if `url` starts with it, ignoring
/// case, as written whole, without its scheme, or without `www.` too.
fn rest_of<'u>(url: &'u str, typed: &str) -> Option<&'u str> {
    let bare = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = bare.strip_prefix("www.").unwrap_or(bare);
    [url, bare, host].into_iter().find_map(|form| {
        form.get(..typed.len())
            .filter(|start| start.eq_ignore_ascii_case(typed))
            .map(|_| &form[typed.len()..])
    })
}

/// How well `page` fits `typed`, if it does.
fn fit(page: &Visited, typed: &str) -> Option<Fit> {
    if rest_of(&page.url, typed).is_some() {
        return Some(Fit::Start);
    }
    let words: Vec<String> = page
        .title
        .split(|c: char| !c.is_alphanumeric())
        .chain(page.url.split(|c: char| !c.is_alphanumeric()))
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    typed
        .to_lowercase()
        .split_whitespace()
        .all(|typed| words.iter().any(|word| word.starts_with(typed)))
        .then_some(Fit::Words)
}

/// How often and how lately `page` was visited, at `now`: its visits,
/// each worth less the longer ago the last was.
fn frecency(page: &Visited, now: u64) -> u64 {
    let days = now.saturating_sub(page.last) / DAY;
    let worth = match days {
        0 => 100,
        1..=6 => 70,
        7..=29 => 50,
        30..=89 => 30,
        _ => 10,
    };
    u64::from(page.visits) * worth
}

#[must_use]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The window's history, and where it is kept.
struct Shared {
    history: History,
    file: Option<PathBuf>,
    writing: Option<Task<()>>,
}

impl Global for Shared {}

/// The window's history, read from Cox's state the first time.
fn shared(cx: &mut App) -> &mut Shared {
    if !cx.has_global::<Shared>() {
        let file = storage::path(storage::Kept::History);
        let history = file.as_deref().map(History::read).unwrap_or_default();
        cx.set_global(Shared {
            history,
            file,
            writing: None,
        });
    }
    cx.global_mut::<Shared>()
}

/// The pages the window's stages visited.
pub fn history(cx: &mut App) -> &History {
    &shared(cx).history
}

/// Makes `history` the window's, kept at `file`, or nowhere with `None`,
/// in place of the one in Cox's state.
pub fn keep(history: History, file: Option<PathBuf>, cx: &mut App) {
    cx.set_global(Shared {
        history,
        file,
        writing: None,
    });
}

/// A stage loaded the page at `url`, titled `title`.
pub fn visited(url: &str, title: &str, cx: &mut App) {
    if shared(cx).history.visit(url, title, now()) {
        write_soon(cx);
    }
}

pub fn titled(url: &str, title: &str, cx: &mut App) {
    if shared(cx).history.title(url, title) {
        write_soon(cx);
    }
}

/// Writes the history in a moment, unless it changes again before then.
fn write_soon(cx: &mut App) {
    if shared(cx).file.is_none() {
        return;
    }
    let writing = cx.spawn(async move |cx| {
        cx.background_executor().timer(WRITE_AFTER).await;
        let Ok(Some((history, file))) = cx.update(|cx| {
            let shared = cx.global::<Shared>();
            Some((shared.history.clone(), shared.file.clone()?))
        }) else {
            return;
        };
        let written = cx
            .background_spawn(async move { history.write(&file).map_err(|error| (file, error)) })
            .await;
        if let Err((file, error)) = written {
            tracing::warn!("cannot write the history at {}: {error}", file.display());
        }
    });
    shared(cx).writing = Some(writing);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    const NOW: u64 = 1_800_000_000;

    fn urls(pages: &[&Visited]) -> Vec<String> {
        pages.iter().map(|page| page.url.clone()).collect()
    }

    #[test]
    fn a_visit_counts_and_keeps_the_last_time_and_title() {
        let mut history = History::default();
        assert!(history.visit("https://example.com/", "Example", NOW - DAY));
        assert!(history.visit("https://example.com/", "", NOW));
        assert_eq!(
            history.pages(),
            [Visited {
                url: "https://example.com/".to_owned(),
                title: "Example".to_owned(),
                visits: 2,
                last: NOW,
            }]
        );
        assert!(history.title("https://example.com/", "Example Domain"));
        assert!(!history.title("https://example.com/", "Example Domain"));
        assert!(!history.title("https://elsewhere.com/", "Elsewhere"));
        assert_eq!(history.pages()[0].title, "Example Domain");
    }

    #[test]
    fn only_pages_of_the_web_are_kept() {
        let mut history = History::default();
        for url in [
            "cox://plugin/x",
            "about:blank",
            "data:text/html,<p>hi",
            "file:///Users/me/a.html",
            "chrome://version",
            "not a url",
        ] {
            assert!(!history.visit(url, "", NOW), "{url}");
        }
        assert!(history.visit("http://localhost:3000/", "", NOW));
        assert!(history.visit("https://example.com/", "", NOW));
        assert_eq!(history.pages().len(), 2);
    }

    #[test]
    fn past_the_most_the_page_visited_least_lately_goes() {
        let mut history = History::default();
        for n in 0..MOST {
            history.visit(&format!("https://example.com/{n}"), "", NOW + n as u64);
        }
        history.visit("https://example.com/0", "", NOW + MOST as u64);
        history.visit("https://new.example.com/", "", NOW + MOST as u64 + 1);
        assert_eq!(history.pages().len(), MOST);
        let kept = |url: &str| history.pages().iter().any(|page| page.url == url);
        assert!(kept("https://example.com/0"), "visited again lately");
        assert!(!kept("https://example.com/1"), "visited least lately");
        assert!(kept("https://new.example.com/"));
    }

    #[test]
    fn pages_whose_address_starts_so_come_first_then_the_visited_more_and_lately() {
        let mut history = History::default();
        history.visit("https://www.github.com/rust-lang/rust", "Rust", NOW);
        for _ in 0..5 {
            history.visit(
                "https://github.com/zed-industries/zed",
                "Zed",
                NOW - 60 * DAY,
            );
        }
        history.visit("https://github.com/", "GitHub", NOW);
        history.visit("https://github.com/", "GitHub", NOW);
        history.visit("https://docs.rs/gpui", "gpui - Rust GitHub docs", NOW);
        history.visit("https://example.com/", "Example", NOW);

        assert_eq!(
            urls(&history.offered("git", NOW, 6)),
            [
                "https://github.com/",
                "https://github.com/zed-industries/zed",
                "https://www.github.com/rust-lang/rust",
                "https://docs.rs/gpui",
            ]
        );
        assert_eq!(
            urls(&history.offered("rust docs", NOW, 6)),
            ["https://docs.rs/gpui"]
        );
        assert_eq!(
            urls(&history.offered("https://GITHUB.com/z", NOW, 6)),
            ["https://github.com/zed-industries/zed"]
        );
        assert_eq!(urls(&history.offered("git", NOW, 1)).len(), 1);
        assert_eq!(urls(&history.offered("  ", NOW, 6)), Vec::<String>::new());
        assert_eq!(
            urls(&history.offered("nothing", NOW, 6)),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_rest_of_the_best_address_that_starts_so_is_filled_in() {
        let mut history = History::default();
        history.visit("https://www.github.com/", "GitHub", NOW);
        history.visit("https://docs.rs/gpui", "gpui GitHub", NOW);
        history.visit("https://docs.rs/gpui", "gpui GitHub", NOW);
        let fill = |typed: &str| history.fill(typed, NOW);
        let github = |rest: &str| {
            Some(Fill {
                rest: rest.to_owned(),
                url: "https://www.github.com/".to_owned(),
            })
        };
        assert_eq!(fill("gi"), github("thub.com/"));
        assert_eq!(fill("Gi"), github("thub.com/"));
        assert_eq!(fill("www.g"), github("ithub.com/"));
        assert_eq!(fill("https://www.github.c"), github("om/"));
        assert_eq!(
            fill("do"),
            Some(Fill {
                rest: "cs.rs/gpui".to_owned(),
                url: "https://docs.rs/gpui".to_owned(),
            })
        );
        assert_eq!(fill("github"), github(".com/"));
        assert_eq!(fill("https://www.github.com/"), None, "nothing is left");
        assert_eq!(fill("gpui"), None, "only a word of a title fits");
        assert_eq!(fill("gi hub"), None);
        assert_eq!(fill("?gi"), None);
        assert_eq!(fill(""), None);
    }

    #[test]
    fn the_history_is_written_and_read_back() {
        let folder = TempDir::new();
        let file = folder.join("history.json");
        assert_eq!(History::read(&file), History::default());

        let mut history = History::default();
        history.visit("https://example.com/", "Example", NOW);
        history.visit("https://docs.rs/", "Docs.rs", NOW + 1);
        history.write(&file).unwrap();
        assert_eq!(History::read(&file), history);

        std::fs::write(&file, "not json").unwrap();
        assert_eq!(History::read(&file), History::default());
    }
}
