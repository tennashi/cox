//! Folders of their own for tests, each removed once dropped, even when
//! its test fails.

use std::ops::Deref;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// An empty folder of its own, by its whole path, links followed, as Git
/// names it.
pub(crate) struct Folder {
    path: PathBuf,
    _removed: TempDir,
}

impl Folder {
    pub(crate) fn new() -> Self {
        let removed = TempDir::new().unwrap();
        let path = std::fs::canonicalize(removed.path()).unwrap();
        Self {
            path,
            _removed: removed,
        }
    }
}

impl Deref for Folder {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Folder {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}
