//! A sketch: a chat that tries something out apart from its project,
//! before the human and the agent that opened it agree what the work is
//! to be. Its agent works in a folder of its own, so what it changes
//! leaves the project's files be: a Git worktree of the project's, made
//! from the project's last commit, to try a change to the project; or an
//! empty folder, made a Git repository, to try something from nothing.
//! The chat that opened it reads what it made there, and proposes a goal
//! from it.
//!
//! Cox makes the folder when the chat opens, in a folder of its own for
//! sketches (see [`crate::storage`]), and keeps where it is with the chat's
//! setup, so the chat goes on there once it is started again. It marks
//! where the sketch starts in the folder's repository, so that what the
//! sketch changed shows on the stage as all it changed since then (see
//! [`cox_git::changes`]).
//!
//! Once the human has what they wanted of it, the sketch ends, in the
//! ways the human's workflow says (see [`crate::workflow::Sketch`]): as
//! Cox's own says, a sketch from nothing is opened as a project of its
//! own, its folder moving into the project as a directory, or the sketch
//! is discarded, its folder with it, its agent first
//! summing it up in the conversation, and what it changed kept apart,
//! but for the files too big to keep. The chat goes on, in the project.
//!
//! However it ends, Cox keeps what the sketch did as a source of its
//! project's (see [`crate::raw`]), under [`SOURCES`]: how it ended, as
//! where its files went, all that was said in it, and the pictures and
//! recordings it showed the human, in [`MEDIA`] beside it. What is made
//! of the sketch, as a page of the wiki or the work it goes on to, names
//! that source, rather than the sketch itself becoming what is made: a
//! sketch from nothing that a project is made for keeps its files apart
//! as a discarded one does, and the project starts with none of them, to
//! make again what the sketch tried, rather than go on with its code.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use cox_git::changes;

use crate::agent::setup::Policy;
use crate::chat::judge;

/// What a sketch starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sketch {
    /// The project's last commit, in a worktree of the project's.
    Project,
    /// Nothing, in an empty folder.
    Empty,
}

impl Sketch {
    /// The name of the sketch, as the agent names it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Empty => "empty",
        }
    }

    /// The sketch `name` says, as the agent names it: `project` or
    /// `empty`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "project" => Some(Self::Project),
            "empty" => Some(Self::Empty),
            _ => None,
        }
    }

    /// What the sketch's folder is, as the agent that opened it is told.
    #[must_use]
    pub fn what(self) -> &'static str {
        match self {
            Self::Project => "a worktree of the project's last commit",
            Self::Empty => "an empty folder, a Git repository of its own",
        }
    }
}

/// `policy` with what a sketch's agent, working in `folder`, goes on with
/// unasked, on top of what the chat that opened it said: the requests
/// `patterns` match, as the human's workflow says them (see
/// [`crate::workflow::Sketch::allow`]), when they change the files in
/// `folder` only, and what a judge may run, which reads and tests.
#[must_use]
pub fn allow(policy: Policy, folder: &Path, patterns: &[String]) -> Policy {
    Policy {
        allow: policy
            .allow
            .into_iter()
            .chain(patterns.iter().cloned())
            .chain(judge::ALLOWED.iter().map(|&pattern| pattern.to_owned()))
            .collect(),
        within: Some(folder.to_owned()),
        ..policy
    }
}

/// Makes the folder of a sketch that starts from `sketch`, for a chat of
/// `project`, in `sketches`, and answers where the sketch works in it.
///
/// # Errors
///
/// When a sketch of the project's is to be made and `project` is not in a
/// Git repository, or when Git cannot make the folder, with what Git
/// said.
pub fn make(sketch: Sketch, project: &Path, sketches: &Path) -> Result<PathBuf, String> {
    let folder = match sketch {
        Sketch::Project => worktree(project, sketches)?,
        Sketch::Empty => {
            let folder = free(sketches, "scratch")?;
            git(&folder, &["init", "--quiet"])?;
            folder
        }
    };
    // What the sketch changed is what it changed since now.
    changes::start(&folder)?;
    Ok(folder)
}

