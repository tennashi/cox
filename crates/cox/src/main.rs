//! `cox [--stage <url>] <agent command> [args...]` starts the agent and opens
//! a window to talk to it, with the stage showing `<url>`.

use gpui::{
    App, Application, Bounds, Focusable, WindowBounds, WindowOptions, prelude::*, px, size,
};

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

fn main() {
    env_logger::init();
    let args = Args::parse(std::env::args().skip(1));

    // CEF first: its helper processes run this same executable with
    // arguments of their own, and exit inside `start`.
    #[cfg(target_os = "linux")]
    let (browser, events) = cox::browser::start(args.stage.clone());
    // The stage is Linux only for now; the option still parses everywhere.
    #[cfg(not(target_os = "linux"))]
    let _ = &args.stage;

    if args.command.is_empty() {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let command = args.command;

    #[cfg(target_os = "linux")]
    let shutdown = browser.clone();
    #[cfg(target_os = "linux")]
    let mut events = Some(events);

    Application::new().run(move |cx: &mut App| {
        cox::input::bind_keys(cx);
        let agent = cox::agent::spawn(move || cox_acp::launch(command));
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    #[cfg(target_os = "linux")]
                    let stage = Some(cx.new(|cx| {
                        cox::stage::Stage::new(
                            browser.clone(),
                            events.take().expect("the window opens once"),
                            cx,
                        )
                    }));
                    #[cfg(not(target_os = "linux"))]
                    let stage = None;
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

    #[cfg(target_os = "linux")]
    shutdown.shutdown();
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
