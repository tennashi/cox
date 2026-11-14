//! The files of the chat's project as a tree, beside the tab of a file,
//! for the human to find another and read it there: a click on a folder
//! opens or closes it, and a click on a file shows it on the tab shown,
//! or on a tab of its own with Command held, or Control off macOS (see
//! [`crate::stage`]). What Git ignores is left out (see
//! [`cox_editor::files`]).
//!
//! The tree marks the file of the tab shown, with the folders it is in
//! open, and scrolls to it. A folder whose only file is a folder is one
//! row with it, as `docs / design`, and opens with it. Lines down the
//! tree show which folder a row is in, the line of the folder of the file
//! marked brighter, and the folders the rows at the top are in stay at
//! the top as the tree scrolls. A file that changed, as the Changes tab
//! tells it, says how on its row, as `M` or `A`, and a folder with one in
//! it says so with a dot; a file a chat's agent wrote there has a dot
//! of the chat's color besides, as long as it differs from the last
//! commit. It looks now and then at the folders open in
//! it, and at what changed, so that it shows the files the agent writes,
//! and no longer those it removes. The human folds it out of the way, beside every tab of a file
//! at once, for as long as Cox runs.
//!
//! [`Unchosen`] is the tab of a file not chosen yet, which the human opens
//! to choose one from the tree.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_editor::files::{self, Listed};
use cox_git::changes::How;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, EventEmitter, FocusHandle, Focusable, FontWeight,
    Global, ScrollHandle, Stateful, Task, Window, div, prelude::*, px, rgb,
};

use crate::stage::tab::Shows;
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::tip;
use crate::vim::hints::Hinted as _;
use crate::vim::{Close, Move, Open};
use coxd::theme::{
    ACCENT, ACCENT_GROUND, BORDER, BUTTON_HOVER, DIM, DISABLED, FAILED, FAINT, FOREGROUND, HOVER,
    SURFACE, chat_color,
};

pub(crate) const KEY_CONTEXT: &str = "Tree";

/// How often the tab looks at the folders open in it.
const LOOK_EVERY: Duration = Duration::from_secs(1);

/// How far in a folder's files and folders are from it.
const INDENT: f32 = 16.;
/// How tall a row is.
const ROW_HEIGHT: f32 = 26.;
/// How far the first row is from the top of the tree.
const TOP: f32 = 4.;
/// How wide the tree is beside a tab.
pub const WIDTH: f32 = 260.;

/// The human chose the file at `path`, to read it: on the tab shown, or
/// on a tab of its own when `apart`.
pub struct Chosen {
    pub path: PathBuf,
    pub apart: bool,
}

