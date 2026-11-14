//! Running Cox as the app, from what the command line says to the window
//! open, in the order things start:
//!
//! 1. The command line says what to be ([`Mode::parse`]): the app, with
//!    its [`Settings`], coxd, which runs the chats, or the bridge to a
//!    chat's tools, which `main` runs instead.
//! 2. What the app depends on starts: the browser, which is CEF (see
//!    [`cox_browser`]), and the signal handlers.
//! 3. The window reaches coxd, which runs the chats, starting it when it
//!    does not run (see [`crate::daemon::client`]).
//! 4. GPUI runs the app: the keys are bound, the globals set, the loops
//!    that keep the browser and the quit asks served start, and the
//!    window opens, telling the human of its chats on macOS.
//! 5. Quitting runs what was registered for it: the browser shuts down
//!    before GPUI ends, and coxd goes on. On macOS, quitting ends the
//!    process there, so nothing runs after `run`.
//!
//! [`run`] is steps 2 to 5, each a function of its own, called in that
//! order. coxd runs apart (see [`crate::daemon::main`]).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Application, Bounds, Focusable, WindowBounds, WindowHandle, WindowOptions, actions,
    prelude::*, px, size,
};

use crate::agents::Agents;
use crate::chat::launcher::CefStages;
use crate::stage::servers::Servers;
use crate::ui::icons::Icons;
use crate::workspace::Workspace;
use cox_browser::Cef;

actions!(cox, [Quit]);

pub const USAGE: &str = "\
Usage: cox [--stage <url>] [--home <dir>] [--data <dir>] [<agent command> [args...]]
       cox [--data <dir>] --daemon
       cox [--data <dir>] --inspect [<tool> [<arguments>...]]
       cox [--data <dir>] --phone-link
       cox --read-page <url>";

/// What the command line says to be coxd, after `--data <dir>`.
pub const DAEMON: &str = "--daemon";

/// What the command line says to print the link that pairs a phone,
/// after `--data <dir>`.
pub const PHONE_LINK: &str = "--phone-link";

/// What the command line asks `cox` to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    /// The app, as `cox [--stage <url>] [--home <dir>] [--data <dir>]
    /// <agent command>`.
    App(Settings),
    /// coxd, which runs the chats, as `cox [--data <dir>] --daemon`, as
    /// the window starts it (see [`crate::daemon::process`]).
    Daemon { data: Option<PathBuf> },
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
    /// The link that pairs a phone with the page coxd serves it, as `cox
    /// [--data <dir>] --phone-link` (see [`crate::daemon::phone`]).
    PhoneLink { data: Option<PathBuf> },
    /// The reader of the web page at the URL given, for `raw_keep`, as
    /// `cox --read-page <url>` (see [`crate::raw::page`]).
    ReadPage(String),
    /// The usage, as `cox --help`.
    Usage,
}

