//! How Cox stands, in words, for an agent to read: what the window shows,
//! as the agent would see it on the screen, and what it stands for, which
//! the screen does not say, such as the state a chat is in.

use std::fmt::Write as _;

use cox_core::store::ChatId;
use cox_core::tabs::KeptTab;
use gpui::{App, Focusable as _, Window};

use crate::chat::conversation::{
    Answer, Content, Entry, GOAL_TURNS, State, StepState, ToolCallResult,
};
use crate::chat::view::ChatView;
use crate::ui::translation::Shown;
use crate::workspace::{Chat, Workspace};

/// The most of a reply, a thought or a note the state tells; the rest is
/// counted.
const TEXT_LENGTH: usize = 2000;

/// How `workspace` stands, with the chat that goes by `chat` told in
/// full, or the chat shown, and the stages' `DevTools` on `devtools`.
#[must_use]
pub fn describe(
    workspace: &Workspace,
    chat: Option<u64>,
    devtools: Option<u16>,
    window: &Window,
    cx: &App,
) -> String {
    let mut told = String::new();
    let _ = writeln!(
        told,
        "Cox {}, process {}, working in {}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        std::env::current_dir().map_or_else(|_| "?".to_owned(), |cwd| cwd.display().to_string())
    );
    if let Some(data) = crate::storage::root() {
        let _ = writeln!(told, "It keeps what lasts in {}", data.display());
    }
    match crate::logs::file() {
        Some(file) => {
            let _ = writeln!(told, "This run is kept in {}", file.display());
        }
        None => told.push_str("This run is kept in no file\n"),
    }
    match devtools {
        Some(port) => {
            let _ = writeln!(told, "The stages' pages: DevTools on port {port}");
        }
        None => told.push_str("The stages' pages: no DevTools\n"),
    }
    let bounds = window.bounds();
    let _ = writeln!(
        told,
        "The window: {}×{} points at {}, {} on the screen, {} pixels a point; the keyboard is in {}",
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        window.scale_factor(),
        focused(workspace, window, cx)
    );
    let _ = writeln!(
        told,
        "Cox's own chats work in {}",
        workspace.home().display()
    );
    if workspace.is_folded() {
        told.push_str("The chat's panel is folded to the window's right edge\n");
    } else if workspace.is_switching() {
        told.push_str("The list of chats is open, over the conversation\n");
    }
    if let Some(settings) = workspace.settings() {
        let _ = writeln!(
            told,
            "The settings are open, the home's input holding {:?}",
            settings.read(cx).input().read(cx).text()
        );
        for agent in settings.read(cx).agents().read(cx).described(cx) {
            let _ = writeln!(told, "  Agent row {agent}");
        }
        for line in settings.read(cx).described(cx) {
            let _ = writeln!(told, "  {line}");
        }
    }
    if let Some(card) = cx
        .try_global::<crate::ui::translation::Shown>()
        .and_then(|shown| shown.0.as_ref())
    {
        let _ = writeln!(told, "A translation shows: {}", card.read(cx).said());
    }
    told.push('\n');
    list(&mut told, workspace);
    let about = match chat {
        Some(key) => workspace.chat(key),
        None => workspace.shown(),
    };
    match about {
        Some(about) => {
            told.push('\n');
            tell(&mut told, about, workspace, cx);
        }
        None if chat.is_some() => {
            let _ = writeln!(told, "\nThere is no chat {}.", chat.unwrap_or_default());
        }
        None => told.push_str("\nNo chat is shown.\n"),
    }
    told
}

