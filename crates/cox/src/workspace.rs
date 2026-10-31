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
//!
//! A chat is named by a number at first, then after the first line the
//! human sends in it, then by the title its agent gives it, until the
//! human names it. Its row in the list tells whether its agent is at
//! work, waits for the human, or finished a turn the human has not seen
//! yet. The menu of the right button on a row renames the chat, as a
//! double click does, or archives it: an archived chat leaves the list
//! for "Archived" at its foot, whose click brings it back.
//!
//! A chat that comes to wait for the human, or whose turn ends, while the
//! human is not looking at it, tells them so (see [`crate::notices`]),
//! and the app's icon counts the chats that wait for them or that they
//! have not seen since a turn ended. A click on what a chat told shows it.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use cox_core::list::List;
use cox_core::store::{ChatId, Named, ProjectId, Record, Store};
use gpui::{
    App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, KeyBinding, MouseButton,
    MouseDownEvent, PathPromptOptions, Pixels, Point, SharedString, Stateful, Subscription, Window,
    actions, anchored, deferred, div, prelude::*, px, rgb,
};

use crate::agent::instructions::Instructions;
use crate::chat::conversation::{Entry, State};
use crate::chat::view::{ChatView, Reopened};
use crate::chat::{Closed, Opens};
use crate::notices::AppNotices;
use crate::ui::input::{LineInput, Submitted};
use crate::ui::paths::shortened;
use crate::ui::theme::{ASKING, DIM, FONT, LINK};

actions!(
    cox,
    [
        /// Opens a chat in no project.
        NewChat,
        /// Asks for a folder, and opens a chat in its project.
        OpenFolder,
        NextChat,
        PreviousChat,
        /// Leaves the chat's name as it was.
        CancelRename,
    ]
);

/// How wide the list of chats is.
const LIST_WIDTH: f32 = 220.;

/// What the project of chats with no folder is called.
const NO_PROJECT: &str = "No project";

/// How long a chat's name grows, in characters, when the agent or the
/// first line names it.
const NAME_LENGTH: usize = 80;

/// The key context of the input that renames a chat.
const RENAMING: &str = "Renaming";

/// Binds the keys that open chats and move between them.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewChat, Some("Workspace")),
        KeyBinding::new("secondary-o", OpenFolder, Some("Workspace")),
        KeyBinding::new("ctrl-tab", NextChat, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PreviousChat, Some("Workspace")),
        KeyBinding::new("escape", CancelRename, Some(RENAMING)),
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
    /// What names it for this run, as it moves along the list, or out of
    /// it.
    key: u64,
    /// The index of its project.
    project: usize,
    /// How it is kept, or `None` when it could not be.
    kept: Option<ChatId>,
    name: SharedString,
    named: Named,
    /// Whether its agent runs.
    open: bool,
    /// Whose move it was when the chat last changed.
    state: State,
    /// Whether a turn ended while the chat was not shown, and it has not
    /// been shown since.
    unread: bool,
    /// The command it works until it passes, if the human set one.
    goal: Option<String>,
    /// Its view, or `None` for a closed chat the human has not read yet.
    view: Option<Entity<ChatView>>,
    /// Redraws the list as the chat's state changes, and hears a closed
    /// chat's agent start again.
    _changes: Vec<Subscription>,
}

impl Chat {
    /// A closed chat of the project at `project`, kept as `kept`, which
    /// the human has not read yet, named `key` for this run, and working
    /// until `goal` passes if the human set one.
    fn closed(
        key: u64,
        project: usize,
        kept: ChatId,
        name: impl Into<SharedString>,
        named: Named,
        goal: Option<String>,
    ) -> Self {
        Self {
            key,
            project,
            kept: Some(kept),
            name: name.into(),
            named,
            open: false,
            state: State::Closed,
            unread: false,
            goal,
            view: None,
            _changes: Vec::new(),
        }
    }

