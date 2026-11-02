//! What changed in the files of a Git repository: since a trial started
//! in it, or else since its last commit, as Git tells it, with the files
//! Git does not track yet but does not ignore either, which are new.
//!
//! A trial marks where it starts with a ref of its worktree's own,
//! [`START`], which Git keeps apart for each worktree: the commit a trial
//! of a project starts from, or the empty tree, for a trial from nothing,
//! so that all it made counts as changed, whatever it commits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The ref that marks where a trial started, in its worktree.
pub const START: &str = "refs/worktree/cox-start";

/// The most of a new file that is read, to show it: a larger one is shown
/// as a binary file is, by its name alone.
const LARGEST: u64 = 1024 * 1024;

/// What changed in a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diff {
    /// The top of the worktree, which the files' paths are from.
    pub top: PathBuf,
    /// What the files changed since.
    pub since: Since,
    /// The files that changed, in order of their paths.
    pub files: Vec<File>,
}

/// What the files of a [`Diff`] changed since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Since {
    /// The commit a trial started from, by its short name.
    Start(String),
    /// Nothing: every file is new, as in a trial from nothing.
    Nothing,
    /// The last commit, by its short name.
    Head(String),
}

/// A file that changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Where it is now, from the top of the worktree, or where it was, if
    /// it was removed.
    pub path: PathBuf,
    /// Where it was, if it moved.
    pub was: Option<PathBuf>,
    pub how: How,
    /// The lines that changed, with some around them, by stretches; none
    /// for a binary file.
    pub lines: Vec<Line>,
    pub binary: bool,
    /// How many lines it gained, and lost.
    pub added: usize,
    pub removed: usize,
}

/// How a file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Added,
    Removed,
    Changed,
    Moved,
}

impl How {
    /// The word for it, as Git's short status has.
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Self::Added => 'A',
            Self::Removed => 'D',
            Self::Changed => 'M',
            Self::Moved => 'R',
        }
    }
}

/// A line of what changed in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    /// Its number in the file as it was, if it was there.
    pub old: Option<usize>,
    /// Its number in the file as it is, if it is there.
    pub new: Option<usize>,
    /// The line of the file as it is where this line is, or would be: to
    /// open the file at.
    pub at: usize,
    pub text: String,
}

/// What a line of what changed is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Where a stretch of changed lines starts, as Git heads it.
    Stretch,
    /// A line that is as it was.
    Same,
    Added,
    Removed,
}

/// The lines each file that changed gained and lost, by its absolute
/// path, as the diff tells it. A file that was removed counts where it
/// was.
#[must_use]
pub fn changed_files(diff: &Diff) -> HashMap<PathBuf, (usize, usize)> {
    diff.files
        .iter()
        .map(|file| (diff.top.join(&file.path), (file.added, file.removed)))
        .collect()
}

impl Since {
    /// What it is, as people are told: what the files changed since.
    #[must_use]
    pub fn said(&self) -> String {
        match self {
            Self::Start(commit) => format!("since the trial started, at {commit}"),
            Self::Nothing => "since nothing, every file new".to_owned(),
            Self::Head(commit) => format!("since the last commit, {commit}"),
        }
    }
}

impl Diff {
    /// How many lines the files gained, and lost, in all.
    #[must_use]
    pub fn counts(&self) -> (usize, usize) {
        self.files.iter().fold((0, 0), |(added, removed), file| {
            (added + file.added, removed + file.removed)
        })
    }
}

/// What changed in the repository `folder` is in.
///
/// # Errors
///
/// When `folder` is not in a Git repository, or Git cannot tell what
/// changed, with what Git said.
pub fn read(folder: &Path) -> Result<Diff, String> {
    let top = PathBuf::from(git(folder, &["rev-parse", "--show-toplevel"])?.trim());
    let empty = git(&top, &["hash-object", "-t", "tree", "/dev/null"])?;
    let empty = empty.trim();
    let (since, from) = if let Some(start) = commit(&top, START) {
        let since = if start == empty {
            Since::Nothing
        } else {
            Since::Start(short(&start))
        };
        (since, start)
    } else if let Some(head) = commit(&top, "HEAD") {
        (Since::Head(short(&head)), head)
    } else {
        (Since::Nothing, empty.to_owned())
    };
    let patch = git(
        &top,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            &from,
            "--",
        ],
    )?;
    let mut files = parse(&patch);
    let untracked = git(&top, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    files.extend(
        untracked
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(|path| new_file(&top, Path::new(path))),
    );
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Diff { top, since, files })
}

