//! Where Cox keeps what lasts across its runs, each kind apart (see
//! [`Kept`]): what the human writes or chooses, at the top; what Cox
//! would lose, as the chats (see [`cox_core::store`]) and the stage's
//! profile (see `cox_browser::profile`), in `state`; what Cox can make
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
use std::time::SystemTime;

/// Where `cargo xtask install` puts Cox, from the home directory.
const INSTALLED: &str = if cfg!(target_os = "macos") {
    "Applications/Cox.app/Contents/MacOS/Cox"
} else {
    ".local/bin/cox"
};

/// Where Homebrew puts Cox, for everyone on the Mac.
const INSTALLED_BY_HOMEBREW: &str = "/Applications/Cox.app/Contents/MacOS/Cox";

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
    /// `cox::stage::servers`).
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
    /// `cox::stage::servers`).
    ServerDownloads,
    /// The translations (see [`crate::translate`]).
    Translations,
    /// What Cox said of each run.
    Logs,
    /// The socket an agent asks Cox how it stands on (see
    /// `cox::inspect`).
    Inspect,
    /// The socket the window reaches coxd on, which runs the chats (see
    /// [`crate::daemon::process`]).
    Daemon,
    /// When each of the human's schedules last ran (see
    /// [`crate::schedule`]).
    Schedules,
    /// The pictures of the regions of the window the human handed the
    /// chats (see `cox::chat::region`).
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
    /// The pages the stage visited (see `cox::stage::page::history`).
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
    let (exe, home) = where_this_runs()?;
    root_at(&exe, home.as_deref(), std::env::var_os("XDG_DATA_HOME"))
}

/// Whether this is an installed Cox, keeping what lasts in the human's
/// own data folder: not one being built, nor one told where to keep it,
/// as a test or `cargo xtask run` tells it. Only an installed Cox reaches
/// beyond its data folder, as to the phone.
#[must_use]
pub fn installed() -> bool {
    if kept_on_this_thread().is_some() || DATA.get().is_some() {
        return false;
    }
    where_this_runs().is_some_and(|(exe, home)| home.is_some_and(|home| installed_at(&exe, &home)))
}

/// This Cox's executable, and the human's home, without links, so the
/// installed Cox is found however it was run.
fn where_this_runs() -> Option<(PathBuf, Option<PathBuf>)> {
    let exe = std::env::current_exe().and_then(fs::canonicalize).ok()?;
    let home = crate::home::home().map(|home| fs::canonicalize(&home).unwrap_or(home));
    Some((exe, home))
}

