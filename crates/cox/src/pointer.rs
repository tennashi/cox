//! The agent's pointer: what the stage draws over the page to show where
//! the agent points, clicks and types, worked out from the input the relay
//! sees, in plain Rust so it is tested without GPUI.
//!
//! The pointer glides to where the agent moves it, a ring spreads where it
//! clicks, and the field it types into is outlined. When the agent leaves
//! the page alone for a while, the pointer fades away.

use std::time::{Duration, Instant};

use crate::relay::AgentInput;

/// How long the pointer takes to glide to where the agent moved it.
const GLIDE: Duration = Duration::from_millis(350);
/// How long a click's ring takes to spread and fade.
const RIPPLE: Duration = Duration::from_millis(500);
/// How far a click's ring spreads, in logical pixels.
const RIPPLE_RADIUS: f32 = 22.;
/// How long after the agent's last key the stage still says it is typing.
const TYPING: Duration = Duration::from_millis(1200);
/// How long the pointer stays after the agent's last input, before it
/// fades.
const LINGER: Duration = Duration::from_secs(4);
/// How long the pointer takes to fade.
const FADE: Duration = Duration::from_millis(600);

/// A point on the page, in logical pixels from its top left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spot {
    pub x: f32,
    pub y: f32,
}

/// A rectangle on the page, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    fn contains(&self, spot: Spot) -> bool {
        (self.x..=self.x + self.width).contains(&spot.x)
            && (self.y..=self.y + self.height).contains(&spot.y)
    }

    fn center(&self) -> Spot {
        Spot {
            x: self.x + self.width / 2.,
            y: self.y + self.height / 2.,
        }
    }
}

/// A click's ring, as it is at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ripple {
    pub at: Spot,
    pub radius: f32,
    pub opacity: f32,
}

/// What the stage draws for the agent at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// Where the pointer's tip is.
    pub pointer: Spot,
    /// How much of the pointer shows, from 0 to 1, as it fades.
    pub opacity: f32,
    pub ripples: Vec<Ripple>,
    /// The field the agent is typing into, once the relay found it.
    pub field: Option<Rect>,
    pub typing: bool,
    /// How long until the look changes on its own: zero while it moves.
    pub next_change: Duration,
}

/// The pointer's way from one spot to another.
#[derive(Clone, Copy, Debug)]
struct Glide {
    from: Spot,
    to: Spot,
    start: Instant,
}

impl Glide {
    fn arrival(&self) -> Instant {
        self.start + GLIDE
    }

    /// Where the pointer is at `now`: fast at first, slowing as it nears
    /// the end.
    fn at(&self, now: Instant) -> Spot {
        let done = now.saturating_duration_since(self.start).as_secs_f32() / GLIDE.as_secs_f32();
        let eased = 1. - (1. - done.min(1.)).powi(3);
        Spot {
            x: self.from.x + (self.to.x - self.from.x) * eased,
            y: self.from.y + (self.to.y - self.from.y) * eased,
        }
    }
}

/// The agent's pointer, as the input the relay saw so far leaves it.
#[derive(Debug, Default)]
pub struct AgentPointer {
    /// `None` until the agent first points somewhere.
    glide: Option<Glide>,
    /// Where the agent clicked, and when each ring starts to spread.
    ripples: Vec<(Spot, Instant)>,
    /// When the agent last typed.
    typed: Option<Instant>,
    field: Option<Rect>,
    /// When the agent last sent any input.
    seen: Option<Instant>,
}

impl AgentPointer {
    /// The agent sent `input` to the page at `now`.
    pub fn see(&mut self, input: AgentInput, now: Instant) {
        self.ripples
            .retain(|&(_, start)| now.saturating_duration_since(start) < RIPPLE);
        match input {
            AgentInput::Move { x, y } | AgentInput::Wheel { x, y } => {
                self.field = None;
                self.go(Spot { x, y }, now);
            }
            AgentInput::Press { x, y } => {
                self.field = None;
                let glide = self.go(Spot { x, y }, now);
                // The ring spreads once the pointer is there.
                self.ripples.push((glide.to, glide.arrival().max(now)));
            }
            AgentInput::Type => self.typed = Some(now),
            AgentInput::Field {
                x,
                y,
                width,
                height,
            } => {
                let field = Rect {
                    x,
                    y,
                    width,
                    height,
                };
                self.field = Some(field);
                let there = self
                    .glide
                    .is_some_and(|glide| self.shown(now) && field.contains(glide.to));
                if !there {
                    self.go(field.center(), now);
                }
            }
        }
        self.seen = Some(now);
    }

