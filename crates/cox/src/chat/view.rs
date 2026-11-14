//! One chat in the window: draws its conversation, in the panel at the
//! window's right, and turns keystrokes and clicks into what the human
//! does, which it tells the daemon, which runs the chat (see
//! [`crate::daemon`]). Escape in the conversation, or its Stop button,
//! stops the turn the agent is taking.
//!
//! The conversation the view draws is the daemon's, as the daemon tells
//! the window how it changes (see [`ChatView::apply`]): the view changes
//! none of it itself.
//!
//! The chat's stage, which the chats of its folder share, is the window's
//! to draw, in its middle (see [`crate::workspace`]): the view keeps it
//! for what the chat opens on it.
//!
//! The human selects the conversation's text with the mouse, a word with
//! a double click and a whole paragraph or block with a triple click, and
//! copies it with the usual key while the input has nothing selected. A
//! click on a link opens it on the chat's stage.
//!
//! The entries of the conversation are drawn in [`entries`]: what the
//! human sent, the agent's replies, and what its tool calls and
//! permission requests show. While the agent waits for an answer to a
//! permission request, the number of an option chooses it. The plan the
//! agent works to stays in sight on a card at the top right of the
//! conversation, after its turn too, until the human sends the next line,
//! and a click on its line folds it to that line. The input, while it is
//! empty, tells what the chat waits for, when it is not the human's next
//! line, and the goal the chat works toward, while it has one; the human
//! sets the goal in an input that opens above it (see
//! [`ChatView::edit_goal`]).
//!
//! A pill above the conversation shows the agent and what it lets the
//! human choose of its session, such as its model,
//! each as it is set, and a click on it opens a list of them, from which
//! each opens its choices (see [`settings`]).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// How long an entry the human asked to see stays marked.
const MARKED_FOR: Duration = Duration::from_millis(1600);

use cox_core::entry::Attachment;
use cox_core::goal;
use cox_core::tabs::KeptTabs;
use gpui::{
    AnyView, App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, KeyBinding,
    KeyDownEvent, ListAlignment, ListOffset, ListState, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Subscription, Window, actions, div, list,
    prelude::*, px, relative, rgb,
};

use crate::chat::region::{self, AddRegionToChat, Described};
use crate::daemon::Link;
use crate::daemon::wire::ToDaemon;
use crate::stage::Stage;
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{self, LineInput, Submitted};
use crate::ui::selected::{self, Selected};
use crate::ui::selection::Spot;
use crate::ui::text::Reading;
use crate::ui::text::cursor::Step;
use crate::ui::{Copy, Panel, tip};
use coxd::agents::Agents;
use coxd::chat::conversation::{Answer, Conversation, GOAL_TURNS, Goal, Patch, Proposed, State};
use coxd::chat::sketch::Sketch;
use coxd::theme::{
    ACCENT, ACCENT_GROUND, BORDER, BUTTON, BUTTON_HOVER, CODE_FONT, DIM, FAINT, FOREGROUND,
    OFFERING_EDGE, RAISED, UI_FONT,
};

use crate::vim::keymap::{Across, Motion, Place, Selects};

mod complete;
mod entries;
mod find;
mod settings;

use crate::vim::hints::Hinted as _;
use entries::{Folds, disclosure, progress, render_steps};

actions!(
    cox,
    [
        /// Stops the turn the agent is taking.
        Stop,
        /// Leaves the chat's goal as it was.
        CancelGoal,
        /// Closes the input that starts a sketch, starting none.
        CancelSketch,
        /// Closes the list over the input, as Escape does without Vim's
        /// keys, if it is open.
        CloseList,
    ]
);

pub(crate) const KEY_CONTEXT: &str = "Conversation";

/// The key context of the conversation, while it has the keyboard, for
/// Vim's keys to move a cursor through it.
pub(crate) const READING: &str = "ChatReading";

/// The key context of the input that sets the chat's goal.
pub(crate) const GOAL: &str = "Goal";

/// The key context of the input that starts a sketch of the chat.
pub(crate) const SKETCH: &str = "Sketch";

/// Binds the keys of the conversation. Escape is the stage's while the
/// stage has the keyboard, and the input method's while it composes.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Stop, Some(KEY_CONTEXT)),
        // The secondary key is Command on macOS, Control elsewhere.
        KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", CancelGoal, Some(GOAL)),
        KeyBinding::new("escape", CancelSketch, Some(SKETCH)),
    ]);
}

/// Opens a link the human clicked: on the chat's stage, where it has
/// one.
pub type OpenLink = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// The bar of a turn toward the goal whose check failed: the color of
/// what failed, dimmed, so that the turn the agent takes stands out.
const FAILED_TURN: u32 = 0x8c_4a48;

/// How wide the card of the plan in sight is, and how tall its steps grow
/// before they scroll on their own.
const PLAN_WIDTH: f32 = 240.;
const PLAN_HEIGHT: f32 = 200.;

/// How far past the conversation in sight the list measures the places
/// it has not laid out yet, so that they are ready as it scrolls.
const OVERDRAW: f32 = 400.;

/// Does what Vim's keys ask of a stage's tabs.
pub type ByKeys = Rc<dyn Fn(crate::stage::Tabs, &mut Window, &mut App)>;

/// Whether a stage shows the tree of the project's files beside the tab
/// shown, which `ctrl-w` then stops at, as at a window of its own.
pub type ShowsFiles = Rc<dyn Fn(&App) -> bool>;

/// A chat's stage, as the window shows it: the view, whether it is in
/// sight, what does what Vim's keys ask of its tabs, and whether it
/// shows the tree of files.
#[derive(Clone)]
pub struct StageView {
    pub view: AnyView,
    pub panel: Entity<Panel>,
    pub by_keys: ByKeys,
    pub shows_files: ShowsFiles,
}

/// How a chat stands in the list, as far as its view shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// Whether its agent runs.
    pub open: bool,
    /// Whether a line sent to it, closed, starts its agent again.
    pub again: bool,
    /// The id of the agent it talks to.
    pub agent: String,
    /// The folder it works in.
    pub cwd: PathBuf,
    /// Whether the folder of its project is not there, as when the human
    /// moved it, so that it does not start again until they say where.
    pub gone: bool,
    /// How its agent is set up, if the chat that opened it said: whether
    /// it merges what it made when the human asks, as one that works in
    /// the project does, and a sketch's, which ends its own ways, does not.
    pub setup: Option<coxd::agent::setup::Setup>,
}

