//! The repositories of a folder Cox works in, and what changed in them,
//! as Git tells it (see [`cox_git::changes`]). A project's folder is
//! not a repository itself: its repositories are its directories (see
//! [`DIRECTORIES`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cox_git::changes::{self, Diff, How};

use crate::settings::DIRECTORIES;

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

/// What changed in one of the repositories a folder's changes are of (see
/// [`of`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    /// The name of the project's directory it is, or none for the
    /// repository the folder is in.
    pub name: Option<String>,
    /// What changed in it, or why it could not be read.
    pub read: Result<Diff, String>,
}

/// What changed in each of the repositories of `folder` (see
/// [`of`]).
#[must_use]
pub fn read(folder: &Path) -> Vec<Repository> {
    of(folder)
        .into_iter()
        .map(|(name, folder)| Repository {
            name,
            read: changes::read(&folder),
        })
        .collect()
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
    use coxd::testing::{TempDir, run_git};

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
}
