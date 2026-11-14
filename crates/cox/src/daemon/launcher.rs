//! What starts a chat's agent, for coxd (see [`Launcher`]), in the order
//! things start:
//!
//! 1. The server of Cox's own tools, on a socket of the chat's own
//!    ([`Launcher::server`]), whose asks of the chat's stage the daemon
//!    passes on to the window.
//! 2. The agent, working in the chat's directory, told of the two MCP
//!    servers it starts to reach the stage's relay and the server
//!    ([`crate::agent::spawn_known`], [`Launcher::tools`]).
//!
//! The window starts the chat's stage, and the relay to it (see
//! [`crate::chat::launcher`]).

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use cox_acp::McpServer;

use super::{Spawned, Spawns};
use crate::agent::mcp::ToolServer;
use crate::agent::{Agent, Start};
use crate::agents::Agents;
use crate::chat::judge::Starts;

/// Starts the agents of the chats, each with the agent it talks to: the
/// daemon's [`Spawns`] when Cox runs.
#[derive(Clone)]
pub struct Launcher {
    /// The agents a chat can talk to, the first being the one it starts
    /// with.
    pub agents: Agents,
    /// The `PATH` the agent's tools are found on.
    pub path: OsString,
    /// The `cox` that runs, which the agent runs to reach Cox's own tools,
    /// if it is known.
    pub exe: Option<PathBuf>,
}

impl Spawns for Launcher {
    fn agents(&self) -> Agents {
        self.agents.clone()
    }

    /// Starts the agent kept as `agent` working in `cwd`, as `start`
    /// says, with Cox's own tools, and the stage's through the relay on
    /// `relay`, when given. The chat's setup, if it has one, says which of
    /// Cox's tools the agent gets, and a chat in no project, as
    /// `in_project` says, gets none of a project's.
    fn chat(
        &self,
        cwd: PathBuf,
        start: Start,
        agent: &str,
        relay: Option<u16>,
        in_project: bool,
    ) -> Spawned {
        // 1. The server of Cox's own tools, with the `cox` that reaches it.
        let only = start.setup().and_then(|setup| setup.tools.as_deref());
        let server = self.server(&cwd, only, in_project);
        // What the agent gets: the MCP servers it starts to reach both.
        let tools = self.tools(&cwd, relay, server.as_ref());
        let (stage, asks, chat_requests) = match server {
            Some((_, server)) => (
                Some(server.requests),
                Some(server.asks),
                Some(server.chat_requests),
            ),
            None => (None, None, None),
        };
        // 2. The agent, on a thread of its own, which the chat asks of the
        // stage through what its tools ask, and hears ask of the daemon.
        let mut agent = self.spawn(cwd, tools, start, agent);
        agent.tools = asks;
        agent.chat_requests = chat_requests;
        Spawned { agent, stage }
    }

    /// What starts an agent with no chat, such as the judge of a goal: the
    /// agent kept as `agent`, with no tools of Cox's own and none for the
    /// stage.
    fn starts(&self, agent: &str) -> Starts {
        let launcher = self.clone();
        let agent = agent.to_owned();
        Rc::new(move |cwd, start| {
            let span = tracing::info_span!("judge", cwd = %cwd.display());
            let _entered = span.enter();
            launcher.spawn(cwd, Vec::new(), start, &agent)
        })
    }
}

impl Launcher {
    /// Starts the server of Cox's own tools for a chat working in `cwd`,
    /// serving the tools named in `only`, or all of them, but those of a
    /// project when the chat is in none, as `in_project` says, and answers
    /// it with the `cox` the agent runs to reach it, or `None` when it did
    /// not start.
    fn server(
        &self,
        cwd: &std::path::Path,
        only: Option<&[String]>,
        in_project: bool,
    ) -> Option<(PathBuf, ToolServer)> {
        let exe = self.exe.clone()?;
        match crate::agent::mcp::start(cwd.to_owned(), only, in_project) {
            Ok(server) => Some((exe, server)),
            Err(error) => {
                tracing::error!("the agent gets no tools of Cox's own: {error:#}");
                None
            }
        }
    }

    /// The MCP servers the agent of a chat working in `cwd` starts: one
    /// for the pages the human sees, through the relay on `relay`, and one
    /// for Cox's own tools, through `server`, each when it started.
    fn tools(
        &self,
        cwd: &std::path::Path,
        relay: Option<u16>,
        server: Option<&(PathBuf, ToolServer)>,
    ) -> Vec<McpServer> {
        relay
            .and_then(|port| crate::agent::tools::stage(port, cwd, &self.path))
            .into_iter()
            .chain(
                server.map(|(exe, server)| crate::agent::tools::cox(exe.clone(), &server.socket)),
            )
            .collect()
    }

    /// Starts the agent kept as `agent` working in `cwd` with `tools`, as
    /// `start` says.
    fn spawn(&self, cwd: PathBuf, tools: Vec<McpServer>, start: Start, agent: &str) -> Agent {
        crate::agent::spawn_known(self.agents.of(Some(agent)), cwd, tools, start)
    }
}
