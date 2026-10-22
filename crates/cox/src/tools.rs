//! The tools Cox hands the agent, as MCP servers the agent starts for the
//! session.
//!
//! For the stage, that is the Chrome `DevTools` MCP server, connected to the
//! stage's remote debugging port: the agent reads and drives the page the
//! human sees. It runs on Node through `npx`, which fetches it the first
//! time.

use std::ffi::OsStr;
use std::path::PathBuf;

use cox_acp::McpServer;

/// The Chrome `DevTools` MCP server, at the version Cox was tried with.
const DEVTOOLS_MCP: &str = "chrome-devtools-mcp@1.11.0";

/// The MCP server that gives the agent the stage, whose remote debugging
/// port is `port`. It runs with the `npx` found on `path`, a value of
/// `PATH`, or is `None` when there is none.
#[must_use]
pub fn stage(port: i32, path: &OsStr) -> Option<McpServer> {
    let Some(npx) = find("npx", path) else {
        log::warn!("no npx on PATH, so the agent gets no tools for the stage");
        return None;
    };
    Some(McpServer {
        name: "stage".to_owned(),
        command: npx,
        args: vec![
            "-y".to_owned(),
            DEVTOOLS_MCP.to_owned(),
            "--browserUrl".to_owned(),
            format!("http://127.0.0.1:{port}"),
            // Both send data to Google unless turned off: usage statistics,
            // and the page's URL to look up field data for performance
            // traces. That is the human's call, not Cox's.
            "--no-usage-statistics".to_owned(),
            "--no-performance-crux".to_owned(),
        ],
    })
}

/// The first `program` in the directories of `path`.
fn find(program: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn the_stage_is_the_devtools_server_on_its_port() {
        let bin = std::env::temp_dir().join(format!("cox-tools-{}", std::process::id()));
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("npx"), "").unwrap();
        let path = std::env::join_paths([bin.join("missing"), bin.clone()]).unwrap();

        let server = stage(9222, &path);
        fs::remove_dir_all(&bin).unwrap();

        let server = server.expect("npx is on the path");
        assert_eq!(server.name, "stage");
        assert_eq!(server.command, bin.join("npx"));
        assert_eq!(
            server.args[1..4],
            [DEVTOOLS_MCP, "--browserUrl", "http://127.0.0.1:9222"]
        );
    }

    #[test]
    fn there_is_no_stage_without_npx() {
        let missing = std::env::temp_dir().join("cox-tools-missing");
        assert_eq!(stage(9222, missing.as_os_str()), None);
    }
}
