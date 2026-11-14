//! What the window starts of a chat, beside the agent the daemon starts
//! (see [`crate::daemon`]'s launcher).
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
//! [`coxd::agent::mcp`]) and the Chrome `DevTools` MCP server (see
//! [`coxd::agent::tools`]).

use std::path::PathBuf;
use std::rc::Rc;

use cox_core::tabs::KeptTabs;
use gpui::{App, Window, prelude::*};

use crate::chat::Stages;
use crate::chat::view::{OpenLink, StageView};
use crate::stage::Stage;
use crate::ui::Panel;
use cox_browser::relay::Relay;

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
        let keyed = stage.clone();
        let read = stage.clone();
        let view = StageView {
            view: stage.into(),
            panel,
            by_keys: Rc::new(move |tabs, window, cx| {
                keyed.update(cx, |stage, cx| stage.by_keys(tabs, window, cx));
            }),
            shows_files: Rc::new(move |cx| read.read(cx).shows_files(cx)),
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
