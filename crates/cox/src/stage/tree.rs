//! The files of the chat's project as a tree, beside the tab of a file,
//! for the human to find another and read it there: a click on a folder
//! opens or closes it, and a click on a file shows it on the tab shown,
//! or on a tab of its own with Command held, or Control off macOS (see
//! [`crate::stage`]). What Git ignores is left out (see
//! [`cox_editor::files`]).
//!
//! The tree marks the file of the tab shown, with the folders it is in
//! open, and scrolls to it. It looks now and then at the folders open in
//! it, so that it shows the files the agent writes, and no longer those it
//! removes. The human folds it out of the way, beside every tab of a file
//! at once, for as long as Cox runs.
//!
//! [`Unchosen`] is the tab of a file not chosen yet, which the human opens
//! to choose one from the tree.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_editor::files::{self, Listed};
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, EventEmitter, FocusHandle, Focusable, FontWeight,
    Global, ScrollHandle, Stateful, Task, Window, div, prelude::*, px, rgb,
};

use crate::stage::tab::Shows;
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::theme::{
    ACCENT_GROUND, BORDER, BUTTON_HOVER, DIM, FAILED, FAINT, FOREGROUND, HOVER,
};
use crate::ui::tip;

/// How often the tab looks at the folders open in it.
const LOOK_EVERY: Duration = Duration::from_secs(1);

/// How far in a folder's files and folders are from it.
const INDENT: f32 = 16.;
/// How tall a row is.
const ROW_HEIGHT: f32 = 26.;
/// How wide the tree is beside a tab.
pub const WIDTH: f32 = 260.;

/// The human chose the file at `path`, to read it: on the tab shown, or
/// on a tab of its own when `apart`.
pub struct Chosen {
    pub path: PathBuf,
    pub apart: bool,
}

/// Whether the tree is folded, beside every tab of a file of every stage,
/// for as long as Cox runs.
#[derive(Default)]
struct Folded(bool);

impl Global for Folded {}

/// Whether the tree is folded out of the way.
#[must_use]
pub fn is_folded(cx: &App) -> bool {
    cx.try_global::<Folded>().is_some_and(|folded| folded.0)
}

/// Folds the tree out of the way, or unfolds it, beside every tab.
pub fn fold(folded: bool, cx: &mut App) {
    cx.set_global(Folded(folded));
    cx.refresh_windows();
}

/// The button that unfolds the tree, for the bar of a tab of a file to
/// show while it is folded, which a test finds as `selector`.
#[must_use]
pub fn unfold_button(selector: &'static str) -> Stateful<Div> {
    fold_button(selector, false)
}

/// The button that folds the tree, or unfolds it when `folded` is false.
fn fold_button(selector: &'static str, folded: bool) -> Stateful<Div> {
    div()
        .id(selector)
        .debug_selector(move || selector.to_owned())
        .flex_shrink_0()
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(BUTTON_HOVER)))
        .tooltip(tip(if folded {
            "Fold the files"
        } else {
            "Show the files"
        }))
        .child(icon(Icon::Sidebar, 14., DIM))
        .on_click(move |_, _, cx| fold(folded, cx))
}

pub struct Tree {
    /// The project's folder, which is always open.
    root: PathBuf,
    /// The folders open in it besides, by their absolute paths.
    open: BTreeSet<PathBuf>,
    /// What is in each folder open, as the tab last listed it, or why it
    /// could not be listed.
    listed: HashMap<PathBuf, Result<Vec<Listed>, String>>,
    /// The file of the tab shown, which the tree marks.
    current: Option<PathBuf>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    _follow: Task<()>,
}

impl EventEmitter<Chosen> for Tree {}

impl Tree {
    /// A tab showing the files under `root`, an absolute path, with the
    /// folders `open` open.
    pub fn new(root: PathBuf, open: Vec<PathBuf>, cx: &mut Context<Self>) -> Self {
        // As the tabs name their files, so that the tree finds them in it.
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        let open: BTreeSet<PathBuf> = open.into_iter().filter(|path| *path != root).collect();
        let listed = std::iter::once(&root)
            .chain(&open)
            .map(|folder| (folder.clone(), list(folder)))
            .collect();
        Self {
            root,
            open,
            listed,
            current: None,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _follow: follow(cx),
        }
    }

    /// The folders open, besides the project's.
    #[must_use]
    pub fn open(&self) -> Vec<PathBuf> {
        self.open.iter().cloned().collect()
    }

