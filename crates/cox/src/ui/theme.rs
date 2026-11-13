//! How the window looks, where its parts look alike: the fonts, and the
//! colors of what most of them show. A part keeps the colors of what
//! only it shows, such as the code's syntax or the terminal's palette.
//!
//! The colors go from the window's back up to what sits on it, then the
//! text, then the few colors that say something: one accent, which marks
//! the agent and what it changed, and what asks or fails. Those three are
//! as light and as vivid as each other, so that none of them stands out
//! of the rest but by what it means.

/// The font the window's words are drawn in: the system's own on macOS.
/// A generic name such as `sans-serif` finds no font, and GPUI then falls
/// back to a font with no bold, so off macOS it is a family most systems
/// have.
#[cfg(target_os = "macos")]
pub const UI_FONT: &str = ".SystemUIFont";
#[cfg(not(target_os = "macos"))]
pub const UI_FONT: &str = "DejaVu Sans";

/// The font code is drawn in, where its columns line up: a file, a
/// terminal, a change, a code block.
#[cfg(target_os = "macos")]
pub const CODE_FONT: &str = "Menlo";
#[cfg(not(target_os = "macos"))]
pub const CODE_FONT: &str = "DejaVu Sans Mono";

/// How far apart the window's cards sit, and from its edges.
pub const GAP: f32 = 10.;
/// How round a card's corners are: the stage's, and the conversation's.
pub const CARD_RADIUS: f32 = 18.;
/// How round the corners of what fills a card are, inside its border.
pub const CARD_INNER_RADIUS: f32 = CARD_RADIUS - 1.;

/// The window's back, which the list of chats and the cards are on.
pub const GROUND: u32 = 0x0e_0e0f;
/// The ground of a card: a chat's conversation and its stage.
pub const SURFACE: u32 = 0x15_1517;
/// What sits a step above the surface: a menu, a code block, the human's
/// lines.
pub const RAISED: u32 = 0x1f_1f23;
/// A row or a button under the mouse.
pub const HOVER: u32 = 0x22_2226;
/// A button at rest.
pub const BUTTON: u32 = 0x26_262b;
/// A button under the mouse.
pub const BUTTON_HOVER: u32 = 0x30_3036;
/// What is chosen: the tab shown, the chat shown, a menu open.
pub const SELECTED: u32 = 0x2a_2a2f;
/// A line between parts, and around a box.
pub const BORDER: u32 = 0x2c_2c31;
/// The edge of what floats over the rest, such as a menu.
pub const BORDER_STRONG: u32 = 0x36_363b;
/// The edge of the card that has the keyboard.
pub const FOCUSED_EDGE: u32 = 0x5a_5a61;

/// Text.
pub const FOREGROUND: u32 = 0xe4_e4e6;
/// Text that says less: a path, a note, a count.
pub const DIM: u32 = 0xa5_a5ab;
/// Text that says less still: a button's name at rest, a closed chat.
pub const FAINT: u32 = 0x8e_8e95;
/// What cannot be used now, and the numbers of a file's lines.
pub const DISABLED: u32 = 0x5f_5f66;

/// The accent: the agent, what it changed, and what went well.
pub const ACCENT: u32 = 0x6f_d3c5;
/// The ground of what the accent marks, such as the lines a tab is at.
pub const ACCENT_GROUND: u32 = 0x15_2c2a;
/// A link.
pub const LINK: u32 = 0xa6_ebe1;
/// Code among words, as `a path` in the conversation, and the stretch of
/// a line a search found.
pub const CODE: u32 = 0xe0_b080;
/// The colors that tell the chats apart: on the tabs of the stage its
/// agent works on, on its dot in the list of chats, and on its agent's
/// lines in the conversation. They are of the accent's lightness and
/// saturation, around the wheel but for the amber and the red of what
/// asks and what went wrong, in an order that sets each far from the one
/// before, the accent first.
pub const CHAT_COLORS: [u32; 8] = [
    0x6f_d3c6, 0xba_6fd3, 0xa1_d36f, 0x6f_88d3, 0xd3_6fba, 0x6f_d388, 0x88_6fd3, 0x6f_b2d3,
];

