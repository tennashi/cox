//! The keys as in Vim, which the human turns on and off as Cox runs, on
//! the settings screen or in `settings.json` with `{"vim": true}`: a
//! mode, normal or insert, in the chat and what opens over it to type
//! in, and in a page; hint mode, which picks what to click in the window
//! by its label (see [`hints`]); and the keys of each (see [`keymap`]).
//!
//! The keys are read ahead of the window's own, in one place
//! ([`turn`]), which takes those Vim's keys do and leaves the rest as
//! they were. Where the keyboard is, its region, is told by the window's
//! key contexts. The stage's tabs keep every key of their own. While an
//! input method composes, every key is its own.

pub(crate) mod cursor;
pub mod ex;
pub mod hints;
pub mod keymap;
pub mod line;
mod region;
pub mod search;

use gpui::{
    Action, App, Context, FocusHandle, Global, Keystroke, KeystrokeEvent, ListState, Pixels,
    ScrollHandle, Subscription, Window, actions, point, px,
};

pub use self::region::Region;

use self::keymap::{
    Across, Act, Answer, Command, Edit, Follow, Keymap, Motion, Place, Reach, Selects, Sent, Sight,
};
use self::region::Kind;
use self::search::{Direction, Find, Search};

/// Moves what the part of the window with the keyboard shows, or its
/// cursor, as Vim's keys ask: an action each part that moves so handles
/// as it can, and the rest leave.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct Move {
    pub motion: Motion,
    /// The count typed before the keys, as `3` of `3j`.
    pub count: Option<u32>,
}

/// Moves the cursor of code across its text, as Vim's keys ask: an
/// action only code handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct MoveAcross {
    pub across: Across,
    /// The count typed before the keys, as `3` of `3w`.
    pub count: Option<u32>,
}

/// Does to the text of the input with the keyboard what normal mode
/// asks: an action only an input handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct EditText {
    pub edit: Edit,
    /// The count typed before the keys, as `3` of `3x`.
    pub count: Option<u32>,
}

impl EditText {
    /// How many times the edit goes: the count, or once.
    #[must_use]
    pub fn times(&self) -> usize {
        self.count
            .map_or(1, |count| usize::try_from(count).unwrap_or(usize::MAX))
    }
}

impl Move {
    /// How many times the motion goes: the count, or once.
    #[must_use]
    pub fn times(&self) -> usize {
        self.count
            .map_or(1, |count| usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// The line, from 0, the count says, as `12` of `12G`, if typed.
    #[must_use]
    pub fn line(&self) -> Option<usize> {
        self.count.map(|_| self.times().saturating_sub(1))
    }

    /// Where a cursor or selection at `at` goes, among lines or rows up to
    /// `last`, `half` of them in half the part.
    #[must_use]
    pub fn moved(&self, at: usize, half: usize, last: usize) -> usize {
        let times = self.times();
        let half = half.max(1).saturating_mul(times);
        match self.motion {
            Motion::Down => at.saturating_add(times),
            Motion::Up => at.saturating_sub(times),
            Motion::HalfDown => at.saturating_add(half),
            Motion::HalfUp => at.saturating_sub(half),
            Motion::Top => self.line().unwrap_or(0),
            Motion::Bottom => self.line().unwrap_or(last),
        }
        .min(last)
    }

    /// How far down a part with no cursor is scrolled once it moves: from
    /// `at`, as far as `most`, its lines `line` tall, `height` in sight.
    #[must_use]
    pub fn scrolled(&self, at: Pixels, most: Pixels, line: Pixels, height: Pixels) -> Pixels {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a count a human types is far below what f32 keeps exactly"
        )]
        let times = self.times().min(1 << 20) as f32;
        let to = match self.motion {
            Motion::Down => at + line * times,
            Motion::Up => at - line * times,
            Motion::HalfDown => at + height / 2. * times,
            Motion::HalfUp => at - height / 2. * times,
            Motion::Top => px(0.),
            Motion::Bottom => most,
        };
        to.clamp(px(0.), most.max(px(0.)))
    }

    /// Scrolls what `scroll` tracks, its lines `line` tall, as the motion
    /// asks.
    pub fn scroll(&self, scroll: &ScrollHandle, line: Pixels) {
        let offset = scroll.offset();
        let height = scroll.bounds().size.height;
        let most = scroll.max_offset().height;
        let to = self.scrolled(-offset.y, most, line, height);
        scroll.set_offset(point(offset.x, -to));
    }

    /// Scrolls the list `list` keeps, its lines `line` tall, as the motion
    /// asks.
    pub fn scroll_list(&self, list: &ListState, line: Pixels) {
        let at = -list.scroll_px_offset_for_scrollbar().y;
        let most = list.max_offset_for_scrollbar().height;
        let height = list.viewport_bounds().size.height;
        list.scroll_by(self.scrolled(at, most, line, height) - at);
    }
}

/// Gives the keyboard back to the input beside the part that has it, to
/// type in, as `place` says: from the conversation, `i` and `a`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct TypeInInput {
    pub place: Place,
}

/// Does what the window does for Vim's keys, pressed in `region` after
/// `count`: an action the window handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct DoInWindow {
    pub act: Act,
    pub region: Region,
    pub count: Option<u32>,
}

