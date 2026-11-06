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
//!
//! A Cox from before kept each of them beside the others, and a Cox being
//! built kept each as `cox-<name>` beside it; [`settle`] moves them to
//! where they are kept now.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

/// What Cox keeps across its runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// What the human sets, as the home and the wiki's folder (see
    /// [`crate::settings`]).
    Settings,
    /// The agents the human lists (see [`crate::agents`]).
    Agents,
    /// The human's workflow, as an earlier Cox kept it when the settings
    /// named none, which moves into the home now (see
    /// [`crate::settings::Folder`]).
    Workflow,
    /// The human's wiki, as an earlier Cox kept it when the settings named
    /// none, which moves into the home now (see
    /// [`crate::settings::Folder`]).
    Wiki,
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
    /// The translations (see [`crate::translate`]).
    Translations,
    /// What Cox said of each run.
    Logs,
    /// The socket an agent asks Cox how it stands on (see
    /// [`crate::inspect`]).
    Inspect,
}

impl Kept {
    /// All that Cox keeps.
    pub const ALL: [Self; 12] = [
        Self::Settings,
        Self::Agents,
        Self::Workflow,
        Self::Wiki,
        Self::Sketches,
        Self::Chats,
        Self::Stage,
        Self::Registry,
        Self::Downloads,
        Self::Translations,
        Self::Logs,
        Self::Inspect,
    ];

