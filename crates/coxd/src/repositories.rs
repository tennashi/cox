//! The repositories of a folder Cox works in, and what changed in them,
//! as Git tells it (see [`cox_git::changes`]). A project's folder is
//! not a repository itself: its repositories are its directories (see
//! [`DIRECTORIES`]).
//!
//! What changed is read in one checkout of each repository at a time:
//! its own, or another of its worktrees, each on a branch of its own (see
//! [`crate::worktrees`]). A worktree goes with the repository it is one
//! of, wherever it is, rather than being a repository of its own, even
//! one among the directories, as the project's card lists them. A branch
//! no worktree has checked out is seen too, by what it committed, from
//! the repository's own checkout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cox_git::changes::{self, Diff, How};

use crate::settings::DIRECTORIES;
use crate::worktrees::{self, Worktree};

/// The most branches no worktree has checked out that a repository lists
/// as checkouts, the newest.
const BRANCHES: usize = 20;

/// The repositories whose changes are those of `folder`: the directories
/// of the project in `folder` that are repositories, by name, after
/// `folder` itself if it is one too; or else, for a folder with no such
/// directory, the repository `folder` is in, if it is in one.
#[must_use]
pub fn of(folder: &Path) -> Vec<(Option<String>, PathBuf)> {
    let mut directories: Vec<(Option<String>, PathBuf)> =
        std::fs::read_dir(folder.join(DIRECTORIES))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join(".git").exists())
            .filter_map(|entry| Some((Some(entry.file_name().into_string().ok()?), entry.path())))
            .collect();
    directories.sort();
    if directories.is_empty() || folder.join(".git").exists() {
        directories.insert(0, (None, folder.to_owned()));
    }
    directories
}

/// A checkout of a repository, where what changed in it is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checkout {
    /// The repository's own, at `folder`, on `branch`, by its short name,
    /// or none when its head is detached, at the commit `head`, by its full
    /// name, if it has one.
    Own {
        folder: PathBuf,
        branch: Option<String>,
        head: Option<String>,
    },
    /// Another worktree of the repository.
    Worktree(Worktree),
    /// A branch of the repository no worktree has checked out, by its
    /// short name, seen from its own checkout, at `folder`.
    Branch { folder: PathBuf, name: String },
}

impl Checkout {
    /// Where it is.
    #[must_use]
    pub fn folder(&self) -> &Path {
        match self {
            Self::Own { folder, .. } | Self::Branch { folder, .. } => folder,
            Self::Worktree(worktree) => &worktree.path,
        }
    }

    /// What it goes by: its branch, or the commit its detached head is
    /// at.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Own {
                branch: Some(branch),
                ..
            } => branch.clone(),
            Self::Own { head, .. } => head.as_deref().map_or_else(
                || "detached".to_owned(),
                |head| format!("detached at {}", head.chars().take(7).collect::<String>()),
            ),
            Self::Worktree(worktree) => worktree.label(),
            Self::Branch { name, .. } => name.clone(),
        }
    }

    /// The branch it is, when no worktree has it checked out.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Branch { name, .. } => Some(name),
            _ => None,
        }
    }
}

