//! A tab of the stage that shows the map of the chat's project: its crates,
//! and the modules in each, as boxes, each with the first paragraph of
//! what its file says of itself, for the human to see where things are
//! before they read them (see [`cox_editor::map`]). A click on a box with
//! modules in it opens the map at that box, the path above the boxes leads
//! back up, and a click on a file opens it on a tab of its own, as a link
//! to it would (see [`crate::stage`]). What changed in the project, as
//! the Changes tab shows it, is laid over the map: a box with a file that
//! changed in it is marked, with how many lines changed.
//!
//! The tab looks again now and then, so that it follows the project as
//! the agent changes it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_editor::changes::{self, Since};
use cox_editor::map::{self, Changed, Crate, Module};
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, ScrollHandle, Task,
    Window, div, prelude::*, px, rgb,
};

use crate::stage::tab::Shows;
use crate::ui::paths::shortened;
use crate::ui::theme::{BACKGROUND, DIM, FAILED, FOREGROUND, LINK};

/// How often the tab looks at the project again.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// How wide a box is.
const BOX_WIDTH: f32 = 236.;
/// How tall what a box says of itself is at most: three lines.
const SAID_HEIGHT: f32 = 60.;

const BORDER: u32 = 0x40_4040;
const HOVER: u32 = 0x2a_2d2e;
/// The border of a box with a file that changed in it, and the color its
/// counts are in.
const CHANGED: u32 = 0x8f_d19e;

/// The human chose the file at this path, to open it.
pub struct Chosen(pub PathBuf);

pub struct Map {
    /// The project's folder.
    root: PathBuf,
    /// Where the map is open: the crate, and the modules in it, from the
    /// crate down, by name; the crates when empty.
    at: Vec<String>,
    /// The map, as the tab last read it, or why it could not be read;
    /// nothing until it is first read.
    read: Option<Result<map::Map, String>>,
    /// What changed, when the project is in a Git repository.
    changes: Option<Overlay>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    _follow: Task<()>,
}

/// What changed in the project, laid over the map.
#[derive(PartialEq, Eq)]
struct Overlay {
    /// What the files changed since.
    since: Since,
    /// How many lines each file that changed gained and lost, by its
    /// path from the project's folder.
    files: HashMap<PathBuf, (usize, usize)>,
}

