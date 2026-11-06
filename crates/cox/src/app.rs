//! Running Cox as the app, from what the command line says to the window
//! open, in the order things start:
//!
//! 1. The command line says what to be ([`Mode::parse`]): the app, with
//!    its [`Settings`], or the bridge to a chat's tools, which `main`
//!    runs instead.
//! 2. What the app depends on starts: the browser, which is CEF (see
//!    [`cox_browser`]), the signal handlers, and the store of the
//!    chats.
//! 3. The app's own parts are made: the [`Launcher`] that opens chats.
//! 4. GPUI runs the app: the keys are bound, the globals set, the loops
//!    that keep the browser and the quit asks served start, and the
//!    window opens, telling the human of its chats on macOS.
//! 5. Quitting runs what was registered for it: the browser shuts down
//!    before GPUI ends. On macOS, quitting ends the process there, so
//!    nothing runs after `run`.
//!
//! [`run`] is steps 2 to 5, each a function of its own, called in that
//! order.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use cox_core::store::Store;
use gpui::{
    App, Application, Bounds, Focusable, WindowBounds, WindowHandle, WindowOptions, actions,
    prelude::*, px, size,
};

use crate::agents::Agents;
use crate::chat::launcher::Launcher;
use crate::stage::servers::Servers;
use crate::ui::icons::Icons;
use crate::workspace::Workspace;
use cox_browser::Cef;

actions!(cox, [Quit]);

pub const USAGE: &str = "\
Usage: cox [--stage <url>] [--home <dir>] [--data <dir>] [<agent command> [args...]]
       cox [--data <dir>] --inspect [<tool> [<arguments>...]]";

/// What the command line asks `cox` to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    /// The app, as `cox [--stage <url>] [--home <dir>] [--data <dir>]
    /// <agent command>`.
    App(Settings),
    /// The bridge between an agent and the server of its chat's tools on
    /// the socket given, as `cox --mcp <socket>` (see [`crate::agent::mcp`]).
    Bridge(PathBuf),
    /// The client that asks a running Cox how it stands, and drives it,
    /// as `cox [--data <dir>] --inspect <tool> [<arguments>...]` (see
    /// [`crate::inspect::client`]): the Cox that keeps what lasts where
    /// `--data` says, or where this one does.
    Inspect {
        data: Option<PathBuf>,
        args: Vec<String>,
    },
    /// The usage, as `cox --help`.
    Usage,
}

/// What the command line gives the app.
#[derive(Debug, PartialEq, Eq)]
pub struct Settings {
    /// The page each chat's stage shows first.
    pub stage: String,
    /// Where chats in no project work for this run, if not where the
    /// settings say, or `~/cox` (see [`crate::settings`]).
    pub home: Option<PathBuf>,
    /// Where Cox keeps what lasts across its runs, if not where it keeps
    /// it from the place it runs from (see [`crate::storage`]).
    pub data: Option<PathBuf>,
    /// The agent each chat starts with, and its arguments, or none when
    /// the system opens Cox, as from the Finder, which then starts the
    /// agent of `agents.json` the settings name (see
    /// [`crate::settings::agent`]).
    pub command: Vec<String>,
}

impl Mode {
    /// What `args`, the command line without the program's name, asks
    /// for.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let args: Vec<String> = args.into_iter().collect();
        if let Some(socket) = crate::agent::mcp::bridge_to(&args) {
            return Self::Bridge(socket);
        }
        let settings = Settings::parse(args);
        if settings.command.first().map(String::as_str) == Some(crate::inspect::OPTION) {
            return Self::Inspect {
                data: settings.data,
                args: settings.command[1..].to_vec(),
            };
        }
        if matches!(
            settings.command.first().map(String::as_str),
            Some("--help" | "-h")
        ) {
            return Self::Usage;
        }
        Self::App(settings)
    }
}

