//! The icons the window draws, as SVG files built into Cox, which GPUI
//! loads through [`Icons`], its source of assets, each in a color of its
//! own.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString, Styled, Svg, px, rgb, svg};

/// An icon of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Book,
    Changes,
    Close,
    Code,
    Document,
    Files,
    Flow,
    Folder,
    Globe,
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
    const ALL: [Self; 18] = [
        Self::Book,
        Self::Changes,
        Self::Close,
        Self::Code,
        Self::Document,
        Self::Files,
        Self::Flow,
        Self::Folder,
        Self::Globe,
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
            Self::Book => "icons/book.svg",
            Self::Changes => "icons/changes.svg",
            Self::Close => "icons/close.svg",
            Self::Code => "icons/code.svg",
            Self::Document => "icons/document.svg",
            Self::Files => "icons/files.svg",
            Self::Flow => "icons/flow.svg",
            Self::Folder => "icons/folder.svg",
            Self::Globe => "icons/globe.svg",
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
            Self::Book => include_bytes!("../../assets/icons/book.svg"),
            Self::Changes => include_bytes!("../../assets/icons/changes.svg"),
            Self::Close => include_bytes!("../../assets/icons/close.svg"),
            Self::Code => include_bytes!("../../assets/icons/code.svg"),
            Self::Document => include_bytes!("../../assets/icons/document.svg"),
            Self::Files => include_bytes!("../../assets/icons/files.svg"),
            Self::Flow => include_bytes!("../../assets/icons/flow.svg"),
            Self::Folder => include_bytes!("../../assets/icons/folder.svg"),
            Self::Globe => include_bytes!("../../assets/icons/globe.svg"),
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

/// The assets GPUI loads: the icons.
pub struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
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
}