/// Where the frame after the next puts the cursor of Vim's keys in the
/// conversation, once the conversation scrolled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Settle {
    /// Where it is, if it shows, or else on the nearest row that does.
    IntoSight,
    /// On the first row in sight (`gg`).
    First,
    /// On the last row in sight (`G`).
    Last,
}

pub struct ChatView {
    /// What the chat goes by to the daemon.
    key: u64,
    /// Where what the human does goes.
    link: Link,
    /// The conversation, as the daemon last told it.
    conversation: Conversation,
    /// How the chat stands in the list.
    row: Row,
    /// The conversation's places as the list draws them: where it is
    /// scrolled to, and how tall each place was when it last showed, so
    /// that only the places in sight are laid out again as it scrolls.
    list: ListState,
    /// The places of the conversation, as the last frame worked them out.
    places: Vec<entries::Place>,
    /// Whether the next frame scrolls the conversation to its end, to
    /// keep it in view as entries arrive.
    to_end: bool,
    /// The entry the next frame scrolls the conversation to, as the human
    /// asked to see it, once the conversation has it.
    to_entry: Option<usize>,
    /// The entry marked a moment, as one the human asked to see, while it
    /// is.
    pub(super) marked: Option<usize>,
    /// The line the human is typing.
    input: Entity<LineInput>,
    /// The list over the input of the agent's commands the line can go on
    /// to run, and of the project's files it can point to.
    completion: complete::Completion,
    /// Where the agent and the human work together, which the window
    /// shows beside the conversation.
    stage: Option<StageView>,
    open_link: OpenLink,
    /// The conversation's text, and what the human selects of it.
    reading: Reading,
    /// The conversation, as it has the keyboard, for Vim's keys to move a
    /// cursor through it.
    reading_focus: FocusHandle,
    /// Where the next frame puts the cursor, once the conversation
    /// scrolled.
    settle: Option<Settle>,
    /// Whether the human scrolled the conversation with the mouse since
    /// the last frame.
    wheeled: Rc<Cell<bool>>,
    /// The search of Vim's `/` going through the conversation, while it
    /// goes on.
    finding: Option<find::Finding>,
    /// What the human opened and closed of the conversation.
    folds: Folds,
    /// Whether the human folded the card of the plan to its line.
    plan_folded: bool,
    /// The input the human sets the goal in, while it is open, with the
    /// index of the proposed goal it changes, when it changes one.
    goal_input: Option<(Entity<LineInput>, Option<usize>, Subscription)>,
    /// The input the human says what a sketch of the chat tries in, while
    /// it is open, with what the sketch is to start from.
    sketch_input: Option<(Entity<LineInput>, Sketch, Subscription)>,
    /// The setting whose choices are open, by its place in the
    /// conversation's settings, while they are.
    setting_open: Option<settings::Menu>,
    /// Where the press that last closed the list of a setting's choices
    /// was, so that the press on the setting's button does not open it
    /// again.
    setting_closed_at: Option<Point<Pixels>>,
    /// The agents the chat can talk to.
    agents: Option<Agents>,
    /// What the human handed the chat from elsewhere on the screen, which
    /// goes with the next line they send.
    attached: Vec<Attachment>,
    _submitted: Subscription,
}

impl ChatView {
    /// The view of the chat the daemon, reached through `link`, knows as
    /// `key`, standing as `row` says, with no stage beside it until it is
    /// given one (see [`Self::set_stage`]), and nothing in it until the
    /// daemon tells it its conversation. The chat talks to one of
    /// `agents`, and lets the human choose another of them, when given.
    pub fn new(
        key: u64,
        link: Link,
        row: Row,
        agents: Option<Agents>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(LineInput::wrapping);
        let submitted = cx.subscribe(&input, |view, input, Submitted, cx| {
            view.submit(&input, cx);
        });
        let completion = complete::Completion::new(&input, cx);
        let list = ListState::new(0, ListAlignment::Top, px(OVERDRAW));
        let wheeled = Rc::new(Cell::new(false));
        list.set_scroll_handler({
            let wheeled = Rc::clone(&wheeled);
            move |_, _, _| wheeled.set(true)
        });
        Self {
            key,
            link,
            conversation: Conversation::closed(Vec::new()),
            row,
            list,
            places: Vec::new(),
            to_end: true,
            to_entry: None,
            marked: None,
            input,
            completion,
            stage: None,
            open_link: Rc::new(|_, _, _| {}),
            reading: Reading::default(),
            reading_focus: cx.focus_handle(),
            settle: None,
            wheeled,
            finding: None,
            folds: Folds::default(),
            plan_folded: false,
            goal_input: None,
            sketch_input: None,
            setting_open: None,
            setting_closed_at: None,
            agents,
            attached: Vec::new(),
            _submitted: submitted,
        }
    }

    /// Takes `attachment`, which the human handed the chat, to go with
    /// the next line they send, unless it has it already, and gives the
    /// keyboard to the input, for them to write the line.
    pub fn attach(&mut self, attachment: Attachment, window: &mut Window, cx: &mut Context<Self>) {
        if !self.attached.contains(&attachment) {
            self.attached.push(attachment);
        }
        window.focus(&self.input.focus_handle(cx));
        cx.notify();
    }

    /// What the human handed the chat, which goes with the next line.
    #[must_use]
    pub fn attached(&self) -> &[Attachment] {
        &self.attached
    }

    /// What of the conversation shows under `over`, a stretch of a region
    /// in the window's points.
    fn described(&self, over: Bounds<Pixels>) -> Described {
        let text = self.reading.text_within(over);
        let shown = if text.trim().is_empty() {
            String::new()
        } else {
            format!(", where it reads:\n\n{}", region::fenced(text.trim()))
        };
        Described {
            name: "Conversation".to_owned(),
            text: format!(
                "the conversation of the chat cox://chat/{}{shown}",
                self.key
            ),
        }
    }

    /// The right button went down on the conversation: offers what to do
    /// with the text selected, if any is.
    fn offer_selection(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let handed = self.attachment();
        if crate::ui::translation::offer(&self.selected_text(), handed, window, cx) {
            cx.stop_propagation();
        }
    }

