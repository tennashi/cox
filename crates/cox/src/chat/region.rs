//! A region of the window the human hands the chat shown, as a
//! screenshot is taken: [`AddRegionToChat`], or "▭ region" under the
//! chat's input, lays a sheet over the window, where the human drags a
//! rectangle, and Escape gives up. The region goes to the chat as a
//! picture (see [`crate::chat::hand`]), with what the parts of the window
//! it covers say of it, as each part marks where it is drawn with
//! [`mark`]: a tab of code the lines in it, a terminal the text of its
//! rows, a page its address and where on the page the region is, an
//! emulator's display or a Mac window where on it, in the pixels the
//! agent's tools take.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use cox_core::entry::Attachment;
use gpui::{
    App, Bounds, IntoElement, KeyBinding, Pixels, Styled, Window, actions, canvas, point, px, size,
};

use crate::storage::{self, Kept};

actions!(
    cox,
    [
        /// Lays the sheet over the window to drag a region of it on, for
        /// the chat shown.
        AddRegionToChat,
        /// Gives up the region being chosen.
        CancelRegion,
    ]
);

/// The keys of the sheet.
pub const KEY_CONTEXT: &str = "Region";

/// The smallest a region is, each way, in points: a click on the sheet
/// is no region.
const SMALLEST: f32 = 4.;

/// Binds the keys that lay the sheet, and give it up.
pub fn bind_keys(cx: &mut App) {
    // Control-Shift-L hands what is selected off macOS (see
    // `crate::chat::hand`), and Control-Alt-L locks the screen there.
    let keys = if cfg!(target_os = "macos") {
        "cmd-shift-l"
    } else {
        "ctrl-alt-shift-l"
    };
    cx.bind_keys([
        KeyBinding::new(keys, AddRegionToChat, None),
        KeyBinding::new("escape", CancelRegion, Some(KEY_CONTEXT)),
    ]);
}

/// The keys that lay the sheet, as a tooltip names them.
#[must_use]
pub fn keys() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘⇧L"
    } else {
        "Ctrl-Alt-Shift-L"
    }
}

/// What a part of the window says of the region over it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Described {
    /// What the chip names the part by, short: `main.rs:5-7`.
    pub name: String,
    /// What the agent is told of it: what the part is, and what of it the
    /// region covers.
    pub text: String,
}

/// What a part, drawn over the first bounds, says of the stretch of the
/// region over it, the second, both in the window's points, if it says
/// anything.
type Describe = Rc<dyn Fn(Bounds<Pixels>, Bounds<Pixels>, &App) -> Option<Described>>;

thread_local! {
    /// Where each part that says what is in a region was drawn in the
    /// last frame, in the order it was drawn.
    static MARKS: RefCell<Vec<(Bounds<Pixels>, Describe)>> = const { RefCell::new(Vec::new()) };
}

/// Forgets where the parts were drawn, as a frame starts drawing them
/// again: the window does, as it draws.
pub fn forget_marks() {
    MARKS.with(|marks| marks.borrow_mut().clear());
}

