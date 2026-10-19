//! Development tasks for the Cox workspace, run as `cargo xtask <task>`.

use std::env;

use anyhow::bail;
use xshell::{Shell, cmd};

const USAGE: &str = "\
Usage: cargo xtask <task>

Tasks:
    ci      Run every check that CI runs: format, lint and test
    help    Show this message
";

fn main() -> anyhow::Result<()> {
    let sh = Shell::new()?;
    match env::args().nth(1).as_deref() {
        Some("ci") => ci(&sh),
        None | Some("help" | "-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(task) => {
            eprint!("{USAGE}");
            bail!("unknown task: {task}")
        }
    }
}

/// Runs the checks in order and stops at the first one that fails.
fn ci(sh: &Shell) -> anyhow::Result<()> {
    cmd!(sh, "cargo fmt --all --check").run()?;
    cmd!(
        sh,
        "cargo clippy --workspace --all-targets -- --deny warnings"
    )
    .run()?;
    cmd!(sh, "cargo test --workspace").run()?;
    Ok(())
}
