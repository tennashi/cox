//! Development tasks for the Cox workspace, run as `cargo xtask <task>`.

use std::env;

use anyhow::{Context, bail};
use xshell::{Shell, cmd};

/// Development tools that `setup` installs, pinned to exact versions so that
/// every machine and CI run the same ones.
const TOOLS: &[(&str, &str)] = &[("cargo-deny", "0.20.2")];

const USAGE: &str = "\
Usage: cargo xtask <task>

Tasks:
    ci      Run every check that CI runs: format, lint, test and dependencies
    setup   Install the pinned development tools that `ci` needs
    help    Show this message
";

fn main() -> anyhow::Result<()> {
    let sh = Shell::new()?;
    match env::args().nth(1).as_deref() {
        Some("ci") => ci(&sh),
        Some("setup") => setup(&sh),
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
    cmd!(sh, "cargo deny check")
        .run()
        .context("cargo-deny failed; if it is not installed, run `cargo xtask setup`")?;
    Ok(())
}

/// Installs each tool in `TOOLS`. Cargo skips a tool whose pinned version is
/// already installed.
fn setup(sh: &Shell) -> anyhow::Result<()> {
    for (name, version) in TOOLS {
        let spec = format!("{name}@{version}");
        cmd!(sh, "cargo install --locked {spec}").run()?;
    }
    Ok(())
}