    /// Names it `name`, which `named` says where it came from, and keeps
    /// the name in `store`.
    fn name_as(&mut self, name: &str, named: Named, store: &Store) {
        self.name = SharedString::from(name.to_owned());
        self.named = named;
        if let Some(kept) = self.kept
            && let Err(error) = store.name_chat(kept, name, named)
        {
            log::error!("the chat's name is not kept: {error:#}");
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

    /// What names it for this run.
    #[must_use]
    pub fn key(&self) -> u64 {
        self.key
    }

    /// Whether a turn ended while the chat was not shown, and it has not
    /// been shown since.
    #[must_use]
    pub fn is_unread(&self) -> bool {
        self.unread
    }

    /// The command it works until it passes, if the human set one.
    #[must_use]
    pub fn goal(&self) -> Option<&str> {
        self.goal.as_deref()
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
    /// The chats the human archived, out of the list, in the order they
    /// were archived.
    archived: Vec<Chat>,
    /// Whether the archived chats show at the foot of the list.
    archived_open: bool,
    /// The key of the last chat opened.
    last_key: u64,
    /// How many chats the app's icon counts.
    badged: usize,
    /// The menu of the right button, where it opened in the window, for
    /// the chat at an index in the list.
    menu: Option<(Point<Pixels>, usize)>,
    /// The chat at an index in the list being renamed, in an input.
    renaming: Option<(usize, Entity<LineInput>, Subscription)>,
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
            archived: Vec::new(),
            archived_open: false,
            last_key: 0,
            badged: 0,
            menu: None,
            renaming: None,
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
                    let key = workspace.next_key();
                    let closed = Chat::closed(
                        key,
                        index,
                        chat.id,
                        chat.name,
                        chat.named,
                        chat.goal_command,
                    );
                    if chat.archived {
                        workspace.archived.push(closed);
                    } else {
                        workspace.add(closed);
                    }
                }
            }
            Err(error) => log::error!("the chats kept cannot be listed: {error:#}"),
        }
        workspace
    }

    /// A key for a chat, which no other chat has had in this run.
    fn next_key(&mut self) -> u64 {
        self.last_key += 1;
        self.last_key
    }

    /// Puts `chat` in the list after the other chats of its project, and
    /// returns its index.
    fn add(&mut self, chat: Chat) -> usize {
        // What was at an index may move along.
        self.menu = None;
        self.renaming = None;
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

    /// The chats the human archived, in the order they were archived.
    #[must_use]
    pub fn archived(&self) -> &[Chat] {
        &self.archived
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
        let changes = cx.observe_in(&view, window, |workspace, view, window, cx| {
            workspace.changed(&view, window, cx);
        });
        let key = self.next_key();
        let chat = self.add(Chat {
            key,
            project: index,
            kept,
            name: name.into(),
            named: Named::Numbered,
            open: true,
            state: State::Idle,
            unread: false,
            goal: None,
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
        self.chats[index].unread = false;
        self.badge(cx);
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
            vec![Entry::Note(format!("Cox cannot read this chat: {error:#}"))]
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
            goal: chat.goal.clone(),
            record: Some(Record::new(Rc::clone(&self.store), kept)),
        };
        let view = self.opens.read(closed, window, cx);
        let changes = cx.observe_in(&view, window, |workspace, view, window, cx| {
            workspace.changed(&view, window, cx);
        });
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
            key: chat.key,
            project: chat.project,
            kept: Some(kept),
            name: chat.name.clone(),
            named: chat.named,
            open: false,
            state: State::Closed,
            unread: chat.unread,
            goal: chat.goal.take(),
            view: Some(view),
            _changes: vec![changes, reopened],
        };
    }

    /// Follows a change of the chat of `view`: a turn that ends while it is
    /// not shown leaves it unread, and what it is about names it, unless a
    /// name that lasts more has. A chat the human is not looking at tells
    /// them when it comes to wait for them, and when its turn ends.
    fn changed(&mut self, view: &Entity<ChatView>, window: &Window, cx: &mut Context<Self>) {
        let shown = self.chats.shown().and_then(Chat::view) == Some(view);
        let seen = shown && window.is_window_active();
        let Some(chat) = self
            .chats
            .iter_mut()
            .chain(&mut self.archived)
            .find(|chat| chat.view.as_ref() == Some(view))
        else {
            return;
        };
        let conversation = view.read(cx).conversation();
        let state = conversation.state();
        let ended = working(chat.state) && !working(state);
        if ended && !shown {
            chat.unread = true;
        }
        let news = if state == State::Asking && chat.state != state {
            let asked = conversation.asking_for().unwrap_or("permission");
            Some(format!("Asks you: {asked}"))
        } else if ended {
            let said = conversation.last_said().unwrap_or("The turn ended.");
            Some(said.to_owned())
        } else {
            None
        };
        chat.state = state;
        chat.goal = conversation.goal().map(|goal| goal.command().to_owned());
        let about = match (conversation.title(), conversation.first_line()) {
            (Some(title), _) => Some((title, Named::Agent)),
            (None, Some(line)) => Some((line, Named::FirstLine)),
            (None, None) => None,
        };
        if let Some((name, named)) = about {
            let name: String = name.chars().take(NAME_LENGTH).collect();
            // The agent may title its session anew.
            let lasts = named > chat.named || (named == Named::Agent && chat.named == named);
            if lasts && name != chat.name.as_ref() {
                chat.name_as(&name, named, &self.store);
            }
        }
        if let Some(body) = news.filter(|_| !seen)
            && let Some(notices) = cx.try_global::<AppNotices>()
        {
            notices.0.notify(chat.key, &chat.name, &body);
        }
        self.badge(cx);
        cx.notify();
    }

    /// Counts on the app's icon the chats that wait for the human, or that
    /// they have not seen since a turn ended.
    fn badge(&mut self, cx: &App) {
        let count = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter(|chat| chat.unread || chat.state == State::Asking)
            .count();
        if count != self.badged {
            self.badged = count;
            if let Some(notices) = cx.try_global::<AppNotices>() {
                notices.0.badge(count);
            }
        }
    }

    /// Shows the chat `key` names, bringing it back to the list if it was
    /// archived, as a click on what it told the human does.
    pub fn reveal(&mut self, key: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.chats.iter().position(|chat| chat.key == key) {
            self.show(index, window, cx);
        } else if let Some(index) = self.archived.iter().position(|chat| chat.key == key) {
            self.unarchive(index, window, cx);
        }
    }

    /// Opens an input in place of the name of the chat at `index`, for the
    /// human to rename it.
    pub fn rename(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.get(index) else {
            return;
        };
        let name = chat.name.clone();
        let input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_text(&name, cx);
            input.select_whole(cx);
            input
        });
        let submitted = cx.subscribe_in(&input, window, |workspace, _, Submitted, window, cx| {
            workspace.end_renaming(true, window, cx);
        });
        window.focus(&input.focus_handle(cx));
        self.menu = None;
        self.renaming = Some((index, input, submitted));
        cx.notify();
    }

    /// Closes the input that renames a chat, naming the chat what it holds
    /// when `keep` says to and it holds a name.
    fn end_renaming(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((index, input, _)) = self.renaming.take() else {
            return;
        };
        let name = input.read(cx).text().trim().to_owned();
        if keep && !name.is_empty() {
            self.chats[index].name_as(&name, Named::Human, &self.store);
        }
        window.focus(&self.focus_handle(cx));
        cx.notify();
    }

    /// Takes the chat at `index` out of the list, to "Archived" at its
    /// foot. Its agent, if it runs, goes on.
    pub fn archive(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let was_shown = self.chats.shown_index() == Some(index);
        self.menu = None;
        self.renaming = None;
        let Some(chat) = self.chats.remove(index) else {
            return;
        };
        keep_archived(&chat, true, &self.store);
        self.archived.push(chat);
        if was_shown {
            match self.chats.shown_index() {
                Some(shown) => self.show(shown, window, cx),
                None => window.focus(&self.focus_handle),
            }
        }
        cx.notify();
    }

    /// Brings the archived chat at `index` among the archived back to the
    /// list, and shows it.
    pub fn unarchive(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.archived.len() {
            return;
        }
        let chat = self.archived.remove(index);
        keep_archived(&chat, false, &self.store);
        let listed = self.add(chat);
        self.show(listed, window, cx);
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
        rows.extend(self.render_archived(cx));
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
    /// chats from 1: its name, or the input that renames it, and a mark of
    /// how it stands.
    fn render_chat(&self, index: usize, chat: &Chat, cx: &mut Context<Self>) -> Stateful<Div> {
        let number = index + 1;
        let state = chat
            .view
            .as_ref()
            .filter(|_| chat.open)
            .map(|view| view.read(cx).conversation().state());
        let (mark, color) = match state {
            Some(State::Busy | State::Stopping | State::Checking) => ("…", rgb(DIM)),
            Some(State::Asking) => ("?", rgb(ASKING)),
            Some(State::Closed) => ("closed", rgb(0x80_8080)),
            _ if chat.unread => ("●", rgb(LINK)),
            Some(State::Idle) | None => ("", rgb(DIM)),
        };
        let name = match &self.renaming {
            Some((renaming, input, _)) if *renaming == index => div()
                .key_context(RENAMING)
                .debug_selector(|| "renaming".to_owned())
                .flex_1()
                .min_w_0()
                .px_1()
                .border_1()
                .border_color(rgb(LINK))
                .rounded_sm()
                // A click in the input stays there, not showing the chat.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_action(cx.listener(|workspace, _: &CancelRename, window, cx| {
                    workspace.end_renaming(false, window, cx);
                }))
                .on_mouse_down_out(cx.listener(|workspace, _, window, cx| {
                    workspace.end_renaming(true, window, cx);
                }))
                .child(input.clone()),
            _ => div()
                .flex_1()
                .min_w_0()
                .truncate()
                // A closed chat is there to read.
                .when(!chat.open, |name| name.text_color(rgb(0x80_8080)))
                .child(chat.name.clone()),
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
                cx.listener(move |workspace, event: &ClickEvent, window, cx| {
                    if event.click_count() == 2 {
                        workspace.rename(index, window, cx);
                    } else {
                        workspace.show(index, window, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |workspace, event: &MouseDownEvent, _, cx| {
                    workspace.menu = Some((event.position, index));
                    cx.notify();
                }),
            )
            .child(name)
            .child(div().flex_shrink_0().text_color(color).child(mark));
        if self.chats.shown_index() == Some(index) {
            row = row.bg(rgb(0x2a_2a2a));
        } else {
            row = row.hover(|style| style.bg(rgb(0x26_2626)));
        }
        row
    }
}

