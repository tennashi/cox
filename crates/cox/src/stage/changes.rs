//! A tab of the stage that shows what changed in the files of the chat's
//! project, or of another folder, such as a sketch the chat opened, as Git
//! tells it (see [`cox_git::changes`]): in a sketch, all it changed
//! since it started, and elsewhere what changed since the last commit.
//! Each file that changed is a card that heads the lines that changed in
//! it, with some around them, drawn in the colors of their syntax as a
//! tab of code draws them, and the words that changed within a line marked
//! (see [`cox_editor::words`]); a click on a file or a line opens the file
//! there, as a link to it would (see [`crate::stage`]).
//!
//! A project's folder is not a repository itself: its repositories are
//! its directories (see [`coxd::repositories`]). The tab of such a
//! folder shows what changed in one of them at a time, which the human
//! chooses by its name under the heading, beside how many files changed
//! in each.
//!
//! Beside the files, a column lists the commits of the repository shown,
//! newest first, as [`cox_git::commits`] reads them: under what is not
//! committed yet, down to where they start, with everything since then
//! as one above them. The human picks one to see what it changed, which
//! the tab shows in place of what it shows at first: in a sketch,
//! everything; elsewhere, what is not committed. A commit an agent made
//! names the turn of the chat it was made in, which a click on the turn,
//! over what the commit changed, shows (see [`ShowTurn`]).
//!
//! A repository is seen in one of its checkouts at a time: its own, or
//! another of its worktrees, wherever it is (see
//! [`coxd::repositories::checkouts`]), which the human chooses under the
//! repositories, from a menu of them, each marked as Cox's, made by hand,
//! or outside the folder, with how many files changed in it; then its
//! branches no checkout has, each seen by everything it committed, read
//! from the repository's own checkout, whose files a click does not open,
//! as they may not be there. A chat that works in one of the checkouts
//! sees that one at first. The tab keeps what the human chose. Beside it,
//! the branch its commits started after, which everything is since, as
//! Cox found it, or as the human chose it from a menu of the repository's
//! branches, which coxd keeps for the directory of the project it is,
//! apart from the repository, whose settings Cox leaves be (see
//! [`CompareWith`]).
//!
//! The tab looks again now and then, so that it shows what the agent
//! changes as it works. It draws only the rows in sight, so a long list
//! of changes scrolls as a short one does, and stays where the human
//! scrolled to as the changes do. The wheel scrolls a file's lines across
//! too, each file's apart, their numbers staying in place, so that a line
//! too long for the tab can be read to its end.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use cox_core::tabs::KeptTab;
use cox_editor::buffer::Buffer;
use cox_editor::language::{Highlight, Language};
use cox_editor::syntax::Syntax;
use cox_editor::words;
use cox_git::changes::{Diff, File, How, Kind, Line};
use cox_git::commits::{self, Base, Commit, History};
use gpui::{
    AnchoredPositionMode, AnyElement, App, AsyncApp, Context, EventEmitter, FocusHandle, Focusable,
    FontWeight, HighlightStyle, Hsla, ListAlignment, ListOffset, ListState, MouseDownEvent, Pixels,
    Point, ScrollWheelEvent, SharedString, StyledText, Task, TextRun, WeakEntity, Window, anchored,
    deferred, div, font, list, point, prelude::*, px, rgb,
};

use crate::stage::code::syntax_style;
use crate::stage::heading::{CARD_RADIUS, INSET, heading};
use crate::stage::tab::Shows;
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::vim::hints::Hinted as _;
use crate::vim::keymap::Motion;
use crate::vim::{Move, NextStretch, Open, PreviousStretch};
use coxd::paths::shortened;
use coxd::repositories::{self, Checkout, Repository};
use coxd::theme::{
    ACCENT, ADDED_GROUND, ADDED_WORD, ASKING, BORDER, CODE_FONT, DIM, DISABLED, FAILED,
    FAILED_MARK, FAINT, FOREGROUND, HOVER, LINK, RAISED, REMOVED_GROUND, REMOVED_WORD, SELECTED,
    UI_FONT,
};

pub(crate) const KEY_CONTEXT: &str = "Changes";

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
/// The room right of a line's text, scrolled across to its end.
const TEXT_END: f32 = 12.;

/// How wide the column of commits is.
const HISTORY_WIDTH: f32 = 236.;

/// How tall a button that opens a menu under the repositories is.
const MENU_BUTTON_HEIGHT: f32 = 26.;

/// What the syntax says the stretches of a line are, as bytes of the line
/// as it is drawn.
type Spans = Vec<(Range<usize>, Highlight)>;

/// What changed, as the tab shows it, and how its lines are drawn, by
/// file, then line.
type ShownRead<'a> = (&'a Result<Diff, String>, &'a [Vec<Look>]);

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

/// The human chose to compare a directory of a project with the branch
/// `base`, in what changed, or, with none, with the one Cox finds, for
/// coxd to keep (see [`cox_core::store::Store::bases`]): the directory
/// `directory` of the project in `folder`, as the store has it, '' for
/// the project's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareWith {
    pub folder: PathBuf,
    pub directory: String,
    pub base: Option<String>,
}

/// The human asks to see the turn of a chat a commit was made in: the
/// chat, by the number Cox keeps it by, and the entry that started the
/// turn, by its place in the conversation.
pub struct ShowTurn {
    pub chat: i64,
    pub entry: usize,
}

/// What of a repository the tab shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Everything that changed since its commits start, as one.
    Everything,
    /// What changed since its last commit.
    Uncommitted,
    /// What the commit of this full name changed.
    Commit(String),
    /// What amending the commit `from` into the commit `to`, by their full
    /// names, changed.
    Amendment { from: String, to: String },
}

impl Pick {
    /// The commit it shows, or the one amended into, by its full name, if
    /// it shows one.
    fn commit(&self) -> Option<&str> {
        match self {
            Self::Commit(id) | Self::Amendment { to: id, .. } => Some(id),
            Self::Everything | Self::Uncommitted => None,
        }
    }
}

/// What the human picked to see, as the tab read it: of the repository of
/// a name, in the checkout at a folder, or on a branch no checkout has,
/// and how its lines are drawn.
struct Picked {
    pick: Pick,
    repository: Option<String>,
    checkout: Option<Seen>,
    read: Result<Diff, String>,
    looks: Vec<Vec<Look>>,
}

pub struct Changes {
    /// The folder whose changes the tab shows (see [`repositories::listed`]).
    folder: PathBuf,
    /// Where the chat the tab is of works, whose checkout of a repository
    /// it shows at first.
    cwd: PathBuf,
    /// The checkout of each repository the human chose to see, once they
    /// chose one (see [`repositories::read`]).
    checkouts: Vec<repositories::Chosen>,
    /// The branch no checkout has of each repository whose commits the
    /// human chose to see, once they chose one, rather than a checkout.
    branches: Vec<repositories::ChosenBranch>,
    /// The menu open under the repositories, if one is.
    menu: Option<Menu>,
    /// How many files changed in each checkout of the repository shown, by
    /// its folder, as the tab counted them as its menu opened.
    counted: HashMap<PathBuf, Result<usize, String>>,
    /// What counts them, while it does.
    counting: Option<Task<()>>,
    /// What reads what changed at once, as the human chose a checkout.
    looking: Option<Task<()>>,
    /// The menu that a press closed, and where the press was, which on its
    /// button leaves it closed.
    closed: Option<(Menu, Point<Pixels>)>,
    /// The branches the repository shown can be compared with, as the tab
    /// read them as their menu opened; none until then.
    bases: Option<Vec<String>>,
    /// What reads them, or reads what changed since the one the human
    /// chose, while it does.
    basing: Option<Task<()>>,
    /// The project's folder and directory the store keeps what the human
    /// chose of each repository by, as the tab last read it, if it is a
    /// project's (see [`repositories::directory_of`]).
    kept_as: Vec<Option<KeptAs>>,
    /// The branch the human chose to compare each repository with, by its
    /// name, or none to let Cox find it, until the store keeps it so.
    comparing: HashMap<Option<String>, Option<String>>,
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
    /// The commits of each repository, as the tab last read them, if it
    /// could.
    histories: Vec<Option<History>>,
    /// What of the repository shown the human picked to see, once they
    /// picked; until then, what the tab shows at first (see
    /// [`first_pick`]).
    pick: Option<Pick>,
    /// What the human picked, once the tab read it.
    picked: Option<Picked>,
    /// The rows of the tab, of the repository it shows: each file's,
    /// followed by its lines'.
    rows: Vec<Row>,
    /// What Vim's keys selected, of the repository shown, once they moved
    /// to something: a file's row, or the row that starts a stretch of
    /// its lines.
    selected: Option<Row>,
    list: ListState,
    /// How far the lines of each file shown are scrolled across, by its
    /// path.
    across: HashMap<PathBuf, Pixels>,
    /// How wide the longest line of each file shown is, by its path, once
    /// the tab measured it to scroll the file across.
    widest: HashMap<PathBuf, Pixels>,
    /// How wide the text of a line is in sight, as a line was last drawn.
    in_sight: Rc<Cell<Pixels>>,
    focus_handle: FocusHandle,
    _follow: Task<()>,
    /// What reads what the human picked, while it does.
    picking: Option<Task<()>>,
}

