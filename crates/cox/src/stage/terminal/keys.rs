//! What the program gets for the human's keys, and for what they paste:
//! the bytes a terminal sends for a key, as xterm does, which depend on
//! the modifiers and on the mode the program put the terminal in. A key
//! that types text sends nothing here: its text reaches the tab as text.

use std::borrow::Cow;

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

/// What the program gets for the key `keystroke`, in a terminal in
/// `mode`, or `None` for a key that types text, which reaches the tab as
/// text, and for the window's own shortcuts.
pub(super) fn key_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Cow<'static, [u8]>> {
    let modifiers = keystroke.modifiers;
    if modifiers.platform || modifiers.function {
        return None;
    }
    // As xterm adds them to a key's sequence: 1, plus Shift 1, Alt 2,
    // Control 4.
    let added = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let cursor = |key: char| -> Cow<'static, [u8]> {
        if added > 1 {
            format!("\x1b[1;{added}{key}").into_bytes().into()
        } else if mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{key}").into_bytes().into()
        } else {
            format!("\x1b[{key}").into_bytes().into()
        }
    };
    let function = |number: u8| -> Cow<'static, [u8]> {
        if added > 1 {
            format!("\x1b[{number};{added}~").into_bytes().into()
        } else {
            format!("\x1b[{number}~").into_bytes().into()
        }
    };
    let fixed = |bytes: &'static [u8]| Some(Cow::Borrowed(bytes));
    match keystroke.key.as_str() {
        "up" => Some(cursor('A')),
        "down" => Some(cursor('B')),
        "right" => Some(cursor('C')),
        "left" => Some(cursor('D')),
        "home" => Some(cursor('H')),
        "end" => Some(cursor('F')),
        "f1" => Some(cursor('P')),
        "f2" => Some(cursor('Q')),
        "f3" => Some(cursor('R')),
        "f4" => Some(cursor('S')),
        "insert" => Some(function(2)),
        "delete" => Some(function(3)),
        "pageup" => Some(function(5)),
        "pagedown" => Some(function(6)),
        "f5" => Some(function(15)),
        "f6" => Some(function(17)),
        "f7" => Some(function(18)),
        "f8" => Some(function(19)),
        "f9" => Some(function(20)),
        "f10" => Some(function(21)),
        "f11" => Some(function(23)),
        "f12" => Some(function(24)),
        "enter" if modifiers.alt => fixed(b"\x1b\r"),
        "enter" => fixed(b"\r"),
        "tab" if modifiers.shift => fixed(b"\x1b[Z"),
        "tab" if modifiers.control => None,
        "tab" => fixed(b"\t"),
        "escape" => fixed(b"\x1b"),
        "backspace" if modifiers.control => fixed(b"\x08"),
        "backspace" if modifiers.alt => fixed(b"\x1b\x7f"),
        "backspace" => fixed(b"\x7f"),
        "space" if modifiers.control => fixed(b"\0"),
        key if modifiers.control => {
            let mut chars = key.chars();
            let (Some(key), None) = (chars.next(), chars.next()) else {
                return None;
            };
            let byte = control(key)?;
            Some(if modifiers.alt {
                vec![0x1b, byte].into()
            } else {
                vec![byte].into()
            })
        }
        // Option types characters on macOS; elsewhere Alt is the Meta key
        // that shells read words with.
        _ if modifiers.alt && !cfg!(target_os = "macos") => {
            let typed = keystroke.key_char.as_deref().unwrap_or(&keystroke.key);
            Some(format!("\x1b{typed}").into_bytes().into())
        }
        _ => None,
    }
}

/// The byte Control with `key` makes.
fn control(key: char) -> Option<u8> {
    let key = key.to_ascii_lowercase();
    Some(match key {
        'a'..='z' => key as u8 - b'a' + 1,
        '@' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// What a paste of `text` sends: between the marks that tell a program it
/// is pasted, for one that asked for them, or with the line breaks as
/// Enter.
pub(super) fn pasted(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // The text must not end the paste early.
        let text = text.replace('\x1b', "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(keystroke: &str, mode: TermMode) -> Option<Vec<u8>> {
        let keystroke = Keystroke::parse(keystroke).expect("a keystroke GPUI reads");
        key_bytes(&keystroke, mode).map(Cow::into_owned)
    }

    #[test]
    fn keys_that_type_text_reach_the_tab_as_text() {
        assert_eq!(keys("a", TermMode::NONE), None);
        assert_eq!(keys("shift-a", TermMode::NONE), None);
        assert_eq!(keys("cmd-c", TermMode::NONE), None);
    }

    #[test]
    fn control_with_a_letter_is_its_control_character() {
        assert_eq!(keys("ctrl-c", TermMode::NONE), Some(b"\x03".to_vec()));
        assert_eq!(keys("ctrl-d", TermMode::NONE), Some(b"\x04".to_vec()));
        assert_eq!(keys("ctrl-[", TermMode::NONE), Some(b"\x1b".to_vec()));
        assert_eq!(keys("ctrl-space", TermMode::NONE), Some(b"\0".to_vec()));
    }

    #[test]
    fn keys_that_type_nothing_send_their_sequences() {
        assert_eq!(keys("enter", TermMode::NONE), Some(b"\r".to_vec()));
        assert_eq!(keys("backspace", TermMode::NONE), Some(b"\x7f".to_vec()));
        assert_eq!(keys("tab", TermMode::NONE), Some(b"\t".to_vec()));
        assert_eq!(keys("shift-tab", TermMode::NONE), Some(b"\x1b[Z".to_vec()));
        assert_eq!(keys("escape", TermMode::NONE), Some(b"\x1b".to_vec()));
        assert_eq!(keys("delete", TermMode::NONE), Some(b"\x1b[3~".to_vec()));
        assert_eq!(keys("f5", TermMode::NONE), Some(b"\x1b[15~".to_vec()));
    }

    #[test]
    fn the_cursor_keys_follow_the_programs_mode_and_the_modifiers() {
        assert_eq!(keys("up", TermMode::NONE), Some(b"\x1b[A".to_vec()));
        assert_eq!(keys("up", TermMode::APP_CURSOR), Some(b"\x1bOA".to_vec()));
        assert_eq!(
            keys("ctrl-left", TermMode::NONE),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            keys("shift-end", TermMode::APP_CURSOR),
            Some(b"\x1b[1;2F".to_vec())
        );
    }

    #[test]
    fn a_paste_is_marked_for_a_program_that_asks() {
        assert_eq!(pasted("a\nb", false), b"a\rb");
        assert_eq!(pasted("a\r\nb", false), b"a\rb");
        assert_eq!(pasted("a\nb\x1b", true), b"\x1b[200~a\nb\x1b[201~");
    }
}
