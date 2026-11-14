//! The commits of a Git repository that the work in it made, newest first,
//! as Git tells them: in a sketch, those since it started (see
//! [`crate::changes::START`]); elsewhere, those of the branch since it left
//! the one it came from, or else the last few.
//!
//! As a turn of a chat ends, Cox marks each commit its agent made in it with
//! the turn, in a note of Git's of its own, [`NOTES`], which Git keeps
//! apart from the notes `git log` shows, and does not push; a commit read
//! here has the turn it was made in, if it was marked.
//!
//! A commit amended is made anew, and the one it was is no longer among
//! them; a commit read here has what it was before, as this worktree's
//! reflog of `HEAD` tells it, until Git lets that go.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::changes::{self, Diff, Since, commit, parse, short};
use crate::git;

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
    /// What it was before it was amended, as the reflog tells it, oldest
    /// first, each with nothing here: the commit it was first, and each it
    /// was after, up to the one this amended.
    pub earlier: Vec<Commit>,
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

    /// Whether it is, or was before it was amended, the commit `id`.
    #[must_use]
    pub fn was(&self, id: &str) -> bool {
        self.id == id || self.earlier.iter().any(|earlier| earlier.id == id)
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
    let top = PathBuf::from(git::raw(folder, &["rev-parse", "--show-toplevel"])?.trim());
    let (base, after) = base(&top);
    let commits = match (&base, after) {
        (_, Some(after)) => log(&top, &[&format!("{after}..HEAD")], MOST),
        (Base::Recent, None) => log(&top, &["HEAD"], RECENT),
        (_, None) => log(&top, &["HEAD"], MOST),
    };
    let commits = with_earlier(&top, commits);
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
    let default = git::raw(
        top,
        &["rev-parse", "--abbrev-ref", "refs/remotes/origin/HEAD"],
    )
    .ok()
    .map(|name| name.trim().to_owned())
    .filter(|name| !name.is_empty() && name != "refs/remotes/origin/HEAD");
    let upstream = git::raw(
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
        let Some(at) = git::raw(top, &["merge-base", "HEAD", &other])
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
    let top = PathBuf::from(git::raw(folder, &["rev-parse", "--show-toplevel"])?.trim());
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
    let top = PathBuf::from(git::raw(folder, &["rev-parse", "--show-toplevel"])?.trim());
    if let Some(head) = commit(&top, "HEAD") {
        changes::read_from(top, Since::Head(short(&head)), &head)
    } else {
        let empty = git::raw(&top, &["hash-object", "-t", "tree", "/dev/null"])?;
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
    let top = PathBuf::from(git::raw(folder, &["rev-parse", "--show-toplevel"])?.trim());
    let before = match commit(&top, &format!("{id}^")) {
        Some(before) => before,
        None => git::raw(&top, &["hash-object", "-t", "tree", "/dev/null"])?
            .trim()
            .to_owned(),
    };
    between(top, &before, id, Since::Commit(short(id)))
}

/// What amending the commit `from` into the commit `to` changed, in the
/// repository `folder` is in: what differs between the files of the two.
///
/// # Errors
///
/// When `folder` is not in a Git repository, or has no such commits, with
/// what Git said.
pub fn amendment(folder: &Path, from: &str, to: &str) -> Result<Diff, String> {
    let top = PathBuf::from(git::raw(folder, &["rev-parse", "--show-toplevel"])?.trim());
    let since = Since::Amended {
        from: short(from),
        to: short(to),
    };
    between(top, from, to, since)
}

/// What differs between the files of `before` and those of `after`, in the
/// worktree at `top`, as a [`Diff`] since `since`.
fn between(top: PathBuf, before: &str, after: &str, since: Since) -> Result<Diff, String> {
    let patch = git::raw(
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
            before,
            after,
            "--",
        ],
    )?;
    let mut files = parse(&patch);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Diff { top, since, files })
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
    let commits = match from {
        Some(from) => log(folder, &[&format!("{from}..HEAD")], MOST),
        None => log(folder, &["HEAD"], MOST),
    };
    with_earlier(folder, commits)
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
    git::raw(
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

/// The commits `commits` were before they were amended, each the commit
/// they were made anew from, by their full names, as the reflog of `HEAD`
/// of the worktree `folder` is in tells: Git logs each amend there, with
/// what `HEAD` was before it.
fn amended(folder: &Path) -> HashMap<String, String> {
    let Ok(printed) = git::raw(
        folder,
        &["log", "--walk-reflogs", "--format=%H%x1f%gs", "HEAD", "--"],
    ) else {
        return HashMap::new();
    };
    // Newest first: each `HEAD` became what the one above it is.
    let logged: Vec<(&str, &str)> = printed
        .lines()
        .filter_map(|line| line.split_once('\x1f'))
        .collect();
    logged
        .iter()
        .zip(logged.iter().skip(1))
        .filter(|((_, said), _)| said.starts_with("commit (amend):"))
        .map(|((after, _), (before, _))| ((*after).to_owned(), (*before).to_owned()))
        .filter(|(after, before)| after != before)
        .collect()
}

/// `commits`, each with what it was before it was amended (see
/// [`Commit::earlier`]).
fn with_earlier(folder: &Path, mut commits: Vec<Commit>) -> Vec<Commit> {
    let amended = amended(folder);
    if amended.is_empty() {
        return commits;
    }
    // What each was, newest first, followed as far as the reflog goes,
    // once round, should it go round.
    let chains: Vec<Vec<String>> = commits
        .iter()
        .map(|commit| {
            let mut seen = HashSet::from([commit.id.clone()]);
            let mut chain = Vec::new();
            let mut at = &commit.id;
            while let Some(before) = amended.get(at) {
                if !seen.insert(before.clone()) {
                    break;
                }
                chain.push(before.clone());
                at = before;
            }
            chain
        })
        .collect();
    // Git lets go of what no ref holds, in time: those are left out.
    let mut seen = HashSet::new();
    let ids: Vec<&str> = chains
        .iter()
        .flatten()
        .map(String::as_str)
        .filter(|id| seen.insert(*id) && commit(folder, &format!("{id}^{{commit}}")).is_some())
        .collect();
    if ids.is_empty() {
        return commits;
    }
    let mut args = vec!["--no-walk=unsorted"];
    args.extend(&ids);
    let read: HashMap<String, Commit> = log(folder, &args, ids.len())
        .into_iter()
        .map(|commit| (commit.id.clone(), commit))
        .collect();
    for (commit, chain) in commits.iter_mut().zip(chains) {
        commit.earlier = chain
            .iter()
            .rev()
            .filter_map(|id| read.get(id).cloned())
            .collect();
    }
    commits
}

/// The commits `args` tell `git log`, at most `most`, newest first, with
/// the turns Cox marked them with. None where Git cannot tell.
fn log(folder: &Path, args: &[&str], most: usize) -> Vec<Commit> {
    let most = most.to_string();
    let notes = format!("--notes={NOTES}");
    let mut all = vec![
        "-c",
        "core.quotePath=false",
        "log",
        "--no-color",
        "--numstat",
        // A merge counts what it changed since its first parent, as its
        // diff shows: with no say, `git log` counts nothing for it.
        "--diff-merges=first-parent",
        "--format=%x1e%H%x1f%ct%x1f%s%x1f%N%x1f",
        &notes,
        "-n",
        &most,
    ];
    all.extend(args);
    all.push("--");
    let Ok(printed) = git::raw(folder, &all) else {
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
                earlier: Vec::new(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Folder;

    fn run(folder: &Path, args: &[&str]) {
        git::raw(folder, args).unwrap();
    }

    /// A folder of its own for a test, made a Git repository with a first
    /// commit.
    fn repository() -> Folder {
        let folder = Folder::new();
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
        let folder = repository();
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
        let shown = git::raw(&folder, &["log", "-1", "--format=%N"]).unwrap();
        assert_eq!(shown.trim(), "");
    }

    #[test]
    fn reads_what_a_commit_changed_and_what_is_not_committed() {
        let folder = repository();
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
    fn a_merge_counts_what_it_changed_since_its_first_parent() {
        let folder = repository();
        run(&folder, &["checkout", "--quiet", "-b", "side"]);
        write_and_commit(&folder, "b.txt", "b\nc\n", "Side");
        run(&folder, &["checkout", "--quiet", "main"]);
        write_and_commit(&folder, "a.txt", "a\nd\n", "Main");
        run(
            &folder,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "merge",
                "--quiet",
                "--no-ff",
                "-m",
                "Merge side",
                "side",
            ],
        );
        let merge = head(&folder).unwrap();
        let commits = made(&folder, None);
        let read = commits.iter().find(|c| c.id == merge).unwrap();
        assert_eq!((read.added, read.removed), (2, 0));
        assert_eq!(diff_of(&folder, &merge).unwrap().counts(), (2, 0));
    }

    #[test]
    fn a_branch_reads_its_commits_since_it_left_another() {
        let origin = repository();
        let folder = Folder::new();
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
        let folder = repository();
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

    fn amend(folder: &Path, path: &str, text: &str, message: &str) -> String {
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
                "--amend",
                "-m",
                message,
            ],
        );
        head(folder).unwrap()
    }

    #[test]
    fn an_amended_commit_has_what_it_was_and_what_amending_it_changed() {
        let folder = repository();
        changes::start(&folder).unwrap();
        let first = write_and_commit(&folder, "a.txt", "a\nb\n", "Greet");
        let turn = Turn {
            chat: 1,
            entry: 2,
            line: "Greet".to_owned(),
        };
        mark(&folder, &first, &turn).unwrap();
        let second = amend(&folder, "a.txt", "a\nc\n", "Greet by name");
        let third = amend(&folder, "a.txt", "a\nc\nd\n", "Greet by name!");
        let made = made(&folder, Some(&second));
        assert_eq!(made.len(), 1);
        let earlier: Vec<_> = made[0].earlier.iter().map(|c| c.id.clone()).collect();
        assert_eq!(earlier, [first.clone(), second.clone()]);
        assert_eq!(made[0].earlier[0].turn, Some(turn));
        assert!(made[0].was(&first) && made[0].was(&third));
        let read = history(&folder).unwrap();
        assert_eq!(read.commits.len(), 1);
        assert_eq!(read.commits[0].earlier.len(), 2);
        // Amending it the first time took b out for c.
        let diff = amendment(&folder, &first, &second).unwrap();
        assert_eq!(
            diff.since,
            Since::Amended {
                from: short(&first),
                to: short(&second)
            }
        );
        assert_eq!(diff.counts(), (1, 1));
        // A commit only made has nothing earlier.
        let other = write_and_commit(&folder, "b.txt", "b\n", "Other");
        let read = history(&folder).unwrap();
        assert_eq!(read.commits[0].id, other);
        assert_eq!(read.commits[0].earlier, Vec::<Commit>::new());
        assert_eq!(read.commits[1].earlier.len(), 2);
    }
}
