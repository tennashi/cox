//! The tools Cox hands the agent, as MCP servers the agent starts for the
//! chat.
//!
//! For the stage's pages, that is the Chrome `DevTools` MCP server,
//! connected to the stage's remote debugging port through the relay: the
//! agent reads and drives the page the human sees, and the stage shows
//! where. It runs on Node through `npx`, which fetches it the first time.
//!
//! The rest are Cox's own tools, which Cox itself serves (see
//! [`crate::agent::mcp`]).

use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use cox_acp::McpServer;

/// The Chrome `DevTools` MCP server, at the version Cox was tried with.
const DEVTOOLS_MCP: &str = "chrome-devtools-mcp@1.11.0";

/// The folder of temporary files every agent writes to, whatever the
/// system's own: Claude Code keeps its scratchpad there, under
/// `/tmp/claude-<uid>`, where macOS's `TMPDIR` is elsewhere.
#[cfg(unix)]
const TMP: &str = "/tmp";

/// The MCP server that gives the agent the stage, through the relay on
/// `relay_port`, for a chat working in `cwd`. It runs with the `npx`
/// found on `path`, a value of `PATH`, or is `None` when there is none.
#[must_use]
pub fn stage(relay_port: u16, cwd: &Path, path: &OsStr) -> Option<McpServer> {
    let Some(npx) = find("npx", path) else {
        tracing::warn!("no npx on PATH, so the agent gets no tools for the stage");
        return None;
    };
    Some(McpServer {
        name: "stage".to_owned(),
        command: npx,
        args: vec![
            "-y".to_owned(),
            DEVTOOLS_MCP.to_owned(),
            // The relay takes any path; this one is the browser's own.
            "--wsEndpoint".to_owned(),
            format!("ws://127.0.0.1:{relay_port}/devtools/browser"),
            // Both send data to Google unless turned off: usage statistics,
            // and the page's URL to look up field data for performance
            // traces. That is the human's call, not Cox's.
            "--no-usage-statistics".to_owned(),
            "--no-performance-crux".to_owned(),
        ]
        .into_iter()
        .chain(workspaces(cwd))
        .collect(),
    })
}

/// Where the server's tools that read or write files, as a screenshot's
/// `filePath`, may: the chat's folder, and the folder of temporary
/// files, beside the system's own, which the server allows anyway.
/// Unless told, it allows only the system's, which on macOS is not
/// `/tmp`, so a screenshot to the agent's scratchpad was refused there.
fn workspaces(cwd: &Path) -> Vec<String> {
    let mut folders = vec![cwd.to_string_lossy().into_owned()];
    #[cfg(unix)]
    folders.push(TMP.to_owned());
    folders
        .into_iter()
        .flat_map(|folder| ["--workspace".to_owned(), folder])
        .collect()
}

/// The MCP server of Cox's own tools for one chat: `exe`, the `cox` that
/// runs, as the bridge to the chat's server on `socket`.
#[must_use]
pub fn cox(exe: PathBuf, socket: &Path) -> McpServer {
    McpServer {
        name: "cox".to_owned(),
        command: exe,
        args: vec![
            crate::agent::mcp::OPTION.to_owned(),
            socket.to_string_lossy().into_owned(),
        ],
    }
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
    use coxd::testing::TempDir;

    #[test]
    fn the_stage_is_the_devtools_server_through_the_relay() {
        let bin = TempDir::new();
        fs::write(bin.join("npx"), "").unwrap();
        let path = std::env::join_paths([bin.join("missing"), bin.to_path_buf()]).unwrap();

        let server = stage(4321, Path::new("/work/app"), &path);

        let server = server.expect("npx is on the path");
        assert_eq!(server.name, "stage");
        assert_eq!(server.command, bin.join("npx"));
        assert_eq!(
            server.args[1..4],
            [
                DEVTOOLS_MCP,
                "--wsEndpoint",
                "ws://127.0.0.1:4321/devtools/browser"
            ]
        );
        // The chat's folder and /tmp, where the agent keeps its scratch.
        assert_eq!(
            server.args[6..],
            ["--workspace", "/work/app", "--workspace", "/tmp"]
        );
    }

    #[test]
    fn there_is_no_stage_without_npx() {
        let empty = TempDir::new();
        assert_eq!(stage(4321, Path::new("/work/app"), empty.as_os_str()), None);
    }
}
