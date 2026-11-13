//! The line at the window's foot, in command mode: Vim's `:` opens it to
//! run one of Cox's commands (see [`super::ex`]), with a list over it of
//! what the line can go on to, and `/` and `?` to find text in the part
//! that had the keyboard (see [`super::search`]). Tab and Shift-Tab, or
//! Control-N and Control-P, go along the list, Up and Down through the
//! lines typed before, Enter runs the line and Escape closes it.
//!
//! Closed, the line shows the mode and what is held toward a command, as
//! Vim's `showmode` and `showcmd` do, while Vim's keys are on.
//!
//! What hosts the line, the window, opens it as [`OpenLine`] asks, and
//! carries out the commands of the table as [`Run`] asks.

use gpui::{
    Action, AnyElement, App, Context, Entity, FocusHandle, Focusable as _, Subscription, Window,
    actions, div, prelude::*, px, rgb, rgba,
};

use super::ex::{self, Does, Ex, Fit, Names};
use super::keymap::Prompt;
use super::search::{Direction, Find, Search};
use super::{Leave, Mode, Status};
use crate::ui::input::{Blurred, LineInput};
use crate::ui::menu;
use crate::ui::theme::{
    ACCENT, ASKING, BORDER, BORDER_STRONG, FAILED, FAINT, FOREGROUND, GAP, LINK, SELECTED, SURFACE,
};

pub const KEY_CONTEXT: &str = "CommandLine";

/// How many rows of the list over the line show at once.
const ROWS: usize = 10;
const LINE_HEIGHT: f32 = 30.;
/// How tall the pill of the mode is.
const PILL_HEIGHT: f32 = 20.;
/// How wide the list over the line is.
const FITS_WIDTH: f32 = 480.;

/// Opens the line, for what `prompt` says.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct OpenLine {
    pub prompt: Prompt,
}

/// Carries out a command of the table, with what was typed after its
/// name.
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = vim, no_json)]
pub struct Run {
    pub does: Does,
    pub args: String,
}

actions!(vim, [Accept, NextFit, PreviousFit, Older, Newer]);

/// The line, while it is open.
struct Opened {
    prompt: Prompt,
    /// What had the keyboard before, which gets it back.
    back_to: Option<FocusHandle>,
    names: Names,
    /// What the human typed, which the list is of, and which the lines
    /// typed before that Up finds start with.
    typed: String,
    /// The place in the list of what Tab put in the line.
    chosen: Option<usize>,
    /// The place among the lines typed before of the one Up put in it.
    browsing: Option<usize>,
}

pub struct Line {
    input: Entity<LineInput>,
    opened: Option<Opened>,
    /// The commands typed before, the latest last.
    commands: Vec<String>,
    /// The text searched for before, the latest last.
    searches: Vec<String>,
    /// What the line put in the input, to tell from the human's typing.
    put: Option<String>,
    /// What the line says once closed, as of a command it does not know,
    /// until it opens again.
    said: Option<String>,
    _input: [Subscription; 2],
}

impl Line {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(LineInput::new);
        let edits = cx.observe(&input, |line, input, cx| {
            let text = input.read(cx).text();
            if line.put.as_deref() == Some(text) {
                return;
            }
            line.put = None;
            if let Some(opened) = &mut line.opened {
                text.clone_into(&mut opened.typed);
                opened.chosen = None;
                opened.browsing = None;
            }
            cx.notify();
        });
        let left = cx.subscribe_in(&input, window, |line, _, Blurred, window, cx| {
            line.close(window, cx);
        });
        Self {
            input,
            opened: None,
            commands: Vec::new(),
            searches: Vec::new(),
            put: None,
            said: None,
            _input: [edits, left],
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.opened.is_some()
    }

