//! What opening a session starts: the agent, working in the session's
//! directory, and on Linux and macOS the session's own page on the stage,
//! which the agent reaches through a relay of its own.

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Entity, Window, prelude::*};

use crate::view::{OpenLink, SessionView};

/// Opens sessions, each with an agent of the same command.
pub struct Launcher {
    /// The agent to start, and its arguments.
    pub command: Vec<String>,
    /// The page each session's stage shows first.
    pub stage: String,
    /// The `PATH` the agent's tools are found on.
    pub path: OsString,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub cef: crate::browser::Cef,
}

impl Launcher {
    /// Starts a session working in `cwd`, an absolute path, and returns its
    /// view.
    pub fn open(&self, cwd: PathBuf, window: &mut Window, cx: &mut App) -> Entity<SessionView> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let (stage, tools, open_link) = {
            let (browser, events) = self.cef.open(&self.stage);
            let (relay_port, agent_input) = self.relay(&browser);
            // The agent gets the page the human sees.
            let tools = relay_port
                .and_then(|port| crate::tools::stage(port, &self.path))
                .into_iter()
                .collect();
            let stage =
                cx.new(|cx| crate::stage::Stage::new(browser, events, agent_input, window, cx));
            // Links in the conversation open on the session's stage.
            let shown = stage.clone();
            let open_link: OpenLink = Rc::new(move |url, window, cx| {
                shown.update(cx, |stage, cx| stage.open(url, window, cx));
            });
            (Some(stage), tools, open_link)
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let (stage, tools, open_link) = {
            let _ = (window, &self.stage, &self.path);
            // With no stage, links open in the human's browser.
            let open_link: OpenLink = Rc::new(|url, _, cx| cx.open_url(url));
            (None::<gpui::AnyView>, Vec::new(), open_link)
        };
        let command = self.command.clone();
        let agent = crate::agent::spawn(move || cox_acp::launch(command), cwd, tools);
        cx.new(|cx| SessionView::new(agent, stage, open_link, cx))
    }

    /// Starts the relay that the agent's browser tools go through, kept to
    /// `browser`'s page, which tells the stage where the agent points,
    /// clicks and types. Answers its port, or `None` when it did not start,
    /// and the input the agent sends.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn relay(
        &self,
        browser: &crate::browser::Browser,
    ) -> (
        Option<u16>,
        tokio::sync::mpsc::UnboundedReceiver<crate::relay::AgentInput>,
    ) {
        match self
            .cef
            .devtools_port()
            .ok_or_else(|| anyhow::anyhow!("the browser has no DevTools port"))
            .and_then(|port| crate::relay::start(port, browser.target_id()))
        {
            Ok((port, input)) => (Some(port), input),
            Err(error) => {
                log::error!("no relay to the stage, so the agent gets no tools for it: {error:#}");
                (None, tokio::sync::mpsc::unbounded_channel().1)
            }
        }
    }
}