/// Marks where a trial starts in the worktree at `folder`: at its last
/// commit, or at nothing, when it has none, as a trial from nothing.
///
/// # Errors
///
/// When Git cannot mark it, with what Git said.
pub fn start(folder: &Path) -> Result<(), String> {
    let from = match commit(folder, "HEAD") {
        Some(head) => head,
        None => git(folder, &["hash-object", "-w", "-t", "tree", "/dev/null"])?
            .trim()
            .to_owned(),
    };
    git(folder, &["update-ref", START, &from]).map(drop)
}

/// Forgets where a trial started in the worktree at `folder`, which goes
/// on as a project of its own: what changed is then what changed since
/// its last commit.
///
/// # Errors
///
/// When Git cannot forget it, with what Git said.
pub fn forget(folder: &Path) -> Result<(), String> {
    if commit(folder, START).is_none() {
        return Ok(());
    }
    git(folder, &["update-ref", "-d", START]).map(drop)
}

/// The full name of what `name` names in the repository at `folder`, if
/// it names anything.
fn commit(folder: &Path, name: &str) -> Option<String> {
    git(folder, &["rev-parse", "--verify", "--quiet", name])
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// The short name of a commit, as people read it.
fn short(name: &str) -> String {
    name.chars().take(7).collect()
}

/// A file Git does not track, at `path` from `top`: all of it is new.
fn new_file(top: &Path, path: &Path) -> File {
    let full = top.join(path);
    let text = std::fs::metadata(&full)
        .ok()
        .filter(|metadata| metadata.len() <= LARGEST)
        .and_then(|_| std::fs::read(&full).ok())
        .filter(|bytes| !bytes.contains(&0))
        .and_then(|bytes| String::from_utf8(bytes).ok());
    let mut file = File {
        path: path.to_owned(),
        was: None,
        how: How::Added,
        lines: Vec::new(),
        binary: text.is_none(),
        added: 0,
        removed: 0,
    };
    if let Some(text) = text {
        let count = text.lines().count();
        if count > 0 {
            file.lines.push(Line {
                kind: Kind::Stretch,
                old: None,
                new: None,
                at: 1,
                text: format!("@@ -0,0 +1,{count} @@"),
            });
        }
        file.lines
            .extend(text.lines().enumerate().map(|(index, text)| Line {
                kind: Kind::Added,
                old: None,
                new: Some(index + 1),
                at: index + 1,
                text: text.to_owned(),
            }));
        file.added = count;
    }
    file
}

/// The files that changed, as the patch Git printed says.
fn parse(patch: &str) -> Vec<File> {
    let mut files: Vec<File> = Vec::new();
    // Where the next lines are, in the file as it was and as it is, once
    // in a stretch of the current file.
    let mut at: Option<(usize, usize)> = None;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            files.push(File {
                path: PathBuf::from(both_named(rest).unwrap_or(rest)),
                was: None,
                how: How::Changed,
                lines: Vec::new(),
                binary: false,
                added: 0,
                removed: 0,
            });
            at = None;
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if let Some(stretch) = line.strip_prefix("@@ ") {
            let (old, new) = starts(stretch).unwrap_or((1, 1));
            file.lines.push(Line {
                kind: Kind::Stretch,
                old: None,
                new: None,
                at: new.max(1),
                text: line.to_owned(),
            });
            at = Some((old, new));
            continue;
        }
        let Some((old, new)) = at.as_mut() else {
            header(file, line);
            continue;
        };
        let (was, is) = (*old, *new);
        let line = match line.split_at_checked(1) {
            Some((" ", text)) => {
                *old += 1;
                *new += 1;
                Line {
                    kind: Kind::Same,
                    old: Some(was),
                    new: Some(is),
                    at: is,
                    text: text.to_owned(),
                }
            }
            Some(("+", text)) => {
                *new += 1;
                file.added += 1;
                Line {
                    kind: Kind::Added,
                    old: None,
                    new: Some(is),
                    at: is,
                    text: text.to_owned(),
                }
            }
            Some(("-", text)) => {
                *old += 1;
                file.removed += 1;
                Line {
                    kind: Kind::Removed,
                    old: Some(was),
                    new: None,
                    at: is.max(1),
                    text: text.to_owned(),
                }
            }
            // "\ No newline at end of file", and what is not a line.
            _ => continue,
        };
        file.lines.push(line);
    }
    files
}

