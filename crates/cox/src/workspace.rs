//! The window: the sessions open in it, listed down its left side, and the
//! one shown beside the list. Each session works in a directory of its
//! own: a folder the human chose, or Cox's home for sessions with no
//! folder.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    App, Context, Div, Entity, FocusHandle, Focusable, KeyBinding, PathPromptOptions, SharedString,
    Stateful, Subscription, Window, actions, div, prelude::*, px, rgb,
};

use crate::conversation::State;
use crate::view::SessionView;

actions!(
    cox,
    [
        /// Opens a session in Cox's home.
        NewSession,
        /// Asks for a folder, and opens a session in it.
        OpenFolder,
        NextSession,
        PreviousSession,
    ]
);

/// How wide the list of sessions is.
const LIST_WIDTH: f32 = 200.;

/// Binds the keys that open sessions and move between them.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewSession, Some("Workspace")),
        KeyBinding::new("secondary-o", OpenFolder, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextSession, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousSession, Some("Workspace")),
    ]);
}

/// Starts a session working in the directory given, and returns its view.
pub type Open = Rc<dyn Fn(PathBuf, &mut Window, &mut App) -> Entity<SessionView>>;

/// A session open in the window.
pub struct Session {
    name: SharedString,
    cwd: PathBuf,
    view: Entity<SessionView>,
    /// Redraws the list as the session's state changes.
    _changes: Subscription,
}

impl Session {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The directory the session works in.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    #[must_use]
    pub fn view(&self) -> &Entity<SessionView> {
        &self.view
    }
}

pub struct Workspace {
    sessions: Vec<Session>,
    /// The index of the session shown, if any is open.
    shown: Option<usize>,
    /// Where sessions with no folder work.
    home: PathBuf,
    /// How many sessions with no folder have been opened, to name the next.
    unnamed: usize,
    open: Open,
    focus_handle: FocusHandle,
}

impl Workspace {
    /// A window with no session open yet. `open` starts sessions, and
    /// sessions with no folder work in `home`.
    pub fn new(home: PathBuf, open: Open, cx: &mut Context<Self>) -> Self {
        Self {
            sessions: Vec::new(),
            shown: None,
            home,
            unnamed: 0,
            open,
            focus_handle: cx.focus_handle(),
        }
    }

    #[must_use]
    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    /// The session shown, if any is open.
    #[must_use]
    pub fn shown(&self) -> Option<&Session> {
        self.sessions.get(self.shown?)
    }