impl Settings {
    fn parse(args: Vec<String>) -> Self {
        let mut args = args.into_iter().peekable();
        let mut stage = "about:blank".to_owned();
        let mut home = None;
        let mut data = None;
        loop {
            match args.peek().map(String::as_str) {
                Some("--stage") => {
                    args.next();
                    stage = args.next().unwrap_or(stage);
                }
                Some("--home") => {
                    args.next();
                    home = args.next().map(PathBuf::from).or(home);
                }
                Some("--data") => {
                    args.next();
                    data = args.next().map(PathBuf::from).or(data);
                }
                _ => break,
            }
        }
        Self {
            stage,
            home,
            data,
            command: args.collect(),
        }
    }
}

/// Runs the app as `settings` say, until the human quits it. The chats
/// that worked in the sketches follow them when they moved, from the
/// first of `sketches` to the second.
#[must_use]
pub fn run(settings: Settings, sketches: Option<(PathBuf, PathBuf)>) -> ExitCode {
    // CEF first: on Linux, its helper processes run this same executable
    // with arguments of their own, and exit inside `start`.
    let cef = cox_browser::start(crate::storage::path(crate::storage::Kept::Stage));
    let devtools = cef.devtools_port();
    // In place of CEF's handlers, which quit without the app's quit handlers.
    if let Err(error) = cox_browser::quit::on_signals() {
        tracing::error!("signals will end Cox without stopping the agents: {error}");
    }
    let store = Rc::new(open_store());
    if let Some((from, to)) = sketches
        && let Err(error) = store.moved(&from, &to)
    {
        tracing::error!(
            "the chats that worked in {} do not find their sketches: {error:#}",
            from.display()
        );
    }

    let home_given = settings.home.is_some();
    let home = settings
        .home
        .or_else(|| crate::settings::Saved::read().home);
    if home.is_none()
        && let Some(user) = std::env::var_os("HOME")
        && let Some((from, to)) = lowercase_home(Path::new(&user))
        && let Err(error) = store.moved(&from, &to)
    {
        tracing::error!(
            "the chats that worked in {} do not find it: {error:#}",
            from.display()
        );
    }
    let home = crate::settings::home_for_chats(home);
    // The workflow and the wiki move after the home, which may have been
    // renamed.
    crate::settings::settle_folders();
    // Opened from the Finder, the Dock or a launcher, Cox is given no
    // command, and starts the agent the settings name.
    let named = settings
        .command
        .is_empty()
        .then(crate::settings::agent)
        .flatten();
    let agents = Agents::read(settings.command, named.as_deref());
    agents.fetch();
    crate::translate::start(agents.clone());
    let followed = agents.clone();
    let launcher = Launcher {
        agents,
        stage: settings.stage,
        path: std::env::var_os("PATH").unwrap_or_default(),
        exe: std::env::current_exe()
            .inspect_err(|error| {
                tracing::error!("the agents get no tools of Cox's own: {error}");
            })
            .ok(),
        cef: cef.clone(),
    };

    Application::new()
        .with_assets(Icons)
        .run(move |cx: &mut App| {
            bind_keys(cx);
            cx.set_global(Servers::default());
            cx.set_global(followed.clone());
            serve_browser(&cef, cx);
            quit_when_asked(cx);
            shut_down_on_quit(cef, cx);
            let window = open_window(home, home_given, store, launcher, cx);
            crate::settings::follow(window, followed, cx);
            crate::inspect::start(window, devtools, cx);
        });
    ExitCode::SUCCESS
}

/// The name an earlier Cox gave the home, `~/cox` now.
const EARLIER_HOME: &str = "Cox";