/// Draws hints over what can be clicked in the page with the keyboard,
/// for hint mode.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct Hint {
    pub follow: Follow,
}

/// Finds the whole word under the cursor of code, as `direction` goes,
/// which becomes the last search (`*`, `#`).
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct FindWord {
    pub direction: Direction,
}

/// Scrolls code so the cursor's line is where `sight` says.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct ScrollTo {
    pub sight: Sight,
}

/// A key typed in hint mode, toward the label of a hint.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct Label {
    pub key: char,
}

actions!(
    vim,
    [
        /// Starts selecting whole lines, from the cursor's, or stops.
        Visual,
        /// Visual mode is left: what it selected is let go of.
        LeaveVisual,
        /// Starts selecting characters, from the cursor's, or stops.
        VisualCharacters,
        /// Goes to where the first name of the cursor's line is defined.
        Definition,
        /// Lists where the name under the cursor is used.
        References,
        /// Shows what the language server says of the name under the
        /// cursor, at once.
        LookUp,
        /// Opens what is selected.
        Open,
        /// Closes the folder selected, or selects the folder it is in.
        Close,
        /// The keys went back to normal mode: what was typed in leaves the
        /// keyboard, and hints go.
        Leave,
        /// Loads the page again.
        Reload,
        /// Gives the keyboard to the page's first field.
        FocusField,
        /// Copies the page's address.
        CopyAddress,
        /// Takes the keyboard to the page's address line.
        Address,
        /// Selects the next entry of the conversation, or block of a
        /// document (`]]`).
        NextEntry,
        /// Selects the entry, or block, before (`[[`).
        PreviousEntry,
        /// Selects the next stretch of changed lines, across files (`]c`).
        NextStretch,
        /// Selects the stretch before (`[c`).
        PreviousStretch,
    ]
);

impl Sent {
    /// The action the part with the keyboard does it with.
    fn action(self) -> Box<dyn Action> {
        match self {
            Self::Copy => Box::new(crate::ui::Copy),
            Self::AddToChat => Box::new(crate::chat::hand::AddToChat),
            Self::Back => Box::new(crate::stage::code::Back),
            Self::Forward => Box::new(crate::stage::code::Forward),
            Self::Visual => Box::new(Visual),
            Self::VisualCharacters => Box::new(VisualCharacters),
            Self::Definition => Box::new(Definition),
            Self::References => Box::new(References),
            Self::LookUp => Box::new(LookUp),
            Self::FindWord(direction) => Box::new(FindWord { direction }),
            Self::ScrollTo(sight) => Box::new(ScrollTo { sight }),
            Self::Open => Box::new(Open),
            Self::Close => Box::new(Close),
            Self::Reload => Box::new(Reload),
            Self::FocusField => Box::new(FocusField),
            Self::CopyAddress => Box::new(CopyAddress),
            Self::Address => Box::new(Address),
            Self::Label(key) => Box::new(Label { key }),
            Self::Accept => Box::new(line::Accept),
            Self::NextFit => Box::new(line::NextFit),
            Self::PreviousFit => Box::new(line::PreviousFit),
            Self::Older => Box::new(line::Older),
            Self::Newer => Box::new(line::Newer),
            Self::NextEntry => Box::new(NextEntry),
            Self::PreviousEntry => Box::new(PreviousEntry),
            Self::NextStretch => Box::new(NextStretch),
            Self::PreviousStretch => Box::new(PreviousStretch),
        }
    }

    /// How many times `count` does it: back and forward, and from stretch
    /// to stretch of changes, go as far as the count; the rest are done
    /// once.
    fn times(self, count: Option<u32>) -> u32 {
        match self {
            Self::Back | Self::Forward | Self::NextStretch | Self::PreviousStretch => {
                count.unwrap_or(1)
            }
            _ => 1,
        }
    }
}

/// How the keys work: as Vim's normal mode, where they do things, as
/// its insert mode, where they type, as Vimperator's hint mode, where
/// they type the label of a hint over a page, or as Vim's command-line
/// mode, where they type in the line at the window's foot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Hint,
    Command,
    /// Selecting from where it started to the cursor, which moves as in
    /// normal mode, for an edit or a copy of what is selected.
    Visual(Selects),
}

impl Mode {
    /// How the window shows it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Insert => "Insert",
            Self::Hint => "Hint",
            Self::Command => "Command",
            Self::Visual(Selects::Characters) => "Visual",
            Self::Visual(Selects::Lines) => "Visual Line",
        }
    }

    /// Whether keys do things rather than type: normal and visual mode.
    const fn acts(self) -> bool {
        matches!(self, Self::Normal | Self::Visual(_))
    }
}

/// What the line at the window's foot shows of Vim's keys, as Vim's
/// `showmode` and `showcmd` do: the mode, and the count and keys held
/// toward a command, as `3g` of `3gt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub mode: Mode,
    pub held: String,
}

/// What becomes of a key Vim's keys read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It goes on to the window, as without Vim's keys.
    Pass,
    /// It goes no further.
    Taken,
    /// It does this, with the count typed before it, and goes no further.
    Do(Command, Option<u32>),
}

