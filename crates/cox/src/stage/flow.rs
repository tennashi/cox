//! A tab of the stage that shows the flow of one tool call through Cox,
//! read from what Cox kept of the run (see [`coxd::logs::flow`]), for the
//! human to see how the parts of Cox pass the call on before they read
//! the code. The parts are lanes across the tab: the agent's process,
//! Cox's thread for the agent, the chat's tool server, and the window.
//! Each step is a row, in the order the steps were taken, with how long
//! after the first it came, an arrow from the lane that led to it to the
//! lane it is taken in, where in Cox's code it is taken, which a click
//! opens when the project has the file, as a link to it would, and, under
//! those, what was said or done. A step in a file that changed, as the Changes tab
//! shows it, is marked, so that the human sees where a change falls in
//! the flow.
//!
//! The tab looks again now and then, so that it follows a call still
//! under way, and the changes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_git::changes;
use cox_git::changes::changed_files;
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    ScrollHandle, StyledText, Task, Window, div, prelude::*, px, rgb,
};

use crate::stage::heading::{INSET, body, card, heading, note};
use crate::stage::tab::Shows;
use crate::ui::markdown;
use crate::vim::hints::Hinted as _;
use coxd::logs::flow::{self, Lane, Step};
use coxd::theme::{ACCENT, BORDER, CODE, DIM, FAILED, FAINT, FOREGROUND, HOVER, LINK, RAISED};

pub(crate) const KEY_CONTEXT: &str = "Flow";

/// What a link to the flow of a tool call starts with, followed by the
/// call's id.
const LINK_PREFIX: &str = "cox:flow/";

/// How often the tab reads the run again.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// How wide the column of the times is, each lane, and all the lanes.
const TIME_WIDTH: f32 = 72.;
const LANE_WIDTH: f32 = 112.;
const LANES_WIDTH: f32 = LANE_WIDTH * 4.;
/// How tall the line of a step's arrow is.
const ARROW_HEIGHT: f32 = 20.;
/// How big the dot of a step is, and the line of an arrow.
const DOT: f32 = 8.;
const LINE: f32 = 2.;

/// The human chose a step, to open where in Cox's code it is taken: the
/// file, an absolute path, at this line.
pub struct Opened {
    pub path: PathBuf,
    pub line: u64,
}

/// A link to the flow of the tool call `call`.
#[must_use]
pub fn link(call: &str) -> String {
    format!("{LINK_PREFIX}{call}")
}

/// The tool call whose flow `url` links to, if it links to one.
#[must_use]
pub fn linked(url: &str) -> Option<&str> {
    url.strip_prefix(LINK_PREFIX)
        .filter(|call| !call.is_empty())
}

/// The flow as the tab shows it, once read.
#[derive(PartialEq, Eq)]
struct Shown {
    flow: flow::Flow,
    /// Where each step's file is in the project, when the project has it,
    /// and whether it changed.
    sources: Vec<Option<(PathBuf, bool)>>,
}

pub struct Flow {
    /// The chat's folder, where Cox's code is when the project is Cox.
    root: PathBuf,
    /// Where the files of the runs are, if Cox keeps any.
    logs: Option<PathBuf>,
    /// The call, by its id.
    call: String,
    /// The flow, as the tab last read it, or why it could not be read;
    /// nothing until it is first read.
    read: Option<Result<Shown, String>>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    _follow: Task<()>,
}

impl EventEmitter<Opened> for Flow {}

impl Flow {
    /// A tab showing the flow of the tool call `call`, for a chat working
    /// in `root`, an absolute path, from the runs kept in `logs`.
    pub fn new(root: PathBuf, logs: Option<PathBuf>, call: String, cx: &mut Context<Self>) -> Self {
        Self {
            root,
            logs,
            call,
            read: None,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _follow: follow(cx),
        }
    }

    /// Shows the flow of the tool call `call` instead.
    pub fn open(&mut self, call: String, cx: &mut Context<Self>) {
        if call != self.call {
            self.call = call;
            self.read = None;
            self.scroll.set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        }
    }

    /// The tab read the run again: draws the flow again if it changed.
    fn looked(&mut self, call: &str, read: Result<Shown, String>, cx: &mut Context<Self>) {
        if call != self.call {
            // Of the call shown before.
            return;
        }
        if self.read.as_ref() != Some(&read) {
            self.read = Some(read);
            cx.notify();
        }
    }