/// The checkouts of the repository at `folder`: its own, then, when it is
/// a repository's main worktree, its other worktrees, as Git lists them,
/// but for those whose folders are gone, those outside `within` last, and
/// its branches no worktree has checked out, the newest first, as many as
/// [`BRANCHES`].
#[must_use]
pub fn checkouts(folder: &Path, within: &Path) -> Vec<Checkout> {
    let run = |args: &[&str]| cox_git::git::run(folder, args).ok();
    let branch = run(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let main = worktrees::is_main(folder);
    let mut others = if main {
        worktrees::of(folder, within)
    } else {
        Vec::new()
    };
    others.retain(|worktree| !worktree.gone);
    others.sort_by_key(|worktree| worktree.outside);
    let checked_out: Vec<&str> = branch
        .iter()
        .chain(
            others
                .iter()
                .filter_map(|worktree| worktree.branch.as_ref()),
        )
        .map(String::as_str)
        .collect();
    let branches: Vec<Checkout> = if main {
        run(&[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ])
        .unwrap_or_default()
        .lines()
        .filter(|name| !name.is_empty() && !checked_out.contains(name))
        .take(BRANCHES)
        .map(|name| Checkout::Branch {
            folder: folder.to_owned(),
            name: name.to_owned(),
        })
        .collect()
    } else {
        Vec::new()
    };
    let own = Checkout::Own {
        folder: folder.to_owned(),
        branch,
        head: run(&["rev-parse", "--verify", "--quiet", "HEAD"]),
    };
    std::iter::once(own)
        .chain(others.into_iter().map(Checkout::Worktree))
        .chain(branches)
        .collect()
}

/// The repositories whose changes are those of `folder`, as [`of`] finds
/// them, each by name with its checkouts (see [`checkouts`]), but for a
/// directory that is a worktree of another of them, which is a checkout
/// of that one.
#[must_use]
pub fn listed(folder: &Path) -> Vec<(Option<String>, Vec<Checkout>)> {
    let found: Vec<(Option<String>, Vec<Checkout>)> = of(folder)
        .into_iter()
        .map(|(name, repository)| (name, checkouts(&repository, folder)))
        .collect();
    let under_another: Vec<PathBuf> = found
        .iter()
        .flat_map(|(_, checkouts)| checkouts)
        .filter_map(|checkout| match checkout {
            Checkout::Worktree(worktree) => Some(canonical(&worktree.path)),
            _ => None,
        })
        .collect();
    found
        .into_iter()
        .filter(|(_, checkouts)| {
            checkouts
                .first()
                .is_some_and(|own| !under_another.contains(&canonical(own.folder())))
        })
        .collect()
}

/// What changed in one of the repositories a folder's changes are of (see
/// [`listed`]), in one of its checkouts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    /// The name of the project's directory it is, or none for the
    /// repository the folder is in.
    pub name: Option<String>,
    /// Its checkouts, its own first (see [`checkouts`]).
    pub checkouts: Vec<Checkout>,
    /// Which of them what changed was read in.
    pub shown: usize,
    /// What changed in it, or why it could not be read.
    pub read: Result<Diff, String>,
}

impl Repository {
    /// The checkout what changed was read in.
    #[must_use]
    pub fn checkout(&self) -> Option<&Checkout> {
        self.checkouts.get(self.shown)
    }
}

/// The checkout of a repository chosen to read what changed in: by the
/// name of the project's directory it is, or none for the repository the
/// folder is in, the folder of the checkout.
pub type Chosen = (Option<String>, PathBuf);

/// A branch of a repository no worktree has checked out, chosen to read
/// what it committed: by the name of the project's directory it is, or
/// none for the repository the folder is in, the branch's short name.
pub type ChosenBranch = (Option<String>, String);

/// What changed in each of the repositories of `folder` (see
/// [`listed`]): what the branch of it `branches` names committed since it
/// left the branch `compared` gives for the repository's name, as the
/// human chose it, or else the one Cox finds; or else what changed in the
/// checkout of it `chosen` names, or else in the one `cwd` is, the folder
/// a chat works in, or else in its own.
#[must_use]
pub fn read(
    folder: &Path,
    chosen: &[Chosen],
    branches: &[ChosenBranch],
    cwd: &Path,
    compared: &dyn Fn(Option<&str>) -> Option<String>,
) -> Vec<Repository> {
    let cwd = canonical(cwd);
    listed(folder)
        .into_iter()
        .map(|(name, checkouts)| {
            let at = |wanted: &Path| {
                let wanted = canonical(wanted);
                checkouts
                    .iter()
                    .position(|checkout| canonical(checkout.folder()) == wanted)
            };
            let branch =
                branches
                    .iter()
                    .find(|(named, _)| *named == name)
                    .and_then(|(_, branch)| {
                        checkouts
                            .iter()
                            .position(|checkout| checkout.branch() == Some(branch))
                    });
            let shown = branch
                .or_else(|| {
                    chosen
                        .iter()
                        .find(|(named, _)| *named == name)
                        .and_then(|(_, folder)| at(folder))
                })
                .or_else(|| at(&cwd))
                .unwrap_or(0);
            let read = match checkouts.get(shown) {
                None => Err("no checkout".to_owned()),
                Some(Checkout::Branch {
                    folder,
                    name: branch,
                }) => {
                    let base = compared(name.as_deref());
                    cox_git::commits::everything_of(folder, Some(branch), base.as_deref()).and_then(
                        |read| read.ok_or_else(|| format!("nothing tells where {branch} started")),
                    )
                }
                Some(checkout) => changes::read(checkout.folder()),
            };
            Repository {
                name,
                checkouts,
                shown,
                read,
            }
        })
        .collect()
}

