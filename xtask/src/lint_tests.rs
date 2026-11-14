//! `cargo xtask lint-tests`: keeps the tests from what makes them fail
//! now and then. A test waits on an event, not on a clock, and keeps its
//! files in a folder of its own, not in the system's shared one. Where a
//! test has a reason to, a comment beside it says why, as
//! `// allow-sleep: <why>` or `// allow-temp-dir: <why>`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::bail;

/// The files that make waiting for the tests, which sleep between looks,
/// and this one, whose tests hold the rules' examples.
const SLEEP_HOMES: &[&str] = &[
    "xtask/src/lint_tests.rs",
    "crates/coxd/src/testing/waits.rs",
    "crates/cox/tests/fake_daemon/mod.rs",
];

/// The files that make each test's own folders in the system's, and this
/// one.
const TEMP_HOMES: &[&str] = &[
    "xtask/src/lint_tests.rs",
    "crates/coxd/src/testing.rs",
    "crates/coxd/src/testing/scratch.rs",
    "crates/cox-android/src/scratch.rs",
    "crates/cox-editor/src/scratch.rs",
    "crates/cox-git/src/scratch.rs",
];

/// What a test must not do without saying why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Sleeps, standing in for an event it could wait on.
    Sleep,
    /// Uses the system's temporary folder, which every test shares.
    TempDir,
}

impl Rule {
    const fn marker(self) -> &'static str {
        match self {
            Self::Sleep => "allow-sleep:",
            Self::TempDir => "allow-temp-dir:",
        }
    }

    const fn says(self) -> &'static str {
        match self {
            Self::Sleep => {
                "a test sleeps: wait on an event or a condition instead, or say why with `// allow-sleep: <why>`"
            }
            Self::TempDir => {
                "a test uses the shared temporary folder: use the test's own folder instead, or say why with `// allow-temp-dir: <why>`"
            }
        }
    }

    fn breaks(self, line: &str) -> bool {
        match self {
            Self::Sleep => line.contains("thread::sleep(") || line.contains("time::sleep("),
            Self::TempDir => line.match_indices("temp_dir()").any(|(at, _)| {
                !line[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
            }),
        }
    }

    fn homes(self) -> &'static [&'static str] {
        match self {
            Self::Sleep => SLEEP_HOMES,
            Self::TempDir => TEMP_HOMES,
        }
    }
}

/// A line of a test that breaks a rule.
#[derive(Debug, PartialEq, Eq)]
pub struct Broken {
    pub line: usize,
    pub rule: Rule,
}

/// Checks every Rust file of the workspace's crates and xtask, and fails
/// telling each line of a test that breaks a rule.
pub fn run(root: &Path) -> anyhow::Result<()> {
    let mut files = Vec::new();
    for folder in ["crates", "xtask"] {
        rust_files(&root.join(folder), &mut files)?;
    }
    files.sort();
    let mut told = 0;
    for file in files {
        let relative = file.strip_prefix(root).unwrap_or(&file);
        let path = relative.to_string_lossy().replace('\\', "/");
        let text = fs::read_to_string(&file)?;
        for broken in check(&path, &text) {
            println!("{path}:{}: {}", broken.line, broken.rule.says());
            told += 1;
        }
    }
    if told > 0 {
        bail!("{told} lines of tests break the rules of `cargo xtask lint-tests`");
    }
    Ok(())
}

fn rust_files(folder: &Path, into: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            rust_files(&path, into)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            into.push(path);
        }
    }
    Ok(())
}

/// The lines of `text`, the file at `path`, that break a rule: in a file
/// of a `tests` folder, or in a `#[cfg(test)]` module, and not in the
/// rule's own files, nor beside a comment that says why.
#[must_use]
pub fn check(path: &str, text: &str) -> Vec<Broken> {
    let lines: Vec<&str> = text.lines().collect();
    let tests = tested_lines(path, &lines);
    let mut broken = Vec::new();
    for rule in [Rule::Sleep, Rule::TempDir] {
        if rule.homes().contains(&path) {
            continue;
        }
        for (at, line) in lines.iter().enumerate() {
            if !tests[at] || !rule.breaks(line) {
                continue;
            }
            let said = lines[at.saturating_sub(2)..=at]
                .iter()
                .any(|line| line.contains(rule.marker()));
            if !said {
                broken.push(Broken { line: at + 1, rule });
            }
        }
    }
    broken.sort_by_key(|broken| broken.line);
    broken
}

