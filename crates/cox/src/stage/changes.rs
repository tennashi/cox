//! A tab of the stage that shows what changed in the files of the chat's
//! project, or of another folder, such as a sketch the chat opened, as Git
//! tells it (see [`cox_editor::changes`]): in a sketch, all it changed
//! since it started, and elsewhere what changed since the last commit.
//! Each file that changed is a card that heads the lines that changed in
//! it, with some around them, drawn in the colors of their syntax as a
//! tab of code draws them, and the words that changed within a line marked
//! (see [`cox_editor::words`]); a click on a file or a line opens the file
//! there, as a link to it would (see [`crate::stage`]).
//!
//! A project's folder is not a repository itself: its repositories are
//! its directories (see [`DIRECTORIES`]). The tab of such a folder shows
//! what changed in one of them at a time, which the human chooses by its
//! name under the heading, beside how many files changed in each (see
//! [`repositories`]).
//!
//! The tab looks again now and then, so that it shows what the agent
//! changes as it works. It draws only the rows in sight, so a long list
//! of changes scrolls as a short one does, and stays where the human
//! scrolled to as the changes do.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_editor::buffer::Buffer;
use cox_editor::changes::{self, Diff, File, How, Kind, Line};
use cox_editor::language::{Highlight, Language};
use cox_editor::syntax::Syntax;
use cox_editor::words;
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, HighlightStyle,
    ListAlignment, ListOffset, ListState, SharedString, StyledText, Task, Window, div, list,
    prelude::*, px, rgb,
};

use crate::settings::DIRECTORIES;
use crate::stage::code::syntax_style;
use crate::stage::heading::{CARD_RADIUS, INSET, heading};
use crate::stage::tab::Shows;
use crate::ui::ellipsis::ellipsis;
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT, ADDED_GROUND, ADDED_WORD, BORDER, CODE_FONT, DIM, DISABLED, FAILED, FAILED_MARK, FAINT,
    FOREGROUND, HOVER, RAISED, REMOVED_GROUND, REMOVED_WORD, SELECTED, UI_FONT,
};

/// How often the tab looks at what changed.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// How wide a line's numbers are, in characters.
const NUMBER: usize = 5;

/// The size code is drawn at, and the height of its lines, as a tab of
/// code draws them.
const CODE_SIZE: f32 = 13.;
const LINE_HEIGHT: f32 = 18.;
/// How long a stretch's row is: a little longer than a line's, to part
/// the stretches.
const STRETCH_HEIGHT: f32 = 26.;

/// What the syntax says the stretches of a line are, as bytes of the line
/// as it is drawn.
type Spans = Vec<(Range<usize>, Highlight)>;

/// How a line is drawn, as bytes of the line as it is drawn: what its
/// syntax says of it, and, for a line a change replaced, or one it put in
/// another's place, the words it changed (see [`cox_editor::words`]).
#[derive(Clone, Default)]
struct Look {
    syntax: Spans,
    changed: Vec<Range<usize>>,
}

/// The human chose a file, at a line of it if they chose one, to open it.
pub struct Chosen {
    /// The file, as an absolute path.
    pub path: PathBuf,
    pub line: Option<usize>,
}

/// What changed in one of the repositories a folder's changes are of (see
/// [`repositories`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    /// The name of the project's directory it is, or none for the
    /// repository the folder is in.
    pub name: Option<String>,
    /// What changed in it, or why it could not be read.
    pub read: Result<Diff, String>,
}

/// The repositories whose changes are those of `folder`: the directories
/// of the project in `folder` that are repositories, by name, after
/// `folder` itself if it is one too; or else, for a folder with no such
/// directory, the repository `folder` is in, if it is in one.
#[must_use]
pub fn repositories(folder: &Path) -> Vec<(Option<String>, PathBuf)> {
    let mut directories: Vec<(Option<String>, PathBuf)> =
        std::fs::read_dir(folder.join(DIRECTORIES))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join(".git").exists())
            .filter_map(|entry| Some((Some(entry.file_name().into_string().ok()?), entry.path())))
            .collect();
    directories.sort();
    if directories.is_empty() || folder.join(".git").exists() {
        directories.insert(0, (None, folder.to_owned()));
    }
    directories
}

