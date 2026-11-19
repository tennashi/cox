//! The worktrees of a project's repositories: the other checkouts of a
//! directory's repository, each on a branch of its own, as Git lists
//! them, wherever they are. A worktree goes with the repository it is
//! one of, rather than being a directory of the project of its own, so
//! that a project of three repositories lists three, each with its
//! worktrees under it.
//!
//! Cox makes a worktree in [`WORKTREES`] in the project's folder, in a
//! folder of the directory's name, for the chat that asked for it (see
//! [`make`]), and marks it as its own there: a file in the worktree's own
//! folder of Git's, which Git leaves be and moves with the worktree, so
//! that one moved with `git worktree move` is still Cox's, and one made
//! by hand never is. Once the chat is done with it, and it holds nothing
//! that is not elsewhere, Cox takes it away (see [`tidy`]); the rest the
//! human sees listed, each with why it is left, to take away themselves.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::DIRECTORIES;

/// The folder of the worktrees Cox makes, in a project's own.
pub const WORKTREES: &str = "worktrees";

/// The file, in a worktree's own folder of Git's, that marks it as one
/// Cox made.
const MARK: &str = "cox-worktree.json";

/// A worktree of a repository, other than its main one, as Git lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    /// Where it is.
    pub path: PathBuf,
    /// The branch it has checked out, by its short name, or none when its
    /// head is detached.
    pub branch: Option<String>,
    /// The commit its head is at.
    pub head: String,
    /// The chat Cox made it for, by the number the chat is kept by, when
    /// Cox made it.
    pub made_for: Option<i64>,
    /// Whether it is outside the project's folder.
    pub outside: bool,
    /// Whether its folder is gone, so that Git would prune it.
    pub gone: bool,
}

impl Worktree {
    /// Whether Cox made it.
    #[must_use]
    pub fn made_by_cox(&self) -> bool {
        self.made_for.is_some()
    }

    /// What it goes by: its branch, or the commit its detached head is
    /// at.
    #[must_use]
    pub fn label(&self) -> String {
        self.branch.clone().unwrap_or_else(|| {
            let short: String = self.head.chars().take(7).collect();
            format!("detached at {short}")
        })
    }
}

/// What Cox marks a worktree it made with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Mark {
    /// The chat it was made for, by the number the chat is kept by.
    chat: i64,
}

/// How what a worktree holds stands against the rest of its repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// Its head is in the branch its repository's work goes into.
    Merged,
    /// Its head is not merged, but a remote has it.
    Pushed,
    /// It has this many commits that are nowhere else.
    Only(usize),
}

/// How a worktree stands now: what it changed and did not commit, and
/// where its commits are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Standing {
    /// How many files it changed, or holds that Git does not track and
    /// does not ignore.
    pub changed: usize,
    /// Where its commits are.
    pub kept: Kept,
}

impl Standing {
    /// What it says of the worktree, in a few words.
    #[must_use]
    pub fn said(self) -> String {
        let changed = match self.changed {
            0 => "clean".to_owned(),
            1 => "1 file changed".to_owned(),
            changed => format!("{changed} files changed"),
        };
        let kept = match self.kept {
            Kept::Merged => "merged".to_owned(),
            Kept::Pushed => "pushed".to_owned(),
            Kept::Only(1) => "1 commit only here".to_owned(),
            Kept::Only(commits) => format!("{commits} commits only here"),
        };
        format!("{changed}, {kept}")
    }
}

/// Whether `folder` is the main worktree of a repository, or in no
/// repository: not a worktree of another.
#[must_use]
pub fn is_main(folder: &Path) -> bool {
    let Ok(dirs) = git(
        folder,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    ) else {
        return true;
    };
    let mut dirs = dirs.lines();
    dirs.next() == dirs.next()
}

