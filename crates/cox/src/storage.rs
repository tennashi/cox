//! Where Cox keeps what lasts across its runs, each kind apart (see
//! [`Kept`]): what the human writes or chooses, at the top; what Cox
//! would lose, as the chats (see [`cox_core::store`]) and the stage's
//! profile (see [`cox_browser::profile`]), in `state`; what Cox can make
//! again, in `cache`; what Cox said of each run (see [`crate::logs`]), in
//! `logs`; and what is there only while Cox runs, in `run`.
//!
//! Each place Cox runs from keeps its own, the way VS Code keeps its data
//! apart from that of a build of its own source: the Cox
//! `cargo xtask install` put in place keeps the human's, and the one being
//! built in `target/` keeps its own beside it, in `cox-data`. A Cox told
//! where to keep it, with `--data`, keeps it there instead, as to run apart
//! from what it kept before, or on what another kept.

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Where `cargo xtask install` puts Cox, from the home directory.
const INSTALLED: &str = if cfg!(target_os = "macos") {
    "Applications/Cox.app/Contents/MacOS/Cox"
} else {
    ".local/bin/cox"
};

/// The folder a Cox other than the installed one keeps what lasts in,
/// beside it.
const BESIDE: &str = "cox-data";

/// Where this Cox was told to keep what lasts, if it was.
static DATA: OnceLock<PathBuf> = OnceLock::new();

thread_local! {
    /// Where what runs on this thread keeps what lasts, in place of
    /// [`DATA`], when it was told.
    static ON_THIS_THREAD: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// What Cox keeps across its runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// What the human sets, as the home and the wiki's folder (see
    /// [`crate::settings`]).
    Settings,
    /// The agents the human lists (see [`crate::agents`]).
    Agents,
    /// The language servers the human names, over Cox's (see
    /// [`crate::stage::servers`]).
    Servers,
    /// The folders the sketches work in, and what is kept of those
    /// discarded (see [`crate::chat::sketch`]).
    Sketches,
    /// The chats, their conversations and their stages' tabs.
    Chats,
    /// The stage's profile: its cookies, logins and storage.
    Stage,
    /// The ACP Registry, as Cox last fetched it (see [`crate::agents`]).
    Registry,
    /// The agents of the registry that Cox downloaded.
    Downloads,
    /// The language servers that Cox downloaded (see
    /// [`crate::stage::servers`]).
    ServerDownloads,
    /// The translations (see [`crate::translate`]).
    Translations,
    /// What Cox said of each run.
    Logs,
    /// The socket an agent asks Cox how it stands on (see
    /// [`crate::inspect`]).
    Inspect,
    /// The socket the window reaches coxd on, which runs the chats (see
    /// [`crate::daemon::process`]).
    Daemon,
    /// When each of the human's schedules last ran (see
    /// [`crate::schedule`]).
    Schedules,
    /// The pictures of the regions of the window the human handed the
    /// chats (see [`crate::chat::region`]).
    Pictures,
    /// The pictures, videos and files shown in the conversations (see
    /// [`crate::media`]).
    Media,
    /// The icons the human gave the projects, as the rail draws them (see
    /// [`crate::project_icon`]).
    Icons,
    /// Which triggers the human turned on or off, and the order they run
    /// in, folder by folder (see [`crate::trigger`]).
    Triggers,
    /// How each trigger last ran, folder by folder (see
    /// [`crate::trigger`]).
    TriggerRuns,
    /// What a chat last said as its turn ended, folder by folder, for a
    /// trigger to be tried on (see [`crate::trigger::card`]).
    LastTurns,
    /// The secret a phone is paired with, to read the chats on the page
    /// coxd serves it (see [`crate::daemon::phone`]).
    PhoneToken,
    /// The key coxd signs what it pushes to the phones with, which their
    /// browsers' push services know it by (see [`crate::daemon::push`]).
    PushKey,
    /// Where coxd pushes to each phone that asked to be told of the
    /// chats (see [`crate::daemon::push`]).
    Pushes,
    /// The pages the stage visited (see [`crate::stage::page::history`]).
    History,
}

impl Kept {
    /// All that Cox keeps.
    pub const ALL: [Self; 24] = [
        Self::Settings,
        Self::Agents,
        Self::Servers,
        Self::Sketches,
        Self::Chats,
        Self::Stage,
        Self::Registry,
        Self::Downloads,
        Self::ServerDownloads,
        Self::Translations,
        Self::Logs,
        Self::Inspect,
        Self::Daemon,
        Self::Schedules,
        Self::Pictures,
        Self::Media,
        Self::Icons,
        Self::Triggers,
        Self::TriggerRuns,
        Self::LastTurns,
        Self::PhoneToken,
        Self::PushKey,
        Self::Pushes,
        Self::History,
    ];

