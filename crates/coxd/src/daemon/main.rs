//! Running coxd, `cox --daemon`, from its start: the login shell's
//! environment, where it keeps what lasts, its log, the agents and
//! settings it follows, and the store of the chats, before
//! [`super::process::run`] serves them.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use cox_core::store::Store;

use super::launcher::Launcher;
use crate::agents::Agents;

/// Runs coxd until it is stopped, keeping what lasts in `data` when
/// given: the chats, kept where this Cox keeps
/// what lasts, their agents started as `agents.json` and `settings.json`
/// say, following them as they change.
#[must_use]
pub fn run(data: Option<&Path>) -> ExitCode {
    // First, while coxd has no other thread.
    let environment = crate::login::take_in();
    if let Some(data) = data {
        crate::storage::keep_in(data);
    }
    crate::logs::init(crate::storage::path(crate::storage::Kept::Logs).as_deref());
    crate::scratch::sweep();
    if let crate::login::Taken::Failed(why) = environment {
        tracing::warn!("the agents get launchd's environment, not the login shell's: {why}");
    }
    let Some(socket) = crate::storage::path(crate::storage::Kept::Daemon) else {
        eprintln!("cox: there is no place for coxd's socket");
        return ExitCode::from(1);
    };
    let saved = crate::settings::Saved::read();
    let home = crate::settings::home_for_chats(saved.home);
    let agents = Agents::read(Vec::new(), saved.agent.as_deref());
    agents.fetch();
    follow_agents(agents.clone());
    let launcher = Launcher {
        agents,
        path: std::env::var_os("PATH").unwrap_or_default(),
        exe: std::env::current_exe()
            .inspect_err(|error| {
                tracing::error!("the agents get no tools of Cox's own: {error}");
            })
            .ok(),
    };
    let ran = crate::daemon::process::run(
        &socket,
        home,
        move || Rc::new(open_store()),
        move || Rc::new(launcher),
    );
    crate::scratch::take_away();
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error:#}");
            eprintln!("cox: {error:#}");
            ExitCode::from(1)
        }
    }
}

/// How often coxd looks whether `settings.json` or `agents.json` changed.
const FOLLOW_EVERY: Duration = Duration::from_secs(1);

/// Follows `settings.json` and `agents.json` for coxd, on a thread of its
/// own, as the window does for itself (see [`crate::settings::window::follow`]):
/// a chat opened from then on talks to the agents they list and name.
fn follow_agents(agents: Agents) {
    let files: Vec<PathBuf> = [crate::storage::Kept::Settings, crate::storage::Kept::Agents]
        .into_iter()
        .filter_map(crate::storage::path)
        .collect();
    let stamps = move || {
        files
            .iter()
            .map(|file| crate::storage::Stamp::of(file).ok())
            .collect::<Vec<_>>()
    };
    let spawned = std::thread::Builder::new()
        .name("coxd's settings".to_owned())
        .spawn(move || {
            let mut seen = stamps();
            loop {
                std::thread::sleep(FOLLOW_EVERY);
                let now = stamps();
                if now != seen {
                    seen = now;
                    agents.reload(crate::settings::agent().as_deref());
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!("coxd does not follow the settings: {error}");
    }
}

/// The store of the chats this Cox keeps, or one for this run only when
/// that cannot be opened.
fn open_store() -> Store {
    let kept = crate::storage::path(crate::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("there is no place to keep them"))
        .and_then(|path| Store::open(&path));
    kept.or_else(|error| {
        tracing::error!("the chats last for this run only: {error:#}");
        Store::in_memory()
    })
    .expect("SQLite opens a database in memory")
}
