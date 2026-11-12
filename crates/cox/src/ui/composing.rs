//! Whether an input method composes text in the window, for what reads
//! the keys ahead of the inputs, as Vim's keys do (see [`crate::vim`]),
//! to leave them to it.

use gpui::{App, EntityId, Global};

/// The input an input method composes in now, if any: the one with the
/// keyboard.
#[derive(Default)]
struct Composing(Option<EntityId>);

impl Global for Composing {}

/// Says whether `input` holds text the input method is still composing.
pub fn note(input: EntityId, composing: bool, cx: &mut App) {
    let was = cx.try_global::<Composing>().and_then(|now| now.0);
    let now = match (composing, was) {
        (true, _) => Some(input),
        (false, Some(other)) if other != input => Some(other),
        (false, _) => None,
    };
    if now != was {
        cx.set_global(Composing(now));
    }
}

/// Whether an input method composes text now.
#[must_use]
pub fn any(cx: &App) -> bool {
    cx.try_global::<Composing>()
        .is_some_and(|composing| composing.0.is_some())
}