/// Where the keyboard is, as far as the state can tell.
fn focused(workspace: &Workspace, window: &Window, cx: &App) -> String {
    let Some(focused) = window.focused(cx) else {
        return "nothing".to_owned();
    };
    if let Some(card) = cx.try_global::<Shown>().and_then(|shown| shown.0.as_ref())
        && focused == card.focus_handle(cx)
    {
        return "the translation's card".to_owned();
    }
    if let Some(settings) = workspace.settings().map(|settings| settings.read(cx)) {
        if focused == settings.input().focus_handle(cx) {
            return "the settings' input".to_owned();
        }
        if focused == *settings.words_focus() {
            return format!("the settings' text, {:?} selected", settings.selected());
        }
    }
    if workspace.is_folded() && focused == workspace.focus_handle(cx) {
        return "the window, the chat folded away".to_owned();
    }
    let Some(shown) = workspace.shown().and_then(Chat::view) else {
        return if focused == workspace.focus_handle(cx) {
            "the window, which shows no chat".to_owned()
        } else {
            "something else".to_owned()
        };
    };
    let shown = shown.read(cx);
    if focused == shown.input().focus_handle(cx) {
        "the input".to_owned()
    } else if shown
        .goal_input()
        .is_some_and(|input| focused == input.focus_handle(cx))
    {
        "the goal's input".to_owned()
    } else if shown
        .sketch_input()
        .is_some_and(|(input, _)| focused == input.focus_handle(cx))
    {
        "the sketch's input".to_owned()
    } else {
        "the stage, or a chat's name being changed".to_owned()
    }
}

/// The chats, in the order of the list, under their projects, and which
/// project the rail has chosen.
fn list(told: &mut String, workspace: &Workspace) {
    told.push_str("The chats, as listed:\n");
    let mut project = None;
    for (chat, depth) in workspace.listed() {
        if project != Some(chat.project()) {
            project = Some(chat.project());
            let named = &workspace.projects()[chat.project()];
            let chosen = if workspace.chosen() == chat.project() {
                ", chosen"
            } else {
                ""
            };
            let _ = writeln!(
                told,
                "  {} ({}){chosen}",
                named.name(),
                named.folder().display()
            );
        }
        let indent = "  ".repeat(2 + usize::from(depth));
        let _ = writeln!(told, "{indent}{}", row(chat, workspace));
    }
    if project.is_none() {
        told.push_str("  none\n");
    }
    if !workspace.archived().is_empty() {
        told.push_str("Archived:\n");
        for chat in workspace.archived() {
            let _ = writeln!(told, "    {}", row(chat, workspace));
        }
    }
}

/// A chat's row of the list, as the state tells it.
fn row(chat: &Chat, workspace: &Workspace) -> String {
    let mut row = format!("chat {} {:?}: {}", chat.key(), chat.name(), stands(chat));
    if workspace
        .shown()
        .is_some_and(|shown| shown.key() == chat.key())
    {
        row.push_str(", shown");
    }
    if chat.is_unread() {
        row.push_str(", unread");
    }
    if chat.goal().is_some() {
        row.push_str(", goal");
    }
    if chat.is_sketch() {
        row.push_str(", sketch");
    }
    if let Some(parent) = chat.parent() {
        let _ = write!(row, ", opened by chat {parent}");
    }
    row
}

/// How `chat` stands, as its row marks it.
fn stands(chat: &Chat) -> &'static str {
    if !chat.is_open() {
        return "closed";
    }
    match chat.state() {
        State::Idle => "idle",
        State::Busy => "busy",
        State::Asking => "asking",
        State::Stopping => "stopping",
        State::Checking => "checking",
        State::Closed => "closed",
    }
}

