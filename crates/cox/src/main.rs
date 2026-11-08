//! `cox [--stage <url>] [--home <dir>] [--data <dir>] [<agent command> [args...]]`
//! opens a window where the human opens chats with `<agent command>`, or
//! the agent of `agents.json` the settings name when the system opens Cox
//! with none, each
//! in the project of a folder of the human's choice, working there, or in
//! Cox's own, of no project, working in the `--home` directory, `~/cox`
//! unless asked.
//! Each chat has a stage of its own, showing `<url>` first, which its agent
//! can read and drive too. The chats are kept as they go, to read after Cox
//! starts again, and to go on with, where this Cox keeps what lasts, or in
//! the `--data` directory when asked (see [`cox::storage`]).
//!
//! On macOS, run it from inside the bundle `cargo xtask bundle` makes, as
//! `Cox.app/Contents/MacOS/Cox`: that is where CEF and its helpers are.
//!
//! The chats run in coxd, `cox [--data <dir>] --daemon`, which the window
//! starts when it does not run, and which goes on after the window closes
//! (see `cox::daemon::process`).
//!
//! The agents run `cox` too, as `cox --mcp <socket>`, the MCP server of
//! Cox's own tools that `cox::agent::mcp` hands them. An agent that debugs
//! Cox runs `cox [--data <dir>] --inspect <tool>` to ask a running Cox how
//! it stands, and to drive it (see `cox::inspect`). `cox [--data <dir>]
//! --phone-link` prints the link that pairs a phone with the page coxd
//! serves it (see `cox::daemon::phone`). Cox reads a web page
//! for `raw_keep` as `cox --read-page <url>`, apart from itself (see
//! `cox::raw::page`). And on Linux, CEF runs
//! `cox` for its helper processes, which `cox_browser` tells apart.

use std::process::ExitCode;

use cox::app::{self, Mode};

fn main() -> ExitCode {
    match Mode::parse(std::env::args().skip(1)) {
        Mode::App(settings) => {
            // First, while Cox has no other thread.
            let environment = cox::login::take_in();
            if let Some(data) = &settings.data {
                cox::storage::keep_in(data);
            }
            // On Linux, CEF runs `cox` again for its helper processes,
            // which end inside `app::run`, and keep no file of their own.
            let helper = std::env::args().any(|arg| arg.starts_with("--type="));
            let logs = (!helper)
                .then(|| cox::storage::path(cox::storage::Kept::Logs))
                .flatten();
            cox::logs::init(logs.as_deref());
            if !helper {
                cox::scratch::sweep();
            }
            match environment {
                cox::login::Taken::NotNeeded => {}
                cox::login::Taken::From(shell, count) => tracing::info!(
                    "took in {count} variables of the environment of {}",
                    shell.display()
                ),
                cox::login::Taken::Failed(why) => tracing::warn!(
                    "the agents get launchd's environment, not the login shell's: {why}"
                ),
            }
            app::run(settings)
        }
        Mode::Daemon { data } => {
            // First, while coxd has no other thread.
            let environment = cox::login::take_in();
            if let Some(data) = &data {
                cox::storage::keep_in(data);
            }
            cox::logs::init(cox::storage::path(cox::storage::Kept::Logs).as_deref());
            cox::scratch::sweep();
            if let cox::login::Taken::Failed(why) = environment {
                tracing::warn!(
                    "the agents get launchd's environment, not the login shell's: {why}"
                );
            }
            app::daemon()
        }
        Mode::Bridge(socket) => {
            cox::logs::init(None);
            match cox::agent::mcp::bridge(&socket) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("cox: {error:#}");
                    ExitCode::from(1)
                }
            }
        }
        Mode::Inspect { data, args } => {
            if let Some(data) = &data {
                cox::storage::keep_in(data);
            }
            cox::logs::init(None);
            let Some(socket) = cox::inspect::socket() else {
                eprintln!("cox: there is no place for the socket of a Cox run from here");
                return ExitCode::from(1);
            };
            match cox::inspect::client::run(&socket, &args) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("cox: {error:#}");
                    ExitCode::from(1)
                }
            }
        }
        Mode::PhoneLink { data } => {
            if let Some(data) = &data {
                cox::storage::keep_in(data);
            }
            match cox::daemon::phone::link() {
                Ok(link) => {
                    print!("{link}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("cox: {error:#}");
                    ExitCode::from(1)
                }
            }
        }
        // A page of anyone's, read in a process of its own (see
        // `cox::raw::page`), which keeps no file.
        Mode::ReadPage(url) => cox::raw::page::run(&url),
        Mode::Usage => {
            println!("{}", app::USAGE);
            ExitCode::SUCCESS
        }
    }
}
