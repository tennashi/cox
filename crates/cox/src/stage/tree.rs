//! A tab of the stage that shows the files of the chat's project as a
//! tree, for the human to find one and open it: a click on a folder opens
//! or closes it, and a click on a file opens it on a tab of its own, as a
//! link to it would (see [`crate::stage`]). What Git ignores is left out
//! (see [`cox_editor::files`]).
//!
//! The tab looks now and then at the folders open in it, so that it shows
//! the files the agent writes, and no longer those it removes.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_editor::files::{self, Listed};
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, ScrollHandle, Task, Window,
    div, prelude::*, px, rgb,
};

use crate::stage::heading::{INSET, body, heading};
use crate::stage::tab::Shows;
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::paths::shortened;
use crate::ui::theme::{ACCENT_GROUND, DIM, FAILED, FAINT, FOREGROUND, HOVER};

/// How often the tab looks at the folders open in it.
const LOOK_EVERY: Duration = Duration::from_secs(1);

/// How far in a folder's files and folders are from it.
const INDENT: f32 = 16.;
/// How tall a row is.
const ROW_HEIGHT: f32 = 26.;

/// The human chose the file at this path, to open it.
pub struct Chosen(pub PathBuf);

pub struct Tree {
    /// The project's folder, which is always open.
    root: PathBuf,
    /// The folders open in it besides, by their absolute paths.
    open: BTreeSet<PathBuf>,
    /// What is in each folder open, as the tab last listed it, or why it
    /// could not be listed.
    listed: HashMap<PathBuf, Result<Vec<Listed>, String>>,
    /// The file the human chose last, which the tab marks.
    chosen: Option<PathBuf>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    _follow: Task<()>,
}

impl EventEmitter<Chosen> for Tree {}

impl Tree {
    /// A tab showing the files under `root`, an absolute path, with the
    /// folders `open` open.
    pub fn new(root: PathBuf, open: Vec<PathBuf>, cx: &mut Context<Self>) -> Self {
        let open: BTreeSet<PathBuf> = open.into_iter().filter(|path| *path != root).collect();
        let listed = std::iter::once(&root)
            .chain(&open)
            .map(|folder| (folder.clone(), list(folder)))
            .collect();
        Self {
            root,
            open,
            listed,
            chosen: None,
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

    /// Opens the folder at `path`, or closes it if it is open.
    fn toggle(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.open.remove(path) {
            self.listed.insert(path.to_owned(), list(path));
            self.open.insert(path.to_owned());
        }
        cx.notify();
    }

    fn choose(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.chosen = Some(path.to_owned());
        cx.emit(Chosen(path.to_owned()));
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
            .on_click(cx.listener(move |tree, _, _, cx| {
                if is_folder {
                    tree.toggle(&path, cx);
                } else {
                    tree.choose(&path, cx);
                }
            }));
        if self.chosen.as_ref() == Some(&entry.path) {
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

impl Shows for Tree {
    /// The project's folder's name.
    fn title(&self) -> String {
        let name = self.root.file_name().map_or_else(
            || self.root.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        format!("{name}/")
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Files { open: self.open() }
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
        body()
            .track_focus(&self.focus_handle)
            .text_color(rgb(FOREGROUND))
            .child(heading(
                "tree-path",
                "Files",
                "without what Git ignores",
                Some(shortened(&self.root)),
            ))
            .child(
                div()
                    .id("tree")
                    .debug_selector(|| "tree".to_owned())
                    .flex_1()
                    .min_h_0()
                    .px(px(INSET))
                    .pt_1()
                    .pb_3()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows),
            )
    }
}