/// `path` as the file system resolves it, or as it is when it cannot.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

/// How many files changed in the repositories of `folder` in all (see
/// [`listed`]), as [`changes::count`] counts them, each in the checkout
/// of it [`read`] reads, as `chosen` and `cwd` choose it, and in no other:
/// what its other worktrees changed does not count; or, for one seen on a
/// branch no checkout has, as `branches` chooses, the files that branch
/// committed since it left the one `compared` gives for the repository's
/// name. Which checkout that is, Git tells from the folder each worktree
/// shares, without listing the worktrees, as [`listed`] does, since the
/// bar counts again every few seconds, but for a repository seen on such
/// a branch.
///
/// # Errors
///
/// When Git can tell what changed in none of them, as when `folder` is in
/// no repository and has no directory that is one, with what Git said.
pub fn count(
    folder: &Path,
    chosen: &[Chosen],
    branches: &[ChosenBranch],
    cwd: &Path,
    compared: &dyn Fn(Option<&str>) -> Option<String>,
) -> Result<usize, String> {
    let found: Vec<(Option<String>, PathBuf, Option<Shared>)> = of(folder)
        .into_iter()
        .map(|(name, folder)| {
            let shared = checkout_at(&folder);
            (name, folder, shared)
        })
        .collect();
    let mains: Vec<&Path> = found
        .iter()
        .filter_map(|(_, _, shared)| shared.as_ref())
        .filter(|shared| shared.main)
        .map(|shared| shared.common.as_path())
        .collect();
    let cwd = checkout_at(cwd).filter(|shared| shared.top == canonical(cwd));
    let mut counted = None;
    let mut failed = None;
    for (name, own, shared) in &found {
        // A worktree of another of them counts as its checkout, if chosen.
        if let Some(shared) = shared
            && !shared.main
            && mains.contains(&shared.common.as_path())
        {
            continue;
        }
        let of_it = |checkout: &Shared| {
            shared
                .as_ref()
                .is_some_and(|shared| shared.common == checkout.common)
        };
        let branch = branches
            .iter()
            .find(|(named, _)| named == name)
            .map(|(_, branch)| branch.as_str())
            .filter(|branch| no_checkout_has(own, branch));
        let checkout = chosen
            .iter()
            .find(|(named, _)| named == name)
            .and_then(|(_, folder)| checkout_at(folder))
            .filter(of_it)
            .or_else(|| cwd.clone().filter(of_it))
            .map_or_else(|| own.clone(), |checkout| checkout.top);
        let count = match branch {
            Some(branch) => {
                let base = compared(name.as_deref());
                cox_git::commits::count_of(own, branch, base.as_deref())
            }
            None => changes::count(&checkout),
        };
        match count {
            Ok(count) => *counted.get_or_insert(0) += count,
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
    }
    counted.ok_or_else(|| failed.unwrap_or_default())
}

/// Whether the repository whose main worktree is at `repository` has a
/// branch `branch` that none of its worktrees has checked out (see
/// [`checkouts`]).
fn no_checkout_has(repository: &Path, branch: &str) -> bool {
    let run = |args: &[&str]| cox_git::git::run(repository, args);
    let exists = run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}"),
    ]);
    let listed = run(&["worktree", "list", "--porcelain"]).unwrap_or_default();
    let checked_out = format!("branch refs/heads/{branch}");
    exists.is_ok() && !listed.lines().any(|line| line == checked_out)
}

/// The checkout a folder is the top of, as Git tells it.
#[derive(Clone)]
struct Shared {
    /// The top of the checkout, as the file system resolves it.
    top: PathBuf,
    /// The folder of Git's its repository's worktrees share.
    common: PathBuf,
    /// Whether it is its repository's main worktree.
    main: bool,
}

