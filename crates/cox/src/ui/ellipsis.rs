//! A line of text that ends in an ellipsis where it does not fit, as a
//! tab's name or the first line of a thought.
//!
//! GPUI's own `truncate()` does not do it in a row: its text keeps the
//! size it was first measured at, which is the line whole, before the
//! row gives it its width, so the line is only cut off at the edge.
//! [`Ellipsis`] is as wide as the line, or as the room it gets, and
//! shortens the line to the width it is laid out at only as it paints.

use gpui::{
    App, AvailableSpace, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, SharedString, Style, TextAlign, TextRun, Window, point, px,
    size,
};

/// What ends a line that does not fit.
const SUFFIX: &str = "…";

/// `text`, on one line, ending in an ellipsis where it does not fit, in
/// the text style of where it is.
pub fn ellipsis(text: impl Into<SharedString>) -> Ellipsis {
    Ellipsis { text: text.into() }
}

/// A line of text that ends in an ellipsis where it does not fit.
pub struct Ellipsis {
    text: SharedString,
}

/// The text, its run in the style of where it is, how wide it is whole,
/// the size of its font and line, and which side it keeps to, as the
/// element was laid out.
pub struct Laid {
    text: SharedString,
    run: TextRun,
    width: Pixels,
    font_size: Pixels,
    line_height: Pixels,
    align: TextAlign,
}

impl Element for Ellipsis {
    type RequestLayoutState = Laid;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // The first line only, as one line shows.
        let text: SharedString = self
            .text
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
            .into();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let run = style.to_run(text.len());
        let align = style.text_align;
        let width = window
            .text_system()
            .shape_line(text.clone(), font_size, std::slice::from_ref(&run), None)
            .width;
        // As wide as the line, but no wider than the room it gets, and
        // narrower as the room shrinks, down to none.
        let mut layout = Style::default();
        layout.min_size.width = Pixels::ZERO.into();
        let id = window.request_measured_layout(layout, move |known, available, _, _| {
            let room = match available.width {
                AvailableSpace::Definite(room) => Some(room),
                AvailableSpace::MinContent => Some(Pixels::ZERO),
                AvailableSpace::MaxContent => None,
            };
            let width = known
                .width
                .unwrap_or_else(|| room.map_or(width, |room| width.min(room)));
            size(width, line_height)
        });
        (
            id,
            Laid {
                text,
                run,
                width,
                font_size,
                line_height,
                align,
            },
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _line: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        line: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut runs = vec![line.run.clone()];
        // Whole where it fits as shaped. GPUI's `truncate_line` adds up
        // each character's width on its own, which comes to more than the
        // line shaped, as its pairs are kerned, and so it cut a line the
        // row was laid out just wide enough for, as "COX.md" to "COX….".
        // The layout's rounding takes off less than a pixel.
        let text = if line.width <= bounds.size.width + px(1.) {
            line.text.clone()
        } else {
            window
                .text_system()
                .line_wrapper(line.run.font.clone(), line.font_size)
                .truncate_line(line.text.clone(), bounds.size.width, SUFFIX, &mut runs)
        };
        let shaped = window
            .text_system()
            .shape_line(text, line.font_size, &runs, None);
        // The line in the middle of the room it has, as text in a box is,
        // and on the side the text keeps to.
        let top = bounds.origin.y + (bounds.size.height - line.line_height) / 2.;
        let spare = (bounds.size.width - shaped.width).max(Pixels::ZERO);
        let left = match line.align {
            TextAlign::Left => bounds.origin.x,
            TextAlign::Center => bounds.origin.x + spare / 2.,
            TextAlign::Right => bounds.origin.x + spare,
        };
        let origin = point(left, top);
        if let Err(error) = shaped.paint(origin, line.line_height, window, cx) {
            tracing::warn!(%error, "could not draw a line of text");
        }
    }
}

impl IntoElement for Ellipsis {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