/// Renames the home an earlier Cox made in `user`, the home directory,
/// `Cox`, to what Cox names it now, `cox`, unless there is a `cox` there
/// too, saying where it was and where it is now. The names are compared
/// as the folder lists them, since on macOS `cox` finds `Cox` too.
fn lowercase_home(user: &Path) -> Option<(PathBuf, PathBuf)> {
    let names: Vec<_> = std::fs::read_dir(user)
        .ok()?
        .filter_map(|entry| Some(entry.ok()?.file_name()))
        .collect();
    if !names.iter().any(|name| name == EARLIER_HOME)
        || names.iter().any(|name| name == crate::settings::HOME)
    {
        return None;
    }
    let (from, to) = (user.join(EARLIER_HOME), user.join(crate::settings::HOME));
    if !std::fs::symlink_metadata(&from).is_ok_and(|metadata| metadata.is_dir()) {
        return None;
    }
    match std::fs::rename(&from, &to) {
        Ok(()) => {
            tracing::info!("renamed {} to {}", from.display(), to.display());
            Some((from, to))
        }
        Err(error) => {
            tracing::warn!("{} stays as it was: {error}", from.display());
            None
        }
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

/// Binds the keys of every part of the window, and of quitting.
fn bind_keys(cx: &mut App) {
    crate::ui::input::bind_keys(cx);
    crate::ui::translation::bind_keys(cx);
    crate::settings::view::bind_keys(cx);
    crate::chat::view::bind_keys(cx);
    crate::stage::document::bind_keys(cx);
    crate::stage::code::bind_keys(cx);
    crate::stage::bind_keys(cx);
    crate::workspace::bind_keys(cx);
    // After the window's, which a terminal's own keys stand before.
    crate::stage::terminal::bind_keys(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    #[cfg(target_os = "macos")]
    cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);
}

/// How often GPUI gives CEF's UI thread a turn on macOS, when CEF does not
/// ask for one sooner (see [`cox_browser::macos::pump_wanted`]).
#[cfg(target_os = "macos")]
const PUMP_INTERVAL: Duration = Duration::from_millis(8);

/// How often the app looks whether it was asked to quit from where it
/// could not quit right away (see [`cox_browser::quit`]).
const QUIT_CHECK_INTERVAL: Duration = Duration::from_millis(50);

/// Gives CEF what it needs of the app while the app runs: on macOS, the
/// app's class, and turns on the main thread, which CEF's UI thread is
/// there. On Linux, CEF has a thread of its own, and needs nothing.
#[cfg(target_os = "macos")]
fn serve_browser(cef: &Cef, cx: &mut App) {
    // Before any browser exists: CEF asks the app about itself.
    cox_browser::macos::adopt_cef_app_protocol();
    let pumped = cef.clone();
    cx.spawn(async move |cx| {
        loop {
            tokio::select! {
                () = cx.background_executor().timer(PUMP_INTERVAL) => {}
                () = cox_browser::macos::pump_wanted() => {}
            }
            pumped.pump();
        }
    })
    .detach();
}

#[cfg(target_os = "linux")]
fn serve_browser(_cef: &Cef, _cx: &mut App) {}

/// Quits once something that could not quit right away asked to.
fn quit_when_asked(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(QUIT_CHECK_INTERVAL).await;
            if cox_browser::quit::take_asked() {
                let _ = cx.update(|cx| cx.quit());
            }
        }
    })
    .detach();
}

/// Shuts the browser down when the app quits, then takes away what this
/// run kept only while it ran (see [`crate::scratch`]). Quitting ends the
/// app's run loop, or on macOS the whole process, so the browsers go
/// before either. The agents go with the window.
fn shut_down_on_quit(cef: Cef, cx: &mut App) {
    cx.on_app_quit(move |_| {
        cef.clone().shutdown();
        crate::scratch::take_away();
        async {}
    })
    .detach();
}

/// Opens the window, with no chat in it, listing the chats kept in
/// `store`, where `launcher` opens chats, and chats in no project work in
/// `home`, which `--home` gave when `home_given` says so. The app quits
/// when the window closes.
fn open_window(
    home: PathBuf,
    home_given: bool,
    store: Rc<Store>,
    launcher: Launcher,
    cx: &mut App,
) -> WindowHandle<Workspace> {
    let opens = Rc::new(launcher);
    let bounds = Bounds::centered(None, size(px(1400.), px(860.)), cx);
    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| Workspace::new(home, home_given, store, opens, cx)),
        )
        .expect("the window opens");
    window
        .update(cx, |workspace, window, cx| {
            window.focus(&workspace.focus_handle(cx));
        })
        .expect("the window is open");
    cx.on_window_closed(|cx| cx.quit()).detach();
    tell_of_chats(window, cx);
    cx.activate(true);
    window
}

