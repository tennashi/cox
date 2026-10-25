//! What opening a chat starts: the agent's session, working in the chat's
//! directory, and on Linux and macOS the chat's own stage, whose pages the
//! agent reaches through a relay of its own.

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Entity, Window, prelude::*};

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
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub cef: crate::browser::Cef,
}

impl Launcher {
    /// Starts a chat working in `cwd`, an absolute path, whose agent gets
    /// `instructions`, and returns its view.
    pub fn open(
        &self,
        cwd: PathBuf,
        instructions: Option<Instructions>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<SessionView> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let (stage, tools, open_link) = {
            let relay = self.relay();
            // The agent gets the pages the human sees.
            let tools = relay
                .as_ref()
                .and_then(|relay| crate::tools::stage(relay.port, &self.path))
                .into_iter()
                .collect();
            let panel = cx.new(|_| Panel::default());
            let stage = cx.new(|cx| {
                crate::stage::Stage::new(
                    self.cef.clone(),
                    &self.stage,
                    relay,
                    panel.clone(),
                    window,
                    cx,
                )
            });
            // Links in the conversation open on the chat's stage.
            let shown = stage.clone();
            let open_link: OpenLink = Rc::new(move |url, window, cx| {
                shown.update(cx, |stage, cx| stage.open(url, window, cx));
            });
            let stage = StageView {
                view: stage.into(),
                panel,
            };
            (Some(stage), tools, open_link)
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let (stage, tools, open_link) = {
            let _ = (window, &self.stage, &self.path);
            // With no stage, links open in the human's browser.
            let open_link: OpenLink = Rc::new(|url, _, cx| cx.open_url(url));
            (None, Vec::new(), open_link)
        };
        let command = self.command.clone();
        let agent = crate::agent::spawn(move || cox_acp::launch(command), cwd, tools, instructions);
        cx.new(|cx| SessionView::new(agent, stage, open_link, cx))
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