/// The project's folder and the name of its directory the repository of
/// `folder` named `name` (see [`Repository::name`]) is, as a project in
/// `projects`, by their folders, has it: the directory `name` of the
/// project in `folder`; or, for none, '' of the project `folder` is, as a
/// repository itself, or else the directory of a project whose worktree
/// `folder` is. None when it is no project's.
#[must_use]
pub fn directory_of(
    projects: &[PathBuf],
    folder: &Path,
    name: Option<&str>,
) -> Option<(PathBuf, String)> {
    let project = |folder: &Path| {
        let folder = canonical(folder);
        projects
            .iter()
            .find(|project| canonical(project) == folder)
            .cloned()
    };
    if let Some(name) = name {
        return project(folder).map(|project| (project, name.to_owned()));
    }
    if let Some(project) = project(folder) {
        return Some((project, String::new()));
    }
    let main = checkout_at(folder)?.common.parent()?.to_owned();
    let directories = main.parent()?;
    if directories.file_name()? != DIRECTORIES {
        return None;
    }
    let name = main.file_name()?.to_str()?.to_owned();
    project(directories.parent()?).map(|project| (project, name))
}

/// The checkout `folder` is in, if it is in one.
fn checkout_at(folder: &Path) -> Option<Shared> {
    let told = cox_git::git::run(
        folder,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--git-dir",
            "--git-common-dir",
        ],
    )
    .ok()?;
    let mut lines = told.lines().map(|line| canonical(Path::new(line)));
    let (top, own, common) = (lines.next()?, lines.next()?, lines.next()?);
    Some(Shared {
        top,
        main: own == common,
        common,
    })
}