/// A repository the human chose to show, by its name (see
/// [`Repository::name`]).
struct Choice(Option<String>);

/// A menu under the repositories, which its button opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    /// The checkouts of the repository shown.
    Checkouts,
    /// The branches to compare it with.
    Bases,
}

/// A row of the tab: the file at an index of the repository shown, or a
/// line of it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    File(usize),
    Line(usize, usize),
}

impl Row {
    /// The index of the file it is, or is a line of.
    fn file(self) -> usize {
        match self {
            Self::File(file) | Self::Line(file, _) => file,
        }
    }
}

impl EventEmitter<Chosen> for Changes {}
impl EventEmitter<ShowTurn> for Changes {}
impl EventEmitter<CompareWith> for Changes {}

impl Changes {
    /// A tab showing what changed in the repositories of `folder` (see
    /// [`repositories::of`]), for a chat working in `cwd`.
    pub fn new(folder: PathBuf, cwd: &Path, cx: &mut Context<Self>) -> Self {
        let title = if folder == cwd {
            "Changes".to_owned()
        } else {
            format!("Changes in {}", name_of(&folder))
        };
        Self {
            folder,
            cwd: cwd.to_owned(),
            checkouts: Vec::new(),
            branches: Vec::new(),
            menu: None,
            counted: HashMap::new(),
            counting: None,
            looking: None,
            closed: None,
            bases: None,
            basing: None,
            kept_as: Vec::new(),
            comparing: HashMap::new(),
            title,
            read: None,
            chosen: None,
            looks: Vec::new(),
            histories: Vec::new(),
            pick: None,
            picked: None,
            rows: Vec::new(),
            selected: None,
            list: ListState::new(0, ListAlignment::Top, px(LINE_HEIGHT * 20.)),
            across: HashMap::new(),
            widest: HashMap::new(),
            in_sight: Rc::new(Cell::new(px(0.))),
            focus_handle: cx.focus_handle(),
            _follow: follow(cx),
            picking: None,
        }
    }

    /// The tab, showing the checkouts `checkouts` choose, or the branches
    /// `branches` choose, as it kept them.
    #[must_use]
    pub fn choosing(
        mut self,
        checkouts: Vec<repositories::Chosen>,
        branches: Vec<repositories::ChosenBranch>,
    ) -> Self {
        self.checkouts = checkouts;
        self.branches = branches;
        self
    }

    /// The folder whose changes the tab shows.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The checkout of each repository the human chose to see.
    #[must_use]
    pub fn checkouts(&self) -> &[repositories::Chosen] {
        &self.checkouts
    }

    /// Shows the repository `name` (see [`Repository::name`]) in its
    /// checkout at `folder`, as the human chose it, as it shows at first,
    /// reading what changed there at once.
    pub fn show_checkout(&mut self, name: Option<String>, folder: PathBuf, cx: &mut Context<Self>) {
        self.forget_choice(name.as_deref());
        self.checkouts.push((name.clone(), folder));
        self.show_chosen(name, cx);
    }

    /// Shows what the branch `branch` of the repository `name`, which no
    /// checkout has, committed, as the human chose it, reading it at once.
    pub fn show_branch(&mut self, name: Option<String>, branch: String, cx: &mut Context<Self>) {
        self.forget_choice(name.as_deref());
        self.branches.push((name.clone(), branch));
        self.show_chosen(name, cx);
    }

    /// The branch no checkout has of each repository whose commits the
    /// human chose to see.
    #[must_use]
    pub fn branches(&self) -> &[repositories::ChosenBranch] {
        &self.branches
    }

    /// The branch each repository is compared with, by its name, if the
    /// human chose one, as the tab last read its commits, or as they chose
    /// it since.
    #[must_use]
    pub fn compared(&self) -> Vec<(Option<String>, Option<String>)> {
        self.repositories()
            .iter()
            .zip(&self.histories)
            .map(|(repository, history)| {
                let base = match self.comparing.get(&repository.name) {
                    Some(chosen) => chosen.clone(),
                    None => history.as_ref().and_then(|history| history.chosen.clone()),
                };
                (repository.name.clone(), base)
            })
            .collect()
    }

    /// Forgets the checkout or branch chosen for the repository `name`.
    fn forget_choice(&mut self, name: Option<&str>) {
        self.checkouts
            .retain(|(chosen, _)| chosen.as_deref() != name);
        self.branches
            .retain(|(chosen, _)| chosen.as_deref() != name);
    }

    /// Shows the repository `name`, as the human chose what of it to see,
    /// reading it at once.
    fn show_chosen(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.show(name, cx);
        self.looking = Some(cx.spawn(async move |this, cx| {
            look(&this, cx).await;
        }));
    }

    /// The branch no checkout has that the repository shown is seen on,
    /// if it is.
    fn shown_branch(&self) -> Option<String> {
        let checkout = self.shown()?.checkout()?;
        checkout.branch().map(str::to_owned)
    }

    /// What the tab shows of the repository shown at first (see
    /// [`first_pick`]).
    fn first_pick(&self) -> Pick {
        first_pick(self.history(), self.shown_branch().is_some())
    }

    /// Compares the repository shown with the branch `base`, as the human
    /// chose it, or, with none, with the one Cox finds, and shows everything
    /// since, once the repository keeps it.
    pub fn compare_with(&mut self, base: Option<String>, cx: &mut Context<Self>) {
        self.menu = None;
        cx.notify();
        let Some(index) = self.shown_index() else {
            return;
        };
        let name = self.repositories()[index].name.clone();
        // A repository of no project's is compared so while the tab is.
        if let Some(Some((folder, directory))) = self.kept_as.get(index) {
            cx.emit(CompareWith {
                folder: folder.clone(),
                directory: directory.clone(),
                base: base.clone(),
            });
        }
        self.comparing.insert(name, base);
        self.basing = Some(cx.spawn(async move |this, cx| {
            if look(&this, cx).await {
                let _ = this.update(cx, |changes, cx| changes.pick(Pick::Everything, cx));
            }
        }));
    }

    /// The branch the repository shown is compared with, if the human chose
    /// one.
    fn shown_base(&self) -> Option<String> {
        let shown = self.shown()?;
        match self.comparing.get(&shown.name) {
            Some(chosen) => chosen.clone(),
            None => self.history()?.chosen.clone(),
        }
    }

    /// Opens `menu`, reading what it lists, or closes it, if it is open.
    fn toggle(&mut self, menu: Menu, cx: &mut Context<Self>) {
        let open = self.menu != Some(menu);
        self.menu = open.then_some(menu);
        cx.notify();
        if open {
            match menu {
                Menu::Checkouts => self.count_checkouts(cx),
                Menu::Bases => self.read_bases(cx),
            }
        }
    }

