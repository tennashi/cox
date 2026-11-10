//! Mermaid diagrams, as a reply or a file of Markdown has them in a code
//! block whose language is `mermaid`: drawn by mermaid-rs-renderer, in
//! the window's colors, as an SVG, which the phone's page shows as it is
//! (see [`svg`]), and as a picture, which the window draws (see
//! [`picture`]).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use anyhow::Context as _;
use gpui::RenderImage;
use mermaid_rs_renderer::{RenderOptions, Theme};
use resvg::{tiny_skia, usvg};

use super::theme::{
    BORDER_STRONG, BUTTON, CHAT_COLORS, DIM, FAINT, FOREGROUND, GROUND, HOVER, RAISED, SELECTED,
    SURFACE,
};

/// How many pixels the window's picture of a diagram has to each of its
/// points: enough for a screen of any density.
pub const SCALE: f32 = 2.;

/// How many diagrams' pictures the window keeps, so as not to draw them
/// again in every frame.
const KEPT: usize = 64;

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

/// A picture of the diagram `source`, for the window to draw at half its
/// size in pixels, or `None` when it is no diagram drawn. The pictures of
/// the last diagrams asked for are kept.
#[must_use]
pub fn picture(source: &str) -> Option<Arc<RenderImage>> {
    static PICTURES: LazyLock<Mutex<HashMap<String, Option<Arc<RenderImage>>>>> =
        LazyLock::new(Mutex::default);
    let mut pictures = PICTURES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(picture) = pictures.get(source) {
        return picture.clone();
    }
    let picture = svg(source)
        .and_then(|svg| draw(&svg))
        .inspect_err(|error| tracing::debug!("a diagram is not drawn: {error:#}"))
        .ok();
    if pictures.len() >= KEPT {
        pictures.clear();
    }
    pictures.insert(source.to_owned(), picture.clone());
    picture
}

/// The SVG `svg` drawn at [`SCALE`], on the ground of a code block.
fn draw(svg: &str) -> anyhow::Result<Arc<RenderImage>> {
    static FONT_DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        Arc::new(fonts)
    });
    let options = usvg::Options {
        fontdb: Arc::clone(&FONT_DB),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).context("the SVG cannot be read")?;
    let size = tree
        .size()
        .to_int_size()
        .scale_by(SCALE)
        .context("no size")?;
    let mut pixmap =
        tiny_skia::Pixmap::new(size.width(), size.height()).context("a picture of no size")?;
    let [_, red, green, blue] = RAISED.to_be_bytes();
    pixmap.fill(tiny_skia::Color::from_rgba8(red, green, blue, 255));
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    // On an opaque ground, no color is premultiplied by its alpha; GPUI
    // takes them as BGRA.
    let mut pixels = pixmap.take();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(size.width(), size.height(), pixels)
        .context("as many pixels as the picture's size")?;
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
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
        assert!(picture("flowchart LR\n  A[Start] --> B[End]").is_some());
    }

    #[test]
    fn draws_nothing_of_what_is_no_diagram() {
        assert!(picture("not a diagram at all").is_none());
    }
}
