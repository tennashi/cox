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
//! The mouse resting on a box shows what it has to do with the others:
//! the boxes it uses, and those that use it, are colored, and the line
//! above the boxes names them, from what the files name (see
//! [`cox_editor::map::Module::names`]), or, for crates, from their
//! manifests. The ‹ and › above the boxes go back and forward between the
//! places the map was open at, as a tab of code does between the places
//! it went to.
//!
//! The tab looks again now and then, so that it follows the project as
//! the agent changes it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use cox_core::tabs::KeptTab;
use cox_editor::changes::{self, Since};
use cox_editor::history::History;
use cox_editor::map::{self, Changed, Crate, Module};
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, NavigationDirection, ScrollHandle, Task, Window, actions, div, prelude::*, px,
    rgb,
};

use crate::stage::tab::Shows;
use crate::ui::paths::shortened;
use crate::ui::theme::{BACKGROUND, DIM, FAILED, FOREGROUND, LINK};

actions!(
    map,
    [
        /// Goes back to where the map was open before.
        Back,
        /// Goes forward again to where the map went back from.
        Forward,
    ]
);

const KEY_CONTEXT: &str = "Map";

/// Binds the keys of the tab.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-[", Back, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-]", Forward, Some(KEY_CONTEXT)),
        KeyBinding::new("alt-left", Back, Some(KEY_CONTEXT)),
        KeyBinding::new("alt-right", Forward, Some(KEY_CONTEXT)),
    ]);
}

/// How often the tab looks at the project again.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// How wide a box is.
const BOX_WIDTH: f32 = 236.;
/// How tall what a box says of itself is at most: three lines.
const SAID_HEIGHT: f32 = 60.;

const BORDER: u32 = 0x40_4040;
const HOVER: u32 = 0x2a_2d2e;
/// The ground of a box the box under the mouse uses, of one that uses
/// it, and of one that does both.
const USES: u32 = 0x1f_2d40;
const USED_BY: u32 = 0x3a_2e1c;
const BOTH: u32 = 0x2e_2b3a;
/// The words for those, in the line above the boxes.
const USES_TEXT: u32 = 0x7f_b4ff;
const USED_BY_TEXT: u32 = 0xe0_b060;
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
    /// The box the mouse rests on, by its index among the boxes shown.
    hovered: Option<usize>,
    /// Where the map was open before, to go back and forward between.
    history: History<Vec<String>>,
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
    /// The crate the map is open in, unless it is open at the crates.
    krate: Option<&'a Crate>,
    boxes: Vec<Shown<'a>>,
}

/// What a box has to do with the box the mouse rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Relation {
    None,
    /// It is that box.
    Hovered,
    /// That box uses it.
    Uses,
    /// It uses that box.
    UsedBy,
    /// Each uses the other.
    Both,
}

impl Shown<'_> {
    /// The name the box shows.
    fn label(&self) -> String {
        match self {
            Shown::Crate(krate) => krate.name.clone(),
            Shown::File(module) => module.path.file_name().map_or_else(
                || module.name.clone(),
                |name| name.to_string_lossy().into_owned(),
            ),
            Shown::Module(module) => module.name.clone(),
        }
    }

    /// What the files of the box name (see [`Module::names`]).
    fn names(&self) -> Vec<&[String]> {
        match self {
            Shown::Crate(krate) => krate
                .entries
                .iter()
                .flat_map(Module::names_in_all)
                .collect(),
            Shown::File(module) => module.names.iter().map(Vec::as_slice).collect(),
            Shown::Module(module) => module.names_in_all().collect(),
        }
    }

    /// Whether `name` leads into the box, in crate `krate` when the box
    /// is in one. A file a crate starts from holds what is at the top of
    /// the crate and in none of its modules, whichever file declares
    /// them: `crate::` in `main.rs` means `lib.rs` as much as itself.
    fn holds(&self, krate: Option<&Crate>, name: &[String]) -> bool {
        match (self, krate) {
            (Shown::Crate(this), _) => name.first() == Some(&this.name),
            (Shown::File(module), Some(krate)) if module.at.is_empty() => match name {
                [first] => *first == krate.name,
                [first, one] => {
                    *first == krate.name && !krate.modules().iter().any(|m| m.name == *one)
                }
                _ => false,
            },
            (Shown::File(module), Some(krate)) => module.file_is_named(&krate.name, name),
            (Shown::Module(module), Some(krate)) => module.is_named(&krate.name, name),
            (Shown::File(_) | Shown::Module(_), None) => false,
        }
    }

    /// Whether the box uses `other`: a crate by its manifest, and the
    /// rest by what their files name.
    fn uses(&self, other: &Self, krate: Option<&Crate>) -> bool {
        match (self, other) {
            (Shown::Crate(this), Shown::Crate(other)) => this.uses.contains(&other.name),
            _ => self.names().iter().any(|name| other.holds(krate, name)),
        }
    }
}