    /// Opens a session working in `folder`, or in Cox's home when there is
    /// none, and shows it.
    pub fn open(&mut self, folder: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let (name, cwd) = if let Some(folder) = folder {
            let name = folder.file_name().map_or_else(
                || folder.to_string_lossy().into_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            (name, folder)
        } else {
            // The agent's session fails to open, and says so, if the home
            // cannot be made.
            if let Err(error) = std::fs::create_dir_all(&self.home) {
                log::error!("cannot make {}: {error}", self.home.display());
            }
            self.unnamed += 1;
            (format!("Session {}", self.unnamed), self.home.clone())
        };
        let view = (self.open)(cwd.clone(), window, cx);
        let changes = cx.observe(&view, |_, _, cx| cx.notify());
        self.sessions.push(Session {
            name: name.into(),
            cwd,
            view,
            _changes: changes,
        });
        self.show(self.sessions.len() - 1, window, cx);
    }

    /// Asks the human for a folder, and opens a session in it.
    pub fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let folder = match chosen.await {
                Ok(Ok(Some(mut paths))) if !paths.is_empty() => paths.swap_remove(0),
                Ok(Err(error)) => {
                    log::error!("no folder chosen: {error:#}");
                    return;
                }
                _ => return,
            };
            this.update_in(cx, |workspace, window, cx| {
                workspace.open(Some(folder), window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Shows the session at `index`, with the keyboard in its input.
    pub fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.sessions.len() {
            return;
        }
        self.shown = Some(index);
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Closes the session at `index`, which stops its agent and closes its
    /// page, and shows the one after it, or before it if it was the last.
    pub fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.sessions.len() {
            return;
        }
        self.sessions.remove(index);
        let shown = match self.shown {
            _ if self.sessions.is_empty() => None,
            Some(shown) if shown > index => Some(shown - 1),
            Some(shown) => Some(shown.min(self.sessions.len() - 1)),
            None => None,
        };
        self.shown = shown;
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Shows the session `step` places along the list, going round.
    fn step(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(shown), Ok(count)) = (self.shown, isize::try_from(self.sessions.len())) else {
            return;
        };
        let Ok(shown) = isize::try_from(shown) else {
            return;
        };
        if let Ok(index) = usize::try_from((shown + step).rem_euclid(count)) {
            self.show(index, window, cx);
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| self.render_row(index, session, cx))
            .collect();
        div()
            .w(px(LIST_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(0x40_4040))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(0x40_4040))
                    .child(Self::button(
                        "new-session",
                        "+ New session",
                        cx,
                        |workspace, window, cx| {
                            workspace.open(None, window, cx);
                        },
                    ))
                    .child(Self::button(
                        "open-folder",
                        "+ Open folder…",
                        cx,
                        |workspace, window, cx| {
                            workspace.choose_folder(window, cx);
                        },
                    )),
            )
            .child(
                div()
                    .id("sessions")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows),
            )
    }

    fn button(
        name: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(name)
            .debug_selector(|| name.to_owned())
            .px_2()
            .rounded_sm()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x30_3030)))
            .child(label)
            .on_click(cx.listener(move |workspace, _, window, cx| action(workspace, window, cx)))
    }

    fn render_row(&self, index: usize, session: &Session, cx: &mut Context<Self>) -> Stateful<Div> {
        let (mark, color) = match session.view.read(cx).conversation().state() {
            State::Idle => ("", rgb(0xa0_a0a0)),
            State::Busy => ("…", rgb(0xa0_a0a0)),
            State::Asking => ("?", rgb(0xd0_a040)),
            State::Closed => ("closed", rgb(0x80_8080)),
        };
        let mut row = div()
            .id(("session", index))
            .debug_selector(|| format!("session-{}", index + 1))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .cursor_pointer()
            .on_click(
                cx.listener(move |workspace, _, window, cx| workspace.show(index, window, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().truncate().child(session.name.clone()))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(0x80_8080))
                            .child(shortened(&session.cwd)),
                    ),
            )
            .child(div().text_color(color).child(mark))
            .child(
                div()
                    .id(("close", index))
                    .debug_selector(|| format!("close-{}", index + 1))
                    .px_1()
                    .rounded_sm()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x40_4040)).text_color(rgb(0xe0_e0e0)))
                    .child("×")
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        cx.stop_propagation();
                        workspace.close(index, window, cx);
                    })),
            );
        if self.shown == Some(index) {
            row = row.bg(rgb(0x2a_2a2a));
        } else {
            row = row.hover(|style| style.bg(rgb(0x26_2626)));
        }
        row
    }
}

/// `path`, with the home directory as `~`.
fn shortened(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl Focusable for Workspace {
    /// The shown session's input, or the window itself while none is open.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.shown() {
            Some(session) => session.view.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = match self.shown() {
            Some(session) => session.view.clone().into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x80_8080))
                .child("Open a session from the list on the left.")
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .key_context("Workspace")
            .on_action(cx.listener(|workspace, _: &NewSession, window, cx| {
                workspace.open(None, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &OpenFolder, window, cx| {
                workspace.choose_folder(window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &NextSession, window, cx| {
                workspace.step(1, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &PreviousSession, window, cx| {
                workspace.step(-1, window, cx);
            }))
            .size_full()
            .flex()
            .bg(rgb(0x1e_1e1e))
            .text_color(rgb(0xe0_e0e0))
            .font_family("monospace")
            .text_size(px(14.))
            .child(self.render_list(cx))
            .child(div().flex_1().min_w_0().h_full().child(shown))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_in_home_are_shortened() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        assert_eq!(shortened(&home), "~");
        assert_eq!(shortened(&home.join("Cox")), "~/Cox");
        assert_eq!(shortened(Path::new("/elsewhere")), "/elsewhere");
    }
}
