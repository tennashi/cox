//! What a key does in Vim's keys: a table of what the keys do, by the
//! mode, the region the keyboard is in, and the keys held before.

use super::search::Direction;
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
    /// What the window does, with the count typed before.
    Window(Act),
    /// Moves what the part with the keyboard shows, or its cursor, with
    /// the count typed before (see [`super::Move`]).
    Move(Motion),
    /// Moves the cursor of code across its text, with the count typed
    /// before (see [`super::MoveAcross`]).
    Across(Across),
    /// What the part with the keyboard does, as an action of its own.
    Send(Sent),
    /// Draws hints over what can be clicked in the page, to follow the
    /// one whose label is typed next, in hint mode.
    Hint(Follow),
    /// Draws hints over what can be clicked in the part of the window
    /// with the keyboard, or in the whole window, to click the one whose
    /// label is typed next, in hint mode.
    Hints(Reach),
    /// Opens the line at the window's foot, to type in, in command mode.
    Line(Prompt),
    /// Finds the last text searched for again: the way it went, or the
    /// other way (`n`, `N`).
    Again(Direction),
    /// What normal mode does to the text of the input with the keyboard,
    /// with the count typed before (see [`super::EditText`]).
    Edit(Edit),
}

/// What normal mode does to the text of an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Moves the cursor (`h`, `w`, `$`).
    Move(Across),
    /// Takes out what the span covers, keeping it to put (`x`, `dw`, `dd`).
    Delete(Span),
    /// Takes it out and goes to insert mode in its place (`s`, `cw`, `cc`).
    Change(Span),
    /// Keeps it to put (`yw`, `yy`).
    Yank(Span),
    /// Puts what was kept after the cursor, or its line, or before them
    /// (`p`, `P`).
    Put {
        before: bool,
    },
    Undo,
    Redo,
    /// Starts selecting as visual mode does from the cursor, or switches
    /// to selecting so, or stops when it selects so already (`v`, `V`).
    Visual(Selects),
}

/// What visual mode selects from where it started to the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selects {
    /// The characters between, both ends in (`v`).
    Characters,
    /// The whole lines between (`V`).
    Lines,
}

/// What an edit of normal mode covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    /// What visual mode selects.
    Selection,
    /// From the cursor to where the motion takes it.
    To(Across),
    /// Its whole lines, from the cursor's (`dd`).
    Line,
    /// The word under the cursor, or the run of spaces (`iw`).
    InnerWord,
    /// The word with the spaces after it, or else before it (`aw`).
    AWord,
}

impl Command {
    /// Whether it only moves the cursor, or what shows, or finds: what
    /// visual mode takes of normal mode's keys, to grow what it selects.
    #[must_use]
    pub const fn moves(self) -> bool {
        matches!(
            self,
            Self::Move(_)
                | Self::Across(_)
                | Self::Edit(Edit::Move(_))
                | Self::Again(_)
                | Self::Line(Prompt::Search(_))
                | Self::Send(Sent::FindWord(_) | Sent::ScrollTo(_))
        )
    }

    /// What visual mode it starts, or switches to, if it does.
    #[must_use]
    pub const fn selects(self) -> Option<Selects> {
        match self {
            Self::Edit(Edit::Visual(selects)) => Some(selects),
            Self::Send(Sent::VisualCharacters) => Some(Selects::Characters),
            Self::Send(Sent::Visual) => Some(Selects::Lines),
            _ => None,
        }
    }
}

/// What the line at the window's foot is opened for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prompt {
    /// A command of Cox's (`:`, see [`super::ex`]).
    Ex,
    /// Text to find in the part with the keyboard, forward (`/`) or back
    /// (`?`).
    Search(Direction),
}

/// What hints are drawn over, out of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// The part of the window with the keyboard (`f`).
    Part,
    /// The whole window, the page shown too (`F`).
    Window,
}

/// Where a hint typed in hint mode leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    /// Clicks it, or gives the keyboard to it if it takes text (`f`).
    Here,
    /// Opens the link on a tab of its own (`;t`).
    NewTab,
}

/// Where Vim's keys move what a part of the window shows, or its cursor
/// or selection, if it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// A line down, or the count's (`j`).
    Down,
    /// A line up, or the count's (`k`).
    Up,
    /// Half the part's height down, or the count's halves (`ctrl-d`).
    HalfDown,
    /// Half the part's height up, or the count's halves (`ctrl-u`).
    HalfUp,
    /// To the top, or to the count's line (`gg`).
    Top,
    /// To the bottom, or to the count's line (`G`).
    Bottom,
}

/// Where Vim's keys move the cursor of code: along its line, or to the
/// words before and after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Across {
    /// A character left, or the count's (`h`).
    Left,
    /// A character right, or the count's (`l`).
    Right,
    /// To the next word's start (`w`).
    WordForward,
    /// To the start of the word, or of the one before (`b`).
    WordBack,
    /// To the end of the word, or of the next (`e`).
    WordEnd,
    /// To the line's start (`0`).
    LineStart,
    /// To the line's first character that is no space (`^`).
    FirstNonBlank,
    /// To the line's end, the count's lines down less one (`$`).
    LineEnd,
    /// To the bracket that matches the one under the cursor, or the next
    /// on its line (`%`).
    Bracket,
    /// To the next empty line, or the count's (`}`).
    ParagraphForward,
    /// To the empty line before, or the count's (`{`).
    ParagraphBack,
}