/// What a box of the map shows.
enum Shown<'a> {
    Crate(&'a Crate),
    /// A file: one a crate starts from, or that of the module the map is
    /// open at.
    File(&'a Module),
    /// A module in the crate or module the map is open at.
    Module(&'a Module),
}

/// Where the map is open, as far as the map has it, and what it shows
/// there.
struct Place<'a> {
    /// How many of the names of [`Map::at`] the map has, from the first.
    depth: usize,
    boxes: Vec<Shown<'a>>,
}

impl EventEmitter<Chosen> for Map {}

impl Map {
    /// A tab showing the map of the project in `root`, an absolute path,
    /// open `at` (see [`KeptTab::Map`]).
    pub fn new(root: PathBuf, at: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self {
            root,
            at,
            read: None,
            changes: None,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _follow: follow(cx),
        }
    }

    /// Where the map is open, by name, from the crate down.
    #[must_use]
    pub fn at(&self) -> &[String] {
        &self.at
    }

    /// The tab read the project again: draws it again if it changed.
    fn looked(
        &mut self,
        read: Result<map::Map, String>,
        changes: Option<Overlay>,
        cx: &mut Context<Self>,
    ) {
        let mut differs = false;
        if self.read.as_ref() != Some(&read) {
            self.read = Some(read);
            differs = true;
        }
        if self.changes != changes {
            self.changes = changes;
            differs = true;
        }
        if differs {
            cx.notify();
        }
    }

    /// Opens the map at `name`, in the crate or module it is open at.
    fn descend(&mut self, name: &str, cx: &mut Context<Self>) {
        let depth = self.place().map_or(0, |place| place.depth);
        self.at.truncate(depth);
        self.at.push(name.to_owned());
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    /// Opens the map `depth` names down from the crates: at the crates
    /// for 0.
    fn ascend(&mut self, depth: usize, cx: &mut Context<Self>) {
        self.at.truncate(depth);
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    /// Where the map is open, and what it shows there, once it is read.
    fn place(&self) -> Option<Place<'_>> {
        let map = self.read.as_ref()?.as_ref().ok()?;
        let Some(krate) = self
            .at
            .first()
            .and_then(|name| map.crates.iter().find(|krate| krate.name == *name))
        else {
            return Some(Place {
                depth: 0,
                boxes: map.crates.iter().map(Shown::Crate).collect(),
            });
        };
        let mut depth = 1;
        let mut files: Vec<&Module> = krate.entries.iter().collect();
        let mut modules: Vec<&Module> = krate.modules();
        for name in self.at.iter().skip(1) {
            let Some(module) = modules.iter().find(|module| module.name == *name) else {
                break;
            };
            files = Vec::from([*module]);
            modules = module.modules.iter().collect();
            depth += 1;
        }
        let boxes = files
            .into_iter()
            .map(Shown::File)
            .chain(modules.into_iter().map(Shown::Module))
            .collect();
        Some(Place { depth, boxes })
    }

    /// What changed in what `shown` shows, if the project is in a
    /// repository.
    fn changed_in(&self, shown: &Shown<'_>) -> Changed {
        let Some(Overlay { files, .. }) = &self.changes else {
            return Changed::default();
        };
        match shown {
            Shown::Crate(krate) => krate.changed(files),
            Shown::File(module) | Shown::Module(module) => module.changed(files),
        }
    }

    /// The path above the boxes: the project, and the names the map is
    /// open at, each a step back up; and what the changes laid over the
    /// map are since.
    fn render_heading(&self, depth: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.root.file_name().map_or_else(
            || self.root.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let mut crumbs: Vec<AnyElement> = Vec::new();
        let names = std::iter::once(name).chain(self.at.iter().take(depth).cloned());
        for (index, name) in names.enumerate() {
            if index > 0 {
                crumbs.push(div().text_color(rgb(DIM)).child("›").into_any_element());
            }
            let last = index == depth;
            let crumb = div()
                .id(("map-crumb", index))
                .debug_selector(move || format!("map-crumb-{index}"))
                .child(name);
            crumbs.push(if last {
                crumb.into_any_element()
            } else {
                crumb
                    .cursor_pointer()
                    .text_color(rgb(LINK))
                    .on_click(cx.listener(move |map, _, _, cx| map.ascend(index, cx)))
                    .into_any_element()
            });
        }
        let since = match &self.changes {
            Some(Overlay { since, files }) if !files.is_empty() => {
                format!("{} changed {}", files_said(files.len()), since.said())
            }
            Some(Overlay { since, .. }) => format!("nothing changed {}", since.said()),
            None => String::new(),
        };
        div()
            .debug_selector(|| "map-heading".to_owned())
            .flex()
            .gap_3()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(div().flex().flex_shrink_0().gap_1().children(crumbs))
            .child(div().min_w_0().truncate().text_color(rgb(DIM)).child(since))
    }

    /// What the tab says instead of boxes, while the map is not read, or
    /// when it cannot be.
    fn render_said(&self) -> Option<AnyElement> {
        let (said, failed) = match &self.read {
            None => ("Reading the project…".to_owned(), false),
            Some(Err(error)) => (format!("Cannot map the project: {error}"), true),
            Some(Ok(map)) if map.crates.is_empty() => {
                (format!("No crate in {}", shortened(&map.root)), true)
            }
            Some(Ok(_)) => return None,
        };
        Some(
            div()
                .debug_selector(|| "map-said".to_owned())
                .px_3()
                .py_2()
                .text_color(rgb(if failed { FAILED } else { DIM }))
                .child(said)
                .into_any_element(),
        )
    }

    fn render_box(&self, number: usize, shown: &Shown<'_>, cx: &mut Context<Self>) -> AnyElement {
        let changed = self.changed_in(shown);
        let (name, said, counts, note, click): (String, String, String, Option<String>, Click) =
            match *shown {
                Shown::Crate(krate) => (
                    krate.name.clone(),
                    krate.said().to_owned(),
                    format!("{} lines", krate.lines()),
                    (!krate.uses.is_empty()).then(|| format!("uses {}", krate.uses.join(", "))),
                    Click::Descend(krate.name.clone()),
                ),
                Shown::File(module) => (
                    module.path.file_name().map_or_else(
                        || module.name.clone(),
                        |name| name.to_string_lossy().into_owned(),
                    ),
                    module.said.clone(),
                    format!("{} lines", module.lines),
                    None,
                    Click::Open(module.path.clone()),
                ),
                Shown::Module(module) if module.modules.is_empty() => (
                    module.name.clone(),
                    module.said.clone(),
                    format!("{} lines", module.lines),
                    None,
                    Click::Open(module.path.clone()),
                ),
                Shown::Module(module) => (
                    module.name.clone(),
                    module.said.clone(),
                    format!(
                        "{}, {} lines",
                        modules_said(module.modules_in_all()),
                        module.lines_in_all()
                    ),
                    None,
                    Click::Descend(module.name.clone()),
                ),
            };
        let is_file = matches!(click, Click::Open(_));
        let mut element = div()
            .id(("map-box", number))
            .debug_selector(move || format!("map-box-{number}"))
            .w(px(BOX_WIDTH))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .border_1()
            .border_color(rgb(if changed.any() { CHANGED } else { BORDER }))
            .rounded_md()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(HOVER)))
            .child(
                div()
                    .truncate()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(if is_file { LINK } else { FOREGROUND }))
                    .child(name),
            )
            .child(div().text_xs().text_color(rgb(DIM)).child(counts))
            .on_click(cx.listener(move |map, _, _, cx| match &click {
                Click::Descend(name) => map.descend(name, cx),
                Click::Open(path) => cx.emit(Chosen(path.clone())),
            }));
        if !said.is_empty() {
            element = element.child(
                div()
                    .max_h(px(SAID_HEIGHT))
                    .overflow_hidden()
                    .text_xs()
                    .text_color(rgb(FOREGROUND))
                    .child(said),
            );
        }
        if let Some(note) = note {
            element = element.child(div().text_xs().text_color(rgb(DIM)).child(note));
        }
        if changed.any() {
            element = element.child(div().text_xs().text_color(rgb(CHANGED)).child(format!(
                "+{} −{} in {}",
                changed.added,
                changed.removed,
                files_said(changed.files)
            )));
        }
        element.into_any_element()
    }
}

