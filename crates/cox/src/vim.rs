//! The keys as in Vim, which the human turns on and off as Cox runs, on
//! the settings screen or in `settings.json` with `{"vim": true}`: a
//! mode, normal or insert, in the chat and what opens over it to type
//! in, and in a page, where hint mode picks what to click by its label,
//! and the keys of each (see [`keymap`]).
//!
//! The keys are read ahead of the window's own, in one place
//! ([`turn`]), which takes those Vim's keys do and leaves the rest as
//! they were. Where the keyboard is, its region, is told by the window's
//! key contexts. The stage's tabs keep every key of their own. While an
//! input method composes, every key is its own.

pub mod ex;
pub mod keymap;
pub mod line;
pub mod search;

use gpui::{
    Action, App, Context, FocusHandle, Global, KeyContext, Keystroke, KeystrokeEvent, Pixels,
    Subscription, Window, actions, px,
};

use self::keymap::{Answer, Command, Follow, Keymap, Motion, Place, Sent};
use self::search::{Find, Search};

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
}

/// Draws hints over what can be clicked in the page with the keyboard,
/// for hint mode.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct Hint {
    pub follow: Follow,
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
        /// Goes to where the first name of the cursor's line is defined.
        Definition,
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
            Self::Definition => Box::new(Definition),
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
        }
    }

    /// How many times `count` does it: back and forward go as far as the
    /// count, the rest are done once.
    fn times(self, count: Option<u32>) -> u32 {
        match self {
            Self::Back | Self::Forward => count.unwrap_or(1),
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
}

impl Mode {
    /// How the window shows it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Hint => "HINT",
            Self::Command => "COMMAND",
        }
    }
}

/// Where the keyboard is, as far as Vim's keys tell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Region {
    /// The window, with no part of it holding the keyboard.
    #[default]
    Workspace,
    /// The rail of projects down the window's left side.
    Rail,
    /// The list of the chosen project's chats, over the chat.
    List,
    /// A chat's input.
    Chat,
    /// The input that sets a chat's goal.
    Goal,
    /// The input that starts a sketch of a chat.
    Sketch,
    /// The input that renames a chat.
    Renaming,
    Settings,
    ProjectCard,
    /// The translation of what was selected.
    Translation,
    /// The sheet a region of the window is dragged on.
    Sheet,
    /// The menu of the stage's +.
    StageMenu,
    Page,
    /// A page's address line.
    Address,
    Terminal,
    Code,
    Document,
    Tree,
    Changes,
    Android,
    Devices,
    Flow,
    Windows,
    MacWindow,
    /// The line at the window's foot, which `:` and `/` open.
    Command,
}

/// The key contexts that say the region the keyboard is in.
const REGIONS: &[(&str, Region)] = &[
    ("Workspace", Region::Workspace),
    (crate::workspace::RAIL, Region::Rail),
    (crate::workspace::LIST, Region::List),
    (crate::chat::view::KEY_CONTEXT, Region::Chat),
    (crate::chat::view::GOAL, Region::Goal),
    (crate::chat::view::SKETCH, Region::Sketch),
    (crate::workspace::RENAMING, Region::Renaming),
    (crate::settings::view::KEY_CONTEXT, Region::Settings),
    (
        crate::workspace::project_card::KEY_CONTEXT,
        Region::ProjectCard,
    ),
    (crate::ui::translation::KEY_CONTEXT, Region::Translation),
    (crate::chat::region::KEY_CONTEXT, Region::Sheet),
    (crate::stage::bar::MENU_CONTEXT, Region::StageMenu),
    (crate::stage::page::KEY_CONTEXT, Region::Page),
    (crate::stage::page::ADDRESS, Region::Address),
    (crate::stage::terminal::KEY_CONTEXT, Region::Terminal),
    (crate::stage::code::KEY_CONTEXT, Region::Code),
    (crate::stage::document::KEY_CONTEXT, Region::Document),
    (crate::stage::tree::KEY_CONTEXT, Region::Tree),
    (crate::stage::changes::KEY_CONTEXT, Region::Changes),
    (crate::stage::android::KEY_CONTEXT, Region::Android),
    (crate::stage::devices::KEY_CONTEXT, Region::Devices),
    (crate::stage::flow::KEY_CONTEXT, Region::Flow),
    (crate::stage::windows::KEY_CONTEXT, Region::Windows),
    (crate::stage::mac_window::KEY_CONTEXT, Region::MacWindow),
    (line::KEY_CONTEXT, Region::Command),
];

impl Region {
    /// The region of the innermost of `contexts`, the window's first, that
    /// says one.
    #[must_use]
    pub fn of(contexts: &[KeyContext]) -> Self {
        contexts
            .iter()
            .rev()
            .filter_map(|context| context.primary())
            .find_map(|entry| {
                REGIONS
                    .iter()
                    .find(|(name, _)| entry.key == *name)
                    .map(|(_, region)| *region)
            })
            .unwrap_or_default()
    }

    /// Whether the region is a program's, which gets every key its tab
    /// sends in insert mode, and none in normal mode: a terminal's, or an
    /// Android device's.
    #[must_use]
    pub const fn runs_a_program(self) -> bool {
        matches!(self, Self::Terminal | Self::Android)
    }

