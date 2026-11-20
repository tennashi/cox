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
/// [`listed`]): what the branch of it `branches` names committed, or else
/// what changed in the checkout of it `chosen` names, or else in the one
/// `cwd` is, the folder a chat works in, or else in its own.
#[must_use]
pub fn read(
    folder: &Path,
    chosen: &[Chosen],
    branches: &[ChosenBranch],
    cwd: &Path,
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
                Some(Checkout::Branch { folder, name }) => {
                    cox_git::commits::everything_of(folder, Some(name)).and_then(|read| {
                        read.ok_or_else(|| format!("nothing tells where {name} started"))
                    })
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
/// [`of`]), as [`changes::count`] counts them.
///
/// # Errors
///
/// When Git can tell what changed in none of them, as when `folder` is in
/// no repository and has no directory that is one, with what Git said.
pub fn count(folder: &Path) -> Result<usize, String> {
    let mut counted = None;
    let mut failed = None;
    for (_, folder) in of(folder) {
        match changes::count(&folder) {
            Ok(count) => *counted.get_or_insert(0) += count,
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
    }
    counted.ok_or_else(|| failed.unwrap_or_default())
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

        assert_eq!(count(&project), Ok(2));
    }

    #[test]
    fn a_folder_in_no_repository_has_no_count_of_what_changed() {
        assert!(count(Path::new("/")).is_err());
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

        let own = read(&project, &[], &[], &project);
        let chats = read(&project, &[], &[], &search);
        let chosen = read(
            &project,
            &[(Some("app".to_owned()), app.clone())],
            &[],
            &search,
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
        // Compared with `main`, which it left.
        run_git(&app, &["config", cox_git::commits::BASE, "main"]).unwrap();
        worktree(&app, &project.join("worktrees/app/search"), "search");
        std::fs::write(app.join("own.txt"), "own\n").unwrap();

        let labels: Vec<String> = checkouts(&app, &project)
            .iter()
            .map(Checkout::label)
            .collect();
        let read = read(
            &project,
            &[],
            &[(Some("app".to_owned()), "side".to_owned())],
            &project,
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