/// What tells the tree which files the agent of each chat of the
/// project wrote, by the chat's key, by the paths the agent named them
/// by (see [`cox_core::entry::written`]).
pub type Wrote = Rc<dyn Fn(&App) -> Vec<(u64, Vec<String>)>>;

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
        .on_hint_click(move |_, _, cx| fold(folded, cx))
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
    /// How each file that changed changed, by its absolute path, as the
    /// tab last looked.
    hows: HashMap<PathBuf, How>,
    /// The folders with a file that changed in them, or in a folder in
    /// them.
    changed: HashSet<PathBuf>,
    /// What tells which files the chats' agents wrote, once the stage
    /// said.
    wrote: Option<Wrote>,
    /// The chats whose agents wrote each file that changed, by their keys,
    /// in the order they first did.
    writers: HashMap<PathBuf, Vec<u64>>,
    /// Each path an agent named a file by, as the tree names it, through
    /// no link.
    canonical: HashMap<String, PathBuf>,
    /// The place of the row Vim's keys selected among the rows, once
    /// they moved it.
    picked: Option<usize>,
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
            hows: HashMap::new(),
            changed: HashSet::new(),
            wrote: None,
            writers: HashMap::new(),
            canonical: HashMap::new(),
            picked: None,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _follow: follow(cx),
        }
    }

    /// From now on, `wrote` tells the tree which files the chats' agents
    /// wrote, as it next looks. Not now: the stage is told while its
    /// window, which `wrote` reads, is busy making it.
    pub fn set_wrote(&mut self, wrote: Wrote) {
        self.wrote = Some(wrote);
    }

    /// The chats whose agents wrote the file at `path`, by their keys, as
    /// the tree marks it: none once it is as at the last commit.
    #[must_use]
    pub fn writers(&self, path: &Path) -> &[u64] {
        self.writers.get(path).map_or(&[], Vec::as_slice)
    }

    /// The chats whose agents wrote each file that differs from the last
    /// commit, by its absolute path.
    fn writers_now(&mut self, cx: &App) -> HashMap<PathBuf, Vec<u64>> {
        let Some(wrote) = self.wrote.clone() else {
            return HashMap::new();
        };
        let mut writers: HashMap<PathBuf, Vec<u64>> = HashMap::new();
        for (chat, paths) in wrote(cx) {
            for named in paths {
                let path = self
                    .canonical
                    .entry(named)
                    .or_insert_with_key(|named| {
                        let path = self.root.join(named);
                        std::fs::canonicalize(&path).unwrap_or(path)
                    })
                    .clone();
                if self.hows.contains_key(&path) {
                    let chats = writers.entry(path).or_default();
                    if !chats.contains(&chat) {
                        chats.push(chat);
                    }
                }
            }
        }
        writers
    }

    /// How the file at `path` changed, as the tree says on its row.
    #[must_use]
    pub fn how(&self, path: &Path) -> Option<How> {
        self.hows.get(path).copied()
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
            let place = self
                .all_rows()
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

    /// Opens the folder at `path`, or closes it if it is open. It opens
    /// with the folder in it, and the folder in that, as long as each is
    /// the only file of the one it is in.
    fn toggle(&mut self, path: &Path, cx: &mut Context<Self>) {
        if !self.open.remove(path) {
            let mut next = Some(path.to_owned());
            while let Some(folder) = next.take() {
                self.open_folder(folder.clone());
                if let Some(Ok(inner)) = self.listed.get(&folder)
                    && let [only] = inner.as_slice()
                    && only.is_folder
                {
                    next = Some(only.path.clone());
                }
            }
        }
        cx.notify();
    }

    /// The tab looked at its folders again, and at what changed in them,
    /// and found what is in them now, and how each file that changed
    /// changed: draws them again if that changed.
    fn looked(
        &mut self,
        listed: HashMap<PathBuf, Result<Vec<Listed>, String>>,
        hows: HashMap<PathBuf, How>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        if self.hows != hows {
            self.changed = hows
                .keys()
                .flat_map(|path| {
                    path.ancestors()
                        .skip(1)
                        .take_while(|folder| folder.starts_with(&self.root))
                })
                .map(Path::to_owned)
                .collect();
            self.hows = hows;
            changed = true;
        }
        let writers = self.writers_now(cx);
        if self.writers != writers {
            self.writers = writers;
            changed = true;
        }
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
        self.all_rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Listed { entry, .. } => Some(entry.path.clone()),
                Row::Unread { .. } => None,
            })
            .collect()
    }

    /// The rows of the tree, in order. Of a folder whose only file is a
    /// folder, one row stands for both; [`Self::shown`] gives its path as
    /// the inner one's.
    fn all_rows(&self) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        self.rows(&self.root, 0, None, &mut rows);
        rows
    }

    /// Adds the rows of what is in `folder`, `depth` folders under the
    /// project's, and of the folders open in it, to `rows`, with the
    /// line of the folder `depth` folders under the project's brighter,
    /// as the folder the file marked is in.
    fn rows<'a>(&'a self, folder: &Path, depth: u16, bright: Option<u16>, rows: &mut Vec<Row<'a>>) {
        match self.listed.get(folder) {
            Some(Ok(entries)) => {
                for entry in entries {
                    let (entry, name) = self.chain(entry);
                    let open = entry.is_folder && self.open.contains(&entry.path);
                    rows.push(Row::Listed {
                        entry,
                        name,
                        depth,
                        open,
                        bright,
                    });
                    if open {
                        let marked = self.current.as_deref().and_then(Path::parent)
                            == Some(entry.path.as_path());
                        let bright = if marked { Some(depth) } else { bright };
                        self.rows(&entry.path, depth + 1, bright, rows);
                    }
                }
            }
            Some(Err(error)) => rows.push(Row::Unread {
                error,
                depth,
                bright,
            }),
            None => {}
        }
    }

    /// The folder `entry` is a row for, as the last of the folders open
    /// one in another from it, each the only file of the one before, with
    /// the name of the row, the names of them all; or `entry`, and its
    /// name.
    fn chain<'a>(&'a self, mut entry: &'a Listed) -> (&'a Listed, String) {
        let mut name = entry.name.clone();
        while entry.is_folder
            && self.open.contains(&entry.path)
            && let Some(Ok(inner)) = self.listed.get(&entry.path)
            && let [only] = inner.as_slice()
            && only.is_folder
        {
            name.push_str(" / ");
            name.push_str(&only.name);
            entry = only;
        }
        (entry, name)
    }

    /// The place of the row Vim's keys select among `rows`, not empty:
    /// the one they moved to, or else the file marked, or else the first.
    fn picked_place(&self, rows: &[Row<'_>]) -> usize {
        let marked = || {
            rows.iter().position(|row| {
                matches!(row, Row::Listed { entry, .. } if Some(&entry.path) == self.current.as_ref())
            })
        };
        self.picked.or_else(marked).unwrap_or(0).min(rows.len() - 1)
    }

    /// Selects the row at `place`, and scrolls it into sight.
    fn pick(&mut self, place: usize, cx: &mut Context<Self>) {
        self.picked = Some(place);
        self.scroll.scroll_to_item(place);
        cx.notify();
    }

    /// Moves the selection as Vim's keys ask.
    fn move_by_keys(&mut self, motion: &Move, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.all_rows();
        if rows.is_empty() {
            return;
        }
        let (at, last) = (self.picked_place(&rows), rows.len() - 1);
        drop(rows);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a count of rows in sight, not below 0"
        )]
        let half = (f32::from(self.scroll.bounds().size.height) / ROW_HEIGHT / 2.) as usize;
        let place = motion.moved(at, half, last);
        self.pick(place, cx);
    }

    /// Opens the folder selected, or goes into it if it is open, or
    /// chooses the file selected.
    fn open_picked(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.all_rows();
        if rows.is_empty() {
            return;
        }
        let place = self.picked_place(&rows);
        let Row::Listed { entry, open, .. } = &rows[place] else {
            return;
        };
        let (path, is_folder, open) = (entry.path.clone(), entry.is_folder, *open);
        drop(rows);
        if !is_folder {
            self.picked = Some(place);
            cx.emit(Chosen { path, apart: false });
        } else if open {
            let last = self.all_rows().len() - 1;
            self.pick((place + 1).min(last), cx);
        } else {
            self.picked = Some(place);
            self.toggle(&path, cx);
        }
    }

    /// Closes the folder selected, if open, or else selects the folder
    /// it is in.
    fn close_picked(&mut self, _: &Close, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.all_rows();
        if rows.is_empty() {
            return;
        }
        let place = self.picked_place(&rows);
        let open = match &rows[place] {
            Row::Listed {
                entry, open: true, ..
            } => Some(entry.path.clone()),
            _ => None,
        };
        let folder = folders_of(&rows, place).last().copied();
        drop(rows);
        if let Some(path) = open {
            self.picked = Some(place);
            self.toggle(&path, cx);
        } else if let Some(folder) = folder {
            self.pick(folder, cx);
        }
    }

    /// Scrolls so that the row at `place` is at the top, below the
    /// folders it is in, which stay there.
    fn scroll_to_folder(&self, place: usize, above: usize, cx: &mut Context<Self>) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "far fewer rows than f32 counts exactly"
        )]
        let rows = (place - above) as f32;
        self.scroll
            .set_offset(gpui::point(px(0.), -px(rows * ROW_HEIGHT)));
        cx.notify();
    }

    /// The row `row` of the tree, which a test finds as `entry-{number}`;
    /// or, as one of the folders the rows at the top are in, as
    /// `pinned-{number}`.
    fn render_row(
        &self,
        row: &Row<'_>,
        at: At,
        picked: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (entry, name, depth, open) = match row {
            Row::Listed {
                entry,
                name,
                depth,
                open,
                ..
            } => (*entry, name, *depth, *open),
            Row::Unread { error, depth, .. } => {
                return div()
                    .relative()
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .pl(indent(*depth) + px(16.))
                    .text_color(rgb(FAILED))
                    .children(guides(*depth, row.bright()))
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
        let (id, number) = match at {
            At::Listed(number) | At::Pinned { number, .. } => (at.name(), number),
        };
        let row = div()
            .id((id, number))
            .debug_selector(move || format!("{id}-{number}"))
            .relative()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(ROW_HEIGHT))
            .pl(indent(depth))
            .pr_2()
            .rounded(px(6.))
            .cursor_pointer()
            .children(guides(depth, row.bright()))
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
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(ellipsis(name.clone())),
            )
            .children(self.writer_marks(entry))
            .children(self.change_mark(entry))
            .on_hint_click(cx.listener(move |tree, click: &ClickEvent, _, cx| {
                if let At::Pinned { place, above, .. } = at {
                    tree.scroll_to_folder(place, above, cx);
                } else if is_folder {
                    tree.toggle(&path, cx);
                } else {
                    cx.emit(Chosen {
                        path: path.clone(),
                        apart: click.modifiers().secondary(),
                    });
                }
            }));
        if matches!(at, At::Listed(number) if picked == Some(number - 1)) {
            row.bg(rgb(BUTTON_HOVER)).into_any_element()
        } else if self.current.as_ref() == Some(&entry.path) {
            row.bg(rgb(ACCENT_GROUND)).into_any_element()
        } else {
            row.hover(|style| style.bg(rgb(HOVER))).into_any_element()
        }
    }
}