/// What a line of the header of a file's patch says of the file.
fn header(file: &mut File, line: &str) {
    if line.starts_with("new file mode") {
        file.how = How::Added;
    } else if line.starts_with("deleted file mode") {
        file.how = How::Removed;
    } else if let Some(was) = line.strip_prefix("rename from ") {
        file.how = How::Moved;
        file.was = Some(PathBuf::from(was));
    } else if let Some(path) = line.strip_prefix("rename to ") {
        file.path = PathBuf::from(path);
    } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
        file.binary = true;
    } else if let Some(path) = line.strip_prefix("+++ b/") {
        file.path = PathBuf::from(path);
    } else if let Some(path) = line.strip_prefix("--- a/")
        && file.how == How::Removed
    {
        file.path = PathBuf::from(path);
    }
}

/// The path that `a/<path> b/<path>` names twice, as the first line of a
/// file's patch does when the file did not move.
fn both_named(names: &str) -> Option<&str> {
    let names = names.strip_prefix("a/")?;
    let length = names.len().checked_sub(3)? / 2;
    let (first, rest) = names.split_at_checked(length)?;
    (rest.strip_prefix(" b/")? == first).then_some(first)
}

/// Where a stretch starts in the file as it was and as it is, from what
/// follows `@@ ` in its heading, as `-12,3 +12,4 @@`.
fn starts(stretch: &str) -> Option<(usize, usize)> {
    let mut parts = stretch.split(' ');
    let start = |part: Option<&str>, sign: char| -> Option<usize> {
        let part = part?.strip_prefix(sign)?;
        part.split(',').next()?.parse().ok()
    };
    let old = start(parts.next(), '-')?;
    let new = start(parts.next(), '+')?;
    Some((old, new))
}

