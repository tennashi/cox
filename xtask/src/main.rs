//! Development tasks for the Cox workspace, run as `cargo xtask <task>`.

use std::env;
use std::process::ExitCode;

const USAGE: &str = "\
Usage: cargo xtask <task>

Tasks:
    help    Show this message
";

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("help" | "-h" | "--help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(task) => {
            eprintln!("unknown task: {task}\n");
            eprint!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}
