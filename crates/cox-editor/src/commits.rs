//! The commits of a Git repository that the work in it made, newest first,
//! as Git tells them: in a sketch, those since it started (see
//! [`crate::changes::START`]); elsewhere, those of the branch since it left
//! the one it came from, or else the last few.
//!
//! As a turn of a chat ends, Cox marks each commit its agent made in it with
//! the turn, in a note of Git's of its own, [`NOTES`], which Git keeps
//! apart from the notes `git log` shows, and does not push; a commit read
//! here has the turn it was made in, if it was marked.

use std::path::{Path, PathBuf};

use crate::changes::{self, Diff, Since, commit, git, parse, short};

/// The notes Cox marks commits with the turn they were made in.
pub const NOTES: &str = "refs/notes/cox";

/// The most commits read at once.
const MOST: usize = 200;

/// How many commits are read where nothing tells which are the work's.
const RECENT: usize = 30;

/// The commits of a repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    /// The top of the worktree.
    pub top: PathBuf,
    /// Where the commits read start, after.
    pub base: Base,
    /// The commits, newest first.
    pub commits: Vec<Commit>,
}

/// Where the commits of a [`History`] start, after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Base {
    /// The commit a sketch started from, by its short name.
    Start(String),
    /// Nothing: a sketch from nothing, all of whose commits are read.
    Nothing,
    /// Where the branch left another, `from`, by its name, as `origin/main`,
    /// at the commit `at`, by its short name.
    Branched { from: String, at: String },
    /// Nothing tells where: the last commits are read, as many as
    /// [`RECENT`].
    Recent,
}

/// A commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    /// Its full name.
    pub id: String,
    /// The first line of its message.
    pub subject: String,
    /// When it was made, in seconds since the epoch.
    pub when: i64,
    /// How many lines its files gained, and lost, in all.
    pub added: usize,
    pub removed: usize,
    /// The turn it was made in, when Cox marked it.
    pub turn: Option<Turn>,
}

/// The turn of a chat a commit was made in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// The chat, by the number Cox keeps it by.
    pub chat: i64,
    /// The entry of the chat's conversation that started the turn, by its
    /// place, from 0.
    pub entry: usize,
    /// The first line of what started it: the human's line, or Cox's in
    /// their place.
    pub line: String,
}

impl Commit {
    /// Its short name, as people read it.
    #[must_use]
    pub fn short(&self) -> String {
        short(&self.id)
    }
}

impl Turn {
    /// The note of a commit made in it.
    #[must_use]
    pub fn note(&self) -> String {
        let line = self.line.lines().next().unwrap_or_default();
        format!("chat {}\nentry {}\n{line}\n", self.chat, self.entry)
    }

    /// The turn `note` marks, if it marks one.
    #[must_use]
    pub fn read(note: &str) -> Option<Self> {
        let mut lines = note.lines();
        let chat = lines.next()?.strip_prefix("chat ")?.trim().parse().ok()?;
        let entry = lines.next()?.strip_prefix("entry ")?.trim().parse().ok()?;
        let line = lines.next().unwrap_or_default().to_owned();
        Some(Self { chat, entry, line })
    }
}

impl Base {
    /// What it is, as people are told: where the commits start, after.
    #[must_use]
    pub fn said(&self) -> String {
        match self {
            Self::Start(commit) => format!("The sketch started at {commit}."),
            Self::Nothing => "The sketch started from nothing.".to_owned(),
            Self::Branched { from, at } => format!("Branched from {from} at {at}."),
            Self::Recent => format!("The last {RECENT} commits."),
        }
    }
}

/// The commits of the repository `folder` is in (see the module's doc).
///
/// # Errors
///
/// When `folder` is not in a Git repository, with what Git said.
pub fn history(folder: &Path) -> Result<History, String> {
    let top = PathBuf::from(git(folder, &["rev-parse", "--show-toplevel"])?.trim());
    let (base, after) = base(&top);
    let commits = match (&base, after) {
        (_, Some(after)) => log(&top, &format!("{after}..HEAD"), MOST),
        (Base::Recent, None) => log(&top, "HEAD", RECENT),
        (_, None) => log(&top, "HEAD", MOST),
    };
    Ok(History { top, base, commits })
}

