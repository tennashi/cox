//! The list over the chat's input of what the line can go on to be: the
//! agent's commands, while the word at the cursor is `/` and the start of
//! one of their names, as `/rev`, and the project's files, while it is
//! `@` and the start of a path, as `@src/ma` or `@chv` (see [`mention`]).
//! Up and Down choose one, Tab puts it in the line, as Enter does unless
//! the line has it whole already, when Enter sends the line, and Escape
//! closes the list until the line changes.
//!
//! A command chosen goes to the start of the line, wherever its name was
//! being written, as the agent runs the command a line starts with and
//! takes the rest of the line for its input: `look at the last commit
//! /rev` becomes `/review look at the last commit`.

use std::ops::Range;
use std::path::Path;

use cox_acp::SlashCommand;
use cox_editor::{changes, files, fuzzy};
use gpui::{
    AnyElement, App, Context, Entity, Focusable, FontWeight, ScrollHandle, Subscription, Task,
    Window, deferred, div, prelude::*, px, rgb,
};

use super::ChatView;
use crate::chat::mention;
use crate::ui::ellipsis::ellipsis;
use crate::ui::input::LineInput;
use crate::ui::theme::{BORDER_STRONG, CODE_FONT, DIM, FAINT, FOREGROUND, RAISED, SELECTED};

/// How many rows the list shows before it scrolls.
const ROWS: f32 = 8.;
const ROW_HEIGHT: f32 = 34.;

/// The most files of the project the list looks among.
const MOST_FILES: usize = 50_000;

/// The most files the list shows.
const MOST_FOUND: usize = 50;

/// What the list over the input knows of the line: which row is chosen,
/// whether the human closed the list for it, and the project's files.
pub(super) struct Completion {
    /// The line, and where its cursor was, as last seen, to tell when
    /// they change.
    seen: (String, usize),
    /// The place in the list of the row chosen, on the line `line`: on
    /// another line, the first is.
    chosen: usize,
    line: String,
    /// The line the human closed the list on, with Escape, which keeps it
    /// closed until the line changes.
    dismissed: Option<String>,
    scroll: ScrollHandle,
    /// The files of the project, by their paths from its folder, those
    /// that changed first, as listed when the human last started a path
    /// after `@`.
    files: Vec<String>,
    /// Lists them, while it does.
    listing: Option<Task<()>>,
    /// The files that fit the path being written, the best first, and
    /// what of it they fit, while one is.
    found: Option<(String, Vec<String>)>,
    _changes: Subscription,
}

impl Completion {
    /// Follows `input`, the chat's input, for the list over it.
    pub(super) fn new(input: &Entity<LineInput>, cx: &mut Context<ChatView>) -> Self {
        let changes = cx.observe(input, |view, input, cx| {
            let input = input.read(cx);
            let (line, cursor) = (input.text(), input.cursor());
            let completion = &mut view.completion;
            if completion.seen.0 == line && completion.seen.1 == cursor {
                return;
            }
            if completion.seen.0 != line {
                completion.scroll.scroll_to_item(0);
                if completion.dismissed.as_deref() != Some(line) {
                    completion.dismissed = None;
                }
            }
            completion.seen = (line.to_owned(), cursor);
            match mention::typing(line, cursor) {
                Some(typing) => {
                    let query = typing.query.to_owned();
                    // A path just started: the files are listed again, as
                    // they are now.
                    if completion.found.is_none()
                        && let Some(folder) = view.cwd()
                    {
                        view.completion.listing = Some(list_files(folder, cx));
                    }
                    view.completion.find(query);
                }
                None => completion.found = None,
            }
            cx.notify();
        });
        Self {
            seen: (String::new(), 0),
            chosen: 0,
            line: String::new(),
            dismissed: None,
            scroll: ScrollHandle::new(),
            files: Vec::new(),
            listing: None,
            found: None,
            _changes: changes,
        }
    }

    /// Finds the files that fit `query`, unless they were found for it.
    fn find(&mut self, query: String) {
        if self
            .found
            .as_ref()
            .is_some_and(|(found_for, _)| *found_for == query)
        {
            return;
        }
        let found = fuzzy::find(&query, &self.files, MOST_FOUND)
            .into_iter()
            .map(str::to_owned)
            .collect();
        self.found = Some((query, found));
    }
}