    /// Whether the region's program gets `keystroke` from its tab: a
    /// terminal every key, an Android device those with no Control or
    /// Alt, which are Cox's.
    const fn program_gets(self, keystroke: &Keystroke) -> bool {
        match self {
            Self::Terminal => true,
            Self::Android => !keystroke.modifiers.control && !keystroke.modifiers.alt,
            _ => false,
        }
    }

    /// The mode the keys go to as the keyboard comes into the region, if
    /// it has one: insert in what opens only to type a line in and in a
    /// program, normal in a screen of settings, and where nothing is
    /// typed. A page tells its own, by whether a field of it has the
    /// keyboard.
    const fn enters_in(self) -> Option<Mode> {
        match self {
            Self::Goal
            | Self::Sketch
            | Self::Renaming
            | Self::Address
            | Self::Terminal
            | Self::Android => Some(Mode::Insert),
            Self::Command => Some(Mode::Command),
            Self::Settings
            | Self::ProjectCard
            | Self::Rail
            | Self::List
            | Self::Code
            | Self::Document
            | Self::Tree
            | Self::Changes
            | Self::Devices
            | Self::Flow
            | Self::Windows
            | Self::MacWindow => Some(Mode::Normal),
            _ => None,
        }
    }
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
        if matches!(self.mode, Mode::Hint | Mode::Command) {
            self.mode = Mode::Normal;
        }
        if let Some(mode) = region.enters_in().filter(|_| !back) {
            self.mode = mode;
        }
    }

    pub fn key(&mut self, keymap: &Keymap, region: Region, keystroke: &Keystroke) -> Outcome {
        self.arrive(region);
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
        if self.mode == Mode::Normal && self.held.is_empty() {
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
                match command {
                    Command::Normal => self.mode = Mode::Normal,
                    Command::Insert(_) => self.mode = Mode::Insert,
                    Command::Hint(_) => self.mode = Mode::Hint,
                    Command::Stop
                    | Command::Line(_)
                    | Command::Again(_)
                    | Command::Reset
                    | Command::Window(_)
                    | Command::Move(_)
                    | Command::Send(_) => {}
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
                    dropped || (self.mode == Mode::Normal && edits_text(keystroke))
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

/// The keyboard came to `region`, before any key there: its mode shows
/// at once, if Vim's keys are on.
pub fn came(region: Region, window: &mut Window, cx: &mut App) {
    if !cx.has_global::<Vim>() {
        return;
    }
    let vim = cx.global_mut::<Vim>();
    let was = vim.mode;
    vim.arrive(region);
    if vim.mode != was {
        window.refresh();
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
    let was = vim.mode;
    let outcome = vim.key(&Keymap::VIM, region, &event.keystroke);
    if vim.mode != was {
        window.refresh();
    }
    match outcome {
        Outcome::Pass => return,
        Outcome::Taken | Outcome::Do(Command::Reset, _) => {}
        Outcome::Do(Command::Normal, _) => {
            window.dispatch_action(Box::new(crate::chat::view::CloseList), cx);
            window.dispatch_action(Box::new(Leave), cx);
        }
        Outcome::Do(Command::Hint(follow), _) => {
            window.dispatch_action(Box::new(Hint { follow }), cx);
        }
        Outcome::Do(Command::Stop, _) => {
            window.dispatch_action(Box::new(crate::chat::view::Stop), cx);
        }
        Outcome::Do(Command::Window(act), count) => {
            if let Some(Some(workspace)) = window.root::<crate::workspace::Workspace>() {
                workspace.update(cx, |workspace, cx| {
                    workspace.act(act, region, count, window, cx);
                });
            }
        }
        Outcome::Do(Command::Line(prompt), _) => {
            window.dispatch_action(Box::new(line::OpenLine { prompt }), cx);
        }
        Outcome::Do(Command::Again(direction), _) => {
            if let Some(search) = last_search(cx) {
                let find = Find {
                    text: search.text.clone(),
                    direction: search.direction.then(direction),
                };
                window.dispatch_action(Box::new(find), cx);
            }
        }
        Outcome::Do(Command::Move(motion), count) => {
            window.dispatch_action(Box::new(Move { motion, count }), cx);
        }
        Outcome::Do(Command::Send(sent), count) => {
            for _ in 0..sent.times(count) {
                window.dispatch_action(sent.action(), cx);
            }
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

    use super::keymap::{Act, Command, Follow, Keymap, Motion, Pick, Place, Prompt, Sent};
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
            press(&mut vim, Region::Chat, "x shift-k space enter backspace"),
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
        assert_eq!(press(&mut vim, Region::Chat, "0"), [Outcome::Taken]);
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
    fn back_from_the_line_a_program_is_in_normal_mode_and_else_in_insert_mode() {
        let mut vim = Vim::default();
        press(&mut vim, Region::Terminal, "ctrl-\\ ctrl-n :");
        vim.mode = Mode::Command;
        press(&mut vim, Region::Command, "escape");
        vim.arrive(Region::Terminal);
        assert_eq!(vim.mode(), Mode::Normal);
        vim.arrive(Region::Chat);
        vim.arrive(Region::Terminal);
        assert_eq!(vim.mode(), Mode::Insert);
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
        assert_eq!(Mode::Hint.label(), "HINT");
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
        press(&mut vim, Region::Page, "shift-f");
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
        assert_eq!(Mode::Command.label(), "COMMAND");
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