    /// Reads the branches the repository shown can be compared with.
    fn read_bases(&mut self, cx: &mut Context<Self>) {
        self.bases = None;
        let Some(folder) = self.shown_folder() else {
            return;
        };
        self.basing = Some(cx.spawn(async move |this, cx| {
            let bases = cx
                .background_executor()
                .spawn(async move { commits::bases(&folder) })
                .await;
            let _ = this.update(cx, |changes, cx| {
                changes.bases = Some(bases);
                cx.notify();
            });
        }));
    }

    /// Counts the files that changed in each checkout of the repository
    /// shown.
    fn count_checkouts(&mut self, cx: &mut Context<Self>) {
        let folders: Vec<PathBuf> = self.shown().map_or_default(|shown| {
            shown
                .checkouts
                .iter()
                .filter(|checkout| checkout.branch().is_none())
                .map(|checkout| checkout.folder().to_owned())
                .collect()
        });
        self.counting = Some(cx.spawn(async move |this, cx| {
            let counted = cx
                .background_executor()
                .spawn(async move {
                    folders
                        .into_iter()
                        .map(|folder| {
                            let count = cox_git::changes::count(&folder);
                            (folder, count)
                        })
                        .collect::<HashMap<_, _>>()
                })
                .await;
            let _ = this.update(cx, |changes, cx| {
                changes.counted = counted;
                cx.notify();
            });
        }));
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
    /// top, as the human chose it, as it shows at first.
    pub fn show(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.chosen = Some(Choice(name));
        self.menu = None;
        self.pick = None;
        self.picked = None;
        self.relist(false);
        cx.notify();
    }

    /// The commits of the repository shown, as the tab read them, if it
    /// could.
    #[must_use]
    pub fn history(&self) -> Option<&History> {
        self.shown_index()
            .and_then(|index| self.histories.get(index))
            .and_then(Option::as_ref)
    }

    /// What of the repository shown the tab shows: what the human picked,
    /// or else what it shows at first.
    #[must_use]
    pub fn shown_pick(&self) -> Pick {
        self.pick.clone().unwrap_or_else(|| self.first_pick())
    }

    /// What changed, as the tab shows it, of the repository shown, and how
    /// its lines are drawn: none while what the human picked is read.
    #[must_use]
    pub fn shown_diff(&self) -> Option<&Result<Diff, String>> {
        self.shown_read().map(|(read, _)| read)
    }

    fn shown_read(&self) -> Option<ShownRead<'_>> {
        let index = self.shown_index()?;
        let shown = self.repositories().get(index)?;
        let at_first = (
            &shown.read,
            self.looks.get(index).map_or(&[][..], Vec::as_slice),
        );
        let Some(pick) = &self.pick else {
            return Some(at_first);
        };
        if *pick == self.first_pick() {
            return Some(at_first);
        }
        let checkout = shown.checkout().map(seen);
        self.picked
            .as_ref()
            .filter(|picked| {
                &picked.pick == pick
                    && picked.repository == shown.name
                    && picked.checkout == checkout
            })
            .map(|picked| (&picked.read, picked.looks.as_slice()))
    }

    /// Shows `pick` of the repository shown, as the human picked it,
    /// reading it first.
    pub fn pick(&mut self, pick: Pick, cx: &mut Context<Self>) {
        self.pick = Some(pick);
        self.relist(false);
        self.read_picked(cx);
        cx.notify();
    }

    /// Shows what the commit `id` changed, of the repository `name` (see
    /// [`Repository::name`]), as a commit the chat shows was picked.
    pub fn show_commit(&mut self, name: Option<String>, id: String, cx: &mut Context<Self>) {
        self.chosen = Some(Choice(name));
        self.pick(Pick::Commit(id), cx);
    }

    /// The folder of the checkout of the repository shown.
    fn shown_folder(&self) -> Option<PathBuf> {
        self.shown()
            .and_then(Repository::checkout)
            .map(|checkout| checkout.folder().to_owned())
    }