/// What changed in each of the repositories of `folder` (see
/// [`repositories`]).
#[must_use]
pub fn read(folder: &Path) -> Vec<Repository> {
    repositories(folder)
        .into_iter()
        .map(|(name, folder)| Repository {
            name,
            read: changes::read(&folder),
        })
        .collect()
}

/// How many files changed in the repositories of `folder` in all (see
/// [`repositories`]), as [`changes::count`] counts them.
///
/// # Errors
///
/// When Git can tell what changed in none of them, as when `folder` is in
/// no repository and has no directory that is one, with what Git said.
pub fn count(folder: &Path) -> Result<usize, String> {
    let mut counted = None;
    let mut failed = None;
    for (_, folder) in repositories(folder) {
        match changes::count(&folder) {
            Ok(count) => *counted.get_or_insert(0) += count,
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
    }
    counted.ok_or_else(|| failed.unwrap_or_default())
}

pub struct Changes {
    /// The folder whose changes the tab shows (see [`repositories`]).
    folder: PathBuf,
    /// The tab's name: "Changes", and the folder's name when it is not
    /// where the chat works.
    title: String,
    /// What changed in each repository, as the tab last read it; nothing
    /// until it is first read.
    read: Option<Vec<Repository>>,
    /// The repository the tab shows, of those, once the human chose one;
    /// until then, the first that changed anything.
    chosen: Option<Choice>,
    /// How each line of each file is drawn, by repository, then file,
    /// then line.
    looks: Vec<Vec<Vec<Look>>>,
    /// The rows of the tab, of the repository it shows: each file's,
    /// followed by its lines'.
    rows: Vec<Row>,
    list: ListState,
    focus_handle: FocusHandle,
    _follow: Task<()>,
}

/// A repository the human chose to show, by its name (see
/// [`Repository::name`]).
struct Choice(Option<String>);

/// A row of the tab: the file at an index of the repository shown, or a
/// line of it.
#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Line(usize, usize),
}

impl EventEmitter<Chosen> for Changes {}

