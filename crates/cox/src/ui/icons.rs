//! The icons the window draws, as SVG files built into Cox, which GPUI
//! loads through [`Icons`], its source of assets, each in a color of its
//! own, and Cox's own icon, the app's, in its colors.

use std::borrow::Cow;

use gpui::{AssetSource, Img, Result, SharedString, Styled, Svg, img, px, rgb, svg};

/// An icon of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Bolt,
    Book,
    Changes,
    Clock,
    Close,
    Code,
    Document,
    Files,
    Flow,
    Folder,
    Globe,
    Inbox,
    Map,
    Phone,
    Plug,
    Plus,
    Search,
    Settings,
    Target,
    Terminal,
    Window,
}

impl Icon {
    const ALL: [Self; 21] = [
        Self::Bolt,
        Self::Book,
        Self::Changes,
        Self::Clock,
        Self::Close,
        Self::Code,
        Self::Document,
        Self::Files,
        Self::Flow,
        Self::Folder,
        Self::Globe,
        Self::Inbox,
        Self::Map,
        Self::Phone,
        Self::Plug,
        Self::Plus,
        Self::Search,
        Self::Settings,
        Self::Target,
        Self::Terminal,
        Self::Window,
    ];

    /// Where GPUI finds it among the assets.
    fn path(self) -> &'static str {
        match self {
            Self::Bolt => "icons/bolt.svg",
            Self::Book => "icons/book.svg",
            Self::Changes => "icons/changes.svg",
            Self::Clock => "icons/clock.svg",
            Self::Close => "icons/close.svg",
            Self::Code => "icons/code.svg",
            Self::Document => "icons/document.svg",
            Self::Files => "icons/files.svg",
            Self::Flow => "icons/flow.svg",
            Self::Folder => "icons/folder.svg",
            Self::Globe => "icons/globe.svg",
            Self::Inbox => "icons/inbox.svg",
            Self::Map => "icons/map.svg",
            Self::Phone => "icons/phone.svg",
            Self::Plug => "icons/plug.svg",
            Self::Plus => "icons/plus.svg",
            Self::Search => "icons/search.svg",
            Self::Settings => "icons/settings.svg",
            Self::Target => "icons/target.svg",
            Self::Terminal => "icons/terminal.svg",
            Self::Window => "icons/window.svg",
        }
    }

    /// Its SVG file.
    fn file(self) -> &'static [u8] {
        match self {
            Self::Bolt => include_bytes!("../../assets/icons/bolt.svg"),
            Self::Book => include_bytes!("../../assets/icons/book.svg"),
            Self::Changes => include_bytes!("../../assets/icons/changes.svg"),
            Self::Clock => include_bytes!("../../assets/icons/clock.svg"),
            Self::Close => include_bytes!("../../assets/icons/close.svg"),
            Self::Code => include_bytes!("../../assets/icons/code.svg"),
            Self::Document => include_bytes!("../../assets/icons/document.svg"),
            Self::Files => include_bytes!("../../assets/icons/files.svg"),
            Self::Flow => include_bytes!("../../assets/icons/flow.svg"),
            Self::Folder => include_bytes!("../../assets/icons/folder.svg"),
            Self::Globe => include_bytes!("../../assets/icons/globe.svg"),
            Self::Inbox => include_bytes!("../../assets/icons/inbox.svg"),
            Self::Map => include_bytes!("../../assets/icons/map.svg"),
            Self::Phone => include_bytes!("../../assets/icons/phone.svg"),
            Self::Plug => include_bytes!("../../assets/icons/plug.svg"),
            Self::Plus => include_bytes!("../../assets/icons/plus.svg"),
            Self::Search => include_bytes!("../../assets/icons/search.svg"),
            Self::Settings => include_bytes!("../../assets/icons/settings.svg"),
            Self::Target => include_bytes!("../../assets/icons/target.svg"),
            Self::Terminal => include_bytes!("../../assets/icons/terminal.svg"),
            Self::Window => include_bytes!("../../assets/icons/window.svg"),
        }
    }
}

/// The icon, `size` points square, in `color`: an icon takes no color
/// from the text around it.
#[must_use]
pub fn icon(icon: Icon, size: f32, color: u32) -> Svg {
    svg()
        .path(icon.path())
        .size(px(size))
        .flex_none()
        .text_color(rgb(color))
}

/// Cox's app icon, as `cargo xtask bundle` renders it for macOS.
const APP_ICON: &str = include_str!("../../assets/app-icon.svg");

/// What the app icon's file says of its size, and the room it leaves
/// around its rounded square for the shadow macOS draws.
const APP_ICON_FRAME: &str = r#"width="1024" height="1024" viewBox="0 0 1024 1024""#;

/// Cox's app icon, `size` points square: its rounded square alone, in its
/// own colors, without the room around it for its shadow.
#[must_use]
pub fn app_icon(size: f32) -> Img {
    img(format!("app-icon/{size}.svg"))
        .size(px(size))
        .flex_none()
}

/// The app icon's file, for [`app_icon`] at `path`, sized to the points
/// it is drawn at: GPUI renders an image's SVG at twice the size it says,
/// so it is sharp, rather than scaled down from 1024 pixels.
fn app_icon_file(path: &str) -> Option<String> {
    let size: f32 = path
        .strip_prefix("app-icon/")?
        .strip_suffix(".svg")?
        .parse()
        .ok()?;
    APP_ICON.contains(APP_ICON_FRAME).then(|| {
        APP_ICON.replacen(
            APP_ICON_FRAME,
            &format!(r#"width="{size}" height="{size}" viewBox="100 100 824 824""#),
            1,
        )
    })
}

/// The assets GPUI loads: the icons, and the app's.
pub struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(file) = app_icon_file(path) {
            return Ok(Some(Cow::Owned(file.into_bytes())));
        }
        Ok(Icon::ALL
            .into_iter()
            .find(|icon| icon.path() == path)
            .map(|icon| Cow::Borrowed(icon.file())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Icon::ALL
            .into_iter()
            .map(Icon::path)
            .filter(|found| found.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_found_where_it_is_drawn_from() {
        for icon in Icon::ALL {
            let file = Icons.load(icon.path()).unwrap().unwrap();
            assert!(file.starts_with(b"<svg"), "{icon:?}");
        }
        assert_eq!(Icons.list("icons/").unwrap().len(), Icon::ALL.len());
    }

    #[test]
    fn the_app_icon_is_its_rounded_square_at_the_size_it_is_drawn() {
        let file = Icons.load("app-icon/42.svg").unwrap().unwrap();
        let file = std::str::from_utf8(&file).unwrap();
        assert!(file.starts_with("<svg"));
        assert!(file.contains(r#"width="42" height="42" viewBox="100 100 824 824""#));
        assert_eq!(Icons.load("app-icon/big.svg").unwrap(), None);
    }
}
