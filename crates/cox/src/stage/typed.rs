//! What the human types at the top of the menu of the stage's +, a URL, a
//! file or a command, and which of them it is: a file that is there, then
//! an address of the web, then a path, then a command for a terminal.

use std::path::{Path, PathBuf};

use super::page::address_to_url;

/// What a line typed in the menu opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Typed {
    /// A page, at this URL.
    Page(String),
    /// A file, at this absolute path.
    File(PathBuf),
    /// A command, which runs in a terminal.
    Command(String),
}

/// What `line` opens, for a chat working in `cwd`, which a relative path
/// is from: a file, when there is one there or the line starts as a path
/// does, or else an address, when it has no space and what comes before
/// its first `/` has a scheme, or a dot or a port, as a host has, or else
/// a file again, when it has no space and a `/`, as `src/main.rs`, or
/// else a command. Blank is nothing.
pub(super) fn typed(line: &str, cwd: &Path) -> Option<Typed> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
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
        return Some(Typed::Command(line.to_owned()));
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
    Some(Typed::Command(line.to_owned()))
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
            typed("src/main.rs", &folder),
            Some(Typed::File(folder.join("src/main.rs")))
        );
        let absolute = folder.join("src/main.rs").display().to_string();
        assert_eq!(
            typed(&absolute, Path::new("/")),
            Some(Typed::File(folder.join("src/main.rs")))
        );
    }

    #[test]
    fn an_address_opens_as_a_page() {
        let folder = folder();
        assert_eq!(
            typed("example.com", &folder),
            Some(Typed::Page("https://example.com".to_owned()))
        );
        assert_eq!(
            typed("localhost:3000", &folder),
            Some(Typed::Page("https://localhost:3000".to_owned()))
        );
        assert_eq!(
            typed("about:blank", &folder),
            Some(Typed::Page("about:blank".to_owned()))
        );
        assert_eq!(
            typed("example.com/a/b.html", &folder),
            Some(Typed::Page("https://example.com/a/b.html".to_owned()))
        );
        assert_eq!(
            typed("https://example.com/a", &folder),
            Some(Typed::Page("https://example.com/a".to_owned()))
        );
    }

    #[test]
    fn a_path_that_is_not_there_opens_as_a_file() {
        let folder = folder();
        assert_eq!(
            typed("./src/gone.rs", &folder),
            Some(Typed::File(folder.join("./src/gone.rs")))
        );
        assert_eq!(
            typed("src/gone.rs", &folder),
            Some(Typed::File(folder.join("src/gone.rs")))
        );
        assert_eq!(
            typed(".cox/maps/map.json", &folder),
            Some(Typed::File(folder.join(".cox/maps/map.json")))
        );
    }

    #[test]
    fn the_rest_runs_as_a_command() {
        let folder = folder();
        assert_eq!(
            typed("cargo test", &folder),
            Some(Typed::Command("cargo test".to_owned()))
        );
        assert_eq!(typed("ls", &folder), Some(Typed::Command("ls".to_owned())));
        assert_eq!(
            typed("cat src/main.rs", &folder),
            Some(Typed::Command("cat src/main.rs".to_owned()))
        );
        assert_eq!(typed("   ", &folder), None);
    }
}
