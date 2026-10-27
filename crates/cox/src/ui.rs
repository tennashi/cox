//! What every part of the window shares, whatever it shows: how things
//! look ([`theme`]), how paths are written ([`paths`]), and what the
//! parts tell the window alike.

pub mod paths;
pub mod theme;

use gpui::actions;

actions!(
    cox,
    [
        /// Copies the text selected in the part with the keyboard.
        Copy,
    ]
);

/// The human clicked a link to the address it holds, which the window
/// opens on the chat's stage.
pub struct LinkClicked(pub String);
