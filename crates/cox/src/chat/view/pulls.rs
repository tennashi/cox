//! The bar over the conversation of the pull requests the chat's agent
//! opened, each with how it stands and how its CI does, for the human to
//! see that the chat waits on CI, and to open the pull request, or a job
//! of its CI, on the stage.

use gpui::{AnyElement, Context, SharedString, div, prelude::*, px, rgb};

use super::ChatView;
use crate::ui::ellipsis::ellipsis;
use crate::vim::hints::Hinted as _;
use coxd::daemon::wire::PullRow;
use coxd::github::{Check, CheckState, PullState};
use coxd::theme::{ACCENT, ASKING, CODE_FONT, DIM, DISABLED, FAINT, LINK, RAISED, SURFACE};

/// The pull requests the bar shows of `pulls`: those still open, and the
/// last opened, however it stands, so that a chat with many merged long
/// ago does not show them all.
fn shown(pulls: &[PullRow]) -> impl Iterator<Item = &PullRow> {
    let last = pulls.len().saturating_sub(1);
    pulls.iter().enumerate().filter_map(move |(at, row)| {
        let done = row.status.as_ref().is_some_and(coxd::github::Status::done);
        (!done || at == last).then_some(row)
    })
}

/// How `row` stands, in a word, as "draft", and how its CI does, as
/// "CI running", in words and the color of its mark.
fn said(row: &PullRow) -> (&'static str, Option<(&'static str, u32)>) {
    let Some(status) = &row.status else {
        return ("reading…", None);
    };
    let state = match status.state {
        PullState::Open => "open",
        PullState::Draft => "draft",
        PullState::Merged => "merged",
        PullState::Closed => "closed",
    };
    let ci = status.checks().map(|checks| match checks {
        CheckState::Pending => ("CI running", LINK),
        CheckState::Passed => ("CI passed", ACCENT),
        CheckState::Failed => ("CI failed", ASKING),
        CheckState::Skipped => ("CI skipped", DISABLED),
    });
    (state, ci)
}

/// A round mark of how a check stands: hollow while it runs, filled once
/// it ended.
fn mark(state: CheckState) -> gpui::Div {
    let (color, filled) = match state {
        CheckState::Pending => (LINK, false),
        CheckState::Passed => (ACCENT, true),
        CheckState::Failed => (ASKING, true),
        CheckState::Skipped => (DISABLED, true),
    };
    div()
        .flex_shrink_0()
        .size(px(7.))
        .rounded_full()
        .border_1()
        .border_color(rgb(color))
        .when(filled, |mark| mark.bg(rgb(color)))
}

/// The chip of `check`, the `at`th of the pull request `number`: its
/// mark and name, which opens its job where it has a link.
fn chip(number: u64, at: usize, check: &Check) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(format!("pull-{number}-check-{at}")))
        .debug_selector(move || format!("pull-{number}-check-{at}"))
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(22.))
        .px(px(8.))
        .max_w(px(240.))
        .rounded_full()
        .bg(rgb(SURFACE))
        .text_size(px(11.5))
        .font_family(CODE_FONT)
        .text_color(rgb(FAINT))
        .when(check.link.is_some(), |chip| {
            chip.cursor_pointer()
                .hover(|style| style.text_color(rgb(DIM)))
        })
        .child(mark(check.state))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .child(ellipsis(SharedString::from(check.name.clone()))),
        )
}

impl ChatView {
    /// The bar of the pull requests the chat's agent opened, if it opened
    /// any.
    pub(super) fn render_pulls(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rows: Vec<AnyElement> = shown(&self.row.pulls)
            .map(|row| Self::render_pull(row, cx))
            .collect();
        if rows.is_empty() {
            return None;
        }
        Some(
            div()
                .debug_selector(|| "pulls".to_owned())
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .mx(px(12.))
                .mt(px(12.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(12.))
                .bg(rgb(RAISED))
                .children(rows)
                .into_any_element(),
        )
    }

    /// The line of the pull request `row`: its number and title, which
    /// open it, how it stands, and a mark for each check, which opens its
    /// job.
    fn render_pull(row: &PullRow, cx: &mut Context<Self>) -> AnyElement {
        let number = row.pull.number;
        let url = row.pull.url();
        let (state, ci) = said(row);
        let title = row
            .status
            .as_ref()
            .map_or_default(|status| status.title.clone());
        let checks = row.status.iter().flat_map(|status| &status.checks);
        let chips: Vec<_> = checks
            .enumerate()
            .map(|(at, check)| {
                let link = check.link.clone();
                chip(number, at, check).on_hint_click(cx.listener(move |view, _, window, cx| {
                    if let Some(link) = &link {
                        view.show_link(link, window, cx);
                    }
                }))
            })
            .collect();
        let head = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.))
            .text_size(px(13.))
            .child(
                div()
                    .id(SharedString::from(format!("pull-{number}")))
                    .debug_selector(move || format!("pull-{number}"))
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .hover(gpui::Styled::underline)
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(LINK))
                            .child(format!("PR #{number}")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(ellipsis(SharedString::from(title))),
                    )
                    .on_hint_click(cx.listener(move |view, _, window, cx| {
                        view.show_link(&url, window, cx);
                    })),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(rgb(FAINT))
                    .child(state),
            )
            .children(ci.map(|(said, color)| {
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(rgb(color))
                    .child(said)
            }));
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(head)
            .when(!chips.is_empty(), |line| {
                line.child(div().flex().flex_wrap().gap(px(6.)).children(chips))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coxd::github::{Pull, Status};

    fn row(number: u64, state: PullState) -> PullRow {
        PullRow {
            pull: Pull {
                repo: "tennashi/cox".to_owned(),
                number,
            },
            status: Some(Status {
                title: "t".to_owned(),
                state,
                head: "a".to_owned(),
                checks: Vec::new(),
                times: coxd::github::Times::default(),
            }),
            unseen: None,
        }
    }

    #[test]
    fn the_bar_shows_the_open_pull_requests_and_the_last_however_it_stands() {
        let pulls = [
            row(1, PullState::Merged),
            row(2, PullState::Draft),
            row(3, PullState::Closed),
            row(4, PullState::Merged),
        ];
        let numbers: Vec<u64> = shown(&pulls).map(|row| row.pull.number).collect();
        assert_eq!(numbers, [2, 4]);
    }
}
