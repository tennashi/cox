//! The human's input as CEF takes it: the mouse and key events the
//! stage's say become, with the key codes CEF reads on each system for
//! GPUI's names of the keys.

use cef::sys::{cef_event_flags_t, cef_key_event_type_t, cef_mouse_button_type_t};
use cef::{KeyEvent, KeyEventType, MouseButtonType, MouseEvent};

use super::{Modifiers, MouseButton};

pub(super) fn mouse_event(x: i32, y: i32, modifiers: u32) -> MouseEvent {
    MouseEvent { x, y, modifiers }
}

/// Modifier bits CEF expects on mouse and key events.
pub(super) fn event_flags(modifiers: Modifiers) -> u32 {
    let mut flags = 0;
    if modifiers.shift {
        flags |= cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0;
    }
    if modifiers.control {
        flags |= cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0;
    }
    if modifiers.alt {
        flags |= cef_event_flags_t::EVENTFLAG_ALT_DOWN.0;
    }
    if modifiers.command {
        flags |= cef_event_flags_t::EVENTFLAG_COMMAND_DOWN.0;
    }
    flags
}

pub(super) fn button_flag(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => cef_event_flags_t::EVENTFLAG_LEFT_MOUSE_BUTTON.0,
        MouseButton::Middle => cef_event_flags_t::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0,
        MouseButton::Right => cef_event_flags_t::EVENTFLAG_RIGHT_MOUSE_BUTTON.0,
    }
}

pub(super) fn button_type(button: MouseButton) -> MouseButtonType {
    MouseButtonType::from(match button {
        MouseButton::Left => cef_mouse_button_type_t::MBT_LEFT,
        MouseButton::Middle => cef_mouse_button_type_t::MBT_MIDDLE,
        MouseButton::Right => cef_mouse_button_type_t::MBT_RIGHT,
    })
}

/// Windows virtual key code for a GPUI key name, which CEF reads on Linux.
/// GPUI names a key by what it types without Shift, on the US layout for
/// the symbols. Keys without a code here still type, since their
/// character arrives through [`Browser::commit`].
fn virtual_key(key: &str) -> i32 {
    match key {
        "backspace" => 0x08,
        "tab" => 0x09,
        "enter" => 0x0D,
        "shift" => 0x10,
        "control" => 0x11,
        "alt" => 0x12,
        "capslock" => 0x14,
        "escape" => 0x1B,
        "space" => 0x20,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "insert" => 0x2D,
        "delete" => 0x2E,
        "platform" => 0x5B,
        ";" => 0xBA,
        "=" => 0xBB,
        "," => 0xBC,
        "-" => 0xBD,
        "." => 0xBE,
        "/" => 0xBF,
        "`" => 0xC0,
        "[" => 0xDB,
        "\\" => 0xDC,
        "]" => 0xDD,
        "'" => 0xDE,
        key => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => {
                    i32::from(c.to_ascii_uppercase() as u8)
                }
                // f1 to f24.
                (Some('f'), Some(_)) => key[1..]
                    .parse::<i32>()
                    .ok()
                    .filter(|n| (1..=24).contains(n))
                    .map_or(0, |n| 0x6F + n),
                _ => 0,
            }
        }
    }
}

/// The Mac's key code (`kVK_...`) for a GPUI key name. CEF on macOS makes
/// an `NSEvent` of a key event and reads the key from this, not from
/// `windows_key_code`, and 0 is the A key, so keys without a code here
/// get one no key has. The letters and symbols are where the US layout
/// has them.
#[cfg(target_os = "macos")]
fn native_key(key: &str) -> i32 {
    match key {
        "enter" => 0x24,
        "tab" => 0x30,
        "space" => 0x31,
        "backspace" => 0x33,
        "escape" => 0x35,
        "platform" => 0x37,
        "shift" => 0x38,
        "capslock" => 0x39,
        "alt" => 0x3A,
        "control" => 0x3B,
        "home" => 0x73,
        "pageup" => 0x74,
        "delete" => 0x75,
        "end" => 0x77,
        "pagedown" => 0x79,
        "left" => 0x7B,
        "right" => 0x7C,
        "down" => 0x7D,
        "up" => 0x7E,
        "f1" => 0x7A,
        "f2" => 0x78,
        "f3" => 0x63,
        "f4" => 0x76,
        "f5" => 0x60,
        "f6" => 0x61,
        "f7" => 0x62,
        "f8" => 0x64,
        "f9" => 0x65,
        "f10" => 0x6D,
        "f11" => 0x67,
        "f12" => 0x6F,
        "a" => 0x00,
        "s" => 0x01,
        "d" => 0x02,
        "f" => 0x03,
        "h" => 0x04,
        "g" => 0x05,
        "z" => 0x06,
        "x" => 0x07,
        "c" => 0x08,
        "v" => 0x09,
        "b" => 0x0B,
        "q" => 0x0C,
        "w" => 0x0D,
        "e" => 0x0E,
        "r" => 0x0F,
        "y" => 0x10,
        "t" => 0x11,
        "1" => 0x12,
        "2" => 0x13,
        "3" => 0x14,
        "4" => 0x15,
        "6" => 0x16,
        "5" => 0x17,
        "=" => 0x18,
        "9" => 0x19,
        "7" => 0x1A,
        "-" => 0x1B,
        "8" => 0x1C,
        "0" => 0x1D,
        "]" => 0x1E,
        "o" => 0x1F,
        "u" => 0x20,
        "[" => 0x21,
        "i" => 0x22,
        "p" => 0x23,
        "l" => 0x25,
        "j" => 0x26,
        "'" => 0x27,
        "k" => 0x28,
        ";" => 0x29,
        "\\" => 0x2A,
        "," => 0x2B,
        "/" => 0x2C,
        "n" => 0x2D,
        "m" => 0x2E,
        "." => 0x2F,
        "`" => 0x32,
        _ => 0xFFFF,
    }
}