    /// Reads what the human picked, of the repository shown, unless the
    /// tab shows it at first, and shows it once read.
    fn read_picked(&mut self, cx: &mut Context<Self>) {
        let Some(pick) = self.pick.clone() else {
            return;
        };
        let Some(folder) = self.shown_folder() else {
            return;
        };
        let repository = match &self.chosen {
            Some(Choice(name)) => name.clone(),
            None => self.shown().and_then(|shown| shown.name.clone()),
        };
        let checkout = self.shown().and_then(Repository::checkout).map(seen);
        let branch = self.shown_branch();
        let base = self.shown_base();
        self.picking = Some(cx.spawn(async move |this, cx| {
            let reading = pick.clone();
            let (read, looks) = cx
                .background_executor()
                .spawn(async move {
                    let read = read_pick(&folder, branch.as_deref(), base.as_deref(), &reading);
                    let looks = read.as_ref().map_or_default(looks);
                    (read, looks)
                })
                .await;
            let _ = this.update(cx, |changes, cx| {
                if changes.pick.as_ref() != Some(&pick) {
                    return;
                }
                let same = changes
                    .picked
                    .as_ref()
                    .is_some_and(|picked| picked.pick == pick && picked.read == read);
                if same {
                    return;
                }
                let stay = changes
                    .picked
                    .as_ref()
                    .is_some_and(|picked| picked.pick == pick);
                changes.picked = Some(Picked {
                    pick,
                    repository,
                    checkout,
                    read,
                    looks,
                });
                changes.relist(stay);
                cx.notify();
            });
        }));
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
    /// scrolled to if `stay`, or else from its top, and its files from
    /// their left.
    fn relist(&mut self, stay: bool) {
        self.widest.clear();
        if !stay {
            self.across.clear();
            self.selected = None;
        }
        self.rows = self
            .shown_diff()
            .and_then(|read| read.as_ref().ok())
            .map_or_default(rows);
        let top = self.list.logical_scroll_top();
        self.list.splice(0..self.list.item_count(), self.rows.len());
        if stay && !self.rows.is_empty() {
            self.list.scroll_to(ListOffset {
                item_ix: top.item_ix.min(self.rows.len() - 1),
                offset_in_item: top.offset_in_item,
            });
        } else if !stay {
            // Rows spliced into an empty list land above its scroll top,
            // which would leave it at their end.
            self.list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
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

    /// The tab read the commits again, which differ from those it shows:
    /// shows them.
    fn read_histories(&mut self, histories: Vec<Option<History>>, cx: &mut Context<Self>) {
        self.histories = histories;
        cx.notify();
    }

    /// Reads again what the human picked, now and then, as what is not
    /// committed changes; what a commit changed does not.
    fn follow_picked(&mut self, cx: &mut Context<Self>) {
        if matches!(self.pick, Some(Pick::Everything | Pick::Uncommitted)) {
            self.read_picked(cx);
        }
    }

    /// Whether a click opens `file`, of what the tab shows: unless it was
    /// removed, or the tab shows a branch no checkout has, whose files may
    /// not be where the tab reads it from.
    fn opens(&self, file: &File) -> bool {
        file.how != How::Removed && self.shown_branch().is_none()
    }

    /// The human chose the file at `index` of the repository shown, at
    /// `line` if they chose one.
    fn choose(&self, index: usize, line: Option<usize>, cx: &mut Context<Self>) {
        let Some(Ok(diff)) = self.shown_diff() else {
            return;
        };
        let Some(file) = diff.files.get(index) else {
            return;
        };
        if self.opens(file) {
            cx.emit(Chosen {
                path: diff.top.join(&file.path),
                line,
            });
        }
    }

    /// Moves the selection from file to file as Vim's keys ask, the file
    /// selected at the top, or scrolls by half the tab's height.
    fn move_by_keys(&mut self, motion: &Move, window: &mut Window, cx: &mut Context<Self>) {
        let files = self
            .shown_diff()
            .and_then(|read| read.as_ref().ok())
            .map_or(0, |diff| diff.files.len());
        let Some(last) = files.checked_sub(1) else {
            return;
        };
        if matches!(motion.motion, Motion::HalfDown | Motion::HalfUp) {
            motion.scroll_list(&self.list, window.line_height());
            cx.notify();
            return;
        }
        let index = match (self.selected, motion.motion) {
            (None, Motion::Down | Motion::Up) => 0,
            (at, _) => motion.moved(at.map_or(0, Row::file), 1, last),
        };
        self.select(Row::File(index), cx);
    }

    /// Selects the next stretch of changed lines as Vim's keys ask, after
    /// what is selected, or with nothing selected the first in sight.
    fn next_stretch(&mut self, _: &NextStretch, _: &mut Window, cx: &mut Context<Self>) {
        let after = self
            .selected_at()
            .map_or(self.list.logical_scroll_top().item_ix, |at| at + 1);
        let found = self.stretches().find(|&at| at >= after);
        if let Some(at) = found {
            self.select(self.rows[at], cx);
        }
    }

    /// Selects the stretch of changed lines before what is selected, or
    /// with nothing selected the last above those in sight.
    fn previous_stretch(&mut self, _: &PreviousStretch, _: &mut Window, cx: &mut Context<Self>) {
        let before = self
            .selected_at()
            .unwrap_or(self.list.logical_scroll_top().item_ix);
        let found = self.stretches().rfind(|&at| at < before);
        if let Some(at) = found {
            self.select(self.rows[at], cx);
        }
    }

    /// Where the row selected is among the tab's rows, if one is.
    fn selected_at(&self) -> Option<usize> {
        let selected = self.selected?;
        self.rows.iter().position(|row| *row == selected)
    }

    /// Where the rows that start a stretch of changed lines are among the
    /// tab's rows.
    fn stretches(&self) -> impl DoubleEndedIterator<Item = usize> + '_ {
        let diff = self.shown_diff().and_then(|read| read.as_ref().ok());
        self.rows.iter().enumerate().filter_map(move |(at, row)| {
            let Row::Line(file, line) = *row else {
                return None;
            };
            let kind = diff?.files.get(file)?.lines.get(line)?.kind;
            (kind == Kind::Stretch).then_some(at)
        })
    }

    /// Selects `row`, scrolling it to the top of the tab.
    fn select(&mut self, row: Row, cx: &mut Context<Self>) {
        self.selected = Some(row);
        if let Some(at) = self.selected_at() {
            self.list.scroll_to(ListOffset {
                item_ix: at,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    /// Opens the file selected, as a click on it does, or the file of the
    /// stretch selected at its first changed line.
    fn open_selected(&mut self, _: &Open, _: &mut Window, cx: &mut Context<Self>) {
        match self.selected {
            Some(Row::File(index)) => self.choose(index, None, cx),
            Some(Row::Line(index, line)) => {
                let at = self
                    .shown_diff()
                    .and_then(|read| read.as_ref().ok())
                    .and_then(|diff| diff.files.get(index))
                    .and_then(|file| first_changed(file.lines.get(line..)?));
                self.choose(index, at, cx);
            }
            None => {}
        }
    }

    /// The wheel turned over the file at `index` of the repository shown:
    /// scrolls its lines across, as far as its longest line ends in sight.
    fn scroll_across(
        &mut self,
        index: usize,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT)).x;
        if delta == px(0.) {
            return;
        }
        let Some(Ok(diff)) = self.shown_diff() else {
            return;
        };
        let Some(file) = diff.files.get(index) else {
            return;
        };
        let path = file.path.clone();
        let widest = if let Some(&widest) = self.widest.get(&path) {
            widest
        } else {
            let widest = widest(file, window);
            self.widest.insert(path.clone(), widest);
            widest
        };
        let most = (widest + px(TEXT_END) - self.in_sight.get()).max(px(0.));
        let at = self.across.entry(path).or_default();
        let across = (*at - delta).clamp(px(0.), most);
        if across != *at {
            *at = across;
            cx.notify();
        }
    }

    /// The tab's heading, as the plugins head their pages: its name, what
    /// changed in the repository shown, and where it is.
    fn render_heading(&self) -> impl IntoElement {
        let shown = self.shown();
        let (said, failed) = match self.shown_diff() {
            None if self.read.is_none() || shown.is_some() => {
                (div().child("Reading what changed…"), false)
            }
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
            let checkout = repository.checkout().map(|checkout| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(11.5))
                    .text_color(rgb(FAINT))
                    .when(matches!(checkout, Checkout::Worktree(_)), |label| {
                        label.child(icon(Icon::Flow, 11., FAINT))
                    })
                    .child(checkout.label())
            });
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
                .children(checkout)
                .children(mark.map(|(mark, color)| {
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(color))
                        .child(mark)
                }))
                .on_hint_click(cx.listener(move |changes, _, _, cx| {
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

    /// The row under the repositories, once what changed in the one shown
    /// was read: a button saying what its checkout goes by, and whether it
    /// is a worktree, which opens the menu of its checkouts; then, but in a
    /// sketch, which is compared with where it started, the branch its
    /// commits started after, which opens the menu of the branches to
    /// compare it with.
    fn render_compared(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let shown = self.shown()?;
        shown.read.as_ref().ok()?;
        let checkout = shown.checkout()?;
        let button = self
            .menu_button("checkout", Menu::Checkouts, cx)
            .child(icon(checkout_icon(checkout), 12., DIM))
            .child(checkout.label())
            .children(checkout_tags(checkout))
            .child(caret());
        let checkouts =
            (self.menu == Some(Menu::Checkouts)).then(|| self.render_checkouts_menu(shown, cx));
        let compared = self
            .history()
            .filter(|history| !matches!(history.base, Base::Start(_) | Base::Nothing))
            .map(|history| {
                let label = match &history.base {
                    Base::Branched { from, .. } => div().child(from.clone()),
                    _ => div().text_color(rgb(FAINT)).child("choose a branch"),
                };
                let button = self
                    .menu_button("base", Menu::Bases, cx)
                    .child(icon(Icon::Changes, 12., DIM))
                    .child(label)
                    .when(history.chosen.is_none(), |button| {
                        button.child(tag("found".to_owned(), FAINT))
                    })
                    .child(caret());
                let bases = (self.menu == Some(Menu::Bases))
                    .then(|| self.render_bases_menu(shown, history, cx));
                [
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(FAINT))
                        .child("compared with"),
                    under(button, bases),
                ]
            });
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(INSET))
                .pt(px(8.))
                .child(under(button, checkouts))
                .children(compared.into_iter().flatten()),
        )
    }

    /// A button of the row under the repositories, found as `id`, which
    /// opens `menu`, or closes it.
    fn menu_button(
        &self,
        id: &'static str,
        menu: Menu,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_owned())
            .flex_shrink_0()
            .h(px(MENU_BUTTON_HEIGHT))
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(9.))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(if self.menu == Some(menu) {
                SELECTED
            } else {
                RAISED
            }))
            .font_family(CODE_FONT)
            .text_size(px(12.))
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(HOVER)))
            .on_hint_mouse_down(cx.listener(move |changes, event: &MouseDownEvent, _, cx| {
                // The press that closed the menu, on its button, leaves it
                // closed.
                if changes.closed.take() != Some((menu, event.position)) {
                    changes.toggle(menu, cx);
                }
                cx.stop_propagation();
            }))
    }

    /// A menu under the repositories, `menu`, found as `id`, headed
    /// `heading`, which a press anywhere else closes.
    fn menu_list(
        id: &'static str,
        menu: Menu,
        heading: String,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        crate::ui::menu::menu()
            .id(id)
            .debug_selector(move || id.to_owned())
            .min_w(px(320.))
            .max_w(px(640.))
            .max_h(px(360.))
            .overflow_y_scroll()
            .on_mouse_down_out(cx.listener(move |changes, event: &MouseDownEvent, _, cx| {
                changes.menu = None;
                changes.closed = Some((menu, event.position));
                cx.notify();
            }))
            .child(
                div()
                    .px(px(10.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(FAINT))
                    .child(heading),
            )
    }

    /// The menu of the checkouts of `shown`, its own first, each with
    /// where it is, whether Cox made it, for which chat, or it was made by
    /// hand, or is outside the folder, and how many files changed in it,
    /// once counted; the one shown marked, and a click on another shows it.
    fn render_checkouts_menu(
        &self,
        shown: &Repository,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let heading = match &shown.name {
            Some(name) => format!("CHECKOUTS OF {}", name.to_uppercase()),
            None => "CHECKOUTS".to_owned(),
        };
        let first_branch = shown
            .checkouts
            .iter()
            .position(|checkout| checkout.branch().is_some());
        let rows = shown.checkouts.iter().enumerate().map(|(index, checkout)| {
            let folder = checkout.folder().to_owned();
            let counted = match checkout
                .branch()
                .map_or_else(|| self.counted.get(&folder), |_| None)
            {
                Some(Ok(0)) => "clean".to_owned(),
                Some(Ok(1)) => "1 changed".to_owned(),
                Some(Ok(count)) => format!("{count} changed"),
                Some(Err(_)) => "!".to_owned(),
                None => String::new(),
            };
            let label = div()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon(checkout_icon(checkout), 12., DIM))
                .child(
                    div()
                        .flex_shrink_0()
                        .font_family(CODE_FONT)
                        .text_size(px(12.5))
                        .child(checkout.label()),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(11.5))
                        .text_color(rgb(FAINT))
                        .child(ellipsis(where_is(&folder, &self.folder))),
                )
                .children(checkout_tags(checkout));
            let name = shown.name.clone();
            let branch = checkout.branch().map(str::to_owned);
            let row = crate::ui::menu::item(("checkout-choice", index), label, &counted)
                .when(index == shown.shown, |row| row.bg(rgb(SELECTED)))
                .on_hint_click(cx.listener(move |changes, _, _, cx| match &branch {
                    Some(branch) => changes.show_branch(name.clone(), branch.clone(), cx),
                    None => changes.show_checkout(name.clone(), folder.clone(), cx),
                }));
            let heading = (Some(index) == first_branch).then(|| {
                div()
                    .mt(px(4.))
                    .px(px(10.))
                    .pt(px(6.))
                    .pb(px(2.))
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(FAINT))
                    .child("BRANCHES NO CHECKOUT HAS")
            });
            div().children(heading).child(row)
        });
        let rows: Vec<_> = rows.collect();
        Self::menu_list("checkouts", Menu::Checkouts, heading, cx).children(rows)
    }

    /// The menu of the branches to compare `shown` with, whose commits
    /// `history` read: first to let Cox find it, as it did unless the
    /// human chose one, then each branch, once read, the remote's default
    /// first; the one compared with marked, and a click on another
    /// compares with it.
    fn render_bases_menu(
        &self,
        shown: &Repository,
        history: &History,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let heading = match &shown.name {
            Some(name) => format!("COMPARE {} WITH", name.to_uppercase()),
            None => "COMPARE WITH".to_owned(),
        };
        let found = crate::ui::menu::item(
            "base-found",
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child("Found by Cox")
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(FAINT))
                        .child("the remote's default branch, or the upstream"),
                ),
            "",
        )
        .when(history.chosen.is_none(), |row| row.bg(rgb(SELECTED)))
        .on_hint_click(cx.listener(|changes, _, _, cx| changes.compare_with(None, cx)));
        let rows: Vec<AnyElement> = match &self.bases {
            None => vec![
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(FAINT))
                    .child("Reading the branches…")
                    .into_any_element(),
            ],
            Some(bases) => bases
                .iter()
                .enumerate()
                .map(|(index, base)| {
                    let chosen = history.chosen.as_ref() == Some(base);
                    let picked = base.clone();
                    crate::ui::menu::item(
                        ("base-choice", index),
                        div()
                            .font_family(CODE_FONT)
                            .text_size(px(12.5))
                            .child(base.clone()),
                        "",
                    )
                    .when(chosen, |row| row.bg(rgb(SELECTED)))
                    .on_hint_click(cx.listener(move |changes, _, _, cx| {
                        changes.compare_with(Some(picked.clone()), cx);
                    }))
                    .into_any_element()
                })
                .collect(),
        };
        Self::menu_list("bases", Menu::Bases, heading, cx)
            .child(found)
            .child(crate::ui::menu::parting())
            .children(rows)
    }

    /// The column of the commits of the repository shown, as the tab read
    /// them, if it could: everything since they start, as one, if
    /// something tells where they start; then, newest first, what is not
    /// committed yet, each commit, and where they start. The one the tab
    /// shows is marked, and a click on another shows it.
    fn render_history(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let history = self.history()?;
        let shown = self.shown_pick();
        let mut rows: Vec<AnyElement> = Vec::new();
        if history.base != Base::Recent {
            let row = history_row(
                "history-everything".into(),
                None,
                shown == Pick::Everything,
                "Everything".into(),
                Some(
                    div()
                        .text_color(rgb(FAINT))
                        .child(ellipsis(history.base.said()))
                        .into_any_element(),
                ),
            )
            .mb(px(6.))
            .on_hint_click(cx.listener(|changes, _, _, cx| changes.pick(Pick::Everything, cx)));
            rows.push(row.into_any_element());
        }
        // A branch no checkout has has nothing that is not committed.
        if self.shown_branch().is_none() {
            let row = history_row(
                "history-uncommitted".into(),
                Some((Dot::Uncommitted, false, true)),
                shown == Pick::Uncommitted,
                "Not committed yet".into(),
                None,
            )
            .on_hint_click(cx.listener(|changes, _, _, cx| changes.pick(Pick::Uncommitted, cx)));
            rows.push(row.into_any_element());
        }
        for (index, commit) in history.commits.iter().enumerate() {
            let picked = Pick::Commit(commit.id.clone());
            let row = history_row(
                format!("history-commit-{index}").into(),
                Some((Dot::Commit, true, true)),
                shown.commit().is_some_and(|id| commit.was(id)),
                commit.subject.clone().into(),
                Some(commit_meta(commit).into_any_element()),
            )
            .on_hint_click(cx.listener(move |changes, _, _, cx| changes.pick(picked.clone(), cx)));
            rows.push(row.into_any_element());
        }
        let base = match &history.base {
            Base::Start(commit) => format!("{commit} · the sketch started here"),
            Base::Nothing => "The sketch started from nothing".to_owned(),
            Base::Branched { from, at } => format!("{at} · branched from {from}"),
            Base::Recent => "Earlier commits are left out".to_owned(),
        };
        let row = history_row(
            "history-base".into(),
            Some((Dot::Base, true, false)),
            false,
            base.into(),
            None,
        )
        .text_color(rgb(FAINT));
        rows.push(row.into_any_element());
        Some(
            div()
                .id("history")
                .debug_selector(|| "history".to_owned())
                .flex_shrink_0()
                .w(px(HISTORY_WIDTH))
                .h_full()
                .overflow_y_scroll()
                .pl(px(INSET - 4.))
                .pr_2()
                .pb_3()
                .border_r_1()
                .border_color(rgb(BORDER))
                .flex()
                .flex_col()
                .children(rows),
        )
    }

    /// Over what a commit changed, as the human picked it to see: what it
    /// says, and the turn of the chat it was made in, if it was marked,
    /// which a click shows; and, if it was amended, what it was each time,
    /// a click on which shows what it changed then.
    fn render_commit(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let pick = self.shown_pick();
        let id = pick.commit()?;
        let commit = self
            .history()?
            .commits
            .iter()
            .find(|commit| commit.was(id))?;
        // The commit as it was, if the human picked what it was before.
        let shown = commit
            .earlier
            .iter()
            .find(|earlier| earlier.id == id)
            .unwrap_or(commit);
        let turn = shown.turn.as_ref().map(|turn| {
            let (chat, entry) = (turn.chat, turn.entry);
            div()
                .id("commit-turn")
                .debug_selector(|| "commit-turn".to_owned())
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .cursor_pointer()
                .text_color(rgb(LINK))
                .hover(|link| link.text_color(rgb(FOREGROUND)))
                .child(ellipsis(format!("From the turn: {} ›", turn.line)))
                .on_hint_click(cx.listener(move |_, _, _, cx| cx.emit(ShowTurn { chat, entry })))
        });
        Some(
            div()
                .debug_selector(|| "commit".to_owned())
                .px(px(INSET + 4.))
                .pb_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(ellipsis(shown.subject.clone())),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .whitespace_nowrap()
                        .text_size(px(12.))
                        .text_color(rgb(DIM))
                        .child(
                            div()
                                .flex_shrink_0()
                                .font_family(CODE_FONT)
                                .child(shown.short()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .child(counts(shown.added, shown.removed)),
                        )
                        .child(div().flex_shrink_0().child(ago(shown.when)))
                        .children(turn),
                )
                .children(Self::render_versions(commit, &pick, cx)),
        )
    }

    /// What `commit` was each time it was amended, if it was: first what
    /// it changes now, then the commit it was first, and each it was made
    /// into, oldest first, with how long ago and the line of the turn it
    /// was made in. The one `pick` shows is marked, and a click on another
    /// shows what it changed: the first all it changed, each after what
    /// amending into it changed.
    fn render_versions(
        commit: &Commit,
        pick: &Pick,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        if commit.earlier.is_empty() {
            return None;
        }
        let row = |selector: String, picked: Pick, label: &'static str, version: &Commit| {
            version_row(selector, *pick == picked, label, version).on_hint_click(
                cx.listener(move |changes, _, _, cx| changes.pick(picked.clone(), cx)),
            )
        };
        let versions: Vec<&Commit> = commit.earlier.iter().chain([commit]).collect();
        let mut rows = vec![row(
            "commit-version-now".to_owned(),
            Pick::Commit(commit.id.clone()),
            "All of it",
            commit,
        )];
        for (index, version) in versions.iter().enumerate() {
            let (picked, label) = match index.checked_sub(1).map(|before| versions[before]) {
                None => (Pick::Commit(version.id.clone()), "Made"),
                Some(before) => (
                    Pick::Amendment {
                        from: before.id.clone(),
                        to: version.id.clone(),
                    },
                    "Amended",
                ),
            };
            rows.push(row(
                format!("commit-version-{index}"),
                picked,
                label,
                version,
            ));
        }
        Some(
            div()
                .debug_selector(|| "commit-versions".to_owned())
                .mt_1()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_2()
                        .pb(px(2.))
                        .text_size(px(10.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(FAINT))
                        .child("VERSIONS"),
                )
                .children(rows),
        )
    }

    /// The row at `row` of the tab.
    fn render_row(&self, row: usize, selected: Option<Row>, cx: &mut Context<Self>) -> AnyElement {
        let Some(&listed) = self.rows.get(row) else {
            return div().into_any_element();
        };
        let Some((Ok(diff), looks)) = self.shown_read() else {
            return div().into_any_element();
        };
        let (index, line) = match listed {
            Row::File(index) => (index, None),
            Row::Line(index, line) => (index, Some(line)),
        };
        let file = &diff.files[index];
        let across = self.across.get(&file.path).copied().unwrap_or_default();
        let last = line.map_or(file.lines.is_empty(), |line| line + 1 == file.lines.len());
        let (inner, at) = match line {
            None => {
                let head = file_row(row, file);
                let head = if selected == Some(listed) {
                    head.bg(rgb(SELECTED))
                } else {
                    head
                };
                (head, None)
            }
            Some(line) => {
                let none = Look::default();
                let look = looks
                    .get(index)
                    .and_then(|looks| looks.get(line))
                    .unwrap_or(&none);
                let first = line == 0;
                let shown = &file.lines[line];
                let in_sight = self.in_sight.clone();
                let drawn = line_row(row, shown, first, last, look, across, in_sight);
                let drawn = if selected == Some(listed) {
                    // Tall enough to see even where the stretch starts the
                    // file and says nothing.
                    drawn.h(px(STRETCH_HEIGHT)).bg(rgb(SELECTED))
                } else {
                    drawn
                };
                (drawn, Some(shown.at))
            }
        };
        let inner = if last {
            inner.rounded_b(px(CARD_RADIUS)).border_b_1()
        } else {
            inner
        };
        let inner = if self.opens(file) {
            inner
                .cursor_pointer()
                .on_hint_click(cx.listener(move |changes, _, _, cx| changes.choose(index, at, cx)))
        } else {
            inner
        };
        div()
            .px(px(INSET))
            .when(line.is_none(), |row| row.pt(px(10.)))
            .when(last, |row| row.pb(px(4.)))
            .on_scroll_wheel(cx.listener(move |changes, event, window, cx| {
                changes.scroll_across(index, event, window, cx);
            }))
            .child(inner.border_x_1().border_color(rgb(BORDER)))
            .into_any_element()
    }
}

