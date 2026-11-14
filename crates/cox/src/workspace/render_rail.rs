//! The rail of projects down the window's left side, its buttons and
//! badges, and the menu of a project on it.

use gpui::{
    Context, Div, FontWeight, MouseButton, MouseDownEvent, Pixels, SharedString, Stateful, Window,
    anchored, deferred, div, img, prelude::*, px, rgb,
};

use super::{PROJECT_RADIUS, PROJECT_SIZE, Project, RAIL, RAIL_WIDTH, Workspace};
use crate::chat::conversation::State;
use crate::ui::ellipsis::ellipsis;
use crate::ui::icons::{Icon, app_icon, icon};
use crate::ui::paths::shortened;
use crate::ui::theme::{
    ACCENT, ACCENT_GROUND, ASKING, BORDER, BORDER_STRONG, FAINT, FOREGROUND, GROUND, HOVER, LINK,
    SELECTED, SURFACE,
};
use crate::ui::tip;
use crate::vim::hints::Hinted as _;

impl Workspace {
    /// The rail down the window's left side: a button for each project,
    /// which chooses it, marked when it is the one chosen and counting its
    /// chats that wait for the human, and at its foot the button that
    /// opens a folder, and the gear that opens the settings.
    pub(super) fn render_rail(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Cox's own, a place of its own rather than a project, stands apart
        // from the projects under it, past a line.
        // A project put away, or deleted, is off the rail.
        let shown = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, project)| !project.archived && !project.deleted);
        let picked = self.rail.picked(shown.clone().count(), window);
        let projects = shown
            .enumerate()
            .flat_map(|(row, (index, project))| {
                let apart = project.id.is_none().then(|| {
                    div()
                        .flex_shrink_0()
                        .w(px(PROJECT_SIZE * 0.6))
                        .h(px(1.))
                        .bg(rgb(BORDER_STRONG))
                        .into_any_element()
                });
                let button = self
                    .render_project(index, project, cx)
                    .when(picked == Some(row), |button| {
                        button.child(crate::ui::picked(PROJECT_RADIUS))
                    });
                std::iter::once(button.into_any_element()).chain(apart)
            })
            .collect::<Vec<_>>();
        div()
            .id("projects")
            // With Vim's keys only: a click would take the keyboard here.
            .when(crate::vim::mode(cx).is_some(), |rail| {
                rail.track_focus(&self.rail.focus).key_context(RAIL)
            })
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .py(px(16.))
            .overflow_y_scroll()
            .children(projects)
            .child(div().flex_1())
            .children(self.render_archived_projects_button(cx))
            .child(
                div()
                    .id("new-project")
                    .debug_selector(|| "new-project".to_owned())
                    .flex_shrink_0()
                    .size(px(PROJECT_SIZE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(PROJECT_RADIUS))
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(BORDER_STRONG))
                    .text_color(rgb(FAINT))
                    .text_size(px(18.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(HOVER)).text_color(rgb(FOREGROUND)))
                    .tooltip(tip("New project"))
                    .child(icon(Icon::Plus, 16., FAINT))
                    .on_hint_click(cx.listener(|workspace, _, _, _| {
                        workspace.new_project();
                    })),
            )
            .child(self.render_settings_button(cx))
    }

    /// The button at the foot of the rail that opens the menu of the
    /// projects put away, while there are any, lit while it is open.
    fn render_archived_projects_button(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let count = self
            .projects
            .iter()
            .filter(|project| project.archived && !project.deleted)
            .count();
        if count == 0 {
            return None;
        }
        let open = self.archived_projects.is_some();
        let ink = if open { FOREGROUND } else { FAINT };
        Some(
            div()
                .id("archived-projects")
                .debug_selector(|| "archived-projects".to_owned())
                .flex_shrink_0()
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .cursor_pointer()
                .when(open, |button| button.bg(rgb(SELECTED)))
                .hover(|style| style.bg(rgb(HOVER)))
                // Not over its menu, while that is open.
                .when(!open, |button| {
                    button.tooltip(tip(format!("Archived projects ({count})")))
                })
                .child(icon(Icon::Inbox, 18., ink))
                .on_hint_mouse_down(cx.listener(|workspace, event: &MouseDownEvent, _, cx| {
                    workspace.archived_projects = match workspace.archived_projects {
                        Some(_) => None,
                        None => Some(event.position),
                    };
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
    }

    /// The menu of the projects put away, if it is open: a click on one
    /// brings it back to the rail, and chooses it.
    pub(super) fn render_archived_projects(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let at = self.archived_projects?;
        let rows = self
            .projects
            .iter()
            .enumerate()
            .filter(|(_, project)| project.archived && !project.deleted)
            .map(|(index, project)| {
                crate::ui::menu::item(("archived-project", index), project.name.clone(), "")
                    .tooltip(tip(format!(
                        "{}\nBring it back to the rail",
                        shortened(&project.folder)
                    )))
                    .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.archive_project(index, false, window, cx);
                    }))
            });
        let menu = crate::ui::menu::menu()
            .min_w(px(160.))
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| {
                workspace.archived_projects = None;
                cx.notify();
            }))
            .children(rows);
        // It opens upward, from the foot of the rail.
        Some(
            deferred(
                anchored()
                    .anchor(gpui::Corner::BottomLeft)
                    .position(at)
                    .child(menu),
            )
            .with_priority(2),
        )
    }

    /// The gear at the foot of the rail, which opens the settings, lit
    /// while they are open.
    fn render_settings_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let open = self.settings.is_some();
        let key = if cfg!(target_os = "macos") {
            "⌘,"
        } else {
            "Ctrl+,"
        };
        let ink = if open { FOREGROUND } else { FAINT };
        div()
            .id("open-settings")
            .debug_selector(|| "open-settings".to_owned())
            .flex_shrink_0()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .cursor_pointer()
            .when(open, |button| button.bg(rgb(SELECTED)))
            .hover(|style| style.bg(rgb(HOVER)))
            .tooltip(tip(format!("Settings ({key})")))
            .child(icon(Icon::Settings, 18., ink))
            .on_hint_click(cx.listener(|workspace, _, window, cx| {
                workspace.open_settings(window, cx);
            }))
    }

    /// The button of the project at `index` on the rail: its icon, or a
    /// folder, in a square, and its name under it, or Cox's icon alone for
    /// Cox's own, with its folder where the mouse rests, after its name
    /// for Cox's own. The right button opens its menu, but on Cox's own.
    fn render_project(
        &self,
        index: usize,
        project: &Project,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let chosen = self.chosen == index;
        let chats = self.chats.iter().filter(|chat| chat.project == index);
        let asking = chats
            .clone()
            .filter(|chat| chat.state == State::Asking)
            .count();
        let waiting = chats
            .filter(|chat| chat.unread || chat.state == State::Asking)
            .count();
        let label = if chosen { LINK } else { FAINT };
        let folder = shortened(&project.folder);
        let home = project.id.is_none();
        let drawing = project.id.is_some_and(|id| self.drawing.contains(&id));
        div()
            .id(("project", index))
            .debug_selector(move || format!("project-{}", index + 1))
            .relative()
            .flex_shrink_0()
            .w(px(RAIL_WIDTH))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(5.))
            .cursor_pointer()
            // Not over its menu, while that is open.
            .when(self.project_menu.is_none(), |button| {
                button.tooltip(tip(if home {
                    format!("{} ({folder})", project.name)
                } else if project.missing {
                    format!("{folder}\nThis folder is not there: Edit project… says where it went.")
                } else if drawing {
                    format!("{folder}\nAn agent draws its icon…")
                } else if let Some(why) = project.id.and_then(|id| self.not_drawn.get(&id)) {
                    format!("{folder}\nNo icon was drawn: {why}")
                } else {
                    folder
                }))
            })
            .on_hint_click(cx.listener(move |workspace, _, window, cx| {
                workspace.choose(index, window, cx);
            }))
            .when(!home, |button| {
                button.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                        workspace.project_menu = Some((event.position, index));
                        workspace.deleting = None;
                        cx.notify();
                    }),
                )
            })
            .when(chosen, |button| {
                button.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(10.))
                        .w(px(3.))
                        .h(px(22.))
                        .rounded_r(px(2.))
                        .bg(rgb(ACCENT)),
                )
            })
            // The badges are the square's siblings, not its children: gpui
            // paints a div's border over its children, and the square's
            // would cross them.
            .child(
                div()
                    .relative()
                    .size(px(PROJECT_SIZE))
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .map(|square| project_square(square, project, home, chosen, drawing)),
                    )
                    .when(waiting > 0, |square| {
                        square.child(waiting_badge(index, waiting, asking > 0))
                    })
                    .when(project.missing, |square| square.child(missing_badge(index))),
            )
            .when(!home, |button| {
                button.child(
                    div()
                        .max_w(px(RAIL_WIDTH - 8.))
                        .overflow_hidden()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(label))
                        .child(ellipsis(project.name.clone())),
                )
            })
    }

    /// The menu of the right button on a project of the rail, if it is
    /// open: to edit it, to put it away, and to delete it.
    pub(super) fn render_project_menu(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let (at, index) = self.project_menu?;
        self.projects.get(index)?;
        // Deleting takes a second click, so that one by mistake does not.
        let delete = match self.deleting {
            Some(deleting) if deleting == index && self.project_at_work(index) => {
                "A chat of it is at work: delete it once the turn ends"
            }
            Some(deleting) if deleting == index => "Click again to move it to the Trash",
            _ => "Delete project…",
        };
        let item = |id: &'static str, label: &'static str| crate::ui::menu::item(id, label, "");
        let menu = crate::ui::menu::menu()
            .min_w(px(160.))
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| {
                workspace.project_menu = None;
                cx.notify();
            }))
            .child(
                item("edit-project", "Edit project…").on_hint_click(cx.listener(
                    move |workspace, _, window, cx| workspace.open_project_card(index, window, cx),
                )),
            )
            .child(
                item("archive-project", "Archive project").on_hint_click(cx.listener(
                    move |workspace, _, window, cx| {
                        workspace.archive_project(index, true, window, cx);
                    },
                )),
            )
            .child(item("delete-project", delete).on_hint_click(cx.listener(
                move |workspace, _, window, cx| {
                    workspace.delete_project(index, window, cx);
                },
            )));
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}