/// What the command line gives the app.
#[derive(Debug, PartialEq, Eq)]
pub struct Settings {
    /// The page each chat's stage shows first.
    pub stage: String,
    /// Where Cox's own chats work for this run, if not where the
    /// settings say, or `~/cox` (see [`crate::settings`]).
    pub home: Option<PathBuf>,
    /// Where Cox keeps what lasts across its runs, if not where it keeps
    /// it from the place it runs from (see [`coxd::storage`]).
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
        if let Some(url) = crate::raw::page::reading(&args) {
            return Self::ReadPage(url.to_owned());
        }
        let settings = Settings::parse(args);
        if settings.command.first().map(String::as_str) == Some(crate::inspect::OPTION) {
            return Self::Inspect {
                data: settings.data,
                args: settings.command[1..].to_vec(),
            };
        }
        if settings.command.first().map(String::as_str) == Some(DAEMON) {
            return Self::Daemon {
                data: settings.data,
            };
        }
        if settings.command.first().map(String::as_str) == Some(PHONE_LINK) {
            return Self::PhoneLink {
                data: settings.data,
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
        let mut stage = crate::stage::links::ACTIVITY_URL.to_owned();
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

/// Runs the app as `settings` say, until the human quits it.
#[must_use]
pub fn run(settings: Settings) -> ExitCode {
    let profile = coxd::storage::path(coxd::storage::Kept::Stage);
    // On Linux, CEF's helper processes run this same executable with
    // arguments of their own, and exit inside `start`, before they reach
    // coxd.
    if std::env::args().any(|arg| arg.starts_with("--type=")) {
        let _ = cox_browser::start(profile.clone());
    }
    let home_given = settings.home.is_some();
    let home = settings
        .home
        .clone()
        .or_else(|| crate::settings::Saved::read().home);
    let home = crate::settings::home_for_chats(home);
    // Opened from the Finder, the Dock or a launcher, Cox is given no
    // command, and starts the agent the settings name.
    let named = settings
        .command
        .is_empty()
        .then(crate::settings::agent)
        .flatten();
    let agents = Agents::read(settings.command.clone(), named.as_deref());
    agents.fetch();
    crate::translate::start(agents.clone());
    crate::trigger::discuss::start(agents.clone());
    // 3. coxd, which runs the chats, in a process of its own.
    let hello = crate::daemon::wire::Hello::new(settings.home, settings.command);
    let data = settings.data;
    let connected = coxd::storage::path(coxd::storage::Kept::Daemon)
        .ok_or_else(|| anyhow::anyhow!("there is no place for coxd's socket"))
        .and_then(|socket| {
            crate::daemon::client::connect(
                &socket,
                &hello,
                &|| start_daemon(data.as_deref()),
                &tell_waiting,
                // The window shows nothing once coxd is gone.
                cox_core::quit::ask,
            )
        });
    let crate::daemon::client::Connected { link, from_daemon } = match connected {
        Ok(connected) => connected,
        Err(error) => {
            tracing::error!("{error:#}");
            eprintln!("cox: {error:#}");
            return ExitCode::from(1);
        }
    };
    // CEF after coxd is started, which would hold what CEF opens
    // otherwise.
    let cef = cox_browser::start(profile);
    let devtools = cef.devtools_port();
    // In place of CEF's handlers, which quit without the app's quit handlers.
    if let Err(error) = cox_core::quit::on_signals() {
        tracing::error!("signals will end Cox without stopping the agents: {error}");
    }
    let stages = CefStages {
        stage: settings.stage,
        cef: cef.clone(),
    };

    Application::new()
        .with_assets(Icons)
        .run(move |cx: &mut App| {
            bind_keys(cx);
            crate::vim::turn(crate::settings::vim(), cx);
            cx.set_global(Servers::default());
            cx.set_global(crate::settings::window::AppAgents(agents.clone()));
            serve_browser(&cef, cx);
            crate::busy::watch(cx);
            quit_when_asked(cx);
            let inspected = cef.clone();
            shut_down_on_quit(cef, cx);
            let workspace = Opened {
                link,
                from_daemon,
                home,
                home_given,
                stages: Rc::new(stages),
                agents: agents.clone(),
            };
            let window = open_window(workspace, cx);
            crate::settings::window::follow(window, agents, cx);
            crate::inspect::start(window, devtools, inspected, cx);
        });
    ExitCode::SUCCESS
}

/// Tells the human why the window does not open yet: the coxd that runs
/// the chats is of another build, which this window may not understand,
/// and stops for this one only once none of its chats is at work.
fn tell_waiting() {
    const WAITING: &str = "Another build of Cox runs the chats, and one of them is at \
                           work: this window opens once none is, with this build.";
    tracing::warn!("{WAITING}");
    eprintln!("cox: {WAITING}");
    #[cfg(target_os = "macos")]
    {
        let shown = std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "display notification \"{WAITING}\" with title \"Cox\""
            ))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if let Err(error) = shown {
            tracing::error!("the human is not told: {error}");
        }
    }
}

/// Starts coxd, as this program run as `cox [--data <dir>] --daemon`,
/// apart from the window: in a process group of its own, so that a
/// Control-C in the terminal the window started from leaves it, with
/// nothing to read or write but its log.
fn start_daemon(data: Option<&Path>) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};
    let mut command = Command::new(std::env::current_exe()?);
    if let Some(data) = data {
        command.arg("--data").arg(data);
    }
    command
        .arg(DAEMON)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    tracing::info!("started coxd");
    Ok(())
}