/// The worktrees of the repository whose main worktree is at
/// `repository`, but for that one, as Git lists them, with whether each
/// is outside the project in `project`.
#[must_use]
pub fn of(repository: &Path, project: &Path) -> Vec<Worktree> {
    let Ok(listed) = git(repository, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let project = canonical(project);
    let common = git(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(PathBuf::from)
    .ok();
    listed
        .split("\n\n")
        .skip(1)
        .filter_map(|entry| {
            let mut path = None;
            let mut head = String::new();
            let mut branch = None;
            let mut gone = false;
            for line in entry.lines() {
                let (key, value) = line.split_once(' ').unwrap_or((line, ""));
                match key {
                    "worktree" => path = Some(PathBuf::from(value)),
                    "HEAD" => value.clone_into(&mut head),
                    "branch" => {
                        branch = Some(
                            value
                                .strip_prefix("refs/heads/")
                                .unwrap_or(value)
                                .to_owned(),
                        );
                    }
                    "prunable" => gone = true,
                    _ => {}
                }
            }
            let path = path?;
            let made_for = common
                .as_deref()
                .and_then(|common| own_folder(common, &path))
                .and_then(|own| std::fs::read(own.join(MARK)).ok())
                .and_then(|mark| serde_json::from_slice::<Mark>(&mark).ok())
                .map(|mark| mark.chat);
            Some(Worktree {
                outside: !canonical(&path).starts_with(&project),
                path,
                branch,
                head,
                made_for,
                gone,
            })
        })
        .collect()
}

/// The folder of Git's own of the worktree at `path`, of the repository
/// whose Git folder is `common`: the one in its `worktrees` whose
/// `gitdir` names the worktree's `.git`.
fn own_folder(common: &Path, path: &Path) -> Option<PathBuf> {
    let path = canonical(path);
    std::fs::read_dir(common.join("worktrees"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|own| {
            std::fs::read_to_string(own.join("gitdir")).is_ok_and(|gitdir| {
                Path::new(gitdir.trim())
                    .parent()
                    .is_some_and(|folder| canonical(folder) == path)
            })
        })
}

/// How the worktree at `path`, of the repository whose main worktree is
/// at `repository`, stands now.
///
/// # Errors
///
/// When Git cannot tell, as when the worktree's folder is gone, with what
/// Git said.
pub fn standing(repository: &Path, path: &Path) -> Result<Standing, String> {
    let changed = git(path, &["status", "--porcelain"])?.lines().count();
    let base = base_of(repository);
    let head = git(path, &["rev-parse", "HEAD"])?;
    let merged = base
        .as_deref()
        .is_some_and(|base| git(path, &["merge-base", "--is-ancestor", &head, base]).is_ok());
    let kept = if merged {
        Kept::Merged
    } else {
        let mut args = vec!["rev-list", "--count", head.as_str(), "--not", "--remotes"];
        args.extend(base.as_deref());
        let only: usize = git(path, &args)?.parse().unwrap_or(0);
        if only == 0 {
            Kept::Pushed
        } else {
            Kept::Only(only)
        }
    };
    Ok(Standing { changed, kept })
}

/// The commit the work of the repository whose main worktree is at
/// `repository` goes into: its remote's default branch, or else the
/// branch its main worktree has checked out.
fn base_of(repository: &Path) -> Option<String> {
    git(
        repository,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "refs/remotes/origin/HEAD",
        ],
    )
    .or_else(|_| git(repository, &["rev-parse", "--verify", "--quiet", "HEAD"]))
    .ok()
}

/// Runs Git in `folder` with `args`, asking nothing.
fn git(folder: &Path, args: &[impl AsRef<OsStr>]) -> Result<String, String> {
    cox_git::git::run(folder, args)
}

/// `path` as the file system resolves it, or as it is when it is not
/// there.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

/// The folder of the directory `directory` of the project in `project`.
#[must_use]
pub fn directory_of(project: &Path, directory: &str) -> PathBuf {
    project.join(DIRECTORIES).join(directory)
}

/// What a worktree is to be made of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// The directory of the project whose repository it is of.
    pub directory: String,
    /// The branch it checks out, made from `from` unless there is one of
    /// that name already, or none, for a head detached at `from`.
    pub branch: Option<String>,
    /// The commit it starts from, as Git names one, or else the remote's
    /// default branch, or the main worktree's head.
    pub from: Option<String>,
}

/// Makes a worktree as `asked` says, of a directory of the project in
/// `project`, for the chat kept as `chat`, in [`WORKTREES`] in the
/// project's folder, in a folder of the directory's name, named after
/// its branch, and answers where it is.
///
/// # Errors
///
/// When the project has no such directory, or it is no repository, or
/// Git cannot make the worktree, with why.
pub fn make(project: &Path, asked: &Asked, chat: Option<i64>) -> Result<PathBuf, String> {
    let name = asked.directory.trim();
    let repository = directory_of(project, name);
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(format!(
            "`{name}` is not the name of a directory of the project"
        ));
    }
    if !repository.join(".git").exists() {
        let names: Vec<String> = repositories(project)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        return Err(format!(
            "the project has no repository {name} in its directories; it has {}",
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            }
        ));
    }
    if !is_main(&repository) {
        return Err(format!(
            "{name} is a worktree itself: make one of the repository it is of"
        ));
    }
    let from = match asked.from.as_deref().map(str::trim) {
        Some(from) if !from.is_empty() => from.to_owned(),
        _ => base_of(&repository).ok_or("the repository has no commit to start from")?,
    };
    let commit = git(
        &repository,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{from}^{{commit}}"),
        ],
    )
    .map_err(|_| format!("the repository has no commit {from}"))?;
    let branch = asked
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty());
    let folder_name = match branch {
        Some(branch) => {
            git(&repository, &["check-ref-format", "--branch", branch])
                .map_err(|_| format!("`{branch}` is not a name Git takes for a branch"))?;
            branch.replace('/', "-")
        }
        None => format!("detached-{}", commit.chars().take(7).collect::<String>()),
    };
    let folder = project.join(WORKTREES).join(name).join(&folder_name);
    if folder.exists() {
        return Err(format!(
            "there is a worktree at {} already: work there, or name another branch",
            folder.display()
        ));
    }
    std::fs::create_dir_all(folder.parent().unwrap_or(project))
        .map_err(|error| format!("the folder of the worktrees is not made: {error}"))?;
    let path = folder.to_string_lossy().into_owned();
    let exists = branch.is_some_and(|branch| {
        git(
            &repository,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .is_ok()
    });
    let args: Vec<&str> = match branch {
        Some(branch) if exists => vec!["worktree", "add", "--quiet", &path, branch],
        Some(branch) => vec!["worktree", "add", "--quiet", "-b", branch, &path, &commit],
        None => vec!["worktree", "add", "--quiet", "--detach", &path, &commit],
    };
    git(&repository, &args)?;
    if let Some(chat) = chat {
        mark(&repository, &folder, chat)?;
    }
    Ok(folder)
}

