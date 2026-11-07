//! The column along the left of the stage, under its bar: the tabs that
//! are not views of the project, one under another in the order they
//! were opened, each with an icon of what it shows and its title, and
//! those of the chats not shown dimmer. A tab an agent is at work on is
//! marked with a ring of a color of its chat's own, and its icon in it.
//! Folded, the column is the tabs' icons alone, named where the mouse
//! rests, which every stage folds or unfolds together.

use std::cell::Cell;

use gpui::{
    App, Context, Div, Global, MouseButton, MouseDownEvent, ScrollHandle, SharedString, Stateful,
    div, prelude::*, px, rgb,
};

use super::tab::{Tab, TabContent};
use super::{Stage, TITLE_LENGTH};
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, icon};
use crate::ui::theme::{BORDER, BUTTON_HOVER, DIM, FAINT, FOREGROUND, HOVER, SELECTED, chat_color};
use crate::ui::tip;

/// How wide the column is, unfolded and folded.
const WIDTH: f32 = 200.;
const FOLDED_WIDTH: f32 = 44.;

/// How tall a tab's row is, and how round.
const ROW_HEIGHT: f32 = 28.;
const ROW_RADIUS: f32 = 8.;

/// Where the tabs scroll down the column, when there are more than it
/// has room for, which keeps the tab shown in sight as another is shown.
#[derive(Default)]
pub(super) struct Strip {
    scroll: ScrollHandle,
    /// The tab the strip last scrolled to, so that it scrolls once each
    /// time another is shown, and the human can scroll it after.
    shown: Cell<Option<usize>>,
}

impl Strip {
    /// Scrolls to the tab shown, `shown`, at `place` among the rows the
    /// strip has, unless it scrolled to it already.
    fn follow(&self, shown: Option<usize>, place: Option<usize>) {
        if self.shown.replace(shown) == shown {
            return;
        }
        if let Some(place) = place {
            self.scroll.scroll_to_item(place);
        }
    }
}

/// Whether the column is folded, the same for every stage, for as long
/// as Cox runs.
#[derive(Default)]
struct Folded(bool);

impl Global for Folded {}

fn is_folded(cx: &App) -> bool {
    cx.try_global::<Folded>().is_some_and(|folded| folded.0)
}

