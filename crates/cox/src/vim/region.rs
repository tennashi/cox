//! Where the keyboard is, as far as Vim's keys tell (see [`super`]), and
//! all they know of each place: one row per region in [`Region::props`].

use gpui::{KeyContext, Keystroke};

use super::{Mode, hints};

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
    /// A chat's conversation, which a cursor moves through.
    Reading,
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

/// What a region is, among the kinds Vim's keys and their keymap tell
/// regions apart by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// It has a mode: the chat, and what opens over it to type in.
    Form,
    /// Its keyboard is in an input of one line or a few.
    Line,
    /// A part of the window `ctrl-w` and the stage's tabs' keys work in:
    /// not an input, screen or menu that opens over them.
    Part,
    /// A part `f` draws hints in: not a page, which draws its own, nor a
    /// screen over them.
    Hinter,
    /// Hint mode can be there, which escape leaves.
    Hinted,
    /// Vim's `/` searches its text.
    Searcher,
    /// It has a selection to move: the rail and the list of chats.
    Picker,
    /// Vim's keys scroll it, or move a cursor or selection in it: the
    /// conversation, from the chat's input too, a page, the stage's tabs
    /// that take no text, the settings and a project's card.
    Mover,
    /// `v` and `V` select there in visual mode: in code, in the
    /// conversation, and in an input. A terminal's `V` only makes its
    /// selection whole lines.
    Visual,
    /// A screen of inputs, each to type in as the keyboard comes to it,
    /// though the screen comes in normal mode.
    Fields,
    /// It is typed in whatever the mode, as a field of a page is: the
    /// page's address line, which Esc leaves.
    Modeless,
}

/// Which keys a tab that runs a program gets in normal mode, as in
/// Neovim's terminal mode, where it gets every key in insert mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// Every key: a terminal's.
    Every,
    /// Those with no Control or Alt, which are Cox's: an Android
    /// device's.
    Plain,
}

/// All Vim's keys know of a region.
#[derive(Clone, Copy, Debug)]
pub struct Props {
    /// The key context that says the keyboard is in it.
    pub context: &'static str,
    pub is: &'static [Kind],
    /// The mode the keys go to as the keyboard comes into it, if it has
    /// one: insert in what opens only to type a line in, normal in a
    /// screen of settings, where nothing is typed, and in a program, as
    /// Neovim's terminal is, until the human types in it (see
    /// [`super::typing`]). A page tells its own, by whether a field of
    /// it has the keyboard.
    pub enters_in: Option<Mode>,
    /// The program it runs, if it runs one.
    pub program: Option<Program>,
    /// The part `f` labels with the keyboard in it, if it labels one: a
    /// page labels its own.
    pub hints: Option<hints::Part>,
}

impl Props {
    const NONE: Self = Self {
        context: "",
        is: &[],
        enters_in: None,
        program: None,
        hints: None,
    };
}

impl Region {
    /// Every region, in the order they are declared.
    pub const ALL: [Self; 26] = [
        Self::Workspace,
        Self::Rail,
        Self::List,
        Self::Chat,
        Self::Reading,
        Self::Goal,
        Self::Sketch,
        Self::Renaming,
        Self::Settings,
        Self::ProjectCard,
        Self::Translation,
        Self::Sheet,
        Self::StageMenu,
        Self::Page,
        Self::Address,
        Self::Terminal,
        Self::Code,
        Self::Document,
        Self::Tree,
        Self::Changes,
        Self::Android,
        Self::Devices,
        Self::Flow,
        Self::Windows,
        Self::MacWindow,
        Self::Command,
    ];