/// Which of `lines` are tests' code: all of a file in a `tests` folder,
/// else the body of each module marked `#[cfg(test)]`, to its closing
/// brace.
fn tested_lines(path: &str, lines: &[&str]) -> Vec<bool> {
    if path.split('/').any(|part| part == "tests") {
        return vec![true; lines.len()];
    }
    let mut tests = vec![false; lines.len()];
    let mut at = 0;
    while at < lines.len() {
        if lines[at].trim() == "#[cfg(test)]" {
            let next = (at + 1..lines.len()).find(|&next| !lines[next].trim().is_empty());
            if let Some(start) = next.filter(|&start| {
                let line = lines[start].trim_start();
                (line.starts_with("mod ") || line.starts_with("pub mod ")) && line.ends_with('{')
            }) {
                let mut depth = 0i32;
                for (offset, line) in lines[start..].iter().enumerate() {
                    tests[start + offset] = true;
                    depth += braces(line);
                    if depth <= 0 {
                        at = start + offset;
                        break;
                    }
                }
            }
        }
        at += 1;
    }
    tests
}

/// How much deeper in braces `line` leaves the code: its opening braces
/// less its closing ones.
fn braces(line: &str) -> i32 {
    line.chars().fold(0, |depth, c| match c {
        '{' => depth + 1,
        '}' => depth - 1,
        _ => depth,
    })
}

#[cfg(test)]
mod tests {
    use super::{Broken, Rule, check};

    #[test]
    fn a_test_that_sleeps_breaks_the_rule() {
        let text = "fn main() {}\n\n#[cfg(test)]\nmod tests {\n    fn t() {\n        std::thread::sleep(D);\n    }\n}\n";
        assert_eq!(
            check("crates/a/src/lib.rs", text),
            [Broken {
                line: 6,
                rule: Rule::Sleep
            }]
        );
    }

    #[test]
    fn code_out_of_the_tests_may_sleep() {
        let text =
            "fn run() {\n    tokio::time::sleep(D).await;\n}\n#[cfg(test)]\nfn helper() {}\n";
        assert_eq!(check("crates/a/src/lib.rs", text), []);
    }

    #[test]
    fn a_comment_saying_why_lets_a_test_sleep() {
        let text = "fn t() {\n    // allow-sleep: holds the lock past the wait\n    std::thread::sleep(D);\n}\n";
        assert_eq!(check("crates/a/tests/it.rs", text), []);
    }

    #[test]
    fn every_line_of_a_tests_folder_is_a_test() {
        let text = "fn helper() -> PathBuf {\n    std::env::temp_dir().join(\"x\")\n}\n";
        assert_eq!(
            check("crates/a/tests/helpers/mod.rs", text),
            [Broken {
                line: 2,
                rule: Rule::TempDir
            }]
        );
    }

    #[test]
    fn a_function_that_makes_a_folder_of_its_own_is_no_shared_folder() {
        let text = "fn t() {\n    let folder = sh.create_temp_dir().unwrap();\n}\n";
        assert_eq!(check("xtask/tests/it.rs", text), []);
    }

    #[test]
    fn the_rules_own_files_may_do_what_they_make_for_the_tests() {
        let text = "fn wait() {\n    std::thread::sleep(D);\n}\n";
        assert_eq!(check("crates/cox/tests/fake_daemon/mod.rs", text), []);
    }

    #[test]
    fn a_test_module_ends_at_its_closing_brace() {
        let text = "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n\nfn after() {\n    std::thread::sleep(D);\n}\n";
        assert_eq!(check("crates/a/src/lib.rs", text), []);
    }
}