    /// What the chat goes by to the daemon.
    #[must_use]
    pub fn key(&self) -> u64 {
        self.key
    }

    /// Gives the chat `stage`, where `open_link` opens the links the
    /// human clicks.
    pub fn set_stage(&mut self, stage: StageView, open_link: OpenLink, cx: &mut Context<Self>) {
        self.stage = Some(stage);
        self.open_link = open_link;
        cx.notify();
    }

    /// Changes the conversation as the daemon says it changed.
    pub fn apply(&mut self, patch: Patch, cx: &mut Context<Self>) {
        self.follow(|conversation| conversation.apply_patch(patch));
        cx.notify();
    }

    /// Follows how the chat stands in the list now.
    pub fn follow_row(&mut self, row: Row, cx: &mut Context<Self>) {
        if self.row != row {
            self.row = row;
            cx.notify();
        }
    }

    /// Tells the daemon what the human did in the chat.
    fn tell(&self, message: ToDaemon) {
        self.link.send(message);
    }

    /// Sets the chat's goal to `goal`, or clears it, as the input that
    /// sets the goal does.
    pub fn set_goal(&self, goal: Option<goal::Goal>) {
        self.tell(ToDaemon::SetGoal {
            key: self.key,
            goal,
        });
    }

    /// The agents the chat can talk to, when it can choose.
    #[must_use]
    pub fn agents(&self) -> Option<&Agents> {
        self.agents.as_ref()
    }

    /// The id of the agent the chat talks to.
    #[must_use]
    pub fn agent_id(&self) -> &str {
        &self.row.agent
    }

    /// What the human knows the agent the chat talks to by: its name,
    /// among those it can choose, or "Agent" when it cannot choose.
    fn agent_name(&self) -> String {
        self.agents
            .as_ref()
            .and_then(|agents| {
                agents
                    .all()
                    .into_iter()
                    .find(|known| known.id == self.row.agent)
                    .map(|known| known.name)
            })
            .unwrap_or_else(|| "Agent".to_owned())
    }

    /// Asks the daemon to switch the chat to the agent kept as `id`, when
    /// it is another and no turn runs.
    fn choose_agent(&mut self, id: &str, cx: &mut Context<Self>) {
        self.setting_open = None;
        let idle = matches!(self.conversation.state(), State::Idle | State::Closed);
        if idle && id != self.agent_id() {
            self.tell(ToDaemon::SwitchAgent {
                key: self.key,
                agent: id.to_owned(),
            });
        }
        cx.notify();
    }

    /// Sets the setting `id` of the chat's session to `value`, as the human
    /// chose.
    fn set(&mut self, id: &str, value: &str, cx: &mut Context<Self>) {
        self.setting_open = None;
        self.tell(ToDaemon::Set {
            key: self.key,
            id: id.to_owned(),
            value: value.to_owned(),
        });
        cx.notify();
    }