/// What the tab shows of a repository at first, of those it can show of
/// its commits, `history`: in a sketch, and on a branch no checkout has,
/// if it is `branch`, everything since it started; elsewhere, what is not
/// committed.
fn first_pick(history: Option<&History>, branch: bool) -> Pick {
    match history.map(|history| &history.base) {
        _ if branch => Pick::Everything,
        Some(Base::Start(_) | Base::Nothing) => Pick::Everything,
        _ => Pick::Uncommitted,
    }
}

/// What a checkout is, to tell what was read in it from what was read in
/// another: its folder, and the branch it is if no checkout has it.
type Seen = (PathBuf, Option<String>);

/// What `checkout` is (see [`Seen`]).
fn seen(checkout: &Checkout) -> Seen {
    (
        checkout.folder().to_owned(),
        checkout.branch().map(str::to_owned),
    )
}

/// What changed in the repository at `folder`, or what its branch
/// `branch` committed, if given, as `pick` says, compared with the branch
/// `base`, if the human chose one.
fn read_pick(
    folder: &Path,
    branch: Option<&str>,
    base: Option<&str>,
    pick: &Pick,
) -> Result<Diff, String> {
    match pick {
        Pick::Everything => commits::everything_of(folder, branch, base)?
            .ok_or_else(|| "Nothing tells where the commits start.".to_owned()),
        Pick::Uncommitted => commits::uncommitted(folder),
        Pick::Commit(id) => commits::diff_of(folder, id),
        Pick::Amendment { from, to } => commits::amendment(folder, from, to),
    }
}

