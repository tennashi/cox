//! What the unit tests share: a folder of a test's own, and Git that reads
//! none of the developer's settings.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A folder of a test's own, made empty among the system's temporary
/// files and taken away when it is dropped, even when the test panics.
/// Its path is canonical, as the paths Cox works out are.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Makes a folder no other test, in this process or another, has.
    #[must_use]
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "cox-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    /// Where the folder is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Default for TempDir {
    fn default() -> Self {
        Self::new()
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Git run in `folder`, reading neither the developer's settings nor the
/// system's, nor the repository the tests themselves run in, and
/// committing as a test.
#[must_use]
pub fn git(folder: &Path) -> Command {
    let mut git = cox_git::git::command();
    git.arg("-C")
        .arg(folder)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com");
    git
}

/// Runs Git in `folder` as [`git`] does with `args`, answering what it
/// printed, or what it said went wrong.
///
/// # Errors
///
/// When Git fails, with what it wrote on its standard error.
pub fn run_git(folder: &Path, args: &[&str]) -> Result<String, String> {
    let output = git(folder).args(args).output().unwrap();
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}
