//! The pictures and videos shown in the conversations, as screenshots of
//! what an agent made, and the files an agent showed on the stage, as a
//! report in Markdown: Cox keeps a copy of each, in its state (see
//! [`Kept::Media`]), named after what it holds, so that the conversation
//! shows it as it was, on the Mac and on the phone, whatever happens to
//! the file it was shown from.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use cox_core::entry::MediaKind;
use sha2::{Digest as _, Sha256};

use crate::storage::{self, Kept};

/// The most a picture or a video shown may hold.
const MOST: u64 = 100 * 1024 * 1024;

/// The kinds of file shown, by their extensions, each with what it is and
/// its media type.
const KINDS: &[(&str, MediaKind, &str)] = &[
    ("png", MediaKind::Image, "image/png"),
    ("jpg", MediaKind::Image, "image/jpeg"),
    ("jpeg", MediaKind::Image, "image/jpeg"),
    ("gif", MediaKind::Image, "image/gif"),
    ("webp", MediaKind::Image, "image/webp"),
    ("mp4", MediaKind::Video, "video/mp4"),
    ("m4v", MediaKind::Video, "video/mp4"),
    ("mov", MediaKind::Video, "video/quicktime"),
    ("webm", MediaKind::Video, "video/webm"),
];

/// The kinds of text kept of a file an agent showed, by the extension of
/// the copy, each with its media type: Markdown, and any other text.
const TEXTS: &[(&str, &str)] = &[
    ("md", "text/markdown; charset=utf-8"),
    ("txt", "text/plain; charset=utf-8"),
];

/// The most of a file an agent showed that is kept: past it, the file is
/// shown on the stage alone.
const MOST_TEXT: usize = 2 * 1024 * 1024;

/// A picture or a video kept to show: the name of the copy, and what it
/// is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub file: String,
    pub kind: MediaKind,
    /// Its width and height in pixels, for a picture Cox could read.
    pub size: Option<(u32, u32)>,
}

/// Keeps a copy of the picture or the video at `path` to show in a
/// conversation, in `folder`, or where Cox keeps them, and answers its
/// name and what it is, or why it cannot be shown.
///
/// # Errors
///
/// When the file is of no kind shown, too big, or cannot be read, or the
/// copy cannot be written.
pub fn keep(path: &Path, folder: Option<&Path>) -> Result<Shown, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let Some(&(extension, kind, _)) = KINDS.iter().find(|(known, _, _)| *known == extension) else {
        let known: Vec<&str> = KINDS.iter().map(|(known, _, _)| *known).collect();
        return Err(format!(
            "{} is no picture or video Cox shows: it shows {}",
            path.display(),
            known.join(", ")
        ));
    };
    let file = std::fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MOST + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() as u64 > MOST {
        return Err(format!(
            "{} holds more than {} MB, as much as Cox shows",
            path.display(),
            MOST / 1024 / 1024
        ));
    }
    let (file, copy) = keep_bytes(&bytes, extension, folder)?;
    let size = match kind {
        MediaKind::Image => image::image_dimensions(&copy).ok(),
        MediaKind::Video => None,
    };
    Ok(Shown { file, kind, size })
}

/// Keeps a copy of `text`, a file an agent showed, Markdown if it is
/// `markdown`, in `folder`, or where Cox keeps them, and answers the
/// copy's name.
///
/// # Errors
///
/// When the text is longer than Cox keeps, or the copy cannot be
/// written.
pub fn keep_text(text: &str, markdown: bool, folder: Option<&Path>) -> Result<String, String> {
    if text.len() > MOST_TEXT {
        return Err(format!(
            "it holds more than {} MB, as much as Cox keeps",
            MOST_TEXT / 1024 / 1024
        ));
    }
    let extension = if markdown { "md" } else { "txt" };
    keep_bytes(text.as_bytes(), extension, folder).map(|(file, _)| file)
}