/// What each box of `place` has to do with the one at `hovered`.
fn relations(place: &Place<'_>, hovered: Option<usize>) -> Vec<Relation> {
    let Some(chosen) = hovered.and_then(|index| place.boxes.get(index)) else {
        return vec![Relation::None; place.boxes.len()];
    };
    place
        .boxes
        .iter()
        .enumerate()
        .map(|(index, shown)| {
            if Some(index) == hovered {
                return Relation::Hovered;
            }
            match (
                chosen.uses(shown, place.krate),
                shown.uses(chosen, place.krate),
            ) {
                (true, true) => Relation::Both,
                (true, false) => Relation::Uses,
                (false, true) => Relation::UsedBy,
                (false, false) => Relation::None,
            }
        })
        .collect()
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
            hovered: None,
            history: History::new(),
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
        let mut at = self.at.clone();
        at.truncate(depth);
        at.push(name.to_owned());
        self.go(at, cx);
    }

    /// Opens the map `depth` names down from the crates: at the crates
    /// for 0.
    fn ascend(&mut self, depth: usize, cx: &mut Context<Self>) {
        let mut at = self.at.clone();
        at.truncate(depth);
        self.go(at, cx);
    }

    /// Opens the map `at`, from where it is open, which it can go back to.
    fn go(&mut self, at: Vec<String>, cx: &mut Context<Self>) {
        if at == self.at {
            return;
        }
        let here = std::mem::replace(&mut self.at, at);
        self.history.leave(here);
        self.moved(cx);
    }

    /// The map opened somewhere else.
    fn moved(&mut self, cx: &mut Context<Self>) {
        self.hovered = None;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    fn back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        self.go_back(cx);
    }

    fn forward(&mut self, _: &Forward, _: &mut Window, cx: &mut Context<Self>) {
        self.go_forward(cx);
    }

    /// Goes back to where the map was open before.
    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(at) = self.history.back() {
            let here = std::mem::replace(&mut self.at, at);
            self.history.went_back(here);
            self.moved(cx);
        }
    }

    /// Goes forward again to where the map went back from.
    fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(at) = self.history.forward() {
            let here = std::mem::replace(&mut self.at, at);
            self.history.went_forward(here);
            self.moved(cx);
        }
    }

    /// The mouse came to rest on the box at `index`, or left it.
    fn hover(&mut self, index: usize, hovered: bool, cx: &mut Context<Self>) {
        let now = if hovered {
            Some(index)
        } else if self.hovered == Some(index) {
            None
        } else {
            self.hovered
        };
        if now != self.hovered {
            self.hovered = now;
            cx.notify();
        }
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
                krate: None,
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
        Some(Place {
            depth,
            krate: Some(krate),
            boxes,
        })
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

    /// The line above the boxes: ‹ and ›, when there is somewhere to go
    /// back or forward to; the path to where the map is open, the
    /// project and the names, each a step back up; and what the box the
    /// mouse rests on has to do with the others, or else what the changes
    /// laid over the map are since.
    fn render_heading(
        &self,
        place: Option<&Place<'_>>,
        relations: &[Relation],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let depth = place.map_or(0, |place| place.depth);
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
        let button = |id: &'static str, label: &'static str, enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .flex_shrink_0()
                .px_1()
                .rounded_sm()
                .map(|button| {
                    if enabled {
                        button
                            .cursor_pointer()
                            .text_color(rgb(FOREGROUND))
                            .hover(|style| style.bg(rgb(0x40_4040)))
                    } else {
                        button.text_color(rgb(0x55_5555))
                    }
                })
                .child(label)
        };
        div()
            .debug_selector(|| "map-heading".to_owned())
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(BORDER))
            .when(
                self.history.has_back() || self.history.has_forward(),
                |heading| {
                    heading.child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .gap_1()
                            .child(
                                button("map-back", "‹", self.history.has_back())
                                    .on_click(cx.listener(|map, _, _, cx| map.go_back(cx))),
                            )
                            .child(
                                button("map-forward", "›", self.history.has_forward())
                                    .on_click(cx.listener(|map, _, _, cx| map.go_forward(cx))),
                            ),
                    )
                },
            )
            .child(div().flex().flex_shrink_0().gap_1().children(crumbs))
            .child(
                match place.and_then(|place| self.render_relations(place, relations)) {
                    Some(relations) => relations,
                    None => div()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(DIM))
                        .child(since)
                        .into_any_element(),
                },
            )
            .into_any_element()
    }

    /// What the box the mouse rests on has to do with the others, in
    /// words: which it uses, and which use it.
    fn render_relations(&self, place: &Place<'_>, relations: &[Relation]) -> Option<AnyElement> {
        let hovered = self.hovered.and_then(|index| place.boxes.get(index))?;
        let named = |wanted: [Relation; 2]| -> Vec<String> {
            place
                .boxes
                .iter()
                .zip(relations)
                .filter(|(_, relation)| wanted.contains(relation))
                .map(|(shown, _)| shown.label())
                .collect()
        };
        let uses = named([Relation::Uses, Relation::Both]);
        let used_by = named([Relation::UsedBy, Relation::Both]);
        let mut line = div()
            .debug_selector(|| "map-relations".to_owned())
            .flex()
            .min_w_0()
            .gap_2()
            .whitespace_nowrap()
            .overflow_hidden()
            .text_color(rgb(DIM))
            .child(div().text_color(rgb(FOREGROUND)).child(hovered.label()));
        if uses.is_empty() && used_by.is_empty() {
            return Some(line.child("uses none of these").into_any_element());
        }
        if !uses.is_empty() {
            line = line
                .child("uses")
                .child(div().text_color(rgb(USES_TEXT)).child(uses.join(", ")));
        }
        if !used_by.is_empty() {
            if !uses.is_empty() {
                line = line.child("·");
            }
            line = line.child("used by").child(
                div()
                    .text_color(rgb(USED_BY_TEXT))
                    .child(used_by.join(", ")),
            );
        }
        Some(line.into_any_element())
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

    /// The box at `index` among those shown, which is `number` to the
    /// human, showing `shown`, with `relation` to the box the mouse rests
    /// on.
    fn render_box(
        &self,
        index: usize,
        shown: &Shown<'_>,
        relation: Relation,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let number = index + 1;
        let changed = self.changed_in(shown);
        let name = shown.label();
        let (said, counts, note, click): (String, String, Option<String>, Click) = match *shown {
            Shown::Crate(krate) => (
                krate.said().to_owned(),
                format!("{} lines", krate.lines()),
                (!krate.uses.is_empty()).then(|| format!("uses {}", krate.uses.join(", "))),
                Click::Descend(krate.name.clone()),
            ),
            Shown::File(module) => (
                module.said.clone(),
                format!("{} lines", module.lines),
                None,
                Click::Open(module.path.clone()),
            ),
            Shown::Module(module) if module.modules.is_empty() => (
                module.said.clone(),
                format!("{} lines", module.lines),
                None,
                Click::Open(module.path.clone()),
            ),
            Shown::Module(module) => (
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
        let ground = match relation {
            Relation::None => None,
            Relation::Hovered => Some(HOVER),
            Relation::Uses => Some(USES),
            Relation::UsedBy => Some(USED_BY),
            Relation::Both => Some(BOTH),
        };
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
            .when_some(ground, |element, ground| element.bg(rgb(ground)))
            .on_hover(cx.listener(move |map, hovered: &bool, _, cx| {
                map.hover(index, *hovered, cx);
            }))
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
        let place = self.place();
        let relations = place
            .as_ref()
            .map(|place| relations(place, self.hovered))
            .unwrap_or_default();
        let boxes: Vec<_> = place
            .as_ref()
            .map(|place| {
                place
                    .boxes
                    .iter()
                    .zip(&relations)
                    .enumerate()
                    .map(|(index, (shown, relation))| self.render_box(index, shown, *relation, cx))
                    .collect()
            })
            .unwrap_or_default();
        let heading = self.render_heading(place.as_ref(), &relations, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::forward))
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .child(heading)
            .children(self.render_said())
            .child(
                div()
                    .id("map")
                    .debug_selector(|| "map".to_owned())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_mouse_down(
                        MouseButton::Navigate(NavigationDirection::Back),
                        cx.listener(|map, _, _, cx| map.go_back(cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Navigate(NavigationDirection::Forward),
                        cx.listener(|map, _, _, cx| map.go_forward(cx)),
                    )
                    .child(div().flex().flex_wrap().gap_2().p_3().children(boxes)),
            )
    }
}
