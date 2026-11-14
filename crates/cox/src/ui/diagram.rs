//! A Mermaid diagram as a picture, which the window draws: the SVG
//! [`coxd::diagram::svg`] makes, drawn at [`SCALE`].

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use anyhow::Context as _;
use gpui::RenderImage;
use resvg::{tiny_skia, usvg};

use coxd::theme::RAISED;

/// How many pixels the window's picture of a diagram has to each of its
/// points: enough for a screen of any density.
pub const SCALE: f32 = 2.;

/// How many diagrams' pictures the window keeps, so as not to draw them
/// again in every frame.
const KEPT: usize = 64;

/// A picture of the diagram `source`, for the window to draw at half its
/// size in pixels, or `None` when it is no diagram drawn. The pictures of
/// the last diagrams asked for are kept.
#[must_use]
pub fn of(source: &str) -> Option<Arc<RenderImage>> {
    static PICTURES: LazyLock<Mutex<HashMap<String, Option<Arc<RenderImage>>>>> =
        LazyLock::new(Mutex::default);
    let mut pictures = PICTURES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(picture) = pictures.get(source) {
        return picture.clone();
    }
    let picture = coxd::diagram::svg(source)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_flowchart() {
        assert!(of("flowchart LR\n  A[Start] --> B[End]").is_some());
    }

    #[test]
    fn draws_nothing_of_what_is_no_diagram() {
        assert!(of("not a diagram at all").is_none());
    }
}