/// Keeps `bytes` in `folder`, or where Cox keeps them, named after what
/// they hold, with `extension`, and answers the copy's name and path.
fn keep_bytes(
    bytes: &[u8],
    extension: &str,
    folder: Option<&Path>,
) -> Result<(String, PathBuf), String> {
    let digest = Sha256::digest(bytes);
    let name = digest[..16].iter().fold(String::new(), |mut name, byte| {
        let _ = write!(name, "{byte:02x}");
        name
    });
    let file = format!("{name}.{extension}");
    let folder = match folder {
        Some(folder) => folder.to_owned(),
        None => storage::path(Kept::Media).ok_or("Cox cannot tell where it keeps what lasts")?,
    };
    std::fs::create_dir_all(&folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let copy = folder.join(&file);
    if !copy.exists() {
        std::fs::write(&copy, bytes).map_err(|error| format!("{}: {error}", copy.display()))?;
    }
    Ok((file, copy))
}

/// Where the copy named `file` is kept, if it is a name Cox gives one,
/// and its media type.
#[must_use]
pub fn kept(file: &str) -> Option<(PathBuf, &'static str)> {
    let (name, extension) = file.split_once('.')?;
    let well_named = name.len() == 32 && name.bytes().all(|byte| byte.is_ascii_hexdigit());
    let mime = KINDS
        .iter()
        .find(|(known, _, _)| *known == extension)
        .map(|&(_, _, mime)| mime)
        .or_else(|| {
            TEXTS
                .iter()
                .find(|(known, _)| *known == extension)
                .map(|&(_, mime)| mime)
        })?;
    if !well_named {
        return None;
    }
    Some((storage::path(Kept::Media)?.join(file), mime))
}

/// The pictures and videos a command shows by printing them as Markdown
/// does, a line `![caption](path)` each, with their captions, if any: how
/// a trigger `by: shell` shows them in the conversation.
#[must_use]
pub fn printed(output: &str) -> Vec<(Option<String>, String)> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim().strip_prefix("![")?;
            let (caption, rest) = line.split_once("](")?;
            let path = rest.strip_suffix(')')?.trim();
            if path.is_empty() {
                return None;
            }
            let caption = Some(caption.trim().to_owned()).filter(|caption| !caption.is_empty());
            Some((caption, path.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_text_of_a_file_shown_as_it_was() {
        let scratch = std::env::temp_dir().join(format!("cox-media-text-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);

        let report = keep_text("# Report\n", true, Some(&scratch)).unwrap();
        let code = keep_text("fn main() {}\n", false, Some(&scratch)).unwrap();

        assert_eq!(Path::new(&report).extension(), Some("md".as_ref()));
        assert_eq!(Path::new(&code).extension(), Some("txt".as_ref()));
        assert_eq!(
            std::fs::read_to_string(scratch.join(&report)).unwrap(),
            "# Report\n"
        );
        assert_eq!(
            keep_text("# Report\n", true, Some(&scratch)).unwrap(),
            report
        );
        assert!(keep_text(&"x".repeat(MOST_TEXT + 1), false, Some(&scratch)).is_err());
        assert!(kept(&report).is_some_and(|(_, mime)| mime.starts_with("text/markdown")));
        assert!(kept(&code).is_some_and(|(_, mime)| mime.starts_with("text/plain")));
        assert!(kept("report.md").is_none());
    }

    #[test]
    fn keeps_a_copy_named_after_what_it_holds() {
        let scratch = std::env::temp_dir().join(format!("cox-media-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let shot = scratch.join("shot.PNG");
        std::fs::write(&shot, b"not really a png").unwrap();
        let folder = scratch.join("kept");

        let kept = keep(&shot, Some(&folder)).unwrap();

        assert_eq!(kept.kind, MediaKind::Image);
        assert_eq!(
            Path::new(&kept.file).extension(),
            Some(std::ffi::OsStr::new("png"))
        );
        assert_eq!(
            std::fs::read(folder.join(&kept.file)).unwrap(),
            b"not really a png"
        );
        assert_eq!(keep(&shot, Some(&folder)).unwrap(), kept);
        let notes = scratch.join("notes.txt");
        std::fs::write(&notes, "text").unwrap();
        assert!(keep(&notes, Some(&folder)).is_err());
        assert_eq!(kept.size, None);
    }

    #[test]
    fn says_how_big_a_picture_is() {
        let scratch = std::env::temp_dir().join(format!("cox-media-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let shot = scratch.join("wide.png");
        image::RgbImage::new(64, 16).save(&shot).unwrap();

        let kept = keep(&shot, Some(&scratch.join("kept"))).unwrap();

        assert_eq!(kept.size, Some((64, 16)));
    }

    #[test]
    fn finds_only_the_names_it_gives() {
        assert!(
            kept("0123456789abcdef0123456789abcdef.mp4")
                .is_some_and(|(_, mime)| mime == "video/mp4")
        );
        assert_eq!(kept("../chats.sqlite"), None);
        assert_eq!(kept("0123456789abcdef0123456789abcdef.sqlite"), None);
        assert_eq!(kept("x/../0123456789abcdef0123456789abcdef.png"), None);
    }

    #[test]
    fn finds_what_a_command_shows_as_markdown_does() {
        assert_eq!(
            printed(
                "built\n![The greeting](shots/greeting.png)\n  ![](demo.mp4)\nnot ![this](x.png) one"
            ),
            [
                (
                    Some("The greeting".to_owned()),
                    "shots/greeting.png".to_owned()
                ),
                (None, "demo.mp4".to_owned()),
            ]
        );
    }
}
