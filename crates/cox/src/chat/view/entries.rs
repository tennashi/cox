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

use cox_core::goal::Goal;

use crate::chat::conversation::{
    self, Answer, Content, Entry, LoginOption, State, Step, StepState, ToolCallResult,
};
use crate::chat::view::ChatView;
use crate::ui::ellipsis::ellipsis;
use crate::ui::text::Writer;
use crate::ui::theme::{
    ACCENT, ASKING, ASKING_BUTTON, ASKING_EDGE, ASKING_GROUND, BORDER, BORDER_STRONG, BUTTON,
    BUTTON_HOVER, CODE_FONT, DIM, DISABLED, FAILED, FAILED_GROUND, FAILED_MARK, FAINT, FOREGROUND,
    GROUND, LINK, RAISED, SURFACE,
};

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
}

/// Who says an entry, as the timeline of the conversation names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speaker {
    Human,
    /// Cox, in the human's place.
    Cox,
    Agent,
}

impl Speaker {
    /// Who says `entry`: none for Cox's notes, which go with whoever
    /// spoke last.
    fn of(entry: &Entry) -> Option<Self> {
        match entry {
            Entry::Human(_) => Some(Self::Human),
            Entry::Cox(_) | Entry::Keep { .. } => Some(Self::Cox),
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
            Shown::Entry(index) => *index,
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

/// A part of the conversation that the human opens and closes: what the
/// entry at an index shows under its line, the work folded from the
/// entry at an index on, or the line of the tool call at an index under
/// the steps of its fold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Fold {
    Content(usize),
    Work(usize),
    Step(usize),
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
                Shown::Entry(index) => *index,
                Shown::Work(work) => work.start,
            };
            let entry = &entries[first];
            if let Entry::Switched { to, .. } = entry {
                agents.switch(to);
            }
            let speaker = Speaker::of(entry);
            // A speaker is named where they start, and what asks the
            // human is marked wherever it is.
            let asking = matches!(entry, Entry::Permission { chosen: None, .. })
                && self.conversation.state() == State::Asking
                && first + 1 == entries.len();
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
                    Speaker::Agent => Mark::Named(agents.now().to_uppercase().into(), ACCENT),
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
        places
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
        };
        // The list lays each place out on its own, so each keeps the
        // conversation's margins itself.
        div()
            .px(px(14.))
            .when(place == 0, |first| first.pt(px(16.)))
            .child(on_timeline(place == 0, mark, element))
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
            .on_click(cx.listener(move |view, _, _, cx| {
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
            .on_click(cx.listener(move |view, _, _, cx| {
                view.folds.flip(fold);
                cx.notify();
            }))
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
        let asking = self.conversation.state() == State::Asking
            && chosen.is_none()
            && index + 1 == self.conversation.entries().len();
        if asking {
            let title = conversation::login_title(failed);
            let names: Vec<String> = options.iter().map(conversation::login_label).collect();
            return render_asking(index, &title, &[], &names, writer, cx);
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

    fn render_entry(&self, index: usize, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let mut writer = self.reading.writer(index);
        match entry {
            Entry::Human(text) => div()
                .text_size(px(15.))
                .child(writer.plain(text, "\n\n"))
                .into_any_element(),
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
            Entry::Proposal { goal, why, answer } => {
                render_proposal(index, goal, why.as_deref(), *answer, &mut writer, cx)
            }
            Entry::Keep { why, kept, .. } => render_keep(index, why, *kept, &mut writer, cx),
            Entry::Note(text) => div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.plain(text, "\n\n"))
                .into_any_element(),
            Entry::Switched { from, to } => div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.plain(&conversation::switched_note(from, to), "\n\n"))
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
                ..
            } => {
                let asking = self.conversation.state() == State::Asking
                    && chosen.is_none()
                    && index + 1 == self.conversation.entries().len();
                if asking {
                    render_asking(index, title, content, options, &mut writer, cx)
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
                    self.render_answered(index, title, content, answer, &mut writer, cx)
                }
            }
        }
    }

    /// A permission request answered with `answer`, on a line that opens
    /// to what was asked, if the request showed something.
    fn render_answered(
        &self,
        index: usize,
        title: &str,
        content: &[Content],
        answer: String,
        writer: &mut Writer,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // What was asked was read when it was answered.
        let fold = Fold::Content(index);
        let open = self.folds.is_open(fold, false);
        let opens = (!content.is_empty()).then_some(open);
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
                asked.children(render_content(index, content, writer))
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
                    .on_click(cx.listener(move |view, _, _, cx| {
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

/// A goal the agent proposed: while the human has not answered it, a
/// card with why the agent proposes it, and buttons to set it, to
/// change it first, or to let it be; then a line that says what they
/// did.
fn render_proposal(
    index: usize,
    goal: &str,
    why: Option<&str>,
    answer: Option<Answer>,
    writer: &mut Writer,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    let read = Goal::parse(goal).map_or_else(|| goal.to_owned(), |goal| conversation::line(&goal));
    let title = format!("Proposed goal: {read}");
    if let Some(answer) = answer {
        let answer = match answer {
            Answer::Set => "set",
            Answer::Changed => "changed, and set",
            Answer::Dismissed => "let be",
        };
        return line_of(index, None, ("◎", LINK), &title, writer)
            .debug_selector(move || format!("proposal-{index}"))
            .child(div().flex_shrink_0().text_color(rgb(DIM)).child(answer))
            .into_any_element();
    }
    let button = |name: &'static str, label: &'static str| {
        div()
            .id(SharedString::from(format!("{name}-{index}")))
            .debug_selector(move || format!("{name}-{index}"))
            .px_2()
            .rounded_sm()
            .bg(rgb(BUTTON))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON_HOVER)))
            .child(label)
    };
    let buttons = [
        button("set-goal", "Set goal").on_click(cx.listener(move |view, _, _, cx| {
            view.answer_proposal(index, Answer::Set, cx);
        })),
        button("change-goal", "Change…").on_click(cx.listener(move |view, _, window, cx| {
            view.change_proposal(index, window, cx);
        })),
        button("dismiss-goal", "Dismiss").on_click(cx.listener(move |view, _, _, cx| {
            view.answer_proposal(index, Answer::Dismissed, cx);
        })),
    ];
    div()
        .debug_selector(move || format!("proposal-{index}"))
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .rounded_sm()
        .border_1()
        .border_color(rgb(LINK))
        .child(
            div()
                .flex()
                .gap_2()
                .text_color(rgb(DIM))
                .child(div().flex_shrink_0().text_color(rgb(LINK)).child("◎"))
                .child("Proposed goal"),
        )
        .child(writer.markdown(&read))
        .children(why.map(|why| {
            div()
                .text_size(px(12.))
                .text_color(rgb(DIM))
                .child(writer.markdown(why))
        }))
        .child(div().flex().flex_wrap().gap_2().children(buttons))
        .into_any_element()
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
    let button = |name: &'static str, label: &'static str| {
        div()
            .id(SharedString::from(format!("{name}-{index}")))
            .debug_selector(move || format!("{name}-{index}"))
            .px_2()
            .rounded_sm()
            .bg(rgb(BUTTON))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON_HOVER)))
            .child(label)
    };
    let buttons = [
        button("keep-in-wiki", "Save to wiki").on_click(cx.listener(move |view, _, _, cx| {
            view.answer_keep(index, true, cx);
        })),
        button("keep-not-now", "Not now").on_click(cx.listener(move |view, _, _, cx| {
            view.answer_keep(index, false, cx);
        })),
    ];
    div()
        .debug_selector(move || format!("keep-{index}"))
        .flex()
        .flex_col()
        .gap_2()
        .p_2()
        .rounded_sm()
        .border_1()
        .border_color(rgb(LINK))
        .child(
            div()
                .flex()
                .gap_2()
                .text_color(rgb(DIM))
                .child(div().flex_shrink_0().text_color(rgb(LINK)).child("✎"))
                .child(writer.plain(why, "\n")),
        )
        .child(writer.plain(
            "Keep what was learned in the wiki, for the agents of every project to find again?",
            "\n\n",
        ))
        .child(div().flex().flex_wrap().gap_2().children(buttons))
        .into_any_element()
}