/// Where in the part's height the cursor's line is scrolled to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sight {
    /// To the top (`zt`).
    Top,
    /// To the middle (`zz`).
    Middle,
    /// To the bottom (`zb`).
    Bottom,
}

/// The actions of the parts of the window Vim's keys send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    Copy,
    AddToChat,
    Back,
    Forward,
    Visual,
    /// Selects characters from the cursor's (`v`).
    VisualCharacters,
    Definition,
    /// Lists where the name under the cursor is used (`gr`).
    References,
    /// Shows at once what the language server says of the name under the
    /// cursor (`K`).
    LookUp,
    /// Finds the whole word under the cursor, as `direction` goes (`*`,
    /// `#`).
    FindWord(Direction),
    /// Scrolls the cursor's line to where in the part it says (`zt`, `zz`,
    /// `zb`).
    ScrollTo(Sight),
    Open,
    Close,
    Reload,
    /// Gives the keyboard to the page's first field (`gi`).
    FocusField,
    /// Copies the page's address (`yy`).
    CopyAddress,
    /// Takes the keyboard to the page's address line (`o`).
    Address,
    /// A key typed in hint mode, toward a hint's label.
    Label(char),
    /// Runs the line at the window's foot (`enter`).
    Accept,
    /// Puts the next of what the line can go on to in it (`tab`).
    NextFit,
    /// Puts the one before in it (`shift-tab`).
    PreviousFit,
    /// Puts the line typed before in it (`up`).
    Older,
    /// Puts the line typed after in it (`down`).
    Newer,
    /// Selects the next entry of the conversation, or block of a
    /// document (`]]`).
    NextEntry,
    /// Selects the entry or block before (`[[`).
    PreviousEntry,
    /// Selects the next stretch of changed lines (`]c`).
    NextStretch,
    /// Selects the stretch of changed lines before (`[c`).
    PreviousStretch,
}

/// What the window does for Vim's keys: where the keyboard goes, and
/// what it does there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    /// Moves the keyboard to another part of the window (`ctrl-w`).
    Go(Toward),
    /// Moves the selection of the rail, or of the list of chats.
    Pick(Pick),
    /// Opens what is selected, with the keyboard in its chat's input.
    Open,
    /// Opens a new chat in the project selected, or chosen.
    New,
    /// Shows the stage's next tab, or with a count the tab of that number
    /// (`gt`).
    NextTab,
    /// Shows the stage's tab before, or the count's before (`gT`).
    PreviousTab,
    /// Closes the tab the stage shows (`ctrl-w c`).
    CloseTab,
    /// Opens the menu of the stage's +, to type what another tab opens
    /// (`t`).
    Menu,
}

/// Where `ctrl-w` moves the keyboard: left or right along the rail, the
/// stage and the chat as the window draws them, on round them, or up to
/// the list of chats over the chat and down from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    Left,
    Right,
    Round,
    Up,
    Down,
}

/// Where a selection moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Down,
    Up,
    First,
    Last,
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

/// The parts of the window `ctrl-w` and the stage's tabs' keys work in:
/// all but the inputs, screens and menus that open over them.
const PARTS: &[Region] = &[
    Region::Workspace,
    Region::Rail,
    Region::List,
    Region::Chat,
    Region::Page,
    Region::Code,
    Region::Document,
    Region::Tree,
    Region::Changes,
    Region::Devices,
    Region::Flow,
    Region::Windows,
    Region::MacWindow,
    Region::Terminal,
    Region::Android,
];

/// The parts `f` draws hints in: all but a page, which draws its own,
/// and the screens over them.
const HINTERS: &[Region] = &[
    Region::Workspace,
    Region::Rail,
    Region::List,
    Region::Chat,
    Region::Code,
    Region::Document,
    Region::Tree,
    Region::Changes,
    Region::Devices,
    Region::Flow,
    Region::Windows,
    Region::MacWindow,
    Region::Terminal,
    Region::Android,
    Region::Settings,
    Region::ProjectCard,
];

/// Where hint mode can be, which escape leaves.
const HINTED: &[Region] = &[
    Region::Workspace,
    Region::Rail,
    Region::List,
    Region::Chat,
    Region::Page,
    Region::Code,
    Region::Document,
    Region::Tree,
    Region::Changes,
    Region::Devices,
    Region::Flow,
    Region::Windows,
    Region::MacWindow,
    Region::Terminal,
    Region::Android,
    Region::Settings,
    Region::ProjectCard,
];

/// The parts Vim's `/` searches the text of.
const SEARCHERS: &[Region] = &[
    Region::Chat,
    Region::Page,
    Region::Code,
    Region::Document,
    Region::Terminal,
];

/// The tabs that run a program, which gets every key in insert mode, as
/// in Neovim's terminal mode: a terminal's, and an Android device's.
const PROGRAMS: &[Region] = &[Region::Terminal, Region::Android];

/// The line at the window's foot.
const LINE: &[Region] = &[Region::Command];

/// The parts with a selection to move: the rail and the list of chats.
const PICKERS: &[Region] = &[Region::Rail, Region::List];