impl Changes {
    /// A tab showing what changed in the repositories of `folder` (see
    /// [`repositories`]), for a chat working in `cwd`.
    pub fn new(folder: PathBuf, cwd: &Path, cx: &mut Context<Self>) -> Self {
        let title = if folder == cwd {
            "Changes".to_owned()
        } else {
            format!("Changes in {}", name_of(&folder))
        };
        Self {
            folder,
            title,
            read: None,
            chosen: None,
            looks: Vec::new(),
            rows: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(LINE_HEIGHT * 20.)),
            focus_handle: cx.focus_handle(),
            _follow: follow(cx),
        }
    }

    /// The folder whose changes the tab shows.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// What changed in each repository, as the tab read it; none until
    /// it is first read.
    #[must_use]
    pub fn repositories(&self) -> &[Repository] {
        self.read.as_deref().unwrap_or_default()
    }

    /// The repository the tab shows, of [`Self::repositories`].
    #[must_use]
    pub fn shown(&self) -> Option<&Repository> {
        self.shown_index()
            .and_then(|index| self.repositories().get(index))
    }

    /// Shows the repository `name` (see [`Repository::name`]), from its
    /// top, as the human chose it.
    pub fn show(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.chosen = Some(Choice(name));
        self.relist(false);
        cx.notify();
    }

    /// The index of the repository the tab shows: the one the human
    /// chose, while there is one of its name, or else the first that
    /// changed anything, or else the first.
    fn shown_index(&self) -> Option<usize> {
        let repositories = self.repositories();
        if let Some(Choice(chosen)) = &self.chosen
            && let Some(index) = repositories.iter().position(|shown| &shown.name == chosen)
        {
            return Some(index);
        }
        let changed = repositories
            .iter()
            .position(|shown| shown.read.as_ref().is_ok_and(|diff| !diff.files.is_empty()));
        changed.or((!repositories.is_empty()).then_some(0))
    }

    /// Lists the rows of the repository shown again: where the human had
    /// scrolled to if `stay`, or else from its top.
    fn relist(&mut self, stay: bool) {
        self.rows = self
            .shown()
            .and_then(|shown| shown.read.as_ref().ok())
            .map(rows)
            .unwrap_or_default();
        let top = self.list.logical_scroll_top();
        self.list.splice(0..self.list.item_count(), self.rows.len());
        if stay && !self.rows.is_empty() {
            self.list.scroll_to(ListOffset {
                item_ix: top.item_ix.min(self.rows.len() - 1),
                offset_in_item: top.offset_in_item,
            });
        }
    }

    /// The tab read what changed again, which differs from what it shows,
    /// and how its lines are drawn: draws it, where the human
    /// had scrolled to.
    fn looked(
        &mut self,
        read: Vec<Repository>,
        looks: Vec<Vec<Vec<Look>>>,
        cx: &mut Context<Self>,
    ) {
        let was = self.shown().map(|shown| shown.name.clone());
        self.read = Some(read);
        self.looks = looks;
        let stay = was == self.shown().map(|shown| shown.name.clone());
        self.relist(stay);
        cx.notify();
    }

    /// The human chose the file at `index` of the repository shown, at
    /// `line` if they chose one.
    fn choose(&self, index: usize, line: Option<usize>, cx: &mut Context<Self>) {
        let Some(Ok(diff)) = self.shown().map(|shown| &shown.read) else {
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

    /// The tab's heading, as the plugins head their pages: its name, what
    /// changed in the repository shown, and where it is.
    fn render_heading(&self) -> impl IntoElement {
        let shown = self.shown();
        let (said, failed) = match shown.map(|shown| &shown.read) {
            None if self.read.is_none() => (div().child("Reading what changed…"), false),
            None => (div(), false),
            Some(Ok(diff)) => (said_of(diff), false),
            Some(Err(error)) => (
                div().child(format!("Cannot tell what changed: {error}")),
                true,
            ),
        };
        let top = shown
            .and_then(|shown| shown.read.as_ref().ok())
            .map_or_else(|| self.folder.clone(), |diff| diff.top.clone());
        heading(
            "changes-heading",
            self.title.clone(),
            said.text_color(rgb(if failed { FAILED } else { DIM })),
            Some(shortened(&top)),
        )
    }

    /// The repositories the tab can show, side by side under its heading,
    /// each by its name, with how many files changed in it, the one shown
    /// marked, to show another; nothing for the one repository a folder
    /// is in, which is all there is to show.
    fn render_repositories(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let repositories = self.repositories();
        if matches!(repositories, [] | [Repository { name: None, .. }]) {
            return None;
        }
        let shown = self.shown_index();
        let chips = repositories.iter().enumerate().map(|(index, repository)| {
            let chosen = shown == Some(index);
            let name = repository
                .name
                .clone()
                .unwrap_or_else(|| name_of(&self.folder));
            let mark = match &repository.read {
                Ok(diff) if diff.files.is_empty() => None,
                Ok(diff) => Some((diff.files.len().to_string(), ACCENT)),
                Err(_) => Some(("!".to_owned(), FAILED)),
            };
            let picked = repository.name.clone();
            div()
                .id(("changed-repository", index))
                .debug_selector(move || format!("changed-repository-{index}"))
                .flex_shrink_0()
                .h(px(26.))
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .rounded(px(8.))
                .font_family(CODE_FONT)
                .text_size(px(12.5))
                .cursor_pointer()
                .map(|chip| {
                    if chosen {
                        chip.bg(rgb(SELECTED)).text_color(rgb(FOREGROUND))
                    } else {
                        chip.text_color(rgb(DIM))
                            .hover(|style| style.bg(rgb(HOVER)))
                    }
                })
                .child(name)
                .children(mark.map(|(mark, color)| {
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(color))
                        .child(mark)
                }))
                .on_click(cx.listener(move |changes, _, _, cx| {
                    changes.show(picked.clone(), cx);
                }))
        });
        Some(
            div().flex().px(px(INSET)).pt(px(6.)).child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(2.))
                    .p(px(3.))
                    .rounded(px(11.))
                    .bg(rgb(RAISED))
                    .children(chips),
            ),
        )
    }

    /// The row at `row` of the tab.
    fn render_row(&self, row: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(&listed) = self.rows.get(row) else {
            return div().into_any_element();
        };
        let (Some(repository), Some(Ok(diff))) =
            (self.shown_index(), self.shown().map(|shown| &shown.read))
        else {
            return div().into_any_element();
        };
        let (index, line) = match listed {
            Row::File(index) => (index, None),
            Row::Line(index, line) => (index, Some(line)),
        };
        let file = &diff.files[index];
        let last = line.map_or(file.lines.is_empty(), |line| line + 1 == file.lines.len());
        let (inner, at) = match line {
            None => (file_row(row, file), None),
            Some(line) => {
                let none = Look::default();
                let look = self
                    .looks
                    .get(repository)
                    .and_then(|looks| looks.get(index))
                    .and_then(|looks| looks.get(line))
                    .unwrap_or(&none);
                let first = line == 0;
                let shown = &file.lines[line];
                (line_row(row, shown, first, last, look), Some(shown.at))
            }
        };
        let inner = if last {
            inner.rounded_b(px(CARD_RADIUS)).border_b_1()
        } else {
            inner
        };
        let inner = if file.how == How::Removed {
            inner
        } else {
            inner
                .cursor_pointer()
                .on_click(cx.listener(move |changes, _, _, cx| changes.choose(index, at, cx)))
        };
        div()
            .px(px(INSET))
            .when(line.is_none(), |row| row.pt(px(10.)))
            .when(last, |row| row.pb(px(4.)))
            .child(inner.border_x_1().border_color(rgb(BORDER)))
            .into_any_element()
    }
}