/// The color of the chat `chat`, by its key, the first chat's the accent.
#[must_use]
pub fn chat_color(chat: u64) -> u32 {
    // The keys count from 1.
    let place = chat.wrapping_sub(1) % CHAT_COLORS.len() as u64;
    CHAT_COLORS[usize::try_from(place).unwrap_or_default()]
}
/// What waits for the human to answer.
pub const ASKING: u32 = 0xe5_b46d;
/// The ground of what asks the human, its edge, and its buttons but the
/// first, which the human most likely answers and which is `ASKING`.
pub const ASKING_GROUND: u32 = 0x1e_1b15;
pub const ASKING_EDGE: u32 = 0x4a_3d25;
pub const ASKING_BUTTON: u32 = 0x2e_2920;
/// The ground of what Cox or the agent offers the human, as a goal to
/// set, its edge, and its buttons but the first, which is `ACCENT`: as
/// what asks them is drawn, in the accent rather than amber.
pub const OFFERING_GROUND: u32 = 0x15_1f1e;
pub const OFFERING_EDGE: u32 = 0x26_4a45;
pub const OFFERING_BUTTON: u32 = 0x1c_2e2b;
/// What went wrong.
pub const FAILED: u32 = 0xec_8580;
/// The ground of what went wrong, and the mark on it.
pub const FAILED_GROUND: u32 = 0x2a_1a1a;
pub const FAILED_MARK: u32 = 0xde_6b66;
/// The ground of lines a change adds, and of those it removes.
pub const ADDED_GROUND: u32 = 0x14_2c29;
pub const REMOVED_GROUND: u32 = 0x2c_1b1c;
/// The ground of the words a change put in a line, and of those it took
/// out, over the line's ground.
pub const ADDED_WORD: u32 = 0x1f_4a44;
pub const REMOVED_WORD: u32 = 0x4d_2629;
/// The ground of the text the human selected.
pub const SELECTION: u32 = 0x22_4a45;

/// The colors above, as CSS custom properties on `:root`, named after
/// them, as `--asking-ground`, for the pages of the stage that draw as
/// the window does, as the plugins, which read them from
/// `cox://api/theme.css`.
#[must_use]
pub fn css() -> String {
    use std::fmt::Write as _;

    const COLORS: &[(&str, u32)] = &[
        ("ground", GROUND),
        ("surface", SURFACE),
        ("raised", RAISED),
        ("hover", HOVER),
        ("button", BUTTON),
        ("button-hover", BUTTON_HOVER),
        ("selected", SELECTED),
        ("border", BORDER),
        ("border-strong", BORDER_STRONG),
        ("foreground", FOREGROUND),
        ("dim", DIM),
        ("faint", FAINT),
        ("disabled", DISABLED),
        ("accent", ACCENT),
        ("accent-ground", ACCENT_GROUND),
        ("link", LINK),
        ("code", CODE),
        ("asking", ASKING),
        ("asking-ground", ASKING_GROUND),
        ("asking-edge", ASKING_EDGE),
        ("asking-button", ASKING_BUTTON),
        ("offering-ground", OFFERING_GROUND),
        ("offering-edge", OFFERING_EDGE),
        ("offering-button", OFFERING_BUTTON),
        ("failed", FAILED),
        ("failed-ground", FAILED_GROUND),
        ("failed-mark", FAILED_MARK),
        ("added-ground", ADDED_GROUND),
        ("removed-ground", REMOVED_GROUND),
        ("added-word", ADDED_WORD),
        ("removed-word", REMOVED_WORD),
        ("selection", SELECTION),
    ];
    let mut css = String::from(":root {\n  color-scheme: dark;\n");
    for (name, color) in COLORS {
        let _ = writeln!(css, "  --{name}: #{color:06x};");
    }
    css.push_str("}\n");
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_css_names_each_color_as_the_window_draws_it() {
        let css = css();
        assert!(css.contains("  --ground: #0e0e0f;\n"), "{css}");
        assert!(css.contains("  --asking-ground: #1e1b15;\n"), "{css}");
        assert!(css.starts_with(":root {") && css.ends_with("}\n"), "{css}");
    }
}