/// Whether the Cox at `exe` is installed, by `cargo xtask install` or by
/// Homebrew.
fn installed_at(exe: &Path, home: &Path) -> bool {
    exe == home.join(INSTALLED)
        || (cfg!(target_os = "macos") && exe == Path::new(INSTALLED_BY_HOMEBREW))
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
    write_json(path, &value).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Writes `contents` to the file at `path` whole, making its folder if
/// need be: to a file of this write's own beside it first, then put in
/// its place, so that another process reading it meanwhile, as coxd reads
/// which triggers the window turned on, reads it as it was or as it is
/// now, and never part of each.
///
/// # Errors
///
/// When the file beside it cannot be written, or put in its place.
pub fn write_whole(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_whole_as(path, contents.as_ref(), 0o666)
}

/// Writes `value` to the file at `path` whole, as [`write_whole`] does,
/// as JSON a human can read, ending in a newline.
///
/// # Errors
///
/// When `value` is no JSON, or it cannot be written.
pub fn write_json(path: &Path, value: &impl serde::Serialize) -> std::io::Result<()> {
    let mut text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    text.push('\n');
    write_whole(path, text)
}

/// Keeps `bytes` in `file` whole, as [`write_whole`] does, for the human
/// alone: none but the human may read it, even while it is written.
///
/// # Errors
///
/// When it cannot be written.
pub fn keep_secret(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_whole_as(file, bytes, 0o600)
}

fn write_whole_as(path: &Path, contents: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let mut part = path.as_os_str().to_owned();
    part.push(format!(
        ".{}-{}.part",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let part = PathBuf::from(part);
    // One a Cox that crashed left, by a process id used again.
    let _ = fs::remove_file(&part);
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&part)
        .and_then(|mut file| file.write_all(contents))
        .and_then(|()| fs::rename(&part, path));
    if written.is_err() {
        let _ = fs::remove_file(&part);
    }
    written
}

/// Where the Cox at `exe` keeps what lasts. The installed one, whether
/// `cargo xtask install` or Homebrew put it there, keeps it in
/// `~/Library/Application Support/Cox` on macOS, and in
/// `$XDG_DATA_HOME/cox`, by default `~/.local/share/cox`, elsewhere. Any
/// other keeps it in `cox-data` beside it, or beside its app on macOS,
/// which `cargo xtask bundle` makes anew each time.
fn root_at(exe: &Path, home: Option<&Path>, xdg_data_home: Option<OsString>) -> Option<PathBuf> {
    let Some(home) = home.filter(|home| installed_at(exe, home)) else {
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

/// When a file last changed, and how long it was, as a tab, or coxd
/// following the settings, last read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    modified: SystemTime,
    len: u64,
}

impl Stamp {
    /// When the file at `path` last changed, and how long it is.
    ///
    /// # Errors
    ///
    /// When the file cannot be looked at, as when there is none.
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            modified: metadata.modified()?,
            len: metadata.len(),
        })
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

    #[cfg(target_os = "macos")]
    #[test]
    fn the_cox_homebrew_installed_keeps_the_same_as_the_one_built_and_installed() {
        let home = Some(Path::new("/Users/h"));
        assert_eq!(
            now(
                root_at(
                    Path::new("/Applications/Cox.app/Contents/MacOS/Cox"),
                    home,
                    None
                ),
                Kept::Stage
            ),
            now(
                root_at(
                    Path::new("/Users/h/Applications/Cox.app/Contents/MacOS/Cox"),
                    home,
                    None
                ),
                Kept::Stage
            ),
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

    #[test]
    fn a_file_written_whole_is_read_as_it_was_or_as_it_is_now() {
        use std::sync::atomic::AtomicBool;

        let dir = TempDir::new();
        let file = dir.join("deep/agents.json");
        let was = "was".repeat(1 << 18);
        let one = "one".repeat(1 << 18);
        let other = "other".repeat(1 << 18);
        write_whole(&file, &was).unwrap();
        let written = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let reader = scope.spawn(|| {
                let mut reads = 0;
                while !written.load(Ordering::Relaxed) || reads == 0 {
                    let read = fs::read_to_string(&file).unwrap();
                    assert!(
                        read == was || read == one || read == other,
                        "read {} bytes that are part of each",
                        read.len()
                    );
                    reads += 1;
                }
            });
            let file = &file;
            let writers = [&one, &other]
                .map(|text| scope.spawn(move || (0..20).try_for_each(|_| write_whole(file, text))));
            let ends = writers.map(|writer| writer.join().unwrap());
            written.store(true, Ordering::Relaxed);
            reader.join().unwrap();
            for end in ends {
                end.unwrap();
            }
        });
        let now = fs::read_to_string(&file).unwrap();
        assert!(now == one || now == other);
        let left: Vec<_> = fs::read_dir(dir.join("deep")).unwrap().collect();
        assert_eq!(left.len(), 1, "no file of a write is left beside it");
    }

    #[test]
    fn a_secret_is_kept_for_the_human_alone() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new();
        let file = dir.join("phone-token");
        fs::write(&file, "old").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        keep_secret(&file, b"new").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn json_is_written_as_a_human_reads_it() {
        let dir = TempDir::new();
        let file = dir.join("schedules.json");
        write_json(&file, &serde_json::json!({"daily": "2026-11-14T00:00:00Z"})).unwrap();
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "{\n  \"daily\": \"2026-11-14T00:00:00Z\"\n}\n"
        );
    }
}
