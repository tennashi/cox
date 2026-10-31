//! What opening a chat starts, in the order things start:
//!
//! 1. The chat's own stage, on the page every chat starts on (see
//!    [`crate::stage`]).
//! 2. The relay the agent's browser tools go through, which keeps the
//!    agent to the stage's pages ([`Launcher::relay`]).
//! 3. The server of Cox's own tools, on a socket of the chat's own
//!    ([`Launcher::server`]).
//! 4. The stage works with them: it tells the relay of its pages, carries
//!    out what the tools ask, and keeps its tabs as they change
//!    ([`Stage::work`]).
//! 5. The agent, working in the chat's directory, told of the two MCP
//!    servers it starts to reach the relay and the server
//!    ([`Launcher::spawn`], [`Launcher::tools`]).
//! 6. The view of the conversation, talking to the agent, with the stage
//!    beside it ([`ChatView`]).
//!
//! Steps 2, 3 and 5 are each a thread of its own, with a tokio runtime;
//! the stage and the view are GPUI entities on its thread, which talk to
//! the threads over channels. The agent is a process, started by step 5,
//! which starts the two MCP servers as processes of its own: `cox --mcp`
//! (see [`crate::agent::mcp`]) and the Chrome `DevTools` MCP server (see
//! [`crate::agent::tools`]).
//!
//! A closed chat, opened to read, starts only its stage (1), on the pages
//! and files it had, and its view (6). The rest starts when the human
//! sends to it: then its agent starts again, going on with its session
//! if it can, and the stage works with it.

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use cox_acp::McpServer;
use cox_core::store::Record;
use gpui::{App, Entity, Window, prelude::*};

use crate::agent::instructions::Instructions;
use crate::agent::mcp::ToolServer;
use crate::agent::setup::{Setup, StageUse};
use crate::agent::{Agent, Start};
use crate::chat::conversation::Conversation;
use crate::chat::view::{Again, ChatView, OpenLink, StageView};
use crate::chat::{Closed, Opens};
use crate::stage::Stage;
use crate::ui::Panel;
use cox_browser::relay::Relay;

/// Opens chats, each with an agent of the same command: the [`Opens`] of
/// the window when Cox runs.
#[derive(Clone)]
pub struct Launcher {
    /// The agent to start, and its arguments.
    pub command: Vec<String>,
    /// The page each chat's stage shows first.
    pub stage: String,
    /// The `PATH` the agent's tools are found on.
    pub path: OsString,
    /// The `cox` that runs, which the agent runs to reach Cox's own tools,
    /// if it is known.
    pub exe: Option<PathBuf>,
    pub cef: cox_browser::Cef,
}

