//! The entries of a chat's conversation, as the window draws them: what
//! the human sent, and Cox in their place, the agent's replies as
//! Markdown, and its tool calls,
//! permission requests, thoughts and plan, each on a line of its own under
//! a mark of how it stands. What a line stands for, such as a command's
//! output, a change to a file line by line, all the agent thought, or the
//! steps of its plan, opens under it with a click, and shows at first only
//! where the human has something to read: the change a request asks to
//! make, the output of a call that failed, or what the agent is thinking
//! as it thinks it. The work the agent did between its replies, its tool
//! calls, thoughts and plan one after another, folds under a line that
//! counts its steps, the tool calls, each of which shows as a chip of how
//! it went, which opens its line with a click, with the last thought
//! under them, until the human opens the whole of it.

use std::collections::HashSet;
use std::ops::Range;

use cox_core::diff::Line;
use gpui::{AnyElement, Context, FontWeight, SharedString, Stateful, div, prelude::*, px, rgb};

use cox_core::entry::MediaKind;
use cox_core::goal::Goal;

use crate::chat::conversation::{
    self, Answer, Attachment, Check, CheckState, Content, Entry, LoginOption, Proposal, Proposed,
    State, Step, StepState, ToolCallResult,
};
use crate::chat::view::ChatView;
use crate::ui::ellipsis::ellipsis;
use crate::ui::text::Writer;
use crate::ui::theme::{
    ACCENT, ACCENT_GROUND, ADDED_GROUND, ASKING, ASKING_BUTTON, ASKING_EDGE, ASKING_GROUND, BORDER,
    BORDER_STRONG, BUTTON, BUTTON_HOVER, CODE_FONT, DIM, DISABLED, FAILED, FAILED_GROUND,
    FAILED_MARK, FAINT, FOREGROUND, GROUND, HOVER, LINK, OFFERING_BUTTON, OFFERING_EDGE,
    OFFERING_GROUND, RAISED, REMOVED_GROUND, SURFACE, chat_color,
};
use crate::vim::hints::Hinted as _;

/// How tall a picture shown in the conversation grows.
const MEDIA_HEIGHT: f32 = 360.;
/// The widest a picture in the conversation is drawn, the panel narrowing it
/// further.
const MEDIA_WIDTH: f32 = 480.;

/// How tall what a tool call shows grows before it scrolls on its own.
const CONTENT_HEIGHT: f32 = 320.;

/// How far the work of a fold, and the steps of a plan, are moved right,
/// under their line.
const INDENT: f32 = 16.;

/// What the conversation shows in one place: one entry, or the work of
/// the entries in a range, which the agent did one after another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Shown {
    Entry(usize),
    Work(Range<usize>),
    /// What the human closes the work on, after the entries, which
    /// number this many (see [`closing`]).
    Closing(usize),
}

/// Who says an entry, as the timeline of the conversation names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speaker {
    Human,
    /// Cox, in the human's place.
    Cox,
    Agent,
}

/// What happened, as the line of what ran on it names it: the event, as
/// a trigger's `on` names it.
fn event_label(on: &str) -> &'static str {
    match on {
        "goal.met" => "GOAL MET",
        "wiki.written" => "WIKI WRITTEN",
        "sketch.end" => "SKETCH ENDED",
        "chat.open" => "CHAT OPENED",
        "plan.changed" => "PLAN CHANGED",
        "permission.asked" => "PERMISSION ASKED",
        _ => "TRIGGERS",
    }
}

impl Speaker {
    /// Who says `entry`: none for Cox's notes, which go with whoever
    /// spoke last.
    fn of(entry: &Entry) -> Option<Self> {
        match entry {
            Entry::Human(_) | Entry::Attached(_) => Some(Self::Human),
            Entry::Cox(_) | Entry::Keep { .. } | Entry::Checks(_) | Entry::Event { .. } => {
                Some(Self::Cox)
            }
            Entry::Note(_) | Entry::Switched { .. } => None,
            _ => Some(Self::Agent),
        }
    }
}

/// What the timeline marks beside an entry: a speaker, by name, where
/// they start, in their color, or, where the entry names them itself, by
/// the dot alone, or what asks the human.
#[derive(Clone, Debug)]
enum Mark {
    Named(SharedString, u32),
    Starts(u32),
    Asking,
}

/// One place of the conversation: what shows there, what the timeline
/// marks beside it, and, for the work of an agent where it starts with
/// it, the agent's name, on the work's own line.
#[derive(Clone, Debug)]
pub(super) struct Place {
    shown: Shown,
    mark: Option<Mark>,
    name: Option<SharedString>,
}

impl Place {
    /// The first entry the place shows, which stays the same as the
    /// place grows, as work does with each step.
    pub(super) fn first(&self) -> usize {
        match &self.shown {
            Shown::Entry(index) | Shown::Closing(index) => *index,
            Shown::Work(work) => work.start,
        }
    }
}

/// The name of the agent, along the conversation: the agent the chat
/// talked to first, until it switched to another.
struct AgentNames {
    now: String,
}

impl AgentNames {
    /// The names along `entries`, of which the agent the chat talks to now
    /// is `current`.
    fn new(entries: &[Entry], current: String) -> Self {
        let first = entries.iter().find_map(|entry| match entry {
            Entry::Switched { from, .. } => Some(from.clone()),
            _ => None,
        });
        Self {
            now: first.unwrap_or(current),
        }
    }

    fn switch(&mut self, to: &str) {
        to.clone_into(&mut self.now);
    }

    fn now(&self) -> &str {
        &self.now
    }
}

/// How far right of the timeline's line what is on it starts, and where
/// the line runs.
const TIMELINE_INDENT: f32 = 28.;
const LINE_LEFT: f32 = 9.;

/// `element` on the timeline down the conversation's left: the line,
/// from the first dot down, and the dot and name of `mark`, if it has one.
fn on_timeline(first: bool, mark: Option<Mark>, element: AnyElement) -> AnyElement {
    let starts = matches!(mark, Some(Mark::Named(..) | Mark::Starts(_)));
    let (dot, name) = match mark {
        Some(Mark::Named(name, color)) => (Some(color), Some(name)),
        Some(Mark::Starts(color)) => (Some(color), None),
        Some(Mark::Asking) => (Some(ASKING), None),
        None => (None, None),
    };
    div()
        .relative()
        .pl(px(TIMELINE_INDENT))
        .pb(px(10.))
        .when(starts && !first, |row| row.pt(px(8.)))
        .child(
            div()
                .absolute()
                .left(px(LINE_LEFT))
                .top(px(if first { 10. } else { 0. }))
                .bottom_0()
                .w(px(2.))
                .bg(rgb(BORDER)),
        )
        .children(dot.map(|color| {
            div()
                .absolute()
                .left(px(LINE_LEFT - 5.))
                .top(px(if starts && !first { 12. } else { 4. }))
                .size(px(12.))
                .rounded_full()
                .bg(rgb(color))
                .border_3()
                .border_color(rgb(SURFACE))
        }))
        .children(name.map(|name| label().child(name)))
        .child(element)
        .into_any_element()
}

/// The text of a speaker's name, and of what goes with it.
fn label() -> gpui::Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(FAINT))
}

/// Whether `entry` is work the agent does on its way to a reply.
fn is_work(entry: &Entry) -> bool {
    matches!(
        entry,
        Entry::ToolCall { .. } | Entry::Thought(_) | Entry::Plan(_)
    )
}

/// What the conversation shows of `entries`, in order: two entries of
/// work or more in a row fold into one place, and so does a tool call
/// alone, to show as a step.
pub(super) fn shown(entries: &[Entry]) -> Vec<Shown> {
    let mut shown = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let work = entries[index..]
            .iter()
            .take_while(|entry| is_work(entry))
            .count();
        if work > 1 || matches!(entries[index], Entry::ToolCall { .. }) {
            shown.push(Shown::Work(index..index + work));
            index += work;
        } else {
            shown.push(Shown::Entry(index));
            index += 1;
        }
    }
    shown
}

/// What the human closes the work their last line left the agent on, in
/// one place, as the phone gathers it: what the agent showed, what it
/// decided on its own, against the plan it started with, and how the
/// checks went as its last turn ended, with those of the goal met.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Closing<'a> {
    /// The pictures and videos shown, by the indexes of their entries.
    pub shown: Vec<usize>,
    /// The steps the agent added and removed, as the plan stands last.
    pub decided: Option<(&'a [String], &'a [String])>,
    pub checks: Vec<&'a Check>,
}

