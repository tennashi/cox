//! What starts a chat, in two halves: what the daemon starts, which draws
//! nothing, and what the window starts beside it.
//!
//! The daemon starts the chat's agent ([`Launcher`], the daemon's
//! [`Spawns`]), in the order things start:
//!
//! 1. The server of Cox's own tools, on a socket of the chat's own
//!    ([`Launcher::server`]), whose asks of the chat's stage the daemon
//!    passes on to the window.
//! 2. The agent, working in the chat's directory, told of the two MCP
//!    servers it starts to reach the stage's relay and the server
//!    ([`spawn`], [`Launcher::tools`]).
//!
//! The window starts the chat's stage ([`CefStages`], the window's
//! [`Stages`]): the stage itself, on the page every chat starts on, or on
//! the pages and files the chat had (see [`crate::stage`]); and, before
//! the daemon starts an agent that drives it, the relay the agent's
//! browser tools go through, which keeps the agent to the stage's pages
//! ([`Stages::relay`]), whose port the window tells the daemon.
//!
//! The relay and the server are each a thread of its own, with a tokio
//! runtime, and so is the agent's connection; the stage is a GPUI entity
//! on the window's thread. The agent is a process, which starts the two
//! MCP servers as processes of its own: `cox --mcp` (see
//! [`crate::agent::mcp`]) and the Chrome `DevTools` MCP server (see
//! [`crate::agent::tools`]).

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use cox_acp::McpServer;
use cox_core::tabs::KeptTabs;
use gpui::{App, Window, prelude::*};

use crate::agent::mcp::ToolServer;
use crate::agent::{Agent, Start};
use crate::agents::{Agents, Known, download};
use crate::chat::Stages;
use crate::chat::judge::Starts;
use crate::chat::view::{OpenLink, StageView};
use crate::daemon::{Spawned, Spawns};
use crate::stage::Stage;
use crate::ui::Panel;
use cox_browser::relay::Relay;

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
    /// Cox's tools the agent gets.
    fn chat(&self, cwd: PathBuf, start: Start, agent: &str, relay: Option<u16>) -> Spawned {
        // 1. The server of Cox's own tools, with the `cox` that reaches it.
        let only = start.setup().and_then(|setup| setup.tools.clone());
        let server = self.server(&cwd, only);
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
    /// serving the tools named in `only`, or all of them, and answers it
    /// with the `cox` the agent runs to reach it, or `None` when it did not
    /// start.
    fn server(
        &self,
        cwd: &std::path::Path,
        only: Option<Vec<String>>,
    ) -> Option<(PathBuf, ToolServer)> {
        let exe = self.exe.clone()?;
        match crate::agent::mcp::start(cwd.to_owned(), only) {
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
        spawn(self.agents.of(Some(agent)), cwd, tools, start)
    }
}

/// Why no agent starts when Cox was opened without an agent command, as
/// from the Finder, and the settings name none.
const NO_AGENT: &str = "Cox was opened without an agent command, and settings.json, \
    where Cox keeps what lasts, names none of the agents agents.json lists beside it, \
    as {\"agent\": \"Claude\"} for [{\"name\": \"Claude\", \"command\": [\"npx\", \"-y\", \
    \"@agentclientprotocol/claude-agent-acp\"]}]";

/// Starts `known`, working in `cwd`, an absolute path, with the tools of
/// `tools`, as `start` says: a chat's agent, or one with no chat, such as
/// the judge of a goal, or the agent that translates the wiki (see
/// [`crate::wiki::translate`]).
pub(crate) fn spawn(known: Known, cwd: PathBuf, tools: Vec<McpServer>, start: Start) -> Agent {
    tracing::info!(agent = %known.id, "starts the agent");
    let Known {
        id,
        name,
        command,
        download,
        ..
    } = known;
    let connect = move |told: &dyn Fn(String)| {
        anyhow::ensure!(!command.is_empty(), NO_AGENT);
        // The program of an agent of the registry that Cox downloads
        // is downloaded the first time it starts.
        let command = match download {
            Some(download) => {
                let installed = download::installed(&id, &download, &|note| {
                    told(format!("{name}: {note}"));
                })?;
                crate::agents::located(&command, &installed)
            }
            None => command,
        };
        cox_acp::launch(command)
    };
    crate::agent::spawn(connect, cwd, tools, start)
}

/// Makes the chats' stages in CEF: the window's [`Stages`] when Cox runs.
#[derive(Clone)]
pub struct CefStages {
    /// The page each chat's stage shows first.
    pub stage: String,
    pub cef: cox_browser::Cef,
}

impl Stages for CefStages {
    fn stage(
        &self,
        cwd: PathBuf,
        tabs: Option<KeptTabs<u64>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<(StageView, OpenLink)> {
        let panel = cx.new(|_| Panel::default());
        let stage = cx.new(|cx| {
            Stage::reading(
                self.cef.clone(),
                cwd,
                tabs,
                &self.stage,
                panel.clone(),
                window,
                cx,
            )
        });
        let opens = stage.clone();
        let open_link: OpenLink = Rc::new(move |url, window, cx| {
            opens.update(cx, |stage, cx| stage.open(url, window, cx));
        });
        let view = StageView {
            view: stage.into(),
            panel,
        };
        Some((view, open_link))
    }

    /// Starts the relay that the agent's browser tools go through, kept to
    /// the chat's pages, which tells the stage where the agent points,
    /// clicks and types, and opens and closes its tabs for the agent.
    /// Answers `None` when it did not start.
    fn relay(&self) -> Option<Relay> {
        match self
            .cef
            .devtools_port()
            .ok_or_else(|| anyhow::anyhow!("the browser has no DevTools port"))
            .and_then(cox_browser::relay::start)
        {
            Ok(relay) => Some(relay),
            Err(error) => {
                tracing::error!(
                    "no relay to the stage, so the agent gets no tools for it: {error:#}"
                );
                None
            }
        }
    }
}