    /// What is typed in the line.
    #[must_use]
    pub fn text<'a>(&self, cx: &'a App) -> &'a str {
        self.input.read(cx).text()
    }

    /// What the line said last, until it opens again.
    #[must_use]
    pub fn said(&self) -> Option<&str> {
        self.said.as_deref()
    }

    /// What the line can go on to, the best first, while it is open.
    #[must_use]
    pub fn fits(&self) -> Vec<Fit> {
        match &self.opened {
            Some(Opened {
                prompt: Prompt::Ex,
                typed,
                names,
                ..
            }) => ex::fits(typed, names),
            _ => Vec::new(),
        }
    }

    /// Opens the line for `prompt`, with the keyboard, to go on to `names`.
    pub fn open(
        &mut self,
        prompt: Prompt,
        names: Names,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let back_to = match self.opened.take() {
            Some(opened) => opened.back_to,
            None => window.focused(cx),
        };
        self.said = None;
        self.opened = Some(Opened {
            prompt,
            back_to,
            names,
            typed: String::new(),
            chosen: None,
            browsing: None,
        });
        self.put(String::new(), cx);
        window.focus(&self.input.focus_handle(cx));
        super::enter(Mode::Command, cx);
        cx.notify();
    }

    /// Says `said` at the window's foot, once the line closes, until it
    /// opens again.
    pub fn say(&mut self, said: impl Into<String>, cx: &mut Context<Self>) {
        self.said = Some(said.into());
        cx.notify();
    }

    /// Closes the line, giving the keyboard back to what had it, and
    /// answers what it was open for.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Opened> {
        let opened = self.opened.take()?;
        if let Some(back_to) = &opened.back_to {
            window.focus(back_to);
        }
        super::enter(Mode::Normal, cx);
        window.refresh();
        cx.notify();
        Some(opened)
    }

    /// Puts `text` in the input, as the human did not type it.
    fn put(&mut self, text: String, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_text(&text, cx));
        self.put = Some(text);
    }

    fn history(&mut self, prompt: Prompt) -> &mut Vec<String> {
        match prompt {
            Prompt::Ex => &mut self.commands,
            Prompt::Search(_) => &mut self.searches,
        }
    }

    fn leave(&mut self, _: &Leave, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }

    /// Runs the line: finds its text, or carries out its command.
    fn accept(&mut self, _: &Accept, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text(cx).trim().to_owned();
        let Some(opened) = self.close(window, cx) else {
            return;
        };
        if !text.is_empty() {
            let history = self.history(opened.prompt);
            history.retain(|typed| *typed != text);
            history.push(text.clone());
        }
        match opened.prompt {
            Prompt::Search(direction) => Self::search(text, direction, window, cx),
            Prompt::Ex => self.run(&text, opened.names, window, cx),
        }
    }

    /// Finds `text` as `direction` goes, or the last text searched for
    /// when it is empty, as Vim does.
    fn search(text: String, direction: Direction, window: &mut Window, cx: &mut App) {
        let (text, word) = if text.is_empty() {
            match super::last_search(cx) {
                Some(last) => (last.text.clone(), last.word),
                None => return,
            }
        } else {
            (text, false)
        };
        super::searched(
            Search {
                text: text.clone(),
                direction,
                word,
            },
            cx,
        );
        window.dispatch_action(
            Box::new(Find {
                text,
                direction,
                word,
            }),
            cx,
        );
    }

    /// Carries out the command `line` names: one of the table, or else a
    /// GPUI action by its name.
    fn run(&mut self, line: &str, names: Names, window: &mut Window, cx: &mut Context<Self>) {
        let Some(typed) = Ex::parse(line) else {
            return;
        };
        match ex::command(typed.name) {
            Some(entry) if entry.does == Does::Help => self.open(Prompt::Ex, names, window, cx),
            Some(entry) => window.dispatch_action(
                Box::new(Run {
                    does: entry.does,
                    args: typed.args.to_owned(),
                }),
                cx,
            ),
            None => match cx.build_action(typed.name, None) {
                Ok(action) => window.dispatch_action(action, cx),
                Err(_) => self.say(format!("Not a command: {}", typed.name), cx),
            },
        }
    }

    fn next_fit(&mut self, _: &NextFit, _: &mut Window, cx: &mut Context<Self>) {
        self.step_fits(1, cx);
    }

    fn previous_fit(&mut self, _: &PreviousFit, _: &mut Window, cx: &mut Context<Self>) {
        self.step_fits(-1, cx);
    }

    /// Puts the fit `step` along the list from the one in the line in it,
    /// going round.
    fn step_fits(&mut self, step: isize, cx: &mut Context<Self>) {
        let fits = self.fits();
        let Some(opened) = &mut self.opened else {
            return;
        };
        if fits.is_empty() {
            return;
        }
        let chosen = match opened.chosen {
            Some(at) => at.checked_add_signed(step).unwrap_or(fits.len() - 1) % fits.len(),
            None if step > 0 => 0,
            None => fits.len() - 1,
        };
        opened.chosen = Some(chosen);
        self.put(fits[chosen].line.clone(), cx);
        cx.notify();
    }

    fn older(&mut self, _: &Older, _: &mut Window, cx: &mut Context<Self>) {
        self.browse(Direction::Backward, cx);
    }

    fn newer(&mut self, _: &Newer, _: &mut Window, cx: &mut Context<Self>) {
        self.browse(Direction::Forward, cx);
    }

    /// Puts the line typed before or after the one in the line in it, of
    /// those that start as the human's typing does: or past the latest,
    /// what they typed.
    fn browse(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(opened) = &self.opened else {
            return;
        };
        let (prompt, typed, at) = (opened.prompt, opened.typed.clone(), opened.browsing);
        let history = self.history(prompt);
        let starts = |place: &usize| history[*place].starts_with(&typed);
        let found = match direction {
            Direction::Backward => (0..at.unwrap_or(history.len())).rev().find(starts),
            Direction::Forward => at.and_then(|at| (at + 1..history.len()).find(starts)),
        };
        if found.is_none() && (direction == Direction::Backward || at.is_none()) {
            return;
        }
        let text = found.map_or_else(|| typed.clone(), |place| history[place].clone());
        if let Some(opened) = &mut self.opened {
            opened.browsing = found;
            opened.chosen = None;
        }
        self.put(text, cx);
        cx.notify();
    }

    fn render_fits(&self, fits: &[Fit]) -> Option<AnyElement> {
        if fits.is_empty() {
            return None;
        }
        let chosen = self.opened.as_ref().and_then(|opened| opened.chosen);
        let first = chosen.map_or(0, |chosen| (chosen + 1).saturating_sub(ROWS));
        let rows = fits
            .iter()
            .enumerate()
            .skip(first)
            .take(ROWS)
            .map(|(place, fit)| {
                div()
                    .debug_selector(move || format!("vim-fit-{place}"))
                    .h(px(menu::ROW_HEIGHT))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap_4()
                    .rounded(px(menu::ROW_RADIUS))
                    .whitespace_nowrap()
                    .when(chosen == Some(place), |row| row.bg(rgb(SELECTED)))
                    .child(div().w(px(120.)).flex_shrink_0().child(fit.shown.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(FAINT))
                            .child(fit.doc.clone()),
                    )
            });
        Some(
            menu::menu()
                .debug_selector(|| "vim-fits".to_owned())
                .absolute()
                .bottom(px(LINE_HEIGHT))
                .left(px(GAP))
                .w(px(FITS_WIDTH))
                .children(rows)
                .into_any_element(),
        )
    }

    fn render_status(&self, status: Status) -> impl IntoElement + use<> {
        let color = match status.mode {
            Mode::Normal => FAINT,
            Mode::Insert => ACCENT,
            Mode::Hint => ASKING,
            Mode::Command => LINK,
            Mode::Visual(_) => FOREGROUND,
        };
        div()
            .debug_selector(|| "vim-status".to_owned())
            .h(px(LINE_HEIGHT))
            .px(px(GAP))
            .flex()
            .items_center()
            .gap(px(12.))
            .text_size(px(12.))
            .child(
                // The mode, as a pill with a dot in its color, as the chat's
                // agent is shown over its input.
                div()
                    .flex_shrink_0()
                    .h(px(PILL_HEIGHT))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE))
                    .child(
                        div()
                            .size(px(9.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgba(color << 8 | 0x1e))
                            .child(div().size(px(5.)).rounded_full().bg(rgb(color))),
                    )
                    .child(status.mode.label()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(FAILED))
                    .children(self.said.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(FAINT))
                    .child(status.held),
            )
    }
}

