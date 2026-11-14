//! Files the human points the agent to in a line: `@` and the file's path
//! from the chat's folder, at the start of a word, as `@src/main.rs`, or
//! `@"docs/a file.md"` for a path with a space. The line goes to the
//! agent as it is written, and each file it points to goes with it.

use std::ops::Range;
use std::path::{Path, PathBuf};

/// What may end a sentence after a path, as `@src/main.rs,` does, and is
/// not taken for part of the path unless the file's name has it.
const AFTER: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}', '\'', '"'];

/// The path being written after `@` where the cursor is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typing<'l> {
    /// Where the `@` and the path are in the line, which a path chosen
    /// takes the place of.
    pub range: Range<usize>,
    /// What of the path is written before the cursor.
    pub query: &'l str,
}

/// The path being written after an `@` at the start of a word, ending at
/// `cursor`, a byte offset of `line`, if one is.
#[must_use]
pub fn typing(line: &str, cursor: usize) -> Option<Typing<'_>> {
    let before = line.get(..cursor)?;
    let at = before
        .char_indices()
        .rev()
        .filter(|(_, letter)| *letter == '@')
        .map(|(at, _)| at)
        .find(|at| starts_word(line, *at))?;
    let written = &before[at + 1..];
    if let Some(quoted) = written.strip_prefix('"') {
        return (!quoted.contains('"')).then_some(Typing {
            range: at..cursor,
            query: quoted,
        });
    }
    if written.contains(char::is_whitespace) {
        return None;
    }
    // The rest of the word goes too, when the cursor is inside it.
    let end = line[cursor..]
        .find(char::is_whitespace)
        .map_or(line.len(), |gap| cursor + gap);
    Some(Typing {
        range: at..end,
        query: written,
    })
}

/// How the path `path` is written to point to it.
#[must_use]
pub fn written(path: &str) -> String {
    if path.contains(char::is_whitespace) {
        format!("@\"{path}\"")
    } else {
        format!("@{path}")
    }
}

/// The paths `line` points to, as written, in their order.
#[must_use]
pub fn mentioned(line: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    let mut rest = 0;
    while let Some(found) = line[rest..].find('@') {
        let at = rest + found;
        rest = at + 1;
        if !starts_word(line, at) {
            continue;
        }
        let after = &line[at + 1..];
        let path = if let Some(quoted) = after.strip_prefix('"') {
            let Some(end) = quoted.find('"') else {
                continue;
            };
            rest = at + 2 + end + 1;
            &quoted[..end]
        } else {
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            rest = at + 1 + end;
            &after[..end]
        };
        if !path.is_empty() {
            paths.push(path);
        }
    }
    paths
}

/// The files `line` points to, in `folder`, each once: those that are
/// there.
#[must_use]
pub fn files(line: &str, folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    for written in mentioned(line) {
        let found = [written, written.trim_end_matches(AFTER)]
            .into_iter()
            .filter(|path| !path.is_empty())
            .map(|path| folder.join(path))
            .find(|path| path.is_file());
        if let Some(file) = found
            && !files.contains(&file)
        {
            files.push(file);
        }
    }
    files
}

/// Whether the `@` at `at` starts a word of `line`.
fn starts_word(line: &str, at: usize) -> bool {
    line[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_path_typed_is_the_word_after_the_at() {
        let line = "look at @src/ma please";
        let cursor = "look at @src/ma".len();
        assert_eq!(
            typing(line, cursor),
            Some(Typing {
                range: 8..cursor,
                query: "src/ma"
            })
        );
        assert_eq!(typing("@", 1).map(|typed| typed.query), Some(""));
        // Inside the word, the rest of it goes too.
        assert_eq!(
            typing("@src/main.rs", 4).map(|typed| typed.range),
            Some(0..12)
        );
    }

    #[test]
    fn a_path_is_typed_only_after_an_at_that_starts_a_word() {
        assert_eq!(typing("me@example", 10), None);
        assert_eq!(typing("@src/main.rs and", 16), None);
        assert_eq!(typing("plain", 5), None);
    }

    #[test]
    fn a_quoted_path_has_spaces_until_it_is_closed() {
        let typed = typing("@\"docs/a fi", 11).unwrap();
        assert_eq!(typed.query, "docs/a fi");
        assert_eq!(typing("@\"docs/a file.md\" ", 18), None);
    }

    #[test]
    fn a_path_with_a_space_is_written_quoted() {
        assert_eq!(written("src/main.rs"), "@src/main.rs");
        assert_eq!(written("docs/a file.md"), "@\"docs/a file.md\"");
    }

    #[test]
    fn the_line_points_to_each_path_after_an_at() {
        assert_eq!(
            mentioned("see @src/main.rs, @\"docs/a file.md\" and me@example.com @"),
            ["src/main.rs,", "docs/a file.md"]
        );
    }

    #[test]
    fn the_files_are_those_in_the_folder() {
        let folder = TempDir::new();
        std::fs::create_dir_all(folder.join("src")).unwrap();
        std::fs::write(folder.join("src/main.rs"), "").unwrap();
        let files = files(
            "read @src/main.rs, then @src/main.rs again, and @src/gone.rs, @src",
            &folder,
        );
        assert_eq!(files, [folder.join("src/main.rs")]);
    }
}