/// Vim's keys as the human has them now.
#[derive(Debug, Default)]
pub struct Vim {
    mode: Mode,
    /// The count typed before a command, as `3` of `3j`.
    count: Option<u32>,
    /// The keys held as the start of a sequence, as `g` of `gg`.
    held: Vec<String>,
    /// Where the keyboard was at the last key.
    region: Region,
    /// Where the keyboard was when it went to the line at the window's
    /// foot, which gets it back in the mode the line left.
    line_from: Option<Region>,
    /// The last text searched for, which `n` and `N` find again.
    search: Option<Search>,
}

impl Global for Vim {}

impl Vim {
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    #[must_use]
    pub const fn count(&self) -> Option<u32> {
        self.count
    }

    /// What the line at the window's foot shows of the keys.
    #[must_use]
    pub fn status(&self) -> Status {
        Status {
            mode: self.mode,
            held: self
                .count
                .map(|count| count.to_string())
                .into_iter()
                .chain(self.held.iter().cloned())
                .collect(),
        }
    }

    fn reset(&mut self) {
        self.count = None;
        self.held.clear();
    }

    /// The keyboard is in `region`: coming in, the count and the keys
    /// held go, and the mode is the region's, unless the keyboard is back
    /// from the line, which left it in normal mode.
    fn arrive(&mut self, region: Region) {
        if region == self.region {
            return;
        }
        let back = self.region == Region::Command && self.line_from == Some(region);
        if region == Region::Command {
            self.line_from = Some(self.region);
        }
        self.region = region;
        self.reset();
        if matches!(self.mode, Mode::Hint | Mode::Command | Mode::Visual(_)) {
            self.mode = Mode::Normal;
        }
        if let Some(mode) = region.props().enters_in.filter(|_| !back) {
            self.mode = mode;
        }
    }

    pub fn key(&mut self, keymap: &Keymap, region: Region, keystroke: &Keystroke) -> Outcome {
        self.arrive(region);
        if region.is(Kind::Modeless) && self.mode != Mode::Insert {
            self.mode = Mode::Insert;
        }
        if !keymap.knows(region) {
            return Outcome::Pass;
        }
        if keystroke.modifiers.platform {
            self.reset();
            return Outcome::Pass;
        }
        let key = keystroke.unparse();
        if self.mode == Mode::Hint && key != "escape" {
            return label_key(keystroke).map_or(Outcome::Taken, |key| {
                Outcome::Do(Command::Send(Sent::Label(key)), None)
            });
        }
        if self.mode.acts() && self.held.is_empty() {
            if key == "escape" && self.count.is_some() {
                self.reset();
                return Outcome::Taken;
            }
            if let Some(digit) = self.count_digit(keystroke) {
                self.count = Some(self.count.unwrap_or(0).saturating_mul(10) + digit);
                return Outcome::Taken;
            }
        }
        let mut keys: Vec<&str> = self.held.iter().map(String::as_str).collect();
        keys.push(&key);
        match keymap.answer(self.mode, region, &keys) {
            Answer::Act(command) => {
                let count = self.count;
                self.reset();
                if let Some(visual) = command.selects().filter(|_| region.is(Kind::Visual)) {
                    self.mode = if self.mode == Mode::Visual(visual) {
                        Mode::Normal
                    } else {
                        Mode::Visual(visual)
                    };
                    return Outcome::Do(command, count);
                }
                let selecting = matches!(self.mode, Mode::Visual(_));
                match command {
                    Command::Normal => self.mode = Mode::Normal,
                    Command::Insert(_) | Command::Edit(Edit::Change(_)) => {
                        self.mode = Mode::Insert;
                    }
                    Command::Hint(_) | Command::Hints(_) => self.mode = Mode::Hint,
                    Command::Stop
                    | Command::Line(_)
                    | Command::Again(_)
                    | Command::Reset
                    | Command::Window(_)
                    | Command::Move(_)
                    | Command::Across(_)
                    | Command::Edit(_)
                    | Command::Send(_) => {
                        // What is selected is edited or copied, and
                        // visual mode is left.
                        if selecting && !command.moves() {
                            self.mode = Mode::Normal;
                        }
                    }
                }
                Outcome::Do(command, count)
            }
            Answer::Held => {
                self.held.push(key);
                Outcome::Taken
            }
            Answer::None => {
                let dropped = !self.held.is_empty();
                self.reset();
                let taken = if region.runs_a_program() {
                    self.mode == Mode::Normal && region.program_gets(keystroke)
                } else {
                    dropped || (self.mode.acts() && edits_text(keystroke))
                };
                if taken { Outcome::Taken } else { Outcome::Pass }
            }
        }
    }

    /// The digit `keystroke` adds to the count, if it does: `0` only after
    /// another.
    fn count_digit(&self, keystroke: &Keystroke) -> Option<u32> {
        if keystroke.modifiers.modified() {
            return None;
        }
        let digit = keystroke
            .key
            .parse::<u32>()
            .ok()
            .filter(|digit| *digit < 10)?;
        (digit > 0 || self.count.is_some()).then_some(digit)
    }
}

