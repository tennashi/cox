//! How the window draws itself: the rail, the stage and the panel side by
//! side, and the settings or a project's card over them.

use gpui::{
    App, Context, Div, FocusHandle, MouseButton, Window, deferred, div, prelude::*, px, rgb,
};

use super::{BACKDROP, NewChat, NewProject, NextChat, PreviousChat, ToggleChat, Workspace};
use crate::chat::region::{self, AddRegionToChat};
use crate::settings::view::OpenSettings;
use crate::stage::{NewPage, NewTerminal, OpenFile, Stage};
use crate::ui::card;
use crate::ui::theme::{
    CARD_INNER_RADIUS, CARD_RADIUS, FOCUSED_EDGE, FOREGROUND, GAP, GROUND, UI_FONT,
};
use crate::vim::hints::{self, Part};

impl Workspace {
    /// The settings screen, over the window and dimming it, while it is
    /// open: a click beside it closes it.
    fn render_settings(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (view, _) = self.settings.as_ref()?;
        Some(
            deferred(
                div()
                    .id("settings-backdrop")
                    .child(hints::cover())
                    .child(hints::part(Part::Settings))
                    .occlude()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(GAP * 2.))
                    .bg(gpui::rgba(BACKDROP))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|workspace, _, window, cx| {
                            workspace.close_settings(window, cx);
                        }),
                    )
                    .child(view.clone()),
            )
            .with_priority(3),
        )
    }

    /// The card of a project, over the window and dimming it, while it is
    /// open: a click beside it closes it.
    fn render_project_card(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (card, _) = self.project_card.as_ref()?;
        Some(
            deferred(
                div()
                    .id("project-card-backdrop")
                    .child(hints::cover())
                    .child(hints::part(Part::ProjectCard))
                    .occlude()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(GAP * 2.))
                    .bg(gpui::rgba(BACKDROP))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|workspace, _, window, cx| {
                            workspace.close_project_card(window, cx);
                        }),
                    )
                    .child(card.clone()),
            )
            .with_priority(3),
        )
    }
}

/// A lighter edge over the part `card` tracks, the rail, the stage's card
/// or the chat's, while it holds the keyboard and Vim's keys, which move
/// the keyboard between them, are on. It lies over what the part shows, a
/// page's picture among it, inside a card's own edge.
pub(super) fn keys_edge(card: &FocusHandle, window: &Window, cx: &App) -> Option<Div> {
    (crate::vim::mode(cx).is_some() && card.contains_focused(window, cx)).then(|| {
        div()
            .debug_selector(|| "keys-edge".to_owned())
            .absolute()
            .inset_0()
            .rounded(px(CARD_INNER_RADIUS))
            .border_1()
            .border_color(rgb(FOCUSED_EDGE))
    })
}

impl Workspace {
    /// The stage of the chat shown, or an empty card until it has one.
    fn render_stage(&self, window: &Window, cx: &App) -> Div {
        card()
            .track_focus(&self.stage_card)
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .map(|card| match self.centered() {
                Some(stage) => card
                    .debug_selector(|| "stage-panel".to_owned())
                    .child(stage.view.clone()),
                None => card,
            })
            .children(keys_edge(&self.stage_card, window, cx))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The parts mark again where they are drawn, for a region dragged.
        region::forget_marks();
        hints::forget_marks(window);
        let stage = self.render_stage(window, cx);
        let chat = if self.folded {
            self.render_folded(cx).into_any_element()
        } else {
            self.render_panel(window, cx).into_any_element()
        };
        div()
            .track_focus(&self.focus_handle)
            .key_context("Workspace")
            // GPUI still hands a key an interceptor stopped to the first key
            // listener on the focus path. This empty one is first, so the
            // conversation's, which answers the agent's options, never sees it.
            .capture_key_down(|_, _, _| {})
            .on_action(cx.listener(|workspace, _: &NewChat, window, cx| {
                workspace.open(None, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &NewProject, _, _| {
                workspace.new_project();
            }))
            .on_action(cx.listener(|workspace, _: &OpenSettings, window, cx| {
                workspace.open_settings(window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &NextChat, window, cx| {
                workspace.step(1, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &PreviousChat, window, cx| {
                workspace.step(-1, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &NewPage, window, cx| {
                workspace.on_stage(window, cx, Stage::new_tab);
            }))
            .on_action(cx.listener(|workspace, _: &NewTerminal, window, cx| {
                workspace.on_stage(window, cx, Stage::new_terminal);
            }))
            .on_action(cx.listener(|workspace, _: &ToggleChat, window, cx| {
                workspace.fold(!workspace.folded, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &OpenFile, window, cx| {
                workspace.on_stage(window, cx, Stage::new_files);
            }))
            .on_action(cx.listener(|workspace, _: &AddRegionToChat, window, cx| {
                workspace.lay_sheet(window, cx);
            }))
            .on_action(cx.listener(Self::open_line))
            .on_action(cx.listener(Self::run))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(GROUND))
            .text_color(rgb(FOREGROUND))
            .font_family(UI_FONT)
            .text_size(px(14.))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .relative()
                            .h_full()
                            .flex_shrink_0()
                            .child(hints::part(Part::Rail))
                            .child(self.render_rail(window, cx))
                            .children(keys_edge(&self.rail.focus, window, cx).map(|edge| {
                                // The rail is not a card: its edge is drawn
                                // as far in as the cards are from the
                                // window's edges.
                                edge.top(px(GAP))
                                    .bottom(px(GAP))
                                    .left(px(GAP))
                                    .right(px(GAP))
                                    .rounded(px(CARD_RADIUS))
                            })),
                    )
                    // The stage, and the chat at its right, as cards on the
                    // ground the rail is on.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .gap(px(GAP))
                            .py(px(GAP))
                            .pr(px(GAP))
                            .child(stage)
                            .child(chat),
                    ),
            )
            .children(self.render_menu(cx))
            .children(self.render_project_menu(cx))
            .children(self.render_archived_projects(cx))
            .children(self.render_project_card(cx))
            .children(self.render_settings(cx))
            .children(self.render_sheet(cx))
            .children(
                cx.try_global::<crate::ui::translation::Shown>()
                    .and_then(|shown| shown.0.clone()),
            )
            .child(self.line.clone())
            .children(hints::overlay(window))
    }
}
