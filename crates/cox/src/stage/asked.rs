//! What the agents ask of the stage, each of its chat's tabs. Through its
//! chat's relay, an agent works on its pages, which the stage shows, and
//! opens and closes them. Through Cox's own tools, it shows the human
//! files, as Markdown or as they are written, what changed in them, and
//! the map of the project, runs commands in terminals, and looks at and
//! uses the windows of the Mac's other apps (see [`mac`]).

mod mac;

use tokio::sync::mpsc::UnboundedReceiver;

use cox_browser::relay::FromAgent;
use gpui::{Context, Task, Window, prelude::*};

use crate::agent::mcp::{ASKED, Ran, ToolRequest};
use crate::stage::Stage;
use crate::stage::code::Code;
use crate::stage::document::Document;
use crate::stage::tab::{Tab, TabContent};
use crate::stage::terminal::{Program, Running, Terminal};

impl Stage {
    /// Carries out what the relay of the chat `chat` tells the stage for
    /// its agent.
    pub(super) fn hear(
        chat: u64,
        mut from_agent: UnboundedReceiver<FromAgent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(heard) = from_agent.recv().await {
                let done = this.update_in(cx, |stage, window, cx| {
                    stage.acting_for(Some(chat), |stage| stage.heard(heard, window, cx));
                });
                if done.is_err() {
                    break;
                }
            }
        })
    }

    /// Carries out `heard`, which the relay of the chat acting told the
    /// stage.
    fn heard(&mut self, heard: FromAgent, window: &mut Window, cx: &mut Context<Self>) {
        let stage = self;
        match heard {
            FromAgent::Worked { page, input } => stage.worked(page, input, cx),
            FromAgent::Open {
                url,
                background,
                answer,
            } => {
                let page = stage.open_tab(&url, !background, window, cx);
                let _ = answer.send(Some(page));
            }
            FromAgent::Close { page, answer } => {
                // The relay refused to close a page another chat's agent
                // is at work on.
                let closed = match stage.index_of(&page) {
                    Some(index) => stage.close_tab(index, window, cx),
                    None => Err("No target with given id found"),
                };
                let _ = answer.send(closed);
            }
        }
    }

    /// Carries out what the tools of the agent of the chat `chat` ask,
    /// saying so in the span this is called in, its chat's (see [`ASKED`]).
    pub(super) fn hear_tools(
        chat: u64,
        mut from_tools: UnboundedReceiver<ToolRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let span = tracing::Span::current();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(request) = from_tools.recv().await {
                let _entered = span.enter();
                tracing::debug!(target: ASKED, "the stage carries it out: {}", request.said());
                let done = this.update_in(cx, |stage, window, cx| {
                    stage.acting_for(Some(chat), |stage| stage.asked(request, window, cx));
                });
                if done.is_err() {
                    break;
                }
            }
        })
    }

    /// Carries out `request`, which the tools of the agent of the chat
    /// acting ask.
    fn asked(&mut self, request: ToolRequest, window: &mut Window, cx: &mut Context<Self>) {
        let stage = self;
        match request {
            ToolRequest::ShowMarkdown { path, text } => {
                stage.show_file::<Document>(path, &text, None, window, cx);
            }
            ToolRequest::ShowFile { path, text, lines } => {
                stage.show_file::<Code>(path, &text, lines, window, cx);
            }
            ToolRequest::ShowChanges { folder } => {
                stage.show_changes(folder, window, cx);
            }
            ToolRequest::ShowMap { map, at } => {
                stage.show_map_at(map.as_deref(), &at, window, cx);
            }
            ToolRequest::RunCommand {
                command,
                cwd,
                answer,
                ..
            } => stage.run_command(command, &cwd, answer, window, cx),
            ToolRequest::MacWindow { act, answer } => stage.mac_act(act, answer, window, cx),
        }
    }

    /// Runs `command`, which the agent of the chat acting asked for, in
    /// `cwd`, and answers `answer` once it ends: on the tab of the agent's
    /// last command if that ended, or a new one.
    fn run_command(
        &mut self,
        command: String,
        cwd: &std::path::Path,
        answer: tokio::sync::oneshot::Sender<Result<Ran, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let running = match Running::start(Program::Command(command), cwd) {
            Ok(running) => running,
            Err(error) => {
                let _ = answer.send(Err(format!("cannot start a terminal: {error}")));
                return;
            }
        };
        let terminal = cx.new(|cx| Terminal::new(running, Some(answer), cx));
        let changes = cx.observe(&terminal, |stage, _, cx| stage.changed(cx));
        let ended = self
            .tabs
            .iter()
            .rposition(|tab| tab.owner == self.acting && tab.content.is_agents_ended(cx));
        if let Some(index) = ended {
            let had_focus = self.tabs[index]
                .content
                .focus_handle(cx)
                .contains_focused(window, cx);
            self.tabs[index] = Tab {
                content: TabContent::Terminal(terminal),
                owner: self.acting,
                _changes: changes,
                _opens: None,
            };
            if self.in_sight() {
                self.tabs.show(index);
            }
            if had_focus {
                self.focus_shown(window, cx);
            }
            self.changed(cx);
        } else {
            self.push(TabContent::Terminal(terminal), changes, None, true, cx);
        }
    }

    /// The agent of the chat acting worked on `page`, sending it `input` if
    /// any: show its tab, if it is in the human's sight.
    fn worked(
        &mut self,
        page: Option<String>,
        input: Option<cox_browser::relay::AgentInput>,
        cx: &mut Context<Self>,
    ) {
        if let Some(page) = &page {
            self.worked_on(page, cx);
        }
        if let Some(index) = page.and_then(|page| self.index_of(&page))
            && self.tabs.shown_index() != Some(index)
            && self.in_sight()
        {
            self.tabs.show(index);
            self.changed(cx);
        }
        if let Some(input) = input
            && self.in_sight()
            && let Some(page) = self.tabs.shown().and_then(|tab| tab.content.page())
        {
            page.update(cx, |page, cx| page.see(input, cx));
        }
    }
}
