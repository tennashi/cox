//! A tab of the stage that shows what changed in the files of the chat's
//! project, or of another folder, such as a trial the chat opened, as Git
//! tells it (see [`cox_editor::changes`]): in a trial, all it changed
//! since it started, and elsewhere what changed since the last commit. Each file that changed heads the lines that changed in
//! it, with some around them, and a click on a file or a line opens the
//! file there, as a link to it would (see [`crate::stage`]).
//!
//! The tab looks again now and then, so that it shows what the agent
//! changes as it works. It draws only the rows in sight, so a long list
//! of changes scrolls as a short one does.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_editor::changes::{self, Diff, File, How, Kind, Line};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Task,
    UniformListScrollHandle, Window, div, prelude::*, px, rgb, uniform_list,
};

use crate::stage::tab::Shows;
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT, ADDED_GROUND, BORDER, CODE_FONT, DIM, FAILED, FOREGROUND, HOVER, RAISED, REMOVED_GROUND,
};

/// How often the tab looks at what changed.
const LOOK_EVERY: Duration = Duration::from_secs(2);

const STRETCH: u32 = 0x25_2b36;

/// How wide a line's numbers are, in characters.
const NUMBER: usize = 5;

/// The human chose a file, at a line of it if they chose one, to open it.
pub struct Chosen {
    /// The file, as an absolute path.
    pub path: PathBuf,
    pub line: Option<usize>,
}

pub struct Changes {
    /// A folder of the repository whose changes the tab shows.
    folder: PathBuf,
    /// The tab's name: "Changes", and the folder's name when it is not
    /// where the chat works.
    title: String,
    /// What changed, as the tab last read it, or why it could not be read;
    /// nothing until it is first read.
    read: Option<Result<Diff, String>>,
    /// The rows of the tab: a file's, then its lines'.
    rows: Vec<Row>,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    _follow: Task<()>,
}

/// A row of the tab: the file at an index of what changed, or a line of
/// it.
#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Line(usize, usize),
}

impl EventEmitter<Chosen> for Changes {}

impl Changes {
    /// A tab showing what changed in the repository `folder` is in, for a
    /// chat working in `cwd`.
    pub fn new(folder: PathBuf, cwd: &Path, cx: &mut Context<Self>) -> Self {
        let title = if folder == cwd {
            "Changes".to_owned()
        } else {
            let name = folder.file_name().map_or_else(
                || folder.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            format!("Changes in {name}")
        };
        Self {
            folder,
            title,
            read: None,
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _follow: follow(cx),
        }
    }

    /// The folder of the repository whose changes the tab shows.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// What changed, as the tab shows it, once read.
    #[must_use]
    pub fn diff(&self) -> Option<&Diff> {
        self.read.as_ref().and_then(|read| read.as_ref().ok())
    }

    /// The tab read what changed again: draws it again if that changed.
    fn looked(&mut self, read: Result<Diff, String>, cx: &mut Context<Self>) {
        if self.read.as_ref() == Some(&read) {
            return;
        }
        self.rows = match &read {
            Ok(diff) => rows(diff),
            Err(_) => Vec::new(),
        };
        self.read = Some(read);
        cx.notify();
    }

    /// The human chose the file at `index`, at `line` if they chose one.
    fn choose(&self, index: usize, line: Option<usize>, cx: &mut Context<Self>) {
        let Some(diff) = self.diff() else {
            return;
        };
        let file = &diff.files[index];
        if file.how != How::Removed {
            cx.emit(Chosen {
                path: diff.top.join(&file.path),
                line,
            });
        }
    }

    fn render_heading(&self) -> impl IntoElement {
        let said = match &self.read {
            None => "Reading what changed…".to_owned(),
            Some(Err(error)) => format!("Cannot tell what changed: {error}"),
            Some(Ok(diff)) if diff.files.is_empty() => {
                format!("Nothing changed {}.", diff.since.said())
            }
            Some(Ok(diff)) => {
                let (added, removed) = diff.counts();
                let files = match diff.files.len() {
                    1 => "1 file".to_owned(),
                    count => format!("{count} files"),
                };
                format!("{files} changed {}: +{added} −{removed}", diff.since.said())
            }
        };
        let failed = matches!(self.read, Some(Err(_)));
        let top = self
            .diff()
            .map_or_else(|| self.folder.clone(), |diff| diff.top.clone());
        div()
            .debug_selector(|| "changes-heading".to_owned())
            .flex()
            .gap_3()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(if failed { FAILED } else { FOREGROUND }))
                    .child(said),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(DIM))
                    .child(shortened(&top)),
            )
    }
}