/// What the human closes the work on, from what `entries` holds since
/// the human's last line: none before they sent one, once they asked the
/// agent to merge it, until their next line, or when the work has nothing
/// to close on.
pub(super) fn closing(entries: &[Entry]) -> Option<Closing<'_>> {
    let from = entries
        .iter()
        .rposition(|entry| matches!(entry, Entry::Human(_)))?;
    let mut closing = Closing {
        shown: Vec::new(),
        decided: None,
        checks: Vec::new(),
    };
    let mut ended: Option<&[Check]> = None;
    let mut met = Vec::new();
    for (index, entry) in entries.iter().enumerate().skip(from + 1) {
        match entry {
            Entry::Merge => return None,
            Entry::Media { .. } | Entry::Shown { .. } => closing.shown.push(index),
            Entry::PlanChanged { added, removed } => {
                closing.decided = Some((added.as_slice(), removed.as_slice()));
            }
            Entry::Checks(checks) => ended = Some(checks),
            Entry::Event { on, checks } if on == "goal.met" => met.extend(checks),
            _ => {}
        }
    }
    closing.decided = closing
        .decided
        .filter(|(added, removed)| !added.is_empty() || !removed.is_empty());
    if let Some(ended) = ended {
        closing.checks.extend(ended);
        closing.checks.extend(met);
    }
    (!closing.shown.is_empty() || closing.decided.is_some() || !closing.checks.is_empty())
        .then_some(closing)
}

/// A part of the conversation that the human opens and closes: what the
/// entry at an index shows under its line, the work folded from the
/// entry at an index on, or the line of the tool call at an index under
/// the steps of its fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Fold {
    Content(usize),
    Work(usize),
    Step(usize),
    /// What one of what ran as a turn ended said, by its place among
    /// them, under their chips.
    Check(usize, usize),
}

/// The parts the human opened or closed, which show otherwise as they do
/// at first.
#[derive(Debug, Default)]
pub(super) struct Folds(HashSet<Fold>);

impl Folds {
    /// Whether `fold` is open, which it is at first when `at_first` is.
    fn is_open(&self, fold: Fold, at_first: bool) -> bool {
        at_first != self.0.contains(&fold)
    }

    /// Opens `fold` if it is closed, or else closes it.
    fn flip(&mut self, fold: Fold) {
        if !self.0.remove(&fold) {
            self.0.insert(fold);
        }
    }
}

impl ChatView {
    /// The places of the conversation, in order: what each shows, and
    /// what the timeline marks beside it. Working them out reads the
    /// entries alone, so it is cheap; drawing them is not, and the list
    /// draws only the places in sight (see [`Self::render_place`]).
    pub(super) fn places(&self) -> Vec<Place> {
        let entries = self.conversation.entries();
        let mut agents = AgentNames::new(entries, self.agent_name());
        let mut speaking = None;
        let mut places = Vec::new();
        for shown in shown(entries) {
            let first = match &shown {
                Shown::Entry(index) | Shown::Closing(index) => *index,
                Shown::Work(work) => work.start,
            };
            let entry = &entries[first];
            if let Entry::Switched { to, .. } = entry {
                agents.switch(to);
            }
            let speaker = Speaker::of(entry);
            // A speaker is named where they start, and what asks the
            // human is marked wherever it is.
            let asking = matches!(entry, Entry::Permission { .. })
                && self.conversation.asked_at() == Some(first);
            let starts = speaker.is_some() && speaker != speaking;
            if speaker.is_some() {
                speaking = speaker;
            }
            let mut mark = if asking {
                Some(Mark::Asking)
            } else if starts {
                speaker.map(|speaker| match speaker {
                    Speaker::Human => Mark::Named("YOU".into(), FOREGROUND),
                    Speaker::Cox => Mark::Named("COX".into(), FAINT),
                    Speaker::Agent => {
                        Mark::Named(agents.now().to_uppercase().into(), chat_color(self.key))
                    }
                })
            } else {
                None
            };
            // The work names who did it on its own line, ahead of its
            // steps.
            let name = match (&shown, mark.take()) {
                (Shown::Work(_), Some(Mark::Named(name, color))) => {
                    mark = Some(Mark::Starts(color));
                    Some(name)
                }
                (_, other) => {
                    mark = other;
                    None
                }
            };
            places.push(Place { shown, mark, name });
        }
        if self.closes() {
            places.push(Place {
                shown: Shown::Closing(entries.len()),
                mark: None,
                name: None,
            });
        }
        places
    }

    /// Whether the chat shows what the human closes the work on: it waits
    /// for them after the work their last line left it, its agent merges
    /// what it made, as one that works in the project does and a sketch's
    /// does not, and the work has something to close on.
    fn closes(&self) -> bool {
        let waits = match self.conversation.state() {
            State::Idle => true,
            State::Closed => self.row.again,
            _ => false,
        };
        waits
            && crate::daemon::merges(self.row.setup.as_ref())
            && closing(self.conversation.entries()).is_some()
    }

    /// Place `place` of the conversation, as [`Self::places`] last worked
    /// them out, on the timeline.
    pub(super) fn render_place(&self, place: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(Place { shown, mark, name }) = self.places.get(place).cloned() else {
            return div().into_any_element();
        };
        let element = match shown {
            Shown::Entry(index) => {
                let entries = self.conversation.entries();
                match entries.get(index) {
                    Some(entry) => self.render_entry(index, entry, cx),
                    None => div().into_any_element(),
                }
            }
            Shown::Work(work) => self.render_work(work, name, cx),
            Shown::Closing(index) => self.render_closing(index, cx),
        };
        let marked = self.marked.is_some_and(|marked| {
            self.places
                .get(place)
                .is_some_and(|at| at.first() == marked)
        });
        // The list lays each place out on its own, so each keeps the
        // conversation's margins itself.
        div()
            .px(px(14.))
            .when(place == 0, |first| first.pt(px(16.)))
            .child(
                div()
                    .rounded(px(10.))
                    .when(marked, |place| place.bg(rgb(ACCENT_GROUND)))
                    .child(on_timeline(place == 0, mark, element)),
            )
            .into_any_element()
    }

    /// Whether the entry at `index` is still arriving: the last of a turn
    /// that runs.
    fn is_arriving(&self, index: usize) -> bool {
        matches!(self.conversation.state(), State::Busy | State::Stopping)
            && index + 1 == self.conversation.entries().len()
    }

    /// The work the agent did one after another, under a line that names
    /// who did it, `name`, where they start with it, and counts its steps,
    /// the tool calls, and tells how far its plan has gone. Closed, it
    /// shows each call as a chip of how it went, which opens the call's
    /// line under the chips, and the last thought, which shows as it
    /// arrives; open, it shows each entry on its line.
    fn render_work(
        &self,
        work: Range<usize>,
        name: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entries = self.conversation.entries();
        let fold = Fold::Work(work.start);
        let open = self.folds.is_open(fold, false);
        let calls: Vec<usize> = work
            .clone()
            .filter(|&index| matches!(entries[index], Entry::ToolCall { .. }))
            .collect();
        let thoughts = work
            .clone()
            .filter(|&index| matches!(entries[index], Entry::Thought(_)))
            .count();
        let plan = entries[work.clone()]
            .iter()
            .rev()
            .find_map(|entry| match entry {
                Entry::Plan(steps) => Some(steps),
                _ => None,
            });
        let mut told: Vec<String> = name.into_iter().map(String::from).collect();
        if !calls.is_empty() {
            told.push(counted(calls.len(), "step").to_uppercase());
        } else if thoughts > 0 {
            told.push(counted(thoughts, "thought").to_uppercase());
        }
        if let Some(steps) = plan {
            told.push(format!("PLAN {}", progress(steps)));
        }
        let start = work.start;
        let header = label()
            .id(("work", start))
            .debug_selector(move || format!("work-{start}"))
            .flex()
            .gap_1p5()
            .cursor_pointer()
            .hover(|style| style.text_color(rgb(FOREGROUND)))
            .child(told.join(" · "))
            .child(disclosure(Some(open)).text_color(rgb(FAINT)))
            .on_hint_click(cx.listener(move |view, _, _, cx| {
                view.folds.flip(fold);
                cx.notify();
            }));
        let body = if open {
            work.map(|index| self.render_entry(index, &entries[index], cx))
                .collect()
        } else {
            let mut body = Vec::new();
            if !calls.is_empty() {
                let chips = calls.iter().map(|&index| self.render_step(index, cx));
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .children(chips)
                        .into_any_element(),
                );
            }
            body.extend(
                calls
                    .iter()
                    .filter(|&&index| self.folds.is_open(Fold::Step(index), false))
                    .map(|&index| self.render_entry(index, &entries[index], cx)),
            );
            let last_thought = work
                .rev()
                .find(|&index| matches!(entries[index], Entry::Thought(_)));
            body.extend(last_thought.map(|index| self.render_entry(index, &entries[index], cx)));
            body
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(header)
            .children(body)
            .into_any_element()
    }

