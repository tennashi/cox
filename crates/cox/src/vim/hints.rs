//! Hint mode over the window (see [`super`]): a label over each thing in
//! sight that can be clicked or typed in, of the part with the keyboard
//! (`f`) or of the whole window (`F`), the page shown's among them, and
//! the labels that still match as the human types one, until one is
//! left, which is clicked.
//!
//! What can be clicked says so as it is drawn, with [`Hinted`], and is
//! clicked by its own listener: GPUI keeps the window's input to itself.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, FocusHandle, InteractiveElement, IntoElement,
    KeyboardButton, KeyboardClickEvent, Modifiers, MouseButton, MouseDownEvent, ParentElement,
    Pixels, StatefulInteractiveElement, Styled, Window, WindowId, canvas, deferred, div, px, rgb,
};

use super::Region;

/// The letters labels are made of, those under the fingers first.
const LETTERS: &[char] = &[
    's', 'a', 'd', 'f', 'j', 'k', 'l', 'e', 'w', 'c', 'm', 'p', 'g', 'h',
];

/// Labels for `count` hints, all as long, so that none starts another.
#[must_use]
pub fn labels(count: usize) -> Vec<String> {
    let mut length = 1;
    let mut most = LETTERS.len();
    while most < count {
        length += 1;
        most = most.saturating_mul(LETTERS.len());
    }
    (0..count)
        .map(|mut number| {
            let mut label = vec![' '; length];
            for at in (0..length).rev() {
                label[at] = LETTERS[number % LETTERS.len()];
                number /= LETTERS.len();
            }
            label.into_iter().collect()
        })
        .collect()
}

/// Clicks what is drawn over the bounds it is given.
pub type Click = Rc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App)>;

/// What taking a hint does: clicks what it is over, or gives the keyboard
/// to the field it is over, in insert mode.
#[derive(Clone)]
pub enum Take {
    Click(Click),
    Field(FocusHandle),
}

/// The parts of the window `f` labels what is in, by the region with the
/// keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Rail,
    /// The list of chats, over the chat.
    List,
    /// The chat: its conversation, its input and what is around them.
    Chat,
    /// The stage: its tabs' bar and the tab shown.
    Stage,
    Settings,
    ProjectCard,
}

impl Part {
    /// The part `f` labels with the keyboard in `region`, if it labels
    /// one: a page labels its own.
    #[must_use]
    pub const fn of(region: Region) -> Option<Self> {
        match region {
            Region::Rail => Some(Self::Rail),
            Region::List => Some(Self::List),
            Region::Workspace | Region::Chat | Region::Goal | Region::Sketch | Region::Renaming => {
                Some(Self::Chat)
            }
            Region::Settings => Some(Self::Settings),
            Region::ProjectCard => Some(Self::ProjectCard),
            Region::Terminal
            | Region::Code
            | Region::Document
            | Region::Tree
            | Region::Changes
            | Region::Android
            | Region::Devices
            | Region::Flow
            | Region::Windows
            | Region::MacWindow => Some(Self::Stage),
            Region::Page
            | Region::Address
            | Region::Translation
            | Region::Sheet
            | Region::StageMenu
            | Region::Command => None,
        }
    }
}

/// A page whose hints join the window's (`F`): it lists what can be
/// clicked in it, draws the labels it is given over them, and takes one.
pub trait Page {
    /// Lists the page's hints, by their ids, to `answer` once it has them.
    fn ask(&self, answer: Answer, cx: &mut App);
    /// Draws each label over the hint of its id, in place of those before.
    fn show(&self, labels: &[(u32, String)], cx: &mut App);
    /// Takes the hint of `id`: clicks it, or gives the keyboard to it.
    fn take(&self, id: u32, window: &mut Window, cx: &mut App);
    /// Takes the labels away.
    fn clear(&self, cx: &mut App);
}

/// Where a page answers with the ids of its hints.
pub type Answer = Box<dyn FnOnce(Vec<u32>, &mut Window, &mut App)>;

/// What a part of the window said of itself as it was drawn.
#[derive(Clone)]
enum Mark {
    /// It can be clicked or typed in, where it is in sight.
    Spot(Bounds<Pixels>, Take),
    /// It hides what was drawn under it.
    Cover(Bounds<Pixels>),
    Part(Part, Bounds<Pixels>),
    Page(Rc<dyn Page>),
}

