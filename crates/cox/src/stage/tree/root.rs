//! What the tree of files shows the files of, its root: a folder on
//! disk, or a branch of a repository that no checkout has, whose files
//! are read from Git's objects, at the commit the branch is at (see
//! [`cox_git::tree`]), and never from the disk, where the repository's
//! own checkout has another branch's.

use std::path::{Path, PathBuf};

use cox_core::tabs::OnBranch;
use cox_editor::files::{self, Listed};
use cox_git::tree::Kind;

/// What the tree shows the files of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Root {
    /// A folder on disk, by its path as the file system resolves it.
    Folder(PathBuf),
    /// A branch no checkout has, its files named as if the repository's
    /// own checkout had them.
    Branch(OnBranch),
}

impl Root {
    /// The folder its files are named under: the folder, or the
    /// repository's own checkout.
    #[must_use]
    pub fn top(&self) -> &Path {
        match self {
            Self::Folder(folder) => folder,
            Self::Branch(on) => &on.repository,
        }
    }

    /// The branch it is, if it is one.
    #[must_use]
    pub fn branch(&self) -> Option<&OnBranch> {
        match self {
            Self::Folder(_) => None,
            Self::Branch(on) => Some(on),
        }
    }

    /// What the tree's heading names it by: the folder's name, or the
    /// branch's.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Folder(folder) => crate::stage::chooser::name_of(folder),
            Self::Branch(on) => on.branch.clone(),
        }
    }
}

/// The commit the branch of `root` is at, if it is a branch that is
/// still there; slow, as it runs Git.
pub(super) fn commit_of(root: &Root) -> Option<String> {
    let on = root.branch()?;
    cox_git::tree::commit_of(&on.repository, &on.branch)
        .ok()
        .flatten()
}

/// What is in `folder`, under the top of `root`, in the order a folder on
/// disk lists it: from disk for a folder, or from the commit `at` of a
/// branch, a submodule as a folder whose files cannot be read; or why it
/// cannot be listed. Slow, as it reads the disk or runs Git.
pub(super) fn list(root: &Root, at: Option<&str>, folder: &Path) -> Result<Vec<Listed>, String> {
    let Root::Branch(on) = root else {
        return files::list(folder).map_err(|error| error.to_string());
    };
    let commit = at.ok_or_else(|| format!("the branch {} is gone", on.branch))?;
    let within = folder
        .strip_prefix(&on.repository)
        .map_err(|_| format!("{} is not in the repository", folder.display()))?;
    let mut listed: Vec<Listed> = cox_git::tree::list(&on.repository, commit, within)?
        .into_iter()
        .map(|entry| Listed {
            path: folder.join(&entry.name),
            is_folder: entry.kind != Kind::File,
            name: entry.name,
        })
        .collect();
    listed.sort_by(files::order);
    Ok(listed)
}