/// How long ago `when`, in seconds since the epoch, was, short, as `3m`.
fn ago(when: i64) -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |now| i64::try_from(now.as_secs()).unwrap_or(i64::MAX));
    let seconds = (now - when).max(0);
    if seconds < 60 {
        "now".to_owned()
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

/// What marks a row of the column of commits, on the line down it.
#[derive(Clone, Copy)]
enum Dot {
    Uncommitted,
    Commit,
    Base,
}

/// A row of the column of commits, found as `selector`: on the line down
/// the column, if it has a dot there, with whether the line goes on above
/// and below it, marked if it is `selected`, saying `title`, and under it
/// `meta`, if given.
fn history_row(
    selector: SharedString,
    dot: Option<(Dot, bool, bool)>,
    selected: bool,
    title: SharedString,
    meta: Option<AnyElement>,
) -> gpui::Stateful<gpui::Div> {
    let line = |shown: bool| div().w(px(2.)).when(shown, |line| line.bg(rgb(BORDER)));
    let column = dot.map(|(dot, above, below)| {
        let mark = div().flex_shrink_0().size(px(10.)).rounded_full();
        let mark = match dot {
            Dot::Uncommitted => mark.border_2().border_color(rgb(ASKING)),
            Dot::Commit => mark.bg(rgb(ACCENT)),
            Dot::Base => mark.border_2().border_color(rgb(DISABLED)),
        };
        div()
            .flex_shrink_0()
            .w(px(18.))
            .flex()
            .flex_col()
            .items_center()
            .child(line(above).h(px(14.)))
            .child(mark)
            .child(line(below).flex_1())
    });
    let id = selector.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .flex()
        .children(column)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .my(px(2.))
                .px_2()
                .py(px(5.))
                .rounded(px(8.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .cursor_pointer()
                .map(|row| {
                    if selected {
                        row.bg(rgb(SELECTED))
                    } else {
                        row.hover(|style| style.bg(rgb(HOVER)))
                    }
                })
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(12.5))
                        .child(ellipsis(title)),
                )
                .children(meta.map(|meta| {
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(11.5))
                        .child(meta)
                })),
        )
}