/// Makes a worktree of the Git repository `project` is in, from its last
/// commit, in `sketches`, and answers where the project is in it.
fn worktree(project: &Path, sketches: &Path) -> Result<PathBuf, String> {
    let top = git(project, &["rev-parse", "--show-toplevel"])
        .map_err(|error| format!("a sketch of the project needs a Git repository: {error}"))?;
    let name = Path::new(top.trim()).file_name().map_or_else(
        || "project".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let folder = free(sketches, &name)?;
    let path = folder.to_string_lossy();
    if let Err(error) = git(project, &["worktree", "add", "--detach", &path, "HEAD"]) {
        let _ = std::fs::remove_dir(&folder);
        return Err(error);
    }
    // Where in the repository the project is, the sketch is too.
    let within = project
        .canonicalize()
        .ok()
        .and_then(|project| {
            let top = Path::new(top.trim()).canonicalize().ok()?;
            project.strip_prefix(top).ok().map(Path::to_owned)
        })
        .unwrap_or_default();
    Ok(if within.as_os_str().is_empty() {
        folder
    } else {
        folder.join(within)
    })
}

/// What Cox calls the folder, in the folder of sketches, that keeps what
/// the discarded sketches from nothing changed.
pub const DISCARDED: &str = "discarded";

/// The topic of the sources that keep what each sketch did once it
/// ended.
pub const SOURCES: &str = "sketches";

/// The folder, beside the sources of the sketches, that holds the
/// pictures and recordings they showed the human.
pub const MEDIA: &str = "media";

/// Where the sketches discarded from the project are kept in its
/// repository, each under the name of its folder: refs, which Git keeps
/// what they name for, that are not branches, so that the project's
/// branches stay the human's.
const DISCARDED_REFS: &str = "refs/cox/discarded";

/// The most a file the sketch changed can hold to be kept, in bytes.
const FILE_KEPT: u64 = 256 * 1024;

/// The most the files kept of a sketch can hold together, in bytes.
const ALL_KEPT: u64 = 1024 * 1024;

/// A file a sketch made that is too big to keep once it is discarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    /// Where it is, from the top of the sketch's worktree.
    pub path: String,
    /// How many bytes it holds.
    pub size: u64,
    /// What Git names what it holds, as `git hash-object` does.
    pub hash: String,
}

/// What of a sketch is kept once it is discarded.
#[derive(Debug, Default, PartialEq, Eq)]
struct Sorted {
    /// The files it changed that are kept, or that it took away, from the
    /// top of its worktree.
    kept: Vec<String>,
    /// The files too big to keep, by their paths.
    left_out: Vec<LeftOut>,
}

/// Sorts what the sketch whose worktree is at `top` changed since it
/// started into what is kept once it is discarded, the files of up to
/// [`FILE_KEPT`] bytes, as many as fit in [`ALL_KEPT`] together, the
/// smaller first, and what is left out, the rest.
fn sort(top: &Path) -> Result<Sorted, String> {
    let changed = git(
        top,
        &["diff", "--name-only", "-z", "--no-renames", changes::START],
    )?;
    let made = git(top, &["ls-files", "-z", "--others", "--exclude-standard"])?;
    let mut paths: Vec<&str> = changed
        .split('\0')
        .chain(made.split('\0'))
        .filter(|path| !path.is_empty())
        .collect();
    paths.sort_unstable();
    paths.dedup();
    // A file taken away holds nothing.
    let mut sized: Vec<(u64, &str)> = paths
        .into_iter()
        .map(|path| {
            let size = std::fs::symlink_metadata(top.join(path)).map_or(0, |file| file.len());
            (size, path)
        })
        .collect();
    sized.sort_unstable();
    let mut sorted = Sorted::default();
    let mut all = 0;
    for (size, path) in sized {
        if size <= FILE_KEPT && all + size <= ALL_KEPT {
            all += size;
            sorted.kept.push(path.to_owned());
        } else {
            let hash = git(top, &["hash-object", "--", path])?.trim().to_owned();
            sorted.left_out.push(LeftOut {
                path: path.to_owned(),
                size,
                hash,
            });
        }
    }
    sorted.kept.sort_unstable();
    sorted
        .left_out
        .sort_unstable_by(|one, other| one.path.cmp(&other.path));
    Ok(sorted)
}

/// The files the sketch that works in `folder`, in `sketches`, made that
/// are too big to keep once it is discarded.
///
/// # Errors
///
/// When the sketch's folder is not a Git repository of its own (see
/// [`discard`]), or Git cannot tell what the sketch changed, with why.
pub fn left_out(folder: &Path, sketches: &Path) -> Result<Vec<LeftOut>, String> {
    sort(&own_top(folder, sketches)?).map(|sorted| sorted.left_out)
}