    /// The folders whose files the tab shows: the project's, and the ones
    /// open.
    fn folders(&self) -> Vec<PathBuf> {
        std::iter::once(&self.root)
            .chain(&self.open)
            .cloned()
            .collect()
    }

    /// Opens the folders at `open` besides those open already.
    pub fn open_too(&mut self, open: Vec<PathBuf>, cx: &mut Context<Self>) {
        for folder in open {
            self.open_folder(folder);
        }
        cx.notify();
    }

    /// The file the tree marks, as the file of the tab shown.
    #[must_use]
    pub fn current(&self) -> Option<&Path> {
        self.current.as_deref()
    }

    /// Marks the file at `path`, as the file of the tab shown, with the
    /// folders it is in open, and scrolls to it; or no file.
    pub fn reveal(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.current == path {
            return;
        }
        self.current = path;
        if let Some(path) = self.current.clone()
            && let Ok(under) = path.strip_prefix(&self.root)
        {
            let mut folder = self.root.clone();
            for part in under.parent().into_iter().flat_map(Path::components) {
                folder.push(part);
                self.open_folder(folder.clone());
            }
            let mut rows = Vec::new();
            self.rows(&self.root, 0, &mut rows);
            let place = rows
                .iter()
                .position(|row| matches!(row, Row::Listed { entry, .. } if entry.path == path));
            if let Some(place) = place {
                self.scroll.scroll_to_item(place);
            }
        }
        cx.notify();
    }

    /// Opens the folder at `folder`, listing what is in it, unless it is
    /// the project's or open already.
    fn open_folder(&mut self, folder: PathBuf) {
        if folder != self.root && !self.open.contains(&folder) {
            self.listed.insert(folder.clone(), list(&folder));
            self.open.insert(folder);
        }
    }

    /// Opens the folder at `path`, or closes it if it is open.
    fn toggle(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.open.remove(path) {
            self.open_folder(path.to_owned());
        }
        cx.notify();
    }

    /// The tab looked at its folders again, and found what is in them now:
    /// draws them again if that changed.
    fn looked(
        &mut self,
        listed: HashMap<PathBuf, Result<Vec<Listed>, String>>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        for (folder, now) in listed {
            if self.listed.get(&folder) != Some(&now) {
                self.listed.insert(folder, now);
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// The paths of the files and folders the tab shows, in order: what
    /// is in the project's folder, and in the folders open in it.
    #[must_use]
    pub fn shown(&self) -> Vec<PathBuf> {
        let mut rows = Vec::new();
        self.rows(&self.root, 0, &mut rows);
        rows.into_iter()
            .filter_map(|row| match row {
                Row::Listed { entry, .. } => Some(entry.path.clone()),
                Row::Unread { .. } => None,
            })
            .collect()
    }

    /// Adds the rows of what is in `folder`, `depth` folders under the
    /// project's, and of the folders open in it, to `rows`.
    fn rows<'a>(&'a self, folder: &Path, depth: u16, rows: &mut Vec<Row<'a>>) {
        match self.listed.get(folder) {
            Some(Ok(entries)) => {
                for entry in entries {
                    let open = entry.is_folder && self.open.contains(&entry.path);
                    rows.push(Row::Listed { entry, depth, open });
                    if open {
                        self.rows(&entry.path, depth + 1, rows);
                    }
                }
            }
            Some(Err(error)) => rows.push(Row::Unread { error, depth }),
            None => {}
        }
    }

    fn render_row(&self, number: usize, row: &Row<'_>, cx: &mut Context<Self>) -> AnyElement {
        let (entry, depth, open) = match *row {
            Row::Listed { entry, depth, open } => (entry, depth, open),
            Row::Unread { error, depth } => {
                return div()
                    .pl(indent(depth) + px(16.))
                    .py_1()
                    .text_color(rgb(FAILED))
                    .child(format!("cannot read: {error}"))
                    .into_any_element();
            }
        };
        let marker = match (entry.is_folder, open) {
            (true, true) => "▾",
            (true, false) => "▸",
            (false, _) => "",
        };
        let kind = if entry.is_folder {
            Icon::Folder
        } else if is_markdown(&entry.path) {
            Icon::Document
        } else {
            Icon::Code
        };
        let path = entry.path.clone();
        let is_folder = entry.is_folder;
        let row = div()
            .id(("entry", number))
            .debug_selector(move || format!("entry-{number}"))
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(ROW_HEIGHT))
            .pl(indent(depth))
            .pr_2()
            .rounded(px(6.))
            .cursor_pointer()
            .child(
                div()
                    .w(px(10.))
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(DIM))
                    .child(marker),
            )
            .child(icon(kind, 14., FAINT).flex_shrink_0())
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .child(ellipsis(entry.name.clone())),
            )
            .on_click(cx.listener(move |tree, click: &ClickEvent, _, cx| {
                if is_folder {
                    tree.toggle(&path, cx);
                } else {
                    cx.emit(Chosen {
                        path: path.clone(),
                        apart: click.modifiers().secondary(),
                    });
                }
            }));
        if self.current.as_ref() == Some(&entry.path) {
            row.bg(rgb(ACCENT_GROUND)).into_any_element()
        } else {
            row.hover(|style| style.bg(rgb(HOVER))).into_any_element()
        }
    }
}

