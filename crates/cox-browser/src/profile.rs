//! Where the stage's browser keeps its state: cookies, logins, storage.
//!
//! Cox keeps one profile for all its runs, so a login on the stage lasts
//! past a restart. Each place Cox runs from has a profile of its own,
//! which the app says where to keep: the Cox `cargo xtask install` put in
//! place keeps the human's logins, and the one being built in `target/`
//! keeps its own beside it. Chromium lets only one process at a time use a profile, so a second
//! Cox from the same place gets a throwaway profile instead, gone when it
//! quits.

use std::fs::{self, DirBuilder, File, TryLockError};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

/// The browser's profile for this run.
#[derive(Debug)]
pub enum Profile {
    /// The profile Cox keeps, held by this run until it ends.
    Kept {
        path: PathBuf,
        /// Its lock, which the system lets go of when the process ends,
        /// however it ends.
        _lock: File,
    },
    /// A profile for this run only.
    Throwaway(PathBuf),
}

impl Profile {
    /// Opens the profile at `kept`, the one this Cox keeps, or a throwaway
    /// one when another Cox from the same place holds it, or there is
    /// none to keep.
    #[must_use]
    pub fn open_at(kept: Option<PathBuf>) -> Self {
        let Some(path) = kept else {
            tracing::warn!("no place to keep the stage's profile, so it lasts for this run only");
            return Self::throwaway();
        };
        match lock(&path) {
            Ok(Some(lock)) => Self::Kept { path, _lock: lock },
            Ok(None) => {
                tracing::info!(
                    "another Cox uses the stage's profile at {}, so this one gets its own for this run",
                    path.display()
                );
                Self::throwaway()
            }
            Err(error) => {
                tracing::warn!(
                    "the stage's profile at {} cannot be used, so this run gets its own: {error}",
                    path.display()
                );
                Self::throwaway()
            }
        }
    }

    fn throwaway() -> Self {
        Self::Throwaway(std::env::temp_dir().join(format!("cox-cef-{}", std::process::id())))
    }

    /// The profile's directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Kept { path, .. } | Self::Throwaway(path) => path,
        }
    }

    /// Whether the profile outlasts this run.
    #[must_use]
    pub fn is_kept(&self) -> bool {
        matches!(self, Self::Kept { .. })
    }

    /// Removes a throwaway profile. Call it once the browser is done with
    /// it.
    pub fn discard(&self) {
        if let Self::Throwaway(path) = self
            && let Err(error) = fs::remove_dir_all(path)
        {
            tracing::warn!("the stage's profile at {} stays: {error}", path.display());
        }
    }
}

/// Takes the lock beside the profile at `path`, creating both as needed,
/// or answers `None` when another process holds it.
fn lock(path: &Path) -> std::io::Result<Option<File>> {
    // Only the human reads their cookies.
    DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    let lock = File::create(path.with_extension("lock"))?;
    match lock.try_lock() {
        Ok(()) => Ok(Some(lock)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_run_gets_a_throwaway_profile_until_the_first_ends() {
        let root = std::env::temp_dir().join(format!("cox-profile-{}", std::process::id()));
        let kept = root.join("stage");

        let first = Profile::open_at(Some(kept.clone()));
        assert!(first.is_kept());
        assert_eq!(first.path(), kept);
        let second = Profile::open_at(Some(kept.clone()));
        assert!(!second.is_kept());
        assert_ne!(second.path(), kept);

        drop(first);
        let third = Profile::open_at(Some(kept.clone()));
        assert!(third.is_kept());
        third.discard();
        assert!(kept.is_dir(), "the kept profile stays");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn there_is_no_kept_profile_without_a_place() {
        assert!(!Profile::open_at(None).is_kept());
    }
}