/// Tells the human of the chats in `window` with the system's
/// notifications, and the Dock's badge, and brings up the chat of a
/// notification they click.
#[cfg(target_os = "macos")]
fn tell_of_chats(window: WindowHandle<Workspace>, cx: &mut App) {
    let (clicked, mut clicks) = tokio::sync::mpsc::unbounded_channel();
    let notices = crate::notices::macos::System::start(clicked);
    cx.set_global(crate::notices::AppNotices(Rc::new(notices)));
    cx.spawn(async move |cx| {
        while let Some(chat) = clicks.recv().await {
            let _ = window.update(cx, |workspace, window, cx| {
                workspace.reveal(chat, window, cx);
                window.activate_window();
            });
            let _ = cx.update(|cx| cx.activate(true));
        }
    })
    .detach();
}

/// Elsewhere, Cox tells of nothing.
#[cfg(target_os = "linux")]
fn tell_of_chats(_window: WindowHandle<Workspace>, _cx: &mut App) {}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Mode, Settings};

    fn parse(words: &[&str]) -> Mode {
        Mode::parse(words.iter().map(|&word| word.to_owned()))
    }

    #[test]
    fn the_command_is_everything_after_the_options() {
        assert_eq!(
            parse(&[
                "--stage",
                "http://localhost:3000/",
                "--home",
                "/tmp/cox",
                "--data",
                "/tmp/cox-data",
                "node",
                "agent.mjs"
            ]),
            Mode::App(Settings {
                stage: "http://localhost:3000/".to_owned(),
                home: Some(PathBuf::from("/tmp/cox")),
                data: Some(PathBuf::from("/tmp/cox-data")),
                command: vec!["node".to_owned(), "agent.mjs".to_owned()],
            })
        );
    }

    #[test]
    fn the_stage_is_blank_and_home_and_data_are_the_defaults_unless_asked() {
        assert_eq!(
            parse(&["node", "agent.mjs"]),
            Mode::App(Settings {
                stage: "about:blank".to_owned(),
                home: None,
                data: None,
                command: vec!["node".to_owned(), "agent.mjs".to_owned()],
            })
        );
    }

    #[test]
    fn without_a_command_the_app_starts_the_agent_the_settings_name() {
        assert_eq!(
            parse(&[]),
            Mode::App(Settings {
                stage: "about:blank".to_owned(),
                home: None,
                data: None,
                command: Vec::new(),
            })
        );
    }

    #[test]
    fn the_usage_is_asked_for_with_help() {
        assert_eq!(parse(&["--help"]), Mode::Usage);
        assert_eq!(parse(&["--stage", "about:blank", "-h"]), Mode::Usage);
    }

    #[test]
    fn the_bridge_is_asked_for_with_its_socket() {
        assert_eq!(
            parse(&["--mcp", "/tmp/cox-1-0/mcp"]),
            Mode::Bridge(PathBuf::from("/tmp/cox-1-0/mcp"))
        );
    }

    #[test]
    fn the_client_is_asked_for_with_its_tool_and_data() {
        assert_eq!(
            parse(&["--data", "/tmp/d", "--inspect", "key", "ctrl-n"]),
            Mode::Inspect {
                data: Some(PathBuf::from("/tmp/d")),
                args: vec!["key".to_owned(), "ctrl-n".to_owned()],
            }
        );
        assert_eq!(
            parse(&["--inspect"]),
            Mode::Inspect {
                data: None,
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn chats_in_no_project_work_in_cox_in_the_home_directory() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        assert_eq!(
            crate::settings::home_for_chats(None),
            PathBuf::from(home).join("cox")
        );
        assert!(crate::settings::home_for_chats(Some("relative".into())).is_absolute());
    }

    #[test]
    fn the_home_an_earlier_cox_made_is_renamed_unless_there_is_one_now() {
        let user = std::env::temp_dir().join(format!("cox-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&user);
        std::fs::create_dir_all(user.join("Cox")).unwrap();
        std::fs::write(user.join("Cox").join("notes.md"), "kept").unwrap();

        assert_eq!(
            super::lowercase_home(&user),
            Some((user.join("Cox"), user.join("cox")))
        );
        assert_eq!(
            std::fs::read_to_string(user.join("cox").join("notes.md")).unwrap(),
            "kept"
        );
        // Once renamed, it is not again.
        assert_eq!(super::lowercase_home(&user), None);
        let _ = std::fs::remove_dir_all(&user);
    }
}