/// The character `keystroke` types toward a hint's label, shifted or not,
/// if it types one.
fn label_key(keystroke: &Keystroke) -> Option<char> {
    let modifiers = &keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    let mut chars = keystroke.key.chars();
    match (chars.next(), chars.next()) {
        (Some(key), None) => Some(key.to_ascii_lowercase()),
        _ => None,
    }
}

/// Whether `keystroke`, gone on to an input, would change its text, which
/// normal mode keeps it from.
fn edits_text(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return false;
    }
    keystroke.key.chars().count() == 1
        || matches!(
            keystroke.key.as_str(),
            "space" | "enter" | "backspace" | "delete"
        )
}

/// The keys are read ahead of the window's own, for Vim's keys while
/// they are on.
struct Reading;

impl Global for Reading {}

/// Turns Vim's keys on, in normal mode, or off, from the next key.
pub fn turn(on: bool, cx: &mut App) {
    if on == cx.has_global::<Vim>() {
        return;
    }
    if on {
        if !cx.has_global::<Reading>() {
            cx.set_global(Reading);
            cx.intercept_keystrokes(intercept).detach();
        }
        cx.set_global(Vim::default());
    } else {
        cx.remove_global::<Vim>();
    }
    cx.refresh_windows();
}

/// Turns Vim's keys on or off while `window` is open, as the human
/// changes the setting: on, in the mode of where its keyboard is.
pub fn switch(on: bool, window: &mut Window, cx: &mut App) {
    if on == cx.has_global::<Vim>() {
        return;
    }
    turn(on, cx);
    if on {
        came(Region::of(&window.context_stack()), window, cx);
    }
}

/// The mode the keys are in, if Vim's keys are on.
#[must_use]
pub fn mode(cx: &App) -> Option<Mode> {
    cx.try_global::<Vim>().map(Vim::mode)
}

/// Whether keys type in the input that has the keyboard: with Vim's keys
/// off, or in insert or command mode. Else the input shows no caret.
#[must_use]
pub fn types(cx: &App) -> bool {
    mode(cx).is_none_or(|mode| matches!(mode, Mode::Insert | Mode::Command))
}

/// What the line at the window's foot shows of Vim's keys, while they
/// are on.
#[must_use]
pub fn status(cx: &App) -> Option<Status> {
    cx.try_global::<Vim>().map(Vim::status)
}

/// The keyboard came to `region`, before any key there: its mode shows
/// at once, if Vim's keys are on.
pub fn came(region: Region, window: &mut Window, cx: &mut App) {
    if !cx.has_global::<Vim>() {
        return;
    }
    let vim = cx.global_mut::<Vim>();
    let was = vim.status();
    vim.arrive(region);
    if vim.status() != was {
        window.refresh();
    }
}

/// Goes to insert mode in `region`, which the human means to type in, as
/// by clicking a terminal or opening one, if Vim's keys are on.
pub fn typing(region: Region, window: &mut Window, cx: &mut App) {
    came(region, window, cx);
    if cx.has_global::<Vim>() && cx.global::<Vim>().mode != Mode::Insert {
        enter(Mode::Insert, cx);
        window.refresh();
    }
}

/// The keyboard came to an input. A screen of settings, or a card, comes
/// in normal mode, though the keyboard is in its input as it opens; an
/// input the human goes to in it is to type in, so the keys go to insert
/// mode, as a browser's Vim keys do in a page's field.
pub fn came_to_field(window: &mut Window, cx: &mut App) {
    let region = Region::of(&window.context_stack());
    if !region.is(Kind::Fields) {
        return;
    }
    if cx
        .try_global::<Vim>()
        .is_some_and(|vim| vim.region == region)
    {
        typing(region, window, cx);
    } else {
        came(region, window, cx);
    }
}

/// Tells Vim's keys each time `focus`, of the part in `region`, gets the
/// keyboard, and now if it has it.
pub fn follow_focus<T: 'static>(
    region: Region,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Subscription {
    let arrived = move |_: &mut T, window: &mut Window, cx: &mut Context<T>| {
        came(region, window, cx);
    };
    if focus.is_focused(window) {
        cx.defer_in(window, arrived);
    }
    cx.on_focus(focus, window, arrived)
}

/// Goes to `mode`, if Vim's keys are on.
pub fn enter(mode: Mode, cx: &mut App) {
    if cx.has_global::<Vim>() {
        cx.global_mut::<Vim>().mode = mode;
    }
}

/// Keeps `search` as the last, for `n` and `N` to find again, if Vim's
/// keys are on.
pub fn searched(search: Search, cx: &mut App) {
    if cx.has_global::<Vim>() {
        cx.global_mut::<Vim>().search = Some(search);
    }
}

/// The last text searched for, if Vim's keys are on.
#[must_use]
pub fn last_search(cx: &App) -> Option<&Search> {
    cx.try_global::<Vim>()?.search.as_ref()
}