impl Opens for Launcher {
    /// Starts a chat working in `cwd`, an absolute path, whose agent gets
    /// `instructions`, set up as `setup` says if given, kept in `record`,
    /// and returns its view.
    fn open(
        &self,
        cwd: PathBuf,
        instructions: Option<Instructions>,
        setup: Option<Setup>,
        record: Option<Record>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<ChatView> {
        // 1. The stage, on the page every chat starts on.
        let panel = cx.new(|_| Panel::default());
        let stage = cx.new(|cx| {
            Stage::new(
                self.cef.clone(),
                cwd.clone(),
                &self.stage,
                panel.clone(),
                window,
                cx,
            )
        });
        // 2 to 5. What reaches the stage, and the agent.
        let agent = self.start(
            &stage,
            cwd,
            record.clone(),
            Start::New {
                instructions,
                setup,
            },
            window,
            cx,
        );
        // 6. The view, talking to the agent, with the stage beside it.
        cx.new(|cx| {
            ChatView::new(
                agent,
                Some(shown(&stage, panel)),
                open_link(&stage),
                record,
                cx,
            )
        })
    }

    /// The view of `closed`, a chat that has closed, with a stage on the
    /// pages and files it had. When the human sends to it, its agent
    /// starts again, going on with its session if it can.
    fn read(&self, closed: Closed, window: &mut Window, cx: &mut App) -> Entity<ChatView> {
        let Closed {
            cwd,
            entries,
            tabs,
            session,
            goal,
            setup,
            record,
        } = closed;
        // 1. The stage, on the pages and files the chat had.
        let panel = cx.new(|_| Panel::default());
        let stage = cx.new(|cx| {
            Stage::reading(
                self.cef.clone(),
                cwd.clone(),
                tabs,
                &self.stage,
                panel.clone(),
                window,
                cx,
            )
        });
        // 2 to 5, once the human sends to the chat.
        let launcher = self.clone();
        let works = stage.clone();
        let kept = record.clone();
        let again: Again = Box::new(move |entries, window, cx| {
            let start = Start::again(&cwd, session, entries, setup);
            launcher.start(&works, cwd, kept, start, window, cx)
        });
        // 6. The view, with the stage beside it, and no agent yet.
        cx.new(|cx| {
            ChatView::closed(
                Conversation::closed(entries).with_goal(goal),
                Some(shown(&stage, panel)),
                open_link(&stage),
                record,
                Some(again),
                window,
                cx,
            )
        })
    }
}

impl Launcher {
    /// Steps 2 to 5: starts the agent of a chat working in `cwd`, as
    /// `start` says, with what it reaches `stage` through, which from then
    /// on works with the agent, keeping its tabs in `record`. The chat's
    /// setup, if it has one, says which tools the agent gets.
    fn start(
        &self,
        stage: &Entity<Stage>,
        cwd: PathBuf,
        record: Option<Record>,
        start: Start,
        window: &mut Window,
        cx: &mut App,
    ) -> Agent {
        let setup = start.setup();
        // 2. The relay the agent's browser tools go through, unless the
        // setup keeps the agent from driving the stage.
        let drives = setup.is_none_or(|setup| setup.stage == StageUse::Drive);
        let relay = drives.then(|| self.relay()).flatten();
        // 3. The server of Cox's own tools, with the `cox` that reaches it.
        let server = self.server(&cwd, setup.and_then(|setup| setup.tools.clone()));
        // What the agent gets: the MCP servers it starts to reach both.
        let tools = self.tools(relay.as_ref(), server.as_ref());
        // 4. The stage works with the relay and the tools, and keeps its tabs.
        let (requests, asks, chat_requests) = match server {
            Some((_, server)) => (
                Some(server.requests),
                Some(server.asks),
                Some(server.chat_requests),
            ),
            None => (None, None, None),
        };
        stage.update(cx, |stage, cx| {
            stage.work(relay, requests, record, window, cx);
        });
        // 5. The agent, on a thread of its own, which the chat asks of the
        // stage through what its tools ask, and hears ask of the window.
        let mut agent = self.spawn(cwd, tools, start);
        agent.tools = asks;
        agent.chat_requests = chat_requests;
        agent
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
                log::error!("no relay to the stage, so the agent gets no tools for it: {error:#}");
                None
            }
        }
    }

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
                log::error!("the agent gets no tools of Cox's own: {error:#}");
                None
            }
        }
    }

    /// The MCP servers the agent starts: one for the pages the human sees,
    /// through `relay`, and one for Cox's own tools, through `server`, each
    /// when it started.
    fn tools(
        &self,
        relay: Option<&Relay>,
        server: Option<&(PathBuf, ToolServer)>,
    ) -> Vec<McpServer> {
        relay
            .and_then(|relay| crate::agent::tools::stage(relay.port, &self.path))
            .into_iter()
            .chain(
                server.map(|(exe, server)| crate::agent::tools::cox(exe.clone(), &server.socket)),
            )
            .collect()
    }

    /// Starts the agent working in `cwd` with `tools`, as `start` says.
    fn spawn(&self, cwd: PathBuf, tools: Vec<McpServer>, start: Start) -> Agent {
        let command = self.command.clone();
        crate::agent::spawn(move || cox_acp::launch(command), cwd, tools, start)
    }
}

/// `stage`, as its chat's conversation shows it: open when `panel` is.
fn shown(stage: &Entity<Stage>, panel: Entity<Panel>) -> StageView {
    StageView {
        view: stage.clone().into(),
        panel,
    }
}

/// What opens the links of the conversation: `stage`.
fn open_link(stage: &Entity<Stage>) -> OpenLink {
    let stage = stage.clone();
    Rc::new(move |url, window, cx| {
        stage.update(cx, |stage, cx| stage.open(url, window, cx));
    })
}
