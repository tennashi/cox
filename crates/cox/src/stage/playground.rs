//! A tab of the stage that is a playground: code an agent wrote, which the
//! human clicked Run on, to edit and run again, as in the Go playground.
//! Its code is above, in an input with Vim's keys, and what the last run
//! printed below, in a terminal, which the program reads what the human
//! types from too.
//!
//! Run, or Command-Enter (Control-Enter off macOS), builds and runs the
//! code as it is now, from a clean folder each time (see [`build`]), from
//! the chat's folder, stopping the run before if it still runs. "Send to
//! agent" hands the chat shown the code of the last run, the end of what
//! it printed and how it ended, to go with the human's next line.
//!
//! A playground is not kept: a stage opened again does not show it, as it
//! does not start a terminal again.

pub mod build;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cox_core::entry::Attachment;
use cox_core::tabs::KeptTab;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyBinding, Subscription, Window,
    actions, div, prelude::*, px, rgb,
};

use crate::stage::tab::Shows;
use crate::stage::terminal::{Exited, Terminal};
use crate::ui::button::{Kind, button};
use crate::ui::input::LineInput;
use crate::ui::snippet::{HANDED_LINES, Lang, Snippet};
use coxd::storage::Kept;
use coxd::terminal::{Program, Running};
use coxd::theme::{BORDER, CODE_FONT, DIM};

pub(crate) const KEY_CONTEXT: &str = "Playground";

actions!(
    playground,
    [
        /// Builds and runs the playground's code as it is now.
        RunCode,
    ]
);

/// Binds the key that runs the code: Command-Enter, or Control-Enter off
/// macOS.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "secondary-enter",
        RunCode,
        Some(KEY_CONTEXT),
    )]);
}

/// The next playground's number, which names its folder.
static NEXT: AtomicU64 = AtomicU64::new(1);

pub struct Playground {
    lang: Lang,
    /// The code the playground opened with, which the stage finds it by
    /// when the human clicks Run on the same code again.
    opened_with: String,
    editor: Entity<LineInput>,
    /// The chat's folder, which the code runs from.
    cwd: PathBuf,
    /// Where each run writes its files, emptied first.
    folder: PathBuf,
    /// Rust's target folder, which every playground shares.
    target: PathBuf,
    run: Option<LastRun>,
}

/// The last run, and what it printed.
struct LastRun {
    /// The code it ran.
    source: String,
    terminal: Entity<Terminal>,
    started: Instant,
    /// How long it ran, once it ended.
    took: Option<Duration>,
    _changes: Subscription,
    _ended: Subscription,
}

