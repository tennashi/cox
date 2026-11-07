//! What the human hands a chat from elsewhere on the screen:
//! [`AddToChat`] in a part of the window with text selected, or "Add to
//! chat" in the menu the right button opens on it, adds what is selected,
//! with where it is from, to the chat shown, as a chip over its input. It
//! goes to the agent with the next line sent there (see [`Attachment`]).

use std::path::Path;

use cox_core::entry::Attachment;
use cox_core::tabs::Lines;
use gpui::{App, Global, KeyBinding, actions};

actions!(
    cox,
    [
        /// Hands the text selected in the part with the keyboard to the
        /// chat shown.
        AddToChat,
    ]
);

/// Binds the keys that hand the selection to the chat shown.
pub fn bind_keys(cx: &mut App) {
    // Control-L is the terminal's, to clear its screen, off macOS.
    let keys = if cfg!(target_os = "macos") {
        "cmd-l"
    } else {
        "ctrl-shift-l"
    };
    cx.bind_keys([KeyBinding::new(keys, AddToChat, None)]);
    cx.set_global(Handed::default());
}

/// The keys that hand the selection, as a menu names them.
#[must_use]
pub fn keys() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘L"
    } else {
        "Ctrl-Shift-L"
    }
}

/// What the human handed, which the chat shown has not taken yet: the
/// window watches it, and gives it to the chat.
#[derive(Default)]
pub struct Handed(Vec<Attachment>);

impl Global for Handed {}

/// Hands `attachment` to the chat shown, unless what it says is only
/// space, saying whether it does.
pub fn hand(attachment: Attachment, cx: &mut App) -> bool {
    if attachment.text.trim().is_empty() || !cx.has_global::<Handed>() {
        return false;
    }
    cx.global_mut::<Handed>().0.push(attachment);
    true
}

/// Takes what the human handed since it was last taken.
pub fn take(cx: &mut App) -> Vec<Attachment> {
    // Changing it tells those who watch it again, who find it empty then.
    if !cx.has_global::<Handed>() || cx.global::<Handed>().0.is_empty() {
        return Vec::new();
    }
    std::mem::take(&mut cx.global_mut::<Handed>().0)
}

/// `text`, selected in the file at `path`, at `lines` when it says,
/// named by the file's name.
#[must_use]
pub fn of_file(path: &Path, lines: Option<Lines>, text: String) -> Attachment {
    let shown = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let uri = cox_acp::file_uri(path);
    let (label, uri) = match lines {
        Some(Lines { first, last }) if first == last => {
            (format!("{shown}:{first}"), format!("{uri}#L{first}"))
        }
        Some(Lines { first, last }) => (
            format!("{shown}:{first}-{last}"),
            format!("{uri}#L{first}-{last}"),
        ),
        None => (shown, uri),
    };
    Attachment {
        label,
        uri,
        text,
        picture: None,
    }
}

/// How many characters of what a chip quotes it shows.
const QUOTED: usize = 24;

/// The start of `text`, quoted, as a chip names what was selected where
/// nothing else names it, as in the conversation.
#[must_use]
pub fn quoted(text: &str) -> String {
    let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.chars().count() <= QUOTED {
        return format!("“{words}”");
    }
    let start: String = words.chars().take(QUOTED).collect();
    format!("“{}…”", start.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_of_a_file_are_named_by_its_name() {
        let attachment = of_file(
            Path::new("/work/app/src/main.rs"),
            Some(Lines { first: 3, last: 5 }),
            "fn main() {}".to_owned(),
        );
        assert_eq!(attachment.label, "main.rs:3-5");
        assert_eq!(attachment.uri, "file:///work/app/src/main.rs#L3-5");
    }

    #[test]
    fn a_line_of_a_file_is_named_by_its_number() {
        let attachment = of_file(
            Path::new("/etc/hosts"),
            Some(Lines { first: 2, last: 2 }),
            "127.0.0.1".to_owned(),
        );
        assert_eq!(attachment.label, "hosts:2");
        assert_eq!(attachment.uri, "file:///etc/hosts#L2");
    }

    #[test]
    fn a_file_with_no_lines_is_named_by_its_name_alone() {
        let attachment = of_file(Path::new("/notes/plan.md"), None, "Plan".to_owned());
        assert_eq!(attachment.label, "plan.md");
        assert_eq!(attachment.uri, "file:///notes/plan.md");
    }

    #[test]
    fn a_quote_is_the_start_of_the_text_on_one_line() {
        assert_eq!(quoted("  short\n text "), "“short text”");
        assert_eq!(
            quoted("The program prints a bare hello from src/main.rs"),
            "“The program prints a bar…”"
        );
    }
}