/// How each file that changed in the repositories of `folder` changed,
/// by its absolute path (see [`of`]), as [`changes::hows`]
/// tells it; nothing of a repository Git can tell nothing of.
#[must_use]
pub fn hows(folder: &Path) -> HashMap<PathBuf, How> {
    of(folder)
        .into_iter()
        .filter_map(|(_, folder)| changes::hows(&folder).ok())
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, run_git};

    /// A Git repository at `folder` with nothing committed and one file
    /// Git does not track yet, `new.txt`.
    fn repository_with_a_new_file(folder: &Path) {
        std::fs::create_dir_all(folder).unwrap();
        run_git(folder, &["init", "--quiet"]).unwrap();
        std::fs::write(folder.join("new.txt"), "new\n").unwrap();
    }

    #[test]
    fn a_projects_repositories_are_its_directories_that_are_ones() {
        let project = TempDir::new();
        repository_with_a_new_file(&project.join(DIRECTORIES).join("app"));
        std::fs::create_dir_all(project.join(DIRECTORIES).join("notes")).unwrap();

        let found = of(&project);

        assert_eq!(
            found,
            [(
                Some("app".to_owned()),
                project.join(DIRECTORIES).join("app")
            )]
        );
    }

    #[test]
    fn a_project_whose_folder_is_a_repository_lists_it_first() {
        let project = TempDir::new();
        repository_with_a_new_file(&project);
        repository_with_a_new_file(&project.join(DIRECTORIES).join("app"));

        let found = of(&project);

        assert_eq!(
            found,
            [
                (None, project.to_path_buf()),
                (
                    Some("app".to_owned()),
                    project.join(DIRECTORIES).join("app")
                ),
            ]
        );
    }

    #[test]
    fn the_files_changed_in_each_repository_of_a_project_add_up() {
        let project = TempDir::new();
        repository_with_a_new_file(&project.join(DIRECTORIES).join("app"));
        repository_with_a_new_file(&project.join(DIRECTORIES).join("lib"));

        assert_eq!(count(&project, &[], &[], &project, &|_| None), Ok(2));
    }

    #[test]
    fn a_folder_in_no_repository_has_no_count_of_what_changed() {
        assert!(count(Path::new("/"), &[], &[], Path::new("/"), &|_| None).is_err());
    }

    #[test]
    fn each_file_changed_is_told_by_its_absolute_path() {
        let folder = TempDir::new();
        run_git(&folder, &["init", "--quiet"]).unwrap();
        std::fs::write(folder.join("kept.txt"), "one\n").unwrap();
        run_git(&folder, &["add", "kept.txt"]).unwrap();
        run_git(&folder, &["commit", "--quiet", "-m", "first"]).unwrap();
        std::fs::write(folder.join("kept.txt"), "two\n").unwrap();
        std::fs::write(folder.join("new.txt"), "new\n").unwrap();

        let told = hows(&folder);

        assert_eq!(
            told,
            HashMap::from([
                (folder.join("kept.txt"), How::Changed),
                (folder.join("new.txt"), How::Added),
            ])
        );
    }

    #[test]
    fn a_folder_in_no_repository_has_no_file_changed() {
        assert_eq!(hows(Path::new("/")), HashMap::new());
    }

    /// A repository at `folder`, on `main`, with a commit of `a.txt`.
    fn committed(folder: &Path) {
        std::fs::create_dir_all(folder).unwrap();
        run_git(folder, &["init", "--quiet", "-b", "main"]).unwrap();
        std::fs::write(folder.join("a.txt"), "a\n").unwrap();
        run_git(folder, &["add", "a.txt"]).unwrap();
        run_git(folder, &["commit", "--quiet", "-m", "first"]).unwrap();
    }

    /// A worktree of the repository at `repository`, at `folder`, on a
    /// new branch `branch`.
    fn worktree(repository: &Path, folder: &Path, branch: &str) {
        let path = folder.to_string_lossy();
        run_git(
            repository,
            &["worktree", "add", "--quiet", "-b", branch, &path],
        )
        .unwrap();
    }

    #[test]
    fn a_worktree_is_a_checkout_of_its_repository_even_among_the_directories() {
        let project = TempDir::new();
        let app = project.join(DIRECTORIES).join("app");
        committed(&app);
        let hotfix = project.join(DIRECTORIES).join("app-hotfix");
        worktree(&app, &hotfix, "hotfix");
        let search = project.join("worktrees/app/search");
        worktree(&app, &search, "search");
        committed(&project.join(DIRECTORIES).join("lib"));

        let listed = listed(&project);

        let names: Vec<_> = listed
            .iter()
            .map(|(name, checkouts)| {
                let labels: Vec<String> = checkouts.iter().map(Checkout::label).collect();
                (name.clone(), labels)
            })
            .collect();
        assert_eq!(
            names,
            [
                (
                    Some("app".to_owned()),
                    vec!["main".to_owned(), "hotfix".to_owned(), "search".to_owned()]
                ),
                (Some("lib".to_owned()), vec!["main".to_owned()]),
            ]
        );
        // As a chat works in the folder, every repository is still one.
        assert_eq!(of(&project).len(), 3);
    }

    #[test]
    fn what_changed_is_read_in_the_checkout_chosen_or_else_the_chats() {
        let project = TempDir::new();
        let app = project.join(DIRECTORIES).join("app");
        committed(&app);
        let search = project.join("worktrees/app/search");
        worktree(&app, &search, "search");
        std::fs::write(app.join("own.txt"), "own\n").unwrap();
        std::fs::write(search.join("search.txt"), "search\n").unwrap();
        let paths = |read: &[Repository]| -> Vec<(usize, Vec<PathBuf>)> {
            read.iter()
                .map(|repository| {
                    let files = repository.read.as_ref().unwrap().files.iter();
                    (
                        repository.shown,
                        files.map(|file| file.path.clone()).collect(),
                    )
                })
                .collect()
        };

        let own = read(&project, &[], &[], &project, &|_| None);
        let chats = read(&project, &[], &[], &search, &|_| None);
        let chosen = read(
            &project,
            &[(Some("app".to_owned()), app.clone())],
            &[],
            &search,
            &|_| None,
        );

        assert_eq!(paths(&own), [(0, vec![PathBuf::from("own.txt")])]);
        assert_eq!(paths(&chats), [(1, vec![PathBuf::from("search.txt")])]);
        assert_eq!(
            chats[0].checkout().map(Checkout::label).as_deref(),
            Some("search")
        );
        assert_eq!(paths(&chosen), [(0, vec![PathBuf::from("own.txt")])]);
    }

    #[test]
    fn a_branch_no_worktree_has_is_read_by_what_it_committed() {
        let project = TempDir::new();
        let app = project.join(DIRECTORIES).join("app");
        committed(&app);
        run_git(&app, &["checkout", "--quiet", "-b", "side"]).unwrap();
        std::fs::write(app.join("side.txt"), "side\n").unwrap();
        run_git(&app, &["add", "side.txt"]).unwrap();
        run_git(&app, &["commit", "--quiet", "-m", "side"]).unwrap();
        run_git(&app, &["checkout", "--quiet", "main"]).unwrap();
        worktree(&app, &project.join("worktrees/app/search"), "search");
        std::fs::write(app.join("own.txt"), "own\n").unwrap();

        let labels: Vec<String> = checkouts(&app, &project)
            .iter()
            .map(Checkout::label)
            .collect();
        // Compared with `main`, which it left, as the human chose.
        let read = read(
            &project,
            &[],
            &[(Some("app".to_owned()), "side".to_owned())],
            &project,
            &|_| Some("main".to_owned()),
        );

        // `main` and `search` are checked out; `side` is not.
        assert_eq!(labels, ["main", "search", "side"]);
        assert_eq!(read[0].checkout().and_then(Checkout::branch), Some("side"));
        let paths: Vec<_> = read[0]
            .read
            .as_ref()
            .unwrap()
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        assert_eq!(paths, [PathBuf::from("side.txt")]);
        // The bar counts what it committed, not what its own checkout
        // changed.
        let side = [(Some("app".to_owned()), "side".to_owned())];
        let main = |_: Option<&str>| Some("main".to_owned());
        assert_eq!(count(&project, &[], &side, &project, &main), Ok(1));
        assert_eq!(count(&project, &[], &[], &project, &main), Ok(1));
        // A branch a worktree has checked out counts as none chosen.
        let search = [(Some("app".to_owned()), "search".to_owned())];
        std::fs::write(app.join("more.txt"), "more\n").unwrap();
        assert_eq!(count(&project, &[], &search, &project, &main), Ok(2));
    }

    #[test]
    fn only_the_checkout_read_counts() {
        let project = TempDir::new();
        let app = project.join(DIRECTORIES).join("app");
        committed(&app);
        let hotfix = project.join(DIRECTORIES).join("app-hotfix");
        worktree(&app, &hotfix, "hotfix");
        let search = project.join("worktrees/app/search");
        worktree(&app, &search, "search");
        std::fs::write(app.join("own.txt"), "own\n").unwrap();
        std::fs::write(hotfix.join("h.txt"), "h\n").unwrap();
        for name in ["s.txt", "t.txt"] {
            std::fs::write(search.join(name), "s\n").unwrap();
        }
        let chosen = |folder: &Path| [(Some("app".to_owned()), folder.to_owned())];

        // Its own checkout, not the worktree among the directories.
        assert_eq!(count(&project, &[], &[], &project, &|_| None), Ok(1));
        // The one the chat works in, or the one chosen.
        assert_eq!(count(&project, &[], &[], &search, &|_| None), Ok(2));
        assert_eq!(
            count(&project, &chosen(&hotfix), &[], &search, &|_| None),
            Ok(1)
        );
        assert_eq!(
            count(&project, &chosen(&search), &[], &project, &|_| None),
            Ok(2)
        );
        // A checkout of no repository of the folder's counts as none chosen.
        let elsewhere = TempDir::new();
        committed(&elsewhere);
        std::fs::write(elsewhere.join("e.txt"), "e\n").unwrap();
        assert_eq!(
            count(&project, &chosen(&elsewhere), &[], &project, &|_| None),
            Ok(1)
        );
        // Nor does a checkout that is gone.
        std::fs::remove_dir_all(&search).unwrap();
        assert_eq!(
            count(&project, &chosen(&search), &[], &project, &|_| None),
            Ok(1)
        );
    }

    #[test]
    fn a_repository_is_kept_as_a_directory_of_its_project_wherever_its_checkout_is() {
        let project = TempDir::new();
        let app = project.join(DIRECTORIES).join("app");
        committed(&app);
        let search = project.join("worktrees/app/search");
        worktree(&app, &search, "search");
        let elsewhere = TempDir::new();
        committed(&elsewhere);
        let projects = [project.to_path_buf()];
        let kept = |folder: &Path, name: Option<&str>| directory_of(&projects, folder, name);

        let as_app = Some((project.to_path_buf(), "app".to_owned()));
        assert_eq!(kept(&project, Some("app")), as_app);
        // A chat working in the worktree has it as the folder's own.
        assert_eq!(kept(&search, None), as_app);
        assert_eq!(kept(&elsewhere, None), None);
        assert_eq!(kept(&elsewhere, Some("app")), None);
    }

    #[test]
    fn a_detached_checkout_goes_by_its_commit() {
        let folder = TempDir::new();
        committed(&folder);
        run_git(&folder, &["checkout", "--quiet", "--detach"]).unwrap();

        let checkouts = checkouts(&folder, &folder);

        assert!(checkouts[0].label().starts_with("detached at "));
        // Its branch is no checkout's now.
        assert_eq!(checkouts[1].branch(), Some("main"));
    }
}