/// The permission request the agent waits on: what it asks to do, and
/// its options, to click, or to choose by their numbers.
fn render_asking(
    index: usize,
    title: &str,
    content: &[Content],
    options: &[String],
    writer: &mut Writer,
    cx: &mut Context<ChatView>,
) -> AnyElement {
    // The first option, which the human most likely answers, stands out.
    let options = options.iter().enumerate().map(|(i, name)| {
        let first = i == 0;
        div()
            .id(("option", i))
            .debug_selector(|| format!("option-{}", i + 1))
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
                        .bg(rgb(ASKING))
                        .text_color(rgb(GROUND))
                        .font_weight(FontWeight::SEMIBOLD)
                        .hover(|style| style.opacity(0.9))
                } else {
                    button
                        .bg(rgb(ASKING_BUTTON))
                        .text_color(rgb(FOREGROUND))
                        .hover(|style| style.bg(rgb(ASKING_EDGE)))
                }
            })
            .child(
                div()
                    .overflow_hidden()
                    .child(ellipsis(SharedString::from(format!("{} · {name}", i + 1)))),
            )
            .on_click(cx.listener(move |view, _, _, cx| view.choose(i, cx)))
    });
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(14.))
        .rounded(px(14.))
        .bg(rgb(ASKING_GROUND))
        .border_1()
        .border_color(rgb(ASKING_EDGE))
        .child(
            div()
                .flex()
                .items_start()
                .gap_2()
                .child(div().flex_1().min_w_0().child(line_of(
                    index,
                    None,
                    ("?", ASKING),
                    title,
                    writer,
                )))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(ASKING))
                        .child("waiting for you"),
                ),
        )
        .children(render_content(index, content, writer))
        .child(div().flex().gap(px(6.)).children(options))
        .into_any_element()
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
        .on_click(cx.listener(move |view, _, window, cx| {
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
        .on_click(cx.listener(move |view, _, _, cx| {
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
            Content::Diff { path, lines } => div()
                .flex()
                .flex_col()
                .font_family(CODE_FONT)
                .child(div().text_color(rgb(DIM)).child(writer.plain(path, "\n")))
                .children(lines.iter().map(|line| render_line(line, writer))),
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

/// One line of a change, its mark in a column of its own, so that a line
/// too long for the conversation wraps clear of it, and copies without it.
fn render_line(line: &Line, writer: &mut Writer) -> gpui::Div {
    let (color, mark, text) = match line {
        Line::Same(text) => (DIM, "", text.clone()),
        Line::Removed(text) => (FAILED, "-", text.clone()),
        Line::Added(text) => (ACCENT, "+", text.clone()),
        Line::Skipped(1) => (DIM, "", "⋯ 1 line".to_owned()),
        Line::Skipped(count) => (DIM, "", format!("⋯ {count} lines")),
    };
    div()
        .flex()
        .text_color(rgb(color))
        .child(div().flex_shrink_0().w(px(14.)).child(mark))
        .child(div().flex_1().min_w_0().child(writer.plain(&text, "\n")))
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
