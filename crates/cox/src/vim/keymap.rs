//! What a key does in Vim's keys: a table of what the keys do, by the
//! mode, the region the keyboard is in, and the keys held before.

use super::{Mode, Region};

/// What the keymap says of the keys pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Act(Command),
    /// They start keys that do something, and are held for the next.
    Held,
    /// They do nothing in Vim's keys.
    None,
}

/// What keys do in Vim's keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Normal,
    Insert(Place),
    /// Stops the turn the agent is taking.
    Stop,
    /// Forgets the count and the keys held, and does nothing else: in the
    /// chat, where Escape would stop the agent, Escape in normal mode.
    Reset,
}

/// Where insert mode puts the cursor of the input with the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// Where it is (`i`).
    Cursor,
    /// After the character it is before (`a`).
    After,
    /// At the start of its line (`I`).
    LineStart,
    /// At the end of its line (`A`).
    LineEnd,
}

/// Keys that do something, in a mode, in some regions.
#[derive(Debug)]
pub struct Binding {
    pub mode: Mode,
    pub regions: &'static [Region],
    /// As GPUI writes keystrokes: `"escape"`, `"shift-a"`, `"ctrl-c"`.
    pub keys: &'static [&'static str],
    pub command: Command,
}

/// The table of what keys do. A region no binding names is left its own
/// keys, every one of them.
#[derive(Debug)]
pub struct Keymap(pub &'static [Binding]);

/// The regions with a mode: the chat, and what opens over it to type in.
const FORMS: &[Region] = &[
    Region::Chat,
    Region::Goal,
    Region::Sketch,
    Region::Renaming,
    Region::Settings,
    Region::ProjectCard,
];

/// The regions whose keyboard is in an input of one line or a few.
const LINES: &[Region] = &[Region::Chat, Region::Goal, Region::Sketch, Region::Renaming];

const fn bind(
    mode: Mode,
    regions: &'static [Region],
    keys: &'static [&'static str],
    command: Command,
) -> Binding {
    Binding {
        mode,
        regions,
        keys,
        command,
    }
}

impl Keymap {
    pub const VIM: Self = Self(&[
        bind(Mode::Insert, FORMS, &["escape"], Command::Normal),
        bind(Mode::Normal, &[Region::Chat], &["escape"], Command::Reset),
        bind(Mode::Normal, &[Region::Chat], &["ctrl-c"], Command::Stop),
        bind(Mode::Normal, FORMS, &["i"], Command::Insert(Place::Cursor)),
        bind(Mode::Normal, LINES, &["a"], Command::Insert(Place::After)),
        bind(
            Mode::Normal,
            LINES,
            &["shift-i"],
            Command::Insert(Place::LineStart),
        ),
        bind(
            Mode::Normal,
            LINES,
            &["shift-a"],
            Command::Insert(Place::LineEnd),
        ),
    ]);

    /// Whether any binding is in `region`.
    #[must_use]
    pub fn knows(&self, region: Region) -> bool {
        self.0
            .iter()
            .any(|binding| binding.regions.contains(&region))
    }

    /// What `keys`, the keys held and the one pressed after them, do in
    /// `mode` in `region`.
    #[must_use]
    pub fn answer(&self, mode: Mode, region: Region, keys: &[&str]) -> Answer {
        let bindings = self
            .0
            .iter()
            .filter(|binding| binding.mode == mode && binding.regions.contains(&region));
        let mut held = false;
        for binding in bindings {
            if binding.keys == keys {
                return Answer::Act(binding.command);
            }
            held |= binding.keys.len() > keys.len() && binding.keys.starts_with(keys);
        }
        if held { Answer::Held } else { Answer::None }
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, Command, Keymap, Mode, Place, Region, bind};

    #[test]
    fn escape_in_insert_mode_goes_to_normal_wherever_there_is_a_mode() {
        for region in super::FORMS {
            assert_eq!(
                Keymap::VIM.answer(Mode::Insert, *region, &["escape"]),
                Answer::Act(Command::Normal),
                "{region:?}"
            );
        }
    }

    #[test]
    fn escape_in_normal_mode_does_nothing_in_the_chat_and_is_the_forms_own_elsewhere() {
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Chat, &["escape"]),
            Answer::Act(Command::Reset)
        );
        for region in [Region::Goal, Region::Settings, Region::ProjectCard] {
            assert_eq!(
                Keymap::VIM.answer(Mode::Normal, region, &["escape"]),
                Answer::None
            );
        }
    }

    #[test]
    fn the_keys_into_insert_mode_put_the_cursor_where_vim_does() {
        let answer = |keys| Keymap::VIM.answer(Mode::Normal, Region::Chat, &[keys]);
        assert_eq!(answer("i"), Answer::Act(Command::Insert(Place::Cursor)));
        assert_eq!(answer("a"), Answer::Act(Command::Insert(Place::After)));
        assert_eq!(
            answer("shift-i"),
            Answer::Act(Command::Insert(Place::LineStart))
        );
        assert_eq!(
            answer("shift-a"),
            Answer::Act(Command::Insert(Place::LineEnd))
        );
        assert_eq!(answer("ctrl-c"), Answer::Act(Command::Stop));
        assert_eq!(
            Keymap::VIM.answer(Mode::Insert, Region::Chat, &["i"]),
            Answer::None
        );
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Settings, &["a"]),
            Answer::None
        );
    }

    #[test]
    fn the_stage_s_tabs_keep_their_keys() {
        for region in [
            Region::Terminal,
            Region::Android,
            Region::Page,
            Region::Code,
            Region::Workspace,
        ] {
            assert!(!Keymap::VIM.knows(region), "{region:?}");
        }
        assert!(Keymap::VIM.knows(Region::Chat));
    }

    #[test]
    fn the_start_of_a_sequence_is_held() {
        const SEQUENCE: &[super::Binding] = &[bind(
            Mode::Normal,
            &[Region::Chat],
            &["g", "g"],
            Command::Reset,
        )];
        let keymap = Keymap(SEQUENCE);
        assert_eq!(
            keymap.answer(Mode::Normal, Region::Chat, &["g"]),
            Answer::Held
        );
        assert_eq!(
            keymap.answer(Mode::Normal, Region::Chat, &["g", "g"]),
            Answer::Act(Command::Reset)
        );
        assert_eq!(
            keymap.answer(Mode::Normal, Region::Chat, &["g", "x"]),
            Answer::None
        );
        assert_eq!(
            keymap.answer(Mode::Normal, Region::Goal, &["g"]),
            Answer::None
        );
    }
}
