//! What opening a chat starts: the agent's session, working in the chat's
//! directory, and on Linux and macOS the chat's own stage, whose pages the
//! agent reaches through a relay of its own, and the server of Cox's own
//! tools for the agent, which reach the stage too. A closed chat, opened to
//! read, starts none of them but a stage on the pages it had.

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use cox_core::store::Record;
use cox_core::tabs::Tabs;
use gpui::{App, Entity, Window, prelude::*};

use crate::conversation::Entry;
use crate::instructions::Instructions;
use crate::view::{OpenLink, SessionView};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::view::{Panel, StageView};

/// Opens chats, each with an agent of the same command.
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
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let (stage, tools, open_link) = {
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
            let panel = cx.new(|_| Panel::default());
            let stage = cx.new(|cx| {
                crate::stage::Stage::new(
                    self.cef.clone(),
                    cwd.clone(),
                    &self.stage,
                    relay,
                    server.map(|(_, server)| server.requests),
                    panel.clone(),
                    record.clone(),
                    window,
                    cx,
                )
            });
            let (stage, open_link) = shown(stage, panel);
            (Some(stage), tools, open_link)
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let (stage, tools, open_link) = {
            let _ = (window, &self.stage, &self.path, &self.exe);
            // With no stage, links open in the human's browser.
            let open_link: OpenLink = Rc::new(|url, _, cx| cx.open_url(url));
            (None, Vec::new(), open_link)
        };
        let command = self.command.clone();
        let agent = crate::agent::spawn(move || cox_acp::launch(command), cwd, tools, instructions);
        cx.new(|cx| SessionView::new(agent, stage, open_link, record, cx))
    }

    /// The view of a closed chat that worked in `cwd`, which showed
    /// `entries`, with a stage on the pages and files of `tabs`.
    pub fn read(
        &self,
        cwd: PathBuf,
        entries: Vec<Entry>,
        tabs: Option<Tabs>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<SessionView> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let (stage, open_link) = {
            let panel = cx.new(|_| Panel::default());
            let stage = cx.new(|cx| {
                crate::stage::Stage::reading(
                    self.cef.clone(),
                    cwd,
                    tabs,
                    &self.stage,
                    panel.clone(),
                    window,
                    cx,
                )
            });
            let (stage, open_link) = shown(stage, panel);
            (Some(stage), open_link)
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let (stage, open_link) = {
            let _ = (cwd, tabs, window);
            let open_link: OpenLink = Rc::new(|url, _, cx| cx.open_url(url));
            (None, open_link)
        };
        cx.new(|cx| SessionView::closed(entries, stage, open_link, cx))
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
