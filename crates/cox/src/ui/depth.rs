//! How far a part stands up from what it sits on, as the window draws
//! it: what the human clicks stands a little up, what they type into is
//! sunk a little in, a card stands well up from the window's ground, and
//! what floats over the rest stands highest, as [`coxd::theme`] says,
//! which the pages of the stage draw alike from its CSS.

use gpui::{BoxShadow, Styled, linear_color_stop, linear_gradient, point, px, rgb, rgba};

use coxd::theme::{ACCENT_TINT, ACCENT_TINT_EDGE, Depth, FOCUS_RING, FOCUS_WIDTH, INSET, Shadow};

/// `shadow` as GPUI casts one.
#[must_use]
pub fn shadow(shadow: Shadow) -> BoxShadow {
    BoxShadow {
        color: rgba(shadow.color).into(),
        offset: point(px(shadow.x), px(shadow.y)),
        blur_radius: px(shadow.blur),
        spread_radius: px(shadow.spread),
    }
}

/// The shadows of `depth`, the nearest first.
#[must_use]
pub fn shadows(depth: Depth) -> Vec<BoxShadow> {
    depth.shadows.iter().copied().map(shadow).collect()
}

/// The ring around what has the keyboard, outside it, to cast after the
/// shadows it casts at rest.
#[must_use]
pub fn focus() -> BoxShadow {
    BoxShadow {
        color: rgba(FOCUS_RING).into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(FOCUS_WIDTH),
    }
}

/// What every part the window draws can do to stand up from what it
/// sits on, or to sink into it.
pub trait Stand: Styled + Sized {
    /// Stands up by `depth`: with its edge, as a border, and its
    /// shadows. A part seen through casts them under itself too, so one
    /// that is takes its edge alone, from [`Stand::edged`].
    #[must_use]
    fn stand(self, depth: Depth) -> Self {
        self.edged(depth).shadow(shadows(depth))
    }

    /// Takes the edge of `depth` alone, as a border.
    #[must_use]
    fn edged(self, depth: Depth) -> Self {
        self.border_1().border_color(rgba(depth.edge))
    }

    /// Marks it chosen, as the chat shown or the tab shown is, faint in
    /// the accent with an edge a little less faint, when `chosen`; its
    /// edge is drawn either way, unseen when not, so that choosing it
    /// moves nothing.
    #[must_use]
    fn chosen(self, chosen: bool) -> Self {
        let part = self.border_1();
        if chosen {
            part.bg(rgba(ACCENT_TINT))
                .border_color(rgba(ACCENT_TINT_EDGE))
        } else {
            part.border_color(gpui::transparent_black())
        }
    }

    /// Sinks into its ground, as what the human types into does: darker
    /// than it, shaded from the top, with its edge, and the ring of
    /// [`focus`] around it while it has the keyboard.
    #[must_use]
    fn sink(self, focused: bool) -> Self {
        let part = self
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(INSET.top), 0.),
                linear_color_stop(rgb(INSET.ground), INSET.shade),
            ))
            .border_1()
            .border_color(rgba(INSET.edge));
        if focused {
            part.shadow(vec![focus()])
        } else {
            part
        }
    }
}

impl<E: Styled> Stand for E {}

#[cfg(test)]
mod tests {
    use super::*;
    use coxd::theme::{CARD, CONTROL};

    #[test]
    fn a_shadow_is_cast_as_the_theme_says() {
        let cast = shadow(Shadow {
            x: 1.,
            y: 2.,
            blur: 3.,
            spread: -4.,
            color: 0x0000_0080,
        });
        assert_eq!(cast.offset, point(px(1.), px(2.)));
        assert_eq!(cast.blur_radius, px(3.));
        assert_eq!(cast.spread_radius, px(-4.));
        assert!(
            (cast.color.a - 128. / 255.).abs() < 0.01,
            "{:?}",
            cast.color
        );
    }

    #[test]
    fn a_depth_casts_each_of_its_shadows_nearest_first() {
        assert_eq!(shadows(CONTROL).len(), 1);
        let card = shadows(CARD);
        assert_eq!(card.len(), CARD.shadows.len());
        assert!(card[0].blur_radius < card[card.len() - 1].blur_radius);
    }

    #[test]
    fn the_focus_ring_stands_outside_and_is_sharp() {
        let ring = focus();
        assert_eq!(ring.blur_radius, px(0.));
        assert_eq!(ring.spread_radius, px(FOCUS_WIDTH));
    }

    #[test]
    fn choosing_a_part_tints_it_and_moves_nothing() {
        let mut chosen = gpui::div().chosen(true);
        let mut not = gpui::div().chosen(false);
        assert_eq!(chosen.style().border_widths, not.style().border_widths);
        assert!(chosen.style().background.is_some());
        assert!(not.style().background.is_none());
        assert_ne!(chosen.style().border_color, not.style().border_color);
    }

    #[test]
    fn a_sunk_part_rings_only_while_it_has_the_keyboard() {
        let mut focused = gpui::div().sink(true);
        let mut not = gpui::div().sink(false);
        assert_eq!(focused.style().box_shadow, Some(vec![focus()]));
        assert_eq!(not.style().box_shadow, None);
        assert_eq!(focused.style().border_widths, not.style().border_widths);
    }
}
