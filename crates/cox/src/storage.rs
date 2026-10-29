//! Where Cox keeps what lasts across its runs: the stage's profile (see
//! [`crate::browser::profile`]) and the chats (see [`cox_core::store`]).
//!
//! Each place Cox runs from keeps its own, the way VS Code keeps its data
//! apart from that of a build of its own source: the Cox
//! `cargo xtask install` put in place keeps the human's, and the one being
//! built in `target/` keeps its own beside it.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// Where `cargo xtask install` puts Cox, from the home directory.
const INSTALLED: &str = if cfg!(target_os = "macos") {
    "Applications/Cox.app/Contents/MacOS/Cox"
} else {
    ".local/bin/cox"
};

/// Where this Cox keeps what it calls `name`, or `None` when it cannot
/// tell where it runs from.
#[must_use]
pub fn path(name: &str) -> Option<PathBuf> {
    // Without links, so the installed Cox is found however it was run.
    let exe = std::env::current_exe().and_then(fs::canonicalize).ok()?;
    let home = std::env::var_os("HOME").map(|home| fs::canonicalize(&home).unwrap_or(home.into()));
    path_at(
        name,
        &exe,
        home.as_deref(),
        std::env::var_os("XDG_DATA_HOME"),
    )
}

/// Where the Cox at `exe` keeps what it calls `name`. The installed one
/// keeps it under `~/Library/Application Support/Cox` on macOS, and under
/// `$XDG_DATA_HOME/cox`, by default `~/.local/share/cox`, elsewhere. Any
/// other keeps it as `cox-<name>` beside it, or beside its app on macOS,
/// which `cargo xtask bundle` makes anew each time.
fn path_at(
    name: &str,
    exe: &Path,
    home: Option<&Path>,
    xdg_data_home: Option<OsString>,
) -> Option<PathBuf> {
    let Some(home) = home.filter(|home| exe == home.join(INSTALLED)) else {
        return Some(outside_app(exe)?.join(format!("cox-{name}")));
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
    Some(data.join(name))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cox_being_built_keeps_its_own_beside_it() {
        let home = Some(Path::new("/h"));
        assert_eq!(
            path_at("stage", Path::new("/src/cox/target/debug/cox"), home, None),
            Some("/src/cox/target/debug/cox-stage".into())
        );
        assert_eq!(
            path_at(
                "chats.sqlite",
                Path::new("/src/cox/target/debug/Cox.app/Contents/MacOS/Cox"),
                home,
                None
            ),
            Some("/src/cox/target/debug/cox-chats.sqlite".into())
        );
        assert_eq!(
            path_at("stage", Path::new("/src/cox/target/debug/cox"), None, None),
            Some("/src/cox/target/debug/cox-stage".into())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_installed_cox_keeps_its_own_in_application_support() {
        assert_eq!(
            path_at(
                "stage",
                Path::new("/Users/h/Applications/Cox.app/Contents/MacOS/Cox"),
                Some(Path::new("/Users/h")),
                Some("/x".into())
            ),
            Some("/Users/h/Library/Application Support/Cox/stage".into())
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_installed_cox_keeps_its_own_in_the_xdg_data_directory() {
        let installed = Path::new("/home/h/.local/bin/cox");
        let home = Some(Path::new("/home/h"));
        assert_eq!(
            path_at("stage", installed, home, Some("/data".into())),
            Some("/data/cox/stage".into())
        );
        assert_eq!(
            path_at("stage", installed, home, Some("data".into())),
            Some("/home/h/.local/share/cox/stage".into())
        );
        assert_eq!(
            path_at("chats.sqlite", installed, home, None),
            Some("/home/h/.local/share/cox/chats.sqlite".into())
        );
    }
}
