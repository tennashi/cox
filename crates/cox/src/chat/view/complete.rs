//! The list over the chat's input of what the line can go on to be: the
//! agent's commands, while the line is `/` and the start of one of their
//! names, as `/rev`. Up and Down choose one, Tab puts it in the line, as
//! Enter does unless the line names it whole already, when Enter sends
//! it, and Escape closes the list until the line changes.

use cox_acp::SlashCommand;
use gpui::{
    AnyElement, App, Context, Entity, Focusable, FontWeight, ScrollHandle, Subscription, Window,
    deferred, div, prelude::*, px, rgb,
};

use super::ChatView;
use crate::ui::input::LineInput;
use crate::ui::theme::{BORDER_STRONG, CODE_FONT, DIM, FAINT, FOREGROUND, RAISED, SELECTED};

/// How many commands the list shows before it scrolls.
const ROWS: f32 = 8.;
const ROW_HEIGHT: f32 = 34.;

/// What the list over the input knows of the line: which command is
/// chosen, and whether the human closed the list for it.
pub(super) struct Completion {
    /// The line as it was last seen, to tell when it changes.
    seen: String,
    /// The place in the list of the command chosen, on the line `line`:
    /// on another line, the first is.
    chosen: usize,
    line: String,
    /// The line the human closed the list on, with Escape, which keeps it
    /// closed until the line changes.
    dismissed: Option<String>,
    scroll: ScrollHandle,
    _changes: Subscription,
}

impl Completion {
    /// Follows `input`, the chat's input, for the list over it.
    pub(super) fn new(input: &Entity<LineInput>, cx: &mut Context<ChatView>) -> Self {
        let changes = cx.observe(input, |view, input, cx| {
            let completion = &mut view.completion;
            let line = input.read(cx).text();
            if completion.seen != line {
                line.clone_into(&mut completion.seen);
                completion.scroll.scroll_to_item(0);
                if completion.dismissed.as_deref() != Some(line) {
                    completion.dismissed = None;
                }
                cx.notify();
            }
        });
        Self {
            seen: String::new(),
            chosen: 0,
            line: String::new(),
            dismissed: None,
            scroll: ScrollHandle::new(),
            _changes: changes,
        }
    }
}

/// The commands of `commands` that `line` can go on to run, the best
/// first, while it is `/` and the start of a name: those whose name starts
/// with what follows the `/`, then those whose name has it, then those
/// whose description has it, whatever its case. `None` for a line that is
/// not, or that no command fits.
pub(super) fn matching<'c>(
    commands: &'c [SlashCommand],
    line: &str,
) -> Option<Vec<&'c SlashCommand>> {
    let typed = line.strip_prefix('/')?;
    if typed.contains(char::is_whitespace) {
        return None;
    }
    let typed = typed.to_lowercase();
    let mut fits: Vec<(usize, &SlashCommand)> = commands
        .iter()
        .filter_map(|command| {
            let name = command.name.to_lowercase();
            let rank = if name.starts_with(&typed) {
                0
            } else if name.contains(&typed) {
                1
            } else if command.description.to_lowercase().contains(&typed) {
                2
            } else {
                return None;
            };
            Some((rank, command))
        })
        .collect();
    // The sort is stable: the agent's order stays within a rank.
    fits.sort_by_key(|(rank, _)| *rank);
    let fits: Vec<_> = fits.into_iter().map(|(_, command)| command).collect();
    (!fits.is_empty()).then_some(fits)
}

impl ChatView {
    /// The commands the list over the input shows, while it is open.
    fn completions(&self, cx: &App) -> Option<Vec<&SlashCommand>> {
        let line = self.input.read(cx).text();
        if self.completion.dismissed.as_deref() == Some(line) {
            return None;
        }
        matching(self.conversation.commands(), line)
    }

    /// The place of the command chosen in the list of `count` for the line
    /// as it is.
    fn chosen(&self, count: usize, cx: &App) -> usize {
        if self.completion.line == self.input.read(cx).text() {
            self.completion.chosen.min(count - 1)
        } else {
            0
        }
    }

    /// Whether the keyboard is in the chat's input, where the list's keys
    /// are the list's.
    fn typing(&self, window: &Window, cx: &App) -> bool {
        self.input.focus_handle(cx).is_focused(window)
    }

    /// Up or Down, by `step`: chooses the command above or below, round
    /// from one end to the other. Answers whether the list took the key.
    pub(super) fn complete_step(
        &mut self,
        step: isize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.typing(window, cx) {
            return false;
        }
        let Some(count) = self.completions(cx).map(|fits| fits.len()) else {
            return false;
        };
        let chosen = self.chosen(count, cx);
        self.completion.chosen = chosen.checked_add_signed(step).unwrap_or(count - 1) % count;
        self.input
            .read(cx)
            .text()
            .clone_into(&mut self.completion.line);
        self.completion
            .scroll
            .scroll_to_item(self.completion.chosen);
        cx.notify();
        true
    }

