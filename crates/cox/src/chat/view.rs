//! One chat in the window: draws its conversation, with its stage beside
//! it while the stage is open, and turns keystrokes and clicks into input
//! for it. Escape in the conversation, or its Stop button, stops the turn
//! the agent is taking.
//!
//! The stage opens only when it is used: the human opens and closes it
//! with the button above the conversation, and it opens on its own when a
//! link opens on it, or the agent works on it. Closed, it keeps its tabs,
//! and the conversation has the window to itself.
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
//! and a click on its line folds it to that line. A line above the input
//! tells what the chat waits for, when it is not the human's next line,
//! and the goal the chat works toward, while it has one; the human sets
//! the goal in an input that opens in its place (see [`ChatView::edit_goal`]).
//!
//! When a turn ends and the chat has a goal, the view runs the goal's
//! command in a terminal on the stage, as the agent's own tool would, or
//! asks the goal's judge, an agent of its own (see [`crate::chat::judge`]),
//! and hands the conversation how it went, which decides whether the
//! agent goes on (see [`Conversation::checked`] and
//! [`Conversation::judged`]).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use cox_core::goal;
use cox_core::store::Record;
use gpui::{
    AnyView, App, ClipboardItem, Context, Entity, FocusHandle, Focusable, KeyBinding, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollHandle, Subscription, Task,
    Window, actions, div, prelude::*, px, relative, rgb,
};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::agent::mcp::{ChatRequest, Ran, ToolRequest};
use crate::agent::{Agent, AgentThread};
use crate::agent::{AgentEvent, Command};
use crate::chat::conversation::{self, Conversation, Entry, GOAL_TURNS, Goal, State};
use crate::chat::judge::{Judge, Starts};
use crate::ui::input::{LineInput, Submitted};
use crate::ui::text::Reading;
use crate::ui::theme::{DIM, FONT, LINK};
use crate::ui::{Copy, Panel};

mod entries;

use entries::{Folds, disclosure, progress, render_steps};

actions!(
    cox,
    [
        /// Stops the turn the agent is taking.
        Stop,
        /// Leaves the chat's goal as it was.
        CancelGoal,
    ]
);

const KEY_CONTEXT: &str = "Conversation";

/// The key context of the input that sets the chat's goal.
const GOAL: &str = "Goal";

/// How long the name of the chat that opened a chat grows, in
/// characters, in the note that says so.
const OPENED_BY_LENGTH: usize = 40;

/// Binds the keys of the conversation. Escape is the stage's while the
/// stage has the keyboard, and the input method's while it composes.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Stop, Some(KEY_CONTEXT)),
        // The secondary key is Command on macOS, Control elsewhere.
        KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", CancelGoal, Some(GOAL)),
    ]);
}

/// Opens a link the human clicked: on the chat's stage, where it has
/// one.
pub type OpenLink = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Starts the agent of a closed chat again, when the human sends to it,
/// telling it what the chat showed, in case it cannot go on with its
/// session.
pub type Again = Box<dyn FnOnce(&[Entry], &mut Window, &mut App) -> Agent>;

/// A closed chat's agent started again.
pub struct Reopened;

/// What the agent's tools asked of the window, such as opening another
/// chat, for the window to take, once, and carry out.
pub struct Asked(pub RefCell<Option<ChatRequest>>);

impl Asked {
    /// Takes the request, the first time.
    pub fn take(&self) -> Option<ChatRequest> {
        self.0.borrow_mut().take()
    }
}

/// How wide the conversation is when the stage sits beside it.
const CONVERSATION_WIDTH: f32 = 420.;

/// How wide the card of the plan in sight is, and how tall its steps grow
/// before they scroll on their own.
const PLAN_WIDTH: f32 = 240.;
const PLAN_HEIGHT: f32 = 200.;

/// How wide the conversation grows, at most, while the stage is closed.
const CONVERSATION_MAX_WIDTH: f32 = 800.;

/// A chat's stage, as its conversation shows it: the view, and whether it
/// is open.
pub struct StageView {
    pub view: AnyView,
    pub panel: Entity<Panel>,
}

