//! The window: the chats, listed down its left side under the projects
//! they are in, and the one shown beside the list. A project is a folder
//! the human opened: each chat in it works there, and its agent gets the
//! instructions in the folder's `COX.md`. Chats in no project work in
//! Cox's home, with the instructions there.
//!
//! The projects and chats are kept as they go (see [`cox_core::store`]).
//! A chat's agent runs until Cox quits; the chats of earlier runs stay in
//! the list, closed, for the human to read what they showed, and to send
//! to again, which starts the chat's agent again.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use cox_core::list::List;
use cox_core::store::{ChatId, ProjectId, Record, Store};
use gpui::{
    App, Context, Div, Entity, FocusHandle, Focusable, KeyBinding, PathPromptOptions, SharedString,
    Stateful, Subscription, Window, actions, div, prelude::*, px, rgb,
};

use crate::agent::instructions::Instructions;
use crate::chat::conversation::{Entry, State};
use crate::chat::view::{ChatView, Reopened};
use crate::chat::{Closed, Opens};
use crate::ui::paths::shortened;
use crate::ui::theme::FONT;

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

/// A folder the chats of a project work in. The chats themselves are in
/// the window's list, under it.
pub struct Project {
    /// How it is kept, or `None` for no project, or a project that could
    /// not be kept.
    id: Option<ProjectId>,
    name: SharedString,
    folder: PathBuf,
    /// How many chats have been opened in it, to name the next.
    opened: usize,
}