/// What the square of `project` on the rail shows: Cox's icon for Cox's
/// own, the home's, or else the icon the human gave the project, filling
/// the square inside its edge, or else a folder; the edge marks it
/// chosen, and, dashed in the accent, that an agent draws it an icon.
fn project_square(square: Div, project: &Project, home: bool, chosen: bool, drawing: bool) -> Div {
    let (ground, edge, ink) = if chosen {
        (ACCENT_GROUND, ACCENT, ACCENT)
    } else {
        (SURFACE, BORDER, FAINT)
    };
    if home {
        return square.child(app_icon(PROJECT_SIZE));
    }
    let square = square
        .rounded(px(PROJECT_RADIUS))
        .border_1()
        .border_color(rgb(edge))
        .when(drawing, |square| {
            square.border_dashed().border_color(rgb(ACCENT))
        })
        .when(project.missing, |square| {
            square
                .border_dashed()
                .border_color(rgb(ASKING))
                .opacity(0.7)
        });
    match project.icon.clone() {
        Some(picture) => square.child(
            img(picture)
                .size(px(PROJECT_SIZE - 2.))
                .rounded(px(PROJECT_RADIUS - 1.)),
        ),
        None => square
            .bg(rgb(ground))
            .when(!chosen, |square| square.hover(|style| style.bg(rgb(HOVER))))
            .child(icon(Icon::Folder, 18., ink)),
    }
}