/// The files `left_out` of a sketch, too big to keep, as its agent is
/// asked to say how to make each of them again before the sketch ends, in
/// place of `{left_out}` (see [`crate::workflow::SKETCH_DISCARD`]); or
/// nothing, when there are none.
#[must_use]
pub fn left_out_lines(left_out: &[LeftOut]) -> String {
    let mut files = String::new();
    if !left_out.is_empty() {
        files.push_str(
            "These files it made are too big for Cox to keep, so say how to make each of \
             them again:",
        );
        for file in left_out {
            let _ = write!(files, "\n- {} ({})", file.path, size(file.size));
        }
    }
    files
}

/// Where what a discarded sketch changed is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// A ref of the project's repository, for a sketch of the project's.
    Ref(String),
    /// A bundle of Git's, for a sketch from nothing, whose repository goes
    /// with its folder.
    Bundle(PathBuf),
}

impl std::fmt::Display for Kept {
    /// Where it is kept: the ref's name, or the bundle's path.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ref(name) => f.write_str(name),
            Self::Bundle(bundle) => write!(f, "{}", bundle.display()),
        }
    }
}

/// What is kept of a discarded sketch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discarded {
    /// Where what it changed is kept, if it changed anything to keep.
    pub kept: Option<Kept>,
    /// The files it made that were too big to keep.
    pub left_out: Vec<LeftOut>,
}

impl Discarded {
    /// What the sketch's chat notes of it.
    #[must_use]
    pub fn note(&self) -> String {
        format!(
            "The sketch is discarded, and its folder is gone.{}",
            self.where_kept()
        )
    }

    /// Where what the sketch changed is kept, and what was left out, as
    /// its note says it after how it ended, each a sentence after a space.
    #[must_use]
    pub fn where_kept(&self) -> String {
        let mut note = String::new();
        match &self.kept {
            Some(Kept::Ref(name)) => {
                let _ = write!(
                    note,
                    " What it changed is kept as {name} of the project's repository, which \
                     `git show {name}` shows."
                );
            }
            Some(Kept::Bundle(bundle)) => {
                let _ = write!(
                    note,
                    " What it changed is kept in {}, which `git clone` makes again.",
                    bundle.display()
                );
            }
            None => {}
        }
        if !self.left_out.is_empty() {
            let files: Vec<_> = self
                .left_out
                .iter()
                .map(|file| format!("{} ({}, {})", file.path, size(file.size), file.hash))
                .collect();
            let _ = write!(note, " Left out as too big to keep: {}.", files.join(", "));
        }
        note
    }
}

/// Discards the sketch that starts from `sketch`, named `title`, that works
/// in `folder`, in `sketches`, taking its folder away. When `keep` says to,
/// what it changed since it started is kept first, as one commit from where
/// it started, but for the files too big to keep: as a ref of the project's
/// repository for a sketch of the project's, and as a bundle in
/// [`DISCARDED`] for a sketch from nothing.
///
/// # Errors
///
/// When the folder Cox made the sketch in, in `sketches`, is not a Git
/// repository of its own; or when Git cannot keep what it changed, or the
/// folder cannot be taken away; with why.
pub fn discard(
    sketch: Sketch,
    folder: &Path,
    title: &str,
    sketches: &Path,
    keep: bool,
) -> Result<Discarded, String> {
    let top = own_top(folder, sketches)?;
    let sorted = if keep { sort(&top)? } else { Sorted::default() };
    let commit = commit_kept(&top, &sorted.kept, title)?;
    let kept = match (commit, sketch) {
        (None, _) => None,
        (Some(commit), Sketch::Project) => {
            let name = free_ref(&top)?;
            git(&top, &["update-ref", &name, &commit])?;
            Some(Kept::Ref(name))
        }
        (Some(commit), Sketch::Empty) => {
            // The repository goes with the folder, so what the bundle
            // holds can be its own: the branch `sketch`, checked out, for
            // a clone of it to check it out too.
            git(&top, &["update-ref", "refs/heads/sketch", &commit])?;
            git(&top, &["symbolic-ref", "HEAD", "refs/heads/sketch"])?;
            let bundle = free_bundle(&sketches.join(DISCARDED), &folder_name(&top))?;
            let path = bundle.to_string_lossy();
            git(
                &top,
                &[
                    "bundle",
                    "create",
                    "--quiet",
                    &path,
                    "HEAD",
                    "refs/heads/sketch",
                ],
            )
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&bundle);
            })?;
            Some(Kept::Bundle(bundle))
        }
    };
    match sketch {
        Sketch::Project => remove_worktree(&top)?,
        Sketch::Empty => std::fs::remove_dir_all(&top)
            .map_err(|error| format!("cannot remove {}: {error}", top.display()))?,
    }
    Ok(Discarded {
        kept,
        left_out: sorted.left_out,
    })
}