    /// The heading over the lanes: the call, how many steps it took and
    /// how long, and which run it is read from.
    fn render_heading(&self) -> AnyElement {
        let (said, run) = match &self.read {
            None => ("Reading what Cox kept of the run…".to_owned(), None),
            Some(Err(_)) => ("Cannot be read".to_owned(), None),
            Some(Ok(shown)) => {
                let flow = &shown.flow;
                let title = flow.title.clone().unwrap_or_else(|| flow.call.clone());
                let took = flow.steps.last().map_or(Duration::ZERO, |step| step.after);
                let run = flow.run.file_name().map_or_else(
                    || flow.run.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                let said = format!(
                    "{title} · {} over {}",
                    counted(flow.steps.len(), "step"),
                    elapsed(took)
                );
                (said, Some(run))
            }
        };
        heading("flow-heading", "Flow", said, run).into_any_element()
    }

    /// The lanes' names, over their columns.
    fn render_lanes() -> AnyElement {
        let lanes = Lane::ALL.iter().map(|lane| {
            div()
                .w(px(LANE_WIDTH))
                .flex()
                .flex_col()
                .items_center()
                .px_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(lane.name()))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(FAINT))
                        .text_center()
                        .child(lane.says()),
                )
        });
        div()
            .flex()
            .flex_shrink_0()
            .px_3()
            .py_2()
            .bg(rgb(RAISED))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(div().w(px(TIME_WIDTH)).flex_shrink_0())
            .children(lanes)
            .into_any_element()
    }

    /// A step: when, the arrow into its lane, what it says, and where in
    /// the code it is taken.
    fn render_step(
        index: usize,
        step: &Step,
        source: Option<&(PathBuf, bool)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let number = index + 1;
        let changed = source.is_some_and(|(_, changed)| *changed);
        let code = step
            .source
            .as_ref()
            .map(|source| format!("{}:{}", file_name(&source.file), source.line));
        let color = match (source.is_some(), changed) {
            (true, true) => ACCENT,
            (true, false) => LINK,
            (false, _) => DIM,
        };
        let opens = source.map(|(path, _)| {
            let line = step.source.as_ref().map_or(1, |source| source.line);
            (path.clone(), line)
        });
        let arrow = div()
            .flex()
            .items_center()
            .child(
                div()
                    .w(px(TIME_WIDTH))
                    .flex_shrink_0()
                    .pr_2()
                    .text_xs()
                    .text_color(rgb(DIM))
                    .child(format!("+{}", elapsed(step.after))),
            )
            .child(render_arrow(step.from, step.to, changed))
            .child(div().flex_1())
            .when_some(code, |arrow, code| {
                arrow.child(
                    div()
                        .flex_shrink_0()
                        .pl_3()
                        .text_xs()
                        .text_color(rgb(color))
                        .child(code),
                )
            });
        let row = div()
            .id(("flow-step", number))
            .debug_selector(move || format!("flow-step-{number}"))
            .flex()
            .flex_col()
            .px_3()
            .py_1()
            .child(arrow)
            .child(
                div()
                    .pl(px(TIME_WIDTH))
                    .text_color(rgb(FOREGROUND))
                    .child(said(&step.said)),
            );
        match opens {
            Some((path, line)) => row
                .cursor_pointer()
                .hover(|style| style.bg(rgb(HOVER)))
                .on_hint_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(Opened {
                        path: path.clone(),
                        line,
                    });
                }))
                .into_any_element(),
            None => row.into_any_element(),
        }
    }
}

/// The lanes of a step's row: an arrow from the lane `from` to the lane
/// `to` when they differ, and a dot in `to`, colored as `changed` says.
fn render_arrow(from: Lane, to: Lane, changed: bool) -> AnyElement {
    let (from, to) = (center(from), center(to));
    let middle = ARROW_HEIGHT / 2.;
    let mut lanes = div()
        .relative()
        .w(px(LANES_WIDTH))
        .h(px(ARROW_HEIGHT))
        .flex_shrink_0();
    if (from - to).abs() > f32::EPSILON {
        let (left, right) = (from.min(to), from.max(to));
        let rightward = to > from;
        let head_at = if rightward {
            to - DOT * 1.5
        } else {
            to + DOT / 2.
        };
        lanes = lanes
            .child(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(middle - LINE / 2.))
                    .w(px(right - left))
                    .h(px(LINE))
                    .bg(rgb(DIM)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(head_at))
                    .top(px(middle - DOT))
                    .text_color(rgb(DIM))
                    .child(if rightward { "▸" } else { "◂" }),
            );
    }
    lanes
        .child(
            div()
                .absolute()
                .left(px(to - DOT / 2.))
                .top(px(middle - DOT / 2.))
                .w(px(DOT))
                .h(px(DOT))
                .rounded_full()
                .bg(rgb(if changed { ACCENT } else { LINK })),
        )
        .into_any_element()
}