    /// The tool call at `index` as a step of its fold: a chip with a mark
    /// of how it goes, a ring while it runs and then a dot, and its title,
    /// short, which opens or closes the call's
    /// line under the chips.
    fn render_step(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Entry::ToolCall { title, result, .. } = &self.conversation.entries()[index] else {
            return div().into_any_element();
        };
        let (dot, ground, text) = match result {
            None => (LINK, RAISED, DIM),
            Some(ToolCallResult::Completed) => (ACCENT, RAISED, DIM),
            Some(ToolCallResult::Failed) => (FAILED_MARK, FAILED_GROUND, FAILED),
            Some(ToolCallResult::Cancelled) => (DISABLED, RAISED, FAINT),
        };
        let fold = Fold::Step(index);
        let open = self.folds.is_open(fold, false);
        div()
            .id(("step", index))
            .debug_selector(move || format!("step-{index}"))
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .px(px(10.))
            .max_w(px(320.))
            .rounded_full()
            .bg(rgb(ground))
            .border_1()
            .border_color(rgb(if open { BORDER_STRONG } else { ground }))
            .text_color(rgb(text))
            .text_size(px(12.))
            .font_family(CODE_FONT)
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(BORDER_STRONG)))
            .child(
                // A ring while the call runs, filled once it has gone.
                div()
                    .flex_shrink_0()
                    .size(px(7.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(dot))
                    .when(result.is_some(), |mark| mark.bg(rgb(dot))),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .child(ellipsis(SharedString::from(short(title)))),
            )
            .on_hint_click(cx.listener(move |view, _, _, cx| {
                view.folds.flip(fold);
                cx.notify();
            }))
            .into_any_element()
    }

    /// What runs as a turn ends, or as something else happens, on one
    /// line: `TURN END`, or what happened, as `what` names it, then a chip
    /// for the goal and each trigger, with a mark of how it went, which a
    /// click opens what it said under, once it ran.
    fn render_checks(
        &self,
        index: usize,
        what: &'static str,
        checks: &[Check],
        writer: &mut Writer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips = checks.iter().enumerate().map(|(at, check)| {
            let fold = Fold::Check(index, at);
            let open = check.said.is_some() && self.folds.is_open(fold, false);
            check_chip(format!("check-{index}-{at}"), check)
                .border_color(rgb(if open { BORDER_STRONG } else { RAISED }))
                .when(check.said.is_some(), |chip| {
                    chip.cursor_pointer()
                        .hover(|style| style.border_color(rgb(BORDER_STRONG)))
                        .on_hint_click(cx.listener(move |view, _, _, cx| {
                            view.folds.flip(fold);
                            cx.notify();
                        }))
                })
        });
        let said = checks.iter().enumerate().filter_map(|(at, check)| {
            let said = check.said.as_ref()?;
            self.folds
                .is_open(Fold::Check(index, at), false)
                .then(|| Content::Text(said.clone()))
        });
        let said: Vec<Content> = said.collect();
        div()
            .debug_selector(move || format!("checks-{index}"))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .child(label().mr_1().child(what))
                    .children(chips),
            )
            .children(render_content(index, &said, writer))
            .into_any_element()
    }

    /// What the human closes the work on, as [`closing`] gathers it, on a
    /// card after the conversation, whose entries number `entries`: what
    /// the agent showed, which a click opens on the stage, what it decided
    /// on its own, and how the checks went, with Merge, which asks the
    /// agent, in the human's place, to merge what it made.
    fn render_closing(&self, entries: usize, cx: &mut Context<Self>) -> AnyElement {
        let all = self.conversation.entries();
        let Some(closing) = closing(all) else {
            return div().into_any_element();
        };
        let mut writer = self.reading.writer(entries);
        let part = |name: &'static str| label().child(name);
        let mut shows = Vec::new();
        // What changed where the chat works, as the stage's bar counts it,
        // which opens on "Changes", as on the phone.
        if let Some(changed) = self
            .stage()
            .and_then(|stage| stage.read(cx).changed_files())
        {
            shows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(part("CHANGED"))
                    .child(render_changed(changed, cx))
                    .into_any_element(),
            );
        }
        if !closing.shown.is_empty() {
            let thumbnails = closing
                .shown
                .iter()
                .filter_map(|&index| render_tile(index, all.get(index)?, cx));
            shows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(part("TO SEE"))
                    .child(div().flex().flex_wrap().gap(px(8.)).children(thumbnails))
                    .into_any_element(),
            );
        }
        if let Some((added, removed)) = closing.decided {
            shows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(part("DECIDED ON ITS OWN"))
                    .children(drift_steps(added, removed, &mut writer))
                    .into_any_element(),
            );
        }
        if !closing.checks.is_empty() {
            let passed = closing
                .checks
                .iter()
                .all(|check| check.state == CheckState::Passed);
            let chips = closing
                .checks
                .iter()
                .enumerate()
                .map(|(at, check)| check_chip(format!("closing-check-{at}"), check));
            shows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(part(if passed { "CHECKS PASSED" } else { "CHECKS" }))
                    .child(div().flex().flex_wrap().gap(px(6.)).children(chips))
                    .into_any_element(),
            );
        }
        shows.push(
            div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.plain(
                    "Merge asks the agent to merge it, as the project merges its work.",
                    "\n",
                ))
                .into_any_element(),
        );
        let merge = answer_button("merge", "merge".to_owned(), "Merge", true, OFFERING_TONE)
            .on_hint_click(cx.listener(|view, _, _, cx| {
                view.merge();
                cx.notify();
            }));
        answer_card(
            OFFERING_TONE,
            card_head("✓", OFFERING_TONE, "Close the work"),
            "for you to merge",
            shows,
            [merge],
        )
        .debug_selector(|| "closing".to_owned())
        .into_any_element()
    }

    /// The card that asks the human to log in, one of the ways of
    /// `options`, while the agent waits for it, and after, the way `chosen`.
    fn render_login(
        &self,
        index: usize,
        options: &[LoginOption],
        failed: Option<&str>,
        chosen: Option<usize>,
        writer: &mut Writer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.conversation.asked_at() == Some(index) {
            let title = conversation::login_title(failed);
            let names: Vec<String> = options.iter().map(conversation::login_label).collect();
            return render_asking(index, (&title, &[], &[]), &names, writer, cx);
        }
        let chosen = chosen
            .and_then(|chosen| options.get(chosen))
            .map_or("none", |option| option.name.as_str());
        div()
            .text_size(px(12.))
            .text_color(rgb(DIM))
            .child(writer.plain(&format!("Log in: {chosen}"), "\n\n"))
            .into_any_element()
    }

    /// What the human handed the chat with the line before, a chip each,
    /// which a click opens, when it is a file or a page.
    fn render_attached_entry(
        index: usize,
        attached: &[Attachment],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .debug_selector(|| format!("attached-{index}"))
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .pb(px(12.))
            .children(attached.iter().enumerate().map(|(at, attachment)| {
                let uri = attachment.uri.clone();
                attachment_chip(
                    SharedString::from(format!("attached-{index}-{at}")).into(),
                    attachment,
                )
                .on_hint_click(cx.listener(move |view, _, window, cx| {
                    if opens(&uri) {
                        view.show_link(&uri, window, cx);
                    }
                }))
            }))
            .into_any_element()
    }

    #[expect(clippy::too_many_lines, reason = "one arm for each kind of entry")]
    fn render_entry(&self, index: usize, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let mut writer = self.reading.writer(index);
        match entry {
            Entry::Human(text) => div()
                .text_size(px(15.))
                .child(writer.plain(text, "\n\n"))
                .into_any_element(),
            Entry::Attached(attached) => Self::render_attached_entry(index, attached, cx),
            // Sent in the human's place, so drawn in theirs, but as Cox's
            // notes are.
            Entry::Cox(text) => div()
                .debug_selector(|| format!("cox-{index}"))
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.plain(text, "\n\n"))
                .into_any_element(),
            Entry::Reply(text) => writer
                .markdown(text)
                .debug_selector(|| format!("reply-{index}"))
                .into_any_element(),
            Entry::ToolCall {
                id,
                title,
                content,
                result,
            } => {
                let (mark, color) = match result {
                    None => ("●", LINK),
                    Some(ToolCallResult::Completed) => ("✓", ACCENT),
                    Some(ToolCallResult::Failed) => ("✗", FAILED),
                    Some(ToolCallResult::Cancelled) => ("–", DIM),
                };
                let fold = Fold::Content(index);
                // The output of a call that failed tells why.
                let open = self
                    .folds
                    .is_open(fold, *result == Some(ToolCallResult::Failed));
                let opens = (!content.is_empty()).then_some(open);
                let line = line_of(index, opens, (mark, color), title, &mut writer);
                let line = line.child(flow_button(index, id, cx));
                let line = folding(line, opens.map(|_| fold), cx);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(line)
                    .when(open, |call| {
                        call.children(render_content(index, content, &mut writer))
                    })
                    .into_any_element()
            }
            Entry::Thought(text) => self.render_thought(index, text, &mut writer, cx),
            Entry::Plan(steps) => self.render_plan_entry(index, steps, cx),
            Entry::PlanChanged { added, removed } => {
                render_drift(index, added, removed, &mut writer)
            }
            Entry::Proposal(proposal) => render_proposal(index, proposal, &mut writer, cx),
            Entry::Keep { why, kept, .. } => render_keep(index, why, *kept, &mut writer, cx),
            Entry::Media {
                file,
                kind,
                from,
                caption,
                size,
            } => render_media(index, (file, *kind, from, caption.as_deref()), *size, cx),
            Entry::Shown {
                path,
                markdown,
                lines,
                ..
            } => render_shown(index, path, *markdown, *lines, cx),
            Entry::Commits(commits) => render_commits(index, commits, cx),
            Entry::Checks(checks) => self.render_checks(index, "TURN END", checks, &mut writer, cx),
            Entry::Event { on, checks } => {
                self.render_checks(index, event_label(on), checks, &mut writer, cx)
            }
            Entry::Note(text) => div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.noted(text, "\n\n"))
                .into_any_element(),
            Entry::Switched { from, to } => div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.plain(&conversation::switched_note(from, to), "\n\n"))
                .into_any_element(),
            Entry::Merge => div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.plain("You asked the agent to merge what it made.", "\n\n"))
                .into_any_element(),
            Entry::Login {
                options,
                failed,
                chosen,
            } => self.render_login(index, options, failed.as_deref(), *chosen, &mut writer, cx),
            Entry::Permission {
                title,
                content,
                options,
                chosen,
                by_setup,
                gathered,
                ..
            } => {
                if self.conversation.asked_at() == Some(index) {
                    let asked = (title.as_str(), content.as_slice(), gathered.as_slice());
                    render_asking(index, asked, options, &mut writer, cx)
                } else {
                    let answer = chosen
                        .and_then(|chosen| options.get(chosen))
                        .map_or("no answer", String::as_str);
                    // The human sees that they were not asked, and why.
                    let answer = if *by_setup {
                        format!("{answer}, as the chat is set up")
                    } else {
                        answer.to_owned()
                    };
                    let asked = (title.as_str(), content.as_slice(), gathered.as_slice());
                    self.render_answered(index, asked, answer, &mut writer, cx)
                }
            }
        }
    }

    /// A permission request answered with `answer`, on a line that opens
    /// to what was asked, if the request showed something.
    fn render_answered(
        &self,
        index: usize,
        (title, content, gathered): (&str, &[Content], &[Check]),
        answer: String,
        writer: &mut Writer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // What was asked was read when it was answered.
        let fold = Fold::Content(index);
        let open = self.folds.is_open(fold, false);
        let opens = (!content.is_empty() || !gathered.is_empty()).then_some(open);
        let line = line_of(index, opens, ("?", ASKING), title, writer).child(
            div()
                .flex_shrink_0()
                .text_color(rgb(DIM))
                .child(SharedString::from(answer)),
        );
        let line = folding(line, opens.map(|_| fold), cx);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(line)
            .when(open, |asked| {
                asked
                    .children(render_gathered(index, gathered, writer))
                    .children(render_content(index, content, writer))
            })
            .into_any_element()
    }

    /// What the agent thought, as an aside: its start, in italics on a
    /// line of its own, which opens to all of it.
    fn render_thought(
        &self,
        index: usize,
        text: &str,
        writer: &mut Writer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold = Fold::Content(index);
        // What the agent is thinking shows as it thinks it.
        let open = self.folds.is_open(fold, self.is_arriving(index));
        let line = div()
            .id(("line", index))
            .debug_selector(move || format!("line-{index}"))
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(ellipsis(if open {
                        "Thinking".to_owned()
                    } else {
                        text.lines().next().unwrap_or_default().to_owned()
                    })),
            );
        div()
            .flex()
            .flex_col()
            .gap_1()
            .italic()
            .text_size(px(13.))
            .text_color(rgb(FAINT))
            .child(
                // An aside, which only its text marks as one to open.
                line.cursor_pointer()
                    .hover(|style| style.text_color(rgb(DIM)))
                    .on_hint_click(cx.listener(move |view, _, _, cx| {
                        view.folds.flip(fold);
                        cx.notify();
                    })),
            )
            .when(open, |thought| thought.child(writer.markdown(text)))
            .into_any_element()
    }

    /// The plan the agent worked to, on a line that tells how far it went,
    /// opening to its steps.
    fn render_plan_entry(
        &self,
        index: usize,
        steps: &[Step],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold = Fold::Content(index);
        let open = self.folds.is_open(fold, false);
        let line = div()
            .id(("line", index))
            .debug_selector(move || format!("line-{index}"))
            .flex()
            .gap_2()
            .text_color(rgb(DIM))
            .child(disclosure(Some(open)))
            .child(div().flex_shrink_0().child("≡"))
            .child(format!("Plan {}", progress(steps)));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(folding(line, Some(fold), cx))
            .when(open, |plan| plan.child(render_steps(steps).pl(px(INDENT))))
            .into_any_element()
    }
}

