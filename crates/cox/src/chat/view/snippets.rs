//! The snippets of the agent's replies that the human ran: each code block
//! that runs, by where it is, stands as [`Run`] says, and its buttons
//! run it on the stage, or hand what it printed to the chat.

use std::collections::HashMap;
use std::rc::Rc;

use cox_core::entry::Attachment;
use gpui::{App, Context, Entity, Subscription, WeakEntity, Window};

use super::ChatView;
use crate::stage::terminal::Terminal;
use crate::ui::snippet::Snippet;
use crate::ui::text::{CodeAt, Run, RunClick, Runs};

/// How many of the last lines a snippet printed go to the chat.
const ATTACHED_LINES: usize = 80;

/// What a snippet the human clicked Run on became.
enum Clicked {
    /// Destructive, and asked once more.
    Confirming,
    /// Started in its terminal tab, which the view watches to show when
    /// it ends.
    Started {
        terminal: WeakEntity<Terminal>,
        _watch: Subscription,
    },
}

#[derive(Default)]
pub(super) struct Snippets(HashMap<CodeAt, Clicked>);

impl Snippets {
    fn state(&self, at: CodeAt, cx: &App) -> Run {
        match self.0.get(&at) {
            None => Run::Ready,
            Some(Clicked::Confirming) => Run::Confirming,
            Some(Clicked::Started { terminal, .. }) => {
                terminal.upgrade().map_or(Run::Ready, |terminal| {
                    if terminal.read(cx).ended().is_some() {
                        Run::Ended
                    } else {
                        Run::Running
                    }
                })
            }
        }
    }

    fn terminal(&self, at: CodeAt) -> Option<Entity<Terminal>> {
        match self.0.get(&at)? {
            Clicked::Started { terminal, .. } => terminal.upgrade(),
            Clicked::Confirming => None,
        }
    }
}

impl ChatView {
    /// The buttons of the snippets of a reply, for its writer.
    pub(super) fn runs(&self, cx: &Context<Self>) -> Runs {
        let states = self
            .snippets
            .0
            .keys()
            .map(|at| (*at, self.snippets.state(*at, cx)))
            .collect();
        let view = cx.weak_entity();
        Runs {
            states: Rc::new(states),
            clicked: Rc::new(move |at, click, snippet, window, cx| {
                let _ = view.update(cx, |view, cx| match click {
                    RunClick::Run => view.run_snippet(at, snippet, window, cx),
                    RunClick::Attach => view.attach_output(at, snippet, window, cx),
                });
            }),
        }
    }

    /// Runs the snippet at `at` on the stage, once the human confirmed it
    /// if it is destructive. While it runs, Run does nothing.
    fn run_snippet(
        &mut self,
        at: CodeAt,
        snippet: &Snippet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = self.snippets.state(at, cx);
        if now == Run::Running {
            return;
        }
        if snippet.danger.is_some() && now != Run::Confirming {
            self.snippets.0.insert(at, Clicked::Confirming);
            cx.notify();
            return;
        }
        let Some(runs) = self.stage.as_ref().map(|stage| Rc::clone(&stage.runs)) else {
            return;
        };
        if let Some(terminal) = runs(snippet, window, cx) {
            let watch = cx.observe(&terminal, |_, _, cx| cx.notify());
            self.snippets.0.insert(
                at,
                Clicked::Started {
                    terminal: terminal.downgrade(),
                    _watch: watch,
                },
            );
        } else {
            self.snippets.0.remove(&at);
        }
        cx.notify();
    }

    /// Hands the chat what the snippet at `at` ran and the end of what it
    /// printed, to go with the human's next line.
    fn attach_output(
        &mut self,
        at: CodeAt,
        snippet: &Snippet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.snippets.terminal(at) else {
            return;
        };
        let attachment = output_of(snippet, terminal.read(cx));
        self.attach(attachment, window, cx);
    }
}

/// What `snippet` is, the last lines it printed in `terminal`, and how
/// it ended, under the snippet's first line.
fn output_of(snippet: &Snippet, terminal: &Terminal) -> Attachment {
    let text = terminal.text();
    let lines: Vec<_> = text.lines().collect();
    let printed = lines[lines.len().saturating_sub(ATTACHED_LINES)..].join("\n");
    let ended = terminal
        .ended()
        .map_or_else(String::new, |ended| format!("\n({ended})"));
    Attachment {
        label: snippet.source.lines().next().unwrap_or_default().to_owned(),
        uri: "cox://terminal".to_owned(),
        text: format!("Ran:\n{}\n\nIt printed:\n{printed}{ended}", snippet.source),
        picture: None,
    }
}
