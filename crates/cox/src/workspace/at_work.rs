//! Whether a project's chats are at work, as its square on the rail shows
//! it: an arc that runs around the square's edge while one of them works,
//! so that the human sees which projects have work going on without
//! opening them. A chat that asks the human something is shown by the
//! amber count over the square instead, and stills the arc: what waits
//! for them matters more than what goes on by itself.
//!
//! The arc moves in steps, ten a second, rather than at the display's
//! rate: each step draws the whole window again. It moves only while a
//! project on the rail is at work: with none, nothing is drawn again.
//! When macOS is set to reduce motion, the arc stands still.

use std::time::Duration;

use gpui::{Context, Svg, prelude::*, px, rgb, svg};

use super::{Chat, PROJECT_RADIUS, PROJECT_SIZE, Workspace};
use coxd::chat::conversation::State;
use coxd::theme::ACCENT;

/// How a project's chats stand, as far as its square shows: the strongest
/// of how each of its chats does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Activity {
    /// None of its chats is at work.
    #[default]
    Idle,
    /// One of its chats is at work: in a turn, or in the background.
    AtWork,
    /// One of its chats asks the human something.
    Asks,
}

impl Activity {
    /// How a chat in `state` stands, `at_work` or not.
    fn of(state: State, at_work: bool) -> Self {
        if state == State::Asking {
            Self::Asks
        } else if at_work {
            Self::AtWork
        } else {
            Self::Idle
        }
    }

    /// The strongest of how each of `chats` stands, or idle with none.
    fn of_all(chats: impl IntoIterator<Item = Self>) -> Self {
        chats.into_iter().max().unwrap_or_default()
    }
}

/// How long the arc rests at each step.
const STEP: Duration = Duration::from_millis(100);

/// How many steps the arc takes to go once around the square.
const STEPS: u16 = 30;

/// How far the arc stands off the square, and how thick it is.
const GAP: f32 = 3.;
const THICKNESS: f32 = 2.;

/// How much of the square's edge the arc covers.
const SHARE: f32 = 0.28;

impl Workspace {
    /// How the chats of the project at `index` stand, the archived ones
    /// too, which work all the same.
    pub(super) fn activity(&self, index: usize) -> Activity {
        Activity::of_all(
            self.chats
                .iter()
                .chain(&self.archived)
                .filter(|chat| chat.project == index)
                .map(Chat::activity),
        )
    }

    /// Whether a project on the rail is at work, so that its arc moves.
    fn rail_moves(&self) -> bool {
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, project)| !project.archived && !project.deleted)
            .any(|(index, _)| self.activity(index) == Activity::AtWork)
    }

    /// Moves the arcs on the rail a step at a time while a project on it
    /// is at work, unless they stand still, and stops once none is.
    pub(super) fn keep_rail_moving(&mut self, cx: &mut Context<Self>) {
        if self.rail_moving || reduce_motion() || !self.rail_moves() {
            return;
        }
        self.rail_moving = true;
        cx.spawn(async move |workspace, cx| {
            loop {
                cx.background_executor().timer(STEP).await;
                let moves = workspace.update(cx, |workspace, cx| {
                    workspace.rail_moving = workspace.rail_moves();
                    if workspace.rail_moving {
                        workspace.rail_step = (workspace.rail_step + 1) % STEPS;
                        cx.notify();
                    }
                    workspace.rail_moving
                });
                if !matches!(moves, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// The arc around the square of a project at work, at the step the
    /// rail is at, or `None` for a project not at work.
    pub(super) fn arc(&self, index: usize) -> Option<Svg> {
        (self.activity(index) == Activity::AtWork).then(|| {
            svg()
                .path(arc_path(self.rail_step))
                .absolute()
                .top(px(-GAP))
                .left(px(-GAP))
                .size(px(PROJECT_SIZE + 2. * GAP))
                .text_color(rgb(ACCENT))
        })
    }
}

impl Chat {
    /// How it stands, as its project's square shows.
    fn activity(&self) -> Activity {
        Activity::of(self.state, self.at_work())
    }
}

/// Where the arc's SVG is among the assets, at `step`.
fn arc_path(step: u16) -> String {
    format!("rail-arc/{step}.svg")
}

/// The arc's SVG for the asset at `path`, if it is one: a stretch of the
/// rounded square's edge, as far along it as its step says.
#[must_use]
pub fn arc_file(path: &str) -> Option<String> {
    let step: u16 = path
        .strip_prefix("rail-arc/")?
        .strip_suffix(".svg")?
        .parse()
        .ok()?;
    let size = PROJECT_SIZE + 2. * GAP;
    let inset = THICKNESS / 2.;
    let width = size - THICKNESS;
    let radius = PROJECT_RADIUS + GAP - inset;
    let around = 4. * (width - 2. * radius) + 2. * std::f32::consts::PI * radius;
    let arc = around * SHARE;
    let offset = -around * f32::from(step) / f32::from(STEPS);
    Some(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}"><rect x="{inset}" y="{inset}" width="{width}" height="{width}" rx="{radius}" fill="none" stroke="black" stroke-width="{THICKNESS}" stroke-linecap="round" stroke-dasharray="{arc} {gap}" stroke-dashoffset="{offset}"/></svg>"#,
        gap = around - arc,
    ))
}

/// Whether macOS is set to reduce motion.
#[cfg(target_os = "macos")]
fn reduce_motion() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

#[cfg(not(target_os = "macos"))]
fn reduce_motion() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_that_asks_stands_above_one_at_work_and_one_at_work_above_one_at_rest() {
        assert_eq!(Activity::of(State::Asking, true), Activity::Asks);
        assert_eq!(Activity::of(State::Busy, true), Activity::AtWork);
        assert_eq!(Activity::of(State::Idle, true), Activity::AtWork);
        assert_eq!(Activity::of(State::Idle, false), Activity::Idle);
        assert_eq!(Activity::of(State::Closed, false), Activity::Idle);
    }

    #[test]
    fn a_project_stands_as_the_strongest_of_its_chats() {
        use Activity::{Asks, AtWork, Idle};
        assert_eq!(Activity::of_all([]), Idle);
        assert_eq!(Activity::of_all([Idle, Idle]), Idle);
        assert_eq!(Activity::of_all([Idle, AtWork, Idle]), AtWork);
        assert_eq!(Activity::of_all([AtWork, Asks, Idle]), Asks);
    }

    #[test]
    fn each_step_of_the_arc_is_an_svg_and_nothing_else_is() {
        for step in 0..STEPS {
            let file = arc_file(&arc_path(step)).unwrap();
            assert!(file.starts_with("<svg"), "{file}");
        }
        assert_eq!(arc_file("rail-arc/x.svg"), None);
        assert_eq!(arc_file("icons/bell.svg"), None);
    }
}
