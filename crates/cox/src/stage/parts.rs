//! The parts a chat runs with, and how they reach each other, for the
//! human to see what is started before they read the code: the agent's
//! process and Cox's thread for it, speaking ACP; the `cox --mcp` the
//! agent starts and the chat's tool server it reaches over the socket,
//! which asks the stage; the Chrome `DevTools` MCP server the agent
//! starts, the relay it goes through and CEF behind it; `rust-analyzer`,
//! when a tab of Rust code needs it; and the judge of a goal in words.
//! Each part has what it is, who starts it, and, while the chat's agent
//! runs, how it was started: the agent's command, the socket, the ports;
//! and the code that starts it, when the project has the file, as it does
//! when the project is Cox. The stage's Parts page draws them, from what
//! [`told`] says of them as JSON at `cox://api/parts` (see
//! [`crate::stage::plugins`]).

use std::path::{Path, PathBuf};

use cox_acp::McpServer;

use crate::ui::paths::shortened;

/// What a chat runs with, as Cox started its agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    /// The agent's command: the program, then its arguments.
    pub command: Vec<String>,
    /// The socket of the chat's tool server, when it started.
    pub socket: Option<PathBuf>,
    /// The port of the chat's relay, when it started.
    pub relay_port: Option<u16>,
    /// The stage's `DevTools` port, when CEF opened one.
    pub devtools_port: Option<u16>,
    /// The MCP servers the agent was handed, to start.
    pub servers: Vec<McpServer>,
}

/// What kind of thing a part is, which the page colors it by.
#[derive(Clone, Copy)]
enum Kind {
    /// A process of its own.
    Process,
    /// A thread of Cox's.
    Thread,
    /// Part of the window, on its thread.
    Window,
}

impl Kind {
    fn said(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Thread => "thread",
            Self::Window => "window",
        }
    }
}

/// A part a chat runs with.
struct Part {
    kind: Kind,
    name: &'static str,
    /// What it is.
    says: &'static str,
    /// Who starts it, and when.
    started: &'static str,
    /// The code that starts it: the file, from the workspace, and words
    /// of the line to open it at.
    file: &'static str,
    find: &'static str,
}

/// Which way what goes between two parts goes.
#[derive(Clone, Copy)]
enum Way {
    Right,
    Left,
    Both,
}

/// What goes between two parts of a row, and which way.
struct Between {
    says: &'static str,
    way: Way,
}

/// A row of the picture: parts across it, with what goes between them,
/// and when the row is there at all, if not always.
struct Row {
    /// What the row is about.
    name: &'static str,
    when: Option<&'static str>,
    parts: Vec<Part>,
    between: Vec<Between>,
}

/// The rows of the picture, as they are for every chat.
fn rows() -> Vec<Row> {
    vec![
        agent_row(),
        tools_row(),
        pages_row(),
        rust_row(),
        judge_row(),
    ]
}

fn part(
    kind: Kind,
    name: &'static str,
    says: &'static str,
    started: &'static str,
    file: &'static str,
    find: &'static str,
) -> Part {
    Part {
        kind,
        name,
        says,
        started,
        file,
        find,
    }
}

fn between(says: &'static str, way: Way) -> Between {
    Between { says, way }
}

/// The agent, and how what it says reaches the conversation.
fn agent_row() -> Row {
    Row {
        name: "Talking with the agent",
        when: None,
        parts: vec![
            part(
                Kind::Process,
                "the agent",
                "The agent's process, which speaks ACP on its standard input and output.",
                "Started by Cox for the chat, in the chat's folder.",
                "crates/cox/src/chat/launcher.rs",
                "fn spawn(",
            ),
            part(
                Kind::Thread,
                "agent thread",
                "Cox's thread for the agent: speaks ACP with it, and hands what it says to the \
                 chat.",
                "Started by Cox with the agent, one for each chat.",
                "crates/cox/src/agent.rs",
                "pub fn spawn<",
            ),
            part(
                Kind::Window,
                "conversation",
                "The chat's conversation, on the window's thread, which shows what the agent says \
                 and asks.",
                "Part of the window.",
                "crates/cox/src/chat/conversation.rs",
                "pub fn apply(",
            ),
        ],
        between: vec![
            between("ACP on its stdio", Way::Both),
            between("what the agent says", Way::Right),
        ],
    }
}

