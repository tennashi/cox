//! How the window looks, where its parts look alike: the font, and the
//! colors of what most of them show. A part keeps the colors of what
//! only it shows.

/// The font the window is drawn in: a monospaced family the system has.
/// A generic name such as `monospace` finds no font, and GPUI then falls
/// back to a font with no bold.
#[cfg(target_os = "macos")]
pub const FONT: &str = "Menlo";
#[cfg(not(target_os = "macos"))]
pub const FONT: &str = "DejaVu Sans Mono";

/// Text, and the ground it is on.
pub const FOREGROUND: u32 = 0xd4_d4d4;
pub const BACKGROUND: u32 = 0x1e_1e1e;
/// Text that says less: a path, a note, a count.
pub const DIM: u32 = 0xa0_a0a0;
/// What went wrong.
pub const FAILED: u32 = 0xf0_8080;
/// What waits for the human to answer.
pub const ASKING: u32 = 0xd0_a040;
/// The ground of the text the human selected.
pub const SELECTION: u32 = 0x3a_4a6a;
/// A link.
pub const LINK: u32 = 0x7f_b4ff;