pub struct ChatView {
    conversation: Conversation,
    /// Where the conversation is scrolled to.
    scroll: ScrollHandle,
    /// The line the human is typing.
    input: Entity<LineInput>,
    /// Where the agent and the human work together, beside the
    /// conversation.
    stage: Option<StageView>,
    open_link: OpenLink,
    /// The conversation's text, and what the human selects of it.
    reading: Reading,
    /// What the human opened and closed of the conversation.
    folds: Folds,
    /// Whether the human folded the card of the plan to its line.
    plan_folded: bool,
    /// Where the conversation is kept as it changes, if it is.
    record: Option<Record>,
    commands: UnboundedSender<Command>,
    /// What starts the agent of a closed chat again, until it has.
    again: Option<Again>,
    /// The agent, and what hears it, and what its tools ask of the window.
    /// The agent lives as long as the chat stays open: dropping this stops
    /// it. A closed chat has none.
    agent: Option<(AgentThread, Vec<Task<()>>)>,
    /// The directory the agent works in, and where the chat asks its stage
    /// to run the goal's command, as the agent's tools ask it to, while
    /// the agent runs and has tools of Cox's own.
    tools: Option<(PathBuf, UnboundedSender<ToolRequest>)>,
    /// The check of the goal that runs, if one does. Dropping it lets go
    /// of the answer, which stops the command.
    check: Option<Task<()>>,
    /// What starts the judge of a goal in words, when the chat can have
    /// one.
    starts: Option<Starts>,
    /// The judge of the goal in words, once it was asked.
    judge: Option<Judge>,
    /// The directory the agent works in, while it runs.
    cwd: Option<PathBuf>,
    /// The input the human sets the goal in, while it is open.
    goal_input: Option<(Entity<LineInput>, Subscription)>,
    _submitted: Subscription,
    /// Redraws the chat as its stage opens and closes.
    _panel_changes: Option<Subscription>,
}

impl ChatView {
    /// Opens on `agent`, with `stage` beside the conversation when there is
    /// one and it is open, keeping the conversation in `record` when there
    /// is one. Links the human clicks go to `open_link`.
    /// `starts` starts the judge of a goal in words, when the chat can
    /// have one.
    pub fn new(
        agent: Agent,
        stage: Option<StageView>,
        open_link: OpenLink,
        record: Option<Record>,
        starts: Option<Starts>,
        cx: &mut Context<Self>,
    ) -> Self {
        // The human sees which instructions the agent gets, and the context
        // it was set up with.
        let notes = agent
            .instructions
            .iter()
            .map(|path| conversation::instructions_note(path))
            .chain(
                agent
                    .setup
                    .iter()
                    .filter_map(|setup| setup.context.as_deref())
                    .map(conversation::context_note),
            );
        let conversation = Conversation::noting_each(notes);
        let mut view = Self {
            record,
            starts,
            ..Self::with(conversation, stage, open_link, None, cx)
        };
        view.attach(agent, cx);
        view.keep();
        view
    }

    /// A chat that has closed, whose `conversation` is for the human to
    /// read (see [`Conversation::closed`]), with `stage` beside it when
    /// there is one and it is open. When the human sends to it, `again`
    /// starts its agent again, if it can, and the chat goes on, kept in
    /// `record` when there is one. `starts` starts the judge of a goal in
    /// words, when the chat can have one.
    #[expect(clippy::too_many_arguments, reason = "what a closed chat is made of")]
    pub fn closed(
        conversation: Conversation,
        stage: Option<StageView>,
        open_link: OpenLink,
        record: Option<Record>,
        again: Option<Again>,
        starts: Option<Starts>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = Self {
            record,
            again,
            starts,
            ..Self::with(conversation, stage, open_link, Some(window), cx)
        };
        view.scroll.scroll_to_bottom();
        view
    }

    /// The view of `conversation`, with no agent. Given the `window`, the
    /// view can start the agent of a closed chat again.
    fn with(
        conversation: Conversation,
        stage: Option<StageView>,
        open_link: OpenLink,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(LineInput::wrapping);
        let submitted = match window {
            // Starting the agent again needs the window, for the stage.
            Some(window) => {
                cx.subscribe_in(&input, window, |view, input, Submitted, window, cx| {
                    view.submit_again(input, window, cx);
                })
            }
            None => cx.subscribe(&input, |view, input, Submitted, cx| {
                view.submit(&input, cx);
            }),
        };
        // The conversation's width changes, and its end stays in view if
        // it was.
        let panel_changes = stage.as_ref().map(|stage| {
            cx.observe(&stage.panel, |view, _, cx| {
                view.follow(|_| ());
                cx.notify();
            })
        });
        // Nothing is sent, nor heard, until there is an agent.
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        Self {
            conversation,
            scroll: ScrollHandle::new(),
            input,
            stage,
            open_link,
            reading: Reading::default(),
            folds: Folds::default(),
            plan_folded: false,
            record: None,
            commands,
            again: None,
            agent: None,
            tools: None,
            check: None,
            starts: None,
            judge: None,
            cwd: None,
            goal_input: None,
            _submitted: submitted,
            _panel_changes: panel_changes,
        }
    }

