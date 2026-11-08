//! The icons the human gives the projects, which the rail draws in place
//! of a folder: each a PNG of [`SIZE`] pixels square, kept where Cox keeps
//! what lasts (see [`Kept::Icons`]), named after the number the project is
//! kept by, so that the project's own folder stays as it is.
//!
//! The human chooses a picture of their own, which [`make`] crops to a
//! square at its middle and scales, or an SVG, which it draws at that size.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, bail};
use cox_core::store::ProjectId;
use gpui::{Image, ImageFormat};
use image::imageops::FilterType;
use image::{DynamicImage, ImageReader, RgbaImage};

use crate::storage::{self, Kept};

/// How many pixels square an icon is kept at: enough for the rail's
/// square on a screen of any density.
pub const SIZE: u32 = 256;

/// Where the icon of the project kept as `id` is, whether it is there or
/// not, or `None` when Cox cannot tell where it keeps what lasts.
#[must_use]
pub fn path(id: ProjectId) -> Option<PathBuf> {
    Some(storage::path(Kept::Icons)?.join(format!("{id}.png")))
}

/// The icon of the project kept as `id`, for GPUI to draw, if it has one.
#[must_use]
pub fn read(id: ProjectId) -> Option<Arc<Image>> {
    let bytes = std::fs::read(path(id)?).ok()?;
    Some(Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
}

/// Gives the project kept as `id` the picture in the file `from` as its
/// icon, in place of the one it had.
///
/// # Errors
///
/// When the file is no picture Cox reads, or the icon cannot be written.
pub fn set(id: ProjectId, from: &Path) -> anyhow::Result<()> {
    let png = make(from)?;
    let path = path(id).context("Cox cannot tell where it keeps what lasts")?;
    keep(&path, &png)
}

/// Takes away the icon of the project kept as `id`, which the rail then
/// draws as a folder again.
///
/// # Errors
///
/// When the icon is there and cannot be taken away.
pub fn remove(id: ProjectId) -> anyhow::Result<()> {
    let Some(path) = path(id) else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("cannot remove {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Writes `png` to `path`, through a file beside it, so that the rail
/// never reads half of it.
fn keep(path: &Path, png: &[u8]) -> anyhow::Result<()> {
    let folder = path.parent().context("an icon's path has a folder")?;
    std::fs::create_dir_all(folder).with_context(|| format!("cannot make {}", folder.display()))?;
    let partial = path.with_extension("png.partial");
    std::fs::write(&partial, png).with_context(|| format!("cannot write {}", partial.display()))?;
    std::fs::rename(&partial, path).with_context(|| format!("cannot write {}", path.display()))
}

/// The icon, as a PNG of [`SIZE`] pixels square, of the picture in the
/// file `from`: an SVG, drawn to fill the square as far as it keeps its
/// shape, or else a picture of any kind the `image` crate reads, cropped
/// to a square at its middle.
///
/// # Errors
///
/// When the file cannot be read, or is no picture.
pub fn make(from: &Path) -> anyhow::Result<Vec<u8>> {
    let bytes = std::fs::read(from).with_context(|| format!("cannot read {}", from.display()))?;
    let svg = from
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
    let icon = if svg {
        draw_svg(&bytes)?
    } else {
        let picture = ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()?
            .decode()
            .with_context(|| format!("{} is no picture Cox reads", from.display()))?;
        square(&picture)
    };
    let mut png = Vec::new();
    icon.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
    Ok(png)
}

/// `picture` cropped to a square at its middle, and scaled to [`SIZE`].
fn square(picture: &DynamicImage) -> DynamicImage {
    let side = picture.width().min(picture.height());
    let x = (picture.width() - side) / 2;
    let y = (picture.height() - side) / 2;
    picture
        .crop_imm(x, y, side, side)
        .resize_exact(SIZE, SIZE, FilterType::Lanczos3)
}

/// The SVG `bytes` drawn in a square of [`SIZE`], as large as it fits,
/// at its middle.
fn draw_svg(bytes: &[u8]) -> anyhow::Result<DynamicImage> {
    use resvg::{tiny_skia, usvg};

    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_data(bytes, &options).context("the SVG cannot be read")?;
    let size = tree.size();
    #[expect(clippy::cast_precision_loss, reason = "a few hundred pixels")]
    let edge = SIZE as f32;
    let scale = (edge / size.width()).min(edge / size.height());
    if !scale.is_finite() || scale <= 0. {
        bail!("the SVG has no size");
    }
    let mut pixmap = tiny_skia::Pixmap::new(SIZE, SIZE).context("a square of SIZE")?;
    let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(
        size.width().mul_add(-scale, edge) / 2.,
        size.height().mul_add(-scale, edge) / 2.,
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia keeps its colors premultiplied by their alpha; a PNG's are
    // not.
    let pixels = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    let picture = RgbaImage::from_raw(SIZE, SIZE, pixels).context("a square of SIZE")?;
    Ok(DynamicImage::ImageRgba8(picture))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file in a folder of the test's own, named `name`, holding
    /// `bytes`.
    fn file(name: &str, bytes: &[u8]) -> (tempfile_dir::Dir, PathBuf) {
        let dir = tempfile_dir::Dir::new();
        let path = dir.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    /// The PNG `png`, read back.
    fn decoded(png: &[u8]) -> RgbaImage {
        image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8()
    }

    #[test]
    fn crops_a_picture_to_a_square_at_its_middle() {
        // Red at the sides, blue in the middle square.
        let mut wide = RgbaImage::from_pixel(300, 100, image::Rgba([255, 0, 0, 255]));
        for x in 100..200 {
            for y in 0..100 {
                wide.put_pixel(x, y, image::Rgba([0, 0, 255, 255]));
            }
        }
        let mut png = Vec::new();
        wide.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        // The name says nothing of what the file holds.
        let (_dir, path) = file("logo.bin", &png);

        let icon = decoded(&make(&path).unwrap());

        assert_eq!(icon.dimensions(), (SIZE, SIZE));
        assert_eq!(icon.get_pixel(2, SIZE / 2).0, [0, 0, 255, 255]);
        assert_eq!(icon.get_pixel(SIZE - 3, SIZE / 2).0, [0, 0, 255, 255]);
    }

    #[test]
    fn draws_an_svg_at_the_middle_of_the_square() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32" viewBox="0 0 64 32">
            <rect width="64" height="32" fill="#00ff00"/></svg>"##;
        let (_dir, path) = file("icon.svg", svg);

        let icon = decoded(&make(&path).unwrap());

        assert_eq!(icon.dimensions(), (SIZE, SIZE));
        assert_eq!(icon.get_pixel(SIZE / 2, SIZE / 2).0, [0, 255, 0, 255]);
        // Twice as wide as high: clear above and under it.
        assert_eq!(icon.get_pixel(SIZE / 2, 10).0[3], 0);
        assert_eq!(icon.get_pixel(SIZE / 2, SIZE - 10).0[3], 0);
    }

    #[test]
    fn refuses_what_is_no_picture() {
        let (_dir, text) = file("notes.txt", b"not a picture");
        let (_dir2, svg) = file("broken.svg", b"<svg");

        assert!(make(&text).is_err());
        assert!(make(&svg).is_err());
    }

    /// A folder of a test's own, taken away after it.
    mod tempfile_dir {
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};

        pub struct Dir(pub PathBuf);

        impl Dir {
            pub fn new() -> Self {
                static NEXT: AtomicUsize = AtomicUsize::new(0);
                let path = std::env::temp_dir().join(format!(
                    "cox-project-icon-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