/// The name of `folder`, its last part's.
fn name_of(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What changed in `diff`, as the heading says it: how many files, the
/// lines they gained and lost, and since what.
fn said_of(diff: &Diff) -> gpui::Div {
    if diff.files.is_empty() {
        return div().child(format!("Nothing changed {}.", diff.since.said()));
    }
    let (added, removed) = diff.counts();
    let files = match diff.files.len() {
        1 => "1 file".to_owned(),
        count => format!("{count} files"),
    };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(format!("{files} ·"))
        .child(counts(added, removed))
        .child(diff.since.said())
}

/// `+added −removed`, in the colors of what they mean, leaving out a
/// count of none.
pub(crate) fn counts(added: usize, removed: usize) -> impl IntoElement {
    div()
        .flex()
        .gap(px(8.))
        .font_family(CODE_FONT)
        .text_size(px(12.))
        .when(added > 0 || removed == 0, |counts| {
            counts.child(div().text_color(rgb(ACCENT)).child(format!("+{added}")))
        })
        .when(removed > 0, |counts| {
            counts.child(div().text_color(rgb(FAILED)).child(format!("−{removed}")))
        })
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

/// How each line of each file of `diff` is drawn, by file, then line.
fn looks(diff: &Diff) -> Vec<Vec<Look>> {
    diff.files
        .iter()
        .map(|file| {
            let texts: Vec<String> = file.lines.iter().map(|line| drawn(&line.text)).collect();
            let lines: Vec<(Kind, &str)> = file
                .lines
                .iter()
                .zip(&texts)
                .map(|(line, text)| (line.kind, text.as_str()))
                .collect();
            file_colors(file)
                .into_iter()
                .zip(words::within(&lines))
                .map(|(syntax, changed)| Look { syntax, changed })
                .collect()
        })
        .collect()
}

/// What the syntax says of each line of `file`, for a file in a language
/// tree-sitter parses: the lines as they were, and those as they are,
/// each parsed as one text, so that a line is read with those around it.
fn file_colors(file: &File) -> Vec<Spans> {
    let none = || vec![Vec::new(); file.lines.len()];
    let Some(language) = Language::of_path(&file.path) else {
        return none();
    };
    // Each side's text, and where each line is in it: on the side it is
    // drawn from, as it is now for a line that is as it was.
    let (mut old, mut new) = (String::new(), String::new());
    let (mut old_count, mut new_count) = (0, 0);
    let mut sides = Vec::with_capacity(file.lines.len());
    for line in &file.lines {
        let text = drawn(&line.text);
        let add = |side: &mut String, count: &mut usize| {
            side.push_str(&text);
            side.push('\n');
            *count += 1;
            *count - 1
        };
        sides.push(match line.kind {
            Kind::Stretch => None,
            Kind::Removed => Some((false, add(&mut old, &mut old_count))),
            Kind::Added => Some((true, add(&mut new, &mut new_count))),
            Kind::Same => {
                add(&mut old, &mut old_count);
                Some((true, add(&mut new, &mut new_count)))
            }
        });
    }
    let parsed = |text: &str| {
        let buffer = Buffer::new(text);
        let syntax = Syntax::parse(language, &buffer, None);
        (buffer, syntax)
    };
    let (old, new) = (parsed(&old), parsed(&new));
    sides
        .into_iter()
        .map(|side| {
            let Some((is_new, at)) = side else {
                return Vec::new();
            };
            let (buffer, syntax) = if is_new { &new } else { &old };
            syntax
                .as_ref()
                .map(|syntax| syntax.highlights(buffer, at, usize::MAX))
                .unwrap_or_default()
        })
        .collect()
}

/// A line's text as the tab draws it, its tabs as spaces.
fn drawn(text: &str) -> String {
    text.replace('\t', "    ")
}

/// Reads what changed in the tab of `cx` at once, and again now and
/// then, for as long as the tab is, reading what the syntax says of it
/// when it changed.
fn follow(cx: &mut Context<Changes>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Ok(folder) = this.read_with(cx, |changes, _| changes.folder.clone()) else {
                break;
            };
            let read = cx
                .background_executor()
                .spawn(async move { read(&folder) })
                .await;
            let Ok(changed) = this.read_with(cx, |changes, _| changes.read.as_ref() != Some(&read))
            else {
                break;
            };
            if changed {
                let (read, looks) = cx
                    .background_executor()
                    .spawn(async move {
                        let looks = read
                            .iter()
                            .map(|shown| shown.read.as_ref().map(looks).unwrap_or_default())
                            .collect();
                        (read, looks)
                    })
                    .await;
                if this
                    .update(cx, |changes, cx| changes.looked(read, looks, cx))
                    .is_err()
                {
                    break;
                }
            }
            cx.background_executor().timer(LOOK_EVERY).await;
        }
    })
}