impl Stage {
    pub(super) fn render_column(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let folded = is_folded(cx);
        let rest = self.rest(cx);
        let shown = self.tabs.shown_index();
        self.strip
            .follow(shown, rest.iter().position(|&index| Some(index) == shown));
        let rows: Vec<_> = rest
            .into_iter()
            .map(|index| self.render_row(index, &self.tabs[index], folded, cx))
            .collect();
        div()
            .id("stage-column")
            .debug_selector(|| "stage-column".to_owned())
            .flex_shrink_0()
            .w(px(if folded { FOLDED_WIDTH } else { WIDTH }))
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .id("stage-tabs")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .px(px(if folded { 6. } else { 8. }))
                    .pt(px(4.))
                    .pb(px(8.))
                    .overflow_y_scroll()
                    .track_scroll(&self.strip.scroll)
                    .children(rows),
            )
            .child(
                div()
                    .id("fold-tabs")
                    .debug_selector(|| "fold-tabs".to_owned())
                    .flex_shrink_0()
                    .m(px(6.))
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ROW_RADIUS))
                    .text_size(px(15.))
                    .text_color(rgb(FAINT))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                    .tooltip(tip(if folded {
                        "Unfold the tabs"
                    } else {
                        "Fold the tabs"
                    }))
                    .child(if folded { "›" } else { "‹" })
                    .on_click(cx.listener(|_, _, _, cx| {
                        let folded = !is_folded(cx);
                        cx.set_global(Folded(folded));
                        cx.refresh_windows();
                    })),
            )
    }

    /// The row of the tab at `index`: an icon of what it shows, its
    /// title, and, while a chat's agent is at work on it, its icon in the
    /// chat's color and a ring of it, which names the chat where the mouse
    /// rests, and, where the mouse rests or for the tab shown, the button
    /// that closes it, unless it is the last; folded, its icon alone. A
    /// chat's that is not shown is dimmer, unless an agent is at work on
    /// it.
    fn render_row(
        &self,
        index: usize,
        tab: &Tab,
        folded: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let number = index + 1;
        let title = tab.content.title(cx);
        let title: String = title.chars().take(TITLE_LENGTH).collect();
        let glyph = glyph(&tab.content);
        let shown = self.tabs.shown_index() == Some(index);
        let others = tab.owner.is_some_and(|owner| Some(owner) != self.showing);
        let worker = self.at_work_on(tab);
        let at_work = worker.is_some();
        let group: SharedString = format!("tab-row-{number}").into();
        let row = div()
            .id(("tab", number))
            .debug_selector(move || format!("tab-{number}"))
            .group(group.clone())
            .flex_shrink_0()
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .rounded(px(ROW_RADIUS))
            .text_size(px(13.))
            .cursor_pointer()
            .map(|row| {
                if shown {
                    row.bg(rgb(SELECTED)).text_color(rgb(FOREGROUND))
                } else {
                    row.text_color(rgb(DIM)).hover(|style| style.bg(rgb(HOVER)))
                }
            })
            .when(others && !at_work, |row| row.opacity(0.5))
            .on_click(cx.listener(move |stage, _, window, cx| stage.show(index, window, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |stage, event: &MouseDownEvent, _, cx| {
                    stage.open_tab_menu(index, event.position, cx);
                }),
            )
            .child(icon(
                glyph,
                14.,
                match worker {
                    Some(chat) => chat_color(chat),
                    None if shown => DIM,
                    None => FAINT,
                },
            ));
        if folded {
            return row.w(px(32.)).justify_center().tooltip(tip(title));
        }
        row.pl(px(8.))
            .pr(px(4.))
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(ellipsis(title)),
            )
            .when_some(worker, |row, chat| {
                row.child(
                    div()
                        .id(("tab-at-work", number))
                        .debug_selector(move || format!("tab-{number}-at-work"))
                        .flex_shrink_0()
                        .size(px(7.))
                        .rounded_full()
                        .border(px(1.5))
                        .border_color(rgb(chat_color(chat)))
                        .tooltip(tip(self.name_of(chat, cx))),
                )
            })
            .when(self.tabs.len() > 1, |row| {
                row.child(Self::render_close(index, shown, group, cx))
            })
    }

    /// The button that closes the tab at `index`, in sight for the tab
    /// shown, or else where the mouse rests on its row, `group`.
    fn render_close(
        index: usize,
        shown: bool,
        group: SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let number = index + 1;
        div()
            .id(("close-tab", number))
            .debug_selector(move || format!("close-tab-{number}"))
            .flex_shrink_0()
            .size(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .when(!shown, |close| {
                close
                    .opacity(0.)
                    .group_hover(group, |style| style.opacity(1.))
            })
            .hover(|style| style.bg(rgb(BUTTON_HOVER)))
            .child(icon(Icon::Close, 10., FAINT))
            .on_click(cx.listener(move |stage, _, window, cx| {
                cx.stop_propagation();
                let _ = stage.close_tab(index, window, cx);
            }))
    }
}

/// The icon of what a tab shows.
fn glyph(content: &TabContent) -> Icon {
    match content {
        TabContent::Page { .. } => Icon::Globe,
        TabContent::Document(_) => Icon::Document,
        TabContent::Code(_) => Icon::Code,
        TabContent::Tree(_) => Icon::Files,
        TabContent::Changes(_) => Icon::Changes,
        TabContent::Flow(_) => Icon::Flow,
        TabContent::Terminal(_) => Icon::Terminal,
        TabContent::Android(_) | TabContent::Devices(_) => Icon::Phone,
        TabContent::MacWindow(_) | TabContent::Windows(_) => Icon::Window,
        TabContent::Workflow(_) => Icon::Settings,
    }
}
