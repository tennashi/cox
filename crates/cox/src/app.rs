//! Running Cox as the app, from what the command line says to the window
//! open, in the order things start:
//!
//! 1. The command line says what to be ([`Program::parse`]): the app, with
//!    its [`Settings`], or the bridge to a chat's tools, which `main`
//!    runs instead.
//! 2. What the app depends on starts: the browser, which is CEF (see
//!    [`crate::browser`]), the signal handlers, and the store of the
//!    chats.
//! 3. The app's own parts are made: the [`Launcher`] that opens chats.
//! 4. GPUI runs the app: the keys are bound, the globals set, the loops
//!    that keep the browser and the quit asks served start, and the
//!    window opens.
//! 5. Quitting runs what was registered for it: the browser shuts down
//!    before GPUI ends. On macOS, quitting ends the process there, so
//!    nothing runs after `run`.
//!
//! [`run`] is steps 2 to 5, each a function of its own, called in that
//! order.

use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use cox_core::store::Store;
use gpui::{
    App, Application, Bounds, Focusable, WindowBounds, WindowOptions, actions, prelude::*, px, size,
};

use crate::browser::Cef;
use crate::servers::Servers;
use crate::session::Launcher;
use crate::workspace::Workspace;

actions!(cox, [Quit]);

pub const USAGE: &str = "Usage: cox [--stage <url>] [--home <dir>] <agent command> [args...]";

/// What the command line asks `cox` to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Program {
    /// The app, as `cox [--stage <url>] [--home <dir>] <agent command>`.
    App(Settings),
    /// The bridge between an agent and the server of its chat's tools on
    /// the socket given, as `cox --mcp <socket>` (see [`crate::mcp`]).
    Bridge(PathBuf),
    /// Neither: the usage is to be shown.
    Usage,
}

/// What the command line gives the app.
#[derive(Debug, PartialEq, Eq)]
pub struct Settings {
    /// The page each chat's stage shows first.
    pub stage: String,
    /// Where chats in no project work, if not in `~/Cox`.
    pub home: Option<PathBuf>,
    /// The agent each chat starts, and its arguments.
    pub command: Vec<String>,
}

impl Program {
    /// What `args`, the command line without the program's name, asks
    /// for.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let args: Vec<String> = args.into_iter().collect();
        if let Some(socket) = crate::mcp::bridge_to(&args) {
            return Self::Bridge(socket);
        }
        let settings = Settings::parse(args);
        if settings.command.is_empty() {
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
                _ => break,
            }
        }
        Self {
            stage,
            home,
            command: args.collect(),
        }
    }
}

/// Runs the app as `settings` say, until the human quits it.
#[must_use]
pub fn run(settings: Settings) -> ExitCode {
    // CEF first: on Linux, its helper processes run this same executable
    // with arguments of their own, and exit inside `start`.
    let cef = crate::browser::start();
    // In place of CEF's handlers, which quit without the app's quit handlers.
    if let Err(error) = crate::quit::on_signals() {
        log::error!("signals will end Cox without stopping the agents: {error}");
    }
    let store = Rc::new(open_store());

    let home = home_for_sessions(settings.home);
    let launcher = Launcher {
        command: settings.command,
        stage: settings.stage,
        path: std::env::var_os("PATH").unwrap_or_default(),
        exe: std::env::current_exe()
            .inspect_err(|error| {
                log::error!("the agents get no tools of Cox's own: {error}");
            })
            .ok(),
        cef: cef.clone(),
    };

    Application::new().run(move |cx: &mut App| {
        bind_keys(cx);
        cx.set_global(Servers::default());
        serve_browser(&cef, cx);
        quit_when_asked(cx);
        shut_down_on_quit(cef, cx);
        open_window(home, store, launcher, cx);
    });
    ExitCode::SUCCESS
}

/// Where chats in no project work: `home`, or `~/Cox`, as an absolute
/// path, which the agent needs.
fn home_for_sessions(home: Option<PathBuf>) -> PathBuf {
    let home = home.unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join("Cox")
    });
    std::path::absolute(&home).unwrap_or(home)
}

/// The store of the chats this Cox keeps, or one for this run only when
/// that cannot be opened.
fn open_store() -> Store {
    let kept = crate::place::kept("chats.sqlite")
        .ok_or_else(|| anyhow::anyhow!("there is no place to keep them"))
        .and_then(|path| Store::open(&path));
    kept.or_else(|error| {
        log::error!("the chats last for this run only: {error:#}");
        Store::in_memory()
    })
    .expect("SQLite opens a database in memory")
}