    /// Talks to `agent` from now on: sends it what the human asks, shows
    /// what it tells, and passes on what its tools ask of the window.
    fn attach(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let Agent {
            commands,
            mut events,
            thread,
            cwd,
            tools,
            chat_requests,
            ..
        } = agent;
        let hears = cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let shown = this.update(cx, |view, cx| {
                    view.hear(event, cx);
                    cx.notify();
                });
                if shown.is_err() {
                    break;
                }
            }
        });
        let mut tasks = vec![hears];
        if let Some(mut chat_requests) = chat_requests {
            tasks.push(cx.spawn(async move |this, cx| {
                while let Some(request) = chat_requests.recv().await {
                    let passed = this.update(cx, |_, cx| {
                        cx.emit(Asked(RefCell::new(Some(request))));
                    });
                    if passed.is_err() {
                        break;
                    }
                }
            }));
        }
        self.commands = commands;
        self.agent = Some((thread, tasks));
        self.cwd = Some(cwd.clone());
        self.tools = tools.map(|tools| (cwd, tools));
    }

    /// Starts the chat, opened from the chat named `opened_by`, on `text`
    /// as its first line, sent in the human's place, working until `goal`
    /// is met if given.
    pub fn tell(
        &mut self,
        opened_by: &str,
        text: &str,
        goal: Option<goal::Goal>,
        cx: &mut Context<Self>,
    ) {
        // A chat named after its first line has a long name.
        let opened_by: String = match opened_by.char_indices().nth(OPENED_BY_LENGTH) {
            Some((end, _)) => format!("{}\u{2026}", opened_by[..end].trim_end()),
            None => opened_by.to_owned(),
        };
        self.follow(|conversation| conversation.note(format!("Opened by: {opened_by}")));
        if goal.is_some() {
            self.set_goal(goal, cx);
        }
        if let Some(command) = self.follow(|conversation| conversation.told(text)) {
            self.send(command);
        }
        self.keep();
        cx.notify();
    }

    /// Shows what the agent side told, and keeps it. A turn that ends
    /// with a goal to meet starts the goal's check, or asks its judge.
    fn hear(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        if let (AgentEvent::Opened { id, .. }, Some(record)) = (&event, &self.record)
            && let Err(error) = record.session(id)
        {
            log::error!("the agent's session is not kept, to resume: {error:#}");
        }
        let was_checking = self.conversation.state() == State::Checking;
        self.follow(|conversation| conversation.apply(event));
        if !was_checking {
            if let Some(command) = self.conversation.checking() {
                self.check(command.to_owned(), cx);
            } else if let Some(words) = self.conversation.judging().map(str::to_owned) {
                self.judge(&words, cx);
            }
        }
        self.keep();
    }

    /// Runs `command`, the goal's, in a terminal on the stage, and hands
    /// the conversation how it went once it ends.
    fn check(&mut self, command: String, cx: &mut Context<Self>) {
        let Some((cwd, tools)) = &self.tools else {
            self.checked(Err("the chat has no terminal to run it in".to_owned()), cx);
            return;
        };
        let (answer, ran) = oneshot::channel();
        let request = ToolRequest::RunCommand {
            command,
            cwd: cwd.clone(),
            answer,
        };
        if tools.send(request).is_err() {
            self.checked(Err("the chat's terminals are gone".to_owned()), cx);
            return;
        }
        self.check = Some(cx.spawn(async move |this, cx| {
            let ran = match ran.await {
                Ok(ran) => ran,
                Err(_) => Err("its terminal went away".to_owned()),
            };
            this.update(cx, |view, cx| view.checked(ran, cx)).ok();
        }));
    }

    /// The goal's command ended as `ran` says: the agent goes on if the
    /// goal is not met yet.
    fn checked(&mut self, ran: Result<Ran, String>, cx: &mut Context<Self>) {
        self.check = None;
        if let Some(command) = self.follow(|conversation| conversation.checked(ran)) {
            self.send(command);
        }
        self.keep();
        cx.notify();
    }

    /// Asks the judge of the goal, `words`, whether it is met now, starting
    /// the judge if the chat has none yet.
    fn judge(&mut self, words: &str, cx: &mut Context<Self>) {
        if self.judge.is_none() {
            let (Some(starts), Some(cwd)) = (&self.starts, &self.cwd) else {
                self.judged(Err("the chat has no agent to judge it".to_owned()), cx);
                return;
            };
            self.judge = Some(Judge::start(starts, cwd.clone(), words, cx));
        }
        let said = self.conversation.last_told().map(str::to_owned);
        if let Some(judge) = &self.judge {
            judge.ask(words, said.as_deref());
        }
    }

    /// The judge said `judged` of the goal, or could not judge it: the
    /// agent goes on if the goal is not met yet.
    pub fn judged(&mut self, judged: Result<String, String>, cx: &mut Context<Self>) {
        if let Some(command) = self.follow(|conversation| conversation.judged(judged)) {
            self.send(command);
        }
        self.keep();
        cx.notify();
    }

    /// The judge's agent closed, as `error` says: the next judgement
    /// starts a judge anew, and the one under way, if any, is not coming.
    pub fn judge_closed(&mut self, error: String, cx: &mut Context<Self>) {
        self.judge = None;
        self.judged(Err(error), cx);
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
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_text(&line, cx);
            input.select_whole(cx);
            input
        });
        let submitted = cx.subscribe_in(&input, window, |view, _, Submitted, window, cx| {
            view.end_editing_goal(true, window, cx);
        });
        window.focus(&input.focus_handle(cx));
        self.goal_input = Some((input, submitted));
        cx.notify();
    }

    /// Closes the input that sets the goal, setting the goal to what it
    /// holds when `keep` says to, and brings the keyboard back to the
    /// chat's input.
    fn end_editing_goal(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((input, _)) = self.goal_input.take() else {
            return;
        };
        if keep {
            let goal = goal::Goal::parse(input.read(cx).text());
            self.set_goal(goal, cx);
        }
        window.focus(&self.input.focus_handle(cx));
        cx.notify();
    }

    /// Sets the chat to work toward `goal`, or, given `None`, toward no
    /// goal, and keeps that. The judge of the goal before, if any, is
    /// let go.
    pub fn set_goal(&mut self, goal: Option<goal::Goal>, cx: &mut Context<Self>) {
        if goal.as_ref() != self.conversation.goal().map(Goal::what) {
            self.judge = None;
        }
        self.follow(|conversation| conversation.set_goal(goal));
        if self.conversation.state() != State::Checking {
            self.check = None;
        }
        if let Some(record) = &self.record
            && let Err(error) = record.goal(self.conversation.goal().map(Goal::what))
        {
            log::error!("the chat's goal is not kept: {error:#}");
        }
        self.keep();
        cx.notify();
    }

    /// Whether the chat's agent runs: it was opened in this run, or the
    /// human sent to it again.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.agent.is_some()
    }

    /// Keeps the entries that changed since this was last called.
    fn keep(&mut self) {
        let changed = self.conversation.take_changed();
        let Some(record) = &self.record else {
            return;
        };
        for index in changed {
            if let Err(error) = record.entry(index, &self.conversation.entries()[index]) {
                log::error!("the chat is not kept as it goes: {error:#}");
                break;
            }
        }
    }

    /// Whether the stage is open beside the conversation.
    #[must_use]
    pub fn stage_is_open(&self, cx: &App) -> bool {
        self.stage
            .as_ref()
            .is_some_and(|stage| stage.panel.read(cx).is_open())
    }

    /// Opens the stage, or closes it and brings the keyboard back to the
    /// input.
    fn toggle_stage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(stage) = &self.stage else {
            return;
        };
        let open = !stage.panel.read(cx).is_open();
        stage.panel.update(cx, |panel, cx| panel.set(open, cx));
        if !open {
            window.focus(&self.input.focus_handle(cx));
        }
    }

    #[must_use]
    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    #[must_use]
    pub fn input(&self) -> &Entity<LineInput> {
        &self.input
    }

    /// Where the conversation is scrolled to.
    #[must_use]
    pub fn scroll(&self) -> &ScrollHandle {
        &self.scroll
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
            if let Some(stage) = &self.stage {
                stage.panel.update(cx, Panel::open);
            }
            (self.open_link)(&url, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Changes the conversation with `change`, keeping its end in view if
    /// it was: the human reads along as entries arrive, unless they have
    /// scrolled back to read something earlier.
    fn follow<T>(&mut self, change: impl FnOnce(&mut Conversation) -> T) -> T {
        // The offset grows downward as negative numbers.
        let at_end = self.scroll.offset().y <= -self.scroll.max_offset().height + px(1.);
        let changed = change(&mut self.conversation);
        if at_end {
            self.scroll.scroll_to_bottom();
        }
        changed
    }

    /// The human pressed Enter: send the text.
    fn submit(&mut self, input: &Entity<LineInput>, cx: &mut Context<Self>) {
        let text = input.read(cx).text().to_owned();
        if let Some(command) = self.follow(|conversation| conversation.submit(&text)) {
            self.keep();
            self.send(command);
            input.update(cx, LineInput::clear);
            cx.notify();
        }
        // Otherwise the text stays: a turn is running.
    }

    /// The human pressed Enter in a chat that may be closed: start its
    /// agent again first, if it is closed and can be, then send.
    fn submit_again(
        &mut self,
        input: &Entity<LineInput>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sending = !input.read(cx).text().trim().is_empty();
        if sending
            && self.conversation.state() == State::Closed
            && let Some(again) = self.again.take()
        {
            let agent = again(self.conversation.entries(), window, cx);
            self.attach(agent, cx);
            self.conversation.reopen();
            cx.emit(Reopened);
        }
        self.submit(input, cx);
    }

    /// A key went down: the number of an option of the permission request
    /// the agent waits on chooses it, in place of typing the number.
    fn choose_with_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(options) = self.conversation.asked() else {
            return;
        };
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        if let Ok(number @ 1..) = keystroke.key.parse::<usize>()
            && number <= options.len()
        {
            self.choose(number - 1, cx);
            cx.stop_propagation();
        }
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(command) = self.conversation.choose(index) {
            self.keep();
            self.send(command);
            cx.notify();
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        let before = self.conversation.state();
        if let Some(command) = self.follow(Conversation::cancel) {
            self.send(command);
        }
        if self.conversation.state() != State::Checking {
            self.check = None;
            if before == State::Checking
                && let Some(judge) = &self.judge
            {
                judge.cancel();
            }
        }
        if self.conversation.state() != before {
            self.keep();
            cx.notify();
        }
    }

    fn send(&self, command: Command) {
        // The agent side is gone once the connection closes, which the
        // conversation shows when the `Closed` event arrives.
        let _ = self.commands.send(command);
    }
}

impl gpui::EventEmitter<Reopened> for ChatView {}
impl gpui::EventEmitter<Asked> for ChatView {}

impl Focusable for ChatView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl ChatView {
    /// What the chat waits for, when it is not the human's next line, or
    /// the goal it works toward, while it has one.
    fn waiting(&self) -> Option<String> {
        let goal = self.conversation.goal();
        match self.conversation.state() {
            State::Idle => goal.map(|goal| format!("Goal: {}", goal.line())),
            State::Busy => Some(match goal {
                Some(goal) => format!(
                    "The agent works toward the goal, turn {} of {GOAL_TURNS}. Escape stops it.",
                    goal.turns()
                ),
                None => "The agent is working. Escape stops it.".to_owned(),
            }),
            State::Checking => Some(match goal.and_then(Goal::command) {
                Some(command) => format!("Checking the goal: `{command}` runs. Escape stops it."),
                None => "Judging the goal. Escape stops it.".to_owned(),
            }),
            State::Asking => {
                let count = self.conversation.asked().map_or(0, <[String]>::len);
                let numbers = if count > 1 {
                    format!("1–{count}")
                } else {
                    "1".to_owned()
                };
                Some(format!(
                    "The agent asks to go on: choose with {numbers}, or stop it with Escape."
                ))
            }
            State::Stopping => Some("Stopping…".to_owned()),
            // A closed chat whose agent can start again takes the next
            // line as an open one does.
            State::Closed if self.again.is_some() => {
                Some("Closed. A line sent starts the agent again.".to_owned())
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
                .border_color(rgb(0x40_4040))
                .bg(rgb(0x25_2526))
                .shadow_md()
                .text_size(px(12.))
                // The conversation under the card hears none of the mouse.
                .occlude()
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
                        .on_click(cx.listener(|view, _, _, cx| {
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

    fn render_conversation(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let running = matches!(
            self.conversation.state(),
            State::Busy | State::Asking | State::Checking
        );
        // The frame lays the text out again.
        self.reading.clear();
        let entries = self.render_entries(cx);
        let waiting = self.waiting();
        let goal_input = self.render_goal_input(cx);
        let plan = self.render_plan(cx);
        let input = self.input.clone();
        div()
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(|view, _: &Stop, _, cx| view.stop(cx)))
            .on_action(cx.listener(Self::copy))
            .capture_key_down(cx.listener(Self::choose_with_key))
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("conversation")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                            .on_mouse_move(cx.listener(Self::mouse_move))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|view, _, _, _| view.reading.stop()),
                            )
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .children(entries),
                    )
                    // Drawn after the conversation, over it.
                    .children(plan),
            )
            .children(waiting.filter(|_| goal_input.is_none()).map(|waiting| {
                div()
                    .debug_selector(|| "waiting".to_owned())
                    .px_3()
                    .py_1()
                    .text_size(px(12.))
                    .text_color(rgb(DIM))
                    .child(waiting)
            }))
            .children(goal_input)
            .child(
                div()
                    .debug_selector(|| "input".to_owned())
                    .flex()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(0x40_4040))
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
                    .child(div().flex_1().min_w_0().child(self.input.clone()))
                    .when(running, |bar| {
                        bar.child(
                            div()
                                .id("stop")
                                .debug_selector(|| "stop".to_owned())
                                .flex_shrink_0()
                                .ml_2()
                                .px_2()
                                .rounded_sm()
                                .bg(rgb(0x30_3030))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(0x40_4040)))
                                .child("Stop")
                                .on_click(cx.listener(|view, _, _, cx| view.stop(cx))),
                        )
                    }),
            )
    }
}