fn intercept(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) {
    if !cx.has_global::<Vim>() || crate::ui::composing::any(cx) {
        return;
    }
    let region = Region::of(&event.context_stack);
    let vim = cx.global_mut::<Vim>();
    let was = vim.status();
    let typed = vim.mode == Mode::Insert;
    let selecting = matches!(vim.mode, Mode::Visual(_));
    let outcome = vim.key(&Keymap::VIM, region, &event.keystroke);
    let mode = vim.mode;
    if vim.status() != was {
        window.refresh();
    }
    if mode != Mode::Hint && hints::shown() {
        hints::stop(cx);
        window.refresh();
    }
    match outcome {
        Outcome::Pass => return,
        Outcome::Taken | Outcome::Do(Command::Reset, _) => {}
        Outcome::Do(Command::Normal, _) if selecting => {
            window.dispatch_action(Box::new(LeaveVisual), cx);
        }
        Outcome::Do(Command::Normal, _) => {
            if typed {
                window.dispatch_action(Box::new(crate::ui::input::Settle), cx);
            }
            window.dispatch_action(Box::new(crate::chat::view::CloseList), cx);
            window.dispatch_action(Box::new(Leave), cx);
        }
        Outcome::Do(Command::Hint(follow), _) => {
            window.dispatch_action(Box::new(Hint { follow }), cx);
        }
        Outcome::Do(Command::Hints(reach), _) => {
            let part = match reach {
                Reach::Part => region.props().hints,
                Reach::Window => None,
            };
            let reaches = reach == Reach::Window || part.is_some();
            let started = reaches
                && (hints::start(part, window, cx)
                    || (part.is_some() && hints::start(None, window, cx)));
            if !started {
                enter(Mode::Normal, cx);
            }
        }
        Outcome::Do(Command::Send(Sent::Label(key)), _) if hints::shown() => {
            hints::typed(key, window, cx);
        }
        Outcome::Do(Command::Stop, _) => {
            window.dispatch_action(Box::new(crate::chat::view::Stop), cx);
        }
        Outcome::Do(Command::Window(act), count) => {
            window.dispatch_action(Box::new(DoInWindow { act, region, count }), cx);
        }
        Outcome::Do(Command::Line(prompt), _) => {
            window.dispatch_action(Box::new(line::OpenLine { prompt }), cx);
        }
        Outcome::Do(Command::Again(direction), _) => {
            if let Some(search) = last_search(cx) {
                let find = Find {
                    text: search.text.clone(),
                    direction: search.direction.then(direction),
                    word: search.word,
                };
                window.dispatch_action(Box::new(find), cx);
            }
        }
        Outcome::Do(Command::Move(motion), count) => {
            window.dispatch_action(Box::new(Move { motion, count }), cx);
        }
        Outcome::Do(Command::Across(across), count) => {
            window.dispatch_action(Box::new(MoveAcross { across, count }), cx);
        }
        Outcome::Do(Command::Send(sent), count) => {
            for _ in 0..sent.times(count) {
                window.dispatch_action(sent.action(), cx);
            }
        }
        Outcome::Do(Command::Edit(edit), count) => {
            window.dispatch_action(Box::new(EditText { edit, count }), cx);
        }
        Outcome::Do(Command::Insert(place), _) if region == Region::Reading => {
            window.dispatch_action(Box::new(TypeInInput { place }), cx);
        }
        Outcome::Do(Command::Insert(place), _) => {
            use crate::ui::input::{End, Home, Right};
            match place {
                Place::Cursor => {}
                Place::After => window.dispatch_action(Box::new(Right), cx),
                Place::LineStart => window.dispatch_action(Box::new(Home), cx),
                Place::LineEnd => window.dispatch_action(Box::new(End), cx),
            }
        }
    }
    cx.stop_propagation();
}

#[cfg(test)]
mod tests {
    use gpui::{KeyContext, Keystroke, px};

    use super::keymap::{
        Across, Act, Command, Edit, Follow, Keymap, Motion, Pick, Place, Prompt, Reach, Selects,
        Sent,
    };
    use super::{Mode, Move, Outcome, Region, Vim};

    fn press(vim: &mut Vim, region: Region, keys: &str) -> Vec<Outcome> {
        keys.split(' ')
            .map(|key| vim.key(&Keymap::VIM, region, &Keystroke::parse(key).unwrap()))
            .collect()
    }

    #[test]
    fn the_region_is_the_innermost_context_that_says_one() {
        let stack = |names: &[&str]| -> Vec<KeyContext> {
            names
                .iter()
                .map(|name| KeyContext::parse(name).unwrap())
                .collect()
        };
        assert_eq!(
            Region::of(&stack(&["Workspace", "Conversation", "LineInput"])),
            Region::Chat
        );
        assert_eq!(
            Region::of(&stack(&["Workspace", "Conversation", "Goal", "LineInput"])),
            Region::Goal
        );
        assert_eq!(
            Region::of(&stack(&["Workspace", "Terminal"])),
            Region::Terminal
        );
        assert_eq!(Region::of(&stack(&["Workspace", "Page"])), Region::Page);
        assert_eq!(Region::of(&stack(&["Unknown"])), Region::Workspace);
        assert_eq!(Region::of(&[]), Region::Workspace);
    }

    #[test]
    fn normal_mode_types_nothing_and_leaves_the_window_s_other_keys() {
        let mut vim = Vim::default();
        assert_eq!(
            press(&mut vim, Region::Chat, "z shift-k space enter backspace"),
            [Outcome::Taken; 5]
        );
        assert_eq!(
            press(&mut vim, Region::Chat, "ctrl-tab left tab cmd-n"),
            [Outcome::Pass; 4]
        );
    }

