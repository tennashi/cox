//! How far a part stands up from what it sits on, as the window draws
//! it: what the human clicks stands a little up, what they type into is
//! sunk a little in, a card stands well up from the window's ground, and
//! what floats over the rest stands highest, as [`coxd::theme`] says,
//! which the pages of the stage draw alike from its CSS.

use gpui::{BoxShadow, Styled, linear_color_stop, linear_gradient, point, px, rgb, rgba};

use coxd::theme::{Depth, FOCUS_RING, FOCUS_WIDTH, INSET, Shadow};

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

/// `part` standing up from its ground by `depth`: with its edge, as a
/// border, and its shadows. A part seen through casts them under itself
/// too, so one that is gets its edge alone, from [`edge`].
pub fn stand<E: Styled>(part: E, depth: Depth) -> E {
    edge(part, depth).shadow(shadows(depth))
}

/// `part` with the edge of `depth` alone, as a border.
pub fn edge<E: Styled>(part: E, depth: Depth) -> E {
    part.border_1().border_color(rgba(depth.edge))
}

/// `part` sunk into its ground, as what the human types into is: darker
/// than it, shaded from the top, with its edge, and the ring of
/// [`focus`] around it while it has the keyboard.
pub fn sink<E: Styled>(part: E, focused: bool) -> E {
    let part = part
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
}
