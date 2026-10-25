//! The window: the chats open in it, listed down its left side under the
//! projects they are in, and the one shown beside the list. A project is a
//! folder the human opened: each chat in it works there, and its agent gets
//! the instructions in the folder's `COX.md`. Chats in no project work in
//! Cox's home, with the instructions there.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    App, Context, Div, Entity, FocusHandle, Focusable, KeyBinding, PathPromptOptions, SharedString,
    Stateful, Subscription, Window, actions, div, prelude::*, px, rgb,
};

use crate::conversation::State;
use crate::instructions::Instructions;
use crate::view::SessionView;

actions!(
    cox,
    [
        /// Opens a chat in no project.
        NewChat,
        /// Asks for a folder, and opens a chat in its project.
        OpenFolder,
        NextChat,
        PreviousChat,
    ]
);

/// How wide the list of chats is.
const LIST_WIDTH: f32 = 220.;

/// What the project of chats with no folder is called.
const NO_PROJECT: &str = "No project";

/// Binds the keys that open chats and move between them.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewChat, Some("Workspace")),
        KeyBinding::new("secondary-o", OpenFolder, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextChat, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousChat, Some("Workspace")),
    ]);
}

/// Starts a chat's agent working in the directory given, with the
/// instructions given, and returns the chat's view.
pub type Open =
    Rc<dyn Fn(PathBuf, Option<Instructions>, &mut Window, &mut App) -> Entity<SessionView>>;

/// A folder, and the chats open in it.
pub struct Project {
    name: SharedString,
    folder: PathBuf,
    chats: Vec<Chat>,
    /// How many chats have been opened in it, to name the next.
    opened: usize,
}

impl Project {
    fn new(name: impl Into<SharedString>, folder: PathBuf) -> Self {
        Self {
            name: name.into(),
            folder,
            chats: Vec::new(),
            opened: 0,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The directory its chats work in.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    #[must_use]
    pub fn chats(&self) -> &[Chat] {
        &self.chats
    }
}

/// A chat open in the window.
pub struct Chat {
    name: SharedString,
    view: Entity<SessionView>,
    /// Redraws the list as the chat's state changes.
    _changes: Subscription,
}

impl Chat {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn view(&self) -> &Entity<SessionView> {
        &self.view
    }
}

/// Where a chat is: the index of its project, and its own in the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub project: usize,
    pub chat: usize,
}

pub struct Workspace {
    /// The projects, in the order they were opened, after no project,
    /// which comes first.
    projects: Vec<Project>,
    /// Where the chat shown is, if any is open.
    shown: Option<Place>,
    open: Open,
    focus_handle: FocusHandle,
}

impl Workspace {
    /// A window with no chat open yet. `open` starts chats, and chats in no
    /// project work in `home`.
    pub fn new(home: PathBuf, open: Open, cx: &mut Context<Self>) -> Self {
        Self {
            projects: vec![Project::new(NO_PROJECT, home)],
            shown: None,
            open,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The projects, no project first.
    #[must_use]
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    /// The chat shown, if any is open.
    #[must_use]
    pub fn shown(&self) -> Option<&Chat> {
        let place = self.shown?;
        self.projects.get(place.project)?.chats.get(place.chat)
    }

    /// The index of the project of `folder`, opened first if it is not
    /// open. Cox's home is no project's.
    fn project(&mut self, folder: PathBuf) -> usize {
        if let Some(index) = self
            .projects
            .iter()
            .position(|project| project.folder == folder)
        {
            return index;
        }
        let name = folder.file_name().map_or_else(
            || folder.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.projects.push(Project::new(name, folder));
        self.projects.len() - 1
    }

    /// Opens a chat in the project of `folder`, or in no project when there
    /// is none, and shows it.
    pub fn open(&mut self, folder: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let index = folder.map_or(0, |folder| self.project(folder));
        let project = &mut self.projects[index];
        if index == 0 {
            // The agent's session fails to open, and says so, if the home
            // cannot be made.
            if let Err(error) = std::fs::create_dir_all(&project.folder) {
                log::error!("cannot make {}: {error}", project.folder.display());
            }
        }
        let instructions = Instructions::read(&project.folder);
        let view = (self.open)(project.folder.clone(), instructions, window, cx);
        let changes = cx.observe(&view, |_, _, cx| cx.notify());
        let project = &mut self.projects[index];
        project.opened += 1;
        project.chats.push(Chat {
            name: format!("Chat {}", project.opened).into(),
            view,
            _changes: changes,
        });
        let chat = project.chats.len() - 1;
        self.show(
            Place {
                project: index,
                chat,
            },
            window,
            cx,
        );
    }

    /// Asks the human for a folder, and opens a chat in its project.
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

    /// Every open chat's place, in the order of the list.
    fn places(&self) -> Vec<Place> {
        self.projects
            .iter()
            .enumerate()
            .flat_map(|(project, open)| {
                (0..open.chats.len()).map(move |chat| Place { project, chat })
            })
            .collect()
    }

    /// Shows the chat at `place`, with the keyboard in its input.
    pub fn show(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        if !self.places().contains(&place) {
            return;
        }
        self.shown = Some(place);
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Closes the chat at `place`, which stops its agent and closes its
    /// page, and shows the one after it in the list, or before it if it
    /// was the last. Its project stays, to open chats in.
    pub fn close(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        let places = self.places();
        let Some(closed) = places.iter().position(|&open| open == place) else {
            return;
        };
        let shown = self
            .shown
            .and_then(|shown| places.iter().position(|&open| open == shown));
        self.projects[place.project].chats.remove(place.chat);
        let places = self.places();
        let shown = match shown {
            _ if places.is_empty() => None,
            Some(shown) if shown > closed => Some(shown - 1),
            Some(shown) => Some(shown.min(places.len() - 1)),
            None => None,
        };
        self.shown = shown.map(|shown| places[shown]);
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Shows the chat `step` places along the list, going round.
    fn step(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let places = self.places();
        let Some(shown) = self
            .shown
            .and_then(|shown| places.iter().position(|&open| open == shown))
        else {
            return;
        };
        let (Ok(shown), Ok(count)) = (isize::try_from(shown), isize::try_from(places.len())) else {
            return;
        };
        if let Ok(index) = usize::try_from((shown + step).rem_euclid(count)) {
            self.show(places[index], window, cx);
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Chats are numbered down the whole list, from 1.
        let mut number = 0;
        let mut rows = Vec::new();
        for (index, project) in self.projects.iter().enumerate() {
            rows.push(Self::render_project(index, project, cx));
            for (chat, open) in project.chats.iter().enumerate() {
                number += 1;
                let place = Place {
                    project: index,
                    chat,
                };
                rows.push(self.render_chat(place, number, open, cx));
            }
        }
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
                        "new-chat",
                        "+ New chat",
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
                    .id("chats")
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

    /// The heading of the project at `index`, with a button that opens a
    /// chat in it.
    fn render_project(index: usize, project: &Project, cx: &mut Context<Self>) -> Stateful<Div> {
        let folder = project.folder.clone();
        div()
            .id(("project", index))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .pt_3()
            .pb_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_color(rgb(0xa0_a0a0))
                            .child(project.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(0x80_8080))
                            .child(shortened(&project.folder)),
                    ),
            )
            .child(
                div()
                    .id(("new-chat-in", index))
                    .debug_selector(move || format!("new-chat-in-{}", index + 1))
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x40_4040)).text_color(rgb(0xe0_e0e0)))
                    .child("+")
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.open(Some(folder.clone()), window, cx);
                    })),
            )
    }