    #[test]
    fn insert_mode_types_every_key_but_escape() {
        let mut vim = Vim::default();
        assert_eq!(
            press(&mut vim, Region::Chat, "shift-a"),
            [Outcome::Do(Command::Insert(Place::LineEnd), None)]
        );
        assert_eq!(vim.mode(), Mode::Insert);
        assert_eq!(
            press(&mut vim, Region::Chat, "j 1 ctrl-c"),
            [Outcome::Pass; 3]
        );
        assert_eq!(
            press(&mut vim, Region::Chat, "escape"),
            [Outcome::Do(Command::Normal, None)]
        );
        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn v_goes_to_visual_mode_selecting_characters() {
        let mut vim = Vim::default();

        press(&mut vim, Region::Chat, "v");

        assert_eq!(vim.mode(), Mode::Visual(Selects::Characters));
        assert_eq!(vim.mode().label(), "Visual");
    }

    #[test]
    fn shift_v_goes_to_visual_mode_selecting_lines() {
        let mut vim = Vim::default();

        press(&mut vim, Region::Chat, "shift-v");

        assert_eq!(vim.mode(), Mode::Visual(Selects::Lines));
        assert_eq!(vim.mode().label(), "Visual Line");
    }

    #[test]
    fn visual_mode_moves_as_normal_mode_does() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Chat, "v");