/// The head of the card of `file`, at `row` of the tab: how it changed,
/// where it is, and how many lines it gained and lost.
fn file_row(row: usize, file: &File) -> gpui::Stateful<gpui::Div> {
    let (dot, word) = match file.how {
        How::Added => (Some(ACCENT), Some("new".to_owned())),
        How::Removed => (Some(FAILED), Some("deleted".to_owned())),
        How::Moved => (
            None,
            file.was
                .as_ref()
                .map(|was| format!("moved from {}", was.display())),
        ),
        How::Changed => (None, None),
    };
    let path = file.path.display().to_string();
    let (folder, name) = match path.rfind('/') {
        Some(slash) => (path[..=slash].to_owned(), path[slash + 1..].to_owned()),
        None => (String::new(), path),
    };
    let counts = if file.binary {
        div()
            .text_size(px(12.))
            .text_color(rgb(FAINT))
            .child("binary")
            .into_any_element()
    } else {
        counts(file.added, file.removed).into_any_element()
    };
    let removed = file.how == How::Removed;
    div()
        .id(("changed-file", row))
        .debug_selector(move || format!("changed-file-{row}"))
        .h(px(34.))
        .flex()
        .items_center()
        .gap(px(10.))
        .px_3()
        .bg(rgb(RAISED))
        .rounded_t(px(CARD_RADIUS))
        .border_t_1()
        .whitespace_nowrap()
        .overflow_hidden()
        .when(!removed, |head| head.hover(|style| style.bg(rgb(SELECTED))))
        .child(
            div()
                .flex_shrink_0()
                .size(px(8.))
                .rounded_full()
                .map(|mark| match dot {
                    Some(color) => mark.bg(rgb(color)),
                    None => mark.border(px(1.5)).border_color(rgb(DIM)),
                }),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .font_family(CODE_FONT)
                .text_size(px(12.5))
                .child(div().flex_shrink_0().text_color(rgb(FAINT)).child(folder))
                .child(div().min_w_0().overflow_hidden().child(ellipsis(name))),
        )
        .children(word.map(|word| {
            div()
                .min_w_0()
                .overflow_hidden()
                .text_size(px(11.5))
                .text_color(rgb(FAINT))
                .child(ellipsis(word))
        }))
        // A space that grows, rather than an auto margin, which the rows'
        // gaps then go missing beside.
        .child(div().flex_1())
        .child(div().flex_shrink_0().child(counts))
}