/// Lists the files of the project in `folder`, those that changed first,
/// in the background, for the paths after `@`, and finds again those that
/// fit the path being written once it has.
fn list_files(folder: std::path::PathBuf, cx: &mut Context<ChatView>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        let listed = cx
            .background_executor()
            .spawn(async move { changed_first(&folder) })
            .await;
        let _ = this.update(cx, |view, cx| {
            let completion = &mut view.completion;
            completion.files = listed;
            completion.listing = None;
            if let Some((query, _)) = completion.found.take() {
                completion.find(query);
            }
            cx.notify();
        });
    })
}

/// The files of the project in `folder`, those that changed, as the
/// stage's "Changes" shows them, first.
fn changed_first(folder: &Path) -> Vec<String> {
    let all = files::all(folder, MOST_FILES);
    let mut changed: Vec<String> = changes::read(folder)
        .map(|diff| {
            diff.files
                .iter()
                .map(|file| diff.top.join(&file.path))
                .filter(|path| path.is_file())
                .filter_map(|path| {
                    let path = path.strip_prefix(folder).ok()?;
                    Some(path.to_string_lossy().replace('\\', "/"))
                })
                .collect()
        })
        .unwrap_or_default();
    changed.retain(|path| all.contains(path));
    let rest: Vec<String> = all
        .into_iter()
        .filter(|path| !changed.contains(path))
        .collect();
    changed.extend(rest);
    changed
}

/// What the list shows.
enum Fits<'v> {
    /// The agent's commands the line can go on to run, the name being
    /// written in `range` of the line.
    Commands {
        range: Range<usize>,
        commands: Vec<&'v SlashCommand>,
    },
    /// The files whose path the line can go on to have, in place of
    /// `range` of the line.
    Files {
        range: Range<usize>,
        paths: &'v [String],
    },
}

impl Fits<'_> {
    fn len(&self) -> usize {
        match self {
            Fits::Commands { commands, .. } => commands.len(),
            Fits::Files { paths, .. } => paths.len(),
        }
    }
}

/// The name of a command being written where the cursor is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Naming<'l> {
    /// Where the `/` and the name are in the line, which leave it for the
    /// command chosen to go to its start.
    pub range: Range<usize>,
    /// What of the name is written before the cursor.
    pub query: &'l str,
}

/// The name of a command being written in the word that ends at `cursor`,
/// a byte offset of `line`, if the word is `/` and the start of a name:
/// anywhere in the line, after what the human wrote for the command, as
/// `look at this /rev`, or before it.
pub(super) fn naming(line: &str, cursor: usize) -> Option<Naming<'_>> {
    let before = line.get(..cursor)?;
    let start = before
        .char_indices()
        .rev()
        .find(|(_, letter)| letter.is_whitespace())
        .map_or(0, |(gap, letter)| gap + letter.len_utf8());
    let query = before[start..].strip_prefix('/')?;
    // A path, as `/usr/bin`, is no command's name.
    if query.contains('/') {
        return None;
    }
    // The rest of the word goes too, when the cursor is inside it.
    let end = line[cursor..]
        .find(char::is_whitespace)
        .map_or(line.len(), |gap| cursor + gap);
    Some(Naming {
        range: start..end,
        query,
    })
}

/// The commands of `commands` whose name can start with `query`, what of
/// a name is written after its `/`, the best first: those whose name
/// starts with it, then those whose name has it, then those whose
/// description has it, whatever its case. `None` for a query that no
/// command fits. The phone's page finds them as this does, in its own
/// `matching`.
pub(super) fn matching<'c>(
    commands: &'c [SlashCommand],
    query: &str,
) -> Option<Vec<&'c SlashCommand>> {
    let typed = query.to_lowercase();
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

/// `line` with the command `name` at its start, ready for its input, and
/// where its cursor goes. The name being written in `range` leaves the
/// line, with the gap before it, and the command of `commands` the line
/// started with, if it did, gives way, as the agent runs the command a
/// line starts with and no other. The cursor goes where the name was
/// being written, for the human to go on writing there. The phone's page
/// does this as this does, in its own `taken`.
pub(super) fn taken(
    line: &str,
    range: Range<usize>,
    name: &str,
    commands: &[SlashCommand],
) -> (String, usize) {
    let before = &line[..range.start];
    let before = before.strip_suffix(char::is_whitespace).unwrap_or(before);
    let rest = format!("{before}{}", &line[range.end..]);
    let first = rest.split(char::is_whitespace).next().unwrap_or_default();
    let started = first
        .strip_prefix('/')
        .is_some_and(|old| commands.iter().any(|command| command.name == old));
    let lead = if started { first.len() } else { 0 };
    let gone = rest.len() - rest[lead..].trim_start().len();
    let command = format!("/{name} ");
    let cursor = command.len() + before.len().saturating_sub(gone);
    (format!("{command}{}", &rest[gone..]), cursor)
}