    /// Opens an input above the chat's own, for the human to set the goal:
    /// words, or `$ ` and the command the chat works until it passes,
    /// which Enter sets, and Escape leaves as it was. Emptied, the input
    /// clears the goal.
    pub fn edit_goal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let line = self
            .conversation
            .goal()
            .map(|goal| goal.what().input())
            .unwrap_or_default();
        self.open_goal_input(&line, None, window, cx);
    }

    /// Opens the input that sets the goal on `line`, which changes the
    /// goal proposed at `proposal`, when given.
    fn open_goal_input(
        &mut self,
        line: &str,
        proposal: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_text(line, cx);
            input.select_whole(cx);
            input
        });
        let submitted = cx.subscribe_in(&input, window, |view, _, Submitted, window, cx| {
            view.end_editing_goal(true, window, cx);
        });
        window.focus(&input.focus_handle(cx));
        self.goal_input = Some((input, proposal, submitted));
        cx.notify();
    }

    /// Closes the input that sets the goal, setting the goal to what it
    /// holds when `keep` says to, and brings the keyboard back to the
    /// chat's input.
    fn end_editing_goal(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((input, proposal, _)) = self.goal_input.take() else {
            return;
        };
        if keep {
            let goal = goal::Goal::parse(input.read(cx).text());
            match proposal {
                None => self.tell(ToDaemon::SetGoal {
                    key: self.key,
                    goal,
                }),
                // Emptied, the input changes the goal proposed into none,
                // and leaves both goals as they were.
                Some(index) => {
                    if let Some(goal) = goal {
                        self.tell(ToDaemon::ChangeProposal {
                            key: self.key,
                            index,
                            goal,
                        });
                    }
                }
            }
        }
        window.focus(&self.input.focus_handle(cx));
        cx.notify();
    }

    /// Opens an input above the chat's own, for the human to say what a
    /// sketch of the chat is to try, such as what to find out, and to
    /// choose what it starts from, `from` at first. Enter makes the chat a
    /// sketch of it, as an agent's `start_sketch` does, and Escape closes the
    /// input, making none.
    pub fn edit_sketch(&mut self, from: Sketch, window: &mut Window, cx: &mut Context<Self>) {
        self.goal_input = None;
        let input = cx.new(LineInput::new);
        let submitted = cx.subscribe_in(&input, window, |view, _, Submitted, window, cx| {
            view.end_editing_sketch(true, window, cx);
        });
        window.focus(&input.focus_handle(cx));
        self.sketch_input = Some((input, from, submitted));
        cx.notify();
    }

    /// Changes what the sketch the input starts starts from: the project,
    /// or nothing, and gives the input back the keyboard.
    fn flip_sketch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((input, from, _)) = &mut self.sketch_input {
            *from = match from {
                Sketch::Project => Sketch::Empty,
                Sketch::Empty => Sketch::Project,
            };
            window.focus(&input.focus_handle(cx));
            cx.notify();
        }
    }

    /// Closes the input that starts a sketch, and, when `start` says to and
    /// it says what to try, asks the daemon to open a sketch of it beside
    /// the chat, and show it; the chat notes why when it cannot open.
    fn end_editing_sketch(&mut self, start: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((input, sketch, _)) = self.sketch_input.take() else {
            return;
        };
        window.focus(&self.input.focus_handle(cx));
        cx.notify();
        let what = input.read(cx).text().trim().to_owned();
        if start && !what.is_empty() {
            self.tell(ToDaemon::StartSketch {
                key: self.key,
                sketch,
                what,
            });
        }
    }

    /// The human answered what was proposed at `index`: carry it out, or
    /// let it be.
    fn answer_proposal(&mut self, index: usize, answer: Answer, _: &mut Context<Self>) {
        self.tell(ToDaemon::AnswerProposal {
            key: self.key,
            index,
            answer,
        });
    }

    /// The human chose, on the card at `index`, whether to keep what a
    /// chat learned in the wiki.
    fn answer_keep(&mut self, index: usize, keep: bool, _: &mut Context<Self>) {
        self.tell(ToDaemon::AnswerKeep {
            key: self.key,
            index,
            keep,
        });
    }

    /// Asks the agent, in the human's place, to merge what it made, the
    /// human being done looking at it.
    fn merge(&self) {
        self.tell(ToDaemon::Merge { key: self.key });
    }

    /// Opens the input that sets the goal on the goal proposed at `index`,
    /// for the human to change it before they set it.
    fn change_proposal(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(goal) = self.conversation.proposed(index).and_then(Proposed::goal) {
            self.open_goal_input(&goal.input(), Some(index), window, cx);
        }
    }

    /// Whether the chat's agent runs.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.row.open
    }

    /// The folder the chat's agent works in, while it runs, whose files
    /// the human's line points to.
    fn cwd(&self) -> Option<PathBuf> {
        self.row.open.then(|| self.row.cwd.clone())
    }

    /// Whether the chat's stage is in sight, in the middle of the window.
    #[must_use]
    pub fn stage_in_sight(&self, cx: &App) -> bool {
        self.stage
            .as_ref()
            .is_some_and(|stage| stage.panel.read(cx).is_open())
    }

    #[must_use]
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    #[must_use]
    pub fn input(&self) -> &Entity<LineInput> {
        &self.input
    }

    /// The input the human sets the goal in, while it is open.
    #[must_use]
    pub fn goal_input(&self) -> Option<&Entity<LineInput>> {
        self.goal_input.as_ref().map(|(input, _, _)| input)
    }

    /// The input the human says what a sketch tries in, while it is open,
    /// with what the sketch is to start from.
    #[must_use]
    pub fn sketch_input(&self) -> Option<(&Entity<LineInput>, Sketch)> {
        self.sketch_input
            .as_ref()
            .map(|(input, sketch, _)| (input, *sketch))
    }

    /// The stage's tabs, as they are kept, each of a chat by its key,
    /// when the chat has a stage.
    #[must_use]
    pub fn tabs(&self, cx: &App) -> Option<KeptTabs<u64>> {
        let stage = self.stage.as_ref()?.view.clone().downcast::<Stage>().ok()?;
        Some(stage.read(cx).tabs(cx))
    }

    /// The chat's stage, when it has one.
    #[must_use]
    pub fn stage(&self) -> Option<Entity<Stage>> {
        self.stage.as_ref()?.view.clone().downcast::<Stage>().ok()
    }

    /// Where the conversation is scrolled to.
    #[must_use]
    pub fn scroll(&self) -> &ListState {
        &self.list
    }

    /// The text the human selected in the conversation.
    #[must_use]
    pub fn selected_text(&self) -> String {
        self.reading.selected_text()
    }

    /// A button went down on the conversation: start selecting there, a
    /// word or a whole piece of text at a time on a double or triple
    /// click. The keyboard goes to the input, so that the human copies
    /// what they select, and types on.
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.input.focus_handle(cx));
        window.prevent_default();
        self.input.update(cx, LineInput::deselect);
        self.reading.press(event);
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.reading.drag(event.position) {
            cx.notify();
        }
    }

    /// The button came up: a click that selected nothing opens the link
    /// under it.
    fn mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.reading.release(event.position) {
            self.show_link(&url, window, cx);
        }
    }

    /// Does `act` on the stage, when the chat has one.
    pub fn on_stage(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Stage, &mut Window, &mut Context<Stage>),
    ) {
        let Some(stage) = &self.stage else {
            return;
        };
        let Ok(entity) = stage.view.clone().downcast::<Stage>() else {
            return;
        };
        entity.update(cx, |stage, cx| act(stage, window, cx));
    }

    /// Opens `url`, a link the human clicked, on the stage.
    pub(super) fn show_link(&self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        (self.open_link)(url, window, cx);
    }

    /// Gives the keyboard to the conversation, for Vim's keys to move a
    /// cursor through it: on the start of what is selected, or of the
    /// first row in sight, if it was nowhere yet.
    pub fn read_by_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.reading_focus);
        let view = self.list.viewport_bounds();
        self.reading.place_cursor(view.top(), view.bottom());
        self.settle = Some(Settle::IntoSight);
        crate::vim::came(crate::vim::Region::Reading, window, cx);
        cx.notify();
    }

    /// Whether the conversation has the keyboard, for Vim's keys.
    #[must_use]
    pub fn reads_by_keys(&self, window: &Window) -> bool {
        self.reading_focus.is_focused(window)
    }

    /// Where the cursor of Vim's keys is in the conversation, once they
    /// came to it.
    #[must_use]
    pub fn reading_cursor(&self) -> Option<Spot> {
        self.reading.cursor()
    }

    /// Moves the cursor of Vim's keys in the conversation as `step` says,
    /// `times` over, and scrolls its row into sight.
    fn step(&mut self, step: Step, times: usize, cx: &mut Context<Self>) {
        if let Some((top, bottom)) = self.reading.move_cursor(step, times) {
            let view = self.list.viewport_bounds();
            if top < view.top() {
                self.list.scroll_by(top - view.top());
            } else if bottom > view.bottom() {
                self.list.scroll_by(bottom - view.bottom());
            }
        }
        cx.notify();
    }

    /// Moves the cursor, or scrolls the conversation, as Vim's keys ask
    /// while it has the keyboard: `j` and `k` move the cursor a row, and
    /// the rest scroll as from the input, the cursor going to what shows.
    fn read_by_motion(
        &mut self,
        motion: &crate::vim::Move,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let times = motion.times();
        match motion.motion {
            Motion::Down => self.step(Step::Down, times, cx),
            Motion::Up => self.step(Step::Up, times, cx),
            Motion::HalfDown | Motion::HalfUp => {
                self.settle = Some(Settle::IntoSight);
                self.move_by_keys(motion, window, cx);
            }
            Motion::Top => {
                self.settle = Some(Settle::First);
                self.move_by_keys(motion, window, cx);
            }
            Motion::Bottom => {
                self.settle = Some(Settle::Last);
                self.move_by_keys(motion, window, cx);
            }
        }
    }

    /// Moves the cursor along the conversation's text, as Vim's keys ask
    /// while it has the keyboard.
    fn read_across(
        &mut self,
        across: &crate::vim::MoveAcross,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let step = match across.across {
            Across::Left => Step::Left,
            Across::Right => Step::Right,
            Across::WordForward => Step::WordForward,
            Across::WordBack => Step::WordBack,
            Across::WordEnd => Step::WordEnd,
            Across::LineStart => Step::RowStart,
            Across::FirstNonBlank => Step::FirstNonBlank,
            Across::LineEnd => Step::RowEnd,
            Across::Bracket | Across::ParagraphForward | Across::ParagraphBack => return,
        };
        let times = across
            .count
            .map_or(1, |count| usize::try_from(count).unwrap_or(usize::MAX));
        self.step(step, times, cx);
    }

    /// Starts selecting as `selects` says, from the cursor, or stops.
    fn read_visually(&mut self, selects: Selects, cx: &mut Context<Self>) {
        self.reading.visual(selects);
        cx.notify();
    }

    /// Gives the keyboard back to the input, to type in it, as `place`
    /// says.
    fn type_in_input(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.input.focus_handle(cx));
        match place {
            Place::Cursor => {}
            Place::After => window.dispatch_action(Box::new(input::Right), cx),
            Place::LineStart => window.dispatch_action(Box::new(input::Home), cx),
            Place::LineEnd => window.dispatch_action(Box::new(input::End), cx),
        }
        cx.notify();
    }

    /// The conversation's entries, which the human reads, selects with
    /// the mouse, and moves a cursor through with Vim's keys.
    fn render_reading(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("conversation")
            .track_focus(&self.reading_focus)
            .map(|conversation| Self::read_by_keys_on(conversation, cx))
            .size_full()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::offer_selection))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, _| view.reading.stop()),
            )
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|view, place, _, cx| view.render_place(place, cx)),
                )
                .size_full(),
            )
            .children(self.render_finding(cx))
            .children(self.render_settling(window, cx))
    }

    /// Draws the cursor of Vim's keys in the conversation while it has
    /// the keyboard. Visual mode is left with the keyboard; what it
    /// selected stays.
    fn follow_reading(&mut self, window: &Window, cx: &App) {
        let reads = crate::vim::mode(cx).is_some() && self.reads_by_keys(window);
        self.reading.show_cursor(reads);
        if !reads {
            self.reading.end_visual(true);
        }
    }

    /// Takes Vim's keys that move the cursor through the conversation and
    /// select with it on `element`, the conversation, while it has the
    /// keyboard.
    fn read_by_keys_on(
        element: gpui::Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        element
            .key_context(READING)
            .on_action(cx.listener(Self::read_by_motion))
            .on_action(cx.listener(Self::read_across))
            .on_action(
                cx.listener(|view, typing: &crate::vim::TypeInInput, window, cx| {
                    view.type_in_input(typing.place, window, cx);
                }),
            )
            .on_action(cx.listener(|view, _: &crate::vim::Visual, _, cx| {
                view.read_visually(Selects::Lines, cx);
            }))
            .on_action(
                cx.listener(|view, _: &crate::vim::VisualCharacters, _, cx| {
                    view.read_visually(Selects::Characters, cx);
                }),
            )
            .on_action(cx.listener(|view, _: &crate::vim::LeaveVisual, _, cx| {
                if view.reading.end_visual(false) {
                    cx.notify();
                }
            }))
    }

    /// Puts the cursor where the scroll left it room, once the frame laid
    /// the conversation out: on the first or last row in sight after `gg`
    /// and `G`, or into sight after the human scrolled.
    fn settle_cursor(&mut self, cx: &mut Context<Self>) {
        let settle = self
            .settle
            .take()
            .or_else(|| self.wheeled.take().then_some(Settle::IntoSight));
        let view = self.list.viewport_bounds();
        let moved = match settle {
            None => false,
            Some(Settle::IntoSight) => self.reading.cursor_into_sight(view.top(), view.bottom()),
            Some(Settle::First) => {
                self.reading
                    .cursor_to_end_in_sight(view.top(), view.bottom(), false)
            }
            Some(Settle::Last) => {
                self.reading
                    .cursor_to_end_in_sight(view.top(), view.bottom(), true)
            }
        };
        if moved {
            cx.notify();
        }
    }

    /// What puts the cursor once the conversation is painted, while it is
    /// to: right after the frame, which cannot ask for another.
    fn render_settling(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let settles = self.settle.is_some() || self.wheeled.get();
        if !settles || !self.reads_by_keys(window) {
            return None;
        }
        let view = cx.entity();
        Some(gpui::canvas(
            |_, _, _| {},
            move |_, (), window, cx| {
                window.defer(cx, move |_, cx| {
                    view.update(cx, ChatView::settle_cursor);
                });
            },
        ))
    }

    /// Scrolls the conversation as Vim's keys ask, while the keyboard is
    /// in the input in normal mode.
    fn move_by_keys(
        &mut self,
        motion: &crate::vim::Move,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match motion.motion {
            Motion::Top => self.list.scroll_to(ListOffset::default()),
            Motion::Bottom => self.list.scroll_to(ListOffset {
                item_ix: self.places.len(),
                offset_in_item: px(0.),
            }),
            _ => motion.scroll_list(&self.list, window.line_height()),
        }
        cx.notify();
    }

    /// Changes the conversation with `change`, keeping its end in view if
    /// it was: the human reads along as entries arrive, unless they have
    /// scrolled back to read something earlier.
    fn follow<T>(&mut self, change: impl FnOnce(&mut Conversation) -> T) -> T {
        // The offset grows downward as negative numbers. A conversation
        // not laid out yet, as a chat not shown, is at its end.
        let laid_out = self.list.viewport_bounds().size.height > px(0.);
        let at_end = !laid_out
            || -self.list.scroll_px_offset_for_scrollbar().y
                >= self.list.max_offset_for_scrollbar().height - px(1.);
        let changed = change(&mut self.conversation);
        if at_end {
            self.to_end = true;
        }
        changed
    }

    /// Whether the chat takes a line the human sends now: it waits for
    /// one, or it is closed and a line starts its agent again.
    fn takes_a_line(&self) -> bool {
        match self.conversation.state() {
            State::Idle => true,
            State::Closed => self.row.again,
            State::Busy | State::Asking => self.conversation.steers(),
            State::Stopping | State::Checking => false,
        }
    }

    /// The human pressed Enter: send the text, when the chat takes it.
    fn submit(&mut self, input: &Entity<LineInput>, cx: &mut Context<Self>) {
        let text = input.read(cx).text().to_owned();
        if text.trim().is_empty() || !self.takes_a_line() {
            // The text stays: a turn is running.
            return;
        }
        let attached = std::mem::take(&mut self.attached);
        self.tell(ToDaemon::Submit {
            key: self.key,
            text,
            attached,
        });
        input.update(cx, LineInput::clear);
        cx.notify();
    }

    /// A key went down: the number of an option of the permission request
    /// the agent waits on chooses it, in place of typing the number.
    fn choose_with_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.conversation.options_asked();
        if count == 0 {
            return;
        }
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        if let Ok(number @ 1..) = keystroke.key.parse::<usize>()
            && number <= count
        {
            self.choose(number - 1, cx);
            cx.stop_propagation();
        }
    }

    fn choose(&mut self, index: usize, _: &mut Context<Self>) {
        self.tell(ToDaemon::Choose {
            key: self.key,
            index,
        });
    }

    /// Stops the turn the agent is taking, if it takes one by the time the
    /// daemon hears of it, which a line just sent may.
    fn stop(&mut self, _: &mut Context<Self>) {
        self.tell(ToDaemon::Stop { key: self.key });
    }
}