/// Binds the keys of every part of the window, and of quitting.
fn bind_keys(cx: &mut App) {
    crate::input::bind_keys(cx);
    crate::view::bind_keys(cx);
    crate::document::bind_keys(cx);
    crate::code::bind_keys(cx);
    crate::workspace::bind_keys(cx);
    // After the window's, which a terminal's own keys stand before.
    crate::terminal::bind_keys(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
    #[cfg(target_os = "macos")]
    cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);
}

/// How often GPUI gives CEF's UI thread a turn on macOS: a little above
/// the stage's frame rate, so a frame never waits for the next turn.
#[cfg(target_os = "macos")]
const PUMP_INTERVAL: Duration = Duration::from_millis(8);

/// How often the app looks whether it was asked to quit from where it
/// could not quit right away (see [`crate::quit`]).
const QUIT_CHECK_INTERVAL: Duration = Duration::from_millis(50);

/// Gives CEF what it needs of the app while the app runs: on macOS, the
/// app's class, and turns on the main thread, which CEF's UI thread is
/// there. On Linux, CEF has a thread of its own, and needs nothing.
#[cfg(target_os = "macos")]
fn serve_browser(cef: &Cef, cx: &mut App) {
    // Before any browser exists: CEF asks the app about itself.
    crate::macos::adopt_cef_app_protocol();
    let pumped = cef.clone();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(PUMP_INTERVAL).await;
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
            if crate::quit::take_asked() {
                let _ = cx.update(|cx| cx.quit());
            }
        }
    })
    .detach();
}

/// Shuts the browser down when the app quits. Quitting ends the app's run
/// loop, or on macOS the whole process, so the browsers go before either.
/// The agents go with the window.
fn shut_down_on_quit(cef: Cef, cx: &mut App) {
    cx.on_app_quit(move |_| {
        cef.clone().shutdown();
        async {}
    })
    .detach();
}

/// Opens the window, with no chat in it, listing the chats kept in
/// `store`, where `launcher` opens chats, and chats in no project work in
/// `home`. The app quits when the window closes.
fn open_window(home: PathBuf, store: Rc<Store>, launcher: Launcher, cx: &mut App) {
    let opens = Rc::new(launcher);
    let bounds = Bounds::centered(None, size(px(1400.), px(860.)), cx);
    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| Workspace::new(home, store, opens, cx)),
        )
        .expect("the window opens");
    window
        .update(cx, |workspace, window, cx| {
            window.focus(&workspace.focus_handle(cx));
        })
        .expect("the window is open");
    cx.on_window_closed(|cx| cx.quit()).detach();
    cx.activate(true);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Program, Settings};

    fn parse(words: &[&str]) -> Program {
        Program::parse(words.iter().map(|&word| word.to_owned()))
    }

    #[test]
    fn the_command_is_everything_after_the_options() {
        assert_eq!(
            parse(&[
                "--stage",
                "http://localhost:3000/",
                "--home",
                "/tmp/cox",
                "node",
                "agent.mjs"
            ]),
            Program::App(Settings {
                stage: "http://localhost:3000/".to_owned(),
                home: Some(PathBuf::from("/tmp/cox")),
                command: vec!["node".to_owned(), "agent.mjs".to_owned()],
            })
        );
    }

    #[test]
    fn the_stage_is_blank_and_home_is_the_default_unless_asked() {
        assert_eq!(
            parse(&["node", "agent.mjs"]),
            Program::App(Settings {
                stage: "about:blank".to_owned(),
                home: None,
                command: vec!["node".to_owned(), "agent.mjs".to_owned()],
            })
        );
    }

    #[test]
    fn without_a_command_the_usage_is_asked_for() {
        assert_eq!(parse(&[]), Program::Usage);
        assert_eq!(parse(&["--stage", "about:blank"]), Program::Usage);
    }

    #[test]
    fn the_bridge_is_asked_for_with_its_socket() {
        assert_eq!(
            parse(&["--mcp", "/tmp/cox-1-0/mcp"]),
            Program::Bridge(PathBuf::from("/tmp/cox-1-0/mcp"))
        );
    }

    #[test]
    fn chats_in_no_project_work_in_cox_in_the_home_directory() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        assert_eq!(
            super::home_for_sessions(None),
            PathBuf::from(home).join("Cox")
        );
        assert!(super::home_for_sessions(Some("relative".into())).is_absolute());
    }
}
