//! `run --linux`: Cox, run headless in a Linux container, from any machine
//! with Docker.

use std::io::IsTerminal as _;
use std::path::Path;

use anyhow::Context as _;
use xshell::{Shell, cmd};

const IMAGE: &str = "cox-linux";
const DOCKERFILE: &str = include_str!("../linux.Dockerfile");
/// The label a container keeps the hash of the Dockerfile it was made from
/// in. An image's ID tells nothing: each build gives a new one.
const MADE_FROM: &str = "cox-linux.dockerfile";
/// Where the container sees the workspace.
const WORKSPACE: &str = "/src";

/// Runs `cargo xtask run --headless` with `args`, `run`'s own but for
/// `--linux` and `--headless`, in the workspace's container, made afresh
/// when the Dockerfile has changed, and restarted so that nothing of a run
/// before is left.
pub fn run(sh: &Shell, root: &Path, args: &[String]) -> anyhow::Result<()> {
    let container = name(root);
    let dockerfile = format!("{:016x}", fnv(DOCKERFILE.as_bytes()));
    let label = format!("{{{{index .Config.Labels \"{MADE_FROM}\"}}}}");
    let made_from = cmd!(sh, "docker container inspect --format {label} {container}")
        .quiet()
        .ignore_stderr()
        .read()
        .ok();
    if made_from.as_deref() == Some(dockerfile.as_str()) {
        cmd!(sh, "docker restart --time 1 {container}")
            .ignore_stdout()
            .run()
            .context("--linux runs Cox in a container; is Docker running?")?;
    } else {
        cmd!(sh, "docker build --quiet --tag {IMAGE} -")
            .stdin(DOCKERFILE)
            .ignore_stdout()
            .run()
            .context("--linux runs Cox in a container; is Docker installed and running?")?;
        if made_from.is_some() {
            cmd!(sh, "docker rm --force {container}")
                .ignore_stdout()
                .run()?;
        }
        let mount = format!("{}:{WORKSPACE}", root.display());
        let label = format!("{MADE_FROM}={dockerfile}");
        cmd!(
            sh,
            "docker run --detach --name {container} --label {label} --ulimit core=0
                --volume {mount}
                --volume cox-linux-target:/target
                --volume cox-linux-cargo:/usr/local/cargo/registry
                --volume cox-linux-rustup:/usr/local/rustup
                --env CARGO_TARGET_DIR=/target --workdir {WORKSPACE}
                {IMAGE} sleep infinity"
        )
        .ignore_stdout()
        .run()?;
    }
    println!(
        "Cox runs in the container {container}: run what it prints there, as `docker exec {container} <command>`"
    );
    let terminal = std::io::stdin().is_terminal().then_some("--tty");
    cmd!(
        sh,
        "docker exec --interactive {terminal...} {container} cargo xtask run --headless {args...}"
    )
    .run()?;
    Ok(())
}

/// The container of the workspace at `root`: each worktree has its own,
/// and they share the build.
fn name(root: &Path) -> String {
    format!(
        "{IMAGE}-{:08x}",
        fnv(root.as_os_str().as_encoded_bytes()) & 0xffff_ffff
    )
}

/// FNV-1a, which hashes the same in every Rust release.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::name;

    #[test]
    fn each_worktree_has_a_container_of_its_own() {
        let one = name(Path::new("/work/cox"));
        assert_eq!(one, name(Path::new("/work/cox")));
        assert_ne!(one, name(Path::new("/work/other/cox")));
        assert!(one.starts_with("cox-linux-"), "{one}");
    }
}