/// The parts Vim's keys scroll, or move a cursor or selection in: the
/// conversation, from the chat's input, a page, the stage's tabs that
/// take no text, the settings and a project's card.
const MOVERS: &[Region] = &[
    Region::Chat,
    Region::Page,
    Region::Code,
    Region::Document,
    Region::Tree,
    Region::Changes,
    Region::Terminal,
    Region::Settings,
    Region::ProjectCard,
];

const PAGE: &[Region] = &[Region::Page];
const ADDRESS: &[Region] = &[Region::Address];
const CODE: &[Region] = &[Region::Code];
const DOCUMENT: &[Region] = &[Region::Document];
const TREE: &[Region] = &[Region::Tree];
const CHANGES: &[Region] = &[Region::Changes];
const TERMINAL: &[Region] = &[Region::Terminal];

const fn act(regions: &'static [Region], keys: &'static [&'static str], act: Act) -> Binding {
    bind(Mode::Normal, regions, keys, Command::Window(act))
}

const fn go(regions: &'static [Region], keys: &'static [&'static str], motion: Motion) -> Binding {
    bind(Mode::Normal, regions, keys, Command::Move(motion))
}

const fn across(regions: &'static [Region], keys: &'static [&'static str], to: Across) -> Binding {
    bind(Mode::Normal, regions, keys, Command::Across(to))
}

const fn edit(keys: &'static [&'static str], edit: Edit) -> Binding {
    bind(Mode::Normal, FORMS, keys, Command::Edit(edit))
}

/// The mode the bindings of visual mode are kept under, whichever it
/// selects: [`Keymap::answer`] looks them up for both.
const VISUAL: Mode = Mode::Visual(Selects::Characters);

const fn visual(
    regions: &'static [Region],
    keys: &'static [&'static str],
    command: Command,
) -> Binding {
    bind(VISUAL, regions, keys, command)
}

const fn send(regions: &'static [Region], keys: &'static [&'static str], sent: Sent) -> Binding {
    bind(Mode::Normal, regions, keys, Command::Send(sent))
}

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
        edit(&["h"], Edit::Move(Across::Left)),
        edit(&["l"], Edit::Move(Across::Right)),
        edit(&["w"], Edit::Move(Across::WordForward)),
        edit(&["b"], Edit::Move(Across::WordBack)),
        edit(&["e"], Edit::Move(Across::WordEnd)),
        edit(&["0"], Edit::Move(Across::LineStart)),
        edit(&["^"], Edit::Move(Across::FirstNonBlank)),
        edit(&["$"], Edit::Move(Across::LineEnd)),
        edit(&["d", "h"], Edit::Delete(Span::To(Across::Left))),
        edit(&["d", "l"], Edit::Delete(Span::To(Across::Right))),
        edit(&["d", "w"], Edit::Delete(Span::To(Across::WordForward))),
        edit(&["d", "b"], Edit::Delete(Span::To(Across::WordBack))),
        edit(&["d", "e"], Edit::Delete(Span::To(Across::WordEnd))),
        edit(&["d", "0"], Edit::Delete(Span::To(Across::LineStart))),
        edit(&["d", "^"], Edit::Delete(Span::To(Across::FirstNonBlank))),
        edit(&["d", "$"], Edit::Delete(Span::To(Across::LineEnd))),
        edit(&["d", "d"], Edit::Delete(Span::Line)),
        edit(&["d", "i", "w"], Edit::Delete(Span::InnerWord)),
        edit(&["d", "a", "w"], Edit::Delete(Span::AWord)),
        edit(&["c", "h"], Edit::Change(Span::To(Across::Left))),
        edit(&["c", "l"], Edit::Change(Span::To(Across::Right))),
        edit(&["c", "w"], Edit::Change(Span::To(Across::WordForward))),
        edit(&["c", "b"], Edit::Change(Span::To(Across::WordBack))),
        edit(&["c", "e"], Edit::Change(Span::To(Across::WordEnd))),
        edit(&["c", "0"], Edit::Change(Span::To(Across::LineStart))),
        edit(&["c", "^"], Edit::Change(Span::To(Across::FirstNonBlank))),
        edit(&["c", "$"], Edit::Change(Span::To(Across::LineEnd))),
        edit(&["c", "c"], Edit::Change(Span::Line)),
        edit(&["c", "i", "w"], Edit::Change(Span::InnerWord)),
        edit(&["c", "a", "w"], Edit::Change(Span::AWord)),
        edit(&["y", "h"], Edit::Yank(Span::To(Across::Left))),
        edit(&["y", "l"], Edit::Yank(Span::To(Across::Right))),
        edit(&["y", "w"], Edit::Yank(Span::To(Across::WordForward))),
        edit(&["y", "b"], Edit::Yank(Span::To(Across::WordBack))),
        edit(&["y", "e"], Edit::Yank(Span::To(Across::WordEnd))),
        edit(&["y", "0"], Edit::Yank(Span::To(Across::LineStart))),
        edit(&["y", "^"], Edit::Yank(Span::To(Across::FirstNonBlank))),
        edit(&["y", "$"], Edit::Yank(Span::To(Across::LineEnd))),
        edit(&["y", "y"], Edit::Yank(Span::Line)),
        edit(&["y", "i", "w"], Edit::Yank(Span::InnerWord)),
        edit(&["y", "a", "w"], Edit::Yank(Span::AWord)),
        edit(&["x"], Edit::Delete(Span::To(Across::Right))),
        edit(&["shift-x"], Edit::Delete(Span::To(Across::Left))),
        edit(&["shift-d"], Edit::Delete(Span::To(Across::LineEnd))),
        edit(&["s"], Edit::Change(Span::To(Across::Right))),
        edit(&["shift-s"], Edit::Change(Span::Line)),
        edit(&["shift-c"], Edit::Change(Span::To(Across::LineEnd))),
        edit(&["shift-y"], Edit::Yank(Span::Line)),
        edit(&["p"], Edit::Put { before: false }),
        edit(&["shift-p"], Edit::Put { before: true }),
        edit(&["u"], Edit::Undo),
        edit(&["ctrl-r"], Edit::Redo),
        act(PARTS, &["ctrl-w", "h"], Act::Go(Toward::Left)),
        act(PARTS, &["ctrl-w", "l"], Act::Go(Toward::Right)),
        act(PARTS, &["ctrl-w", "w"], Act::Go(Toward::Round)),
        act(PARTS, &["ctrl-w", "c"], Act::CloseTab),
        act(PARTS, &["g", "t"], Act::NextTab),
        act(PARTS, &["g", "shift-t"], Act::PreviousTab),
        act(&[Region::Chat], &["ctrl-w", "k"], Act::Go(Toward::Up)),
        act(&[Region::List], &["ctrl-w", "j"], Act::Go(Toward::Down)),
        act(&[Region::List], &["escape"], Act::Go(Toward::Down)),
        act(PICKERS, &["j"], Act::Pick(Pick::Down)),
        act(PICKERS, &["k"], Act::Pick(Pick::Up)),
        act(PICKERS, &["g", "g"], Act::Pick(Pick::First)),
        act(PICKERS, &["shift-g"], Act::Pick(Pick::Last)),
        act(PICKERS, &["enter"], Act::Open),
        act(PICKERS, &["l"], Act::Open),
        act(PICKERS, &["o"], Act::New),
        go(MOVERS, &["j"], Motion::Down),
        go(MOVERS, &["k"], Motion::Up),
        go(MOVERS, &["ctrl-d"], Motion::HalfDown),
        go(MOVERS, &["ctrl-u"], Motion::HalfUp),
        go(MOVERS, &["g", "g"], Motion::Top),
        go(MOVERS, &["shift-g"], Motion::Bottom),
        across(CODE, &["h"], Across::Left),
        across(CODE, &["l"], Across::Right),
        across(CODE, &["w"], Across::WordForward),
        across(CODE, &["b"], Across::WordBack),
        across(CODE, &["e"], Across::WordEnd),
        across(CODE, &["0"], Across::LineStart),
        across(CODE, &["^"], Across::FirstNonBlank),
        across(CODE, &["$"], Across::LineEnd),
        across(CODE, &["%"], Across::Bracket),
        across(CODE, &["}"], Across::ParagraphForward),
        across(CODE, &["{"], Across::ParagraphBack),
        send(&[Region::Chat], &["]", "]"], Sent::NextEntry),
        send(&[Region::Chat], &["[", "["], Sent::PreviousEntry),
        send(&[Region::Chat], &["g", "y"], Sent::Copy),
        send(CODE, &["shift-v"], Sent::Visual),
        send(CODE, &["v"], Sent::VisualCharacters),
        visual(CODE, &["shift-v"], Command::Send(Sent::Visual)),
        visual(CODE, &["v"], Command::Send(Sent::VisualCharacters)),
        visual(CODE, &["y"], Command::Send(Sent::Copy)),
        visual(CODE, &["g", "a"], Command::Send(Sent::AddToChat)),
        visual(CODE, &["escape"], Command::Normal),
        edit(&["v"], Edit::Visual(Selects::Characters)),
        edit(&["shift-v"], Edit::Visual(Selects::Lines)),
        visual(
            FORMS,
            &["v"],
            Command::Edit(Edit::Visual(Selects::Characters)),
        ),
        visual(
            FORMS,
            &["shift-v"],
            Command::Edit(Edit::Visual(Selects::Lines)),
        ),
        visual(FORMS, &["d"], Command::Edit(Edit::Delete(Span::Selection))),
        visual(FORMS, &["x"], Command::Edit(Edit::Delete(Span::Selection))),
        visual(FORMS, &["c"], Command::Edit(Edit::Change(Span::Selection))),
        visual(FORMS, &["s"], Command::Edit(Edit::Change(Span::Selection))),
        visual(FORMS, &["y"], Command::Edit(Edit::Yank(Span::Selection))),
        visual(FORMS, &["escape"], Command::Normal),
        send(CODE, &["g", "r"], Sent::References),
        send(CODE, &["shift-k"], Sent::LookUp),
        send(CODE, &["*"], Sent::FindWord(Direction::Forward)),
        send(CODE, &["#"], Sent::FindWord(Direction::Backward)),
        send(CODE, &["z", "t"], Sent::ScrollTo(Sight::Top)),
        send(CODE, &["z", "z"], Sent::ScrollTo(Sight::Middle)),
        send(CODE, &["z", "b"], Sent::ScrollTo(Sight::Bottom)),
        send(CODE, &["y"], Sent::Copy),
        send(CODE, &["g", "a"], Sent::AddToChat),
        send(DOCUMENT, &["]", "]"], Sent::NextEntry),
        send(DOCUMENT, &["[", "["], Sent::PreviousEntry),
        send(DOCUMENT, &["y"], Sent::Copy),
        send(DOCUMENT, &["g", "a"], Sent::AddToChat),
        send(CODE, &["g", "d"], Sent::Definition),
        send(CODE, &["ctrl-o"], Sent::Back),
        send(CODE, &["ctrl-i"], Sent::Forward),
        send(TREE, &["l"], Sent::Open),
        send(TREE, &["enter"], Sent::Open),
        send(TREE, &["h"], Sent::Close),
        send(CHANGES, &["enter"], Sent::Open),
        send(CHANGES, &["]", "c"], Sent::NextStretch),
        send(CHANGES, &["[", "c"], Sent::PreviousStretch),
        bind(
            Mode::Insert,
            PROGRAMS,
            &["ctrl-\\", "ctrl-n"],
            Command::Normal,
        ),
        bind(
            Mode::Normal,
            PROGRAMS,
            &["ctrl-\\", "ctrl-n"],
            Command::Normal,
        ),
        bind(
            Mode::Normal,
            PROGRAMS,
            &["i"],
            Command::Insert(Place::Cursor),
        ),
        bind(
            Mode::Normal,
            TERMINAL,
            &["a"],
            Command::Insert(Place::Cursor),
        ),
        send(TERMINAL, &["shift-v"], Sent::Visual),
        send(TERMINAL, &["y"], Sent::Copy),
        bind(Mode::Normal, PAGE, &["i"], Command::Insert(Place::Cursor)),
        bind(Mode::Insert, PAGE, &["escape"], Command::Normal),
        bind(Mode::Hint, HINTED, &["escape"], Command::Normal),
        bind(Mode::Normal, PAGE, &["f"], Command::Hint(Follow::Here)),
        bind(
            Mode::Normal,
            PAGE,
            &[";", "t"],
            Command::Hint(Follow::NewTab),
        ),
        bind(Mode::Normal, HINTERS, &["f"], Command::Hints(Reach::Part)),
        bind(
            Mode::Normal,
            HINTED,
            &["shift-f"],
            Command::Hints(Reach::Window),
        ),
        send(PAGE, &["shift-h"], Sent::Back),
        send(PAGE, &["shift-l"], Sent::Forward),
        send(PAGE, &["r"], Sent::Reload),
        send(PAGE, &["g", "i"], Sent::FocusField),
        send(PAGE, &["y", "y"], Sent::CopyAddress),
        send(PAGE, &["o"], Sent::Address),
        act(PAGE, &["t"], Act::Menu),
        bind(Mode::Insert, ADDRESS, &["escape"], Command::Normal),
        bind(Mode::Normal, PARTS, &[":"], Command::Line(Prompt::Ex)),
        bind(
            Mode::Normal,
            SEARCHERS,
            &["/"],
            Command::Line(Prompt::Search(Direction::Forward)),
        ),
        bind(
            Mode::Normal,
            SEARCHERS,
            &["?"],
            Command::Line(Prompt::Search(Direction::Backward)),
        ),
        bind(
            Mode::Normal,
            SEARCHERS,
            &["n"],
            Command::Again(Direction::Forward),
        ),
        bind(
            Mode::Normal,
            SEARCHERS,
            &["shift-n"],
            Command::Again(Direction::Backward),
        ),
        bind(Mode::Command, LINE, &["escape"], Command::Normal),
        bind(Mode::Command, LINE, &["enter"], Command::Send(Sent::Accept)),
        bind(Mode::Command, LINE, &["tab"], Command::Send(Sent::NextFit)),
        bind(
            Mode::Command,
            LINE,
            &["ctrl-n"],
            Command::Send(Sent::NextFit),
        ),
        bind(
            Mode::Command,
            LINE,
            &["shift-tab"],
            Command::Send(Sent::PreviousFit),
        ),
        bind(
            Mode::Command,
            LINE,
            &["ctrl-p"],
            Command::Send(Sent::PreviousFit),
        ),
        bind(Mode::Command, LINE, &["up"], Command::Send(Sent::Older)),
        bind(Mode::Command, LINE, &["down"], Command::Send(Sent::Newer)),
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
        if !matches!(mode, Mode::Visual(_)) {
            return self.answer_in(mode, region, keys);
        }
        // Visual mode has keys of its own, and moves as normal mode does.
        match self.answer_in(VISUAL, region, keys) {
            Answer::None => match self.answer_in(Mode::Normal, region, keys) {
                Answer::Act(command) if command.moves() => Answer::Act(command),
                Answer::Held => Answer::Held,
                Answer::Act(_) | Answer::None => Answer::None,
            },
            answer => answer,
        }
    }

    fn answer_in(&self, mode: Mode, region: Region, keys: &[&str]) -> Answer {
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
    use super::{
        Across, Act, Answer, Command, Direction, Follow, Keymap, Mode, Motion, Pick, Place, Prompt,
        Reach, Region, Sent, Toward, bind,
    };

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
    fn the_forms_keep_control_w_and_g_t() {
        for region in [Region::Goal, Region::Settings] {
            for keys in [&["ctrl-w"][..], &["ctrl-w", "h"], &["g", "t"]] {
                assert_eq!(
                    Keymap::VIM.answer(Mode::Normal, region, keys),
                    Answer::None,
                    "{region:?} {keys:?}"
                );
            }
        }
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Goal, &["g"]),
            Answer::None
        );
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Settings, &["g"]),
            Answer::Held,
            "toward gg"
        );
        assert!(Keymap::VIM.knows(Region::Code));
        assert!(Keymap::VIM.knows(Region::Workspace));
    }

    #[test]
    fn a_program_leaves_insert_mode_with_control_backslash_control_n_only() {
        for region in [Region::Terminal, Region::Android] {
            let answer = |mode, keys: &[&str]| Keymap::VIM.answer(mode, region, keys);
            assert_eq!(answer(Mode::Insert, &["ctrl-\\"]), Answer::Held);
            assert_eq!(
                answer(Mode::Insert, &["ctrl-\\", "ctrl-n"]),
                Answer::Act(Command::Normal)
            );
            for keys in [&["escape"][..], &["ctrl-w"], &["g"], &[":"], &["i"]] {
                assert_eq!(answer(Mode::Insert, keys), Answer::None, "{keys:?}");
            }
            assert_eq!(
                answer(Mode::Normal, &["i"]),
                Answer::Act(Command::Insert(Place::Cursor))
            );
            assert_eq!(
                answer(Mode::Normal, &["ctrl-w", "l"]),
                Answer::Act(Command::Window(Act::Go(Toward::Right)))
            );
            assert_eq!(
                answer(Mode::Normal, &["g", "t"]),
                Answer::Act(Command::Window(Act::NextTab))
            );
            assert_eq!(
                answer(Mode::Normal, &[":"]),
                Answer::Act(Command::Line(Prompt::Ex))
            );
        }
        let terminal = |keys: &[&str]| Keymap::VIM.answer(Mode::Normal, Region::Terminal, keys);
        assert_eq!(
            terminal(&["a"]),
            Answer::Act(Command::Insert(Place::Cursor))
        );
        assert_eq!(
            terminal(&["g", "g"]),
            Answer::Act(Command::Move(Motion::Top))
        );
        assert_eq!(
            terminal(&["shift-v"]),
            Answer::Act(Command::Send(Sent::Visual))
        );
        assert_eq!(terminal(&["y"]), Answer::Act(Command::Send(Sent::Copy)));
        assert_eq!(
            terminal(&["/"]),
            Answer::Act(Command::Line(Prompt::Search(Direction::Forward)))
        );
        let android = |keys: &[&str]| Keymap::VIM.answer(Mode::Normal, Region::Android, keys);
        for keys in [&["a"][..], &["j"], &["g", "g"], &["y"], &["/"]] {
            assert_eq!(android(keys), Answer::None, "{keys:?}");
        }
    }

    #[test]
    fn control_w_and_the_tabs_keys_work_in_every_part_in_normal_mode() {
        let answer = |region, keys: &[&str]| Keymap::VIM.answer(Mode::Normal, region, keys);
        for region in [
            Region::Chat,
            Region::Rail,
            Region::List,
            Region::Page,
            Region::Code,
        ] {
            assert_eq!(answer(region, &["ctrl-w"]), Answer::Held, "{region:?}");
            assert_eq!(
                answer(region, &["ctrl-w", "h"]),
                Answer::Act(Command::Window(Act::Go(Toward::Left)))
            );
            assert_eq!(
                answer(region, &["ctrl-w", "l"]),
                Answer::Act(Command::Window(Act::Go(Toward::Right)))
            );
            assert_eq!(
                answer(region, &["ctrl-w", "w"]),
                Answer::Act(Command::Window(Act::Go(Toward::Round)))
            );
            assert_eq!(
                answer(region, &["ctrl-w", "c"]),
                Answer::Act(Command::Window(Act::CloseTab))
            );
            assert_eq!(
                answer(region, &["g", "t"]),
                Answer::Act(Command::Window(Act::NextTab))
            );
            assert_eq!(
                answer(region, &["g", "shift-t"]),
                Answer::Act(Command::Window(Act::PreviousTab))
            );
        }
        assert_eq!(
            answer(Region::Chat, &["ctrl-w", "k"]),
            Answer::Act(Command::Window(Act::Go(Toward::Up)))
        );
        assert_eq!(
            answer(Region::List, &["escape"]),
            Answer::Act(Command::Window(Act::Go(Toward::Down)))
        );
        assert_eq!(
            Keymap::VIM.answer(Mode::Insert, Region::Chat, &["ctrl-w"]),
            Answer::None
        );
    }

    #[test]
    fn the_rail_and_the_list_move_a_selection_and_open_it() {
        for region in [Region::Rail, Region::List] {
            let answer = |keys: &[&str]| Keymap::VIM.answer(Mode::Normal, region, keys);
            let pick = |pick| Answer::Act(Command::Window(Act::Pick(pick)));
            assert_eq!(answer(&["j"]), pick(Pick::Down));
            assert_eq!(answer(&["k"]), pick(Pick::Up));
            assert_eq!(answer(&["g", "g"]), pick(Pick::First));
            assert_eq!(answer(&["shift-g"]), pick(Pick::Last));
            assert_eq!(answer(&["enter"]), Answer::Act(Command::Window(Act::Open)));
            assert_eq!(answer(&["l"]), Answer::Act(Command::Window(Act::Open)));
            assert_eq!(answer(&["o"]), Answer::Act(Command::Window(Act::New)));
        }
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Chat, &["j"]),
            Answer::Act(Command::Move(Motion::Down))
        );
    }

    #[test]
    fn the_conversation_and_the_quiet_tabs_move_with_j_k_and_the_rest() {
        let moves = [
            (&["j"][..], Motion::Down),
            (&["k"], Motion::Up),
            (&["ctrl-d"], Motion::HalfDown),
            (&["ctrl-u"], Motion::HalfUp),
            (&["g", "g"], Motion::Top),
            (&["shift-g"], Motion::Bottom),
        ];
        for region in [
            Region::Chat,
            Region::Page,
            Region::Code,
            Region::Document,
            Region::Tree,
            Region::Changes,
        ] {
            for (keys, motion) in moves {
                assert_eq!(
                    Keymap::VIM.answer(Mode::Normal, region, keys),
                    Answer::Act(Command::Move(motion)),
                    "{region:?} {keys:?}"
                );
                assert_eq!(
                    Keymap::VIM.answer(Mode::Insert, region, keys),
                    Answer::None,
                    "{region:?} {keys:?}"
                );
            }
        }
        for region in [Region::Android, Region::Goal] {
            assert_eq!(
                Keymap::VIM.answer(Mode::Normal, region, &["j"]),
                Answer::None
            );
        }
    }

    #[test]
    fn a_page_goes_back_and_forth_reloads_hints_and_opens_as_vimperator_does() {
        let answer = |keys: &[&str]| Keymap::VIM.answer(Mode::Normal, Region::Page, keys);
        let sent = |sent| Answer::Act(Command::Send(sent));
        assert_eq!(answer(&["shift-h"]), sent(Sent::Back));
        assert_eq!(answer(&["shift-l"]), sent(Sent::Forward));
        assert_eq!(answer(&["r"]), sent(Sent::Reload));
        assert_eq!(answer(&["g"]), Answer::Held);
        assert_eq!(answer(&["g", "i"]), sent(Sent::FocusField));
        assert_eq!(answer(&["y"]), Answer::Held);
        assert_eq!(answer(&["y", "y"]), sent(Sent::CopyAddress));
        assert_eq!(answer(&["o"]), sent(Sent::Address));
        assert_eq!(answer(&["t"]), Answer::Act(Command::Window(Act::Menu)));
        assert_eq!(answer(&["f"]), Answer::Act(Command::Hint(Follow::Here)));
        assert_eq!(answer(&[";"]), Answer::Held);
        assert_eq!(
            answer(&[";", "t"]),
            Answer::Act(Command::Hint(Follow::NewTab))
        );
        assert_eq!(
            answer(&["shift-f"]),
            Answer::Act(Command::Hints(Reach::Window))
        );
        assert_eq!(answer(&["i"]), Answer::Act(Command::Insert(Place::Cursor)));
        assert_eq!(answer(&["escape"]), Answer::None);
    }

    #[test]
    fn escape_leaves_a_page_s_field_its_hints_and_its_address_for_normal_mode() {
        for (mode, region) in [
            (Mode::Insert, Region::Page),
            (Mode::Hint, Region::Page),
            (Mode::Insert, Region::Address),
        ] {
            assert_eq!(
                Keymap::VIM.answer(mode, region, &["escape"]),
                Answer::Act(Command::Normal),
                "{mode:?} {region:?}"
            );
        }
        for keys in [&["j"][..], &["f"], &["ctrl-w"], &["shift-h"]] {
            assert_eq!(
                Keymap::VIM.answer(Mode::Insert, Region::Page, keys),
                Answer::None,
                "{keys:?}"
            );
        }
    }

    #[test]
    fn f_hints_the_part_with_the_keyboard_and_shift_f_the_whole_window() {
        for region in [
            Region::Rail,
            Region::List,
            Region::Chat,
            Region::Code,
            Region::Terminal,
            Region::Settings,
            Region::ProjectCard,
        ] {
            let answer = |keys: &str| Keymap::VIM.answer(Mode::Normal, region, &[keys]);
            assert_eq!(
                answer("f"),
                Answer::Act(Command::Hints(Reach::Part)),
                "{region:?}"
            );
            assert_eq!(
                answer("shift-f"),
                Answer::Act(Command::Hints(Reach::Window)),
                "{region:?}"
            );
            assert_eq!(
                Keymap::VIM.answer(Mode::Hint, region, &["escape"]),
                Answer::Act(Command::Normal),
                "{region:?}"
            );
            assert_eq!(
                Keymap::VIM.answer(Mode::Insert, region, &["f"]),
                Answer::None,
                "{region:?}"
            );
        }
        for region in [Region::Goal, Region::Address, Region::Command] {
            assert_eq!(
                Keymap::VIM.answer(Mode::Normal, region, &["f"]),
                Answer::None,
                "{region:?}"
            );
        }
    }

    #[test]
    fn code_selects_lines_copies_and_hands_them_and_goes_back_and_forth() {
        let answer = |keys: &[&str]| Keymap::VIM.answer(Mode::Normal, Region::Code, keys);
        let sent = |sent| Answer::Act(Command::Send(sent));
        assert_eq!(answer(&["shift-v"]), sent(Sent::Visual));
        assert_eq!(answer(&["y"]), sent(Sent::Copy));
        assert_eq!(answer(&["g", "a"]), sent(Sent::AddToChat));
        assert_eq!(answer(&["g", "d"]), sent(Sent::Definition));
        assert_eq!(answer(&["ctrl-o"]), sent(Sent::Back));
        assert_eq!(answer(&["ctrl-i"]), sent(Sent::Forward));
    }

    #[test]
    fn a_document_goes_from_block_to_block_copies_and_hands_them() {
        let answer = |keys: &[&str]| Keymap::VIM.answer(Mode::Normal, Region::Document, keys);
        let sent = |sent| Answer::Act(Command::Send(sent));
        assert_eq!(answer(&["]", "]"]), sent(Sent::NextEntry));
        assert_eq!(answer(&["[", "["]), sent(Sent::PreviousEntry));
        assert_eq!(answer(&["y"]), sent(Sent::Copy));
        assert_eq!(answer(&["g", "a"]), sent(Sent::AddToChat));
    }

    #[test]
    fn code_moves_its_cursor_by_characters_and_words() {
        let code = |key: &str| Keymap::VIM.answer(Mode::Normal, Region::Code, &[key]);
        for (key, across) in [
            ("h", Across::Left),
            ("l", Across::Right),
            ("w", Across::WordForward),
            ("b", Across::WordBack),
            ("e", Across::WordEnd),
            ("0", Across::LineStart),
            ("^", Across::FirstNonBlank),
            ("$", Across::LineEnd),
            ("%", Across::Bracket),
            ("}", Across::ParagraphForward),
            ("{", Across::ParagraphBack),
        ] {
            assert_eq!(code(key), Answer::Act(Command::Across(across)), "{key}");
        }
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Tree, &["w"]),
            Answer::None,
            "only code has a cursor in a line"
        );
    }

    #[test]
    fn the_tree_opens_and_closes_and_changes_open() {
        let tree = |keys: &[&str]| Keymap::VIM.answer(Mode::Normal, Region::Tree, keys);
        assert_eq!(tree(&["l"]), Answer::Act(Command::Send(Sent::Open)));
        assert_eq!(tree(&["enter"]), Answer::Act(Command::Send(Sent::Open)));
        assert_eq!(tree(&["h"]), Answer::Act(Command::Send(Sent::Close)));
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Changes, &["enter"]),
            Answer::Act(Command::Send(Sent::Open))
        );
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Changes, &["h"]),
            Answer::None
        );
    }

    #[test]
    fn colon_opens_the_line_from_every_part_and_slash_from_those_with_text() {
        for region in [
            Region::Chat,
            Region::Rail,
            Region::Page,
            Region::Code,
            Region::Tree,
        ] {
            assert_eq!(
                Keymap::VIM.answer(Mode::Normal, region, &[":"]),
                Answer::Act(Command::Line(Prompt::Ex)),
                "{region:?}"
            );
        }
        for region in [Region::Page, Region::Code, Region::Document] {
            let answer = |keys: &str| Keymap::VIM.answer(Mode::Normal, region, &[keys]);
            assert_eq!(
                answer("/"),
                Answer::Act(Command::Line(Prompt::Search(Direction::Forward)))
            );
            assert_eq!(
                answer("?"),
                Answer::Act(Command::Line(Prompt::Search(Direction::Backward)))
            );
            assert_eq!(answer("n"), Answer::Act(Command::Again(Direction::Forward)));
            assert_eq!(
                answer("shift-n"),
                Answer::Act(Command::Again(Direction::Backward))
            );
        }
        for (mode, region) in [
            (Mode::Insert, Region::Chat),
            (Mode::Insert, Region::Terminal),
            (Mode::Normal, Region::Goal),
        ] {
            assert_eq!(Keymap::VIM.answer(mode, region, &[":"]), Answer::None);
        }
        assert_eq!(
            Keymap::VIM.answer(Mode::Normal, Region::Tree, &["/"]),
            Answer::None
        );
    }

    #[test]
    fn the_line_runs_cycles_and_goes_through_its_history() {
        let answer = |keys: &str| Keymap::VIM.answer(Mode::Command, Region::Command, &[keys]);
        let sent = |sent| Answer::Act(Command::Send(sent));
        assert_eq!(answer("escape"), Answer::Act(Command::Normal));
        assert_eq!(answer("enter"), sent(Sent::Accept));
        assert_eq!(answer("tab"), sent(Sent::NextFit));
        assert_eq!(answer("ctrl-n"), sent(Sent::NextFit));
        assert_eq!(answer("shift-tab"), sent(Sent::PreviousFit));
        assert_eq!(answer("ctrl-p"), sent(Sent::PreviousFit));
        assert_eq!(answer("up"), sent(Sent::Older));
        assert_eq!(answer("down"), sent(Sent::Newer));
        assert_eq!(answer("q"), Answer::None);
        assert_eq!(answer("j"), Answer::None);
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