/// Commits the files `kept` of the sketch whose worktree is at `top`, as
/// they are now, onto where it started, as `title` says, leaving the rest
/// as they were then, and answers the commit, unless that changes
/// nothing. The commit is made apart from the worktree's own index and
/// commits, so that it holds none of what the sketch left out, even of
/// what the sketch committed itself.
fn commit_kept(top: &Path, kept: &[String], title: &str) -> Result<Option<String>, String> {
    if kept.is_empty() {
        return Ok(None);
    }
    let index = git(
        top,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "cox-discarded-index",
        ],
    )?;
    let index = PathBuf::from(index.trim());
    let committed = (|| {
        let with_index = [("GIT_INDEX_FILE", index.as_os_str())];
        let start = format!("{}^{{tree}}", changes::START);
        let from = git(top, &["rev-parse", &start])?.trim().to_owned();
        git_with(top, &["read-tree", &from], &with_index, None)?;
        let paths = kept.join("\0");
        git_with(
            top,
            &["update-index", "--add", "--remove", "-z", "--stdin"],
            &with_index,
            Some(paths.as_bytes()),
        )?;
        let tree = git_with(top, &["write-tree"], &with_index, None)?
            .trim()
            .to_owned();
        if tree == from {
            return Ok(None);
        }
        let message = format!("Discarded sketch: {title}");
        let mut args = vec!["commit-tree", &tree, "-m", &message];
        let parent = format!("{}^{{commit}}", changes::START);
        let parent = git(top, &["rev-parse", "--verify", "--quiet", &parent])
            .map(|parent| parent.trim().to_owned())
            .ok();
        if let Some(parent) = &parent {
            args.extend(["-p", parent]);
        }
        git_with(top, &args, &identity(top), None).map(|commit| Some(commit.trim().to_owned()))
    })();
    let _ = std::fs::remove_file(&index);
    committed
}

/// Who Cox commits what a discarded sketch changed as: whoever Git says
/// commits in the repository at `top`, or else, when Git knows no one, as
/// on a machine where no `user.name` is set, Cox itself, since the commit
/// is Cox's to keep and not the human's to share.
fn identity(top: &Path) -> Vec<(&'static str, &'static OsStr)> {
    let known = ["GIT_AUTHOR_IDENT", "GIT_COMMITTER_IDENT"]
        .iter()
        .all(|ident| git(top, &["var", ident]).is_ok());
    if known {
        return Vec::new();
    }
    let name = OsStr::new("Cox");
    let email = OsStr::new("cox@localhost");
    vec![
        ("GIT_AUTHOR_NAME", name),
        ("GIT_AUTHOR_EMAIL", email),
        ("GIT_COMMITTER_NAME", name),
        ("GIT_COMMITTER_EMAIL", email),
    ]
}