/// Binds the keys of every part of the window, and of quitting.
pub fn bind_keys(cx: &mut App) {
    crate::ui::input::bind_keys(cx);
    crate::ui::translation::bind_keys(cx);
    crate::chat::hand::bind_keys(cx);
    crate::chat::region::bind_keys(cx);
    crate::settings::view::bind_keys(cx);
    crate::chat::view::bind_keys(cx);
    crate::stage::document::bind_keys(cx);
    crate::stage::code::bind_keys(cx);
    crate::stage::bind_keys(cx);
    crate::stage::page::address::bind_keys(cx);
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
/// could not quit right away (see [`cox_core::quit`]).
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
            crate::busy::timed("CEF's turn", || pumped.pump());
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
            if cox_core::quit::take_asked() {
                let _ = cx.update(|cx| cx.quit());
            }
        }
    })
    .detach();
}

/// Shuts the browser down when the app quits, then takes away what this
/// run kept only while it ran (see [`coxd::scratch`]). Quitting ends the
/// app's run loop, or on macOS the whole process, so the browsers go
/// before either. coxd, and the chats' agents, go on.
fn shut_down_on_quit(cef: Cef, cx: &mut App) {
    cx.on_app_quit(move |_| {
        cef.clone().shutdown();
        coxd::scratch::take_away();
        async {}
    })
    .detach();
}

/// What the window is opened with: the daemon it shows the chats of, and
/// what it makes their stages with.
struct Opened {
    link: crate::daemon::Link,
    from_daemon: tokio::sync::mpsc::UnboundedReceiver<crate::daemon::wire::FromDaemon>,
    /// Where Cox's own chats work, which `--home` gave when
    /// `home_given` says so.
    home: PathBuf,
    home_given: bool,
    stages: Rc<CefStages>,
    agents: Agents,
}

/// Opens the window, with no chat in it, showing the chats of the daemon
/// `opened` says. The app quits when the window closes.
fn open_window(opened: Opened, cx: &mut App) -> WindowHandle<Workspace> {
    let Opened {
        link,
        from_daemon,
        home,
        home_given,
        stages,
        agents,
    } = opened;
    let bounds = Bounds::centered(None, size(px(1400.), px(860.)), cx);
    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        link,
                        from_daemon,
                        home,
                        home_given,
                        stages,
                        Some(agents),
                        window,
                        cx,
                    )
                })
            },
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

    /// The settings the app starts with when given only an agent's
    /// command.
    fn not_asked() -> Settings {
        match parse(&["node", "agent.mjs"]) {
            Mode::App(settings) => settings,
            mode => panic!("the app is not started: {mode:?}"),
        }
    }

    #[test]
    fn the_stage_shows_activity_unless_asked() {
        assert_eq!(not_asked().stage, "cox://activity/");
    }

    #[test]
    fn the_home_is_the_default_unless_asked() {
        assert_eq!(not_asked().home, None);
    }

    #[test]
    fn the_data_folder_is_the_default_unless_asked() {
        assert_eq!(not_asked().data, None);
    }

    #[test]
    fn without_a_command_the_app_starts_the_agent_the_settings_name() {
        assert_eq!(
            parse(&[]),
            Mode::App(Settings {
                stage: "cox://activity/".to_owned(),
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
    fn the_reader_of_a_page_is_asked_for_with_its_url() {
        assert_eq!(
            parse(&["--read-page", "https://example.com/"]),
            Mode::ReadPage("https://example.com/".to_owned())
        );
    }

    #[test]
    fn coxd_is_asked_for_with_its_data() {
        assert_eq!(
            parse(&["--data", "/tmp/d", "--daemon"]),
            Mode::Daemon {
                data: Some(PathBuf::from("/tmp/d")),
            }
        );
        assert_eq!(parse(&["--daemon"]), Mode::Daemon { data: None });
        assert_eq!(
            parse(&["--data", "/tmp/d", "--phone-link"]),
            Mode::PhoneLink {
                data: Some(PathBuf::from("/tmp/d"))
            }
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
    fn coxs_own_chats_work_in_cox_in_the_home_directory() {
        let user = || Some(PathBuf::from("/home/me"));
        assert_eq!(
            crate::settings::home_for_chats_of(None, user()),
            PathBuf::from("/home/me/cox")
        );
        assert_eq!(
            crate::settings::home_for_chats_of(Some("/work".into()), user()),
            PathBuf::from("/work")
        );
        assert!(crate::settings::home_for_chats_of(Some("relative".into()), user()).is_absolute());
    }
}