    /// Where it is kept, from the folder Cox keeps what lasts in.
    #[must_use]
    pub const fn place(self) -> &'static str {
        match self {
            Self::Settings => "settings.json",
            Self::Agents => "agents.json",
            Self::Workflow => "workflow",
            Self::Wiki => "wiki",
            Self::Sketches => "sketches",
            Self::Chats => "state/chats.sqlite",
            Self::Stage => "state/stage",
            Self::Registry => "cache/registry.json",
            Self::Downloads => "cache/agents",
            Self::Translations => "cache/translations",
            Self::Logs => "logs",
            Self::Inspect => "run/inspect.sock",
        }
    }

    /// What it was called by a Cox from before, beside the others: the
    /// last part of where it is kept now.
    fn name(self) -> &'static str {
        let place = self.place();
        place.rsplit_once('/').map_or(place, |(_, name)| name)
    }

    /// What goes with it, by what follows its name: the journal of the
    /// chats' database, and the lock of the stage's profile.
    fn companions(self) -> &'static [&'static str] {
        match self {
            Self::Chats => &["-wal", "-shm"],
            Self::Stage => &[".lock"],
            _ => &[],
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

/// The folder this Cox keeps what lasts in, or `None` when it cannot
/// tell where it runs from.
#[must_use]
pub fn root() -> Option<PathBuf> {
    Some(places()?.now)
}

/// Where this Cox keeps `kept`, making the folder it is in, or `None`
/// when it cannot tell where it runs from. What a Cox from before kept is
/// where it kept it until [`settle`] moves it, as while that Cox runs.
#[must_use]
pub fn path(kept: Kept) -> Option<PathBuf> {
    Some(places()?.of(kept))
}

/// Where this Cox keeps what lasts, and where a Cox from before kept it.
struct Places {
    /// The folder it is kept in now.
    now: PathBuf,
    /// The folder it was kept in.
    before: PathBuf,
    /// What went ahead of each name there: `cox-` beside a Cox being
    /// built.
    prefix: &'static str,
}

impl Places {
    /// Where a Cox from before kept `kept`, and what goes with it, after
    /// `companion`.
    fn before(&self, kept: Kept, companion: &str) -> PathBuf {
        self.before
            .join(format!("{}{}{companion}", self.prefix, kept.name()))
    }

    /// Where `kept` is, making the folder it is in: where it is kept now,
    /// unless it is only where a Cox from before kept it.
    fn of(&self, kept: Kept) -> PathBuf {
        let (now, before) = (self.now(kept, ""), self.before(kept, ""));
        if !now.exists() && before.exists() {
            return before;
        }
        if let Some(folder) = now.parent() {
            // A file there that cannot be made says why where it is used.
            let _ = fs::create_dir_all(folder);
        }
        now
    }

    /// Where `kept`, and what goes with it, after `companion`, is kept
    /// now.
    fn now(&self, kept: Kept, companion: &str) -> PathBuf {
        self.now.join(format!("{}{companion}", kept.place()))
    }
}

/// Where this Cox keeps what lasts, and where it kept it before.
fn places() -> Option<Places> {
    if let Some(data) = DATA.get() {
        return Some(Places {
            now: data.clone(),
            before: data.clone(),
            prefix: "",
        });
    }
    // Without links, so the installed Cox is found however it was run.
    let exe = std::env::current_exe().and_then(fs::canonicalize).ok()?;
    let home = std::env::var_os("HOME").map(|home| fs::canonicalize(&home).unwrap_or(home.into()));
    places_at(&exe, home.as_deref(), std::env::var_os("XDG_DATA_HOME"))
}

/// Where the Cox at `exe` keeps what lasts. The installed one keeps it in
/// `~/Library/Application Support/Cox` on macOS, and in
/// `$XDG_DATA_HOME/cox`, by default `~/.local/share/cox`, elsewhere. Any
/// other keeps it in `cox-data` beside it, or beside its app on macOS,
/// which `cargo xtask bundle` makes anew each time, where it kept each of
/// them as `cox-<name>` before.
fn places_at(exe: &Path, home: Option<&Path>, xdg_data_home: Option<OsString>) -> Option<Places> {
    let Some(home) = home.filter(|home| exe == home.join(INSTALLED)) else {
        let beside = outside_app(exe)?;
        return Some(Places {
            now: beside.join(BESIDE),
            before: beside.to_owned(),
            prefix: "cox-",
        });
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
    Some(Places {
        now: data.clone(),
        before: data,
        prefix: "",
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

/// What [`settle`] did.
#[derive(Debug, Default)]
pub struct Settled {
    /// What it moved, to say once Cox has a log.
    pub moved: Vec<String>,
    /// What it could not move, and why, to say once Cox has a log.
    pub left: Vec<String>,
    /// Where the sketches were, and where they are now, when they moved,
    /// for the chats that work in them to follow.
    pub sketches: Option<(PathBuf, PathBuf)>,
}

/// Moves what a Cox from before kept, from where it kept it to where this
/// Cox keeps it, unless this Cox keeps it already. Cox settles before it
/// uses any of it, and while no other Cox from the same place runs, as
/// the lock of the stage's profile tells.
#[must_use]
pub fn settle() -> Settled {
    places().map_or_else(Settled::default, |places| settle_in(&places))
}

/// Moves what was kept before in `places` to where it is kept now.
fn settle_in(places: &Places) -> Settled {
    let mut settled = Settled::default();
    let lock = places.before(Kept::Stage, ".lock");
    if lock.exists() && places.before(Kept::Stage, "") != places.now(Kept::Stage, "") {
        match fs::File::open(&lock).map(|file| file.try_lock().map(|()| file)) {
            Ok(Ok(_held)) => {}
            Ok(Err(fs::TryLockError::WouldBlock)) => {
                settled.left.push(format!(
                    "another Cox runs from here, so what it keeps stays in {} until it quits",
                    places.before.display()
                ));
                return settled;
            }
            Ok(Err(fs::TryLockError::Error(error))) | Err(error) => {
                settled.left.push(format!(
                    "cannot tell whether another Cox runs from here: {error}"
                ));
                return settled;
            }
        }
    }
    for kept in Kept::ALL {
        let from = places.before(kept, "");
        let to = places.now(kept, "");
        if from == to || !from.exists() {
            continue;
        }
        if kept == Kept::Inspect {
            // A socket no Cox answers at is left from a Cox that ended.
            if UnixStream::connect(&from).is_err() {
                let _ = fs::remove_file(&from);
            }
            continue;
        }
        if to.exists() {
            settled.left.push(format!(
                "{} is left as it was, since {} is there already",
                from.display(),
                to.display()
            ));
            continue;
        }
        let moved = [""]
            .iter()
            .chain(kept.companions())
            .map(|companion| (places.before(kept, companion), places.now(kept, companion)))
            .filter(|(from, _)| from.exists())
            .try_for_each(|(from, to)| move_to(&from, &to));
        match moved {
            Ok(()) => {
                settled
                    .moved
                    .push(format!("moved {} to {}", from.display(), to.display()));
                if kept == Kept::Sketches {
                    settled.left.extend(repair_worktrees(&to));
                    settled.sketches = Some((from, to));
                }
            }
            Err(error) => settled.left.push(format!(
                "cannot move {} to {}: {error}",
                from.display(),
                to.display()
            )),
        }
    }
    settled
}

/// Moves `from` to `to`, making the folder `to` is in.
fn move_to(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(folder) = to.parent() {
        fs::create_dir_all(folder)?;
    }
    fs::rename(from, to)
}

/// Tells the repositories of the sketches in `sketches` that are Git
/// worktrees where each is now, answering what could not be told.
fn repair_worktrees(sketches: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(sketches) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| Some(entry.ok()?.path()))
        // A worktree's `.git` is a file that names its repository.
        .filter(|sketch| sketch.join(".git").is_file())
        .filter_map(|sketch| {
            let repaired = std::process::Command::new("git")
                .arg("-C")
                .arg(&sketch)
                .args(["worktree", "repair"])
                .output();
            match repaired {
                Ok(output) if output.status.success() => None,
                Ok(output) => Some(format!(
                    "Git cannot find the sketch at {}: {}",
                    sketch.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                )),
                Err(error) => Some(format!(
                    "cannot run Git for the sketch at {}: {error}",
                    sketch.display()
                )),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now(places: Option<Places>, kept: Kept) -> PathBuf {
        places.unwrap().now(kept, "")
    }

    fn before(places: Option<Places>, kept: Kept) -> PathBuf {
        places.unwrap().before(kept, "")
    }

    #[test]
    fn a_cox_being_built_keeps_its_own_beside_it() {
        let home = Some(Path::new("/h"));
        let built = Path::new("/src/cox/target/debug/cox");
        assert_eq!(
            now(places_at(built, home, None), Kept::Stage),
            PathBuf::from("/src/cox/target/debug/cox-data/state/stage")
        );
        assert_eq!(
            now(
                places_at(
                    Path::new("/src/cox/target/debug/Cox.app/Contents/MacOS/Cox"),
                    home,
                    None
                ),
                Kept::Chats
            ),
            PathBuf::from("/src/cox/target/debug/cox-data/state/chats.sqlite")
        );
        assert_eq!(
            now(places_at(built, None, None), Kept::Settings),
            PathBuf::from("/src/cox/target/debug/cox-data/settings.json")
        );
        assert_eq!(
            before(places_at(built, home, None), Kept::Downloads),
            PathBuf::from("/src/cox/target/debug/cox-agents")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_installed_cox_keeps_its_own_in_application_support() {
        let places = || {
            places_at(
                Path::new("/Users/h/Applications/Cox.app/Contents/MacOS/Cox"),
                Some(Path::new("/Users/h")),
                Some("/x".into()),
            )
        };
        assert_eq!(
            now(places(), Kept::Stage),
            PathBuf::from("/Users/h/Library/Application Support/Cox/state/stage")
        );
        assert_eq!(
            before(places(), Kept::Stage),
            PathBuf::from("/Users/h/Library/Application Support/Cox/stage")
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_installed_cox_keeps_its_own_in_the_xdg_data_directory() {
        let installed = Path::new("/home/h/.local/bin/cox");
        let home = Some(Path::new("/home/h"));
        assert_eq!(
            now(
                places_at(installed, home, Some("/data".into())),
                Kept::Stage
            ),
            PathBuf::from("/data/cox/state/stage")
        );
        assert_eq!(
            now(places_at(installed, home, Some("data".into())), Kept::Stage),
            PathBuf::from("/home/h/.local/share/cox/state/stage")
        );
        assert_eq!(
            now(places_at(installed, home, None), Kept::Registry),
            PathBuf::from("/home/h/.local/share/cox/cache/registry.json")
        );
        assert_eq!(
            before(places_at(installed, home, None), Kept::Registry),
            PathBuf::from("/home/h/.local/share/cox/registry.json")
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("cox-storage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn what_a_cox_being_built_kept_before_moves_into_its_folder() {
        let debug = scratch("built");
        fs::write(debug.join("cox-chats.sqlite"), "chats").unwrap();
        fs::write(debug.join("cox-chats.sqlite-wal"), "journal").unwrap();
        fs::create_dir_all(debug.join("cox-stage")).unwrap();
        fs::write(debug.join("cox-stage.lock"), "").unwrap();
        fs::create_dir_all(debug.join("cox-logs")).unwrap();
        fs::write(debug.join("cox-settings.json"), "{}").unwrap();
        // Built beside what Cox keeps, and none of it.
        fs::write(debug.join("cox-helper"), "").unwrap();
        let places = Places {
            now: debug.join(BESIDE),
            before: debug.clone(),
            prefix: "cox-",
        };

        let settled = settle_in(&places);

        let data = debug.join(BESIDE);
        assert_eq!(
            fs::read_to_string(data.join("state/chats.sqlite")).unwrap(),
            "chats"
        );
        assert_eq!(
            fs::read_to_string(data.join("state/chats.sqlite-wal")).unwrap(),
            "journal"
        );
        assert!(data.join("state/stage").is_dir());
        assert!(data.join("state/stage.lock").is_file());
        assert!(data.join("logs").is_dir());
        assert!(data.join("settings.json").is_file());
        let mut left: Vec<_> = fs::read_dir(&debug)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, [BESIDE, "cox-helper"], "{:?}", settled.left);
        assert_eq!(settled.sketches, None);
    }

    #[test]
    fn what_the_installed_cox_kept_beside_the_rest_moves_by_its_kind() {
        let data = scratch("installed");
        fs::write(data.join("chats.sqlite"), "chats").unwrap();
        fs::write(data.join("registry.json"), "[]").unwrap();
        fs::create_dir_all(data.join("agents/codex/1.0")).unwrap();
        fs::create_dir_all(data.join("sketches/greet-1")).unwrap();
        fs::write(data.join("agents.json"), "[]").unwrap();
        let places = Places {
            now: data.clone(),
            before: data.clone(),
            prefix: "",
        };

        let settled = settle_in(&places);

        assert!(data.join("state/chats.sqlite").is_file());
        assert!(data.join("cache/registry.json").is_file());
        assert!(data.join("cache/agents/codex/1.0").is_dir());
        assert!(!data.join("agents").exists());
        // Kept where they were.
        assert!(data.join("sketches/greet-1").is_dir());
        assert!(data.join("agents.json").is_file());
        assert_eq!(settled.sketches, None);
    }

    #[test]
    fn what_is_kept_already_stays() {
        let data = scratch("both");
        fs::write(data.join("chats.sqlite"), "old").unwrap();
        fs::create_dir_all(data.join("state")).unwrap();
        fs::write(data.join("state/chats.sqlite"), "new").unwrap();
        let places = Places {
            now: data.clone(),
            before: data.clone(),
            prefix: "",
        };

        let settled = settle_in(&places);

        assert_eq!(
            fs::read_to_string(data.join("state/chats.sqlite")).unwrap(),
            "new"
        );
        assert!(data.join("chats.sqlite").is_file());
        assert_eq!(settled.left.len(), 1, "{:?}", settled.left);
    }

    #[test]
    fn nothing_moves_while_another_cox_runs_from_here() {
        let debug = scratch("running");
        fs::write(debug.join("cox-chats.sqlite"), "chats").unwrap();
        let held = fs::File::create(debug.join("cox-stage.lock")).unwrap();
        held.lock().unwrap();
        let places = Places {
            now: debug.join(BESIDE),
            before: debug.clone(),
            prefix: "cox-",
        };

        let settled = settle_in(&places);

        assert!(debug.join("cox-chats.sqlite").is_file());
        assert!(!debug.join(BESIDE).exists());
        assert_eq!(settled.left.len(), 1, "{:?}", settled.left);
        // Used where it is, until it moves.
        assert_eq!(places.of(Kept::Chats), debug.join("cox-chats.sqlite"));
        assert_eq!(
            places.of(Kept::Settings),
            debug.join(BESIDE).join("settings.json")
        );
    }

    #[test]
    fn a_sketch_that_moves_tells_its_repository() {
        let root = scratch("sketch");
        let git = |dir: &Path, args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(status.status.success(), "{status:?}");
            String::from_utf8(status.stdout).unwrap()
        };
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q"]);
        git(
            &project,
            &[
                "-c",
                "user.name=Cox",
                "-c",
                "user.email=cox@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "start",
            ],
        );
        let debug = root.join("debug");
        let sketch = debug.join("cox-sketches/greet-1");
        git(
            &project,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                sketch.to_str().unwrap(),
            ],
        );
        let places = Places {
            now: debug.join(BESIDE),
            before: debug.clone(),
            prefix: "cox-",
        };

        let settled = settle_in(&places);

        let moved = debug.join(BESIDE).join("sketches");
        assert_eq!(
            settled.sketches,
            Some((debug.join("cox-sketches"), moved.clone()))
        );
        let listed = git(&project, &["worktree", "list", "--porcelain"]);
        let moved = fs::canonicalize(moved.join("greet-1")).unwrap();
        assert!(
            listed.contains(&format!("worktree {}\n", moved.display())),
            "{listed}"
        );
    }
}
