//! What the window does for Vim's keys (see [`crate::vim`]): the
//! keyboard moved among the rail, the stage and the chat, a selection
//! moved along the rail or the list of chats, and the stage's tabs.

use gpui::{Context, FocusHandle, Focusable as _, Window};

use super::Workspace;
use crate::stage::Tabs;
use crate::vim::Region;
use crate::vim::keymap::{Act, Pick, Toward};

/// A part of the window with no cursor of its own, where Vim's keys move
/// a selection: the rail of projects, or the list of chats.
pub(super) struct Picker {
    pub(super) focus: FocusHandle,
    /// The place of the row selected among the part's rows.
    at: usize,
}

impl Picker {
    pub(super) fn new<T>(cx: &mut Context<T>) -> Self {
        Self {
            focus: cx.focus_handle(),
            at: 0,
        }
    }

    /// The place of the row selected among `rows` rows, while the part
    /// has the keyboard.
    pub(super) fn picked(&self, rows: usize, window: &Window) -> Option<usize> {
        (rows > 0 && self.focus.is_focused(window)).then(|| self.at.min(rows - 1))
    }
}

/// Where `pick`, `count` times, moves the selection at `at` among `rows`
/// rows.
fn moved(at: usize, pick: Pick, count: usize, rows: usize) -> usize {
    let last = rows.saturating_sub(1);
    let at = at.min(last);
    match pick {
        Pick::Down => at.saturating_add(count).min(last),
        Pick::Up => at.saturating_sub(count),
        Pick::First => 0,
        Pick::Last => last,
    }
}

/// The parts of the window `ctrl-w` moves the keyboard among, from left
/// to right as the window draws them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Rail,
    Stage,
    Chat,
    /// The list of chats, over the chat.
    List,
}

impl Part {
    fn of(region: Region) -> Self {
        match region {
            Region::Rail => Self::Rail,
            Region::List => Self::List,
            Region::Workspace | Region::Chat => Self::Chat,
            _ => Self::Stage,
        }
    }

    /// The part `toward` goes to from this one, of `parts`, left to
    /// right, if it goes to one.
    fn toward(self, toward: Toward, parts: &[Self]) -> Option<Self> {
        let column = if self == Self::List { Self::Chat } else { self };
        let at = parts.iter().position(|part| *part == column)?;
        match toward {
            Toward::Left => at.checked_sub(1).map(|at| parts[at]),
            Toward::Right => parts.get(at + 1).copied(),
            Toward::Round => Some(parts[(at + 1) % parts.len()]),
            Toward::Up => (self == Self::Chat).then_some(Self::List),
            Toward::Down => (self == Self::List).then_some(Self::Chat),
        }
    }
}

impl Workspace {
    /// Does `act` for Vim's keys, pressed in `region` after `count`.
    pub fn act(
        &mut self,
        act: Act,
        region: Region,
        count: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let times = count.map_or(1, |count| usize::try_from(count).unwrap_or(usize::MAX));
        match act {
            Act::Go(toward) => {
                let parts: &[Part] = if self.centered().is_some() {
                    &[Part::Rail, Part::Stage, Part::Chat]
                } else {
                    &[Part::Rail, Part::Chat]
                };
                if let Some(part) = Part::of(region).toward(toward, parts) {
                    self.go(part, window, cx);
                }
            }
            Act::Pick(pick) => {
                let rows = self.rows(region).len();
                if let Some(picker) = self.picker(region) {
                    picker.at = moved(picker.at, pick, times, rows);
                    cx.notify();
                }
            }
            Act::Open => self.open_picked(region, window, cx),
            Act::New => {
                let project = match region {
                    Region::Rail => self.picked_row(region),
                    _ => None,
                }
                .unwrap_or(self.chosen);
                let folder = self.projects[project].folder.clone();
                self.open(Some(folder), window, cx);
            }
            Act::NextTab => self.tabs(
                if count.is_some() {
                    Tabs::Number(times)
                } else {
                    Tabs::Along(1)
                },
                window,
                cx,
            ),
            Act::PreviousTab => self.tabs(
                Tabs::Along(-isize::try_from(times).unwrap_or(1)),
                window,
                cx,
            ),
            Act::CloseTab => self.tabs(Tabs::Close, window, cx),
            Act::Menu => self.tabs(Tabs::Menu, window, cx),
        }
    }