impl Focusable for ChatView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Selected for ChatView {
    fn chosen_text(&self) -> String {
        self.selected_text()
    }

    /// The text selected in the conversation, with where it is from.
    fn attachment(&self) -> Option<Attachment> {
        let text = self.selected_text();
        Some(Attachment {
            label: crate::chat::hand::quoted(&text),
            uri: format!("cox://chat/{}", self.key),
            text,
            picture: None,
        })
    }

    fn end_visual(&mut self) -> bool {
        self.reading.end_visual(false)
    }
}

impl ChatView {
    /// What the chat waits for, when it is not the human's next line, or
    /// the goal it works toward, while it has one: what the input shows
    /// while it is empty.
    #[must_use]
    pub fn waiting(&self) -> Option<String> {
        let goal = self.conversation.goal();
        match self.conversation.state() {
            // The goal shows above the conversation.
            State::Idle => None,
            State::Busy => Some(match goal {
                Some(goal) => format!(
                    "The agent works toward the goal, turn {} of {GOAL_TURNS}. Escape stops it.",
                    goal.turns()
                ),
                None if self.conversation.steers() => {
                    "A line goes into this turn. Escape stops it.".to_owned()
                }
                None => "The agent is working. Escape stops it.".to_owned(),
            }),
            State::Checking => Some(
                match (
                    self.conversation.running_check(),
                    goal.and_then(Goal::command),
                ) {
                    (Some(trigger), _) if trigger != "goal" => {
                        format!("The trigger `{trigger}` runs. Escape stops it.")
                    }
                    (_, Some(command)) => {
                        format!("Checking the goal: `{command}` runs. Escape stops it.")
                    }
                    _ => "Judging the goal. Escape stops it.".to_owned(),
                },
            ),
            State::Asking => {
                let count = self.conversation.options_asked();
                let numbers = if count > 1 {
                    format!("1–{count}")
                } else {
                    "1".to_owned()
                };
                Some(if self.conversation.logging_in() {
                    format!(
                        "The agent asks you to log in: choose how with {numbers}, or give up with Escape."
                    )
                } else {
                    format!(
                        "The agent asks to go on: choose with {numbers}, or stop it with Escape."
                    )
                })
            }
            State::Stopping => Some("Stopping…".to_owned()),
            // A closed chat whose agent can start again takes the next
            // line as an open one does.
            State::Closed if self.row.again => {
                Some("Closed. A line sent starts the agent again.".to_owned())
            }
            State::Closed if self.row.gone => {
                Some("Its folder is gone: its agent starts again once it is back.".to_owned())
            }
            State::Closed => Some("Closed.".to_owned()),
        }
    }