impl Tree {
    /// The dots of the colors of the chats whose agents wrote `entry`, if
    /// it differs from the last commit.
    fn writer_marks(&self, entry: &Listed) -> Option<AnyElement> {
        let chats = self.writers.get(&entry.path)?;
        Some(
            div()
                .flex_shrink_0()
                .flex()
                .gap(px(2.))
                .children(
                    chats
                        .iter()
                        .map(|&chat| div().size(px(6.)).rounded_full().bg(rgb(chat_color(chat)))),
                )
                .into_any_element(),
        )
    }

    /// What says on the row of `entry` how it changed: the letter of a
    /// file that changed, in the accent if it is new, as the Changes tab
    /// marks it; or a dot, for a folder with a file that changed in it.
    fn change_mark(&self, entry: &Listed) -> Option<AnyElement> {
        if entry.is_folder {
            return self.changed.contains(&entry.path).then(|| {
                div()
                    .flex_shrink_0()
                    .size(px(6.))
                    .mr(px(4.))
                    .rounded_full()
                    .bg(rgb(FAINT))
                    .into_any_element()
            });
        }
        let how = self.hows.get(&entry.path)?;
        Some(
            div()
                .flex_shrink_0()
                .w(px(14.))
                .text_size(px(11.))
                .text_color(rgb(if *how == How::Added { ACCENT } else { DIM }))
                .child(how.letter().to_string())
                .into_any_element(),
        )
    }
}