/// Cox's own tools: from the bridge the agent starts to the stage.
fn tools_row() -> Row {
    Row {
        name: "Cox's own tools",
        when: None,
        parts: vec![
            part(
                Kind::Process,
                "cox --mcp",
                "Cox run again, as the bridge that carries what the agent says to the chat's \
                 tools.",
                "Started by the agent, as its MCP server `cox`.",
                "crates/cox/src/agent/tools.rs",
                "pub fn cox(",
            ),
            part(
                Kind::Thread,
                "tools thread",
                "The chat's server of Cox's own tools, which answers on the chat's socket.",
                "Started by Cox with the agent, on a thread of its own.",
                "crates/cox/src/agent/mcp.rs",
                "pub fn start(",
            ),
            part(
                Kind::Window,
                "the stage",
                "The chat's stage, on the window's thread, which carries out what the tools ask.",
                "Part of the window.",
                "crates/cox/src/stage/asked.rs",
                "fn hear_tools(",
            ),
        ],
        between: vec![
            between("the chat's socket", Way::Right),
            between("asks", Way::Right),
        ],
    }
}

/// The pages: from the browser tools the agent starts to CEF.
fn pages_row() -> Row {
    Row {
        name: "The stage's pages",
        when: None,
        parts: vec![
            part(
                Kind::Process,
                "chrome-devtools-mcp",
                "The Chrome DevTools MCP server, with which the agent reads and drives the pages.",
                "Started by the agent, as its MCP server `stage`, through npx.",
                "crates/cox/src/agent/tools.rs",
                "pub fn stage(",
            ),
            part(
                Kind::Thread,
                "relay",
                "Cox's relay for the chat: passes everything on, but keeps the agent to the \
                 chat's pages.",
                "Started by Cox with the agent, one for each chat.",
                "crates/cox-browser/src/relay.rs",
                "pub fn start(",
            ),
            part(
                Kind::Process,
                "CEF",
                "Chromium, which draws the stage's pages with no window of its own.",
                "Started by Cox once, for every chat.",
                "crates/cox-browser/src/lib.rs",
                "pub struct Cef",
            ),
        ],
        between: vec![
            between("the relay's port", Way::Right),
            between("the DevTools port", Way::Right),
        ],
    }
}

/// The language server of the project's Rust code.
fn rust_row() -> Row {
    Row {
        name: "Rust code",
        when: Some("when a tab shows Rust code"),
        parts: vec![
            part(
                Kind::Process,
                "rust-analyzer",
                "The language server of the project's Rust code, which says what a name means.",
                "Started by Cox when a tab of Rust code first needs it, one for the project.",
                "crates/cox/src/stage/servers.rs",
                "pub fn of(",
            ),
            part(
                Kind::Window,
                "code tabs",
                "The tabs of code, which ask the server what the code means.",
                "Part of the stage.",
                "crates/cox/src/stage/code.rs",
                "pub struct Code {",
            ),
        ],
        between: vec![between("LSP on its stdio", Way::Left)],
    }
}

/// The judge of a goal in words.
fn judge_row() -> Row {
    Row {
        name: "A goal in words",
        when: Some("when the chat has a goal in words"),
        parts: vec![
            part(
                Kind::Process,
                "the judge",
                "An agent of the chat's command, which judges whether the goal is met.",
                "Started by Cox when the goal is set, in the chat's folder, with none of Cox's \
                 tools.",
                "crates/cox/src/chat/judge.rs",
                "pub fn start(",
            ),
            part(
                Kind::Window,
                "the chat",
                "The chat asks it, each time a turn ends, whether the goal is met now.",
                "Part of the window.",
                "crates/cox/src/chat/judge.rs",
                "pub fn ask(",
            ),
        ],
        between: vec![between("asked at each turn's end", Way::Left)],
    }
}

