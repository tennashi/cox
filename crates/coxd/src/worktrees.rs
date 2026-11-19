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
//! that is not elsewhere, Cox takes it away (see [`removable`]).

use std::ffi::OsStr;
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
}