    /// The rows of the part `region` says, by their indices among the
    /// projects or the chats: the projects on the rail, or the chats of
    /// the project chosen.
    fn rows(&self, region: Region) -> Vec<usize> {
        match region {
            Region::Rail => self
                .projects
                .iter()
                .enumerate()
                .filter(|(_, project)| !project.archived && !project.deleted)
                .map(|(index, _)| index)
                .collect(),
            Region::List => self
                .chats
                .iter()
                .enumerate()
                .filter(|(_, chat)| chat.project == self.chosen)
                .map(|(index, _)| index)
                .collect(),
            _ => Vec::new(),
        }
    }

    fn picker(&mut self, region: Region) -> Option<&mut Picker> {
        match region {
            Region::Rail => Some(&mut self.rail),
            Region::List => Some(&mut self.list),
            _ => None,
        }
    }

    /// The index of the project or chat selected in `region`.
    fn picked_row(&self, region: Region) -> Option<usize> {
        let rows = self.rows(region);
        let picker = match region {
            Region::Rail => &self.rail,
            Region::List => &self.list,
            _ => return None,
        };
        rows.get(picker.at.min(rows.len().checked_sub(1)?)).copied()
    }

    /// Opens what is selected: the project on the rail, which shows its
    /// chat, or the chat in the list, with the keyboard in its input.
    fn open_picked(&mut self, region: Region, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.picked_row(region) else {
            return;
        };
        match region {
            Region::Rail => self.choose(index, window, cx),
            _ => self.show(index, window, cx),
        }
    }

    /// Moves the keyboard to `part`, its selection on what is chosen or
    /// shown there.
    fn go(&mut self, part: Part, window: &mut Window, cx: &mut Context<Self>) {
        match part {
            Part::Rail => {
                let rows = self.rows(Region::Rail);
                self.rail.at = rows
                    .iter()
                    .position(|index| *index == self.chosen)
                    .unwrap_or(0);
                self.switching = false;
                window.focus(&self.rail.focus);
            }
            Part::Stage => {
                self.switching = false;
                self.tabs(Tabs::Focus, window, cx);
            }
            Part::Chat => {
                self.switching = false;
                self.folded = false;
                window.focus(&self.focus_handle(cx));
            }
            Part::List => {
                let shown = self.chats.shown_index();
                self.list.at = self
                    .rows(Region::List)
                    .iter()
                    .position(|index| Some(*index) == shown)
                    .unwrap_or(0);
                self.switching = true;
                self.folded = false;
                window.focus(&self.list.focus);
            }
        }
        cx.notify();
    }

    /// Asks `tabs` of the stage in the middle of the window.
    fn tabs(&self, tabs: Tabs, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(stage) = self.centered() {
            (stage.by_keys)(tabs, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Part, moved};
    use crate::vim::Region;
    use crate::vim::keymap::{Pick, Toward};

    const ALL: &[Part] = &[Part::Rail, Part::Stage, Part::Chat];

    #[test]
    fn control_w_goes_left_and_right_as_the_window_draws_its_parts() {
        let from = |region| Part::of(region);
        assert_eq!(
            from(Region::Chat).toward(Toward::Left, ALL),
            Some(Part::Stage)
        );
        assert_eq!(
            from(Region::Code).toward(Toward::Left, ALL),
            Some(Part::Rail)
        );
        assert_eq!(from(Region::Rail).toward(Toward::Left, ALL), None);
        assert_eq!(
            from(Region::Tree).toward(Toward::Right, ALL),
            Some(Part::Chat)
        );
        assert_eq!(
            from(Region::List).toward(Toward::Left, ALL),
            Some(Part::Stage)
        );
        assert_eq!(from(Region::Chat).toward(Toward::Right, ALL), None);
        assert_eq!(
            from(Region::Chat).toward(Toward::Round, ALL),
            Some(Part::Rail)
        );
        assert_eq!(
            from(Region::Workspace).toward(Toward::Up, ALL),
            Some(Part::List)
        );
        assert_eq!(
            from(Region::List).toward(Toward::Down, ALL),
            Some(Part::Chat)
        );
        assert_eq!(from(Region::Rail).toward(Toward::Up, ALL), None);
        let no_stage = &[Part::Rail, Part::Chat];
        assert_eq!(
            from(Region::Rail).toward(Toward::Right, no_stage),
            Some(Part::Chat)
        );
    }

    #[test]
    fn a_selection_stays_among_the_rows() {
        assert_eq!(moved(0, Pick::Down, 3, 5), 3);
        assert_eq!(moved(3, Pick::Down, 9, 5), 4);
        assert_eq!(moved(4, Pick::Up, 1, 2), 0);
        assert_eq!(moved(0, Pick::Last, 1, 5), 4);
        assert_eq!(moved(4, Pick::First, 1, 5), 0);
        assert_eq!(moved(0, Pick::Down, 1, 0), 0);
    }
}