    /// Tab, or Enter when `sending` is false: puts the command chosen in
    /// the line. Enter, with `sending`, sends a line that names the
    /// command chosen whole already instead. Answers whether the list took
    /// the key.
    pub(super) fn complete_take(
        &mut self,
        sending: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.typing(window, cx) {
            return false;
        }
        let Some(name) = self
            .completions(cx)
            .map(|fits| fits[self.chosen(fits.len(), cx)].name.clone())
        else {
            return false;
        };
        if sending && self.input.read(cx).text() == format!("/{name}") {
            return false;
        }
        self.take_command(&name, cx);
        true
    }

    /// Puts the command `name` in the line, ready for its input.
    fn take_command(&mut self, name: &str, cx: &mut Context<Self>) {
        let line = format!("/{name} ");
        self.input.update(cx, |input, cx| input.set_text(&line, cx));
    }

    /// Escape: closes the list until the line changes. Answers whether the
    /// list took the key.
    pub(super) fn complete_close(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.typing(window, cx) || self.completions(cx).is_none() {
            return false;
        }
        self.completion.dismissed = Some(self.input.read(cx).text().to_owned());
        cx.notify();
        true
    }

    /// The "/ commands" chip under the input: starts an empty line with
    /// `/`, which opens the list, while the agent has commands.
    pub(super) fn start_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).text().is_empty() {
            self.input.update(cx, |input, cx| input.set_text("/", cx));
        }
        self.completion.dismissed = None;
        window.focus(&self.input.focus_handle(cx));
        cx.notify();
    }

    /// The list over the input, while it is open: each command by its
    /// name, with what it does, and what to write after it, the one chosen
    /// marked.
    pub(super) fn render_completions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let fits = self.completions(cx)?;
        let chosen = self.chosen(fits.len(), cx);
        let rows: Vec<_> =
            fits.into_iter()
                .enumerate()
                .map(|(place, command)| {
                    let name = command.name.clone();
                    div()
                        .id(("command", place))
                        .debug_selector(move || format!("command-{place}"))
                        .flex_shrink_0()
                        .h(px(ROW_HEIGHT))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(9.))
                        .text_size(px(13.))
                        .cursor_pointer()
                        .when(place == chosen, |row| row.bg(rgb(SELECTED)))
                        .hover(|style| style.bg(rgb(SELECTED)))
                        .child(
                            div()
                                .flex_shrink_0()
                                .font_family(CODE_FONT)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(FOREGROUND))
                                .child(format!("/{}", command.name)),
                        )
                        .children(
                            command.hint.clone().map(|hint| {
                                div().flex_shrink_0().text_color(rgb(FAINT)).child(hint)
                            }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(DIM))
                                .child(command.description.clone()),
                        )
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.take_command(&name, cx);
                            window.focus(&view.input.focus_handle(cx));
                        }))
                })
                .collect();
        Some(
            deferred(
                div()
                    .id("commands")
                    .debug_selector(|| "commands".to_owned())
                    .occlude()
                    .absolute()
                    .bottom_full()
                    .left(px(12.))
                    .right(px(12.))
                    .p(px(6.))
                    .bg(rgb(RAISED))
                    .border_1()
                    .border_color(rgb(BORDER_STRONG))
                    .rounded(px(14.))
                    .shadow_lg()
                    .child(
                        div()
                            .id("commands-rows")
                            .max_h(px(ROW_HEIGHT * ROWS))
                            .overflow_y_scroll()
                            .track_scroll(&self.completion.scroll)
                            .flex()
                            .flex_col()
                            .children(rows),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str, description: &str) -> SlashCommand {
        SlashCommand {
            name: name.to_owned(),
            description: description.to_owned(),
            hint: None,
        }
    }

    fn names(fits: Option<Vec<&SlashCommand>>) -> Option<Vec<&str>> {
        fits.map(|fits| fits.iter().map(|command| command.name.as_str()).collect())
    }

    #[test]
    fn the_names_that_start_so_come_first() {
        let commands = [
            command("pr-comments", "Read the comments of a pull request"),
            command("review", "Review the changes"),
            command("compact", "Sum up the conversation"),
        ];
        assert_eq!(
            names(matching(&commands, "/")),
            Some(vec!["pr-comments", "review", "compact"])
        );
        assert_eq!(
            names(matching(&commands, "/re")),
            Some(vec!["review", "pr-comments"])
        );
        assert_eq!(names(matching(&commands, "/SUM")), Some(vec!["compact"]));
    }

    #[test]
    fn the_list_is_for_a_name_being_typed() {
        let commands = [command("review", "Review the changes")];
        assert_eq!(names(matching(&commands, "review")), None);
        assert_eq!(names(matching(&commands, "/review ")), None);
        assert_eq!(names(matching(&commands, "/review the last commit")), None);
        assert_eq!(names(matching(&commands, "/zzz")), None);
        assert_eq!(names(matching(&[], "/")), None);
    }
}