/// Runs Git in `folder` with `args`, and answers what it printed, or what
/// it said went wrong.
fn git(folder: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test, made a Git repository with a
    /// commit of the files `files`, saying what they say.
    fn repository(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("cox-changes-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        git(&folder, &["init", "--quiet"]).unwrap();
        for (path, text) in files {
            std::fs::write(folder.join(path), text).unwrap();
        }
        if !files.is_empty() {
            commit_all(&folder);
        }
        folder
    }

    fn commit_all(folder: &Path) {
        git(folder, &["add", "--all"]).unwrap();
        git(
            folder,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "-m",
                "Files",
            ],
        )
        .unwrap();
    }

    #[test]
    fn reads_the_lines_changed_since_the_last_commit() {
        let folder = repository("changed", &[("a.txt", "one\ntwo\nthree\n")]);
        std::fs::write(folder.join("a.txt"), "one\n2\nthree\n").unwrap();
        let diff = read(&folder).unwrap();
        assert!(matches!(diff.since, Since::Head(_)));
        assert_eq!(diff.files.len(), 1);
        let file = &diff.files[0];
        assert_eq!(file.path, Path::new("a.txt"));
        assert_eq!(file.how, How::Changed);
        assert_eq!((file.added, file.removed), (1, 1));
        let lines: Vec<_> = file
            .lines
            .iter()
            .map(|line| (line.kind, line.old, line.new, line.at, line.text.as_str()))
            .collect();
        assert_eq!(
            lines[1..],
            [
                (Kind::Same, Some(1), Some(1), 1, "one"),
                (Kind::Removed, Some(2), None, 2, "two"),
                (Kind::Added, None, Some(2), 2, "2"),
                (Kind::Same, Some(3), Some(3), 3, "three"),
            ]
        );
        assert_eq!(lines[0].0, Kind::Stretch);
    }

    #[test]
    fn a_file_git_does_not_track_is_new() {
        let folder = repository("new", &[("a.txt", "a\n")]);
        std::fs::write(folder.join("b.txt"), "first\nsecond\n").unwrap();
        std::fs::write(folder.join(".gitignore"), "ignored\n").unwrap();
        std::fs::write(folder.join("ignored"), "not shown\n").unwrap();
        let diff = read(&folder).unwrap();
        let paths: Vec<_> = diff.files.iter().map(|file| file.path.clone()).collect();
        assert_eq!(paths, [PathBuf::from(".gitignore"), PathBuf::from("b.txt")]);
        let file = &diff.files[1];
        assert_eq!(file.how, How::Added);
        assert_eq!(file.added, 2);
        assert_eq!(file.lines[2].text, "second");
        assert_eq!(file.lines[2].new, Some(2));
    }

    #[test]
    fn a_file_removed_or_moved_says_so() {
        let folder = repository(
            "moved",
            &[
                ("gone.txt", "bye\n"),
                ("old.txt", "the same\nlines\nhere\n"),
            ],
        );
        std::fs::remove_file(folder.join("gone.txt")).unwrap();
        git(&folder, &["mv", "old.txt", "new.txt"]).unwrap();
        let diff = read(&folder).unwrap();
        let files: Vec<_> = diff
            .files
            .iter()
            .map(|file| (file.path.clone(), file.was.clone(), file.how))
            .collect();
        assert_eq!(
            files,
            [
                (PathBuf::from("gone.txt"), None, How::Removed),
                (
                    PathBuf::from("new.txt"),
                    Some(PathBuf::from("old.txt")),
                    How::Moved
                ),
            ]
        );
        assert_eq!(diff.counts(), (0, 1));
    }

    #[test]
    fn a_trial_counts_what_it_committed_since_it_started() {
        let folder = repository("trial", &[("a.txt", "a\n")]);
        start(&folder).unwrap();
        std::fs::write(folder.join("a.txt"), "b\n").unwrap();
        commit_all(&folder);
        let diff = read(&folder).unwrap();
        assert!(matches!(diff.since, Since::Start(_)));
        assert_eq!(diff.counts(), (1, 1));
        forget(&folder).unwrap();
        let diff = read(&folder).unwrap();
        assert!(matches!(diff.since, Since::Head(_)));
        assert_eq!(diff.files, []);
    }

    #[test]
    fn a_trial_from_nothing_counts_all_it_made() {
        let folder = repository("nothing", &[]);
        start(&folder).unwrap();
        std::fs::write(folder.join("a.txt"), "a\n").unwrap();
        commit_all(&folder);
        std::fs::write(folder.join("b.txt"), "b\n").unwrap();
        let diff = read(&folder).unwrap();
        assert_eq!(diff.since, Since::Nothing);
        let paths: Vec<_> = diff.files.iter().map(|file| file.path.clone()).collect();
        assert_eq!(paths, [PathBuf::from("a.txt"), PathBuf::from("b.txt")]);
        assert!(diff.files.iter().all(|file| file.how == How::Added));
    }

    #[test]
    fn a_folder_out_of_git_is_no_repository() {
        let folder = std::env::temp_dir().join(format!("cox-changes-{}-none", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        // The temporary folder may be in a repository, on some machines.
        if git(&folder, &["rev-parse", "--show-toplevel"]).is_err() {
            assert!(read(&folder).is_err());
        }
    }

    #[test]
    fn names_the_path_of_a_file_that_did_not_move() {
        assert_eq!(
            both_named("a/src/main.rs b/src/main.rs"),
            Some("src/main.rs")
        );
        assert_eq!(both_named("a/a b/c b/a b/c"), Some("a b/c"));
        assert_eq!(both_named("a/old b/new"), None);
        assert_eq!(starts("-12,3 +14,4 @@ fn main"), Some((12, 14)));
        assert_eq!(starts("-0,0 +1 @@"), Some((0, 1)));
    }
}