/// How the plan differs from the one the human left the agent to: each
/// step `added`, in amber, as what the agent decided on its own, and each
/// it `removed`, dimmer.
fn render_drift(
    index: usize,
    added: &[String],
    removed: &[String],
    writer: &mut Writer,
) -> AnyElement {
    div()
        .debug_selector(move || format!("plan-changed-{index}"))
        .flex()
        .flex_col()
        .gap_1()
        .child(label().child("PLAN CHANGED"))
        .children(drift_steps(added, removed, writer))
        .into_any_element()
}

/// The steps of how the plan differs from the one the human left the
/// agent to, as [`render_drift`] shows them.
fn drift_steps(added: &[String], removed: &[String], writer: &mut Writer) -> Vec<gpui::Div> {
    let step = |mark: &'static str, color: u32, text: &str, writer: &mut Writer| {
        div()
            .flex()
            .gap_2()
            .text_size(px(13.))
            .text_color(rgb(color))
            .child(div().flex_shrink_0().w(px(10.)).child(mark))
            .child(div().flex_1().min_w_0().child(writer.plain(text, "\n")))
    };
    let mut steps: Vec<gpui::Div> = added
        .iter()
        .map(|text| step("+", ASKING, text, writer))
        .collect();
    steps.extend(removed.iter().map(|text| step("−", DIM, text, writer)));
    if steps.is_empty() {
        steps.push(step(
            "=",
            DIM,
            "The plan is back to the one the line started with.",
            writer,
        ));
    }
    steps
}

/// A chip of one of what ran as a turn ended, `check`, which a test finds
/// as `selector`: its name, after a ring while it waits or runs, and a
/// dot once it went: passed, found something, or could not run.
/// The commits the agent made in a turn, at `index`, as chips on a line,
/// each of which a click shows on the stage's "Changes" of the folder the
/// chat worked in, as what the commit changed.
fn render_commits(
    index: usize,
    commits: &[cox_core::entry::Committed],
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let chips = commits.iter().enumerate().map(|(at, commit)| {
        let selector = format!("commit-{index}-{at}");
        let (folder, name, id) = (
            commit.folder.clone(),
            commit.repository.clone(),
            commit.id.clone(),
        );
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(24.))
            .px(px(10.))
            .max_w(px(320.))
            .rounded_full()
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(RAISED))
            .text_size(px(12.))
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(BORDER_STRONG)))
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(7.))
                    .rounded_full()
                    .bg(rgb(ACCENT)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .font_family(CODE_FONT)
                    .text_color(rgb(FAINT))
                    .child(commit.short().to_owned()),
            )
            .children(commit.repository.clone().map(|name| {
                div()
                    .flex_shrink_0()
                    .font_family(CODE_FONT)
                    .text_color(rgb(FAINT))
                    .child(name)
            }))
            .children(commit.amended.is_some().then(|| {
                div()
                    .flex_shrink_0()
                    .text_color(rgb(ASKING))
                    .child("amended")
            }))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(rgb(DIM))
                    .child(ellipsis(commit.subject.clone())),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .child(crate::stage::changes::counts(commit.added, commit.removed)),
            )
            .on_hint_click(cx.listener(move |view, _, window, cx| {
                if let Some(stage) = view.stage() {
                    let (folder, name, id) = (folder.clone(), name.clone(), id.clone());
                    stage.update(cx, |stage, cx| {
                        stage.show_commit(&folder, name, id, window, cx);
                    });
                }
            }))
    });
    div()
        .debug_selector(move || format!("commits-{index}"))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .child(label().mr_1().child("COMMITTED"))
        .children(chips)
        .into_any_element()
}

fn check_chip(selector: String, check: &Check) -> Stateful<gpui::Div> {
    let (dot, filled, text) = match check.state {
        CheckState::Waiting => (DISABLED, false, FAINT),
        CheckState::Running => (LINK, false, DIM),
        CheckState::Passed => (ACCENT, true, DIM),
        CheckState::Failed => (ASKING, true, DIM),
        CheckState::Broken => (FAILED_MARK, true, FAILED),
    };
    div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .px(px(10.))
        .max_w(px(240.))
        .rounded_full()
        .bg(rgb(RAISED))
        .border_1()
        .border_color(rgb(RAISED))
        .text_color(rgb(text))
        .text_size(px(12.))
        .font_family(CODE_FONT)
        .child(
            div()
                .flex_shrink_0()
                .size(px(7.))
                .rounded_full()
                .border_1()
                .border_color(rgb(dot))
                .when(filled, |mark| mark.bg(rgb(dot))),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .child(ellipsis(SharedString::from(check.name.clone()))),
        )
}