/// A row of the tab: a file or folder, `depth` folders under the
/// project's, drawn as `name`, open if it is a folder that is, or why a
/// folder open could not be listed. Of the lines down the tree beside it,
/// the one `bright` folders in is brighter.
enum Row<'a> {
    Listed {
        entry: &'a Listed,
        name: String,
        depth: u16,
        open: bool,
        bright: Option<u16>,
    },
    Unread {
        error: &'a str,
        depth: u16,
        bright: Option<u16>,
    },
}

impl Row<'_> {
    fn depth(&self) -> u16 {
        match *self {
            Self::Listed { depth, .. } | Self::Unread { depth, .. } => depth,
        }
    }

    fn bright(&self) -> Option<u16> {
        match *self {
            Self::Listed { bright, .. } | Self::Unread { bright, .. } => bright,
        }
    }
}

/// Where a row is drawn: among the rest, as the `number`th, or at the top
/// as the `number`th of the folders the rows there are in, `place` among
/// the rest, under `above` folders.
#[derive(Clone, Copy)]
enum At {
    Listed(usize),
    Pinned {
        number: usize,
        place: usize,
        above: usize,
    },
}

impl At {
    fn name(self) -> &'static str {
        match self {
            Self::Listed(_) => "entry",
            Self::Pinned { .. } => "pinned",
        }
    }
}

/// The lines down the tree beside a row `depth` folders under the
/// project's, one for each folder it is in, the one `bright` folders in
/// brighter.
fn guides(depth: u16, bright: Option<u16>) -> impl Iterator<Item = AnyElement> {
    (0..depth).map(move |column| {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(indent(column) + px(4.5))
            .w(px(1.))
            .bg(rgb(if bright == Some(column) {
                DISABLED
            } else {
                BORDER
            }))
            .into_any_element()
    })
}

