//! What the human types at the top of the menu of the stage's +, a URL, a
//! file, a command or words to search for, and which of them it is: a
//! file that is there, then an address of the web as the page's address
//! line sorts it (see [`Address`]), then a path, then a command for a
//! terminal, or, where Vim's keys opened the menu, words to search for,
//! unless they start with `!`, which runs the rest.

use std::path::{Path, PathBuf};

use super::page::address::Address;
use crate::settings::SearchEngine;
use crate::ui::paths::expanded;

/// What a line typed in the menu opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Typed {
    /// A page, or words to search for with the engine the settings name.
    Web(Address),
    /// A file, at this absolute path.
    File(PathBuf),
    /// A command, which runs in a terminal.
    Command(String),
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
            Self::Web(address) => address.said(engine),
            Self::File(path) => format!("Open the file {}", path.display()),
            Self::Command(command) => format!("Run {command} in a terminal"),
        }
    }
}

/// What `line` opens, for a chat working in `cwd`, which a relative path
/// is from: a file, when there is one there or the line starts as a path
/// does, or else a URL, when [`Address`] sorts it as one, or else a file
/// again, when it has no space and a `/`, as `src/main.rs`, or else what
/// `unmatched` says, which searches the words [`Address`] found. Where it
/// says search, a line that starts with `!` runs the rest as a command,
/// whatever else it is. Blank is nothing.
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
    let path = expanded(line).map_or_else(|| PathBuf::from(line), |path| cwd.join(path));
    let a_path = ["/", "./", "../", "~/"]
        .iter()
        .any(|start| line.starts_with(start));
    if a_path || path.is_file() {
        return Some(Typed::File(path));
    }
    Some(match Address::of(line)? {
        Address::Search(words)
            if words == line && line.contains('/') && !line.contains(char::is_whitespace) =>
        {
            Typed::File(path)
        }
        Address::Search(words) => match unmatched {
            Unmatched::Search => Typed::Web(Address::Search(words)),
            Unmatched::Command => Typed::Command(line.to_owned()),
        },
        url @ Address::Url(_) => Typed::Web(url),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn url(url: &str) -> Typed {
        Typed::Web(Address::Url(url.to_owned()))
    }

    fn words(words: &str) -> Typed {
        Typed::Web(Address::Search(words.to_owned()))
    }

    fn folder() -> TempDir {
        let folder = TempDir::new();
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
            Some(url("https://example.com"))
        );
        assert_eq!(
            typed("localhost:3000", &folder, Unmatched::Command),
            Some(url("http://localhost:3000"))
        );
        assert_eq!(
            typed("about:blank", &folder, Unmatched::Command),
            Some(url("about:blank"))
        );
        assert_eq!(
            typed("example.com/a/b.html", &folder, Unmatched::Command),
            Some(url("https://example.com/a/b.html"))
        );
        assert_eq!(
            typed("https://example.com/a", &folder, Unmatched::Command),
            Some(url("https://example.com/a"))
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
        assert_eq!(search("rust gpui"), Some(words("rust gpui")));
        assert_eq!(search("ls"), Some(words("ls")));
        assert_eq!(search("検索 する"), Some(words("検索 する")));
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
        assert_eq!(search("example.com"), Some(url("https://example.com")));
        assert_eq!(
            search("src/gone.rs"),
            Some(Typed::File(folder.join("src/gone.rs")))
        );
        assert_eq!(search("  "), None);
        assert_eq!(search("gpui"), Some(words("gpui")));
        assert_eq!(search("node.js"), Some(url("https://node.js")));
        assert_eq!(search("?example.com"), Some(words("example.com")));
        assert_eq!(search("1.5"), Some(words("1.5")));
    }

    #[test]
    fn where_the_mouse_opened_the_menu_a_word_runs_as_a_command() {
        let folder = folder();
        let command = |line| typed(line, &folder, Unmatched::Command);
        assert_eq!(command("gpui"), Some(Typed::Command("gpui".to_owned())));
        assert_eq!(
            command("foo:bar"),
            Some(Typed::Command("foo:bar".to_owned()))
        );
        assert_eq!(command("node.js"), Some(url("https://node.js")));
    }

    #[test]
    fn the_menu_says_what_enter_does() {
        let engine = SearchEngine::default();
        assert_eq!(
            words("rust gpui").said(&engine),
            "Search google.com for rust gpui"
        );
        assert_eq!(
            Typed::Command("echo hi".to_owned()).said(&engine),
            "Run echo hi in a terminal"
        );
    }
}