/// The colors of a card that waits for the human to answer: in amber
/// for what asks them, as a permission request, and in the accent for
/// what Cox or the agent offers them, as a goal to set.
#[derive(Clone, Copy)]
struct Tone {
    /// The mark at the card's head, its tag, and the first button.
    mark: u32,
    ground: u32,
    edge: u32,
    /// The buttons but the first.
    button: u32,
}

const ASKING_TONE: Tone = Tone {
    mark: ASKING,
    ground: ASKING_GROUND,
    edge: ASKING_EDGE,
    button: ASKING_BUTTON,
};

const OFFERING_TONE: Tone = Tone {
    mark: ACCENT,
    ground: OFFERING_GROUND,
    edge: OFFERING_EDGE,
    button: OFFERING_BUTTON,
};

/// A card in `tone` that waits for the human: `head` at its top, with
/// `tag` at its right, what it shows, and `buttons` along its foot.
fn answer_card(
    tone: Tone,
    head: impl IntoElement,
    tag: &'static str,
    shows: impl IntoIterator<Item = AnyElement>,
    buttons: impl IntoIterator<Item = Stateful<gpui::Div>>,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(14.))
        .rounded(px(14.))
        .bg(rgb(tone.ground))
        .border_1()
        .border_color(rgb(tone.edge))
        .child(
            div()
                .flex()
                .items_start()
                .gap_2()
                .child(div().flex_1().min_w_0().child(head))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(tone.mark))
                        .child(tag),
                ),
        )
        .children(shows)
        .child(div().flex().gap(px(6.)).children(buttons))
}

/// A button of a card in `tone`, which a test finds as `selector`: the
/// first, which the human most likely answers, stands out.
fn answer_button(
    id: impl Into<gpui::ElementId>,
    selector: String,
    label: impl IntoElement,
    first: bool,
    tone: Tone,
) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .debug_selector(move || selector)
        .flex_1()
        .min_w_0()
        .h(px(34.))
        .px_2()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.))
        .text_size(px(13.))
        .cursor_pointer()
        .map(|button| {
            if first {
                button
                    .bg(rgb(tone.mark))
                    .text_color(rgb(GROUND))
                    .font_weight(FontWeight::SEMIBOLD)
                    .hover(|style| style.opacity(0.9))
            } else {
                button
                    .bg(rgb(tone.button))
                    .text_color(rgb(FOREGROUND))
                    .hover(|style| style.bg(rgb(tone.edge)))
            }
        })
        .child(label)
}

/// The head of a card that waits for the human: its mark, in the card's
/// tone, and what it is.
fn card_head(mark: &'static str, tone: Tone, what: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .gap_2()
        .child(div().flex_shrink_0().text_color(rgb(tone.mark)).child(mark))
        .child(div().flex_1().min_w_0().child(what))
}

/// How big the icon an agent proposes for its project is drawn on the
/// card that proposes it, square.
const PROPOSED_ICON: f32 = 64.;

/// How big that icon is drawn on the line the card leaves once the human
/// answered it, square.
const ANSWERED_ICON: f32 = 16.;

/// The icon kept as `file`, as the rail draws a project's, rounded,
/// `size` points square.
fn icon_of(file: &str, size: f32) -> Option<AnyElement> {
    let (path, _) = crate::media::kept(file)?;
    Some(
        gpui::img(path)
            .size(px(size))
            .flex_shrink_0()
            .rounded(px(size / 4.))
            .into_any_element(),
    )
}

/// What the agent proposed: while the human has not answered it, a card
/// that shows it as its kind does, with why the agent proposes it, and
/// buttons to carry it out, to change it first, for a goal, or to let it
/// be; then a line that says what they did.
fn render_proposal(
    index: usize,
    Proposal {
        proposed,
        why,
        answer,
    }: &Proposal,
    writer: &mut Writer,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let goal = match proposed {
        Proposed::Goal(goal) => {
            Goal::parse(goal).map_or_else(|| goal.clone(), |goal| conversation::line(&goal))
        }
        _ => String::new(),
    };
    let (mark, selector, title) = match proposed {
        Proposed::Goal(_) => ("◎", "proposal", format!("Proposed goal: {goal}")),
        Proposed::Name(name) => ("✎", "name-proposal", format!("Proposed name: {name}")),
        Proposed::About(about) => (
            "✎",
            "about-proposal",
            format!("What the project is: {about}"),
        ),
        Proposed::Icon { from, .. } => (
            "✎",
            "icon-proposal",
            format!("Proposed icon, from `{from}`"),
        ),
        Proposed::Project { name, .. } => {
            ("✦", "project-proposal", format!("Proposed project: {name}"))
        }
    };
    let selector = move || format!("{selector}-{index}");
    if let Some(answer) = answer {
        let done = match (proposed, answer) {
            (_, Answer::Dismissed) => "let be",
            (Proposed::Goal(_), Answer::Changed) => "changed, and set",
            (Proposed::Name(_), _) => "renamed",
            (Proposed::Project { .. }, _) => "made",
            (Proposed::Goal(_) | Proposed::About(_) | Proposed::Icon { .. }, _) => "set",
        };
        let icon = match proposed {
            Proposed::Icon { file, .. } => icon_of(file, ANSWERED_ICON),
            _ => None,
        };
        return line_of(index, None, (mark, LINK), &title, writer)
            .debug_selector(selector)
            .children(icon)
            .child(div().flex_shrink_0().text_color(rgb(DIM)).child(done))
            .into_any_element();
    }
    let (head, tag, (set, set_label), dismiss) = card_words(proposed);
    let button = |name: &'static str, label: &'static str, first: bool| {
        let id = format!("{name}-{index}");
        answer_button(
            SharedString::from(id.clone()),
            id,
            label,
            first,
            OFFERING_TONE,
        )
    };
    let change = matches!(proposed, Proposed::Goal(_)).then(|| {
        button("change-goal", "Change…", false).on_hint_click(cx.listener(
            move |view, _, window, cx| {
                view.change_proposal(index, window, cx);
            },
        ))
    });
    let buttons = [
        Some(
            button(set, set_label, true).on_hint_click(cx.listener(move |view, _, _, cx| {
                view.answer_proposal(index, Answer::Set, cx);
            })),
        ),
        change,
        Some(button(dismiss, "Dismiss", false).on_hint_click(cx.listener(
            move |view, _, _, cx| {
                view.answer_proposal(index, Answer::Dismissed, cx);
            },
        ))),
    ];
    answer_card(
        OFFERING_TONE,
        card_head(mark, OFFERING_TONE, head),
        tag,
        render_proposed(proposed, &goal, why.as_deref(), writer),
        buttons.into_iter().flatten(),
    )
    .debug_selector(selector)
    .into_any_element()
}

/// What the card of what was proposed says, as its kind does: its head,
/// the tag at its right, the button that carries it out, by its name and
/// label, and the name of the one that lets it be.
fn card_words(
    proposed: &Proposed,
) -> (
    &'static str,
    &'static str,
    (&'static str, &'static str),
    &'static str,
) {
    match proposed {
        Proposed::Goal(_) => (
            "Proposed goal",
            "for you to set",
            ("set-goal", "Set goal"),
            "dismiss-goal",
        ),
        Proposed::Name(_) => (
            "Proposed name",
            "for the project, and its folder",
            ("rename-project", "Rename"),
            "dismiss-name",
        ),
        Proposed::About(_) => (
            "About the project",
            "each chat is told it first",
            ("set-about", "Set"),
            "dismiss-about",
        ),
        Proposed::Icon { .. } => (
            "Proposed icon",
            "for the project, on the rail",
            ("set-icon", "Set"),
            "dismiss-icon",
        ),
        Proposed::Project { .. } => (
            "Proposed project",
            "this chat moves into it",
            ("make-project", "Make project"),
            "dismiss-project",
        ),
    }
}

