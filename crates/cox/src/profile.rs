//! Where the stage's browser keeps its state: cookies, logins, storage.
//!
//! Cox keeps one profile for all its runs, so a login on the stage lasts
//! past a restart. Each place Cox runs from has a profile of its own, the
//! way VS Code keeps its data apart from that of a build of its own
//! source: the Cox `cargo xtask install` put in place keeps the human's
//! logins, and the one being built in `target/` keeps its own beside it.
//! Chromium lets only one process at a time use a profile, so a second
//! Cox from the same place gets a throwaway profile instead, gone when it
//! quits.

use std::ffi::OsString;
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
    /// Opens the profile this Cox keeps, or a throwaway one when another
    /// Cox from the same place holds it, or there is none.
    #[must_use]
    pub fn open() -> Self {
        // Without links, so the installed Cox is found however it was run.
        let exe = std::env::current_exe().and_then(fs::canonicalize);
        let home =
            std::env::var_os("HOME").map(|home| fs::canonicalize(&home).unwrap_or(home.into()));
        Self::open_at(
            exe.ok().and_then(|exe| {
                kept_path(&exe, home.as_deref(), std::env::var_os("XDG_DATA_HOME"))
            }),
        )
    }

    /// Opens the profile at `kept`, or a throwaway one like [`open`] does.
    ///
    /// [`open`]: Self::open
    #[must_use]
    pub fn open_at(kept: Option<PathBuf>) -> Self {
        let Some(path) = kept else {
            log::warn!("no place to keep the stage's profile, so it lasts for this run only");
            return Self::throwaway();
        };
        match lock(&path) {
            Ok(Some(lock)) => Self::Kept { path, _lock: lock },
            Ok(None) => {
                log::info!(
                    "another Cox uses the stage's profile at {}, so this one gets its own for this run",
                    path.display()
                );
                Self::throwaway()
            }
            Err(error) => {
                log::warn!(
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
            log::warn!("the stage's profile at {} stays: {error}", path.display());
        }
    }
}

/// Where `cargo xtask install` puts Cox, from the home directory.
const INSTALLED: &str = if cfg!(target_os = "macos") {
    "Applications/Cox.app/Contents/MacOS/Cox"
} else {
    ".local/bin/cox"
};

/// Where the Cox at `exe` keeps its profile. The installed one keeps it
/// under `~/Library/Application Support/Cox` on macOS, and under
/// `$XDG_DATA_HOME/cox`, by default `~/.local/share/cox`, elsewhere. Any
/// other keeps it in `cox-stage` beside it, or beside its app on macOS,
/// which `cargo xtask bundle` makes anew each time.
fn kept_path(exe: &Path, home: Option<&Path>, xdg_data_home: Option<OsString>) -> Option<PathBuf> {
    let Some(home) = home.filter(|home| exe == home.join(INSTALLED)) else {
        return Some(outside_app(exe)?.join("cox-stage"));
    };
    let data = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Cox")
    } else {
        // The XDG spec says to ignore a relative path.
        match xdg_data_home.map(PathBuf::from) {
            Some(path) if path.is_absolute() => path.join("cox"),
            _ => home.join(".local/share/cox"),
        }
    };
    Some(data.join("stage"))
}

/// The directory that holds `exe`, or the app `exe` is in, as in
/// `Cox.app/Contents/MacOS/Cox`.
fn outside_app(exe: &Path) -> Option<&Path> {
    let directory = exe.parent()?;
    match directory.parent().and_then(Path::parent) {
        Some(app) if app.extension().is_some_and(|extension| extension == "app") => app.parent(),
        _ => Some(directory),
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

    #[test]
    fn a_cox_being_built_keeps_its_profile_beside_it() {
        let home = Some(Path::new("/h"));
        assert_eq!(
            kept_path(Path::new("/src/cox/target/debug/cox"), home, None),
            Some("/src/cox/target/debug/cox-stage".into())
        );
        assert_eq!(
            kept_path(
                Path::new("/src/cox/target/debug/Cox.app/Contents/MacOS/Cox"),
                home,
                None
            ),
            Some("/src/cox/target/debug/cox-stage".into())
        );
        assert_eq!(
            kept_path(Path::new("/src/cox/target/debug/cox"), None, None),
            Some("/src/cox/target/debug/cox-stage".into())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_installed_cox_keeps_its_profile_in_application_support() {
        assert_eq!(
            kept_path(
                Path::new("/Users/h/Applications/Cox.app/Contents/MacOS/Cox"),
                Some(Path::new("/Users/h")),
                Some("/x".into())
            ),
            Some("/Users/h/Library/Application Support/Cox/stage".into())
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_installed_cox_keeps_its_profile_in_the_xdg_data_directory() {
        let installed = Path::new("/home/h/.local/bin/cox");
        let home = Some(Path::new("/home/h"));
        assert_eq!(
            kept_path(installed, home, Some("/data".into())),
            Some("/data/cox/stage".into())
        );
        assert_eq!(
            kept_path(installed, home, Some("data".into())),
            Some("/home/h/.local/share/cox/stage".into())
        );
        assert_eq!(
            kept_path(installed, home, None),
            Some("/home/h/.local/share/cox/stage".into())
        );
    }
}
