//! `cox [--stage <url>] <agent command> [args...]` starts the agent and opens
//! a window to talk to it, with the stage showing `<url>`. The agent can
//! read and drive the stage too.
//!
//! On macOS, run it from inside the bundle `cargo xtask bundle` makes, as
//! `Cox.app/Contents/MacOS/Cox`: that is where CEF and its helpers are.

use gpui::{
    App, Application, Bounds, Focusable, WindowBounds, WindowOptions, actions, prelude::*, px, size,
};

actions!(cox, [Quit]);

const USAGE: &str = "Usage: cox [--stage <url>] <agent command> [args...]";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    /// The page the stage shows.
    stage: String,
    /// The agent to start, and its arguments.
    command: Vec<String>,
}

impl Args {
    fn parse(args: impl IntoIterator<Item = String>) -> Self {
        let mut args = args.into_iter().peekable();
        let mut stage = "about:blank".to_owned();
        if args.peek().is_some_and(|arg| arg == "--stage")
            && let Some(url) = args.nth(1)
        {
            stage = url;
        }
        Self {
            stage,
            command: args.collect(),
        }
    }
}

/// How often GPUI gives CEF's UI thread a turn on macOS: a little above
/// the stage's frame rate, so a frame never waits for the next turn.
#[cfg(target_os = "macos")]
const PUMP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(8);

/// How often the app looks whether it was asked to quit from where it
/// could not quit right away (see `cox::quit`).
#[cfg(any(target_os = "linux", target_os = "macos"))]
const QUIT_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

fn main() {
    env_logger::init();
    let args = Args::parse(std::env::args().skip(1));

    // CEF first: on Linux, its helper processes run this same executable
    // with arguments of their own, and exit inside `start`.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let (browser, events) = cox::browser::start(args.stage.clone());
    // Elsewhere the option still parses, and does nothing.
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = &args.stage;
    // In place of CEF's handlers, which quit without the app's quit handlers.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Err(error) = cox::quit::on_signals() {
        log::error!("signals will end Cox without stopping the agent: {error}");
    }

    if args.command.is_empty() {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let command = args.command;

    // The agent's browser tools go through the relay, which tells the
    // stage where the agent points, clicks and types.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let (relay_port, agent_input) = match cox::relay::start(cox::browser::DEVTOOLS_PORT) {
        Ok((port, input)) => (Some(port), input),
        Err(error) => {
            log::error!("no relay to the stage, so the agent gets no tools for it: {error:#}");
            (None, tokio::sync::mpsc::unbounded_channel().1)
        }
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let mut stage_input = Some((events, agent_input));

    Application::new().run(move |cx: &mut App| {
        cox::input::bind_keys(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        #[cfg(target_os = "macos")]
        cx.bind_keys([gpui::KeyBinding::new("cmd-q", Quit, None)]);
        // The agent gets the page the human sees.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let tools = relay_port
            .and_then(|port| cox::tools::stage(port, &std::env::var_os("PATH").unwrap_or_default()))
            .into_iter()
            .collect();
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let tools = Vec::new();
        let agent = cox::agent::spawn(move || cox_acp::launch(command), tools);

        #[cfg(target_os = "macos")]
        {
            // Before the browser exists: CEF asks the app about itself.
            cox::macos::adopt_cef_app_protocol();
            let pumped = browser.clone();
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor().timer(PUMP_INTERVAL).await;
                    pumped.pump();
                }
            })
            .detach();
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
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
        // so the browser goes before either. The agent goes with the
        // window.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let closing = browser.clone();
            cx.on_app_quit(move |_| {
                closing.clone().shutdown();
                async {}
            })
            .detach();
        }

        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    #[cfg(any(target_os = "linux", target_os = "macos"))]
                    let stage = Some(cx.new(|cx| {
                        let (events, agent_input) =
                            stage_input.take().expect("the window opens once");
                        cox::stage::Stage::new(browser.clone(), events, agent_input, window, cx)
                    }));
                    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                    let _ = window;
                    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                    let stage = None::<gpui::AnyView>;
                    cx.new(|cx| cox::view::CoxView::new(agent, stage, cx))
                },
            )
            .expect("the window opens");
        window
            .update(cx, |view, window, cx| window.focus(&view.focus_handle(cx)))
            .expect("the window is open");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::Args;

    fn args(words: &[&str]) -> Args {
        Args::parse(words.iter().map(|&word| word.to_owned()))
    }

    #[test]
    fn the_command_is_everything_after_the_options() {
        assert_eq!(
            args(&["--stage", "http://localhost:3000/", "node", "agent.mjs"]),
            Args {
                stage: "http://localhost:3000/".to_owned(),
                command: vec!["node".to_owned(), "agent.mjs".to_owned()],
            }
        );
    }

    #[test]
    fn the_stage_is_blank_unless_asked() {
        assert_eq!(
            args(&["node", "agent.mjs"]),
            Args {
                stage: "about:blank".to_owned(),
                command: vec!["node".to_owned(), "agent.mjs".to_owned()],
            }
        );
    }
}