/// The name of the folder of the worktree at `top`, as its sketch goes by.
fn folder_name(top: &Path) -> String {
    top.file_name().map_or_else(
        || "sketch".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The first ref under [`DISCARDED_REFS`], named after the folder of the
/// worktree at `top`, that its repository does not have yet.
fn free_ref(top: &Path) -> Result<String, String> {
    let name = folder_name(top);
    (1..=u32::MAX)
        .map(|number| match number {
            1 => format!("{DISCARDED_REFS}/{name}"),
            _ => format!("{DISCARDED_REFS}/{name}-{number}"),
        })
        .find(|kept| git(top, &["rev-parse", "--verify", "--quiet", kept]).is_err())
        .ok_or_else(|| "there is no name left for the ref".to_owned())
}

/// The first bundle in `bundles` named `name` that is not there yet,
/// making `bundles` if it is not there.
fn free_bundle(bundles: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(bundles)
        .map_err(|error| format!("cannot make {}: {error}", bundles.display()))?;
    (1..=u32::MAX)
        .map(|number| match number {
            1 => bundles.join(format!("{name}.bundle")),
            _ => bundles.join(format!("{name}-{number}.bundle")),
        })
        .find(|bundle| !bundle.exists())
        .ok_or_else(|| format!("{} is full", bundles.display()))
}

/// How big `bytes` is, as people read it.
fn size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    #[expect(clippy::cast_precision_loss, reason = "a size shown to one decimal")]
    match bytes {
        0..KB => format!("{bytes} bytes"),
        KB..MB => format!("{} KB", bytes / KB),
        _ => format!("{:.1} MB", bytes as f64 / MB as f64),
    }
}

/// What a sketch the human starts for a chat working in `folder` starts
/// from at first: the project, when `folder` is in a Git repository,
/// which a sketch of the project's needs, or else nothing.
#[must_use]
pub fn from_at_first(folder: &Path) -> Sketch {
    if top(folder).is_ok() {
        Sketch::Project
    } else {
        Sketch::Empty
    }
}

/// The top of the worktree `folder` is in.
fn top(folder: &Path) -> Result<PathBuf, String> {
    Ok(PathBuf::from(
        git(folder, &["rev-parse", "--show-toplevel"])?.trim(),
    ))
}

/// The top of the worktree of the sketch that works in `folder`, in
/// `sketches`: the folder Cox made it in there, as long as Git finds that
/// folder the top of a worktree, and not a repository around it.
fn own_top(folder: &Path, sketches: &Path) -> Result<PathBuf, String> {
    let canonical = |path: &Path| {
        path.canonicalize()
            .map_err(|error| format!("cannot find {}: {error}", path.display()))
    };
    let sketches = canonical(sketches)?;
    let folder = canonical(folder)?;
    let own = folder
        .strip_prefix(&sketches)
        .ok()
        .and_then(|within| within.components().next())
        .map(|name| sketches.join(name))
        .ok_or_else(|| {
            format!(
                "{} is not a sketch's, in {}",
                folder.display(),
                sketches.display()
            )
        })?;
    let mine = |why: String| {
        format!(
            "the sketch's folder {} is not a Git repository of its own: {why}",
            own.display()
        )
    };
    let top = top(&folder).map_err(mine)?;
    if canonical(&top)? == own {
        Ok(own)
    } else {
        Err(mine(format!("Git finds {}", top.display())))
    }
}

/// Takes away the worktree at `top`, with what is in it, from the
/// repository it is a worktree of.
fn remove_worktree(top: &Path) -> Result<(), String> {
    // Git takes a worktree away from another one, not from itself.
    let common = git(
        top,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let path = top.to_string_lossy();
    git(
        Path::new(common.trim()),
        &["worktree", "remove", "--force", &path],
    )
    .map(drop)
}

/// Tells Git that the repository of the project in `folder` moved there,
/// for each of `sketches`, the folders of sketches from the project,
/// worktrees of the repository that would not find it otherwise.
pub fn repair(folder: &Path, sketches: &[PathBuf]) {
    for works in sketches {
        // A sketch of a project in a folder of its repository works in
        // that folder of the worktree.
        let Some(top) = works.ancestors().find(|path| path.join(".git").exists()) else {
            continue;
        };
        let top = top.to_string_lossy();
        if let Err(error) = git(folder, &["worktree", "repair", &top]) {
            tracing::error!("the sketch in {top} does not find its repository: {error}");
        }
    }
}

/// Makes the first folder in `sketches` named `name`, a dash and a number
/// that is not there yet, making `sketches` if it is not there, and
/// answers where it is. Making it claims it, so two sketches made at once
/// get folders of their own.
fn free(sketches: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(sketches)
        .map_err(|error| format!("cannot make {}: {error}", sketches.display()))?;
    for number in 1..=u32::MAX {
        let folder = sketches.join(format!("{name}-{number}"));
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("cannot make {}: {error}", folder.display())),
        }
    }
    Err(format!("{} is full", sketches.display()))
}

/// Runs Git in `folder` with `args`, and answers what it printed, or
/// what it said went wrong.
fn git(folder: &Path, args: &[&str]) -> Result<String, String> {
    cox_git::git::raw(folder, args)
}