/// `chat` in full.
fn tell(told: &mut String, chat: &Chat, workspace: &Workspace, cx: &App) {
    let _ = writeln!(told, "Chat {} {:?}:", chat.key(), chat.name());
    let Some(view) = chat.view() else {
        told.push_str("  Not read yet: showing it reads it.\n");
        return;
    };
    let view = view.read(cx);
    let shown = workspace
        .shown()
        .is_some_and(|shown| shown.key() == chat.key());
    let _ = writeln!(told, "  {}", stands(chat));
    if !shown {
        told.push_str("  Not shown: its input and stage are as they were left.\n");
    }
    if let Some(waiting) = view.waiting() {
        let _ = writeln!(told, "  Waiting: {waiting}");
    }
    let _ = writeln!(told, "  Input: {:?}", view.input().read(cx).text());
    if let Some(input) = view.goal_input() {
        let _ = writeln!(told, "  Goal input, open: {:?}", input.read(cx).text());
    }
    if let Some((input, sketch)) = view.sketch_input() {
        let _ = writeln!(
            told,
            "  Sketch input, open, from {}: {:?}",
            sketch.name(),
            input.read(cx).text()
        );
    }
    let conversation = view.conversation();
    if let Some(goal) = conversation.goal() {
        let _ = writeln!(
            told,
            "  Goal: {}, turn {} of {GOAL_TURNS}",
            goal.line(),
            goal.turns()
        );
    }
    if let Some(agents) = view.agents() {
        let known = agents.of(Some(view.agent_id()));
        let _ = writeln!(
            told,
            "  Agent {:?} ({}), of {} agents",
            known.name,
            known.id,
            agents.all().len()
        );
    }
    for setting in conversation.settings() {
        let chosen = setting
            .chosen()
            .map_or(setting.value.as_str(), |choice| choice.name.as_str());
        let choices: Vec<&str> = setting.choices.iter().map(|c| c.name.as_str()).collect();
        let _ = writeln!(
            told,
            "  Setting {:?}: {chosen:?}, of {}",
            setting.name,
            choices.join(", ")
        );
    }
    tabs(told, view, cx);
    let entries = conversation.entries();
    let _ = writeln!(told, "  Conversation, {} entries:", entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let _ = writeln!(told, "    {} {}", index + 1, describe_entry(entry));
    }
}

/// The stage's tabs, each with the chat it is of.
fn tabs(told: &mut String, view: &ChatView, cx: &App) {
    let Some(tabs) = view.tabs(cx) else {
        told.push_str("  Stage: none\n");
        return;
    };
    let _ = write!(
        told,
        "  Stage: {}; tabs:",
        if view.stage_in_sight(cx) {
            "in sight"
        } else {
            "out of sight"
        }
    );
    if tabs.tabs.is_empty() {
        told.push_str(" none");
    }
    for (index, tab) in tabs.tabs.iter().enumerate() {
        let what = match tab {
            KeptTab::Page { url, title } => format!("page {url} {title:?}"),
            KeptTab::Markdown { path } => format!("markdown {}", path.display()),
            KeptTab::File { path, lines } => match lines {
                Some(lines) => format!(
                    "file {} lines {}–{}",
                    path.display(),
                    lines.first,
                    lines.last
                ),
                None => format!("file {}", path.display()),
            },
            KeptTab::Files { open } => format!("files, {} folders open", open.len()),
            KeptTab::Changes { folder } => format!("changes in {}", folder.display()),
            KeptTab::Map { .. } => "map".to_owned(),
            KeptTab::Flow { call } => format!("flow of tool call {call}"),
            KeptTab::Android { device } => format!("android emulator {device}"),
            KeptTab::AndroidDevices => "android devices".to_owned(),
            KeptTab::MacWindow { app, title } => format!("mac window {app} {title:?}"),
            KeptTab::MacWindows => "mac windows".to_owned(),
            KeptTab::Workflow => "workflow".to_owned(),
            KeptTab::Parts => "parts the chat ran with".to_owned(),
            KeptTab::Terminal { title } => format!("terminal {title:?}"),
        };
        let _ = write!(told, " {}. {what}", index + 1);
        match tabs.owner(index) {
            Some(owner) => {
                let _ = write!(told, " of chat {owner}");
            }
            None => told.push_str(" of the human"),
        }
        if index == tabs.shown {
            told.push_str(" (shown)");
        }
        if index + 1 < tabs.tabs.len() {
            told.push(',');
        }
    }
    told.push('\n');
}

