//! The colors of the terminal: the sixteen programs name, the cube and
//! the grays above them, and the terminal's own, brighter than the
//! window's as a terminal's text is; and what a program's color names,
//! once it has set some of its own.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui::{Hsla, rgb};

use crate::ui::theme::BACKGROUND;

/// Brighter than the window's, as a terminal's text is.
pub(super) const FOREGROUND: u32 = 0xe0_e0e0;
pub(super) const CURSOR: u32 = 0xc0_c0c0;
/// The sixteen colors programs name: black, red, green, yellow, blue,
/// magenta, cyan and white, then the bright of each.
const PALETTE: [u32; 16] = [
    0x3a_3a3a, 0xe0_6c75, 0x98_c379, 0xe5_c07b, 0x61_afef, 0xc6_78dd, 0x56_b6c2, 0xc8_c8c8,
    0x6a_6a6a, 0xff_7b86, 0xb5_e890, 0xff_d68a, 0x7c_c4ff, 0xde_8ef5, 0x6f_d3df, 0xff_ffff,
];

/// The color at `index` of the terminal's: the program's own if it set
/// one, or Cox's.
pub(super) fn color_at(index: usize, colors: &Colors) -> Rgb {
    if let Some(color) = colors[index] {
        return color;
    }
    let named = |color: u32| {
        let [_, r, g, b] = color.to_be_bytes();
        Rgb { r, g, b }
    };
    match index {
        0..16 => named(PALETTE[index]),
        16..232 => {
            let level = |value: usize| [0, 95, 135, 175, 215, 255][value % 6];
            let index = index - 16;
            Rgb {
                r: level(index / 36),
                g: level(index / 6),
                b: level(index),
            }
        }
        232..256 => {
            let gray = 8 + 10 * u8::try_from(index - 232).unwrap_or(0);
            Rgb {
                r: gray,
                g: gray,
                b: gray,
            }
        }
        _ => match index {
            i if i == NamedColor::Background as usize => named(BACKGROUND),
            i if i == NamedColor::Cursor as usize => named(CURSOR),
            i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
                dim(color_at(i - NamedColor::DimBlack as usize, colors))
            }
            i if i == NamedColor::DimForeground as usize => dim(named(FOREGROUND)),
            _ => named(FOREGROUND),
        },
    }
}

pub(super) fn dim(color: Rgb) -> Rgb {
    let dim = |value: u8| u8::try_from(u16::from(value) * 2 / 3).unwrap_or(value);
    Rgb {
        r: dim(color.r),
        g: dim(color.g),
        b: dim(color.b),
    }
}

/// The color `color` names, as the terminal's colors are now.
pub(super) fn resolve(color: Color, colors: &Colors) -> Rgb {
    match color {
        Color::Spec(color) => color,
        Color::Named(name) => color_at(name as usize, colors),
        Color::Indexed(index) => color_at(usize::from(index), colors),
    }
}

pub(super) fn hsla(color: Rgb) -> Hsla {
    rgb(u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_come_from_the_palette_the_cube_and_the_grays() {
        let colors = Colors::default();
        assert_eq!(
            color_at(1, &colors),
            Rgb {
                r: 0xe0,
                g: 0x6c,
                b: 0x75
            }
        );
        assert_eq!(
            color_at(16 + 36 * 5 + 6 * 2 + 1, &colors),
            Rgb {
                r: 255,
                g: 135,
                b: 95
            }
        );
        assert_eq!(
            color_at(255, &colors),
            Rgb {
                r: 238,
                g: 238,
                b: 238
            }
        );
        assert_eq!(
            color_at(NamedColor::Background as usize, &colors),
            Rgb {
                r: 0x1e,
                g: 0x1e,
                b: 0x1e
            }
        );
    }
}