/// The places of the rows of the folders the row at `place` is in, from
/// the project's down.
fn folders_of(rows: &[Row<'_>], place: usize) -> Vec<usize> {
    let mut depth = rows[place].depth();
    let mut folders = Vec::new();
    for at in (0..place).rev() {
        if depth == 0 {
            break;
        }
        if rows[at].depth() == depth - 1 {
            folders.push(at);
            depth -= 1;
        }
    }
    folders.reverse();
    folders
}

/// The places of the rows of the folders to keep at the top of the tree
/// when the row at `top` is the first there: the folders of the first row
/// they leave to be seen.
fn pinned(rows: &[Row<'_>], top: usize) -> Vec<usize> {
    let mut count = 0;
    loop {
        if top + count >= rows.len() {
            return Vec::new();
        }
        let folders = folders_of(rows, top + count);
        if folders.len() <= count {
            return folders;
        }
        count = folders.len();
    }
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

/// Looks at the folders of the tab of `cx`, and at what changed in the
/// project, now and then, from the start, for as long as the tab is.
fn follow(cx: &mut Context<Tree>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Ok((root, folders)) =
                this.read_with(cx, |tree, _| (tree.root.clone(), tree.folders()))
            else {
                break;
            };
            let (listed, hows) = cx
                .background_executor()
                .spawn(async move {
                    let listed = folders
                        .into_iter()
                        .map(|folder| {
                            let listed = list(&folder);
                            (folder, listed)
                        })
                        .collect();
                    (listed, coxd::repositories::hows(&root))
                })
                .await;
            if this
                .update(cx, |tree, cx| tree.looked(listed, hows, cx))
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(LOOK_EVERY).await;
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.all_rows();
        let picked = self.picked.filter(|_| self.focus_handle.is_focused(window));
        let scrolled = -f32::from(self.scroll.offset().y);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a count of rows, not below 0"
        )]
        let top = (scrolled.max(0.) / ROW_HEIGHT).floor() as usize;
        let pinned: Vec<_> = pinned(&rows, top)
            .into_iter()
            .enumerate()
            .map(|(index, place)| {
                let at = At::Pinned {
                    number: index + 1,
                    place,
                    above: index,
                };
                self.render_row(&rows[place], at, None, cx)
            })
            .collect();
        let rows: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_row(row, At::Listed(index + 1), picked, cx))
            .collect();
        div()
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::move_by_keys))
            .on_action(cx.listener(Self::open_picked))
            .on_action(cx.listener(Self::close_picked))
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
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("tree")
                            .debug_selector(|| "tree".to_owned())
                            .size_full()
                            .px(px(8.))
                            .pt(px(TOP))
                            .pb_3()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .children(rows),
                    )
                    .when(!pinned.is_empty(), |tree| {
                        tree.child(
                            div()
                                .debug_selector(|| "pinned".to_owned())
                                // Not the rows below, under it.
                                .occlude()
                                .child(crate::vim::hints::cover())
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .px(px(8.))
                                .pt(px(TOP))
                                .bg(rgb(SURFACE))
                                .border_b_1()
                                .border_color(rgb(BORDER))
                                .children(pinned),
                        )
                    }),
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
            .key_context(KEY_CONTEXT)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_tree_is_unfolded_until_the_human_folds_it(cx: &mut gpui::TestAppContext) {
        assert!(!cx.update(|cx| is_folded(cx)));
    }

    #[gpui::test]
    fn the_tree_folded_stays_folded(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| fold(true, cx));

        assert!(cx.update(|cx| is_folded(cx)));
    }

    #[gpui::test]
    fn the_tree_unfolded_again_is_unfolded(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| fold(true, cx));

        cx.update(|cx| fold(false, cx));

        assert!(!cx.update(|cx| is_folded(cx)));
    }

    /// Rows at the depths `depths`, one after another.
    fn rows(depths: &[u16]) -> Vec<Row<'static>> {
        depths
            .iter()
            .map(|&depth| Row::Unread {
                error: "",
                depth,
                bright: None,
            })
            .collect()
    }

    #[test]
    fn a_row_is_in_the_folders_above_it_one_level_up_each() {
        let rows = rows(&[0, 1, 2, 2, 1, 2, 3]);

        assert_eq!(folders_of(&rows, 6), [0, 4, 5]);
    }

    #[test]
    fn a_row_at_the_top_level_is_in_no_folder() {
        let rows = rows(&[0, 1, 0]);

        assert_eq!(folders_of(&rows, 2), Vec::<usize>::new());
    }
}