    /// Where it is kept, from the folder Cox keeps what lasts in.
    #[must_use]
    pub const fn place(self) -> &'static str {
        match self {
            Self::Settings => "settings.json",
            Self::Agents => "agents.json",
            Self::Servers => "servers.json",
            Self::Sketches => "sketches",
            Self::Chats => "state/chats.sqlite",
            Self::Stage => "state/stage",
            Self::Registry => "cache/registry.json",
            Self::Downloads => "cache/agents",
            Self::ServerDownloads => "cache/servers",
            Self::Translations => "cache/translations",
            Self::Logs => "logs",
            Self::Inspect => "run/inspect.sock",
            Self::Daemon => "run/coxd.sock",
            Self::Schedules => "state/schedules.json",
            Self::Pictures => "state/pictures",
            Self::Media => "state/media",
            Self::Icons => "state/icons",
            Self::Triggers => "state/triggers.json",
            Self::TriggerRuns => "state/trigger-runs.json",
            Self::LastTurns => "state/last-turns.json",
            Self::PhoneToken => "state/phone-token",
            Self::PushKey => "state/push-key",
            Self::Pushes => "state/pushes.json",
            Self::History => "state/history.json",
        }
    }
}

/// Keeps what lasts in `directory` from now on, for the rest of the
/// process, in place of where this Cox keeps it from the place it runs
/// from. Only the first call counts.
pub fn keep_in(directory: &Path) {
    let directory = std::path::absolute(directory).unwrap_or_else(|_| directory.to_owned());
    let _ = DATA.set(directory);
}

/// Keeps what lasts in `directory` for what runs on this thread, and on
/// the thread of each daemon it starts, in place of where the process
/// keeps it, or, with `None`, goes back to that. A test runs on a thread
/// of its own, beside others in one process, each with its own.
pub fn keep_on_this_thread(directory: Option<PathBuf>) {
    ON_THIS_THREAD.with_borrow_mut(|kept| *kept = directory);
}

/// Where what runs on this thread was told to keep what lasts, if it was.
#[must_use]
pub fn kept_on_this_thread() -> Option<PathBuf> {
    ON_THIS_THREAD.with_borrow(Clone::clone)
}

/// The folder this Cox keeps what lasts in, or `None` when it cannot
/// tell where it runs from.
#[must_use]
pub fn root() -> Option<PathBuf> {
    if let Some(data) = kept_on_this_thread() {
        return Some(data);
    }
    if let Some(data) = DATA.get() {
        return Some(data.clone());
    }
    // Without links, so the installed Cox is found however it was run.
    let exe = std::env::current_exe().and_then(fs::canonicalize).ok()?;
    let home = std::env::var_os("HOME").map(|home| fs::canonicalize(&home).unwrap_or(home.into()));
    root_at(&exe, home.as_deref(), std::env::var_os("XDG_DATA_HOME"))
}

/// Where this Cox keeps `kept`, making the folder it is in, or `None`
/// when it cannot tell where it runs from.
#[must_use]
pub fn path(kept: Kept) -> Option<PathBuf> {
    let path = root()?.join(kept.place());
    if let Some(folder) = path.parent() {
        // A file there that cannot be made says why where it is used.
        let _ = fs::create_dir_all(folder);
    }
    Some(path)
}

/// What Cox keeps by the paths of folders, or of files in them, outside
/// its database: how the triggers of each folder are arranged and last
/// ran, what a chat of each said last as its turn ended, and when each
/// schedule last ran, by its file.
const BY_FOLDER: [Kept; 4] = [
    Kept::Triggers,
    Kept::TriggerRuns,
    Kept::LastTurns,
    Kept::Schedules,
];