    /// The plan the agent works to, on a card over the top right of the
    /// conversation, which a click on its line folds to that line.
    fn render_plan(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let steps = self.conversation.last_plan()?;
        let open = !self.plan_folded;
        Some(
            div()
                .id("plan")
                .debug_selector(|| "plan".to_owned())
                .absolute()
                .top_2()
                .right_3()
                // Folded, it is only as wide as its line.
                .when(open, |plan| plan.w(px(PLAN_WIDTH)))
                .max_w(relative(0.8))
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .rounded_md()
                .border_1()
                .border_color(rgb(BORDER))
                .bg(rgb(RAISED))
                .shadow_md()
                .text_size(px(12.))
                // The conversation under the card hears none of the mouse.
                .occlude()
                .child(crate::vim::hints::cover())
                .child(
                    div()
                        .id("plan-line")
                        .debug_selector(|| "plan-line".to_owned())
                        .flex()
                        .gap_2()
                        .text_color(rgb(DIM))
                        .cursor_pointer()
                        .child(disclosure(Some(open)))
                        .child(format!("Plan {}", progress(steps)))
                        .on_hint_click(cx.listener(|view, _, _, cx| {
                            view.plan_folded = !view.plan_folded;
                            cx.notify();
                        })),
                )
                .when(open, |plan| {
                    plan.child(
                        div()
                            .id("plan-steps")
                            .debug_selector(|| "plan-steps".to_owned())
                            .max_h(px(PLAN_HEIGHT))
                            .overflow_y_scroll()
                            .child(render_steps(steps)),
                    )
                }),
        )
    }