thread_local! {
    /// What each window's parts said of themselves in its last frame, in
    /// the order they were drawn.
    static MARKS: RefCell<Vec<(WindowId, Mark)>> = const { RefCell::new(Vec::new()) };
    /// The hints over a window, while it is in hint mode.
    static HINTS: RefCell<Option<Hints>> = const { RefCell::new(None) };
}

fn window_id(window: &Window) -> WindowId {
    window.window_handle().window_id()
}

/// Forgets what `window`'s parts said, as a frame starts drawing them
/// again: the window does, as it draws.
pub fn forget_marks(window: &Window) {
    let id = window_id(window);
    MARKS.with(|marks| marks.borrow_mut().retain(|(window, _)| *window != id));
}

fn mark(mark: impl FnOnce(Bounds<Pixels>, &Window) -> Option<Mark> + 'static) -> impl IntoElement {
    canvas(
        move |bounds, window, _| {
            if let Some(mark) = mark(bounds, window) {
                let id = window_id(window);
                MARKS.with(|marks| marks.borrow_mut().push((id, mark)));
            }
        },
        |_, (), _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Marks the element it is put in as something to take as `take` says,
/// where it is in sight. It takes the whole of the element and draws
/// nothing.
#[must_use]
pub fn spot(take: Take) -> impl IntoElement {
    mark(move |bounds, window| {
        let seen = bounds.intersect(&window.content_mask().bounds);
        (seen.size.width > px(0.) && seen.size.height > px(0.)).then(|| Mark::Spot(seen, take))
    })
}

/// Marks the element it is put in as hiding what is under it. It has to
/// come before the element's other children, which it does not hide.
#[must_use]
pub fn cover() -> impl IntoElement {
    mark(|bounds, _| Some(Mark::Cover(bounds)))
}

/// Marks the element it is put in as `part`, for `f`.
#[must_use]
pub fn part(part: Part) -> impl IntoElement {
    mark(move |bounds, _| Some(Mark::Part(part, bounds)))
}

/// Marks the element it is put in as showing `page`, whose hints join
/// the window's.
#[must_use]
pub fn page(page: Rc<dyn Page>) -> impl IntoElement {
    mark(move |_, _| Some(Mark::Page(page)))
}

/// What can be clicked, labelled in hint mode.
pub trait Hinted: InteractiveElement + ParentElement + Sized {
    /// Clicks with `listener`, as `on_click` does, and is labelled in
    /// hint mode, which clicks it so.
    #[must_use]
    fn on_hint_click(self, listener: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self
    where
        Self: StatefulInteractiveElement,
    {
        let listener = Rc::new(listener);
        let clicked = listener.clone();
        let click: Click = Rc::new(move |bounds, window, cx| {
            let event = ClickEvent::Keyboard(KeyboardClickEvent {
                button: KeyboardButton::Enter,
                bounds,
            });
            clicked(&event, window, cx);
        });
        self.child(spot(Take::Click(click)))
            .on_click(move |event, window, cx| listener(event, window, cx))
    }

    /// Takes the left button's press with `listener`, as `on_mouse_down`
    /// does, and is labelled in hint mode, which presses it so, in its
    /// middle.
    #[must_use]
    fn on_hint_mouse_down(
        self,
        listener: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let listener = Rc::new(listener);
        let pressed = listener.clone();
        let click: Click = Rc::new(move |bounds, window, cx| {
            let event = MouseDownEvent {
                button: MouseButton::Left,
                position: bounds.center(),
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            };
            pressed(&event, window, cx);
        });
        self.child(spot(Take::Click(click))).on_mouse_down(
            MouseButton::Left,
            move |event, window, cx| {
                listener(event, window, cx);
            },
        )
    }
}

impl<E: InteractiveElement + ParentElement> Hinted for E {}

/// What a hint is over.
#[derive(Clone)]
enum Target {
    Spot(Bounds<Pixels>, Take),
    Page(Rc<dyn Page>, u32),
}

enum State {
    /// The pages were asked for their hints, and this many have not yet
    /// answered.
    Asking(usize),
    /// The hints are labelled, and this much of a label is typed.
    Shown(String),
}

/// The hints over a window.
struct Hints {
    window: WindowId,
    /// The pages asked for their hints.
    pages: Vec<Rc<dyn Page>>,
    targets: Vec<Target>,
    labels: Vec<String>,
    state: State,
}

/// What the window's parts said of themselves in its last frame, in
/// `part`, or the whole window's, `None`: what is in sight and not
/// under what was drawn after it, and the pages shown.
fn targets(part: Option<Part>, window: &Window) -> (Vec<Target>, Vec<Rc<dyn Page>>) {
    let id = window_id(window);
    let marks: Vec<Mark> = MARKS.with(|marks| {
        marks
            .borrow()
            .iter()
            .filter(|(window, _)| *window == id)
            .map(|(_, mark)| mark.clone())
            .collect()
    });
    let within: Vec<Bounds<Pixels>> = part
        .map(|part| {
            marks
                .iter()
                .filter_map(|mark| match mark {
                    Mark::Part(marked, bounds) if *marked == part => Some(*bounds),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut targets = Vec::new();
    for (at, mark) in marks.iter().enumerate() {
        let Mark::Spot(bounds, take) = mark else {
            continue;
        };
        let middle = bounds.center();
        let covered = marks[at + 1..]
            .iter()
            .any(|mark| matches!(mark, Mark::Cover(cover) if cover.contains(&middle)));
        let inside = part.is_none() || within.iter().any(|bounds| bounds.contains(&middle));
        if inside && !covered {
            targets.push(Target::Spot(*bounds, take.clone()));
        }
    }
    let pages = if part.is_none() {
        marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Page(page) => Some(page.clone()),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    (targets, pages)
}

/// Draws hints over what can be clicked in `part`, or in the whole
/// window, the page shown's too, for hint mode: false if there is
/// nothing to hint.
pub fn start(part: Option<Part>, window: &mut Window, cx: &mut App) -> bool {
    stop(cx);
    let (targets, pages) = targets(part, window);
    if targets.is_empty() && pages.is_empty() {
        return false;
    }
    let id = window_id(window);
    HINTS.with(|hints| {
        *hints.borrow_mut() = Some(Hints {
            window: id,
            pages: pages.clone(),
            targets,
            labels: Vec::new(),
            state: State::Asking(pages.len()),
        });
    });
    for page in pages {
        let asked = page.clone();
        page.ask(
            Box::new(move |ids, window, cx| {
                let pending = HINTS.with(|hints| {
                    let mut hints = hints.borrow_mut();
                    let hints = hints.as_mut()?;
                    let State::Asking(waiting) = &mut hints.state else {
                        return None;
                    };
                    hints
                        .targets
                        .extend(ids.into_iter().map(|id| Target::Page(asked.clone(), id)));
                    *waiting = waiting.saturating_sub(1);
                    Some(*waiting)
                });
                if pending == Some(0) {
                    window.defer(cx, label);
                }
            }),
            cx,
        );
    }
    label(window, cx);
    true
}

/// Labels the hints, once every page answered, or leaves hint mode if
/// there are none.
fn label(window: &mut Window, cx: &mut App) {
    let labelled = HINTS.with(|hints| {
        let mut hints = hints.borrow_mut();
        let hints = hints.as_mut()?;
        if !matches!(hints.state, State::Asking(0)) {
            return None;
        }
        hints.labels = labels(hints.targets.len());
        hints.state = State::Shown(String::new());
        Some(hints.targets.is_empty())
    });
    match labelled {
        Some(true) => {
            stop(cx);
            super::enter(super::Mode::Normal, cx);
        }
        Some(false) => show(cx),
        None => return,
    }
    window.refresh();
}

/// The labels that match what was typed, with what they are over.
fn matching(hints: &Hints) -> impl Iterator<Item = (&String, &Target)> {
    let typed = match &hints.state {
        State::Shown(typed) => Some(typed.as_str()),
        State::Asking(_) => None,
    };
    hints
        .labels
        .iter()
        .zip(&hints.targets)
        .filter(move |(label, _)| typed.is_some_and(|typed| label.starts_with(typed)))
}

/// Draws the labels that match over the pages.
fn show(cx: &mut App) {
    let pages = HINTS.with(|hints| {
        let hints = hints.borrow();
        let Some(hints) = hints.as_ref() else {
            return Vec::new();
        };
        hints
            .pages
            .iter()
            .map(|page| {
                let labels: Vec<(u32, String)> = matching(hints)
                    .filter_map(|(label, target)| match target {
                        Target::Page(shown, id) if Rc::ptr_eq(shown, page) => {
                            Some((*id, label.clone()))
                        }
                        _ => None,
                    })
                    .collect();
                (page.clone(), labels)
            })
            .collect()
    });
    for (page, labels) in pages {
        page.show(&labels, cx);
    }
}

/// Whether hints are over the window, or being asked for.
#[must_use]
pub fn shown() -> bool {
    HINTS.with(|hints| hints.borrow().is_some())
}

/// Takes the hints away, from the pages too.
pub fn stop(cx: &mut App) {
    let Some(hints) = HINTS.with(|hints| hints.borrow_mut().take()) else {
        return;
    };
    for page in hints.pages {
        page.clear(cx);
    }
}

/// Types `key` toward a label: the hint left alone is taken, in normal
/// mode, or in insert mode for a field, and none left leaves hint mode.
pub fn typed(key: char, window: &mut Window, cx: &mut App) {
    let typed = HINTS.with(|hints| {
        let mut hints = hints.borrow_mut();
        let hints = hints.as_mut()?;
        let State::Shown(typed) = &mut hints.state else {
            return None;
        };
        typed.push(key);
        let mut left = matching(hints).map(|(_, target)| target.clone());
        match (left.next(), left.next()) {
            (Some(target), None) => Some(Some(target)),
            (None, _) => Some(None),
            _ => None,
        }
    });
    match typed {
        None => show(cx),
        Some(chosen) => {
            super::enter(super::Mode::Normal, cx);
            if let Some(target) = chosen {
                take(target, window, cx);
            }
            stop(cx);
        }
    }
    window.refresh();
}

fn take(target: Target, window: &mut Window, cx: &mut App) {
    match target {
        Target::Spot(bounds, Take::Click(click)) => click(bounds, window, cx),
        Target::Spot(_, Take::Field(focus)) => {
            window.focus(&focus);
            super::enter(super::Mode::Insert, cx);
        }
        Target::Page(page, id) => page.take(id, window, cx),
    }
}

/// The labels that match what was typed over `window`, but those a page
/// draws, with what each is over.
#[must_use]
pub fn shown_labels(window: &Window) -> Vec<(String, Bounds<Pixels>)> {
    let id = window_id(window);
    HINTS.with(|hints| {
        let hints = hints.borrow();
        let Some(hints) = hints.as_ref().filter(|hints| hints.window == id) else {
            return Vec::new();
        };
        matching(hints)
            .filter_map(|(label, target)| match target {
                Target::Spot(bounds, _) => Some((label.clone(), *bounds)),
                Target::Page(..) => None,
            })
            .collect()
    })
}

/// The labels over `window`, over all else, while it has hints, as the
/// pages draw theirs.
#[must_use]
pub fn overlay(window: &Window) -> Option<AnyElement> {
    let labels = shown_labels(window);
    if labels.is_empty() {
        return None;
    }
    Some(
        deferred(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .children(labels.iter().map(|(label, over)| tag(label, *over))),
        )
        .with_priority(usize::MAX)
        .into_any_element(),
    )
}

/// A label, as the pages draw theirs, at the top left of what it is over.
fn tag(label: &str, over: Bounds<Pixels>) -> AnyElement {
    let selector = format!("hint-{label}");
    div()
        .debug_selector(move || selector)
        .absolute()
        .left(over.origin.x)
        .top(over.origin.y)
        .px(px(2.))
        .border_1()
        .border_color(rgb(0x00aa_8800))
        .rounded(px(2.))
        .bg(rgb(0x00ff_dd00))
        .text_color(rgb(0x0000_0000))
        .font_family(crate::ui::theme::CODE_FONT)
        .font_weight(gpui::FontWeight::BOLD)
        .text_size(px(11.))
        .line_height(px(13.))
        .child(label.to_uppercase())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::labels;

    #[test]
    fn labels_are_as_long_as_they_need_and_none_starts_another() {
        assert_eq!(labels(3), ["s", "a", "d"]);
        assert_eq!(labels(14).len(), 14);
        assert!(labels(14).iter().all(|label| label.len() == 1));
        let many = labels(15);
        assert_eq!(&many[..2], ["ss", "sa"]);
        assert_eq!(many[14], "as");
        let unique: std::collections::HashSet<_> = labels(200).into_iter().collect();
        assert_eq!(unique.len(), 200);
        assert!(labels(200).iter().all(|label| label.len() == 3));
        assert_eq!(labels(0), Vec::<String>::new());
    }
}
