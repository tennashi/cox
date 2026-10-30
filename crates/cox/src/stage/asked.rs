//! What the agent asks of the stage. Through the chat's relay, it works on
//! pages, which the stage shows, and opens and closes them. Through Cox's
//! own tools, it shows the human files, as Markdown or as they are
//! written, and runs commands in terminals.

use tokio::sync::mpsc::UnboundedReceiver;

use cox_browser::relay::FromAgent;
use gpui::{Context, Task, Window, prelude::*};

use crate::agent::mcp::{Ran, ToolRequest};
use crate::stage::Stage;
use crate::stage::code::Code;
use crate::stage::document::Document;
use crate::stage::tab::{Tab, TabContent};
use crate::stage::terminal::{Program, Running, Terminal};

impl Stage {
    /// Carries out what the relay tells the stage for the agent.
    pub(super) fn hear(
        mut from_agent: UnboundedReceiver<FromAgent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(heard) = from_agent.recv().await {
                let done = this.update_in(cx, |stage, window, cx| match heard {
                    FromAgent::Worked { page, input } => stage.worked(page, input, cx),
                    FromAgent::Open {
                        url,
                        background,
                        answer,
                    } => {
                        let page = stage.open_tab(&url, !background, window, cx);
                        if !background {
                            stage.show_panel(cx);
                        }
                        let _ = answer.send(Some(page));
                    }
                    FromAgent::Close { page, answer } => {
                        let closed = match stage.index_of(&page) {
                            Some(index) => stage.close_tab(index, window, cx),
                            None => Err("No target with given id found"),
                        };
                        let _ = answer.send(closed);
                    }
                });
                if done.is_err() {
                    break;
                }
            }
        })
    }

    /// Carries out what the agent's tools ask.
    pub(super) fn hear_tools(
        mut from_tools: UnboundedReceiver<ToolRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(request) = from_tools.recv().await {
                let done = this.update_in(cx, |stage, window, cx| match request {
                    ToolRequest::ShowMarkdown { path, text } => {
                        stage.show_file::<Document>(path, &text, None, window, cx);
                    }
                    ToolRequest::ShowFile { path, text, lines } => {
                        stage.show_file::<Code>(path, &text, lines, window, cx);
                    }
                    ToolRequest::RunCommand {
                        command,
                        cwd,
                        answer,
                    } => stage.run_command(command, &cwd, answer, window, cx),
                });
                if done.is_err() {
                    break;
                }
            }
        })
    }

    /// Runs `command`, which the agent asked for, in `cwd`, and answers
    /// `answer` once it ends: on the tab of the agent's last command if
    /// that ended, or a new one. The stage opens if it was closed.
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
            .rposition(|tab| tab.content.is_agents_ended(cx));
        if let Some(index) = ended {
            let had_focus = self.tabs[index]
                .content
                .focus_handle(cx)
                .contains_focused(window, cx);
            self.tabs[index] = Tab {
                content: TabContent::Terminal(terminal),
                _changes: changes,
                _opens: None,
            };
            self.tabs.show(index);
            if had_focus {
                self.focus_shown(window, cx);
            }
            self.changed(cx);
        } else {
            self.push(TabContent::Terminal(terminal), changes, None, true, cx);
        }
        self.show_panel(cx);
    }

    /// The agent worked on `page`, sending it `input` if any: show its tab,
    /// and the stage.
    fn worked(
        &mut self,
        page: Option<String>,
        input: Option<cox_browser::relay::AgentInput>,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = page.and_then(|page| self.index_of(&page))
            && self.tabs.shown_index() != Some(index)
        {
            self.tabs.show(index);
            self.changed(cx);
        }
        self.show_panel(cx);
        if let Some(input) = input
            && let Some(page) = self.tabs.shown().and_then(|tab| tab.content.page())
        {
            page.update(cx, |page, cx| page.see(input, cx));
        }
    }
}