/// What the card of what was proposed shows: the goal as `goal` reads
/// it, a name, what the project is, or the icon; then `why` the agent
/// proposes it, if it said, and, of a project, the sketch whose files go
/// into it, if any.
fn render_proposed(
    proposed: &Proposed,
    goal: &str,
    why: Option<&str>,
    writer: &mut Writer,
) -> impl Iterator<Item = AnyElement> {
    let named = |name: &str| {
        div()
            .text_size(px(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(SharedString::from(name.to_owned()))
            .into_any_element()
    };
    let shown = match proposed {
        Proposed::Goal(_) => writer.markdown(goal).into_any_element(),
        Proposed::Name(name) | Proposed::Project { name, .. } => named(name),
        Proposed::About(about) => div()
            .text_size(px(14.))
            .child(writer.markdown(about))
            .into_any_element(),
        Proposed::Icon { file, from } => div()
            .flex()
            .items_center()
            .gap_3()
            .children(icon_of(file, PROPOSED_ICON))
            .child(
                div()
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(rgb(DIM))
                    .child(ellipsis(SharedString::from(from.clone()))),
            )
            .into_any_element(),
    };
    let why = why.map(|why| {
        div()
            .text_size(px(12.))
            .text_color(rgb(DIM))
            .child(writer.markdown(why))
            .into_any_element()
    });
    let sketch = match proposed {
        Proposed::Project {
            sketch_name: Some(sketch),
            ..
        } => Some(
            div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(SharedString::from(format!(
                    "The files of the sketch \u{201c}{sketch}\u{201d} go into it, as its \
                     directory."
                )))
                .into_any_element(),
        ),
        _ => None,
    };
    [Some(shown), why, sketch].into_iter().flatten()
}

/// Cox's card asking the human whether to keep what a chat learned in
/// the wiki, at the milestone `why` says, with "Save to wiki" and "Not
/// now", or, once they chose, a line saying which.
fn render_keep(
    index: usize,
    why: &str,
    kept: Option<bool>,
    writer: &mut Writer,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let title = format!("{why} Keep it in the wiki?");
    if let Some(kept) = kept {
        let answer = if kept { "asked to save" } else { "not now" };
        return line_of(index, None, ("✎", LINK), &title, writer)
            .debug_selector(move || format!("keep-{index}"))
            .child(div().flex_shrink_0().text_color(rgb(DIM)).child(answer))
            .into_any_element();
    }
    let button = |name: &'static str, label: &'static str, first: bool| {
        let id = format!("{name}-{index}");
        answer_button(
            SharedString::from(id.clone()),
            id,
            label,
            first,
            OFFERING_TONE,
        )
    };
    let buttons = [
        button("keep-in-wiki", "Save to wiki", true).on_hint_click(cx.listener(
            move |view, _, _, cx| {
                view.answer_keep(index, true, cx);
            },
        )),
        button("keep-not-now", "Not now", false).on_hint_click(cx.listener(
            move |view, _, _, cx| {
                view.answer_keep(index, false, cx);
            },
        )),
    ];
    let shows = [div()
        .text_color(rgb(DIM))
        .child(writer.plain(
            "Keep what was learned in the wiki, for the agents of every project to find again?",
            "\n\n",
        ))
        .into_any_element()];
    answer_card(
        OFFERING_TONE,
        card_head("✎", OFFERING_TONE, writer.plain(why, "\n")),
        "keep it?",
        shows,
        buttons,
    )
    .debug_selector(move || format!("keep-{index}"))
    .into_any_element()
}

/// The permission request the agent waits on: what it asks to do, and
/// its options, to click, or to choose by their numbers.
fn render_asking(
    index: usize,
    (title, content, gathered): (&str, &[Content], &[Check]),
    options: &[String],
    writer: &mut Writer,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let options = options.iter().enumerate().map(|(i, name)| {
        answer_button(
            ("option", i),
            format!("option-{}", i + 1),
            div()
                .overflow_hidden()
                .child(ellipsis(SharedString::from(format!("{} · {name}", i + 1)))),
            i == 0,
            ASKING_TONE,
        )
        .on_hint_click(cx.listener(move |view, _, _, cx| view.choose(i, cx)))
    });
    let head = line_of(index, None, ("?", ASKING), title, writer);
    // What the triggers gathered is what the human reads first.
    let shows = render_gathered(index, gathered, writer)
        .into_iter()
        .chain(render_content(index, content, writer));
    let gathering = gathered
        .iter()
        .any(|check| check.state == CheckState::Running);
    let tag = if gathering {
        "gathering"
    } else {
        "waiting for you"
    };
    answer_card(ASKING_TONE, head, tag, shows, options).into_any_element()
}

/// A picture shown in the conversation, as big as fits, or a video, on
/// a line of its own, each with its caption, or where it was shown from,
/// under it: a click opens it on the stage, where a video plays.
/// How wide a picture of `size` is drawn, as big as fits in
/// `MEDIA_WIDTH` by `MEDIA_HEIGHT`, keeping its shape, so that its height
/// follows from its width with no empty band beside it; one of no known size
/// takes the whole width.
#[expect(
    clippy::cast_precision_loss,
    reason = "a picture is far smaller than f32 counts exactly"
)]
fn media_width(size: Option<(u32, u32)>) -> f32 {
    match size {
        Some((width, height)) if width > 0 && height > 0 => {
            let (width, height) = (width as f32, height as f32);
            width.min(MEDIA_HEIGHT * width / height).min(MEDIA_WIDTH)
        }
        _ => MEDIA_WIDTH,
    }
}

fn render_media(
    index: usize,
    (file, kind, from, caption): (&str, MediaKind, &str, Option<&str>),
    size: Option<(u32, u32)>,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let link = format!("cox://api/media/{file}");
    let shown = match (kind, crate::media::kept(file)) {
        (MediaKind::Image, Some((path, _))) => div()
            .max_w_full()
            .child(
                gpui::img(path)
                    .w(px(media_width(size)))
                    .max_w_full()
                    .object_fit(gpui::ObjectFit::Contain)
                    .rounded(px(8.)),
            )
            .into_any_element(),
        _ => div()
            .flex()
            .items_center()
            .gap_2()
            .px(px(12.))
            .py(px(10.))
            .rounded(px(8.))
            .bg(rgb(RAISED))
            .text_color(rgb(LINK))
            .child("▶")
            .child(
                div()
                    .min_w_0()
                    .child(ellipsis(SharedString::from(from.to_owned()))),
            )
            .into_any_element(),
    };
    let under = caption.unwrap_or(from).to_owned();
    div()
        .id(("media", index))
        .debug_selector(move || format!("media-{index}"))
        .flex()
        .flex_col()
        .gap_1()
        .cursor_pointer()
        .child(shown)
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(SharedString::from(under)),
        )
        .on_hint_click(cx.listener(move |view, _, window, cx| view.show_link(&link, window, cx)))
        .into_any_element()
}

/// How big a picture or video shown is drawn on the card the human closes
/// the work on, square.
const THUMBNAIL: f32 = 80.;

