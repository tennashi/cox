//! `cox [--stage <url>] [--home <dir>] <agent command> [args...]` opens a
//! window where the human opens chats with `<agent command>`, each in the
//! project of a folder of the human's choice, working there, or in no
//! project, working in `<dir>`, `~/Cox` unless asked. Each chat has a stage
//! of its own, showing `<url>` first, which its agent can read and drive
//! too. The chats are kept as they go, to read after Cox starts again, and
//! to go on with.
//!
//! On macOS, run it from inside the bundle `cargo xtask bundle` makes, as
//! `Cox.app/Contents/MacOS/Cox`: that is where CEF and its helpers are.
//!
//! The agents run `cox` too, as `cox --mcp <socket>`, the MCP server of
//! Cox's own tools that `cox::mcp` hands them.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    App, Application, Bounds, Focusable, WindowBounds, WindowOptions, actions, prelude::*, px, size,
};

actions!(cox, [Quit]);

const USAGE: &str = "Usage: cox [--stage <url>] [--home <dir>] <agent command> [args...]";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    /// The page each chat's stage shows first.
    stage: String,
    /// Where chats in no project work, if not in `~/Cox`.
    home: Option<PathBuf>,
    /// The agent each session starts, and its arguments.
    command: Vec<String>,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
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
fn open_store() -> cox_core::store::Store {
    let kept = cox::place::kept("chats.sqlite")
        .ok_or_else(|| anyhow::anyhow!("there is no place to keep them"))
        .and_then(|path| cox_core::store::Store::open(&path));
    kept.or_else(|error| {
        log::error!("the chats last for this run only: {error:#}");
        cox_core::store::Store::in_memory()
    })
    .expect("SQLite opens a database in memory")
}

/// What opens chats with `launcher`, and what shows closed ones.
fn chats(launcher: cox::session::Launcher) -> (cox::workspace::Open, cox::workspace::Read) {
    let launcher = Rc::new(launcher);
    let opens = Rc::clone(&launcher);
    let open: cox::workspace::Open = Rc::new(move |cwd, instructions, record, window, cx| {
        opens.open(cwd, instructions, record, window, cx)
    });
    let read: cox::workspace::Read =
        Rc::new(move |closed, window, cx| launcher.read(closed, window, cx));
    (open, read)
}

/// How often GPUI gives CEF's UI thread a turn on macOS: a little above
/// the stage's frame rate, so a frame never waits for the next turn.
#[cfg(target_os = "macos")]
const PUMP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);

/// How often the app looks whether it was asked to quit from where it
/// could not quit right away (see `cox::quit`).
const QUIT_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Before CEF, since the bridge runs no browser.
    if let Some(socket) = cox::mcp::bridge_to(&args) {
        if let Err(error) = cox::mcp::bridge(&socket) {
            eprintln!("cox: {error:#}");
            std::process::exit(1);
        }
        return;
    }
    let args = Args::parse(args);

    // CEF first: on Linux, its helper processes run this same executable
    // with arguments of their own, and exit inside `start`.
    let cef = cox::browser::start();
    // In place of CEF's handlers, which quit without the app's quit handlers.
    if let Err(error) = cox::quit::on_signals() {
        log::error!("signals will end Cox without stopping the agents: {error}");
    }

    if args.command.is_empty() {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let home = home_for_sessions(args.home);
    let store = Rc::new(open_store());
    let launcher = cox::session::Launcher {
        command: args.command,
        stage: args.stage,
        path: std::env::var_os("PATH").unwrap_or_default(),
        exe: std::env::current_exe()
            .inspect_err(|error| {
                log::error!("the agents get no tools of Cox's own: {error}");
            })
            .ok(),
        cef: cef.clone(),
    };

    Application::new().run(move |cx: &mut App| {
        cox::input::bind_keys(cx);
        cox::view::bind_keys(cx);
        cox::document::bind_keys(cx);
        cox::code::bind_keys(cx);
        cox::workspace::bind_keys(cx);
        // After the window's, which a terminal's own keys stand before.
        cox::terminal::bind_keys(cx);
        cx.set_global(cox::servers::Servers::default());
        cx.on_action(|_: &Quit, cx| cx.quit());
        #[cfg(target_os = "macos")]
        cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);

        #[cfg(target_os = "macos")]
        {
            // Before any browser exists: CEF asks the app about itself.
            cox::macos::adopt_cef_app_protocol();
            let pumped = cef.clone();
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor().timer(PUMP_INTERVAL).await;
                    pumped.pump();
                }
            })
            .detach();
        }
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(QUIT_CHECK_INTERVAL).await;
                if cox::quit::take_asked() {
                    let _ = cx.update(|cx| cx.quit());
                }
            }
        })
        .detach();
        // Quitting ends the app's run loop, or on macOS the whole process,
        // so the browsers go before either. The agents go with the window.
        {
            let closing = cef.clone();
            cx.on_app_quit(move |_| {
                closing.clone().shutdown();
                async {}
            })
            .detach();
        }

        let (open, read) = chats(launcher);
        let bounds = Bounds::centered(None, size(px(1400.), px(860.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| cox::workspace::Workspace::new(home, store, open, read, cx)),
            )
            .expect("the window opens");
        window
            .update(cx, |workspace, window, cx| {
                window.focus(&workspace.focus_handle(cx));
            })
            .expect("the window is open");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Args;

    fn args(words: &[&str]) -> Args {
        Args::parse(words.iter().map(|&word| word.to_owned()))
    }

    #[test]
    fn the_command_is_everything_after_the_options() {
        assert_eq!(
            args(&[
                "--stage",
                "http://localhost:3000/",
                "--home",
                "/tmp/cox",
                "node",
                "agent.mjs"
            ]),
            Args {
                stage: "http://localhost:3000/".to_owned(),
                home: Some(PathBuf::from("/tmp/cox")),
                command: vec!["node".to_owned(), "agent.mjs".to_owned()],
            }
        );
    }

    #[test]
    fn the_stage_is_blank_and_home_is_the_default_unless_asked() {
        assert_eq!(
            args(&["node", "agent.mjs"]),
            Args {
                stage: "about:blank".to_owned(),
                home: None,
                command: vec!["node".to_owned(), "agent.mjs".to_owned()],
            }
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