    /// Sends the pointer to `to`: gliding from where it is, if it shows,
    /// or straight there if not. Already on its way there, it carries on.
    fn go(&mut self, to: Spot, now: Instant) -> Glide {
        let from = match self.glide {
            Some(glide) if self.shown(now) => {
                if glide.to == to {
                    return glide;
                }
                glide.at(now)
            }
            _ => to,
        };
        let glide = Glide {
            from,
            to,
            start: now,
        };
        self.glide = Some(glide);
        glide
    }

    fn shown(&self, now: Instant) -> bool {
        self.seen
            .is_some_and(|seen| now.saturating_duration_since(seen) < LINGER + FADE)
    }

    /// What the stage draws at `now`, or `None` while there is no pointer
    /// to show.
    #[must_use]
    pub fn look(&self, now: Instant) -> Option<Look> {
        let glide = self.glide?;
        if !self.shown(now) {
            return None;
        }
        let idle = self
            .seen
            .map_or(Duration::ZERO, |seen| now.saturating_duration_since(seen));
        let opacity = if idle < LINGER {
            1.
        } else {
            1. - idle.saturating_sub(LINGER).as_secs_f32() / FADE.as_secs_f32()
        };
        let ripples = self
            .ripples
            .iter()
            .filter(|&&(_, start)| start <= now && now - start < RIPPLE)
            .map(|&(at, start)| {
                let spread = (now - start).as_secs_f32() / RIPPLE.as_secs_f32();
                Ripple {
                    at,
                    radius: RIPPLE_RADIUS * (1. - (1. - spread).powi(2)),
                    opacity: 1. - spread,
                }
            })
            .collect();
        let typing_left = self
            .typed
            .and_then(|typed| TYPING.checked_sub(now.saturating_duration_since(typed)))
            .filter(|left| !left.is_zero());
        let moving = now < glide.arrival()
            || self
                .ripples
                .iter()
                .any(|&(_, start)| now.saturating_duration_since(start) < RIPPLE)
            || idle >= LINGER;
        let lingering = LINGER.saturating_sub(idle);
        let next_change = if moving {
            Duration::ZERO
        } else {
            typing_left.map_or(lingering, |left| left.min(lingering))
        };
        Some(Look {
            pointer: glide.at(now),
            opacity,
            ripples,
            field: self.field.filter(|_| typing_left.is_some()),
            typing: typing_left.is_some(),
            next_change,
        })
    }
}

#[cfg(test)]
#[expect(
    clippy::unchecked_time_subtraction,
    reason = "the expected durations are known to be positive"
)]
mod tests {
    use super::*;

    #[test]
    fn there_is_no_pointer_until_the_agent_points() {
        let now = Instant::now();
        let mut pointer = AgentPointer::default();
        assert_eq!(pointer.look(now), None);
        // Typing alone says nothing of where.
        pointer.see(AgentInput::Type, now);
        assert_eq!(pointer.look(now), None);
    }

    #[test]
    fn the_pointer_appears_where_the_agent_first_points() {
        let now = Instant::now();
        let mut pointer = AgentPointer::default();
        pointer.see(AgentInput::Move { x: 10., y: 20. }, now);
        let look = pointer.look(now).unwrap();
        assert_eq!(look.pointer, Spot { x: 10., y: 20. });
        assert!((look.opacity - 1.).abs() < f32::EPSILON);
    }