impl Project {
    fn new(id: Option<ProjectId>, name: impl Into<SharedString>, folder: PathBuf) -> Self {
        Self {
            id,
            name: name.into(),
            folder,
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
}

/// A chat in the list: open, with its agent, or closed, from an earlier
/// run, for the human to read, until they send to it.
pub struct Chat {
    /// The index of its project.
    project: usize,
    /// How it is kept, or `None` when it could not be.
    kept: Option<ChatId>,
    name: SharedString,
    /// Whether its agent runs.
    open: bool,
    /// Its view, or `None` for a closed chat the human has not read yet.
    view: Option<Entity<ChatView>>,
    /// Redraws the list as the chat's state changes, and hears a closed
    /// chat's agent start again.
    _changes: Vec<Subscription>,
}

impl Chat {
    /// A closed chat of the project at `project`, kept as `kept`, which
    /// the human has not read yet.
    fn closed(project: usize, kept: ChatId, name: impl Into<SharedString>) -> Self {
        Self {
            project,
            kept: Some(kept),
            name: name.into(),
            open: false,
            view: None,
            _changes: Vec::new(),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Its view, once it has been shown.
    #[must_use]
    pub fn view(&self) -> Option<&Entity<ChatView>> {
        self.view.as_ref()
    }
}

pub struct Workspace {
    /// The projects, in the order they were first opened, after no
    /// project, which comes first.
    projects: Vec<Project>,
    /// The chats, in the order of the list: under their projects, each
    /// after the ones opened before it; and which is shown, if any is.
    chats: List<Chat>,
    opens: Rc<dyn Opens>,
    store: Rc<Store>,
    focus_handle: FocusHandle,
}

impl Workspace {
    /// A window with no chat open, listing the chats kept in `store`.
    /// `opens` opens chats, and chats in no project work in `home`.
    pub fn new(
        home: PathBuf,
        store: Rc<Store>,
        opens: Rc<dyn Opens>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut workspace = Self {
            projects: vec![Project::new(None, NO_PROJECT, home)],
            chats: List::new(),
            opens,
            store,
            focus_handle: cx.focus_handle(),
        };
        let kept = workspace
            .store
            .projects()
            .and_then(|kept| Ok((kept, workspace.store.chats()?)));
        match kept {
            Ok((kept, chats)) => {
                workspace.projects.extend(
                    kept.into_iter().map(|project| {
                        Project::new(Some(project.id), project.name, project.folder)
                    }),
                );
                for chat in chats {
                    let Some(index) = workspace
                        .projects
                        .iter()
                        .position(|project| project.id == chat.project)
                    else {
                        continue;
                    };
                    workspace.projects[index].opened += 1;
                    workspace.add(Chat::closed(index, chat.id, chat.name));
                }
            }
            Err(error) => log::error!("the chats kept cannot be listed: {error:#}"),
        }
        workspace
    }

    /// Puts `chat` in the list after the other chats of its project, and
    /// returns its index.
    fn add(&mut self, chat: Chat) -> usize {
        let after = self
            .chats
            .iter()
            .rposition(|other| other.project <= chat.project)
            .map_or(0, |index| index + 1);
        self.chats.insert(after, chat, false)
    }

    /// The projects, no project first.
    #[must_use]
    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    /// The chats of the project at `project`, in the order of the list.
    pub fn chats(&self, project: usize) -> impl Iterator<Item = &Chat> {
        self.chats
            .iter()
            .filter(move |chat| chat.project == project)
    }

    /// The chat shown, if any is.
    #[must_use]
    pub fn shown(&self) -> Option<&Chat> {
        self.chats.shown()
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
        let id = self
            .store
            .project(&folder, &name)
            .inspect_err(|error| log::error!("the project is not kept: {error:#}"))
            .ok();
        self.projects.push(Project::new(id, name, folder));
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
        project.opened += 1;
        let name = format!("Chat {}", project.opened);
        let kept = self
            .store
            .add_chat(project.id, &name)
            .inspect_err(|error| log::error!("the chat is not kept: {error:#}"))
            .ok();
        let record = kept.map(|kept| Record::new(Rc::clone(&self.store), kept));
        let view = self
            .opens
            .open(project.folder.clone(), instructions, record, window, cx);
        let changes = cx.observe(&view, |_, _, cx| cx.notify());
        let chat = self.add(Chat {
            project: index,
            kept,
            name: name.into(),
            open: true,
            view: Some(view),
            _changes: vec![changes],
        });
        self.show(chat, window, cx);
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

    /// Shows the chat at `index` in the list, with the keyboard in its
    /// input.
    pub fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.chats.show(index) {
            return;
        }
        self.read(index, window, cx);
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Gives the closed chat at `index` its view, from what is kept of it,
    /// unless it has one. Once the human sends to it, it is open.
    fn read(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let chat = &self.chats[index];
        let project = &self.projects[chat.project];
        let (Some(kept), None) = (chat.kept, &chat.view) else {
            return;
        };
        let entries = self.store.entries(kept).unwrap_or_else(|error| {
            log::error!("the chat cannot be read: {error:#}");
            vec![Entry::Note(format!("[cannot read this chat: {error:#}]"))]
        });
        let tabs = self
            .store
            .tabs(kept)
            .inspect_err(|error| log::error!("the chat's tabs cannot be read: {error:#}"))
            .ok()
            .flatten();
        let session = self
            .store
            .session(kept)
            .inspect_err(|error| log::error!("the chat's session cannot be read: {error:#}"))
            .ok()
            .flatten();
        let closed = Closed {
            cwd: project.folder.clone(),
            entries,
            tabs,
            session,
            record: Some(Record::new(Rc::clone(&self.store), kept)),
        };
        let view = self.opens.read(closed, window, cx);
        let changes = cx.observe(&view, |_, _, cx| cx.notify());
        let reopened = cx.subscribe(&view, move |workspace, _, Reopened, cx| {
            // The chat may have moved along the list since.
            if let Some(chat) = workspace
                .chats
                .iter_mut()
                .find(|chat| chat.kept == Some(kept))
            {
                chat.open = true;
            }
            cx.notify();
        });
        let chat = &mut self.chats[index];
        *chat = Chat {
            project: chat.project,
            kept: Some(kept),
            name: chat.name.clone(),
            open: false,
            view: Some(view),
            _changes: vec![changes, reopened],
        };
    }

    /// Shows the chat `step` places along the list, going round.
    fn step(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.chats.along(step) {
            self.show(index, window, cx);
        }
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        for (index, project) in self.projects.iter().enumerate() {
            rows.push(Self::render_project(index, project, cx));
            let chats = self.chats.iter().enumerate();
            for (chat, open) in chats.filter(|(_, chat)| chat.project == index) {
                rows.push(self.render_chat(chat, open, cx));
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

    /// The row of the chat at `index` in the list, which numbers its
    /// chats from 1.
    fn render_chat(&self, index: usize, chat: &Chat, cx: &mut Context<Self>) -> Stateful<Div> {
        let number = index + 1;
        let state = chat
            .view
            .as_ref()
            .filter(|_| chat.open)
            .map(|view| view.read(cx).conversation().state());
        let (mark, color) = match state {
            Some(State::Idle) | None => ("", rgb(0xa0_a0a0)),
            Some(State::Busy | State::Stopping) => ("…", rgb(0xa0_a0a0)),
            Some(State::Asking) => ("?", rgb(0xd0_a040)),
            Some(State::Closed) => ("closed", rgb(0x80_8080)),
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
                cx.listener(move |workspace, _, window, cx| workspace.show(index, window, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    // A closed chat is there to read.
                    .when(!chat.open, |name| name.text_color(rgb(0x80_8080)))
                    .child(chat.name.clone()),
            )
            .child(div().text_color(color).child(mark));
        if self.chats.shown_index() == Some(index) {
            row = row.bg(rgb(0x2a_2a2a));
        } else {
            row = row.hover(|style| style.bg(rgb(0x26_2626)));
        }
        row
    }
}

impl Focusable for Workspace {
    /// The shown chat's input, or the window itself while none is open.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.shown().and_then(Chat::view) {
            Some(view) => view.focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = match self.shown().and_then(Chat::view) {
            Some(view) => view.clone().into_any_element(),
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
            .font_family(FONT)
            .text_size(px(14.))
            .child(self.render_list(cx))
            .child(div().flex_1().min_w_0().h_full().child(shown))
    }
}