impl ChatView {
    /// The input that sets the goal, in place of the line that tells what
    /// the chat waits for, while it is open.
    fn render_goal_input(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (input, _) = self.goal_input.as_ref()?;
        Some(
            div()
                .key_context(GOAL)
                .debug_selector(|| "goal-input".to_owned())
                .px_3()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(12.))
                .on_action(cx.listener(|view, _: &CancelGoal, window, cx| {
                    view.end_editing_goal(false, window, cx);
                }))
                .on_mouse_down_out(cx.listener(|view, _, window, cx| {
                    view.end_editing_goal(true, window, cx);
                }))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(DIM))
                        .child("Goal: words, or $ and a command"),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px_1()
                        .border_1()
                        .border_color(rgb(LINK))
                        .rounded_sm()
                        .child(input.clone()),
                ),
        )
    }

    /// The bar above the conversation, with the button that opens and
    /// closes the stage, when there is one.
    fn render_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let stage = self.stage.as_ref()?;
        let label = if stage.panel.read(cx).is_open() {
            "Hide stage"
        } else {
            "Show stage"
        };
        Some(
            div().flex().justify_end().px_3().pt_2().child(
                div()
                    .id("toggle-stage")
                    .debug_selector(|| "toggle-stage".to_owned())
                    .px_2()
                    .rounded_sm()
                    .text_color(rgb(DIM))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x30_3030)).text_color(rgb(0xe0_e0e0)))
                    .child(label)
                    .on_click(cx.listener(|view, _, window, cx| view.toggle_stage(window, cx))),
            ),
        )
    }
}

impl Render for ChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = self.render_bar(cx);
        let conversation = div()
            .size_full()
            .flex()
            .flex_col()
            .children(bar)
            .child(div().flex_1().min_h_0().child(self.render_conversation(cx)));
        let root = div()
            .size_full()
            .flex()
            .bg(rgb(0x1e_1e1e))
            .text_color(rgb(0xe0_e0e0))
            .font_family(FONT)
            .text_size(px(14.));
        match &self.stage {
            Some(stage) if stage.panel.read(cx).is_open() => root
                .child(
                    div()
                        .w(px(CONVERSATION_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .border_r_1()
                        .border_color(rgb(0x40_4040))
                        .child(conversation),
                )
                .child(
                    div()
                        .debug_selector(|| "stage-panel".to_owned())
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(stage.view.clone()),
                ),
            _ => root.justify_center().child(
                // Narrower than its text unwrapped, as a window can be, the
                // conversation wraps its text in place of growing past it.
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(px(CONVERSATION_MAX_WIDTH))
                    .h_full()
                    .child(conversation),
            ),
        }
    }
}