/// The rows of `diff`: each file's, then its lines'.
fn rows(diff: &Diff) -> Vec<Row> {
    diff.files
        .iter()
        .enumerate()
        .flat_map(|(index, file)| {
            std::iter::once(Row::File(index))
                .chain((0..file.lines.len()).map(move |line| Row::Line(index, line)))
        })
        .collect()
}

/// Reads what changed in the tab of `cx` at once, and again now and
/// then, for as long as the tab is.
fn follow(cx: &mut Context<Changes>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Ok(folder) = this.read_with(cx, |changes, _| changes.folder.clone()) else {
                break;
            };
            let read = cx
                .background_executor()
                .spawn(async move { changes::read(&folder) })
                .await;
            if this
                .update(cx, |changes, cx| changes.looked(read, cx))
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(LOOK_EVERY).await;
        }
    })
}

/// The rows of `range` of the tab `this`.
fn render_rows(this: &Entity<Changes>, range: Range<usize>, cx: &App) -> Vec<AnyElement> {
    let changes = this.read(cx);
    let Some(diff) = changes.diff() else {
        return Vec::new();
    };
    range
        .filter_map(|row| changes.rows.get(row).map(|&listed| (row, listed)))
        .map(|(row, listed)| {
            let this = this.clone();
            let (index, line) = match listed {
                Row::File(index) => (index, None),
                Row::Line(index, line) => (index, Some(&diff.files[index].lines[line])),
            };
            let file = &diff.files[index];
            let element = match line {
                None => file_row(row, file),
                Some(line) => line_row(row, line),
            };
            let at = line.map(|line| line.at);
            let element = if file.how == How::Removed {
                element
            } else {
                element
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)))
                    .on_click(move |_, _, cx| {
                        this.update(cx, |changes, cx| changes.choose(index, at, cx));
                    })
            };
            element.into_any_element()
        })
        .collect()
}

/// The row of `file`, at `row` of the tab: how it changed, where it is,
/// and how many lines it gained and lost.
fn file_row(row: usize, file: &File) -> gpui::Stateful<gpui::Div> {
    let how = file.how;
    let path = match &file.was {
        Some(was) => format!("{} ← {}", file.path.display(), was.display()),
        None => file.path.display().to_string(),
    };
    let counts = if file.binary {
        "binary".to_owned()
    } else {
        format!("+{} −{}", file.added, file.removed)
    };
    let letter_color = match how {
        How::Added => ACCENT,
        How::Removed => FAILED,
        How::Changed | How::Moved => DIM,
    };
    div()
        .id(("changed-file", row))
        .debug_selector(move || format!("changed-file-{row}"))
        .w_full()
        .flex()
        .gap_2()
        .px_3()
        .bg(rgb(RAISED))
        .whitespace_nowrap()
        .overflow_hidden()
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(letter_color))
                .child(how.letter().to_string()),
        )
        .child(div().min_w_0().truncate().child(path))
        .child(div().flex_shrink_0().text_color(rgb(DIM)).child(counts))
}

/// The row of `line`, at `row` of the tab: its numbers, as it was and as
/// it is, and its text, on the ground its kind has.
fn line_row(row: usize, line: &Line) -> gpui::Stateful<gpui::Div> {
    let number = |number: Option<usize>| match number {
        Some(number) => format!("{number:>NUMBER$}"),
        None => " ".repeat(NUMBER),
    };
    let (sign, ground, color) = match line.kind {
        Kind::Stretch => ("", Some(STRETCH), DIM),
        Kind::Same => (" ", None, FOREGROUND),
        Kind::Added => ("+", Some(ADDED_GROUND), FOREGROUND),
        Kind::Removed => ("-", Some(REMOVED_GROUND), FOREGROUND),
    };
    let text = format!("{sign}{}", line.text.replace('\t', "    "));
    let element = div()
        .id(("changed-line", row))
        .debug_selector(move || format!("changed-line-{row}"))
        .w_full()
        .flex()
        .gap_2()
        .pl_1()
        .pr_3()
        .whitespace_nowrap()
        .overflow_hidden()
        .child(div().flex_shrink_0().text_color(rgb(DIM)).child(format!(
            "{} {}",
            number(line.old),
            number(line.new)
        )))
        .child(div().min_w_0().text_color(rgb(color)).child(text));
    match ground {
        Some(ground) => element.bg(rgb(ground)),
        None => element,
    }
}

impl Shows for Changes {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Changes {
            folder: self.folder.clone(),
        }
    }
}

impl Focusable for Changes {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Changes {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        let list = uniform_list("changes", self.rows.len(), move |range, _, cx| {
            render_rows(&this, range, cx)
        })
        .track_scroll(self.scroll.clone())
        .flex_1()
        .min_h_0();
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .text_color(rgb(FOREGROUND))
            .font_family(CODE_FONT)
            .text_size(px(13.))
            .child(self.render_heading())
            .child(list)
    }
}
