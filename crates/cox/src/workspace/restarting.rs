//! Restarting Cox from the window (see [`crate::restart`]): the button at
//! the foot of the rail, while something waits for a restart, and its
//! menu, which tells what waits and restarts the window alone, or Cox and
//! coxd, once no chat is at work or now; and the commands of the line
//! that do the same at any time.

use gpui::{
    Context, MouseDownEvent, PromptLevel, Window, anchored, deferred, div, prelude::*, px, rgb,
};

use super::{Chat, Workspace};
use crate::restart::{How, Pending};
use crate::ui::icons::{Icon, icon};
use crate::ui::tip;
use crate::vim::hints::Hinted as _;
use coxd::theme::{ACCENT, ASKING, FAINT, HOVER, SELECTED};

impl Workspace {
    /// The names of the chats at work, which a restart with coxd waits
    /// for, or cuts.
    fn at_work_names(&self) -> Vec<String> {
        self.chats
            .iter()
            .chain(&self.archived)
            .filter(|chat| chat.at_work())
            .map(Chat::name)
            .map(str::to_owned)
            .collect()
    }

    /// Restarts Cox as `how` says, once the human agreed to cut the chats
    /// at work, if it cuts any.
    pub(super) fn restart(&mut self, how: How, window: &mut Window, cx: &mut Context<Self>) {
        self.restart_menu = None;
        let at_work = self.at_work_names();
        if how != How::WithCoxdNow || at_work.is_empty() {
            self.restart_as(how, cx);
            return;
        }
        let detail = format!(
            "coxd stops at once, cutting what these chats are at work on:\n\n{}",
            at_work
                .iter()
                .map(|name| format!("• {name}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let answer = window.prompt(
            PromptLevel::Warning,
            "Restart Cox and coxd now?",
            Some(&detail),
            &["Restart Now", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |workspace, cx| {
            if answer.await == Ok(0) {
                let _ = workspace.update(cx, |workspace, cx| {
                    workspace.restart_as(How::WithCoxdNow, cx);
                });
            }
        })
        .detach();
    }

    fn restart_as(&self, how: How, cx: &mut Context<Self>) {
        if let Err(error) = crate::restart::restart(how, cx) {
            tracing::error!("Cox does not restart: {error:#}");
            self.line.update(cx, |line, cx| {
                line.say(format!("Cox does not restart: {error}"), cx);
            });
        }
    }

    /// The button at the foot of the rail that opens the menu that
    /// restarts Cox, while something waits for a restart: in the accent
    /// when the window alone runs it, in amber when coxd has to restart
    /// too.
    pub(super) fn render_restart_button(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let pending = crate::restart::pending(cx)?;
        let open = self.restart_menu.is_some();
        let ink = if pending.window_runs_it() {
            ACCENT
        } else {
            ASKING
        };
        Some(
            div()
                .id("restart")
                .debug_selector(|| "restart".to_owned())
                .flex_shrink_0()
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .cursor_pointer()
                .when(open, |button| button.bg(rgb(SELECTED)))
                .hover(|style| style.bg(rgb(HOVER)))
                .when(!open, |button| button.tooltip(tip(pending.told())))
                .child(icon(Icon::Restart, 18., ink))
                .on_hint_mouse_down(cx.listener(|workspace, event: &MouseDownEvent, _, cx| {
                    workspace.restart_menu = match workspace.restart_menu {
                        Some(_) => None,
                        None => Some(event.position),
                    };
                    cx.stop_propagation();
                    cx.notify();
                })),
        )
    }

    /// The menu that restarts Cox, if it is open: what waits for a
    /// restart, then the window alone, unless the build installed needs
    /// coxd to restart too, and Cox and coxd, once no chat is at work or
    /// now, each with the chats at work it waits for or cuts.
    pub(super) fn render_restart_menu(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let at = self.restart_menu?;
        let pending = crate::restart::pending(cx);
        let at_work = self.at_work_names().len();
        let chats = |count: usize| match count {
            1 => "1 chat".to_owned(),
            count => format!("{count} chats"),
        };
        let item =
            |id: &'static str, label: String, keys: &str| crate::ui::menu::item(id, label, keys);
        let wait = if at_work == 0 {
            "Restart Cox and coxd".to_owned()
        } else {
            let end = if at_work == 1 { "ends" } else { "end" };
            format!("Restart Cox and coxd once {} at work {end}", chats(at_work))
        };
        let now = if at_work == 0 {
            "Restart Cox and coxd now".to_owned()
        } else {
            format!("Restart Cox and coxd now, cutting {}…", chats(at_work))
        };
        let menu = crate::ui::menu::menu()
            .min_w(px(260.))
            .max_w(px(420.))
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| {
                workspace.restart_menu = None;
                cx.notify();
            }))
            .children(pending.map(|pending| {
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(FAINT))
                    .whitespace_normal()
                    .child(pending.told())
            }))
            .when(pending.is_none_or(Pending::window_runs_it), |menu| {
                menu.child(
                    item(
                        "restart-window",
                        "Restart window: the chats go on".to_owned(),
                        ":restart",
                    )
                    .on_hint_click(cx.listener(|workspace, _, window, cx| {
                        workspace.restart(How::Window, window, cx);
                    })),
                )
            })
            .child(
                item("restart-coxd", wait, ":restartcoxd").on_hint_click(cx.listener(
                    |workspace, _, window, cx| {
                        workspace.restart(How::WithCoxd, window, cx);
                    },
                )),
            )
            .when(at_work > 0, |menu| {
                menu.child(
                    item("restart-coxd-now", now, ":restartcoxdnow").on_hint_click(cx.listener(
                        |workspace, _, window, cx| {
                            workspace.restart(How::WithCoxdNow, window, cx);
                        },
                    )),
                )
            });
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
}
