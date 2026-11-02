//! What Cox keeps of a chat's stage: the tabs it had open, in their
//! order, and which was shown. Not what runs in them: a page is kept by its
//! address, a file by its path, the tree of the project's files by the
//! folders open in it, what changed in them as such, the map of the
//! project by where it is open, and a terminal only as having been there.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A stage's tabs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptTabs {
    pub tabs: Vec<KeptTab>,
    /// The index of the tab shown.
    pub shown: usize,
}

/// One tab, by what it shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeptTab {
    /// A page of the browser.
    Page { url: String, title: String },
    /// A Markdown file, by its absolute path.
    Markdown { path: PathBuf },
    /// A terminal, with the title it last had.
    Terminal { title: String },
    /// A file shown as it is written, such as code, by its absolute path,
    /// with the lines it was shown at, if any.
    File { path: PathBuf, lines: Option<Lines> },
    /// The project's files as a tree, with the folders open in it, by
    /// their absolute paths.
    Files { open: Vec<PathBuf> },
    /// What changed in the files of the Git repository of a folder, by
    /// its absolute path, as Git tells it.
    Changes { folder: PathBuf },
    /// The map of the project's crates and modules, open at the crate and
    /// the modules in it `at` names, from the crate down, or at the
    /// crates when it names none.
    Map { at: Vec<String> },
}

/// Lines of a file, as people number them: from 1, the last included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lines {
    pub first: usize,
    pub last: usize,
}