    fn render_conversation(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = matches!(
            self.conversation.state(),
            State::Busy | State::Asking | State::Checking
        );
        // The frame lays the text in sight out again, and keeps what the
        // frames before laid out of the rest, for a selection to copy.
        self.reading.age(self.conversation.entries().len());
        self.follow_reading(window, cx);
        self.lay_out_places();
        // What the chat waits for shows in the input, while it is empty.
        let placeholder = self.waiting().unwrap_or_else(|| "Reply".to_owned());
        self.input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        let goal_input = self.render_goal_input(cx);
        let sketch_input = self.render_sketch_input(cx);
        let plan = self.render_plan(cx);
        let goal = self.render_goal();
        div()
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|view, _: &Stop, _, cx| view.stop(cx)))
            .map(|conversation| selected::on_actions(conversation, cx))
            .on_action(cx.listener(Self::move_by_keys))
            .map(|conversation| Self::go_through_by_keys(conversation, cx))
            .capture_key_down(cx.listener(Self::choose_with_key))
            // The list over the input takes its keys first, while it is
            // open.
            .capture_action(cx.listener(|view, _: &input::Up, window, cx| {
                if view.complete_step(-1, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|view, _: &input::Down, window, cx| {
                if view.complete_step(1, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|view, _: &input::Submit, window, cx| {
                if view.complete_take(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|view, _: &Stop, window, cx| {
                if view.complete_close(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|view, _: &CloseList, window, cx| {
                view.complete_close(window, cx);
            }))
            .capture_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key == "tab"
                    && !keystroke.modifiers.modified()
                    && view.complete_take(false, window, cx)
                {
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .children(goal)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_reading(window, cx))
                    .child(region::mark({
                        let view = cx.weak_entity();
                        move |_, over, cx| Some(view.upgrade()?.read(cx).described(over))
                    }))
                    // Drawn after the conversation, over it.
                    .children(plan),
            )
            .children(goal_input)
            .children(sketch_input)
            .child(
                // A column, which stretches the input across it, whatever
                // its placeholder's width.
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(self.render_input_box(running, cx))
                    .children(self.render_completions(cx)),
            )
    }
}

impl ChatView {
    /// Works out the conversation's places again, telling the list which
    /// changed: the places from the first that starts at another entry
    /// on are new to it, and the rest keep the heights they were laid out
    /// at, the list laying out again those in sight. Scrolls to the end,
    /// when it is to.
    fn lay_out_places(&mut self) {
        let places = self.places();
        let kept = self
            .places
            .iter()
            .zip(&places)
            .take_while(|(was, is)| was.first() == is.first())
            .count();
        if kept < self.places.len() || kept < places.len() {
            self.list
                .splice(kept..self.places.len(), places.len() - kept);
        }
        self.places = places;
        if std::mem::take(&mut self.to_end) {
            self.list.scroll_to(ListOffset {
                item_ix: self.places.len(),
                offset_in_item: px(0.),
            });
        }
        if let Some(entry) = self.to_entry
            && entry < self.conversation.entries().len()
            && let Some(place) = self.places.iter().rposition(|place| place.first() <= entry)
        {
            self.to_entry = None;
            self.list.scroll_to(ListOffset {
                item_ix: place,
                offset_in_item: px(0.),
            });
        }
    }

    /// Scrolls the conversation to the entry at `index`, and marks it a
    /// moment, as the human asked to see it: the line that started a turn
    /// a commit was made in.
    pub fn show_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        self.to_entry = Some(index);
        self.to_end = false;
        self.marked = Some(index);
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(MARKED_FOR).await;
            let _ = view.update(cx, |view, cx| {
                if view.marked == Some(index) {
                    view.marked = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The box the human types the line in: the input, and under it the
    /// chips that start a command or a file's path, and the button that
    /// stops the turn while it runs.
    fn render_input_box(&self, running: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let input = self.input.clone();
        let commands = !self.conversation.commands().is_empty();
        let files = self.cwd().is_some();
        div()
            .debug_selector(|| "input".to_owned())
            .flex()
            .flex_col()
            .gap(px(8.))
            .m(px(12.))
            .min_h(px(48.))
            .pl(px(16.))
            .pr(px(8.))
            .py(px(8.))
            .rounded(px(16.))
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(BORDER))
            .text_size(px(15.))
            // The box is where text goes in, past the input's own row too,
            // but for the chips and the Stop button, which point.
            .cursor_text()
            // A click brings the keyboard back from the stage, and
            // keeps it from the window, which takes it otherwise.
            // What was selected in the conversation is let go, so
            // that the keys copy from the input.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, window, cx| {
                    window.focus(&input.focus_handle(cx));
                    window.prevent_default();
                    if view.reading.deselect() {
                        cx.notify();
                    }
                }),
            )
            .when(!self.attached.is_empty(), |bar| {
                bar.child(self.render_attached(cx))
            })
            .child(
                div()
                    .pt(px(4.))
                    .w_full()
                    .min_w_0()
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(commands, |row| {
                        row.child(Self::render_chip(
                            "start-command",
                            "/ commands",
                            Self::start_command,
                            cx,
                        ))
                    })
                    .when(files, |row| {
                        row.child(Self::render_chip(
                            "start-file",
                            "@ files",
                            Self::start_file,
                            cx,
                        ))
                    })
                    .child(Self::render_chip(
                        "start-region",
                        "▭ region",
                        Self::start_region,
                        cx,
                    ))
                    .child(div().flex_1())
                    .when(running, |row| row.child(Self::render_stop(cx))),
            )
    }

    /// Lays the sheet over the window, for the human to drag a region of
    /// it to hand the chat.
    #[expect(
        clippy::unused_self,
        reason = "a chip's action is a method of the view"
    )]
    fn start_region(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.dispatch_action(Box::new(AddRegionToChat), cx);
    }

    /// What the human handed the chat, over the input, a chip each: a
    /// click on one opens where it is from, and its × takes it out.
    fn render_attached(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .debug_selector(|| "attached".to_owned())
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .pr(px(8.))
            .children(self.attached.iter().enumerate().map(|(index, attachment)| {
                let uri = attachment.uri.clone();
                entries::attachment_chip(("attached", index).into(), attachment)
                    .on_hint_click(cx.listener(move |view, _, window, cx| {
                        if entries::opens(&uri) {
                            view.show_link(&uri, window, cx);
                        }
                    }))
                    .child(
                        div()
                            .id(("unattach", index))
                            .debug_selector(move || format!("unattach-{index}"))
                            .px(px(2.))
                            .rounded(px(4.))
                            .text_color(rgb(DIM))
                            .cursor_pointer()
                            .hover(|style| style.text_color(rgb(FOREGROUND)))
                            .child("×")
                            .on_hint_click(cx.listener(move |view, _, _, cx| {
                                cx.stop_propagation();
                                if index < view.attached.len() {
                                    view.attached.remove(index);
                                    cx.notify();
                                }
                            })),
                    )
            }))
    }

    /// A chip under the input, named `id`, that `start`s what its
    /// `label` says, as the key it names does.
    fn render_chip(
        id: &'static str,
        label: &'static str,
        start: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .id(id)
            .debug_selector(move || id.to_owned())
            .h(px(24.))
            .px(px(9.))
            .flex()
            .items_center()
            .rounded(px(12.))
            .bg(rgb(BUTTON))
            .text_size(px(12.))
            .text_color(rgb(FAINT))
            .cursor_pointer()
            .hover(|style| style.text_color(rgb(FOREGROUND)))
            .child(label)
            .on_hint_click(cx.listener(move |view, _, window, cx| start(view, window, cx)))
    }
}

impl ChatView {
    /// The button that stops the turn, beside the input while it runs.
    fn render_stop(cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("stop")
            .debug_selector(|| "stop".to_owned())
            .flex_shrink_0()
            .ml_2()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .bg(rgb(BUTTON))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON_HOVER)))
            .tooltip(tip("Stop (Escape)"))
            // A square, as a player's stop is.
            .child(div().size(px(10.)).rounded(px(2.)).bg(rgb(FOREGROUND)))
            .on_hint_click(cx.listener(|view, _, _, cx| view.stop(cx)))
    }

    /// The goal, above the conversation, while the chat has one: what it
    /// is, and, while the agent works toward it, a bar for each turn its
    /// line gets, those whose check failed, the one it takes, and the
    /// rest.
    fn render_goal(&self) -> Option<impl IntoElement + use<>> {
        let goal = self.conversation.goal()?;
        let turns = goal.turns();
        let what = match goal.command() {
            Some(command) => div()
                .font_family(CODE_FONT)
                .text_size(px(12.5))
                .child(command.to_owned()),
            None => div()
                .text_size(px(13.))
                .child(goal.words().unwrap_or_default().to_owned()),
        };
        let bars = (1..=GOAL_TURNS).map(move |turn| {
            let color = match turn.cmp(&turns) {
                std::cmp::Ordering::Less => FAILED_TURN,
                std::cmp::Ordering::Equal => ACCENT,
                std::cmp::Ordering::Greater => BORDER,
            };
            div().w(px(5.)).h(px(12.)).rounded(px(2.)).bg(rgb(color))
        });
        Some(
            div()
                .debug_selector(|| "goal".to_owned())
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(10.))
                .mx(px(12.))
                .mt(px(12.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(12.))
                .bg(rgb(RAISED))
                .child(icon(Icon::Target, 14., ACCENT))
                .child(div().flex_1().min_w_0().truncate().child(what))
                .when(turns > 0, |bar| {
                    bar.child(div().flex_shrink_0().flex().gap(px(3.)).children(bars))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(12.))
                                .text_color(rgb(FAINT))
                                .child(format!("{turns} / {GOAL_TURNS}")),
                        )
                }),
        )
    }
}