/// Marks where the element it is put in is drawn, as `describe` says what
/// of it a region covers, given where the element is drawn and the
/// stretch of the region over it: it takes the whole of the element,
/// which has to be `relative`, and draws nothing.
pub fn mark(
    describe: impl Fn(Bounds<Pixels>, Bounds<Pixels>, &App) -> Option<Described> + 'static,
) -> impl IntoElement {
    let describe: Describe = Rc::new(describe);
    canvas(
        move |bounds, _, _| {
            MARKS.with(|marks| marks.borrow_mut().push((bounds, describe)));
        },
        |_, (), _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// What the parts the region `region` covers say of it, in the order
/// they were drawn.
#[must_use]
pub fn described(region: Bounds<Pixels>, cx: &App) -> Vec<Described> {
    let marks: Vec<(Bounds<Pixels>, Describe)> = MARKS.with(|marks| marks.borrow().clone());
    marks
        .into_iter()
        .filter_map(|(bounds, describe)| {
            let over = bounds.intersect(&region);
            (over.size.width > px(0.) && over.size.height > px(0.))
                .then(|| describe(bounds, over, cx))
                .flatten()
        })
        .collect()
}

/// The region between `from` and `to`, two corners of it, if it is big
/// enough to be one.
#[must_use]
pub fn between(from: gpui::Point<Pixels>, to: gpui::Point<Pixels>) -> Option<Bounds<Pixels>> {
    let origin = point(from.x.min(to.x), from.y.min(to.y));
    let size = size((from.x - to.x).abs(), (from.y - to.y).abs());
    (size.width >= px(SMALLEST) && size.height >= px(SMALLEST)).then_some(Bounds { origin, size })
}

/// Takes the picture of `region`, in the window's points, as the screen
/// shows it, keeps it, and hands it to the chat shown with what the parts
/// under it say, `described`, once it is taken: in a while, for the
/// sheet to be gone from the screen first.
pub fn take(region: Bounds<Pixels>, described: Vec<Described>, window: &Window, cx: &mut App) {
    let on_screen = Bounds {
        origin: window.bounds().origin + region.origin,
        size: region.size,
    };
    let scale = window.scale_factor();
    let background = cx.background_executor().clone();
    cx.spawn(async move |cx| {
        background
            .timer(std::time::Duration::from_millis(250))
            .await;
        let kept = background
            .spawn(async move { keep(&crate::ui::screen::capture(on_screen, scale)?) })
            .await;
        let attachment = attachment(region, &described, kept);
        let _ = cx.update(|cx| crate::chat::hand::hand(attachment, cx));
    })
    .detach();
}

/// Keeps `png`, a picture of a region, where Cox keeps them, answering
/// where.
fn keep(png: &[u8]) -> Result<PathBuf, String> {
    let folder =
        storage::path(Kept::Pictures).ok_or("Cox cannot tell where it keeps what lasts")?;
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("cannot make {}: {error}", folder.display()))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let path = folder.join(format!("{}-{:09}.png", now.as_secs(), now.subsec_nanos()));
    std::fs::write(&path, png)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(path)
}

/// What goes to the chat of `region`, in the window's points, under the
/// parts that say `described`: its picture, once `kept`, and what the
/// parts say; or, when the picture could not be taken, what they say and
/// why.
fn attachment(
    region: Bounds<Pixels>,
    described: &[Described],
    kept: Result<PathBuf, String>,
) -> Attachment {
    let names: Vec<&str> = described.iter().map(|part| part.name.as_str()).collect();
    let label = if names.is_empty() {
        "▭ Region".to_owned()
    } else {
        format!("▭ {}", names.join(", "))
    };
    let (x, y) = (f32::from(region.origin.x), f32::from(region.origin.y));
    let (width, height) = (f32::from(region.size.width), f32::from(region.size.height));
    let (picture, shown) = match kept {
        Ok(path) => (Some(path), "the picture after this shows it".to_owned()),
        Err(error) => (None, format!("its picture could not be taken: {error}")),
    };
    let mut text = format!(
        "A region of Cox's window the human chose, {width:.0}×{height:.0} points, \
        {x:.0} across and {y:.0} down from the window's top left corner; {shown}."
    );
    if !described.is_empty() {
        text.push_str(" It covers:");
        for part in described {
            text.push_str("\n\n- ");
            text.push_str(&part.text);
        }
    }
    Attachment {
        label,
        uri: "cox://region".to_owned(),
        text,
        picture,
    }
}

/// `text` in a Markdown code block, fenced with more backticks than any
/// run of them in it.
#[must_use]
pub fn fenced(text: &str) -> String {
    let longest = text
        .split(|letter| letter != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{text}\n{fence}")
}

/// `stretch`, from `start` to `end` along one way, where `whole` is
/// drawn from `from` to `from + along` that way: how far along it, in
/// the picture's `pixels` that way.
#[must_use]
pub fn along(
    (start, end): (Pixels, Pixels),
    (from, length): (Pixels, Pixels),
    pixels: u32,
) -> (u32, u32) {
    let at = |point: Pixels| {
        let across = ((point - from) / length).clamp(0., 1.);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            reason = "a picture's pixels fit in u32, and far below what f32 keeps exactly"
        )]
        let pixel = (across * pixels as f32).round() as u32;
        pixel
    };
    (at(start), at(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_is_between_two_corners_either_way() {
        let region = between(point(px(30.), px(40.)), point(px(10.), px(20.)));
        assert_eq!(
            region,
            Some(Bounds {
                origin: point(px(10.), px(20.)),
                size: size(px(20.), px(20.)),
            })
        );
        assert_eq!(
            between(point(px(10.), px(10.)), point(px(12.), px(40.))),
            None
        );
    }

    #[test]
    fn says_where_the_region_is_and_what_it_covers() {
        let region = Bounds {
            origin: point(px(100.), px(50.)),
            size: size(px(320.), px(80.)),
        };
        let described = [Described {
            name: "main.rs:5-7".to_owned(),
            text: "lines 5-7 of /work/main.rs".to_owned(),
        }];
        let attachment = attachment(region, &described, Ok(PathBuf::from("/kept/1.png")));
        assert_eq!(attachment.label, "▭ main.rs:5-7");
        assert_eq!(attachment.picture, Some(PathBuf::from("/kept/1.png")));
        assert_eq!(
            attachment.text,
            "A region of Cox's window the human chose, 320×80 points, 100 across and 50 down \
            from the window's top left corner; the picture after this shows it. It covers:\
            \n\n- lines 5-7 of /work/main.rs"
        );
    }

    #[test]
    fn a_region_whose_picture_failed_says_why() {
        let region = Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(10.), px(10.)),
        };
        let attachment = attachment(region, &[], Err("no screen".to_owned()));
        assert_eq!(attachment.label, "▭ Region");
        assert_eq!(attachment.picture, None);
        assert!(
            attachment
                .text
                .ends_with("its picture could not be taken: no screen.")
        );
    }

    #[test]
    fn a_fence_is_longer_than_the_backticks_in_it() {
        assert_eq!(fenced("a"), "```\na\n```");
        assert_eq!(fenced("````x"), "`````\n````x\n`````");
    }

    #[test]
    fn a_stretch_is_in_the_picture_s_pixels() {
        assert_eq!(
            along((px(150.), px(250.)), (px(100.), px(400.)), 1200),
            (150, 450)
        );
        // Past the picture, it stops at its edges.
        assert_eq!(
            along((px(50.), px(600.)), (px(100.), px(400.)), 1200),
            (0, 1200)
        );
    }
}