/// Marks the worktree at `folder`, which Cox made, as the chat's kept as
/// `chat`, as once that chat opened for it.
///
/// # Errors
///
/// When `folder` is no worktree, or the mark cannot be written.
pub fn claim(folder: &Path, chat: i64) -> Result<(), String> {
    mark(folder, folder, chat)
}

/// Marks the worktree at `folder`, of the repository whose main worktree
/// is at `repository`, as one Cox made for the chat kept as `chat`.
fn mark(repository: &Path, folder: &Path, chat: i64) -> Result<(), String> {
    let common = git(
        repository,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let own = own_folder(Path::new(&common), folder)
        .ok_or("Git keeps no folder of the worktree's own")?;
    let mark = serde_json::to_vec(&Mark { chat }).map_err(|error| error.to_string())?;
    std::fs::write(own.join(MARK), mark)
        .map_err(|error| format!("the worktree is not marked as Cox's: {error}"))
}

/// The directories of the project in `project` that are repositories, but
/// for worktrees of another, by name, with their folders.
#[must_use]
pub fn repositories(project: &Path) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(project.join(DIRECTORIES))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join(".git").exists() && is_main(&entry.path()))
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .collect();
    found.sort();
    found
}

/// Why Cox leaves `worktree`, which stands as `standing` says, be, rather
/// than take it away by itself, each in a few words, or nothing when it
/// may: it made it, in the project, for a chat that is done, as `busy`
/// tells of the chat it was made for, and it holds nothing that is not
/// elsewhere, no change left uncommitted, and its commits merged or
/// pushed.
#[must_use]
pub fn why_left(
    worktree: &Worktree,
    standing: Option<&Result<Standing, String>>,
    busy: impl Fn(i64) -> bool,
) -> Vec<String> {
    let mut why = Vec::new();
    match worktree.made_for {
        None => why.push("made by hand".to_owned()),
        Some(chat) if busy(chat) => why.push(format!("chat {chat} is not done")),
        Some(_) => {}
    }
    if worktree.outside {
        why.push("outside the project".to_owned());
    }
    if worktree.gone {
        why.push("its folder is gone".to_owned());
        return why;
    }
    match standing {
        None => why.push("not read yet".to_owned()),
        Some(Err(_)) => why.push("Git cannot tell how it stands".to_owned()),
        Some(Ok(standing)) => {
            match standing.changed {
                0 => {}
                1 => why.push("1 file changed".to_owned()),
                changed => why.push(format!("{changed} files changed")),
            }
            match standing.kept {
                Kept::Merged | Kept::Pushed => {}
                Kept::Only(1) => why.push("1 commit only here".to_owned()),
                Kept::Only(commits) => why.push(format!("{commits} commits only here")),
            }
        }
    }
    why
}