/// The mark over the bottom right of the square of the project at
/// `index`, on the rail, whose folder is not there.
fn missing_badge(index: usize) -> Div {
    badge(ASKING, px(11.), "?")
        .debug_selector(move || format!("project-{}-missing", index + 1))
        .bottom(px(-5.))
        .right(px(-5.))
}

/// The count over the top right of the project's square, on the rail, at
/// `index`: how many of its chats wait for the human or that they have
/// not seen, in amber when one of them `asks`.
fn waiting_badge(index: usize, waiting: usize, asks: bool) -> Div {
    badge(
        if asks { ASKING } else { ACCENT },
        px(10.),
        waiting.to_string(),
    )
    .debug_selector(move || format!("project-{}-waiting", index + 1))
    .top(px(-5.))
    .right(px(-5.))
}

/// A badge over a corner of a project's square, placed by the caller: a
/// disc in `color`, or a pill when `label` is wider than a digit, ringed
/// in the ground so that it stands off the square.
///
/// The ring is a disc of the ground under the colored one, not a border:
/// gpui smooths the edges of a border and of the background under it
/// apart, and the background's color would show as a thin halo outside
/// the ring.
fn badge(color: u32, size: Pixels, label: impl Into<SharedString>) -> Div {
    div()
        .absolute()
        .p(px(2.))
        .rounded_full()
        .bg(rgb(GROUND))
        .child(
            div()
                .min_w(px(13.))
                .h(px(13.))
                .px(px(3.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(color))
                .text_color(rgb(GROUND))
                .text_size(size)
                .font_weight(FontWeight::BOLD)
                .child(label.into()),
        )
}