/// One entry of a conversation, on a line, or on more where its text
/// has them.
fn describe_entry(entry: &Entry) -> String {
    match entry {
        Entry::Human(text) => format!("human: {}", text_of(text)),
        Entry::Cox(text) => format!("cox, in the human's place: {}", text_of(text)),
        Entry::Reply(text) => format!("reply: {}", text_of(text)),
        Entry::Thought(text) => format!("thought: {}", text_of(text)),
        Entry::Note(text) => format!("note: {}", text_of(text)),
        Entry::Switched { from, to } => format!("switched: from {from} to {to}"),
        Entry::Login {
            options,
            failed,
            chosen,
        } => {
            let options: Vec<String> = options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    let how = if option.terminal.is_some() {
                        ", in a terminal"
                    } else {
                        ""
                    };
                    format!("{}. {}{how}", index + 1, option.name)
                })
                .collect();
            let failed = failed
                .as_ref()
                .map(|failed| format!("; the last failed: {failed}"))
                .unwrap_or_default();
            let chosen = chosen.map_or("none yet".to_owned(), |chosen| (chosen + 1).to_string());
            format!("login: {}; chosen: {chosen}{failed}", options.join(", "))
        }
        Entry::Plan(steps) => {
            let steps: Vec<String> = steps
                .iter()
                .map(|step| {
                    let state = match step.state {
                        StepState::Pending => "waiting",
                        StepState::InProgress => "under way",
                        StepState::Completed => "done",
                    };
                    format!("[{state}] {}", step.text)
                })
                .collect();
            format!("plan: {}", steps.join("; "))
        }
        Entry::ToolCall {
            title,
            content,
            result,
            ..
        } => {
            let result = match result {
                None => "running",
                Some(ToolCallResult::Completed) => "completed",
                Some(ToolCallResult::Failed) => "failed",
                Some(ToolCallResult::Cancelled) => "cancelled",
            };
            format!("tool call {title:?}: {result}{}", contents(content))
        }
        Entry::Permission {
            title,
            content,
            options,
            chosen,
            by_setup,
            ..
        } => {
            let options: Vec<String> = options
                .iter()
                .enumerate()
                .map(|(index, option)| format!("{} {option}", index + 1))
                .collect();
            let chosen = match chosen {
                Some(chosen) if *by_setup => format!("; Cox chose {}, as set up", chosen + 1),
                Some(chosen) => format!("; chosen {}", chosen + 1),
                None => "; waiting for the choice".to_owned(),
            };
            format!(
                "permission {title:?}: options {}{chosen}{}",
                options.join(", "),
                contents(content)
            )
        }
        Entry::Proposal { goal, why, answer } => {
            let answer = match answer {
                None => "open",
                Some(Answer::Set) => "set",
                Some(Answer::Changed) => "changed and set",
                Some(Answer::Dismissed) => "dismissed",
            };
            match why {
                Some(why) => format!("proposal {goal:?}, {answer}: {}", text_of(why)),
                None => format!("proposal {goal:?}, {answer}"),
            }
        }
        Entry::Keep { why, chat, kept } => describe_keep(why, *chat, *kept),
    }
}

/// Cox's card asking whether to keep in the wiki what a chat learned, at
/// the milestone `why` says, of the chat kept as `chat`, or this one.
fn describe_keep(why: &str, chat: Option<ChatId>, kept: Option<bool>) -> String {
    let kept = match kept {
        None => "open",
        Some(true) => "kept",
        Some(false) => "not now",
    };
    match chat {
        Some(chat) => format!("keep in the wiki, chat {chat}'s, {kept}: {}", text_of(why)),
        None => format!("keep in the wiki, {kept}: {}", text_of(why)),
    }
}

/// What a tool call or a permission request shows, counted.
fn contents(content: &[Content]) -> String {
    if content.is_empty() {
        return String::new();
    }
    let shown: Vec<String> = content
        .iter()
        .map(|content| match content {
            Content::Text(text) => format!("{} characters of text", text.chars().count()),
            Content::Diff { path, lines } => format!("a diff of {path}, {} lines", lines.len()),
        })
        .collect();
    format!("; shows {}", shown.join(", "))
}

/// `text`, with its lines after the first indented under it, and cut
/// where it is long.
fn text_of(text: &str) -> String {
    let text = text.trim_end();
    let mut cut: String = text.chars().take(TEXT_LENGTH).collect();
    let left = text.chars().count().saturating_sub(TEXT_LENGTH);
    if left > 0 {
        let _ = write!(cut, "… ({left} more characters)");
    }
    cut.replace('\n', "\n      ")
}