/// The row of `line`, at `row` of the tab, the first of its file's if
/// `first` and the last if `last`: its numbers, as it was and as it is,
/// and its text, drawn as `look` says, on the ground its kind has.
fn line_row(
    row: usize,
    line: &Line,
    first: bool,
    last: bool,
    look: &Look,
) -> gpui::Stateful<gpui::Div> {
    let element = div()
        .id(("changed-line", row))
        .debug_selector(move || format!("changed-line-{row}"))
        .flex()
        .items_center()
        .whitespace_nowrap()
        .overflow_hidden()
        .font_family(CODE_FONT)
        .text_size(px(CODE_SIZE))
        .line_height(px(LINE_HEIGHT));
    if line.kind == Kind::Stretch {
        return stretch_row(element, line, first);
    }
    let number = |number: Option<usize>| match number {
        Some(number) => format!("{number:>NUMBER$}"),
        None => " ".repeat(NUMBER),
    };
    let (sign, mark, ground, word) = match line.kind {
        Kind::Added => ("+", Some(ACCENT), Some(ADDED_GROUND), ADDED_WORD),
        Kind::Removed => ("−", Some(FAILED_MARK), Some(REMOVED_GROUND), REMOVED_WORD),
        Kind::Same | Kind::Stretch => (" ", None, None, ADDED_WORD),
    };
    let text = SharedString::from(drawn(&line.text));
    let highlights = styled(text.len(), look, word);
    element
        .h(px(LINE_HEIGHT))
        .map(|row| match ground {
            Some(ground) => row.bg(rgb(ground)),
            None => row.hover(|style| style.bg(rgb(HOVER))),
        })
        .child(
            div().flex_shrink_0().w(px(2.)).h_full().child(
                div()
                    .w_full()
                    .h_full()
                    // Short of the card's round corner.
                    .when(last, |bar| bar.h(px(LINE_HEIGHT - 6.)))
                    .when_some(mark, |bar, mark| bar.bg(rgb(mark))),
            ),
        )
        .child(
            div()
                .flex_shrink_0()
                .pl_1()
                .text_color(rgb(if ground.is_some() { FAINT } else { DISABLED }))
                .child(format!("{} {}", number(line.old), number(line.new))),
        )
        .child(
            div()
                .flex_shrink_0()
                .w(px(24.))
                .flex()
                .justify_center()
                .text_color(rgb(mark.map_or(DISABLED, |_| match line.kind {
                    Kind::Added => ACCENT,
                    _ => FAILED,
                })))
                .child(sign),
        )
        .child(
            div()
                .min_w_0()
                .pr_3()
                .text_color(rgb(FOREGROUND))
                .child(StyledText::new(text).with_highlights(highlights)),
        )
}

