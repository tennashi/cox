//! The keys as in Vim, which the human turns on in `settings.json` with
//! `{"vim": true}`: a mode, normal or insert, in the chat and what opens
//! over it to type in, and the keys of each (see [`keymap`]).
//!
//! The keys are read ahead of the window's own, in one place
//! ([`install`]), which takes those Vim's keys do and leaves the rest as
//! they were. Where the keyboard is, its region, is told by the window's
//! key contexts. The stage's tabs keep every key of their own. While an
//! input method composes, every key is its own.

pub mod keymap;

use gpui::{App, Global, KeyContext, Keystroke, KeystrokeEvent, Window};

use self::keymap::{Answer, Command, Keymap, Place};

/// How the keys work: as Vim's normal mode, where they do things, or as
/// its insert mode, where they type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
}

impl Mode {
    /// How the window shows it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
        }
    }
}

/// Where the keyboard is, as far as Vim's keys tell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Region {
    /// The window, with no part of it holding the keyboard.
    #[default]
    Workspace,
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
}

/// The key contexts that say the region the keyboard is in.
const REGIONS: &[(&str, Region)] = &[
    ("Workspace", Region::Workspace),
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

    /// The mode the keys go to as the keyboard comes into the region, if
    /// it has one: insert in what opens only to type a line in, normal in
    /// a screen of settings.
    const fn enters_in(self) -> Option<Mode> {
        match self {
            Self::Goal | Self::Sketch | Self::Renaming => Some(Mode::Insert),
            Self::Settings | Self::ProjectCard => Some(Mode::Normal),
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
    /// It does this, and goes no further.
    Do(Command),
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

    pub fn key(&mut self, keymap: &Keymap, region: Region, keystroke: &Keystroke) -> Outcome {
        if region != self.region {
            self.region = region;
            self.reset();
            if let Some(mode) = region.enters_in() {
                self.mode = mode;
            }
        }
        if !keymap.knows(region) {
            return Outcome::Pass;
        }
        if keystroke.modifiers.platform {
            self.reset();
            return Outcome::Pass;
        }
        let key = keystroke.unparse();
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
                self.reset();
                match command {
                    Command::Normal => self.mode = Mode::Normal,
                    Command::Insert(_) => self.mode = Mode::Insert,
                    Command::Stop | Command::Reset => {}
                }
                Outcome::Do(command)
            }
            Answer::Held => {
                self.held.push(key);
                Outcome::Taken
            }
            Answer::None => {
                let dropped = !self.held.is_empty();
                self.reset();
                if dropped || (self.mode == Mode::Normal && edits_text(keystroke)) {
                    Outcome::Taken
                } else {
                    Outcome::Pass
                }
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

/// Reads every key ahead of the window's own, with Vim's keys, from now
/// on. Called as the app starts, when the human turned them on.
pub fn install(cx: &mut App) {
    cx.set_global(Vim::default());
    cx.intercept_keystrokes(intercept).detach();
}

/// The mode the keys are in, if Vim's keys are on.
#[must_use]
pub fn mode(cx: &App) -> Option<Mode> {
    cx.try_global::<Vim>().map(Vim::mode)
}

/// Goes to insert mode, if Vim's keys are on: for a new chat, which the
/// human opens to type in.
pub fn insert(cx: &mut App) {
    if cx.has_global::<Vim>() {
        cx.global_mut::<Vim>().mode = Mode::Insert;
    }
}

fn intercept(event: &KeystrokeEvent, window: &mut Window, cx: &mut App) {
    if crate::ui::composing::any(cx) {
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
        Outcome::Taken | Outcome::Do(Command::Normal | Command::Reset) => {}
        Outcome::Do(Command::Stop) => {
            window.dispatch_action(Box::new(crate::chat::view::Stop), cx);
        }
        Outcome::Do(Command::Insert(place)) => {
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
    use gpui::{KeyContext, Keystroke};

    use super::keymap::{Command, Keymap, Place};
    use super::{Mode, Outcome, Region, Vim};

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
            press(&mut vim, Region::Chat, "j shift-k space enter backspace"),
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
            [Outcome::Do(Command::Insert(Place::LineEnd))]
        );
        assert_eq!(vim.mode(), Mode::Insert);
        assert_eq!(
            press(&mut vim, Region::Chat, "j 1 ctrl-c"),
            [Outcome::Pass; 3]
        );
        assert_eq!(
            press(&mut vim, Region::Chat, "escape"),
            [Outcome::Do(Command::Normal)]
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
            [Outcome::Do(Command::Reset)]
        );
        assert_eq!(press(&mut vim, Region::Chat, "0"), [Outcome::Taken]);
        assert_eq!(vim.count(), None);
    }

    #[test]
    fn escape_in_normal_mode_goes_on_to_close_what_opened_over_the_chat() {
        let mut vim = Vim::default();
        assert_eq!(
            press(&mut vim, Region::Goal, "escape escape"),
            [Outcome::Do(Command::Normal), Outcome::Pass]
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
    fn the_stage_s_tabs_keep_every_key() {
        let mut vim = Vim::default();
        for region in [Region::Terminal, Region::Android, Region::Page] {
            assert_eq!(
                press(&mut vim, region, "j 1 escape ctrl-c i"),
                [Outcome::Pass; 5]
            );
        }
        assert_eq!(vim.mode(), Mode::Normal);
    }
}