/// Takes away the worktree at `path` of the repository whose main
/// worktree is at `repository`, as the human or Cox does: its folder,
/// and, once merged, its branch, unless it holds what would be lost, and
/// answers what was taken away. One whose folder is gone, Git forgets.
///
/// # Errors
///
/// When it has changes not committed, or its detached head commits that
/// are nowhere else, or Git cannot take it away, with why.
pub fn remove(repository: &Path, worktree: &Worktree) -> Result<String, String> {
    if worktree.gone {
        git(repository, &["worktree", "prune"])?;
        return Ok(format!(
            "Git forgot the worktree at {}, whose folder was gone.",
            worktree.path.display()
        ));
    }
    let standing = standing(repository, &worktree.path)?;
    if standing.changed > 0 {
        return Err(format!(
            "The worktree at {} has {} not committed: commit or drop them first.",
            worktree.path.display(),
            match standing.changed {
                1 => "1 file changed".to_owned(),
                changed => format!("{changed} files changed"),
            }
        ));
    }
    if let (None, Kept::Only(commits)) = (&worktree.branch, standing.kept) {
        return Err(format!(
            "The worktree at {} has its head detached at {commits} commits that are nowhere \
             else, which would be lost: put them on a branch first.",
            worktree.path.display()
        ));
    }
    let path = worktree.path.to_string_lossy();
    git(repository, &["worktree", "remove", &path])?;
    let mut said = format!("Took away the worktree at {}", worktree.path.display());
    match (&worktree.branch, standing.kept) {
        (Some(branch), Kept::Merged) => match git(repository, &["branch", "-d", branch]) {
            Ok(_) => {
                let _ = write!(said, ", and its branch {branch}, merged");
            }
            Err(why) => {
                let _ = write!(said, "; its branch {branch} is kept: {why}");
            }
        },
        (Some(branch), _) => {
            let _ = write!(said, "; its branch {branch} is kept, not merged yet");
        }
        (None, _) => {}
    }
    // A folder of the worktrees left empty goes too.
    if let Some(parent) = worktree.path.parent()
        && parent.parent().and_then(Path::file_name) == Some(OsStr::new(WORKTREES))
    {
        let _ = std::fs::remove_dir(parent);
    }
    said.push('.');
    Ok(said)
}