/// How the stretches of a line `len` bytes long are drawn, as `look`
/// says: in the colors of its syntax, and the words it changed on the
/// ground `word`. The stretches are in order, and none overlaps another,
/// as GPUI takes them.
fn styled(len: usize, look: &Look, word: u32) -> Vec<(Range<usize>, HighlightStyle)> {
    let syntax = look.syntax.iter().filter(|(range, _)| range.end <= len);
    let changed = look.changed.iter().filter(|range| range.end <= len);
    // Where a stretch can start or end: where either says one does.
    let mut cuts: Vec<usize> = syntax
        .clone()
        .flat_map(|(range, _)| [range.start, range.end])
        .chain(changed.clone().flat_map(|range| [range.start, range.end]))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|cut| {
            let stretch = cut[0]..cut[1];
            let within =
                |range: &Range<usize>| range.start <= stretch.start && stretch.end <= range.end;
            let highlight = syntax
                .clone()
                .find(|(range, _)| within(range))
                .map(|(_, h)| *h);
            let changed = changed.clone().any(within);
            if highlight.is_none() && !changed {
                return None;
            }
            let mut style = HighlightStyle::default();
            if let Some(highlight) = highlight {
                let (color, weight, font_style) = syntax_style(highlight);
                style.color = Some(rgb(color).into());
                style.font_weight = Some(weight);
                style.font_style = Some(font_style);
            }
            if changed {
                style.background_color = Some(rgb(word).into());
            }
            Some((stretch, style))
        })
        .collect()
}

/// The row where a stretch of `line`'s file starts, the first of its
/// file's if `first`: what Git says the stretch is within, as the function
/// it is in, and, but for a stretch at the top of the file, a mark that
/// lines are left out above it.
fn stretch_row(
    element: gpui::Stateful<gpui::Div>,
    line: &Line,
    first: bool,
) -> gpui::Stateful<gpui::Div> {
    let within = line
        .text
        .strip_prefix("@@ ")
        .and_then(|rest| rest.split_once(" @@"))
        .map(|(_, within)| drawn(within.trim()))
        .unwrap_or_default();
    let left_out = !(first && line.at <= 1);
    if !left_out && within.is_empty() {
        // Nothing above it, and nothing it is within: a little room under
        // the file's head, as the card's padding.
        return element.h(px(6.));
    }
    element
        .h(px(STRETCH_HEIGHT))
        .text_size(px(12.))
        .text_color(rgb(FAINT))
        .hover(|style| style.bg(rgb(HOVER)))
        .child(div().flex_shrink_0().w(px(2.)))
        .child(
            div()
                .flex_shrink_0()
                .pl_1()
                .text_color(rgb(DISABLED))
                .child(format!(
                    "{:^width$}",
                    if left_out { "⋯" } else { "" },
                    width = NUMBER * 2 + 1
                )),
        )
        .child(div().flex_shrink_0().w(px(24.)))
        .child(div().min_w_0().overflow_hidden().child(ellipsis(within)))
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
        let list = list(
            self.list.clone(),
            cx.processor(|changes, row, _, cx| changes.render_row(row, cx)),
        )
        .flex_1()
        .min_h_0();
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .text_color(rgb(FOREGROUND))
            .font_family(UI_FONT)
            .text_size(px(13.))
            .child(self.render_heading())
            .children(self.render_repositories(cx))
            .child(list.pb_3())
    }
}