impl ChatView {
    /// What the list over the input shows, while it is open.
    fn completions(&self, cx: &App) -> Option<Fits<'_>> {
        let input = self.input.read(cx);
        let line = input.text();
        if self.completion.dismissed.as_deref() == Some(line) {
            return None;
        }
        if let Some(naming) = naming(line, input.cursor())
            && let Some(commands) = matching(self.conversation.commands(), naming.query)
        {
            return Some(Fits::Commands {
                range: naming.range,
                commands,
            });
        }
        let typing = mention::typing(line, input.cursor())?;
        let (_, paths) = self.completion.found.as_ref()?;
        (!paths.is_empty()).then_some(Fits::Files {
            range: typing.range,
            paths,
        })
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

    /// Tab, or Enter when `sending` is false: puts the row chosen in the
    /// line. Enter, with `sending`, sends a line that has it whole already
    /// instead. Answers whether the list took the key.
    pub(super) fn complete_take(
        &mut self,
        sending: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.typing(window, cx) {
            return false;
        }
        let Some(fits) = self.completions(cx) else {
            return false;
        };
        let chosen = self.chosen(fits.len(), cx);
        let line = self.input.read(cx).text();
        match fits {
            Fits::Commands { range, commands } => {
                let name = commands[chosen].name.clone();
                // Enter sends a line that starts with the command whole,
                // and takes one written after something else to the start.
                if sending && range.start == 0 && line[range.clone()] == format!("/{name}") {
                    return false;
                }
                self.take_command(range, &name, cx);
            }
            Fits::Files { range, paths } => {
                let written = mention::written(&paths[chosen]);
                if sending && line[range.clone()] == written {
                    return false;
                }
                self.take_file(range, &written, cx);
            }
        }
        true
    }

    /// Puts `written`, the way to point to a file, in place of `range` of
    /// the line, with a space after it.
    fn take_file(&mut self, range: Range<usize>, written: &str, cx: &mut Context<Self>) {
        let written = format!("{written} ");
        self.input
            .update(cx, |input, cx| input.replace(range, &written, cx));
    }

    /// Puts the command `name` at the start of the line, ready for its
    /// input, in place of what of it was being written in `range`.
    fn take_command(&mut self, range: Range<usize>, name: &str, cx: &mut Context<Self>) {
        let (line, cursor) = taken(
            self.input.read(cx).text(),
            range,
            name,
            self.conversation.commands(),
        );
        self.input
            .update(cx, |input, cx| input.set_text_at(&line, cursor, cx));
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

    /// The "/ commands" chip under the input: starts a command's name at
    /// the cursor with `/`, which opens the list of the agent's commands,
    /// while the agent has commands.
    pub(super) fn start_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_word('/', window, cx);
    }