/// Runs Git in `folder` with `args`, with `env` added to its environment
/// and `input` on its standard input, if given, and answers what it
/// printed, or what it said went wrong.
fn git_with(
    folder: &Path,
    args: &[&str],
    env: &[(&str, &OsStr)],
    input: Option<&[u8]>,
) -> Result<String, String> {
    let mut child = cox_git::git::command()
        .arg("-C")
        .arg(folder)
        .args(args)
        .envs(env.iter().copied())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(input)
            .map_err(|error| format!("cannot write to git: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use cox_acp::ToolKind;

    use super::*;
    use crate::agent::setup::{Asked, Decision};
    use crate::testing::TempDir;
    use crate::testing::run_git as git;

    #[test]
    fn a_sketch_works_in_a_worktree_of_the_last_commit() {
        let root = TempDir::new();
        let project = root.join("greet");
        std::fs::create_dir_all(project.join("app")).unwrap();
        let run = |args: &[&str]| git(&project, args).unwrap();
        run(&["init", "-q"]);
        std::fs::write(project.join("app/main.rs"), "fn main() {}\n").unwrap();
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "first",
        ]);
        // Not committed, so not in the sketch.
        std::fs::write(project.join("app/main.rs"), "fn main() { todo!() }\n").unwrap();

        let sketches = root.join("sketches");
        let sketch = make(Sketch::Project, &project.join("app"), &sketches).unwrap();
        assert_eq!(sketch, sketches.join("greet-1/app"));
        assert_eq!(
            std::fs::read_to_string(sketch.join("main.rs")).unwrap(),
            "fn main() {}\n"
        );
        std::fs::write(sketch.join("main.rs"), "fn main() { sketch() }\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(project.join("app/main.rs")).unwrap(),
            "fn main() { todo!() }\n"
        );
        // What it changed is what it changed since it started.
        let diff = changes::read(&sketch).unwrap();
        assert!(matches!(diff.since, changes::Since::Start(_)));
        assert_eq!(diff.counts(), (1, 1));
        // Each sketch gets a worktree of its own.
        assert_eq!(
            make(Sketch::Project, &project, &sketches).unwrap(),
            sketches.join("greet-2")
        );

        let error = make(
            Sketch::Project,
            &root.join("sketches"),
            &root.join("elsewhere"),
        );
        assert!(error.is_err_and(|error| error.starts_with("a sketch of the project needs")));

        // One in a folder of the repository goes with its whole worktree.
        discard(Sketch::Project, &sketch, "App", &sketches, true).unwrap();
        assert!(!sketches.join("greet-1").exists());
    }

    #[test]
    fn a_sketch_finds_its_repository_again_once_told_it_moved() {
        let root = TempDir::new();
        let project = root.join("greet");
        std::fs::create_dir_all(project.join("app")).unwrap();
        let run = |args: &[&str]| git(&project, args).unwrap();
        run(&["init", "-q"]);
        std::fs::write(project.join("app/main.rs"), "fn main() {}\n").unwrap();
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "first",
        ]);
        let sketches = root.join("sketches");
        let sketch = make(Sketch::Project, &project.join("app"), &sketches).unwrap();

        let moved = root.join("greet-2");
        std::fs::rename(&project, &moved).unwrap();
        assert!(git(&sketch, &["status"]).is_err());
        repair(&moved.join("app"), std::slice::from_ref(&sketch));
        assert!(git(&sketch, &["status"]).is_ok());
        assert!(changes::read(&sketch).is_ok());
    }

    #[test]
    fn a_sketch_from_nothing_works_in_an_empty_repository() {
        let root = TempDir::new();
        // The project need not be in a Git repository.
        let project = root.join("notes");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("todo.md"), "- greet\n").unwrap();

        let sketches = root.join("sketches");
        let sketch = make(Sketch::Empty, &project, &sketches).unwrap();
        assert_eq!(sketch, sketches.join("scratch-1"));
        let files: Vec<_> = std::fs::read_dir(&sketch)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(files, [".git"]);
        assert_eq!(
            changes::read(&sketch).unwrap().since,
            changes::Since::Nothing
        );
        assert_eq!(
            git(&sketch, &["rev-parse", "--show-toplevel"])
                .unwrap()
                .trim(),
            sketch.to_string_lossy()
        );
        assert_eq!(
            make(Sketch::Empty, &project, &sketches).unwrap(),
            sketches.join("scratch-2")
        );
    }

    /// A repository at `project`, with `main.rs` committed.
    fn repository(project: &Path) {
        std::fs::create_dir_all(project).unwrap();
        let run = |args: &[&str]| git(project, args).unwrap();
        run(&["init", "-q"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "user.email", "t@t"]);
        std::fs::write(project.join("main.rs"), "fn main() {}\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "first"]);
    }

    #[test]
    fn a_sketch_is_discarded_keeping_what_it_changed() {
        let root = TempDir::new();
        let project = root.join("greet");
        repository(&project);
        let bundles = root.join("sketches/discarded");
        let sketches = root.join("sketches");
        let sketch = make(Sketch::Project, &project, &sketches).unwrap();
        std::fs::write(sketch.join("main.rs"), "fn main() { sketch() }\n").unwrap();
        std::fs::write(sketch.join("new.rs"), "fn sketch() {}\n").unwrap();
        // A file too big to keep, which the sketch's agent committed too.
        let big = vec![b'x'; 300 * 1024];
        std::fs::write(sketch.join("big.bin"), &big).unwrap();
        git(&sketch, &["add", "--all"]).unwrap();
        git(&sketch, &["commit", "-q", "-m", "the agent's own"]).unwrap();
        let big_hash = git(&sketch, &["hash-object", "big.bin"]).unwrap();
        // Files that fit alone, but not all together.
        for name in ["a", "b", "c", "d", "e"] {
            std::fs::write(sketch.join(format!("{name}.txt")), vec![b'y'; 250 * 1024]).unwrap();
        }
        // What Git ignores is not the sketch's to keep.
        std::fs::write(sketch.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(sketch.join("target")).unwrap();
        std::fs::write(sketch.join("target/out"), "built\n").unwrap();

        let left_out = left_out(&sketch, &sketches).unwrap();
        let paths: Vec<_> = left_out.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["big.bin", "e.txt"]);
        assert_eq!(left_out[0].size, 300 * 1024);
        assert_eq!(left_out[0].hash, big_hash.trim());
        let lines = left_out_lines(&left_out);
        assert!(lines.contains("- big.bin (300 KB)"), "{lines}");
        assert_eq!(left_out_lines(&[]), "");

        let discarded =
            discard(Sketch::Project, &sketch, "Try a greeting", &sketches, true).unwrap();
        assert!(!sketch.exists());
        let name = "refs/cox/discarded/greet-1";
        assert_eq!(discarded.kept, Some(Kept::Ref(name.to_owned())));
        assert_eq!(discarded.left_out, left_out);
        let note = discarded.note();
        assert!(note.contains(&format!("`git show {name}`")), "{note}");
        assert!(note.contains("big.bin (300 KB, "), "{note}");
        let shown = |what: &str| git(&project, &["show", what]);
        assert_eq!(
            shown(&format!("{name}:main.rs")).unwrap(),
            "fn main() { sketch() }\n"
        );
        assert_eq!(
            shown(&format!("{name}:new.rs")).unwrap(),
            "fn sketch() {}\n"
        );
        assert!(shown(&format!("{name}:d.txt")).is_ok());
        assert!(shown(&format!("{name}:e.txt")).is_err());
        assert!(shown(&format!("{name}:big.bin")).is_err());
        assert!(shown(&format!("{name}:target/out")).is_err());
        // One commit on the project's, which holds nothing it left out.
        assert_eq!(
            git(&project, &["rev-parse", &format!("{name}^")]).unwrap(),
            git(&project, &["rev-parse", "HEAD"]).unwrap()
        );
        assert_eq!(
            git(&project, &["log", "-1", "--format=%s", name])
                .unwrap()
                .trim(),
            "Discarded sketch: Try a greeting"
        );
        let objects = git(&project, &["rev-list", "--objects", name]).unwrap();
        assert!(!objects.contains(big_hash.trim()), "{objects}");
        // No branch, and no worktree, is left.
        assert_eq!(
            git(&project, &["branch", "--list", "sketch/*"]).unwrap(),
            ""
        );
        let worktrees = git(&project, &["worktree", "list"]).unwrap();
        assert_eq!(worktrees.lines().count(), 1, "{worktrees}");

        // A sketch that took a file away keeps that it did, under a name of
        // its own when the first is taken.
        let again = make(Sketch::Project, &project, &sketches).unwrap();
        std::fs::remove_file(again.join("main.rs")).unwrap();
        let discarded = discard(Sketch::Project, &again, "Again", &sketches, true).unwrap();
        let name = "refs/cox/discarded/greet-1-2";
        assert_eq!(discarded.kept, Some(Kept::Ref(name.to_owned())));
        assert!(shown(&format!("{name}:main.rs")).is_err());

        // A sketch that changed nothing keeps nothing.
        let unchanged = make(Sketch::Project, &project, &sketches).unwrap();
        let discarded = discard(Sketch::Project, &unchanged, "Nothing", &sketches, true).unwrap();
        assert_eq!(discarded.kept, None);
        assert_eq!(
            discarded.note(),
            "The sketch is discarded, and its folder is gone."
        );

        // One whose ending keeps nothing goes as it is.
        let unkept = make(Sketch::Project, &project, &sketches).unwrap();
        std::fs::write(unkept.join("main.rs"), "fn main() { unkept() }\n").unwrap();
        let discarded = discard(Sketch::Project, &unkept, "Unkept", &sketches, false).unwrap();
        assert!(!unkept.exists());
        assert_eq!(discarded.kept, None);

        // A sketch from nothing is kept as a bundle, which makes it again.
        let empty = make(Sketch::Empty, &project, &sketches).unwrap();
        std::fs::write(empty.join("main.rs"), "fn main() {}\n").unwrap();
        let discarded = discard(Sketch::Empty, &empty, "From nothing", &sketches, true).unwrap();
        assert!(!empty.exists());
        let bundle = bundles.join("scratch-1.bundle");
        assert_eq!(discarded.kept, Some(Kept::Bundle(bundle.clone())));
        let again = root.join("again");
        let path = bundle.to_string_lossy();
        git(&root, &["clone", "-q", &path, &again.to_string_lossy()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(again.join("main.rs")).unwrap(),
            "fn main() {}\n"
        );
    }

    #[test]
    fn a_sketch_whose_repository_is_gone_leaves_the_one_around_it_be() {
        let root = TempDir::new();
        let outer = root.join("dotfiles");
        repository(&outer);
        let sketches = outer.join("cox/sketches");
        let bundles = sketches.join(DISCARDED);
        let empty = make(Sketch::Empty, &root, &sketches).unwrap();
        std::fs::write(empty.join("notes.md"), "- greet\n").unwrap();
        std::fs::remove_dir_all(empty.join(".git")).unwrap();
        let head = git(&outer, &["rev-parse", "HEAD"]).unwrap();

        for keep in [false, true] {
            let error = discard(Sketch::Empty, &empty, "Gone", &sketches, keep).unwrap_err();
            assert!(error.contains("not a Git repository of its own"), "{error}");
            assert!(left_out(&empty, &sketches).is_err());
        }
        assert_eq!(
            std::fs::read_to_string(outer.join("main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert_eq!(git(&outer, &["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(git(&outer, &["for-each-ref"]).unwrap().lines().count(), 1);
        assert!(empty.join("notes.md").exists());
        assert!(!bundles.exists());
    }

    #[test]
    fn a_size_reads_as_people_read_it() {
        assert_eq!(size(12), "12 bytes");
        assert_eq!(size(300 * 1024), "300 KB");
        assert_eq!(size(3 * 1024 * 1024 + 512 * 1024), "3.5 MB");
    }

    #[test]
    fn a_sketch_is_named_by_what_it_starts_from() {
        assert_eq!(Sketch::parse("project"), Some(Sketch::Project));
        assert_eq!(Sketch::parse("empty"), Some(Sketch::Empty));
        assert_eq!(Sketch::parse("container"), None);
    }

    #[test]
    fn a_sketch_changes_its_files_unasked() {
        let opened = Policy {
            allow: vec!["fetch:*".to_owned()],
            reject: vec!["execute:git push*".to_owned()],
            within: None,
        };
        let own = crate::workflow::sketch_in(None);
        let policy = allow(opened, Path::new("/sketches/scratch-1"), &own.allow);
        let decide = |title, kind, location: &str| {
            policy.decide(Asked {
                title,
                kind: Some(kind),
                locations: &[PathBuf::from(location)],
            })
        };
        assert_eq!(
            decide(
                "Write notes.md",
                ToolKind::Edit,
                "/sketches/scratch-1/notes.md"
            ),
            Some(Decision::Allow)
        );
        assert_eq!(
            decide("Write notes.md", ToolKind::Edit, "/home/me/notes.md"),
            None
        );
        assert_eq!(
            decide("cargo test", ToolKind::Execute, "/sketches/scratch-1"),
            Some(Decision::Allow)
        );
        // What the chat that opened it said stays.
        assert_eq!(
            decide("Fetch https://example.com", ToolKind::Fetch, "/"),
            Some(Decision::Allow)
        );
        assert_eq!(
            decide("git push", ToolKind::Execute, "/sketches/scratch-1"),
            Some(Decision::Reject)
        );
    }
}
