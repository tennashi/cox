//! Mermaid diagrams, as a reply or a file of Markdown has them in a code
//! block whose language is `mermaid`: drawn by mermaid-rs-renderer, in
//! the window's colors, as an SVG (see [`svg`]), which the phone's page
//! shows as it is, and the window draws as a picture.

use mermaid_rs_renderer::{RenderOptions, Theme};

use crate::theme::{
    BORDER_STRONG, BUTTON, CHAT_COLORS, DIM, FAINT, FOREGROUND, GROUND, HOVER, RAISED, SELECTED,
    SURFACE,
};

/// The fonts the diagrams' words are in: fonts both macOS and a phone's
/// browser have, so that the words fit the boxes measured for them on
/// the Mac, then fonts most other systems have.
const FONTS: &str = "'Helvetica Neue', 'Hiragino Sans', 'Noto Sans CJK JP', 'DejaVu Sans', \
    sans-serif";

/// The diagram `source` as an SVG, in the window's colors.
///
/// # Errors
///
/// When `source` is no diagram mermaid-rs-renderer reads.
pub fn svg(source: &str) -> anyhow::Result<String> {
    let options = RenderOptions {
        theme: theme(),
        ..RenderOptions::default()
    };
    mermaid_rs_renderer::render_with_options(source, options)
}

/// Mermaid's dark theme, in the window's colors: the boxes as buttons at
/// rest, on the ground of a code block, their words as the window's text.
fn theme() -> Theme {
    let hex = |color: u32| format!("#{color:06x}");
    let mut pie_colors = Theme::dark().pie_colors;
    for (pie, color) in pie_colors.iter_mut().zip(CHAT_COLORS) {
        *pie = hex(color);
    }
    Theme {
        font_family: FONTS.to_owned(),
        font_size: 14.,
        primary_color: hex(BUTTON),
        primary_text_color: hex(FOREGROUND),
        primary_border_color: hex(FAINT),
        line_color: hex(DIM),
        secondary_color: hex(SELECTED),
        tertiary_color: hex(HOVER),
        edge_label_background: hex(RAISED),
        cluster_background: hex(SURFACE),
        cluster_border: hex(BORDER_STRONG),
        background: hex(RAISED),
        sequence_actor_fill: hex(BUTTON),
        sequence_actor_border: hex(FAINT),
        sequence_actor_line: hex(DIM),
        sequence_note_fill: hex(SELECTED),
        sequence_note_border: hex(BORDER_STRONG),
        sequence_activation_fill: hex(SELECTED),
        sequence_activation_border: hex(FAINT),
        text_color: hex(FOREGROUND),
        pie_colors,
        pie_title_text_color: hex(FOREGROUND),
        pie_section_text_color: hex(GROUND),
        pie_legend_text_color: hex(FOREGROUND),
        pie_stroke_color: hex(RAISED),
        pie_outer_stroke_color: hex(BORDER_STRONG),
        pie_opacity: 1.,
        ..Theme::dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_flowchart_in_the_windows_colors() {
        let svg = svg("flowchart LR\n  A[Start] --> B[End]").expect("a diagram");

        assert!(svg.starts_with("<svg"), "{svg}");
        assert!(svg.contains("Start") && svg.contains("End"), "{svg}");
        assert!(svg.contains(&format!("#{BUTTON:06x}")), "{svg}");
    }
}