/// A worktree Cox took away by itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Removed {
    /// The chat it was made for, by the number the chat is kept by.
    pub chat: i64,
    /// What was taken away.
    pub said: String,
}

/// Takes away each worktree of the repositories of the project in
/// `project` that Cox may take away by itself (see [`why_left`]), as
/// `busy` tells of a chat, by the number it is kept by, and of a folder a
/// chat works in, and answers those taken away.
pub fn tidy(project: &Path, busy: &Busy) -> Vec<Removed> {
    let mut removed = Vec::new();
    for (_, repository) in repositories(project) {
        for worktree in of(&repository, project) {
            let Some(chat) = worktree.made_for else {
                continue;
            };
            if worktree.gone || busy.works_in(&worktree.path) {
                continue;
            }
            let standing = standing(&repository, &worktree.path);
            if !why_left(&worktree, Some(&standing), |chat| {
                busy.chats.contains(&chat)
            })
            .is_empty()
            {
                continue;
            }
            match remove(&repository, &worktree) {
                Ok(said) => removed.push(Removed { chat, said }),
                Err(why) => tracing::warn!(
                    path = %worktree.path.display(),
                    "the worktree is not taken away: {why}"
                ),
            }
        }
    }
    removed
}

/// What Cox's chats that are not done hold on to, as [`tidy`] leaves be.
#[derive(Clone, Debug, Default)]
pub struct Busy {
    /// The chats not done, by the numbers they are kept by.
    pub chats: Vec<i64>,
    /// The folders they work in.
    pub folders: Vec<PathBuf>,
}