impl ChatView {
    /// The input that sets the goal, in place of the line that tells what
    /// the chat waits for, while it is open.
    fn render_goal_input(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (input, _, _) = self.goal_input.as_ref()?;
        Some(
            Self::render_input_card("Goal", "words, or $ and a command · Enter sets it")
                .key_context(GOAL)
                .debug_selector(|| "goal-input".to_owned())
                .on_action(cx.listener(|view, _: &CancelGoal, window, cx| {
                    view.end_editing_goal(false, window, cx);
                }))
                .on_mouse_down_out(cx.listener(|view, _, window, cx| {
                    view.end_editing_goal(true, window, cx);
                }))
                .child(input.clone()),
        )
    }

    /// The input that starts a sketch, with what the sketch starts from,
    /// which a click on the other changes, while it is open.
    fn render_sketch_input(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (input, sketch, _) = self.sketch_input.as_ref()?;
        // What the sketch starts from, lit, and the other, which a click
        // chooses instead.
        let from = |label: &'static str, chosen: bool| {
            let chip = div()
                .h(px(24.))
                .px(px(9.))
                .flex()
                .items_center()
                .rounded(px(12.))
                .text_size(px(12.))
                .child(label);
            if chosen {
                chip.bg(rgb(ACCENT_GROUND))
                    .text_color(rgb(ACCENT))
                    .into_any_element()
            } else {
                chip.id("sketch-from")
                    .debug_selector(|| "sketch-from".to_owned())
                    .bg(rgb(BUTTON))
                    .text_color(rgb(FAINT))
                    .cursor_pointer()
                    .hover(|style| style.text_color(rgb(FOREGROUND)))
                    .on_hint_click(cx.listener(|view, _, window, cx| view.flip_sketch(window, cx)))
                    .into_any_element()
            }
        };
        let project = *sketch == Sketch::Project;
        Some(
            Self::render_input_card("Sketch", "what to try out · Enter opens it")
                .key_context(SKETCH)
                .debug_selector(|| "sketch-input".to_owned())
                .on_action(cx.listener(|view, _: &CancelSketch, window, cx| {
                    view.end_editing_sketch(false, window, cx);
                }))
                .child(input.clone())
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(from("from the project", project))
                        .child(from("from nothing", !project)),
                ),
        )
    }

    /// A card over the chat's input, as the input's own box is drawn but
    /// edged in the accent, for an input that takes `what`, as `hint`
    /// says.
    fn render_input_card(what: &'static str, hint: &'static str) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mx(px(12.))
            .mt(px(12.))
            .px(px(16.))
            .py(px(10.))
            .rounded(px(16.))
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(OFFERING_EDGE))
            .text_size(px(15.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(11.))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(FAINT))
                            .child(what.to_uppercase()),
                    )
                    .child(div().text_color(rgb(FAINT)).child(hint)),
            )
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(rgb(FOREGROUND))
            .font_family(UI_FONT)
            .text_size(px(14.))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_conversation(window, cx)),
            )
    }
}