/// Where the commits of the worktree at `top` start, and the commit they
/// start after, if they do not start at its first.
fn base(top: &Path) -> (Base, Option<String>) {
    if let Some(start) = commit(top, changes::START) {
        let is_commit = commit(top, &format!("{start}^{{commit}}")).is_some();
        return if is_commit {
            (Base::Start(short(&start)), Some(start))
        } else {
            (Base::Nothing, None)
        };
    }
    let Some(head) = commit(top, "HEAD") else {
        return (Base::Recent, None);
    };
    let default = git(
        top,
        &["rev-parse", "--abbrev-ref", "refs/remotes/origin/HEAD"],
    )
    .ok()
    .map(|name| name.trim().to_owned())
    .filter(|name| !name.is_empty() && name != "refs/remotes/origin/HEAD");
    let upstream = git(
        top,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .ok()
    .map(|name| name.trim().to_owned());
    let others = default
        .into_iter()
        .chain(["origin/main".to_owned(), "origin/master".to_owned()])
        .chain(upstream);
    for other in others {
        let Some(at) = git(top, &["merge-base", "HEAD", &other])
            .ok()
            .map(|at| at.trim().to_owned())
            .filter(|at| !at.is_empty())
        else {
            continue;
        };
        if at != head {
            return (
                Base::Branched {
                    from: other,
                    at: short(&at),
                },
                Some(at),
            );
        }
    }
    (Base::Recent, None)
}

/// Everything that changed in the repository `folder` is in since its
/// commits start (see [`history`]), as one: in a sketch, what changed
/// since it started; on a branch, since it left the other; none where
/// nothing tells where they start.
///
/// # Errors
///
/// When `folder` is not in a Git repository, or Git cannot tell what
/// changed, with what Git said.
pub fn everything(folder: &Path) -> Result<Option<Diff>, String> {
    let top = PathBuf::from(git(folder, &["rev-parse", "--show-toplevel"])?.trim());
    match base(&top) {
        (Base::Start(_) | Base::Nothing, _) => changes::read(folder).map(Some),
        (Base::Branched { from, at }, Some(after)) => {
            changes::read_from(top, Since::Branched { from, at }, &after).map(Some)
        }
        _ => Ok(None),
    }
}

/// What changed in the repository `folder` is in since its last commit,
/// with the files Git does not track, which are new.
///
/// # Errors
///
/// When `folder` is not in a Git repository, or Git cannot tell what
/// changed, with what Git said.
pub fn uncommitted(folder: &Path) -> Result<Diff, String> {
    let top = PathBuf::from(git(folder, &["rev-parse", "--show-toplevel"])?.trim());
    if let Some(head) = commit(&top, "HEAD") {
        changes::read_from(top, Since::Head(short(&head)), &head)
    } else {
        let empty = git(&top, &["hash-object", "-t", "tree", "/dev/null"])?;
        changes::read_from(top, Since::Nothing, empty.trim())
    }
}

/// What the commit `id` changed, in the repository `folder` is in: since
/// the commit before it, or, for a first commit, since nothing.
///
/// # Errors
///
/// When `folder` is not in a Git repository, or has no such commit, with
/// what Git said.
pub fn diff_of(folder: &Path, id: &str) -> Result<Diff, String> {
    let top = PathBuf::from(git(folder, &["rev-parse", "--show-toplevel"])?.trim());
    let before = match commit(&top, &format!("{id}^")) {
        Some(before) => before,
        None => git(&top, &["hash-object", "-t", "tree", "/dev/null"])?
            .trim()
            .to_owned(),
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
            &before,
            id,
            "--",
        ],
    )?;
    let mut files = parse(&patch);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Diff {
        top,
        since: Since::Commit(short(id)),
        files,
    })
}

/// The last commit of the repository `folder` is in, by its full name, if
/// it has one.
#[must_use]
pub fn head(folder: &Path) -> Option<String> {
    commit(folder, "HEAD")
}

/// The commits made in the repository `folder` is in since `from`, its last
/// commit as a turn started, or all of them, when it had none then, newest
/// first. Commits `from` was and that are no longer, as one amended, are
/// not among them.
#[must_use]
pub fn made(folder: &Path, from: Option<&str>) -> Vec<Commit> {
    if commit(folder, "HEAD").is_none() {
        return Vec::new();
    }
    match from {
        Some(from) => log(folder, &format!("{from}..HEAD"), MOST),
        None => log(folder, "HEAD", MOST),
    }
}

/// Marks the commit `id`, in the repository `folder` is in, as made in
/// `turn`, in Cox's notes.
///
/// # Errors
///
/// When Git cannot, with what Git said.
pub fn mark(folder: &Path, id: &str, turn: &Turn) -> Result<(), String> {
    // A note is a commit of the notes, which Git makes as someone: Cox, as
    // the human may have told Git no name.
    git(
        folder,
        &[
            "-c",
            "user.name=Cox",
            "-c",
            "user.email=cox@localhost",
            "notes",
            "--ref",
            NOTES,
            "add",
            "--force",
            "--message",
            &turn.note(),
            id,
        ],
    )
    .map(drop)
}