impl Busy {
    /// Whether a chat not done works in the worktree at `path`, or in a
    /// folder in it.
    fn works_in(&self, path: &Path) -> bool {
        let path = canonical(path);
        self.folders
            .iter()
            .any(|folder| canonical(folder).starts_with(&path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, run_git};

    /// A repository at `folder` with one commit, on `main`.
    pub(super) fn repository(folder: &Path) {
        std::fs::create_dir_all(folder).unwrap();
        run_git(folder, &["init", "--quiet", "-b", "main"]).unwrap();
        std::fs::write(folder.join("a.txt"), "a\n").unwrap();
        run_git(folder, &["add", "a.txt"]).unwrap();
        run_git(folder, &["commit", "--quiet", "-m", "first"]).unwrap();
    }

    /// Commits a new file `name` in the worktree at `folder`.
    pub(super) fn commit(folder: &Path, name: &str) {
        std::fs::write(folder.join(name), "x\n").unwrap();
        run_git(folder, &["add", name]).unwrap();
        run_git(folder, &["commit", "--quiet", "-m", name]).unwrap();
    }

    #[test]
    fn a_repository_lists_its_worktrees_but_its_main_one() {
        let project = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        let side = directory_of(&project, "app-side");
        let path = side.to_string_lossy();
        run_git(&main, &["worktree", "add", "--quiet", "-b", "side", &path]).unwrap();

        let listed = of(&main, &project);

        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(canonical(&listed[0].path), side);
        assert_eq!(listed[0].branch.as_deref(), Some("side"));
        assert!(!listed[0].outside);
        assert!(!listed[0].made_by_cox());
        assert!(is_main(&main));
        assert!(!is_main(&side));
    }

    #[test]
    fn a_worktree_outside_the_project_is_marked_so() {
        let project = TempDir::new();
        let elsewhere = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        let path = elsewhere.join("side");
        let path = path.to_string_lossy();
        run_git(&main, &["worktree", "add", "--quiet", "--detach", &path]).unwrap();

        let listed = of(&main, &project);

        assert!(listed[0].outside);
        assert_eq!(listed[0].branch, None);
        assert!(listed[0].label().starts_with("detached at "));
    }

    #[test]
    fn a_worktree_stands_as_what_it_changed_and_where_its_commits_are() {
        let project = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        let side = project.join("side");
        let path = side.to_string_lossy();
        run_git(&main, &["worktree", "add", "--quiet", "-b", "side", &path]).unwrap();

        assert_eq!(
            standing(&main, &side),
            Ok(Standing {
                changed: 0,
                kept: Kept::Merged
            })
        );

        commit(&side, "b.txt");
        std::fs::write(side.join("c.txt"), "c\n").unwrap();
        assert_eq!(
            standing(&main, &side),
            Ok(Standing {
                changed: 1,
                kept: Kept::Only(1)
            })
        );

        run_git(&main, &["merge", "--quiet", "side"]).unwrap();
        std::fs::remove_file(side.join("c.txt")).unwrap();
        assert_eq!(
            standing(&main, &side),
            Ok(Standing {
                changed: 0,
                kept: Kept::Merged
            })
        );
    }

    #[test]
    fn a_worktree_a_remote_has_is_pushed() {
        let project = TempDir::new();
        let remote = TempDir::new();
        run_git(&remote, &["init", "--quiet", "--bare"]).unwrap();
        let main = directory_of(&project, "app");
        repository(&main);
        run_git(
            &main,
            &["remote", "add", "origin", &remote.to_string_lossy()],
        )
        .unwrap();
        run_git(&main, &["push", "--quiet", "origin", "main"]).unwrap();
        run_git(&main, &["remote", "set-head", "origin", "main"]).unwrap();
        let side = project.join("side");
        let path = side.to_string_lossy();
        run_git(&main, &["worktree", "add", "--quiet", "-b", "side", &path]).unwrap();
        commit(&side, "b.txt");
        run_git(&side, &["push", "--quiet", "origin", "side"]).unwrap();

        assert_eq!(standing(&main, &side).map(|s| s.kept), Ok(Kept::Pushed));
    }

    #[test]
    fn cox_makes_a_worktree_in_the_projects_worktrees_marked_as_its_own() {
        let project = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        let asked = Asked {
            directory: "app".to_owned(),
            branch: Some("fix/login".to_owned()),
            from: None,
        };

        let made = make(&project, &asked, Some(42)).unwrap();

        assert_eq!(made, project.join(WORKTREES).join("app").join("fix-login"));
        let listed = of(&main, &project);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].branch.as_deref(), Some("fix/login"));
        assert_eq!(listed[0].made_for, Some(42));
        assert!(make(&project, &asked, Some(42)).is_err(), "made once only");
    }

    #[test]
    fn a_worktree_cox_made_stays_its_own_once_moved_with_git() {
        let project = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        let asked = Asked {
            directory: "app".to_owned(),
            branch: Some("fix".to_owned()),
            from: None,
        };
        let made = make(&project, &asked, Some(7)).unwrap();
        let moved = project.join("moved");

        run_git(
            &main,
            &[
                "worktree",
                "move",
                &made.to_string_lossy(),
                &moved.to_string_lossy(),
            ],
        )
        .unwrap();

        let listed = of(&main, &project);
        assert_eq!(canonical(&listed[0].path), moved);
        assert_eq!(listed[0].made_for, Some(7));
    }

    #[test]
    fn a_worktree_is_made_detached_without_a_branch_and_not_of_what_is_no_repository() {
        let project = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        std::fs::create_dir_all(directory_of(&project, "notes")).unwrap();

        let made = make(
            &project,
            &Asked {
                directory: "app".to_owned(),
                ..Asked::default()
            },
            None,
        )
        .unwrap();
        assert!(
            made.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("detached-")
        );
        assert_eq!(of(&main, &project)[0].made_for, None);

        let refused = make(
            &project,
            &Asked {
                directory: "notes".to_owned(),
                ..Asked::default()
            },
            None,
        );
        assert_eq!(
            refused,
            Err("the project has no repository notes in its directories; it has app".to_owned())
        );
    }