/// A row of the tab: a file or folder, `depth` folders under the
/// project's, open if it is a folder that is, or why a folder open could
/// not be listed.
enum Row<'a> {
    Listed {
        entry: &'a Listed,
        depth: u16,
        open: bool,
    },
    Unread {
        error: &'a str,
        depth: u16,
    },
}

/// How far in a row `depth` folders under the project's is drawn.
fn indent(depth: u16) -> gpui::Pixels {
    px(4. + INDENT * f32::from(depth))
}

/// Whether the file at `path` is Markdown, which a tab draws as a page.
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// What is in `folder`, or why it could not be listed.
fn list(folder: &Path) -> Result<Vec<Listed>, String> {
    files::list(folder).map_err(|error| error.to_string())
}

/// Looks at the folders of the tab of `cx` now and then, for as long as
/// the tab is.
fn follow(cx: &mut Context<Tree>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(LOOK_EVERY).await;
            let Ok(folders) = this.read_with(cx, |tree, _| tree.folders()) else {
                break;
            };
            let listed = cx
                .background_executor()
                .spawn(async move {
                    folders
                        .into_iter()
                        .map(|folder| {
                            let listed = list(&folder);
                            (folder, listed)
                        })
                        .collect()
                })
                .await;
            if this.update(cx, |tree, cx| tree.looked(listed, cx)).is_err() {
                break;
            }
        }
    })
}

impl Tree {
    /// The project's folder's name.
    fn name(&self) -> String {
        self.root.file_name().map_or_else(
            || self.root.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}

impl Focusable for Tree {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Tree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        self.rows(&self.root, 0, &mut rows);
        let rows: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_row(index + 1, row, cx))
            .collect();
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .text_color(rgb(FOREGROUND))
            .child(
                div()
                    .debug_selector(|| "tree-heading".to_owned())
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl(px(14.))
                    .pr(px(8.))
                    .pt(px(8.))
                    .pb_1()
                    .whitespace_nowrap()
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Files"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_color(rgb(DIM))
                            .child(ellipsis(self.name())),
                    )
                    .child(fold_button("fold-files", true)),
            )
            .child(
                div()
                    .id("tree")
                    .debug_selector(|| "tree".to_owned())
                    .flex_1()
                    .min_h_0()
                    .px(px(8.))
                    .pt_1()
                    .pb_3()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
    }
}

/// The tab of a file not chosen yet, beside the tree, for the human to
/// choose one there.
pub struct Unchosen {
    focus_handle: FocusHandle,
}

impl Unchosen {
    #[must_use]
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Shows for Unchosen {
    fn title(&self) -> String {
        "Files".to_owned()
    }

    /// The folders open in the tree are not kept: the tree opens to the
    /// file of the tab shown.
    fn kept(&self) -> KeptTab {
        KeptTab::Files { open: Vec::new() }
    }
}

impl Focusable for Unchosen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Unchosen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let folded = is_folded(cx);
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_h(px(30.))
                    .px_3()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .text_color(rgb(DIM))
                    .when(folded, |bar| bar.child(unfold_button("unfold-files")))
                    .child("No file chosen"),
            )
            .child(
                div()
                    .debug_selector(|| "unchosen".to_owned())
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(FAINT))
                    .child(if folded {
                        "Show the files to choose one"
                    } else {
                        "Choose a file from the files"
                    }),
            )
    }
}