/// Where the middle of the lane of `lane` is, in points from the left of
/// the lanes.
fn center(lane: Lane) -> f32 {
    let column = Lane::ALL.iter().position(|&l| l == lane).unwrap_or(0);
    LANE_WIDTH * (f32::from(u8::try_from(column).unwrap_or(0)) + 0.5)
}

/// The name of the file at `path`.
fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `took`, in words: milliseconds, or seconds past ten of them.
fn elapsed(took: Duration) -> String {
    if took < Duration::from_secs(10) {
        format!("{} ms", took.as_millis())
    } else {
        format!("{:.1} s", took.as_secs_f64())
    }
}

/// `count` of `what`, with the plural.
fn counted(count: usize, what: &str) -> String {
    if count == 1 {
        format!("1 {what}")
    } else {
        format!("{count} {what}s")
    }
}

/// Reads the run now and then, for as long as the tab lasts.
fn follow(cx: &mut Context<Flow>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Ok((root, logs, call)) = this.read_with(cx, |flow, _| {
                (flow.root.clone(), flow.logs.clone(), flow.call.clone())
            }) else {
                break;
            };
            let read_call = call.clone();
            let read = cx
                .background_executor()
                .spawn(async move { read(&root, logs.as_deref(), &read_call) })
                .await;
            if this
                .update(cx, |flow, cx| flow.looked(&call, read, cx))
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(LOOK_EVERY).await;
        }
    })
}

/// Reads the flow of the tool call `call` from the runs kept in `logs`,
/// and where each step's file is in `root`, if it is there, and whether
/// it changed.
fn read(
    root: &std::path::Path,
    logs: Option<&std::path::Path>,
    call: &str,
) -> Result<Shown, String> {
    let logs = logs.ok_or_else(|| {
        "Cox keeps no log of this run, so the flow of the call cannot be read".to_owned()
    })?;
    let flow = flow::read(logs, call)?;
    let changed: HashMap<PathBuf, (usize, usize)> = changes::read(root)
        .ok()
        .map(|diff| changed_files(&diff))
        .unwrap_or_default();
    let sources = flow
        .steps
        .iter()
        .map(|step| {
            let source = step.source.as_ref()?;
            let path = std::fs::canonicalize(root.join(&source.file)).ok()?;
            path.is_file().then(|| {
                let changed = changed.contains_key(&path);
                (path, changed)
            })
        })
        .collect();
    Ok(Shown { flow, sources })
}

impl Shows for Flow {
    fn title(&self) -> String {
        "Flow".to_owned()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Flow {
            call: self.call.clone(),
        }
    }
}

impl Focusable for Flow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Flow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let steps: Vec<AnyElement> = match &self.read {
            Some(Ok(shown)) => shown
                .flow
                .steps
                .iter()
                .zip(&shown.sources)
                .enumerate()
                .map(|(index, (step, source))| Self::render_step(index, step, source.as_ref(), cx))
                .collect(),
            _ => Vec::new(),
        };
        let error = match &self.read {
            Some(Err(error)) => Some(note("flow-error", error.clone(), FAILED)),
            _ => None,
        };
        let lanes = (!steps.is_empty()).then(|| {
            div().flex_1().min_h_0().px(px(INSET)).pt_1().pb_3().child(
                card().max_h_full().child(Self::render_lanes()).child(
                    div()
                        .id("flow")
                        .debug_selector(|| "flow".to_owned())
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .py_1()
                        .children(steps),
                ),
            )
        });
        body()
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .text_color(rgb(FOREGROUND))
            .child(self.render_heading())
            .children(error)
            .children(lanes)
    }
}

/// What a step says, the names between its backticks drawn as code, as
/// the conversation draws code in a line.
fn said(text: &str) -> StyledText {
    let noted = markdown::Text::noted(text);
    let code = HighlightStyle {
        color: Some(rgb(CODE).into()),
        background_color: Some(rgb(RAISED).into()),
        ..HighlightStyle::default()
    };
    let highlights: Vec<_> = noted
        .runs
        .iter()
        .filter(|(_, style)| style.code)
        .map(|(range, _)| (range.clone(), code))
        .collect();
    StyledText::new(noted.text).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_names_the_call() {
        assert_eq!(link("call-1"), "cox:flow/call-1");
        assert_eq!(linked("cox:flow/call-1"), Some("call-1"));
        assert_eq!(linked("cox:flow/"), None);
        assert_eq!(linked("src/main.rs"), None);
    }

    #[test]
    fn times_are_said_in_milliseconds_then_seconds() {
        assert_eq!(elapsed(Duration::from_millis(12)), "12 ms");
        assert_eq!(elapsed(Duration::from_millis(9_999)), "9999 ms");
        assert_eq!(elapsed(Duration::from_millis(12_340)), "12.3 s");
    }
}