impl Playground {
    /// A playground of `snippet`, which runs from `cwd`, not run yet.
    pub fn new(snippet: &Snippet, cwd: &Path, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut editor = LineInput::filling(cx);
            editor.set_text_at(&snippet.source, 0, cx);
            editor
        });
        let playgrounds = coxd::storage::path(Kept::Playgrounds)
            .unwrap_or_else(|| std::env::temp_dir().join("cox-playgrounds"));
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        Self {
            lang: snippet.lang,
            opened_with: snippet.source.clone(),
            editor,
            cwd: cwd.to_owned(),
            folder: playgrounds.join(number.to_string()),
            target: playgrounds.join("target"),
            run: None,
        }
    }

    /// Whether the playground opened with `snippet`.
    #[must_use]
    pub fn opened_with(&self, snippet: &Snippet) -> bool {
        self.lang == snippet.lang && self.opened_with == snippet.source
    }

    /// The code as the human edited it.
    #[must_use]
    pub fn source(&self, cx: &App) -> String {
        self.editor.read(cx).text().to_owned()
    }

    /// Puts `source` in place of the code.
    pub fn set_source(&mut self, source: &str, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_text_at(source, 0, cx));
    }

    /// The terminal of the last run, if it ran.
    #[must_use]
    pub fn terminal(&self) -> Option<&Entity<Terminal>> {
        self.run.as_ref().map(|run| &run.terminal)
    }

    /// How long the last run took, once it ended.
    #[must_use]
    pub fn took(&self) -> Option<Duration> {
        self.run.as_ref().and_then(|run| run.took)
    }

    /// Builds and runs the code as it is now, from a clean folder, once
    /// the run before, if it still runs, is stopped.
    pub fn run(&mut self, cx: &mut Context<Self>) {
        let source = self.source(cx);
        self.run = None;
        let command = build::build(self.lang, &source, &self.folder, &self.target)
            .write(&self.folder)
            .unwrap_or_else(|error| {
                build::failing(&format!("Cox cannot write the playground's files: {error}"))
            });
        let running = match Running::start(Program::Command(command), &self.cwd) {
            Ok(running) => running,
            Err(error) => {
                tracing::error!("cannot start a playground's run: {error}");
                return;
            }
        };
        let terminal = cx.new(|cx| Terminal::new(running, None, cx));
        let changes = cx.observe(&terminal, |_, _, cx| cx.notify());
        let ended = cx.subscribe(&terminal, |playground, _, _: &Exited, cx| {
            if let Some(run) = &mut playground.run {
                run.took = Some(run.started.elapsed());
            }
            cx.notify();
        });
        self.run = Some(LastRun {
            source,
            terminal,
            started: Instant::now(),
            took: None,
            _changes: changes,
            _ended: ended,
        });
        cx.notify();
    }

    /// What "Send to agent" hands the chat: the code of the last run, the
    /// end of what it printed, and how it ended, once it did.
    #[must_use]
    pub fn attachment(&self, cx: &App) -> Option<Attachment> {
        let run = self.run.as_ref()?;
        let terminal = run.terminal.read(cx);
        let ended = terminal.ended()?;
        let took = run.took.map_or_else(String::new, |took| {
            format!(", in {:.1}s", took.as_secs_f64())
        });
        Some(Attachment {
            label: format!("{} playground", self.lang.title()),
            uri: "cox://playground".to_owned(),
            text: format!(
                "Ran in a {} playground:\n```{}\n{}\n```\n\nIt printed:\n{}\n({ended}{took})",
                self.lang.title(),
                self.lang.title().to_ascii_lowercase(),
                run.source.trim_end(),
                terminal.last_lines(HANDED_LINES),
            ),
            picture: None,
        })
    }

    /// Hands the chat shown what the last run ran and printed.
    fn send(&mut self, cx: &mut Context<Self>) {
        if let Some(attachment) = self.attachment(cx) {
            crate::chat::hand::hand(attachment, cx);
        }
    }

    /// The line under the buttons: how the last run went, or, before one,
    /// what Run does.
    fn status(&self, cx: &App) -> String {
        let Some(run) = &self.run else {
            return match self.lang {
                Lang::Rust => "Dependencies: a //! ```cargo block at the top, \
                               or the crates `use` names, at their newest"
                    .to_owned(),
                _ => String::new(),
            };
        };
        match (run.terminal.read(cx).ended(), run.took) {
            (Some(ended), Some(took)) => format!("{ended} in {:.1}s", took.as_secs_f64()),
            (Some(ended), None) => ended.to_string(),
            (None, _) => "Running…".to_owned(),
        }
    }

    fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ended = self
            .run
            .as_ref()
            .is_some_and(|run| run.terminal.read(cx).ended().is_some());
        let keys = if cfg!(target_os = "macos") {
            "⌘↩"
        } else {
            "Ctrl-↩"
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex_shrink_0()
                    .child(format!("{} playground", self.lang.title())),
            )
            .child(
                button("playground-run", format!("Run {keys}"), Kind::Primary)
                    .on_click(cx.listener(|playground, _, _, cx| playground.run(cx))),
            )
            .when(ended, |bar| {
                bar.child(
                    button("playground-send", "Send to agent", Kind::Plain)
                        .on_click(cx.listener(|playground, _, _, cx| playground.send(cx))),
                )
            })
            .child(
                div()
                    .debug_selector(|| "playground-status".to_owned())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(px(12.))
                    .text_color(rgb(DIM))
                    .child(self.status(cx)),
            )
    }
}

impl Render for Playground {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let output = self.run.as_ref().map(|run| run.terminal.clone());
        div()
            .size_full()
            .flex()
            .flex_col()
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|playground, _: &RunCode, _, cx| playground.run(cx)))
            .child(self.render_bar(cx))
            .child(
                div()
                    .id("playground-code")
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .px_3()
                    .py_2()
                    .font_family(CODE_FONT)
                    .text_size(px(13.))
                    .child(self.editor.clone()),
            )
            .child(
                div()
                    .h(gpui::relative(0.45))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .font_family(CODE_FONT)
                    .children(output),
            )
    }
}

impl Focusable for Playground {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Shows for Playground {
    fn title(&self) -> String {
        format!("{} playground", self.lang.title())
    }

    /// Kept as a terminal is, which a stage opened again does not start.
    fn kept(&self) -> KeptTab {
        KeptTab::Terminal {
            title: self.title(),
        }
    }
}
