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
use coxd::agents::Agents;

/// Runs coxd until it is stopped, keeping what lasts in `data` when
/// given: the chats, kept where this Cox keeps
/// what lasts, their agents started as `agents.json` and `settings.json`
/// say, following them as they change.
#[must_use]
pub fn run(data: Option<&Path>) -> ExitCode {
    // First, while coxd has no other thread.
    let environment = coxd::login::take_in();
    if let Some(data) = data {
        coxd::storage::keep_in(data);
    }
    coxd::logs::init(coxd::storage::path(coxd::storage::Kept::Logs).as_deref());
    coxd::scratch::sweep();
    if let coxd::login::Taken::Failed(why) = environment {
        tracing::warn!("the agents get launchd's environment, not the login shell's: {why}");
    }
    let Some(socket) = coxd::storage::path(coxd::storage::Kept::Daemon) else {
        eprintln!("cox: there is no place for coxd's socket");
        return ExitCode::from(1);
    };
    let saved = coxd::settings::Saved::read();
    let home = coxd::settings::home_for_chats(saved.home);
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
    coxd::scratch::take_away();
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
    let files: Vec<PathBuf> = [coxd::storage::Kept::Settings, coxd::storage::Kept::Agents]
        .into_iter()
        .filter_map(coxd::storage::path)
        .collect();
    let stamps = move || {
        files
            .iter()
            .map(|file| coxd::storage::Stamp::of(file).ok())
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
                    agents.reload(coxd::settings::agent().as_deref());
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
    let kept = coxd::storage::path(coxd::storage::Kept::Chats)
        .ok_or_else(|| anyhow::anyhow!("there is no place to keep them"))
        .and_then(|path| Store::open(&path));
    kept.or_else(|error| {
        tracing::error!("the chats last for this run only: {error:#}");
        Store::in_memory()
    })
    .expect("SQLite opens a database in memory")
}