/// How `part` was started, as `started` says, while the agent runs.
fn detail(started: Option<&Started>, part: &Part) -> Option<String> {
    let started = started?;
    let server = |name: &str| {
        started
            .servers
            .iter()
            .find(|server| server.name == name)
            .map(|server| {
                let mut words = vec![server.command.to_string_lossy().into_owned()];
                words.extend(server.args.iter().cloned());
                words.join(" ")
            })
    };
    match part.name {
        "the agent" => Some(started.command.join(" ")),
        "cox --mcp" => started
            .socket
            .as_ref()
            .map(|socket| format!("on {}", shortened(socket))),
        "chrome-devtools-mcp" => server("stage"),
        "relay" => started.relay_port.map(|port| format!("port {port}")),
        "CEF" => started
            .devtools_port
            .map(|port| format!("DevTools on port {port}")),
        _ => None,
    }
}

/// The picture as JSON, for a page of the stage to draw it (see
/// [`crate::stage::plugins`]): whether the agent runs, and the rows, each
/// named, with its parts and what goes between them. A part is of a
/// [`Kind`], and names the file and line of the code that starts it, as
/// absolute paths, when the project in `root` has the file.
#[must_use]
pub fn told(root: &Path, started: Option<&Started>) -> serde_json::Value {
    let rows: Vec<serde_json::Value> = rows()
        .iter()
        .map(|row| {
            let parts: Vec<serde_json::Value> = row
                .parts
                .iter()
                .map(|part| {
                    let path = root.join(part.file);
                    let has = path.is_file();
                    serde_json::json!({
                        "kind": part.kind.said(),
                        "name": part.name,
                        "says": part.says,
                        "started": part.started,
                        "detail": detail(started, part),
                        "file": has.then(|| path.to_string_lossy().into_owned()),
                        "line": has.then(|| line_saying(&path, part.find)).flatten(),
                    })
                })
                .collect();
            let between: Vec<serde_json::Value> = row
                .between
                .iter()
                .map(|between| {
                    serde_json::json!({
                        "says": between.says,
                        "way": match between.way {
                            Way::Right => "right",
                            Way::Left => "left",
                            Way::Both => "both",
                        },
                    })
                })
                .collect();
            serde_json::json!({
                "name": row.name,
                "when": row.when,
                "parts": parts,
                "between": between,
            })
        })
        .collect();
    serde_json::json!({ "running": started.is_some(), "rows": rows })
}

/// The line of the file at `path` that first says `find`, as people
/// number lines, if one does.
#[must_use]
pub fn line_saying(path: &Path, find: &str) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .position(|line| line.contains(find))
        .map(|index| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_names_code_that_exists() {
        // The tests run from the crate's folder, two below the workspace.
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for row in rows() {
            assert_eq!(
                row.between.len() + 1,
                row.parts.len(),
                "{}",
                row.parts[0].name
            );
            for part in row.parts {
                let path = workspace.join(part.file);
                assert!(
                    line_saying(&path, part.find).is_some(),
                    "{} says nothing of {:?}",
                    part.file,
                    part.find
                );
            }
        }
    }

    #[test]
    fn the_line_saying_words_is_found_as_people_number_lines() {
        let path = std::env::temp_dir().join(format!("cox-parts-{}.rs", std::process::id()));
        std::fs::write(&path, "fn a() {}\n\npub fn start() {}\n").unwrap();
        assert_eq!(line_saying(&path, "pub fn start("), Some(3));
        assert_eq!(line_saying(&path, "fn stop("), None);
        std::fs::remove_file(path).unwrap();
    }
}