/// The picture or video `file` the agent showed, at `index`, small and
/// square, for the card the human closes the work on: a click opens it
/// on the stage, as the picture in the conversation does.
/// The row of the closing card that says how many files changed where
/// the chat works, which a click shows on the stage's "Changes".
fn render_changed(changed: usize, cx: &mut Context<ChatView>) -> AnyElement {
    let said = match changed {
        0 => "No file changed".to_owned(),
        1 => "1 file changed".to_owned(),
        many => format!("{many} files changed"),
    };
    div()
        .id("closing-changes")
        .debug_selector(|| "closing-changes".to_owned())
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py(px(6.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(RAISED))
        .cursor_pointer()
        .hover(|row| row.bg(rgb(HOVER)))
        .child(div().flex_1().child(said))
        .child(div().text_color(rgb(DIM)).child("Changes ›"))
        .on_hint_click(cx.listener(|view, _, window, cx| {
            if let Some(stage) = view.stage() {
                stage.update(cx, |stage, cx| stage.show_folder_changes(window, cx));
            }
        }))
        .into_any_element()
}

/// What the agent showed of the work at `index`, `entry`, as a tile of the
/// card the human closes the work on: a picture, a video or a file.
fn render_tile(index: usize, entry: &Entry, cx: &mut Context<ChatView>) -> Option<AnyElement> {
    match entry {
        Entry::Media {
            file, kind, from, ..
        } => Some(render_thumbnail(index, file, *kind, from, cx)),
        Entry::Shown {
            path,
            markdown,
            lines,
            ..
        } => Some(render_shown_tile(index, path, *markdown, *lines, cx)),
        _ => None,
    }
}

/// Where the file the agent showed at `path` opens, at `lines`: as
/// Markdown, or as code at the lines, as a path in the conversation does.
fn shown_link(path: &str, lines: Option<(usize, usize)>) -> String {
    match lines {
        Some((first, last)) if first == last => format!("{path}:{first}"),
        Some((first, last)) => format!("{path}:{first}-{last}"),
        None => path.to_owned(),
    }
}

/// What a file the agent showed is called, with its lines, if it named
/// them, as `main.rs:12-20`.
fn shown_name(path: &str, lines: Option<(usize, usize)>) -> String {
    let name = std::path::Path::new(path).file_name().map_or_else(
        || path.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    match lines {
        Some((first, last)) if first == last => format!("{name}:{first}"),
        Some((first, last)) => format!("{name}:{first}-{last}"),
        None => name,
    }
}

/// A file the agent showed the human on the stage, as a row of the
/// conversation, which a click opens there again.
fn render_shown(
    index: usize,
    path: &str,
    markdown: bool,
    lines: Option<(usize, usize)>,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let link = shown_link(path, lines);
    let at = lines.map(|_| shown_name(path, lines));
    div()
        .id(("shown", index))
        .debug_selector(move || format!("shown-{index}"))
        .flex()
        .items_center()
        .gap_2()
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .bg(rgb(RAISED))
        .cursor_pointer()
        .hover(|row| row.bg(rgb(HOVER)))
        .child(
            div()
                .text_color(rgb(LINK))
                .child(if markdown { "≡" } else { "‹›" }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(ellipsis(SharedString::from(path.to_owned()))),
        )
        .children(at.map(|at| {
            div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(SharedString::from(at))
        }))
        .on_hint_click(cx.listener(move |view, _, window, cx| view.show_link(&link, window, cx)))
        .into_any_element()
}

/// A file the agent showed, as a tile of the card the human closes the
/// work on, beside the pictures, which a click opens on the stage.
fn render_shown_tile(
    index: usize,
    path: &str,
    markdown: bool,
    lines: Option<(usize, usize)>,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let link = shown_link(path, lines);
    let name = shown_name(path, lines);
    div()
        .id(("shown-tile", index))
        .debug_selector(move || format!("shown-tile-{index}"))
        .size(px(THUMBNAIL))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .p_1()
        .rounded(px(8.))
        .overflow_hidden()
        .bg(rgb(RAISED))
        .cursor_pointer()
        .child(
            div()
                .text_size(px(18.))
                .text_color(rgb(LINK))
                .child(if markdown { "≡" } else { "‹›" }),
        )
        .child(
            div()
                .w_full()
                .text_size(px(11.))
                .text_color(rgb(DIM))
                .text_center()
                .child(ellipsis(SharedString::from(name))),
        )
        .tooltip(crate::ui::tip(path.to_owned()))
        .on_hint_click(cx.listener(move |view, _, window, cx| view.show_link(&link, window, cx)))
        .into_any_element()
}

fn render_thumbnail(
    index: usize,
    file: &str,
    kind: MediaKind,
    from: &str,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let link = format!("cox://api/media/{file}");
    let shown = match (kind, crate::media::kept(file)) {
        (MediaKind::Image, Some((path, _))) => gpui::img(path)
            .size(px(THUMBNAIL))
            .object_fit(gpui::ObjectFit::Cover)
            .into_any_element(),
        _ => div().text_color(rgb(LINK)).child("▶").into_any_element(),
    };
    div()
        .id(("thumbnail", index))
        .debug_selector(move || format!("thumbnail-{index}"))
        .size(px(THUMBNAIL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.))
        .overflow_hidden()
        .bg(rgb(RAISED))
        .cursor_pointer()
        .child(shown)
        .tooltip(crate::ui::tip(from.to_owned()))
        .on_hint_click(cx.listener(move |view, _, window, cx| view.show_link(&link, window, cx)))
        .into_any_element()
}

/// What the triggers that run as the agent asks the human's permission
/// gathered for the human to answer with, each by its name, with a mark of
/// how it went, and what it said: its headline first, which the human
/// reads first, as on the phone. One that gathers yet says so, dimmer.
fn render_gathered(index: usize, gathered: &[Check], writer: &mut Writer) -> Option<AnyElement> {
    if gathered.is_empty() {
        return None;
    }
    let each = gathered.iter().map(|check| {
        let (dot, filled) = match check.state {
            CheckState::Waiting => (DISABLED, false),
            CheckState::Running => (LINK, false),
            CheckState::Passed => (ACCENT, true),
            CheckState::Failed => (ASKING, true),
            CheckState::Broken => (FAILED_MARK, true),
        };
        let name = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.))
            .font_family(CODE_FONT)
            .text_color(rgb(DIM))
            .child(
                div()
                    .flex_shrink_0()
                    .size(px(7.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(dot))
                    .when(filled, |mark| mark.bg(rgb(dot))),
            )
            .child(SharedString::from(check.name.clone()));
        let said = match (check.state, check.said.as_deref()) {
            (CheckState::Running, _) => div()
                .text_size(px(13.))
                .text_color(rgb(FAINT))
                .child("Gathering what you need to answer…"),
            (_, Some(said)) => {
                let headline = conversation::headline(said).unwrap_or_default();
                let rest = said
                    .trim()
                    .split_once('\n')
                    .map_or("", |(_, rest)| rest.trim());
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_size(px(13.))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(writer.plain(headline, "\n")),
                    )
                    .when(!rest.is_empty(), |said| {
                        said.child(div().text_color(rgb(DIM)).child(writer.markdown(rest)))
                    })
            }
            (_, None) => div()
                .text_size(px(13.))
                .text_color(rgb(FAINT))
                .child("It said nothing."),
        };
        div().flex().flex_col().gap_1().child(name).child(said)
    });
    Some(
        div()
            .debug_selector(move || format!("gathered-{index}"))
            .flex()
            .flex_col()
            .gap(px(10.))
            .children(each)
            .into_any_element(),
    )
}

/// The line of a tool call or a permission request: whether what it
/// shows is open, if it shows something, its mark, in its color, and its
/// title, as Markdown.
fn line_of(
    index: usize,
    open: Option<bool>,
    (mark, color): (&'static str, u32),
    title: &str,
    writer: &mut Writer,
) -> Stateful<gpui::Div> {
    div()
        .id(("line", index))
        .debug_selector(move || format!("line-{index}"))
        .flex()
        .gap_2()
        .child(disclosure(open))
        .child(div().flex_shrink_0().text_color(rgb(color)).child(mark))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(rgb(DIM))
                .child(writer.markdown(title)),
        )
}

/// A small "flow" at the end of the line of the tool call `id`, which
/// opens the flow of the call through Cox on the stage (see
/// [`crate::stage::flow`]).
fn flow_button(index: usize, id: &str, cx: &mut Context<ChatView>) -> impl IntoElement {
    let link = crate::stage::flow::link(id);
    div()
        .id(("flow", index))
        .debug_selector(move || format!("flow-{index}"))
        .flex_shrink_0()
        .px_1()
        .rounded_sm()
        .text_xs()
        .text_color(rgb(DIM))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(BUTTON_HOVER)).text_color(rgb(LINK)))
        .child("flow")
        .on_hint_click(cx.listener(move |view, _, window, cx| {
            // Not a click on the line, which opens what the call shows.
            cx.stop_propagation();
            view.show_link(&link, window, cx);
        }))
}

/// `line`, for a click to open or close `fold`, if there is something to
/// open.
fn folding(
    line: Stateful<gpui::Div>,
    fold: Option<Fold>,
    cx: &mut Context<ChatView>,
) -> Stateful<gpui::Div> {
    let Some(fold) = fold else {
        return line;
    };
    line.cursor_pointer()
        .hover(|style| style.bg(rgb(BUTTON)))
        .on_hint_click(cx.listener(move |view, _, _, cx| {
            view.folds.flip(fold);
            cx.notify();
        }))
}

/// The title of a tool call, short, as its chip shows it: as text, not
/// Markdown, and each path by its last part, as `Read src/main.rs` is
/// `Read main.rs`. A word with more than a path in it, as a command's
/// `PATH="$HOME/.cargo/bin:$PATH"`, stays whole, since its last part
/// alone would read as something else.
fn short(title: &str) -> String {
    title
        .split_whitespace()
        .map(|word| {
            let word = word.trim_matches('`');
            let path = word
                .chars()
                .all(|c| c.is_alphanumeric() || "/._-~@+".contains(c));
            match word.rsplit_once('/') {
                Some((_, last)) if path && !last.is_empty() => last,
                _ => word,
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `count` of what `one` names, as "1 thought" or "3 thoughts".
fn counted(count: usize, one: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {one}s")
    }
}

/// How far a plan has gone, as how many of its steps are done of all.
pub(super) fn progress(steps: &[Step]) -> String {
    let done = steps
        .iter()
        .filter(|step| step.state == StepState::Completed)
        .count();
    format!("{done}/{}", steps.len())
}

/// The steps of a plan, each under a mark of how far it has gone.
pub(super) fn render_steps(steps: &[Step]) -> gpui::Div {
    div().flex().flex_col().children(steps.iter().map(|step| {
        let (mark, color, text) = match step.state {
            StepState::Completed => ("✓", ACCENT, DIM),
            StepState::InProgress => ("●", LINK, FOREGROUND),
            StepState::Pending => ("○", DIM, FOREGROUND),
        };
        div()
            .flex()
            .gap_2()
            .child(div().flex_shrink_0().text_color(rgb(color)).child(mark))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(rgb(text))
                    .child(SharedString::from(step.text.clone())),
            )
    }))
}

/// The mark of whether a part is open, `None` for one with nothing to
/// open, which keeps the place of the mark.
pub(super) fn disclosure(open: Option<bool>) -> gpui::Div {
    let mark = match open {
        Some(true) => "▾",
        Some(false) => "▸",
        None => "",
    };
    div()
        .flex_shrink_0()
        .w(px(12.))
        .text_color(rgb(DIM))
        .child(mark)
}

/// What a tool call or a permission request shows, under its title, or
/// `None` when it shows nothing: text as Markdown, as the agent sends
/// what a command printed in a code block. It scrolls on its own once it
/// is tall, such as a long output or a change to a large file.
fn render_content(
    index: usize,
    content: &[Content],
    writer: &mut Writer,
) -> Option<gpui::AnyElement> {
    if content.is_empty() {
        return None;
    }
    let shown: Vec<_> = content
        .iter()
        .map(|content| match content {
            Content::Text(text) => writer.markdown(text),
            Content::Diff { path, lines } => render_diff(path, lines, writer),
        })
        .collect();
    let content = div()
        .id(("content", index))
        .debug_selector(move || format!("content-{index}"))
        .max_h(px(CONTENT_HEIGHT))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_1()
        .pl_2()
        .border_l_1()
        .border_color(rgb(BORDER))
        .text_size(px(12.))
        .children(shown);
    Some(content.into_any_element())
}

/// A change to the file at `path`, as the Changes tab draws one: a card
/// headed by the path and the lines it gains and loses, over its lines.
fn render_diff(path: &str, lines: &[Line], writer: &mut Writer) -> gpui::Div {
    let added = lines
        .iter()
        .filter(|line| matches!(line, Line::Added(_)))
        .count();
    let removed = lines
        .iter()
        .filter(|line| matches!(line, Line::Removed(_)))
        .count();
    div()
        .flex()
        .flex_col()
        .border_1()
        .border_color(rgb(BORDER))
        .rounded(px(8.))
        .overflow_hidden()
        .font_family(CODE_FONT)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_2()
                .py_1()
                .bg(rgb(RAISED))
                .border_b_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(rgb(DIM))
                        .child(writer.plain(path, "\n")),
                )
                .child(crate::stage::changes::counts(added, removed)),
        )
        .children(lines.iter().map(|line| render_line(line, writer)))
}