    /// A worktree Cox made of `app` in `project` for the chat `chat`, on
    /// the branch `branch`.
    fn made(project: &Path, branch: &str, chat: i64) -> PathBuf {
        let asked = Asked {
            directory: "app".to_owned(),
            branch: Some(branch.to_owned()),
            from: None,
        };
        make(project, &asked, Some(chat)).unwrap()
    }

    #[test]
    fn cox_takes_away_a_worktree_of_a_chat_done_that_holds_nothing_not_elsewhere() {
        let project = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        // Merged: taken away, its branch too.
        let merged = made(&project, "merged", 1);
        commit(&merged, "m.txt");
        run_git(&main, &["merge", "--quiet", "merged"]).unwrap();
        // Its chat is not done.
        let busy_one = made(&project, "busy", 2);
        // Changes left, and commits only there.
        let dirty = made(&project, "dirty", 3);
        std::fs::write(dirty.join("d.txt"), "d\n").unwrap();
        let ahead = made(&project, "ahead", 4);
        commit(&ahead, "a.txt");
        // Made by hand.
        let by_hand = project.join("by-hand");
        run_git(
            &main,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "hand",
                &by_hand.to_string_lossy(),
            ],
        )
        .unwrap();
        // A chat not done works in it.
        let worked_in = made(&project, "worked-in", 5);

        let busy = Busy {
            chats: vec![2],
            folders: vec![worked_in.join("src")],
        };
        let removed = tidy(&project, &busy);

        assert_eq!(
            removed,
            [Removed {
                chat: 1,
                said: format!(
                    "Took away the worktree at {}, and its branch merged, merged.",
                    merged.display()
                ),
            }]
        );
        assert!(!merged.exists());
        assert!(
            run_git(
                &main,
                &["rev-parse", "--verify", "--quiet", "refs/heads/merged"]
            )
            .is_err()
        );
        for left in [&busy_one, &dirty, &ahead, &by_hand, &worked_in] {
            assert!(left.exists(), "{}", left.display());
        }
        let listed = of(&main, &project);
        let why = |path: &Path| {
            let worktree = listed.iter().find(|w| canonical(&w.path) == *path).unwrap();
            let standing = standing(&main, path);
            why_left(worktree, Some(&standing), |chat| chat == 2)
        };
        assert_eq!(why(&busy_one), ["chat 2 is not done"]);
        assert_eq!(why(&dirty), ["1 file changed"]);
        assert_eq!(why(&ahead), ["1 commit only here"]);
        assert_eq!(why(&by_hand), ["made by hand"]);
    }

    #[test]
    fn the_human_takes_away_a_worktree_keeping_a_branch_not_merged() {
        let project = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        let ahead = made(&project, "ahead", 1);
        commit(&ahead, "a.txt");
        let worktree = of(&main, &project).remove(0);

        let said = remove(&main, &worktree).unwrap();

        assert!(
            said.ends_with("; its branch ahead is kept, not merged yet."),
            "{said}"
        );
        assert!(!ahead.exists());
        assert!(
            !project.join(WORKTREES).join("app").exists(),
            "its empty folder goes too"
        );
        assert!(
            run_git(
                &main,
                &["rev-parse", "--verify", "--quiet", "refs/heads/ahead"]
            )
            .is_ok()
        );
    }

    #[test]
    fn a_worktree_is_not_taken_away_with_what_would_be_lost() {
        let project = TempDir::new();
        let main = directory_of(&project, "app");
        repository(&main);
        let dirty = made(&project, "dirty", 1);
        std::fs::write(dirty.join("d.txt"), "d\n").unwrap();
        let detached = make(
            &project,
            &Asked {
                directory: "app".to_owned(),
                ..Asked::default()
            },
            Some(2),
        )
        .unwrap();
        commit(&detached, "x.txt");

        for worktree in of(&main, &project) {
            assert!(remove(&main, &worktree).is_err(), "{worktree:?}");
            assert!(worktree.path.exists());
        }
    }
}