impl Render for Line {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let foot = div().relative().flex_shrink_0();
        let Some(opened) = &self.opened else {
            return foot.children(super::status(cx).map(|status| self.render_status(status)));
        };
        let mark = match opened.prompt {
            Prompt::Ex => ":",
            Prompt::Search(Direction::Forward) => "/",
            Prompt::Search(Direction::Backward) => "?",
        };
        let fits = self.fits();
        foot.key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::leave))
            .on_action(cx.listener(Self::accept))
            .on_action(cx.listener(Self::next_fit))
            .on_action(cx.listener(Self::previous_fit))
            .on_action(cx.listener(Self::older))
            .on_action(cx.listener(Self::newer))
            .children(self.render_fits(&fits))
            .child(
                div()
                    .debug_selector(|| "vim-line".to_owned())
                    .h(px(LINE_HEIGHT))
                    .px(px(GAP))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .h(px(PILL_HEIGHT + 4.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(BORDER_STRONG))
                            .bg(rgb(SURFACE))
                            .text_size(px(13.))
                            .child(div().text_color(rgb(FAINT)).child(mark))
                            .child(div().flex_1().min_w_0().child(self.input.clone())),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use super::*;

    /// A line in a window of its own.
    fn line(cx: &mut TestAppContext) -> (Entity<Line>, &mut VisualTestContext) {
        cx.add_window_view(Line::new)
    }

    /// Opens the line for `prompt`, types `text` and runs it.
    fn run(line: &Entity<Line>, prompt: Prompt, text: &str, cx: &mut VisualTestContext) {
        line.update_in(cx, |line, window, cx| {
            line.open(prompt, Names::default(), window, cx);
            line.input.update(cx, |input, cx| input.set_text(text, cx));
        });
        line.update_in(cx, |line, window, cx| line.accept(&Accept, window, cx));
    }

    /// Opens the line for `prompt`, and puts in it the line typed
    /// before, `times` over, as Up does.
    fn up(line: &Entity<Line>, prompt: Prompt, times: usize, cx: &mut VisualTestContext) -> String {
        line.update_in(cx, |line, window, cx| {
            line.open(prompt, Names::default(), window, cx);
        });
        for _ in 0..times {
            line.update_in(cx, |line, window, cx| line.older(&Older, window, cx));
        }
        line.read_with(cx, |line, cx| line.text(cx).to_owned())
    }

    #[gpui::test]
    fn a_line_run_again_is_kept_once_as_the_latest(cx: &mut TestAppContext) {
        let (line, cx) = line(cx);
        for typed in ["one", "two", "one"] {
            run(&line, Prompt::Ex, typed, cx);
        }

        let older: Vec<String> = (1..=3)
            .map(|times| up(&line, Prompt::Ex, times, cx))
            .collect();

        assert_eq!(older, ["one", "two", "two"]);
    }

    #[gpui::test]
    fn what_was_searched_for_is_kept_apart_from_the_commands_run(cx: &mut TestAppContext) {
        let (line, cx) = line(cx);
        run(&line, Prompt::Ex, "command", cx);
        run(&line, Prompt::Search(Direction::Forward), "found", cx);

        assert_eq!(up(&line, Prompt::Ex, 1, cx), "command");
        assert_eq!(
            up(&line, Prompt::Search(Direction::Backward), 1, cx),
            "found"
        );
    }

    #[gpui::test]
    fn an_empty_line_run_is_not_kept(cx: &mut TestAppContext) {
        let (line, cx) = line(cx);
        run(&line, Prompt::Ex, "command", cx);
        run(&line, Prompt::Ex, "  ", cx);

        assert_eq!(up(&line, Prompt::Ex, 1, cx), "command");
    }
}