impl Workspace {
    /// "Archived" at the foot of the list, if the human archived a chat,
    /// with the archived chats under it while it is open: each with its
    /// project's name, brought back to the list by a click.
    fn render_archived(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        if self.archived.is_empty() {
            return Vec::new();
        }
        let open = self.archived_open;
        let mut rows = vec![
            div()
                .id("archived")
                .debug_selector(|| "archived".to_owned())
                .flex()
                .gap_1()
                .px_2()
                .pt_3()
                .pb_1()
                .cursor_pointer()
                .text_color(rgb(DIM))
                .child(if open { "▾" } else { "▸" })
                .child(format!("Archived ({})", self.archived.len()))
                .on_click(cx.listener(|workspace, _, _, cx| {
                    workspace.archived_open = !workspace.archived_open;
                    cx.notify();
                })),
        ];
        if open {
            rows.extend(self.archived.iter().enumerate().map(|(index, chat)| {
                let number = index + 1;
                div()
                    .id(("archived-chat", number))
                    .debug_selector(move || format!("archived-{number}"))
                    .flex()
                    .gap_2()
                    .pl_4()
                    .pr_2()
                    .py_1()
                    .cursor_pointer()
                    .text_color(rgb(0x80_8080))
                    .hover(|style| style.bg(rgb(0x26_2626)))
                    .child(div().flex_1().min_w_0().truncate().child(chat.name.clone()))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .child(self.projects[chat.project].name.clone()),
                    )
                    .on_click(cx.listener(move |workspace, _, window, cx| {
                        workspace.unarchive(index, window, cx);
                    }))
            }));
        }
        rows
    }

    /// The menu of the right button on a chat, if it is open.
    fn render_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (at, index) = self.menu?;
        let item = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x04_395e)))
                .child(label)
        };
        let menu = div()
            .occlude()
            .min_w(px(160.))
            .p_1()
            .bg(rgb(0x25_2526))
            .border_1()
            .border_color(rgb(0x45_4545))
            .rounded_md()
            .shadow_md()
            .on_mouse_down_out(cx.listener(|workspace, _, _, cx| {
                workspace.menu = None;
                cx.notify();
            }))
            .child(item("rename-chat", "Rename").on_click(
                cx.listener(move |workspace, _, window, cx| workspace.rename(index, window, cx)),
            ))
            .child(item("archive-chat", "Archive").on_click(
                cx.listener(move |workspace, _, window, cx| workspace.archive(index, window, cx)),
            ));
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}

/// Whether an agent that stands as `state` is at work.
fn working(state: State) -> bool {
    matches!(
        state,
        State::Busy | State::Asking | State::Stopping | State::Checking
    )
}

/// Keeps in `store` whether `chat` is archived.
fn keep_archived(chat: &Chat, archived: bool, store: &Store) {
    if let Some(kept) = chat.kept
        && let Err(error) = store.archive_chat(kept, archived)
    {
        log::error!("whether the chat is archived is not kept: {error:#}");
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
            .children(self.render_menu(cx))
    }
}
