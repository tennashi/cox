//! The human's keys over the page: the browser's own shortcuts, which
//! Chromium with a window of its own gets from the window's menu and the
//! page has to supply, and the rest, which goes to the page as it is.

use gpui::{Context, KeyDownEvent, KeyUpEvent, Keystroke, Window};

use super::Page;
use cox_browser::{EditCommand, Modifiers};

impl Page {
    /// A key went down over the page. The browser's own shortcuts come
    /// first; the rest goes to the page.
    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        if let Some(shortcut) = shortcut(keystroke) {
            match shortcut {
                Shortcut::Edit(command) => self.browser.edit(command),
                Shortcut::Reload => self.browser.reload(),
                Shortcut::Address => {
                    self.address
                        .update(cx, |address, cx| address.focus(window, cx));
                }
            }
            cx.stop_propagation();
            return;
        }
        self.browser
            .key_down(&keystroke.key, modifiers(keystroke.modifiers));
    }

    pub(super) fn key_up(
        &mut self,
        event: &KeyUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        self.browser
            .key_up(&keystroke.key, modifiers(keystroke.modifiers));
    }
}

pub(super) fn modifiers(modifiers: gpui::Modifiers) -> Modifiers {
    Modifiers {
        shift: modifiers.shift,
        control: modifiers.control,
        alt: modifiers.alt,
        command: cfg!(target_os = "macos") && modifiers.platform,
    }
}

/// What the stage does with a keystroke itself, instead of handing it to
/// the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shortcut {
    Edit(EditCommand),
    Reload,
    /// Go to the address line.
    Address,
}

/// The browser's shortcuts: Command on macOS, Control elsewhere, with a
/// letter. Chromium with a window of its own gets these from the window's
/// menu; the stage has to supply them.
fn shortcut(keystroke: &Keystroke) -> Option<Shortcut> {
    let modifiers = keystroke.modifiers;
    if !modifiers.secondary() || modifiers.alt || modifiers.function {
        return None;
    }
    let shortcut = match (keystroke.key.as_str(), modifiers.shift) {
        ("z", false) => Shortcut::Edit(EditCommand::Undo),
        ("z", true) | ("y", false) => Shortcut::Edit(EditCommand::Redo),
        ("x", false) => Shortcut::Edit(EditCommand::Cut),
        ("c", false) => Shortcut::Edit(EditCommand::Copy),
        ("v", false) => Shortcut::Edit(EditCommand::Paste),
        ("a", false) => Shortcut::Edit(EditCommand::SelectAll),
        ("r", false) => Shortcut::Reload,
        ("l", false) => Shortcut::Address,
        _ => return None,
    };
    Some(shortcut)
}

#[cfg(test)]
mod tests {
    use gpui::Keystroke;

    use super::{EditCommand, Shortcut, shortcut};

    fn parse(keystroke: &str) -> Keystroke {
        Keystroke::parse(keystroke).expect("a keystroke GPUI reads")
    }

    #[test]
    fn the_secondary_key_with_a_letter_is_a_shortcut() {
        let secondary = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        assert_eq!(
            shortcut(&parse(&format!("{secondary}-c"))),
            Some(Shortcut::Edit(EditCommand::Copy))
        );
        assert_eq!(
            shortcut(&parse(&format!("{secondary}-shift-z"))),
            Some(Shortcut::Edit(EditCommand::Redo))
        );
        assert_eq!(
            shortcut(&parse(&format!("{secondary}-l"))),
            Some(Shortcut::Address)
        );
    }

    #[test]
    fn other_keys_go_to_the_page() {
        assert_eq!(shortcut(&parse("c")), None);
        assert_eq!(shortcut(&parse("shift-c")), None);
        assert_eq!(shortcut(&parse("alt-c")), None);
        let secondary = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        assert_eq!(shortcut(&parse(&format!("{secondary}-k"))), None);
    }
}
