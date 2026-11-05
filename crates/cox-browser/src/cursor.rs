//! The cursor the page asks for where the mouse is: a pointing hand over a
//! link, an I-beam over text, a resize arrow over a split, and so on, as
//! Chromium tells it to the browser's display handler. The browser draws
//! no cursor of its own, as it has no window, so the stage shows this one
//! over the page.

use cef::CursorType;

/// What the page asks the cursor to look like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    /// The arrow, as over most of a page.
    #[default]
    Arrow,
    /// A pointing hand, over a link or what the page marks as clickable.
    Hand,
    /// An I-beam, over text to select or a field to type in.
    Text,
    /// An I-beam on its side, over text written top to bottom.
    VerticalText,
    Crosshair,
    /// An open hand, over what can be dragged, and closed while it is.
    Grab,
    Grabbing,
    /// Over an edge that resizes, by the way it moves.
    ResizeLeftRight,
    ResizeUpDown,
    ResizeUpLeftDownRight,
    ResizeUpRightDownLeft,
    ResizeColumn,
    ResizeRow,
    /// Over what cannot be clicked or dropped on.
    NotAllowed,
    /// Over what opens a menu.
    ContextMenu,
    /// While dragging, over where a drop copies or links.
    Copy,
    Link,
    /// No cursor at all, as a page that hides it asks.
    None,
}

impl From<CursorType> for Cursor {
    fn from(kind: CursorType) -> Self {
        match kind {
            CursorType::HAND => Self::Hand,
            CursorType::IBEAM => Self::Text,
            CursorType::VERTICALTEXT => Self::VerticalText,
            CursorType::CROSS | CursorType::CELL => Self::Crosshair,
            CursorType::GRAB | CursorType::MOVE => Self::Grab,
            CursorType::GRABBING => Self::Grabbing,
            CursorType::EASTRESIZE | CursorType::WESTRESIZE | CursorType::EASTWESTRESIZE => {
                Self::ResizeLeftRight
            }
            CursorType::NORTHRESIZE | CursorType::SOUTHRESIZE | CursorType::NORTHSOUTHRESIZE => {
                Self::ResizeUpDown
            }
            CursorType::NORTHWESTRESIZE
            | CursorType::SOUTHEASTRESIZE
            | CursorType::NORTHWESTSOUTHEASTRESIZE => Self::ResizeUpLeftDownRight,
            CursorType::NORTHEASTRESIZE
            | CursorType::SOUTHWESTRESIZE
            | CursorType::NORTHEASTSOUTHWESTRESIZE => Self::ResizeUpRightDownLeft,
            CursorType::COLUMNRESIZE => Self::ResizeColumn,
            CursorType::ROWRESIZE => Self::ResizeRow,
            CursorType::NOTALLOWED | CursorType::NODROP | CursorType::DND_NONE => Self::NotAllowed,
            CursorType::CONTEXTMENU => Self::ContextMenu,
            CursorType::COPY | CursorType::DND_COPY => Self::Copy,
            CursorType::ALIAS | CursorType::DND_LINK => Self::Link,
            CursorType::NONE => Self::None,
            // The rest, as the panning arrows, the zooms, waiting and
            // help, have no cursor of their own on every platform: the
            // arrow stands for them.
            _ => Self::Arrow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_s_cursors_map_to_their_own() {
        assert_eq!(Cursor::from(CursorType::HAND), Cursor::Hand);
        assert_eq!(Cursor::from(CursorType::IBEAM), Cursor::Text);
        assert_eq!(
            Cursor::from(CursorType::EASTWESTRESIZE),
            Cursor::ResizeLeftRight
        );
        assert_eq!(Cursor::from(CursorType::POINTER), Cursor::Arrow);
        assert_eq!(Cursor::from(CursorType::WAIT), Cursor::Arrow);
    }
}