/// Makes what Cox keeps by the paths of a folder that moved, as `moved`
/// says, outside its database, name where it is now, so that its
/// triggers stay arranged as they were, and its schedules know when
/// they last ran.
///
/// # Errors
///
/// When a file of it cannot be read or written, saying which.
pub fn moved(moved: cox_core::moved::Moved<'_>) -> Result<(), String> {
    let mut failed = Vec::new();
    for kept in BY_FOLDER {
        if let Some(path) = path(kept)
            && let Err(error) = move_in(&path, moved)
        {
            failed.push(error);
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

/// Keeps `bytes` in `file` for the human alone, whole or not at all:
/// through a file beside it, which none but the human may read, put in
/// its place once written.
///
/// # Errors
///
/// When it cannot be written.
pub fn keep_secret(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut partial = file.as_os_str().to_owned();
    partial.push(format!(".{}.partial", std::process::id()));
    let partial = PathBuf::from(partial);
    let _ = fs::remove_file(&partial);
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&partial)
        .and_then(|mut kept| kept.write_all(bytes))
        .and_then(|()| fs::rename(&partial, file));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
    written
}

/// Makes the JSON file at `path`, if there is one, name where a folder
/// moved, as `moved` says.
fn move_in(path: &Path, moved: cox_core::moved::Moved<'_>) -> Result<(), String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let mut value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if !moved.json(&mut value) {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
    write_whole(path, &(text + "\n"))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Writes `text` to the file at `path` whole: to a file of this write's
/// own beside it first, then put in its place, so that another process
/// reading it meanwhile, as coxd reads which triggers the window turned
/// on, reads it as it was or as it is now, and never part of each.
///
/// # Errors
///
/// When the file beside it cannot be written, or put in its place.
pub fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut part = path.as_os_str().to_owned();
    part.push(format!(
        ".{}-{}.part",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let part = PathBuf::from(part);
    let written = fs::write(&part, text).and_then(|()| fs::rename(&part, path));
    if written.is_err() {
        let _ = fs::remove_file(&part);
    }
    written
}

/// Where the Cox at `exe` keeps what lasts. The installed one keeps it in
/// `~/Library/Application Support/Cox` on macOS, and in
/// `$XDG_DATA_HOME/cox`, by default `~/.local/share/cox`, elsewhere. Any
/// other keeps it in `cox-data` beside it, or beside its app on macOS,
/// which `cargo xtask bundle` makes anew each time.
fn root_at(exe: &Path, home: Option<&Path>, xdg_data_home: Option<OsString>) -> Option<PathBuf> {
    let Some(home) = home.filter(|home| exe == home.join(INSTALLED)) else {
        return Some(outside_app(exe)?.join(BESIDE));
    };
    Some(if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Cox")
    } else {
        // The XDG spec says to ignore a relative path.
        match xdg_data_home.map(PathBuf::from) {
            Some(path) if path.is_absolute() => path.join("cox"),
            _ => home.join(".local/share/cox"),
        }
    })
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
    use crate::testing::TempDir;

    fn now(root: Option<PathBuf>, kept: Kept) -> PathBuf {
        root.unwrap().join(kept.place())
    }

    #[test]
    fn a_file_kept_by_folder_names_where_the_folder_moved() {
        let dir = TempDir::new();
        let file = dir.join("triggers.json");
        fs::write(
            &file,
            r#"{"/work/app": {"off": ["/work/app/triggers/ci.md"]}, "/work/b": {}}"#,
        )
        .unwrap();
        let moved = cox_core::moved::Moved {
            from: Path::new("/work/app"),
            to: Path::new("/src/app"),
        };
        move_in(&file, moved).unwrap();
        let now: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            now,
            serde_json::json!({"/src/app": {"off": ["/src/app/triggers/ci.md"]}, "/work/b": {}})
        );
        // A file that is not there is left so.
        move_in(&dir.join("none.json"), moved).unwrap();
        assert!(!dir.join("none.json").exists());
    }

    #[test]
    fn a_cox_being_built_keeps_its_own_beside_it() {
        let home = Some(Path::new("/h"));
        let built = Path::new("/src/cox/target/debug/cox");
        assert_eq!(
            now(root_at(built, home, None), Kept::Stage),
            PathBuf::from("/src/cox/target/debug/cox-data/state/stage")
        );
        assert_eq!(
            now(
                root_at(
                    Path::new("/src/cox/target/debug/Cox.app/Contents/MacOS/Cox"),
                    home,
                    None
                ),
                Kept::Chats
            ),
            PathBuf::from("/src/cox/target/debug/cox-data/state/chats.sqlite")
        );
        assert_eq!(
            now(root_at(built, None, None), Kept::Settings),
            PathBuf::from("/src/cox/target/debug/cox-data/settings.json")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_installed_cox_keeps_its_own_in_application_support() {
        let root = || {
            root_at(
                Path::new("/Users/h/Applications/Cox.app/Contents/MacOS/Cox"),
                Some(Path::new("/Users/h")),
                Some("/x".into()),
            )
        };
        assert_eq!(
            now(root(), Kept::Stage),
            PathBuf::from("/Users/h/Library/Application Support/Cox/state/stage")
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_installed_cox_keeps_its_own_in_the_xdg_data_directory() {
        let installed = Path::new("/home/h/.local/bin/cox");
        let home = Some(Path::new("/home/h"));
        assert_eq!(
            now(root_at(installed, home, Some("/data".into())), Kept::Stage),
            PathBuf::from("/data/cox/state/stage")
        );
        assert_eq!(
            now(root_at(installed, home, Some("data".into())), Kept::Stage),
            PathBuf::from("/home/h/.local/share/cox/state/stage")
        );
        assert_eq!(
            now(root_at(installed, home, None), Kept::Registry),
            PathBuf::from("/home/h/.local/share/cox/cache/registry.json")
        );
    }
}