    #[test]
    fn the_pointer_glides_to_where_the_agent_moves_it() {
        let start = Instant::now();
        let mut pointer = AgentPointer::default();
        pointer.see(AgentInput::Move { x: 0., y: 0. }, start);
        pointer.see(AgentInput::Move { x: 100., y: 0. }, start);

        let midway = pointer.look(start + GLIDE / 2).unwrap();
        assert!(50. < midway.pointer.x && midway.pointer.x < 100.);
        assert_eq!(midway.next_change, Duration::ZERO);

        let there = pointer.look(start + GLIDE).unwrap();
        assert_eq!(there.pointer, Spot { x: 100., y: 0. });
        assert_eq!(there.next_change, LINGER - GLIDE);
    }

    #[test]
    fn a_click_spreads_a_ring_once_the_pointer_is_there() {
        let start = Instant::now();
        let mut pointer = AgentPointer::default();
        pointer.see(AgentInput::Move { x: 0., y: 0. }, start);
        // A click is a move, a press and a release, all at once.
        pointer.see(AgentInput::Move { x: 40., y: 30. }, start);
        pointer.see(AgentInput::Press { x: 40., y: 30. }, start);
        pointer.see(AgentInput::Move { x: 40., y: 30. }, start);

        assert_eq!(
            pointer.look(start + GLIDE / 2).unwrap().ripples,
            [] as [Ripple; 0]
        );
        let spreading = pointer.look(start + GLIDE + RIPPLE / 2).unwrap();
        let [ripple] = spreading.ripples[..] else {
            panic!("one ring: {:?}", spreading.ripples);
        };
        assert_eq!(ripple.at, Spot { x: 40., y: 30. });
        assert!(0. < ripple.radius && ripple.radius < RIPPLE_RADIUS);
        assert!(0. < ripple.opacity && ripple.opacity < 1.);

        let done = pointer.look(start + GLIDE + RIPPLE).unwrap();
        assert_eq!(done.ripples, [] as [Ripple; 0]);
        assert_eq!(done.next_change, LINGER - GLIDE - RIPPLE);
    }

    #[test]
    fn typing_outlines_the_field_and_brings_the_pointer_to_it() {
        let start = Instant::now();
        let mut pointer = AgentPointer::default();
        pointer.see(AgentInput::Move { x: 0., y: 0. }, start);
        pointer.see(AgentInput::Type, start);
        let field = Rect {
            x: 100.,
            y: 50.,
            width: 200.,
            height: 20.,
        };
        pointer.see(
            AgentInput::Field {
                x: field.x,
                y: field.y,
                width: field.width,
                height: field.height,
            },
            start,
        );

        let look = pointer.look(start + GLIDE).unwrap();
        assert!(look.typing);
        assert_eq!(look.field, Some(field));
        assert_eq!(look.pointer, Spot { x: 200., y: 60. });
        assert_eq!(look.next_change, TYPING - GLIDE);

        let done = pointer.look(start + TYPING).unwrap();
        assert!(!done.typing);
        assert_eq!(done.field, None);
    }

    #[test]
    fn a_pointer_already_in_the_field_stays_put() {
        let start = Instant::now();
        let mut pointer = AgentPointer::default();
        pointer.see(AgentInput::Press { x: 110., y: 55. }, start);
        pointer.see(AgentInput::Type, start);
        pointer.see(
            AgentInput::Field {
                x: 100.,
                y: 50.,
                width: 200.,
                height: 20.,
            },
            start,
        );
        assert_eq!(
            pointer.look(start + GLIDE).unwrap().pointer,
            Spot { x: 110., y: 55. }
        );
    }

    #[test]
    fn the_pointer_fades_when_the_agent_leaves_the_page_alone() {
        let start = Instant::now();
        let mut pointer = AgentPointer::default();
        pointer.see(AgentInput::Move { x: 10., y: 20. }, start);

        let fading = pointer.look(start + LINGER + FADE / 2).unwrap();
        assert!(0. < fading.opacity && fading.opacity < 1.);
        assert_eq!(fading.next_change, Duration::ZERO);
        assert_eq!(pointer.look(start + LINGER + FADE), None);

        // Back again, it shows at once where the agent points now.
        let later = start + LINGER * 2;
        pointer.see(AgentInput::Move { x: 300., y: 40. }, later);
        assert_eq!(
            pointer.look(later).unwrap().pointer,
            Spot { x: 300., y: 40. }
        );
    }
}
