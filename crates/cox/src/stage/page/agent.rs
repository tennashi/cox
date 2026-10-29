//! What the page draws for the agent: its pointer, as an arrow with a
//! label saying whose it is, the ring of each click, and the outline of
//! the field it types into, as the relay saw them (see
//! [`crate::browser::pointer`]).

use std::time::Instant;

use gpui::{
    BorderStyle, Bounds, Context, Div, Hsla, PathBuilder, Pixels, Window, div, point, prelude::*,
    px, quad, rgb, size, white,
};

use super::Page;
use crate::browser::pointer::{Look, Rect, Spot};

impl Page {
    /// Works out how the agent's pointer looks now, and arranges to render
    /// again when that changes on its own: the stage's paint asks for the
    /// next frame while it moves, and a timer wakes it for what comes
    /// later.
    #[expect(
        clippy::used_underscore_binding,
        reason = "the timer is only held: dropping it cancels it"
    )]
    pub(super) fn update_look(&mut self, cx: &mut Context<Self>) {
        self.look = self.pointer.look(Instant::now());
        self._pointer_changes = self
            .look
            .as_ref()
            .map(|look| look.next_change)
            .filter(|next| !next.is_zero())
            .map(|next| {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(next).await;
                    this.update(cx, |_, cx| cx.notify()).ok();
                })
            });
    }

    /// The label by the agent's pointer, which says whose it is, and that
    /// the agent is typing.
    pub(super) fn render_label(look: &Look) -> Div {
        div()
            .absolute()
            .left(px(look.pointer.x + 14.))
            .top(px(look.pointer.y + 18.))
            .opacity(look.opacity)
            .px_1p5()
            .rounded_sm()
            .bg(agent_color())
            .text_color(white())
            .text_xs()
            .whitespace_nowrap()
            .child(if look.typing {
                "Agent · typing"
            } else {
                "Agent"
            })
    }
}

/// The color of everything the stage draws for the agent.
fn agent_color() -> Hsla {
    rgb(0x4f_6bff).into()
}

/// The agent's pointer at `look`, over the stage at `bounds`: the ring of
/// each click, the outline of the field it types into, and the arrow.
pub(super) fn paint_agent(look: &Look, bounds: Bounds<Pixels>, window: &mut Window) {
    let color = agent_color();
    let on_stage = |spot: Spot| bounds.origin + point(px(spot.x), px(spot.y));
    if let Some(Rect {
        x,
        y,
        width,
        height,
    }) = look.field
    {
        let field = Bounds {
            origin: on_stage(Spot { x, y }) - point(px(3.), px(3.)),
            size: size(px(width + 6.), px(height + 6.)),
        };
        window.paint_quad(quad(
            field,
            px(4.),
            color.opacity(0.08 * look.opacity),
            px(2.),
            color.opacity(look.opacity),
            BorderStyle::Solid,
        ));
    }
    for ripple in &look.ripples {
        let radius = px(ripple.radius);
        let ring = Bounds {
            origin: on_stage(ripple.at) - point(radius, radius),
            size: size(radius * 2., radius * 2.),
        };
        window.paint_quad(quad(
            ring,
            radius,
            color.opacity(0.2 * ripple.opacity),
            px(2.),
            color.opacity(ripple.opacity),
            BorderStyle::Solid,
        ));
    }

    // An arrow with its tip on the spot, the shape of the usual pointer.
    let tip = on_stage(look.pointer);
    let corners = [
        (0., 0.),
        (0., 17.),
        (4.5, 13.),
        (7.5, 20.),
        (10., 19.),
        (7., 12.),
        (12.5, 12.),
    ]
    .map(|(x, y)| tip + point(px(x), px(y)));
    let mut arrow = PathBuilder::fill();
    arrow.add_polygon(&corners, true);
    let mut edge = PathBuilder::stroke(px(1.5));
    edge.add_polygon(&corners, true);
    if let Ok(arrow) = arrow.build() {
        window.paint_path(arrow, color.opacity(look.opacity));
    }
    if let Ok(edge) = edge.build() {
        window.paint_path(edge, white().opacity(look.opacity));
    }
}