    /// All Vim's keys know of the region.
    #[must_use]
    #[expect(clippy::too_many_lines, reason = "one arm for each region")]
    pub const fn props(self) -> Props {
        use Kind::{
            Fields, Form, Hinted, Hinter, Line, Modeless, Mover, Part, Picker, Searcher, Visual,
        };
        const NORMAL: Option<Mode> = Some(Mode::Normal);
        const INSERT: Option<Mode> = Some(Mode::Insert);
        const STAGE: Option<hints::Part> = Some(hints::Part::Stage);
        match self {
            Self::Workspace => Props {
                context: "Workspace",
                is: &[Part, Hinter, Hinted],
                hints: Some(hints::Part::Chat),
                ..Props::NONE
            },
            Self::Rail => Props {
                context: crate::workspace::RAIL,
                is: &[Part, Hinter, Hinted, Picker],
                enters_in: NORMAL,
                hints: Some(hints::Part::Rail),
                ..Props::NONE
            },
            Self::List => Props {
                context: crate::workspace::LIST,
                is: &[Part, Hinter, Hinted, Picker],
                enters_in: NORMAL,
                hints: Some(hints::Part::List),
                ..Props::NONE
            },
            Self::Chat => Props {
                context: crate::chat::view::KEY_CONTEXT,
                is: &[Form, Line, Part, Hinter, Hinted, Searcher, Mover, Visual],
                hints: Some(hints::Part::Chat),
                ..Props::NONE
            },
            Self::Reading => Props {
                context: crate::chat::view::READING,
                is: &[Part, Hinter, Hinted, Searcher, Mover, Visual],
                enters_in: NORMAL,
                hints: Some(hints::Part::Chat),
                ..Props::NONE
            },
            Self::Goal => Props {
                context: crate::chat::view::GOAL,
                is: &[Form, Line, Visual],
                enters_in: INSERT,
                hints: Some(hints::Part::Chat),
                ..Props::NONE
            },
            Self::Sketch => Props {
                context: crate::chat::view::SKETCH,
                is: &[Form, Line, Visual],
                enters_in: INSERT,
                hints: Some(hints::Part::Chat),
                ..Props::NONE
            },
            Self::Renaming => Props {
                context: crate::workspace::RENAMING,
                is: &[Form, Line, Visual],
                enters_in: INSERT,
                hints: Some(hints::Part::Chat),
                ..Props::NONE
            },
            Self::Settings => Props {
                context: crate::settings::view::KEY_CONTEXT,
                is: &[Form, Hinter, Hinted, Mover, Visual, Fields],
                enters_in: NORMAL,
                hints: Some(hints::Part::Settings),
                ..Props::NONE
            },
            Self::ProjectCard => Props {
                context: crate::workspace::project_card::KEY_CONTEXT,
                is: &[Form, Hinter, Hinted, Mover, Visual, Fields],
                enters_in: NORMAL,
                hints: Some(hints::Part::ProjectCard),
                ..Props::NONE
            },
            Self::Translation => Props {
                context: crate::ui::translation::KEY_CONTEXT,
                ..Props::NONE
            },
            Self::Sheet => Props {
                context: crate::chat::region::KEY_CONTEXT,
                ..Props::NONE
            },
            Self::StageMenu => Props {
                context: crate::stage::bar::MENU_CONTEXT,
                ..Props::NONE
            },
            Self::Page => Props {
                context: crate::stage::page::KEY_CONTEXT,
                is: &[Part, Hinted, Searcher, Mover],
                ..Props::NONE
            },
            Self::Address => Props {
                context: crate::stage::page::ADDRESS,
                is: &[Modeless],
                enters_in: INSERT,
                ..Props::NONE
            },
            Self::Terminal => Props {
                context: crate::stage::terminal::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted, Searcher, Mover],
                enters_in: NORMAL,
                program: Some(Program::Every),
                hints: STAGE,
            },
            Self::Code => Props {
                context: crate::stage::code::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted, Searcher, Mover, Visual],
                enters_in: NORMAL,
                hints: STAGE,
                ..Props::NONE
            },
            Self::Document => Props {
                context: crate::stage::document::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted, Searcher, Mover],
                enters_in: NORMAL,
                hints: STAGE,
                ..Props::NONE
            },
            Self::Tree => Props {
                context: crate::stage::tree::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted, Mover],
                enters_in: NORMAL,
                hints: STAGE,
                ..Props::NONE
            },
            Self::Changes => Props {
                context: crate::stage::changes::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted, Mover],
                enters_in: NORMAL,
                hints: STAGE,
                ..Props::NONE
            },
            Self::Android => Props {
                context: crate::stage::android::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted],
                enters_in: NORMAL,
                program: Some(Program::Plain),
                hints: STAGE,
            },
            Self::Devices => Props {
                context: crate::stage::devices::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted],
                enters_in: NORMAL,
                hints: STAGE,
                ..Props::NONE
            },
            Self::Flow => Props {
                context: crate::stage::flow::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted],
                enters_in: NORMAL,
                hints: STAGE,
                ..Props::NONE
            },
            Self::Windows => Props {
                context: crate::stage::windows::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted],
                enters_in: NORMAL,
                hints: STAGE,
                ..Props::NONE
            },
            Self::MacWindow => Props {
                context: crate::stage::mac_window::KEY_CONTEXT,
                is: &[Part, Hinter, Hinted],
                enters_in: NORMAL,
                hints: STAGE,
                ..Props::NONE
            },
            Self::Command => Props {
                context: super::line::KEY_CONTEXT,
                enters_in: Some(Mode::Command),
                ..Props::NONE
            },
        }
    }

    /// Whether the region is of `kind`.
    #[must_use]
    pub fn is(self, kind: Kind) -> bool {
        self.props().is.contains(&kind)
    }

    /// The region of the innermost of `contexts`, the window's first, that
    /// says one.
    #[must_use]
    pub fn of(contexts: &[KeyContext]) -> Self {
        contexts
            .iter()
            .rev()
            .filter_map(|context| context.primary())
            .find_map(|entry| {
                Self::ALL
                    .into_iter()
                    .find(|region| entry.key == region.props().context)
            })
            .unwrap_or_default()
    }

    /// Whether the region is a program's, which gets every key its tab
    /// sends in insert mode, and none in normal mode: a terminal's, or an
    /// Android device's.
    #[must_use]
    pub const fn runs_a_program(self) -> bool {
        self.props().program.is_some()
    }

    /// Whether the region's program gets `keystroke` from its tab.
    pub(super) const fn program_gets(self, keystroke: &Keystroke) -> bool {
        match self.props().program {
            Some(Program::Every) => true,
            Some(Program::Plain) => !keystroke.modifiers.control && !keystroke.modifiers.alt,
            None => false,
        }
    }
}

const _: () = {
    let mut at = 0;
    while at < Region::ALL.len() {
        assert!(
            Region::ALL[at] as usize == at,
            "Region::ALL keeps the order regions are declared in"
        );
        at += 1;
    }
    assert!(
        Region::ALL.len() == Region::Command as usize + 1,
        "Region::ALL ends with the last region"
    );
};

#[cfg(test)]
mod tests {
    use gpui::KeyContext;

    use super::Region;

    #[test]
    fn each_region_is_told_by_a_key_context_of_its_own() {
        for region in Region::ALL {
            let context = region.props().context;
            let stack = [
                KeyContext::parse("Workspace").unwrap(),
                KeyContext::parse(context).unwrap(),
            ];
            assert_eq!(Region::of(&stack), region, "{context}");
        }
    }
}
