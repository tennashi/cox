//! What the human types at the top of the menu of the stage's +, a URL, a
//! file, a command or words to search for, and which of them it is: a
//! file that is there, then an address of the web, then a path, then a
//! command for a terminal, or, where Vim's keys opened the menu, words to
//! search for, unless they start with `!`, which runs the rest.

use std::path::{Path, PathBuf};

use super::page::address_to_url;
use crate::settings::SearchEngine;

/// What a line typed in the menu opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Typed {
    /// A page, at this URL.
    Page(String),
    /// A file, at this absolute path.
    File(PathBuf),
    /// A command, which runs in a terminal.
    Command(String),
    /// Words to search for, with the engine the settings name.
    Search(String),
}

/// What a line that is no file, address or path opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unmatched {
    /// A search for its words, as a browser's address line does; a line
    /// that starts with `!` runs the rest as a command.
    Search,
    /// A command in a terminal, as the menu of the stage's + runs it.
    Command,
}

impl Typed {
    /// What Enter does with it, for the menu to say, searching with
    /// `engine`.
    pub(super) fn said(&self, engine: &SearchEngine) -> String {
        match self {
            Self::Page(url) => format!("Open {url}"),
            Self::File(path) => format!("Open the file {}", path.display()),
            Self::Command(command) => format!("Run {command} in a terminal"),
            Self::Search(words) => format!("Search {} for {words}", engine.host()),
        }
    }
}

/// What `line` opens, for a chat working in `cwd`, which a relative path
/// is from: a file, when there is one there or the line starts as a path
/// does, or else an address, when it has no space and what comes before
/// its first `/` has a scheme, or a dot or a port, as a host has, or else
/// a file again, when it has no space and a `/`, as `src/main.rs`, or
/// else what `unmatched` says. Where it says search, a line that starts
/// with `!` runs the rest as a command, whatever else it is. Blank is
/// nothing.
pub(super) fn typed(line: &str, cwd: &Path, unmatched: Unmatched) -> Option<Typed> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if unmatched == Unmatched::Search
        && let Some(command) = line.strip_prefix('!')
    {
        let command = command.trim();
        return (!command.is_empty()).then(|| Typed::Command(command.to_owned()));
    }
    let otherwise = || match unmatched {
        Unmatched::Search => Typed::Search(line.to_owned()),
        Unmatched::Command => Typed::Command(line.to_owned()),
    };
    let path = match line.strip_prefix("~/") {
        Some(rest) => {
            std::env::home_dir().map_or_else(|| PathBuf::from(line), |home| home.join(rest))
        }
        None => cwd.join(line),
    };
    let a_path = ["/", "./", "../", "~/"]
        .iter()
        .any(|start| line.starts_with(start));
    if a_path || path.is_file() {
        return Some(Typed::File(path));
    }
    if line.contains(char::is_whitespace) {
        return Some(otherwise());
    }
    // What would be the host, as `example.com` of `example.com/a`, or the
    // scheme, as `https:` of `https://example.com`.
    let host = line.split('/').next().unwrap_or(line);
    let address = !host.starts_with('.') && (host.contains(['.', ':']) || host == "localhost");
    if address {
        return address_to_url(line).map(Typed::Page);
    }
    if line.contains('/') {
        return Some(Typed::File(path));
    }
    Some(otherwise())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> PathBuf {
        let folder = std::env::temp_dir().join(format!("cox-typed-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("src")).unwrap();
        std::fs::write(folder.join("src/main.rs"), "fn main() {}\n").unwrap();
        folder
    }

    #[test]
    fn a_file_that_is_there_opens_as_a_file() {
        let folder = folder();
        assert_eq!(
            typed("src/main.rs", &folder, Unmatched::Command),
            Some(Typed::File(folder.join("src/main.rs")))
        );
        let absolute = folder.join("src/main.rs").display().to_string();
        assert_eq!(
            typed(&absolute, Path::new("/"), Unmatched::Command),
            Some(Typed::File(folder.join("src/main.rs")))
        );
    }

    #[test]
    fn an_address_opens_as_a_page() {
        let folder = folder();
        assert_eq!(
            typed("example.com", &folder, Unmatched::Command),
            Some(Typed::Page("https://example.com".to_owned()))
        );
        assert_eq!(
            typed("localhost:3000", &folder, Unmatched::Command),
            Some(Typed::Page("http://localhost:3000".to_owned()))
        );
        assert_eq!(
            typed("about:blank", &folder, Unmatched::Command),
            Some(Typed::Page("about:blank".to_owned()))
        );
        assert_eq!(
            typed("example.com/a/b.html", &folder, Unmatched::Command),
            Some(Typed::Page("https://example.com/a/b.html".to_owned()))
        );
        assert_eq!(
            typed("https://example.com/a", &folder, Unmatched::Command),
            Some(Typed::Page("https://example.com/a".to_owned()))
        );
    }

    #[test]
    fn a_path_that_is_not_there_opens_as_a_file() {
        let folder = folder();
        assert_eq!(
            typed("./src/gone.rs", &folder, Unmatched::Command),
            Some(Typed::File(folder.join("./src/gone.rs")))
        );
        assert_eq!(
            typed("src/gone.rs", &folder, Unmatched::Command),
            Some(Typed::File(folder.join("src/gone.rs")))
        );
        assert_eq!(
            typed(".cox/maps/map.json", &folder, Unmatched::Command),
            Some(Typed::File(folder.join(".cox/maps/map.json")))
        );
    }

    #[test]
    fn the_rest_runs_as_a_command() {
        let folder = folder();
        assert_eq!(
            typed("cargo test", &folder, Unmatched::Command),
            Some(Typed::Command("cargo test".to_owned()))
        );
        assert_eq!(
            typed("ls", &folder, Unmatched::Command),
            Some(Typed::Command("ls".to_owned()))
        );
        assert_eq!(
            typed("cat src/main.rs", &folder, Unmatched::Command),
            Some(Typed::Command("cat src/main.rs".to_owned()))
        );
        assert_eq!(typed("   ", &folder, Unmatched::Command), None);
    }

    #[test]
    fn where_vim_opened_the_menu_the_rest_is_searched_for() {
        let folder = folder();
        let search = |line| typed(line, &folder, Unmatched::Search);
        assert_eq!(
            search("rust gpui"),
            Some(Typed::Search("rust gpui".to_owned()))
        );
        assert_eq!(search("ls"), Some(Typed::Search("ls".to_owned())));
        assert_eq!(
            search("検索 する"),
            Some(Typed::Search("検索 する".to_owned()))
        );
        assert_eq!(
            search("!echo hi"),
            Some(Typed::Command("echo hi".to_owned()))
        );
        assert_eq!(
            search("!cat src/main.rs"),
            Some(Typed::Command("cat src/main.rs".to_owned()))
        );
        assert_eq!(search("! "), None);
        assert_eq!(
            search("src/main.rs"),
            Some(Typed::File(folder.join("src/main.rs")))
        );
        assert_eq!(
            search("example.com"),
            Some(Typed::Page("https://example.com".to_owned()))
        );
        assert_eq!(
            search("src/gone.rs"),
            Some(Typed::File(folder.join("src/gone.rs")))
        );
        assert_eq!(search("  "), None);
    }

    #[test]
    fn the_menu_says_what_enter_does() {
        let engine = SearchEngine::default();
        assert_eq!(
            Typed::Search("rust gpui".to_owned()).said(&engine),
            "Search google.com for rust gpui"
        );
        assert_eq!(
            Typed::Command("echo hi".to_owned()).said(&engine),
            "Run echo hi in a terminal"
        );
    }
}