    /// The row of the chat at `place`, the `number`th in the list.
    fn render_chat(
        &self,
        place: Place,
        number: usize,
        chat: &Chat,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (mark, color) = match chat.view.read(cx).conversation().state() {
            State::Idle => ("", rgb(0xa0_a0a0)),
            State::Busy | State::Stopping => ("…", rgb(0xa0_a0a0)),
            State::Asking => ("?", rgb(0xd0_a040)),
            State::Closed => ("closed", rgb(0x80_8080)),
        };
        let mut row = div()
            .id(("chat", number))
            .debug_selector(move || format!("chat-{number}"))
            .flex()
            .items_center()
            .gap_1()
            .pl_4()
            .pr_2()
            .py_1()
            .cursor_pointer()
            .on_click(
                cx.listener(move |workspace, _, window, cx| workspace.show(place, window, cx)),
            )
            .child(div().flex_1().min_w_0().truncate().child(chat.name.clone()))
            .child(div().text_color(color).child(mark))
            .child(
                div()
                    .id(("close", number))
                    .debug_selector(move || format!("close-{number}"))
                    .px_1()
                    .rounded_sm()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x40_4040)).text_color(rgb(0xe0_e0e0)))
                    .child("×")
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        cx.stop_propagation();
                        workspace.close(place, window, cx);
                    })),
            );
        if self.shown == Some(place) {
            row = row.bg(rgb(0x2a_2a2a));
        } else {
            row = row.hover(|style| style.bg(rgb(0x26_2626)));
        }
        row
    }
}

/// `path`, with the home directory as `~`.
#[must_use]
pub fn shortened(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl Focusable for Workspace {
    /// The shown chat's input, or the window itself while none is open.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.shown() {
            Some(chat) => chat.view.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = match self.shown() {
            Some(chat) => chat.view.clone().into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0x80_8080))
                .child("Open a chat from the list on the left.")
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .key_context("Workspace")
            .on_action(cx.listener(|workspace, _: &NewChat, window, cx| {
                workspace.open(None, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &OpenFolder, window, cx| {
                workspace.choose_folder(window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &NextChat, window, cx| {
                workspace.step(1, window, cx);
            }))
            .on_action(cx.listener(|workspace, _: &PreviousChat, window, cx| {
                workspace.step(-1, window, cx);
            }))
            .size_full()
            .flex()
            .bg(rgb(0x1e_1e1e))
            .text_color(rgb(0xe0_e0e0))
            .font_family(crate::view::FONT)
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