/// The commits of `range`, as `git log` takes it, at most `most`, newest
/// first, with the turns Cox marked them with. None where Git cannot tell.
fn log(folder: &Path, range: &str, most: usize) -> Vec<Commit> {
    let most = most.to_string();
    let notes = format!("--notes={NOTES}");
    let Ok(printed) = git(
        folder,
        &[
            "-c",
            "core.quotePath=false",
            "log",
            "--no-color",
            "--numstat",
            "--format=%x1e%H%x1f%ct%x1f%s%x1f%N%x1f",
            &notes,
            "-n",
            &most,
            range,
            "--",
        ],
    ) else {
        return Vec::new();
    };
    printed
        .split('\x1e')
        .filter_map(|record| {
            let mut fields = record.split('\x1f');
            let id = fields.next()?.trim().to_owned();
            if id.is_empty() {
                return None;
            }
            let when = fields.next()?.trim().parse().unwrap_or_default();
            let subject = fields.next()?.to_owned();
            let turn = Turn::read(fields.next().unwrap_or_default());
            let (added, removed) = fields
                .next()
                .unwrap_or_default()
                .lines()
                .filter_map(|line| {
                    let mut counts = line.split('\t');
                    // A binary file counts no lines: `-`.
                    let added = counts.next()?.parse().unwrap_or(0);
                    let removed = counts.next()?.parse().unwrap_or(0);
                    Some((added, removed))
                })
                .fold((0, 0), |(a, r), (added, removed)| (a + added, r + removed));
            Some(Commit {
                id,
                subject,
                when,
                added,
                removed,
                turn,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(folder: &Path, args: &[&str]) {
        git(folder, args).unwrap();
    }

    /// A folder of its own for a test, made a Git repository with a first
    /// commit.
    fn repository(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("cox-commits-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        run(&folder, &["init", "--quiet", "--initial-branch=main"]);
        write_and_commit(&folder, "a.txt", "a\n", "First");
        folder
    }

    fn write_and_commit(folder: &Path, path: &str, text: &str, message: &str) -> String {
        std::fs::write(folder.join(path), text).unwrap();
        run(folder, &["add", "--all"]);
        run(
            folder,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "-m",
                message,
            ],
        );
        head(folder).unwrap()
    }

    #[test]
    fn a_sketch_reads_its_commits_since_it_started_with_their_turns() {
        let folder = repository("sketch");
        changes::start(&folder).unwrap();
        let from = head(&folder);
        let one = write_and_commit(&folder, "a.txt", "a\nb\n", "One");
        let two = write_and_commit(&folder, "b.txt", "x\ny\n", "Two");
        let made = made(&folder, from.as_deref());
        assert_eq!(
            made.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            [two.as_str(), one.as_str()]
        );
        let turn = Turn {
            chat: 3,
            entry: 12,
            line: "Add b\nand more".to_owned(),
        };
        mark(&folder, &two, &turn).unwrap();
        let history = history(&folder).unwrap();
        assert!(matches!(history.base, Base::Start(_)));
        let read: Vec<_> = history
            .commits
            .iter()
            .map(|c| (c.subject.as_str(), c.added, c.removed, c.turn.clone()))
            .collect();
        assert_eq!(
            read,
            [
                (
                    "Two",
                    2,
                    0,
                    Some(Turn {
                        chat: 3,
                        entry: 12,
                        line: "Add b".to_owned()
                    })
                ),
                ("One", 1, 0, None),
            ]
        );
        // The notes are Cox's own, not those `git log` shows.
        let shown = git(&folder, &["log", "-1", "--format=%N"]).unwrap();
        assert_eq!(shown.trim(), "");
    }

    #[test]
    fn reads_what_a_commit_changed_and_what_is_not_committed() {
        let folder = repository("diff");
        let one = write_and_commit(&folder, "a.txt", "a\nb\n", "One");
        std::fs::write(folder.join("a.txt"), "a\nc\n").unwrap();
        let diff = diff_of(&folder, &one).unwrap();
        assert_eq!(diff.since, Since::Commit(short(&one)));
        assert_eq!(diff.counts(), (1, 0));
        let diff = uncommitted(&folder).unwrap();
        assert_eq!(diff.counts(), (1, 1));
        // The first commit changed everything from nothing.
        let first = history(&folder).unwrap().commits.last().unwrap().id.clone();
        assert_eq!(diff_of(&folder, &first).unwrap().counts(), (1, 0));
    }

    #[test]
    fn a_branch_reads_its_commits_since_it_left_another() {
        let origin = repository("origin");
        let folder = std::env::temp_dir().join(format!("cox-commits-{}-clone", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        run(
            &origin,
            &["clone", "--quiet", ".", folder.to_str().unwrap()],
        );
        // On the branch it came from, with nothing new: the last commits.
        assert_eq!(history(&folder).unwrap().base, Base::Recent);
        run(&folder, &["checkout", "--quiet", "-b", "work"]);
        write_and_commit(&folder, "b.txt", "b\n", "Work");
        let history = history(&folder).unwrap();
        assert!(matches!(
            &history.base,
            Base::Branched { from, .. } if from == "origin/main"
        ));
        assert_eq!(history.commits.len(), 1);
        std::fs::write(folder.join("c.txt"), "c\n").unwrap();
        let everything = everything(&folder).unwrap().unwrap();
        let paths: Vec<_> = everything.files.iter().map(|f| f.path.clone()).collect();
        assert_eq!(paths, [PathBuf::from("b.txt"), PathBuf::from("c.txt")]);
    }

    #[test]
    fn an_amended_commit_is_made_anew() {
        let folder = repository("amend");
        let from = head(&folder);
        write_and_commit(&folder, "a.txt", "a\nb\n", "One");
        let before = head(&folder);
        run(
            &folder,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "--amend",
                "-m",
                "One, again",
            ],
        );
        let made_after = made(&folder, before.as_deref());
        assert_eq!(made_after.len(), 1);
        assert_eq!(made_after[0].subject, "One, again");
        assert_eq!(made(&folder, from.as_deref()).len(), 1);
    }
}