        assert_eq!(
            press(&mut vim, Region::Chat, "w 2 e"),
            [
                Outcome::Do(Command::Edit(Edit::Move(Across::WordForward)), None),
                Outcome::Taken,
                Outcome::Do(Command::Edit(Edit::Move(Across::WordEnd)), Some(2)),
            ]
        );
        assert_eq!(vim.mode(), Mode::Visual(Selects::Characters));
    }

    #[test]
    fn d_in_visual_mode_takes_out_what_is_selected_and_goes_back_to_normal() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Chat, "v");

        press(&mut vim, Region::Chat, "d");

        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn c_in_visual_mode_goes_on_to_type() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Chat, "v");

        press(&mut vim, Region::Chat, "c");

        assert_eq!(vim.mode(), Mode::Insert);
    }

    #[test]
    fn i_types_nothing_in_visual_mode_and_escape_leaves_it() {
        let mut vim = Vim::default();

        assert_eq!(
            press(&mut vim, Region::Code, "v i escape"),
            [
                Outcome::Do(Command::Send(Sent::VisualCharacters), None),
                Outcome::Taken,
                Outcome::Do(Command::Normal, None),
            ]
        );
        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn a_count_is_taken_and_escape_forgets_it_before_it_does_anything() {
        let mut vim = Vim::default();
        assert_eq!(press(&mut vim, Region::Chat, "1 0 2"), [Outcome::Taken; 3]);
        assert_eq!(vim.count(), Some(102));
        assert_eq!(press(&mut vim, Region::Chat, "escape"), [Outcome::Taken]);
        assert_eq!(vim.count(), None);
        assert_eq!(
            press(&mut vim, Region::Chat, "escape"),
            [Outcome::Do(Command::Reset, None)]
        );
        assert_eq!(
            press(&mut vim, Region::Chat, "0"),
            [Outcome::Do(
                Command::Edit(Edit::Move(Across::LineStart)),
                None
            )],
            "0 with no count goes to the line's start"
        );
        assert_eq!(vim.count(), None);
    }

    #[test]
    fn escape_in_normal_mode_goes_on_to_close_what_opened_over_the_chat() {
        let mut vim = Vim::default();
        assert_eq!(
            press(&mut vim, Region::Goal, "escape escape"),
            [Outcome::Do(Command::Normal, None), Outcome::Pass]
        );
        assert_eq!(press(&mut vim, Region::Settings, "escape"), [Outcome::Pass]);
        assert_eq!(
            press(&mut vim, Region::ProjectCard, "escape"),
            [Outcome::Pass]
        );
    }

    #[test]
    fn a_new_region_forgets_the_count_and_may_change_the_mode() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Chat, "3");
        assert_eq!(vim.count(), Some(3));
        assert_eq!(press(&mut vim, Region::Renaming, "x"), [Outcome::Pass]);
        assert_eq!(vim.count(), None);
        assert_eq!(vim.mode(), Mode::Insert);
        press(&mut vim, Region::Chat, "escape");
        press(&mut vim, Region::Settings, "x");
        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn a_key_with_command_goes_on_and_forgets_the_count() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Chat, "4");
        assert_eq!(press(&mut vim, Region::Chat, "cmd-n"), [Outcome::Pass]);
        assert_eq!(vim.count(), None);
    }

    #[test]
    fn a_program_has_every_key_in_insert_mode_and_none_in_normal_mode() {
        for region in [Region::Terminal, Region::Android] {
            let mut vim = Vim::default();
            vim.arrive(region);
            assert_eq!(vim.mode(), Mode::Normal, "come to, {region:?}");
            vim.mode = Mode::Insert;
            assert_eq!(
                press(&mut vim, region, "j 1 escape ctrl-c ctrl-w g t :"),
                [Outcome::Pass; 8],
                "{region:?}"
            );
            assert_eq!(vim.mode(), Mode::Insert, "{region:?}");
            assert_eq!(
                press(&mut vim, region, "ctrl-\\ x ctrl-\\ ctrl-\\"),
                [Outcome::Taken, Outcome::Pass, Outcome::Taken, Outcome::Pass],
                "a key after ctrl-\\ goes on, itself too"
            );
            assert_eq!(
                press(&mut vim, region, "ctrl-\\ ctrl-n"),
                [Outcome::Taken, Outcome::Do(Command::Normal, None)]
            );
            assert_eq!(vim.mode(), Mode::Normal);
            assert_eq!(
                press(&mut vim, region, "x escape enter up ctrl-\\ ctrl-n"),
                [
                    Outcome::Taken,
                    Outcome::Taken,
                    Outcome::Taken,
                    Outcome::Taken,
                    Outcome::Taken,
                    Outcome::Do(Command::Normal, None)
                ],
                "{region:?}"
            );
            let control_c = if region == Region::Android {
                Outcome::Pass
            } else {
                Outcome::Taken
            };
            assert_eq!(press(&mut vim, region, "ctrl-c"), [control_c], "{region:?}");
            assert_eq!(press(&mut vim, region, "cmd-v"), [Outcome::Pass]);
            assert_eq!(
                press(&mut vim, region, "i"),
                [Outcome::Do(Command::Insert(Place::Cursor), None)]
            );
            assert_eq!(vim.mode(), Mode::Insert);
        }
    }

    #[test]
    fn back_from_the_line_a_program_is_in_normal_mode_and_a_line_s_input_in_insert_mode() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Terminal, "ctrl-\\ ctrl-n :");
        vim.mode = Mode::Command;
        press(&mut vim, Region::Command, "escape");
        vim.arrive(Region::Terminal);
        assert_eq!(vim.mode(), Mode::Normal);
        vim.arrive(Region::Chat);
        vim.arrive(Region::Terminal);
        assert_eq!(vim.mode(), Mode::Normal, "come to anew");
        press(&mut vim, Region::Command, "escape");
        vim.arrive(Region::Goal);
        assert_eq!(vim.mode(), Mode::Insert, "the line's command opened it");
    }

    #[test]
    fn a_count_goes_with_what_the_window_does() {
        let mut vim = Vim::default();
        assert_eq!(
            press(&mut vim, Region::Chat, "3 g t"),
            [
                Outcome::Taken,
                Outcome::Taken,
                Outcome::Do(Command::Window(Act::NextTab), Some(3))
            ]
        );
        assert_eq!(
            press(&mut vim, Region::Rail, "j"),
            [Outcome::Do(Command::Window(Act::Pick(Pick::Down)), None)]
        );
        assert_eq!(vim.count(), None);
    }

    #[test]
    fn a_count_goes_with_a_motion_and_back() {
        let mut vim = Vim::default();
        assert_eq!(
            press(&mut vim, Region::Code, "1 2 shift-g"),
            [
                Outcome::Taken,
                Outcome::Taken,
                Outcome::Do(Command::Move(Motion::Bottom), Some(12))
            ]
        );
        assert_eq!(
            press(&mut vim, Region::Code, "2 ctrl-o"),
            [
                Outcome::Taken,
                Outcome::Do(Command::Send(Sent::Back), Some(2))
            ]
        );
        assert_eq!(Sent::Back.times(Some(2)), 2);
        assert_eq!(Sent::Copy.times(Some(2)), 1);
    }

    #[test]
    fn a_motion_scrolls_within_the_part() {
        let scrolled = |motion, count, at| {
            Move { motion, count }.scrolled(px(at), px(1000.), px(20.), px(300.))
        };
        assert_eq!(scrolled(Motion::Down, Some(3), 100.), px(160.));
        assert_eq!(scrolled(Motion::Up, None, 10.), px(0.));
        assert_eq!(scrolled(Motion::HalfDown, Some(2), 0.), px(300.));
        assert_eq!(scrolled(Motion::HalfUp, None, 500.), px(350.));
        assert_eq!(scrolled(Motion::Bottom, None, 0.), px(1000.));
        assert_eq!(scrolled(Motion::Top, Some(4), 500.), px(0.));
        assert_eq!(scrolled(Motion::Down, Some(u32::MAX), 0.), px(1000.));
        assert_eq!(
            Move {
                motion: Motion::Bottom,
                count: Some(12)
            }
            .line(),
            Some(11)
        );
        assert_eq!(
            Move {
                motion: Motion::Top,
                count: None
            }
            .line(),
            None
        );
        let moved = |motion, count, at| Move { motion, count }.moved(at, 5, 20);
        assert_eq!(moved(Motion::Down, Some(3), 4), 7);
        assert_eq!(moved(Motion::Up, Some(9), 4), 0);
        assert_eq!(moved(Motion::HalfDown, Some(2), 4), 14);
        assert_eq!(moved(Motion::HalfDown, Some(9), 4), 20);
        assert_eq!(moved(Motion::Top, Some(3), 9), 2);
        assert_eq!(moved(Motion::Bottom, None, 9), 20);
    }

    #[test]
    fn the_rail_and_the_quiet_tabs_are_in_normal_mode() {
        for region in [Region::Rail, Region::List, Region::Code, Region::Tree] {
            let mut vim = Vim::default();
            press(&mut vim, Region::Chat, "i");
            assert_eq!(vim.mode(), Mode::Insert);
            assert_eq!(press(&mut vim, region, "x"), [Outcome::Taken], "{region:?}");
            assert_eq!(vim.mode(), Mode::Normal, "{region:?}");
        }
    }

    #[test]
    fn a_page_keeps_its_mode_as_the_keyboard_comes_in_and_its_field_says_it() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Chat, "i");
        assert_eq!(press(&mut vim, Region::Page, "x"), [Outcome::Pass]);
        assert_eq!(vim.mode(), Mode::Insert);
        vim.mode = Mode::Normal;
        assert_eq!(
            press(&mut vim, Region::Page, "3 j"),
            [
                Outcome::Taken,
                Outcome::Do(Command::Move(Motion::Down), Some(3))
            ]
        );
        assert_eq!(
            press(&mut vim, Region::Page, "cmd-l cmd-c"),
            [Outcome::Pass; 2]
        );
        assert_eq!(
            press(&mut vim, Region::Address, "x"),
            [Outcome::Pass],
            "the address line is typed in"
        );
        assert_eq!(vim.mode(), Mode::Insert);
        vim.mode = Mode::Normal;
        assert_eq!(
            press(&mut vim, Region::Address, "i x"),
            [Outcome::Pass; 2],
            "the address line has no normal mode"
        );
        assert_eq!(
            press(&mut vim, Region::Address, "ctrl-n ctrl-p up down enter"),
            [Outcome::Pass; 5],
            "the address line's list takes its keys"
        );
        assert_eq!(vim.mode(), Mode::Insert);
    }

    #[test]
    fn a_page_s_field_takes_every_key_but_escape() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Page, "x");
        vim.mode = Mode::Insert;
        assert_eq!(
            press(&mut vim, Region::Page, "j f g g ctrl-w shift-h 1 enter"),
            [Outcome::Pass; 8]
        );
        assert_eq!(
            press(&mut vim, Region::Page, "escape"),
            [Outcome::Do(Command::Normal, None)]
        );
        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn hint_mode_types_labels_until_escape_or_the_keyboard_leaves() {
        let mut vim = Vim::default();
        assert_eq!(
            press(&mut vim, Region::Page, "f"),
            [Outcome::Do(Command::Hint(Follow::Here), None)]
        );
        assert_eq!(vim.mode(), Mode::Hint);
        assert_eq!(Mode::Hint.label(), "Hint");
        let label = |key| Outcome::Do(Command::Send(Sent::Label(key)), None);
        assert_eq!(
            press(&mut vim, Region::Page, "j shift-k 1"),
            [label('j'), label('k'), label('1')]
        );
        assert_eq!(
            press(&mut vim, Region::Page, "ctrl-w enter"),
            [Outcome::Taken; 2]
        );
        assert_eq!(
            press(&mut vim, Region::Page, "escape"),
            [Outcome::Do(Command::Normal, None)]
        );
        assert_eq!(vim.mode(), Mode::Normal);
        press(&mut vim, Region::Page, "; t");
        assert_eq!(vim.mode(), Mode::Hint);
        press(&mut vim, Region::Page, "escape");
        assert_eq!(
            press(&mut vim, Region::Rail, "shift-f"),
            [Outcome::Do(Command::Hints(Reach::Window), None)]
        );
        assert_eq!(vim.mode(), Mode::Hint);
        press(&mut vim, Region::Chat, "x");
        assert_eq!(vim.mode(), Mode::Normal);
    }

    #[test]
    fn the_line_types_every_key_but_its_own_and_its_mode_goes_as_it_closes() {
        let mut vim = Vim::default();
        assert_eq!(
            press(&mut vim, Region::Code, ":"),
            [Outcome::Do(Command::Line(Prompt::Ex), None)]
        );
        assert_eq!(vim.mode(), Mode::Normal, "the line says when it opens");
        assert_eq!(
            press(&mut vim, Region::Command, "q j 1 ctrl-w"),
            [Outcome::Pass; 4]
        );
        assert_eq!(vim.mode(), Mode::Command);
        assert_eq!(Mode::Command.label(), "Command");
        assert_eq!(
            press(&mut vim, Region::Command, "tab enter"),
            [
                Outcome::Do(Command::Send(Sent::NextFit), None),
                Outcome::Do(Command::Send(Sent::Accept), None)
            ]
        );
        assert_eq!(
            press(&mut vim, Region::Code, "j"),
            [Outcome::Do(Command::Move(Motion::Down), None)]
        );
        assert_eq!(vim.mode(), Mode::Normal);
        press(&mut vim, Region::Command, "x");
        assert_eq!(
            press(&mut vim, Region::Command, "escape"),
            [Outcome::Do(Command::Normal, None)]
        );
        assert_eq!(vim.mode(), Mode::Normal);
    }
}