    /// The "@ files" chip under the input: starts a path at the cursor
    /// with `@`, which opens the list of the project's files, while the
    /// chat has a folder.
    pub(super) fn start_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_word('@', window, cx);
    }

    /// Starts a word with `mark` at the cursor, which opens the list.
    fn start_word(&mut self, mark: char, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.input.read(cx);
        let cursor = input.cursor();
        let before = &input.text()[..cursor];
        // A name or a path is written at the start of a word.
        let start = if before.is_empty() || before.ends_with(char::is_whitespace) {
            mark.to_string()
        } else {
            format!(" {mark}")
        };
        self.input
            .update(cx, |input, cx| input.replace(cursor..cursor, &start, cx));
        self.completion.dismissed = None;
        window.focus(&self.input.focus_handle(cx));
        cx.notify();
    }

    /// The list over the input, while it is open, the row chosen marked.
    pub(super) fn render_completions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let fits = self.completions(cx)?;
        let chosen = self.chosen(fits.len(), cx);
        let rows = match fits {
            Fits::Commands { range, commands } => {
                Self::render_commands(&range, commands, chosen, cx)
            }
            Fits::Files { range, paths } => render_files(&range, paths, chosen, cx),
        };
        Some(
            deferred(
                div()
                    .id("completions")
                    .debug_selector(|| "completions".to_owned())
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
                            .id("completions-rows")
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

    /// Each command by its name, with what it does, and what to write
    /// after it.
    fn render_commands(
        range: &Range<usize>,
        commands: Vec<&SlashCommand>,
        chosen: usize,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        commands
            .into_iter()
            .enumerate()
            .map(|(place, command)| {
                let name = command.name.clone();
                let range = range.clone();
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
                        command
                            .hint
                            .clone()
                            .map(|hint| div().flex_shrink_0().text_color(rgb(FAINT)).child(hint)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_color(rgb(DIM))
                            .child(ellipsis(command.description.clone())),
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.take_command(range.clone(), &name, cx);
                        window.focus(&view.input.focus_handle(cx));
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

/// Each file by its name, with the folder it is in.
fn render_files(
    range: &Range<usize>,
    paths: &[String],
    chosen: usize,
    cx: &mut Context<ChatView>,
) -> Vec<AnyElement> {
    paths
        .iter()
        .enumerate()
        .map(|(place, path)| {
            let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
            let written = mention::written(path);
            let range = range.clone();
            div()
                .id(("file", place))
                .debug_selector(move || format!("file-{place}"))
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
                        .child(name.to_owned()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .font_family(CODE_FONT)
                        .text_color(rgb(DIM))
                        .child(ellipsis(folder.to_owned())),
                )
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.take_file(range.clone(), &written, cx);
                    window.focus(&view.input.focus_handle(cx));
                }))
                .into_any_element()
        })
        .collect()
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

    /// The commands the list shows for `line`, with the cursor at its end.
    fn listed<'c>(commands: &'c [SlashCommand], line: &str) -> Option<Vec<&'c str>> {
        let naming = naming(line, line.len())?;
        names(matching(commands, naming.query))
    }

    #[test]
    fn the_names_that_start_so_come_first() {
        let commands = [
            command("pr-comments", "Read the comments of a pull request"),
            command("review", "Review the changes"),
            command("compact", "Sum up the conversation"),
        ];
        assert_eq!(
            listed(&commands, "/"),
            Some(vec!["pr-comments", "review", "compact"])
        );
        assert_eq!(
            listed(&commands, "/re"),
            Some(vec!["review", "pr-comments"])
        );
        assert_eq!(listed(&commands, "/SUM"), Some(vec!["compact"]));
    }

    #[test]
    fn the_list_is_for_a_name_being_typed() {
        let commands = [command("review", "Review the changes")];
        assert_eq!(listed(&commands, "review"), None);
        assert_eq!(listed(&commands, "/review "), None);
        assert_eq!(listed(&commands, "/review the last commit"), None);
        assert_eq!(listed(&commands, "/zzz"), None);
        assert_eq!(listed(&[], "/"), None);
    }

    #[test]
    fn a_name_is_written_anywhere_in_the_line() {
        let commands = [command("review", "Review the changes")];
        assert_eq!(
            listed(&commands, "look at the last commit /rev"),
            Some(vec!["review"])
        );
        assert_eq!(
            listed(&commands, "最後のコミットを見て /"),
            Some(vec!["review"])
        );
        assert_eq!(listed(&commands, "and/or"), None);
        assert_eq!(listed(&commands, "see /usr/re"), None);

        let line = "/rev the last commit";
        assert_eq!(
            naming(line, 4),
            Some(Naming {
                range: 0..4,
                query: "rev"
            })
        );
        assert_eq!(naming(line, 2).map(|naming| naming.range), Some(0..4));
        assert_eq!(naming(line, 5), None);
    }

    #[test]
    fn a_command_chosen_goes_to_the_start() {
        let commands = [
            command("review", "Review the changes"),
            command("compact", "Sum up the conversation"),
        ];
        // `|` marks the cursor.
        let take = |marked: &str, name: &str| {
            let cursor = marked.find('|').unwrap();
            let line = marked.replacen('|', "", 1);
            let range = naming(&line, cursor).unwrap().range;
            let (line, cursor) = taken(&line, range, name, &commands);
            format!("{}|{}", &line[..cursor], &line[cursor..])
        };
        assert_eq!(take("/rev|", "review"), "/review |");
        assert_eq!(
            take("/rev| the last commit", "review"),
            "/review |the last commit"
        );
        assert_eq!(
            take("look at the last commit /rev|", "review"),
            "/review look at the last commit|"
        );
        assert_eq!(
            take("look at /r|ev the last commit", "review"),
            "/review look at| the last commit"
        );
        assert_eq!(
            take("最後のコミットを\n/rev|", "review"),
            "/review 最後のコミットを|"
        );
        assert_eq!(
            take("/review the last commit /com|", "compact"),
            "/compact the last commit|"
        );
        assert_eq!(
            take("/usr/bin is gone /rev|", "review"),
            "/review /usr/bin is gone|"
        );
    }
}