#[cfg(target_os = "linux")]
fn native_key(_key: &str) -> i32 {
    0
}

/// The character the `NSEvent` of a key has, which CEF on macOS also makes
/// the key of: Enter is "\r", Backspace is DEL, and the keys that type
/// nothing have characters of their own (`NSUpArrowFunctionKey`, ...).
/// Keys that type are their own character.
#[cfg(target_os = "macos")]
pub(super) fn key_character(key: &str) -> u16 {
    match key {
        "enter" => 0x0D,
        "tab" => 0x09,
        "space" => 0x20,
        "backspace" => 0x7F,
        "escape" => 0x1B,
        "up" => 0xF700,
        "down" => 0xF701,
        "left" => 0xF702,
        "right" => 0xF703,
        "delete" => 0xF728,
        "home" => 0xF729,
        "end" => 0xF72B,
        "pageup" => 0xF72C,
        "pagedown" => 0xF72D,
        key => {
            let mut units = key.encode_utf16();
            match (units.next(), units.next()) {
                (Some(unit), None) => unit,
                // f1 to f12 (`NSF1FunctionKey`, ...).
                _ => key
                    .strip_prefix('f')
                    .and_then(|n| n.parse::<u16>().ok())
                    .filter(|n| (1..=12).contains(n))
                    .map_or(0, |n| 0xF703 + n),
            }
        }
    }
}

/// CEF on Linux needs no character for a key going down or up.
#[cfg(target_os = "linux")]
pub(super) fn key_character(_key: &str) -> u16 {
    0
}

#[derive(Clone, Copy)]
pub(super) enum KeyKind {
    Down,
    Up,
    Char,
}

pub(super) fn key_event(kind: KeyKind, key: &str, modifiers: u32, character: u16) -> KeyEvent {
    let type_ = match kind {
        KeyKind::Down => cef_key_event_type_t::KEYEVENT_RAWKEYDOWN,
        KeyKind::Up => cef_key_event_type_t::KEYEVENT_KEYUP,
        KeyKind::Char => cef_key_event_type_t::KEYEVENT_CHAR,
    };
    KeyEvent {
        type_: KeyEventType::from(type_),
        modifiers,
        windows_key_code: virtual_key(key),
        native_key_code: native_key(key),
        is_system_key: 0,
        character,
        unmodified_character: character,
        focus_on_editable_field: 0,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::virtual_key;

    #[test]
    fn keys_have_their_windows_codes() {
        assert_eq!(virtual_key("a"), 0x41);
        assert_eq!(virtual_key("7"), 0x37);
        assert_eq!(virtual_key("enter"), 0x0D);
        assert_eq!(virtual_key("-"), 0xBD);
        assert_eq!(virtual_key("f1"), 0x70);
        assert_eq!(virtual_key("f12"), 0x7B);
    }

    #[test]
    fn unknown_keys_have_no_code() {
        assert_eq!(virtual_key("f0"), 0);
        assert_eq!(virtual_key("fn"), 0);
        assert_eq!(virtual_key("ä"), 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn keys_have_their_mac_codes_and_characters() {
        use super::{key_character, native_key};
        assert_eq!(native_key("a"), 0x00);
        assert_eq!(native_key("enter"), 0x24);
        assert_eq!(native_key("backspace"), 0x33);
        assert_eq!(native_key("ä"), 0xFFFF);
        assert_eq!(key_character("enter"), 0x0D);
        assert_eq!(key_character("backspace"), 0x7F);
        assert_eq!(key_character("a"), u16::from(b'a'));
        assert_eq!(key_character("f1"), 0xF704);
        assert_eq!(key_character("f12"), 0xF70F);
    }
}
