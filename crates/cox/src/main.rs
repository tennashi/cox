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
//! Cox's own tools that `cox::agent::mcp` hands them. And on Linux, CEF runs
//! `cox` for its helper processes, which `cox_browser` tells apart.

use std::process::ExitCode;

use cox::app::{self, Mode};

fn main() -> ExitCode {
    cox::logs::init();
    match Mode::parse(std::env::args().skip(1)) {
        Mode::App(settings) => app::run(settings),
        Mode::Bridge(socket) => match cox::agent::mcp::bridge(&socket) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("cox: {error:#}");
                ExitCode::from(1)
            }
        },
        Mode::Usage => {
            eprintln!("{}", app::USAGE);
            ExitCode::from(2)
        }
    }
}
