//! What opening a chat starts: the agent's session, working in the chat's
//! directory, and on Linux and macOS the chat's own stage, whose pages the
//! agent reaches through a relay of its own, and the server of Cox's own
//! tools for the agent, which reach the stage too. A closed chat, opened to
//! read, starts none of them but a stage on the pages it had, until the
//! human sends to it: then its agent starts again, going on with its
//! session if it can, and the stage works with it.

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use cox_acp::McpServer;
use cox_core::store::Record;
use gpui::{App, Entity, Window, prelude::*};

use crate::agent::{Agent, Start};
use crate::instructions::Instructions;
use crate::view::{Again, OpenLink, SessionView};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::view::{Panel, StageView};
use crate::workspace::Closed;

/// Opens chats, each with an agent of the same command.
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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub cef: crate::browser::Cef,
}

impl Launcher {
    /// Starts a chat working in `cwd`, an absolute path, whose agent gets
    /// `instructions`, kept in `record`, and returns its view.
    pub fn open(
        &self,
        cwd: PathBuf,
        instructions: Option<Instructions>,
        record: Option<Record>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<SessionView> {
        let start = Start::New(instructions);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let (stage, agent, open_link) = {
            let panel = cx.new(|_| Panel::default());
            let stage = cx.new(|cx| {
                crate::stage::Stage::new(
                    self.cef.clone(),
                    cwd.clone(),
                    &self.stage,
                    panel.clone(),
                    window,
                    cx,
                )
            });
            let agent = self.start(&stage, cwd, record.clone(), start, window, cx);
            let (stage, open_link) = shown(stage, panel);
            (Some(stage), agent, open_link)
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let (stage, agent, open_link) = {
            let _ = window;
            // With no stage, links open in the human's browser.
            let open_link: OpenLink = Rc::new(|url, _, cx| cx.open_url(url));
            (None, self.spawn(cwd, Vec::new(), start), open_link)
        };
        cx.new(|cx| SessionView::new(agent, stage, open_link, record, cx))
    }

    /// The view of `closed`, a chat that has closed, with a stage on the
    /// pages and files it had. When the human sends to it, its agent
    /// starts again, going on with its session if it can.
    pub fn read(&self, closed: Closed, window: &mut Window, cx: &mut App) -> Entity<SessionView> {
        let Closed {
            cwd,
            entries,
            tabs,
            session,
            record,
        } = closed;
        let launcher = self.clone();
        let kept = record.clone();
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let (stage, again, open_link) = {
            let panel = cx.new(|_| Panel::default());
            let stage = cx.new(|cx| {
                crate::stage::Stage::reading(
                    self.cef.clone(),
                    cwd.clone(),
                    tabs,
                    &self.stage,
                    panel.clone(),
                    window,
                    cx,
                )
            });
            let works = stage.clone();
            let again: Again = Box::new(move |entries, window, cx| {
                let start = Start::again(&cwd, session, entries);
                launcher.start(&works, cwd, kept, start, window, cx)
            });
            let (stage, open_link) = shown(stage, panel);
            (Some(stage), again, open_link)
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let (stage, again, open_link) = {
            let _ = (tabs, kept, window);
            let again: Again = Box::new(move |entries, _, _| {
                let start = Start::again(&cwd, session, entries);
                launcher.spawn(cwd, Vec::new(), start)
            });
            let open_link: OpenLink = Rc::new(|url, _, cx| cx.open_url(url));
            (None, again, open_link)
        };
        cx.new(|cx| SessionView::closed(entries, stage, open_link, record, Some(again), window, cx))
    }

    /// Starts the agent of a chat working in `cwd`, as `start` says, with
    /// tools for `stage`, which from then on works with the agent, keeping
    /// its tabs in `record`.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn start(
        &self,
        stage: &Entity<crate::stage::Stage>,
        cwd: PathBuf,
        record: Option<Record>,
        start: Start,
        window: &mut Window,
        cx: &mut App,
    ) -> Agent {
        let relay = self.relay();
        let server = self.server(&cwd);
        // The agent gets the pages the human sees, and Cox's own tools.
        let tools = relay
            .as_ref()
            .and_then(|relay| crate::tools::stage(relay.port, &self.path))
            .into_iter()
            .chain(
                server
                    .as_ref()
                    .map(|(exe, server)| crate::tools::cox(exe.clone(), &server.socket)),
            )
            .collect();
        let requests = server.map(|(_, server)| server.requests);
        stage.update(cx, |stage, cx| {
            stage.work(relay, requests, record, window, cx);
        });
        self.spawn(cwd, tools, start)
    }

    /// Starts the agent working in `cwd` with `tools`, as `start` says.
    fn spawn(&self, cwd: PathBuf, tools: Vec<McpServer>, start: Start) -> Agent {
        let command = self.command.clone();
        crate::agent::spawn(move || cox_acp::launch(command), cwd, tools, start)
    }

    /// Starts the server of Cox's own tools for a chat working in `cwd`,
    /// and answers it with the `cox` the agent runs to reach it, or `None`
    /// when it did not start.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn server(&self, cwd: &std::path::Path) -> Option<(PathBuf, crate::mcp::Server)> {
        let exe = self.exe.clone()?;
        match crate::mcp::start(cwd.to_owned()) {
            Ok(server) => Some((exe, server)),
            Err(error) => {
                log::error!("the agent gets no tools of Cox's own: {error:#}");
                None
            }
        }
    }

    /// Starts the relay that the agent's browser tools go through, kept to
    /// the chat's pages, which tells the stage where the agent points,
    /// clicks and types, and opens and closes its tabs for the agent.
    /// Answers `None` when it did not start.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn relay(&self) -> Option<crate::relay::Relay> {
        match self
            .cef
            .devtools_port()
            .ok_or_else(|| anyhow::anyhow!("the browser has no DevTools port"))
            .and_then(crate::relay::start)
        {
            Ok(relay) => Some(relay),
            Err(error) => {
                log::error!("no relay to the stage, so the agent gets no tools for it: {error:#}");
                None
            }
        }
    }
}

/// `stage`, as its chat's conversation shows it, open when `panel` is, and
/// what opens the links of the conversation: the stage.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn shown(stage: Entity<crate::stage::Stage>, panel: Entity<Panel>) -> (StageView, OpenLink) {
    let opens = stage.clone();
    let open_link: OpenLink = Rc::new(move |url, window, cx| {
        opens.update(cx, |stage, cx| stage.open(url, window, cx));
    });
    let stage = StageView {
        view: stage.into(),
        panel,
    };
    (stage, open_link)
}