/// One line of a change, on the ground of what the change did to it, its
/// mark in a column of its own, so that a line too long for the
/// conversation wraps clear of it, and copies without it.
fn render_line(line: &Line, writer: &mut Writer) -> gpui::Div {
    let (mark, ground, sign, text) = match line {
        Line::Same(text) => (None, None, "", text.clone()),
        Line::Removed(text) => (
            Some((FAILED_MARK, FAILED)),
            Some(REMOVED_GROUND),
            "−",
            text.clone(),
        ),
        Line::Added(text) => (
            Some((ACCENT, ACCENT)),
            Some(ADDED_GROUND),
            "+",
            text.clone(),
        ),
        Line::Skipped(1) => (None, None, "", "⋯ 1 line".to_owned()),
        Line::Skipped(count) => (None, None, "", format!("⋯ {count} lines")),
    };
    let skipped = matches!(line, Line::Skipped(_));
    div()
        .flex()
        .when_some(ground, |row, ground| row.bg(rgb(ground)))
        .child(
            div()
                .flex_shrink_0()
                .w(px(2.))
                .when_some(mark, |bar, (mark, _)| bar.bg(rgb(mark))),
        )
        .child(
            div()
                .flex_shrink_0()
                .w(px(18.))
                .flex()
                .justify_center()
                .when_some(mark, |sign, (_, color)| sign.text_color(rgb(color)))
                .child(sign),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pr_2()
                .text_color(rgb(if skipped { DIM } else { FOREGROUND }))
                .child(writer.plain(&text, "\n")),
        )
}

/// A chip of what the human handed the chat, named `id`: where it is
/// from, and, where the mouse rests, the start of what it says.
pub(super) fn attachment_chip(id: gpui::ElementId, attachment: &Attachment) -> Stateful<gpui::Div> {
    let shown: String = attachment
        .text
        .lines()
        .take(12)
        .collect::<Vec<_>>()
        .join("\n");
    let uri = attachment.uri.clone();
    div()
        .id(id)
        .h(px(24.))
        .max_w(px(260.))
        .pl(px(9.))
        .pr(px(5.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(12.))
        .bg(rgb(BUTTON))
        .text_size(px(12.))
        .text_color(rgb(FAINT))
        .when(opens(&uri), |chip| {
            chip.cursor_pointer()
                .hover(|style| style.text_color(rgb(FOREGROUND)))
        })
        .tooltip(crate::ui::tip(format!("{}\n\n{shown}", attachment.uri)))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(attachment.label.clone()),
        )
}

/// Whether a click on a chip of what the human handed opens where it is
/// from: a file or a page does, a plugin's too, and the conversation, a
/// terminal or the settings, which no address shows, do not.
pub(super) fn opens(uri: &str) -> bool {
    !["cox://chat/", "cox://terminal", "cox://settings"]
        .iter()
        .any(|shown| uri.starts_with(shown))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str) -> Entry {
        Entry::ToolCall {
            id: id.to_owned(),
            title: "ls".to_owned(),
            content: Vec::new(),
            result: None,
        }
    }

    #[test]
    fn folds_tool_calls_in_a_row() {
        let entries = [
            Entry::Human("look".to_owned()),
            call("call-1"),
            Entry::Reply("one".to_owned()),
            call("call-2"),
            call("call-3"),
            call("call-4"),
            Entry::Reply("three".to_owned()),
            call("call-5"),
            call("call-6"),
        ];

        assert_eq!(
            shown(&entries),
            [
                Shown::Entry(0),
                Shown::Work(1..2),
                Shown::Entry(2),
                Shown::Work(3..6),
                Shown::Entry(6),
                Shown::Work(7..9),
            ]
        );
    }

    #[test]
    fn folds_thoughts_and_plans_with_the_tool_calls() {
        let entries = [
            Entry::Human("look".to_owned()),
            Entry::Thought("Look where?".to_owned()),
            Entry::Reply("Here.".to_owned()),
            Entry::Plan(Vec::new()),
            call("call-1"),
            Entry::Thought("Found it.".to_owned()),
            Entry::Reply("Done.".to_owned()),
        ];

        assert_eq!(
            shown(&entries),
            [
                Shown::Entry(0),
                Shown::Entry(1),
                Shown::Entry(2),
                Shown::Work(3..6),
                Shown::Entry(6),
            ]
        );
    }

    fn shown_media(file: &str) -> Entry {
        Entry::Media {
            file: file.to_owned(),
            kind: MediaKind::Image,
            from: "shot.png".to_owned(),
            caption: None,
            size: None,
        }
    }

    fn passed(name: &str) -> Check {
        Check {
            name: name.to_owned(),
            state: CheckState::Passed,
            said: None,
        }
    }

    #[test]
    fn closes_the_work_of_the_last_line_on_what_it_showed_decided_and_checked() {
        let added = vec!["Fall back to there".to_owned()];
        let entries = [
            Entry::Human("first".to_owned()),
            shown_media("old"),
            Entry::Human("fix the greeting".to_owned()),
            call("call-1"),
            shown_media("new"),
            Entry::Shown {
                file: "0123.md".to_owned(),
                path: "docs/report.md".to_owned(),
                markdown: true,
                lines: None,
            },
            Entry::PlanChanged {
                added: added.clone(),
                removed: Vec::new(),
            },
            Entry::Checks(vec![passed("goal")]),
            Entry::Event {
                on: "goal.met".to_owned(),
                checks: vec![passed("smoke")],
            },
        ];

        let closing = closing(&entries).expect("the work closes");
        assert_eq!(closing.shown, [4, 5]);
        assert_eq!(closing.decided, Some((added.as_slice(), &[][..])));
        assert_eq!(
            closing
                .checks
                .iter()
                .map(|check| check.name.as_str())
                .collect::<Vec<_>>(),
            ["goal", "smoke"]
        );
    }

    #[test]
    fn closes_nothing_before_a_line_after_a_merge_or_with_nothing_to_close_on() {
        assert_eq!(closing(&[shown_media("before")]), None);
        assert_eq!(
            closing(&[
                Entry::Human("fix it".to_owned()),
                shown_media("shot"),
                Entry::Merge,
                Entry::Cox("Merge it.".to_owned()),
            ]),
            None
        );
        assert_eq!(
            closing(&[
                Entry::Human("what is this?".to_owned()),
                Entry::Reply("A greeter.".to_owned()),
                Entry::PlanChanged {
                    added: Vec::new(),
                    removed: Vec::new(),
                },
            ]),
            None
        );
    }

    #[test]
    fn a_step_names_each_path_by_its_last_part() {
        assert_eq!(short("Read src/main.rs"), "Read main.rs");
        assert_eq!(short("`cargo test`"), "cargo test");
        assert_eq!(short("Edit `/home/me/app/src/a.rs`"), "Edit a.rs");
        assert_eq!(short("grep greeting src/"), "grep greeting src/");
        assert_eq!(
            short(r#"PATH="$HOME/.cargo/bin:$PATH" cargo test -p cox"#),
            r#"PATH="$HOME/.cargo/bin:$PATH" cargo test -p cox"#
        );
    }

    #[test]
    fn a_fold_opens_and_closes_from_how_it_shows_at_first() {
        let mut folds = Folds::default();
        let fold = Fold::Content(3);
        assert!(!folds.is_open(fold, false));
        assert!(folds.is_open(fold, true));

        folds.flip(fold);
        assert!(folds.is_open(fold, false));
        assert!(!folds.is_open(fold, true));
        assert!(!folds.is_open(Fold::Work(3), false));

        folds.flip(fold);
        assert!(!folds.is_open(fold, false));
    }
}