/// A row of what an amended commit was, found as `selector`, marked if
/// it is `selected`, saying `label`, then the short name of `version`, how
/// long ago it was made, and the line of the turn it was made in, if it
/// was marked.
fn version_row(
    selector: String,
    selected: bool,
    label: &'static str,
    version: &Commit,
) -> gpui::Stateful<gpui::Div> {
    let said = version
        .turn
        .as_ref()
        .map_or_default(|turn| format!("“{}”", turn.line));
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .gap(px(10.))
        .px_2()
        .py(px(3.))
        .rounded(px(6.))
        .whitespace_nowrap()
        .text_size(px(12.))
        .cursor_pointer()
        .map(|row| {
            if selected {
                row.bg(rgb(SELECTED))
            } else {
                row.hover(|style| style.bg(rgb(HOVER)))
            }
        })
        .child(
            div()
                .flex_shrink_0()
                .w(px(64.))
                .text_color(rgb(if selected { FOREGROUND } else { DIM }))
                .child(label),
        )
        .child(
            div()
                .flex_shrink_0()
                .font_family(CODE_FONT)
                .text_color(rgb(FAINT))
                .child(version.short()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(FAINT))
                .child(ago(version.when)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_color(rgb(DIM))
                .child(ellipsis(said)),
        )
}

/// What a row of the column of commits says of `commit` under it: its
/// short name, how many lines it gained and lost, how long ago it was
/// made, and under them whether it was amended, and the line that started
/// the turn it was made in, if it was marked.
fn commit_meta(commit: &Commit) -> impl IntoElement {
    let amended = !commit.earlier.is_empty();
    let said = commit.turn.as_ref().map(|turn| {
        div()
            .min_w_0()
            .overflow_hidden()
            .text_color(rgb(DIM))
            .child(ellipsis(format!("“{}”", turn.line)))
    });
    div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .text_color(rgb(FAINT))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .whitespace_nowrap()
                .child(div().font_family(CODE_FONT).child(commit.short()))
                .child(counts(commit.added, commit.removed))
                .child(div().flex_1())
                .child(ago(commit.when)),
        )
        .when(amended || said.is_some(), |meta| {
            meta.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .whitespace_nowrap()
                    .when(amended, |line| {
                        line.child(
                            div()
                                .flex_shrink_0()
                                .text_color(rgb(ASKING))
                                .child("amended"),
                        )
                    })
                    .children(said),
            )
        })
}

/// The icon of `checkout`: a branch's for a repository's own, a
/// worktree's for another.
fn checkout_icon(checkout: &Checkout) -> Icon {
    match checkout {
        Checkout::Own { .. } | Checkout::Branch { .. } => Icon::Changes,
        Checkout::Worktree(_) => Icon::Flow,
    }
}

/// `button`, with `menu` under it, if it is open, kept in the window.
fn under(button: gpui::Stateful<gpui::Div>, menu: Option<gpui::Stateful<gpui::Div>>) -> gpui::Div {
    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .child(button)
        .children(menu.map(|menu| {
            deferred(
                anchored()
                    .position_mode(AnchoredPositionMode::Local)
                    .position(point(px(0.), px(MENU_BUTTON_HEIGHT + 4.)))
                    .snap_to_window()
                    .child(menu),
            )
            .with_priority(2)
        }))
}

/// The mark at the end of a button that opens a menu.
fn caret() -> gpui::Div {
    div().text_size(px(10.)).text_color(rgb(FAINT)).child("▾")
}

/// A word that marks what a button or a row of a menu names, as `said`,
/// in `color`.
fn tag(said: String, color: u32) -> gpui::Div {
    div()
        .flex_shrink_0()
        .px(px(5.))
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(BORDER))
        .font_family(UI_FONT)
        .text_size(px(10.5))
        .text_color(rgb(color))
        .child(said)
}

/// What marks `checkout`: for a worktree, the chat Cox made it for, or
/// that it was made by hand, and whether it is outside the tab's folder;
/// for a branch no checkout has, that only its commits are seen.
fn checkout_tags(checkout: &Checkout) -> Vec<gpui::Div> {
    let worktree = match checkout {
        Checkout::Worktree(worktree) => worktree,
        Checkout::Branch { .. } => return vec![tag("commits only".to_owned(), FAINT)],
        Checkout::Own { .. } => return Vec::new(),
    };
    let made = match worktree.made_for {
        Some(chat) => tag(format!("chat {chat}"), ACCENT),
        None => tag("by hand".to_owned(), FAINT),
    };
    std::iter::once(made)
        .chain(worktree.outside.then(|| tag("outside".to_owned(), ASKING)))
        .collect()
}

/// Where `checkout`, a folder, is: from `folder`, the tab's, when it is
/// in it, or else as people read a path.
fn where_is(checkout: &Path, folder: &Path) -> String {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_owned());
    canonical(checkout)
        .strip_prefix(canonical(folder))
        .ok()
        .filter(|within| !within.as_os_str().is_empty())
        .map_or_else(
            || shortened(checkout),
            |within| within.display().to_string(),
        )
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

/// The line of the file as it is to open the stretch that starts `lines`
/// at: where it first changes, or where it starts.
fn first_changed(lines: &[Line]) -> Option<usize> {
    let (start, rest) = lines.split_first()?;
    let changed = rest
        .iter()
        .take_while(|line| line.kind != Kind::Stretch)
        .find(|line| line.kind != Kind::Same);
    Some(changed.unwrap_or(start).at)
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
                .map_or_default(|syntax| syntax.highlights(buffer, at, usize::MAX))
        })
        .collect()
}

/// A line's text as the tab draws it, its tabs as spaces.
fn drawn(text: &str) -> String {
    text.replace('\t', "    ")
}

/// Reads what changed in the tab of `cx` at once, and again now and
/// then, for as long as the tab is (see [`look`]).
fn follow(cx: &mut Context<Changes>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while look(&this, cx).await {
            cx.background_executor().timer(LOOK_EVERY).await;
        }
    })
}