/// What a click on a box does.
enum Click {
    /// Opens the map at the box.
    Descend(String),
    /// Opens the file at this path.
    Open(PathBuf),
}

fn files_said(count: usize) -> String {
    match count {
        1 => "1 file".to_owned(),
        count => format!("{count} files"),
    }
}

fn modules_said(count: usize) -> String {
    match count {
        1 => "1 module".to_owned(),
        count => format!("{count} modules"),
    }
}

/// Reads the project of the tab of `cx` at once, and again now and then,
/// for as long as the tab is.
fn follow(cx: &mut Context<Map>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let Ok(root) = this.read_with(cx, |map, _| map.root.clone()) else {
                break;
            };
            let (read, changes) = cx
                .background_executor()
                .spawn(async move {
                    let read = map::read(&root);
                    let changes = changes::read(&root).ok().map(|diff| Overlay {
                        since: diff.since.clone(),
                        files: map::changed_files(&diff),
                    });
                    (read, changes)
                })
                .await;
            if this
                .update(cx, |map, cx| map.looked(read, changes, cx))
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(LOOK_EVERY).await;
        }
    })
}

impl Shows for Map {
    fn title(&self) -> String {
        "Map".to_owned()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Map {
            at: self.at.clone(),
        }
    }
}

impl Focusable for Map {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Map {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (depth, boxes) = match self.place() {
            Some(place) => {
                let boxes: Vec<_> = place
                    .boxes
                    .iter()
                    .enumerate()
                    .map(|(index, shown)| self.render_box(index + 1, shown, cx))
                    .collect();
                (place.depth, boxes)
            }
            None => (0, Vec::new()),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .child(self.render_heading(depth, cx))
            .children(self.render_said())
            .child(
                div()
                    .id("map")
                    .debug_selector(|| "map".to_owned())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(div().flex().flex_wrap().gap_2().p_3().children(boxes)),
            )
    }
}