/// Reads what changed in the tab `this`, in the checkouts it chooses, and
/// the commits there, reading what the syntax says of what changed when
/// it changed, and shows them, unless the human chose other checkouts
/// meanwhile; answers whether the tab is still there.
async fn look(this: &WeakEntity<Changes>, cx: &mut AsyncApp) -> bool {
    let Ok((folder, chosen, cwd, comparing)) = this.read_with(cx, |changes, _| {
        let chosen = (changes.checkouts.clone(), changes.branches.clone());
        let comparing = changes.comparing.clone();
        (
            changes.folder.clone(),
            chosen,
            changes.cwd.clone(),
            comparing,
        )
    }) else {
        return false;
    };
    // Where the store is, as this thread was told.
    let store = coxd::storage::path(coxd::storage::Kept::Chats);
    let asked = (chosen.clone(), comparing.clone());
    let (read, histories, kept_as, kept) = cx
        .background_executor()
        .spawn(async move {
            let (checkouts, branches) = &chosen;
            let kept = KeptBases::read(store.as_deref(), &folder);
            let compared = |name: Option<&str>| match comparing.get(&name.map(str::to_owned)) {
                Some(chosen) => chosen.clone(),
                None => kept.base(&folder, name),
            };
            let read = repositories::read(&folder, checkouts, branches, &cwd, &compared);
            let kept_as: Vec<Option<KeptAs>> = read
                .iter()
                .map(|repository| kept.kept_as(&folder, repository.name.as_deref()))
                .collect();
            let kept: Vec<Option<String>> = read
                .iter()
                .map(|repository| kept.base(&folder, repository.name.as_deref()))
                .collect();
            let histories = read
                .iter()
                .map(|repository| {
                    let checkout = repository.checkout()?;
                    let base = compared(repository.name.as_deref());
                    commits::history_of(checkout.folder(), checkout.branch(), base.as_deref()).ok()
                })
                .collect::<Vec<_>>();
            (read, histories, kept_as, kept)
        })
        .await;
    let still = |changes: &Changes| {
        (&changes.checkouts, &changes.branches, &changes.comparing)
            == (&asked.0.0, &asked.0.1, &asked.1)
    };
    let Ok(changed) = this.update(cx, |changes, cx| {
        if !still(changes) {
            return false;
        }
        // What the store keeps now is what the human chose.
        for (repository, kept) in read.iter().zip(&kept) {
            if changes.comparing.get(&repository.name) == Some(kept) {
                changes.comparing.remove(&repository.name);
            }
        }
        changes.kept_as = kept_as;
        if changes.histories != histories {
            changes.read_histories(histories, cx);
        }
        changes.follow_picked(cx);
        changes.read.as_ref() != Some(&read)
    }) else {
        return false;
    };
    if !changed {
        return true;
    }
    let (read, looks) = cx
        .background_executor()
        .spawn(async move {
            let looks = read
                .iter()
                .map(|shown| shown.read.as_ref().map_or_default(looks))
                .collect();
            (read, looks)
        })
        .await;
    this.update(cx, |changes, cx| {
        if (&changes.checkouts, &changes.branches) == (&asked.0.0, &asked.0.1) {
            changes.looked(read, looks, cx);
        }
    })
    .is_ok()
}

/// The project's folder and the name of its directory a repository is, as
/// the store keeps what the human chose of it by (see
/// [`repositories::directory_of`]).
type KeptAs = (PathBuf, String);

/// What the store keeps of the branches the human chose to compare the
/// repositories of a tab with: the folders of the projects, and the
/// branches chosen for the directories of those its repositories can be
/// of, by their folders.
struct KeptBases {
    projects: Vec<PathBuf>,
    bases: Vec<(PathBuf, Vec<(String, String)>)>,
}

impl KeptBases {
    /// What the store at `store` keeps of the repositories of the tab of
    /// `folder`: nothing where it cannot be read.
    fn read(store: Option<&Path>, folder: &Path) -> Self {
        let store = store.and_then(|store| cox_core::store::Store::read(store).ok());
        let projects: Vec<PathBuf> = store
            .as_ref()
            .and_then(|store| store.projects().ok())
            .map_or_default(|projects| {
                projects.into_iter().map(|project| project.folder).collect()
            });
        let mut bases: Vec<(PathBuf, Vec<(String, String)>)> = Vec::new();
        if let Some(store) = &store {
            // The project of the folder, whose directories they are, or
            // the one the folder is a worktree of a directory of.
            let of =
                [Some(""), None].map(|name| repositories::directory_of(&projects, folder, name));
            for (project, _) in of.into_iter().flatten() {
                if !bases.iter().any(|(kept, _)| *kept == project) {
                    let chosen = store.bases(&project).unwrap_or_default();
                    bases.push((project, chosen));
                }
            }
        }
        Self { projects, bases }
    }

    /// The project's folder and directory the repository `name` of the
    /// tab of `folder` is kept as, if it is a project's.
    fn kept_as(&self, folder: &Path, name: Option<&str>) -> Option<KeptAs> {
        repositories::directory_of(&self.projects, folder, name)
    }

    /// The branch the human chose to compare the repository `name` of the
    /// tab of `folder` with, as the store keeps it, if they chose one.
    fn base(&self, folder: &Path, name: Option<&str>) -> Option<String> {
        let (project, directory) = self.kept_as(folder, name)?;
        let (_, bases) = self.bases.iter().find(|(kept, _)| *kept == project)?;
        bases
            .iter()
            .find(|(named, _)| *named == directory)
            .map(|(_, base)| base.clone())
    }
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
/// and its text, drawn as `look` says, scrolled `across`, on the ground
/// its kind has. Drawing it tells `in_sight` how wide its text is in
/// sight.
fn line_row(
    row: usize,
    line: &Line,
    first: bool,
    last: bool,
    look: &Look,
    across: Pixels,
    in_sight: Rc<Cell<Pixels>>,
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
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                // Its one child is as wide as it is, however long the text
                // it holds.
                .on_children_prepainted(move |bounds, _, _| {
                    if let Some(text) = bounds.first() {
                        in_sight.set(text.size.width);
                    }
                })
                .child(
                    div()
                        .relative()
                        .left(-across)
                        .text_color(rgb(FOREGROUND))
                        .child(StyledText::new(text).with_highlights(highlights)),
                ),
        )
}

/// How wide the longest line of `file` is, drawn as the tab draws it.
fn widest(file: &File, window: &Window) -> Pixels {
    let font = font(CODE_FONT);
    file.lines
        .iter()
        .filter(|line| line.kind != Kind::Stretch)
        .map(|line| {
            let text = drawn(&line.text);
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(text.into(), px(CODE_SIZE), &[run], None)
                .width
        })
        .fold(px(0.), Pixels::max)
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
        .map_or_default(|(_, within)| drawn(within.trim()));
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
            checkouts: self.checkouts.clone(),
            branches: self.branches.clone(),
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
            cx.processor(|changes, row, window, cx| {
                let selected = changes
                    .selected
                    .filter(|_| changes.focus_handle.is_focused(window));
                changes.render_row(row, selected, cx)
            }),
        )
        .flex_1()
        .min_h_0();
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::move_by_keys))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::next_stretch))
            .on_action(cx.listener(Self::previous_stretch))
            .text_color(rgb(FOREGROUND))
            .font_family(UI_FONT)
            .text_size(px(13.))
            .child(self.render_heading())
            .children(self.render_repositories(cx))
            .children(self.render_compared(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .pt(px(6.))
                    .children(self.render_history(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .children(self.render_commit(cx))
                            .child(list.pb_3()),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> i64 {
        i64::try_from(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap()
    }

    fn line(kind: Kind, at: usize) -> Line {
        Line {
            kind,
            old: None,
            new: None,
            at,
            text: String::new(),
        }
    }

    #[test]
    fn a_time_under_a_minute_ago_is_now() {
        assert_eq!(ago(now() - 30), "now");
    }

    #[test]
    fn a_time_to_come_is_now() {
        assert_eq!(ago(now() + 3600), "now");
    }

    #[test]
    fn a_time_long_ago_is_told_in_whole_minutes_hours_or_days() {
        let ago = |seconds: i64| ago(now() - seconds);

        assert_eq!(
            [ago(150), ago(2 * 3600 + 1800), ago(3 * 86_400 + 3600)],
            ["2m", "2h", "3d"]
        );
    }

    #[test]
    fn a_stretch_opens_where_it_first_changes() {
        let lines = [
            line(Kind::Stretch, 10),
            line(Kind::Same, 10),
            line(Kind::Same, 11),
            line(Kind::Added, 12),
            line(Kind::Removed, 13),
        ];

        assert_eq!(first_changed(&lines), Some(12));
    }

    #[test]
    fn a_stretch_with_no_change_before_the_next_opens_where_it_starts() {
        let lines = [
            line(Kind::Stretch, 10),
            line(Kind::Same, 10),
            line(Kind::Stretch, 40),
            line(Kind::Added, 40),
        ];

        assert_eq!(first_changed(&lines), Some(10));
    }

    #[test]
    fn no_lines_open_nowhere() {
        assert_eq!(first_changed(&[]), None);
    }
}
