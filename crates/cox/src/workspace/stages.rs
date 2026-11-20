//! The stages of the folders the chats work in: making them, telling
//! them how the chats stand, and carrying out what the daemon asks of
//! them for the chats' agents.

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use cox_core::tabs::KeptTabs;
use gpui::{App, Context, Window};

use super::{Chat, NO_TAB, NO_TERMINAL, Staged, Workspace};
use crate::chat::view::StageView;
use crate::stage::Stage;
use crate::stage::changes::{CompareWith, ShowTurn};
use crate::stage::owners::ShowChat;
use coxd::agent::mcp::ToolRequest;
use coxd::chat::conversation::State;
use coxd::daemon::wire::{FromDaemon, StageAsk, ToDaemon};

impl Workspace {
    /// Gives the chat `key` the stage of the folder it works in, unless it
    /// has one: the stage made already for another chat there, or one made
    /// on `tabs`, as the stage had them; and carries out what was asked of
    /// it until then.
    pub(super) fn make_stage(
        &mut self,
        key: u64,
        tabs: Option<KeptTabs<u64>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.staged.contains_key(&key) {
            return;
        }
        let Some(chat) = self.chat(key) else {
            return;
        };
        let Some(view) = chat.view.clone() else {
            return;
        };
        let cwd = self.cwd_of(chat);
        if !self.folders.contains_key(&cwd) {
            self.new_stage(cwd.clone(), tabs, window, cx);
        }
        let shown_key = self.shown().map(Chat::key);
        let tools = self
            .folders
            .get(&cwd)
            .cloned()
            .and_then(|(shown, open_link)| {
                let stage = shown.view.clone().downcast::<Stage>().ok();
                view.update(cx, |view, cx| view.set_stage(shown, open_link, cx));
                // A stage works with the chat's agent: it carries out what the
                // agent's tools ask.
                let stage = stage?;
                let (tools, from_tools) = tokio::sync::mpsc::unbounded_channel();
                stage.update(cx, |stage, cx| {
                    stage.work(key, None, Some(from_tools), window, cx);
                    if shown_key == Some(key) {
                        stage.showing(key, cx);
                    }
                });
                Some(tools)
            });
        self.staged.insert(key, Staged { tools });
        self.sight(cx);
        for asked in self.waiting.remove(&key).unwrap_or_default() {
            self.ask_stage(asked, window, cx);
        }
    }

    /// Makes the stage of `folder`, on `tabs`, as the stage had them,
    /// which keeps its tabs, as those of its folder, names the chats, to
    /// hand a tab to one, and shows the chat of a tab the human asks it
    /// to.
    pub(super) fn new_stage(
        &mut self,
        folder: PathBuf,
        tabs: Option<KeptTabs<u64>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.asked_stages.remove(&folder);
        if self.folders.contains_key(&folder) {
            return;
        }
        let Some((made, open_link)) = self.stages.stage(folder.clone(), tabs, window, cx) else {
            return;
        };
        self.folders
            .insert(folder.clone(), (made.clone(), open_link));
        let Ok(stage) = made.view.downcast::<Stage>() else {
            return;
        };
        let wrote = Self::wrote_in(folder.clone(), cx);
        let link = self.link.clone();
        let keep: crate::stage::Keep = Rc::new(move |tabs: &KeptTabs<u64>| {
            link.send(ToDaemon::Tabs {
                folder: folder.clone(),
                tabs: tabs.clone(),
            });
        });
        let names = self.chat_names();
        stage.update(cx, |stage, cx| {
            stage.set_names(names);
            stage.set_wrote(wrote, cx);
            stage.keep_with(keep, cx);
        });
        let shows = cx.subscribe_in(&stage, window, |workspace, _, ShowChat(key), window, cx| {
            workspace.reveal(*key, window, cx);
        });
        self.shows_chats.push(shows);
        let turns = cx.subscribe_in(
            &stage,
            window,
            |workspace, _, &ShowTurn { chat, entry }, window, cx| {
                workspace.show_turn(chat, entry, window, cx);
            },
        );
        self.shows_chats.push(turns);
        let compares = cx.subscribe(&stage, |workspace, _, compare: &CompareWith, _| {
            workspace.tell(ToDaemon::CompareWith {
                folder: compare.folder.clone(),
                directory: compare.directory.clone(),
                base: compare.base.clone(),
            });
        });
        self.shows_chats.push(compares);
        self.tell_names();
        self.at_work(cx);
    }

    /// What names the chats, by their keys, for the stages: the names
    /// the workspace last told them, not the workspace itself, which a
    /// stage may be asked to name a chat from inside an update of, as
    /// the daemon asking a new chat's stage to work is heard in one.
    #[must_use]
    pub fn chat_names(&self) -> crate::stage::owners::Names {
        let names = Rc::clone(&self.chat_names);
        Rc::new(move |key, _: &App| names.borrow().get(&key).cloned())
    }

    /// Tells the stages the chats' names as they are now, those archived
    /// too, whose tabs a stage still shows.
    pub(super) fn tell_names(&self) {
        let names = self
            .chats
            .iter()
            .chain(&self.archived)
            .map(|chat| (chat.key, chat.name.clone()))
            .collect();
        *self.chat_names.borrow_mut() = names;
    }

    /// Asks the daemon for the stage of the project chosen, while no chat
    /// is shown, unless it is made or asked for already: the stage shows
    /// with no chat, as the home's does where Cox lands.
    pub(super) fn stage_chosen(&mut self) {
        if self.shown().is_some() {
            return;
        }
        let Some(project) = self.projects.get(self.chosen) else {
            return;
        };
        let folder = project.folder.clone();
        if self.folders.contains_key(&folder) || !self.asked_stages.insert(folder.clone()) {
            return;
        }
        self.tell(ToDaemon::StageOf { folder });
    }

    /// The stage in the middle of the window: that of the folder the chat
    /// shown works in, or, with none shown, of the project chosen, once
    /// it is made.
    pub(super) fn centered(&self) -> Option<&StageView> {
        let folder = match self.shown() {
            Some(chat) => self.cwd_of(chat),
            None => self.projects.get(self.chosen)?.folder.clone(),
        };
        self.folders.get(&folder).map(|(stage, _)| stage)
    }

    /// Tells each stage which chats' agents are at work, taking a turn,
    /// so that the others do not act on their pages.
    pub(super) fn at_work(&self, cx: &mut App) {
        let working: HashSet<u64> = self
            .chats
            .iter()
            .chain(&self.archived)
            .filter(|chat| {
                matches!(
                    chat.state,
                    State::Busy | State::Asking | State::Stopping | State::Checking
                )
            })
            .map(|chat| chat.key)
            .collect();
        for (stage, _) in self.folders.values() {
            if let Ok(stage) = stage.view.clone().downcast::<Stage>() {
                let working = working.clone();
                stage.update(cx, |stage, cx| stage.set_working(working, cx));
            }
        }
    }

    /// Tells the stages' Activity pages what the agent of the chat `key`
    /// does now, as its view has the conversation (see
    /// [`crate::stage::commands`]).
    pub(super) fn tell_commands(&self, key: u64, cx: &App) {
        let Some(chat) = self.chat(key) else {
            return;
        };
        let Some(view) = &chat.view else {
            return;
        };
        let entries = view.read(cx).conversation().entries();
        crate::stage::commands::tell(crate::stage::commands::Chat::of(key, entries));
    }

    /// Tells the stages' Activity pages of every chat with a view, as the
    /// list of chats changed, having the conversation of each chat at
    /// work, or that asks the human something, told, for the pages to show
    /// what it runs and asks while no one looks at it.
    pub(super) fn follow_commands(&mut self, cx: &mut Context<Self>) {
        let unseen: Vec<u64> = self
            .chats
            .iter()
            .filter(|chat| chat.view.is_none())
            .filter(|chat| matches!(chat.state, State::Busy | State::Checking | State::Asking))
            .map(|chat| chat.key)
            .collect();
        for key in unseen {
            self.view(key, cx);
        }
        let keys: Vec<u64> = self
            .chats
            .iter()
            .chain(&self.archived)
            .map(|chat| chat.key)
            .collect();
        crate::stage::commands::keep(&keys);
        for &key in &keys {
            self.tell_commands(key, cx);
        }
    }

    /// Tells each stage whether it is in sight now.
    pub(super) fn sight(&self, cx: &mut App) {
        let centered = self.centered().map(|stage| stage.panel.clone());
        for (stage, _) in self.folders.values() {
            let in_sight = centered.as_ref() == Some(&stage.panel);
            stage.panel.update(cx, |panel, cx| panel.set(in_sight, cx));
        }
    }

    /// Tells the stage of the chat `key`, if it has one, that the chat is
    /// shown.
    pub(super) fn stage_shows(&self, key: u64, cx: &mut Context<Self>) {
        if let Some(stage) = self
            .chat(key)
            .and_then(Chat::view)
            .and_then(|view| view.read(cx).stage())
        {
            stage.update(cx, |stage, cx| stage.showing(key, cx));
        }
    }

    /// Carries out what the daemon asks of a chat's stage: what the agent's
    /// tools ask, or a relay for an agent that starts; once the stage is
    /// made, when it is not yet.
    pub(super) fn ask_stage(
        &mut self,
        asked: FromDaemon,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = match &asked {
            FromDaemon::Stage { key, .. } | FromDaemon::Prepare { key, .. } => *key,
            _ => return,
        };
        if !self.staged.contains_key(&key) {
            self.waiting.entry(key).or_default().push(asked);
            self.view(key, cx);
            return;
        }
        let stage = self
            .chat(key)
            .and_then(Chat::view)
            .and_then(|view| view.read(cx).stage());
        match asked {
            FromDaemon::Prepare { id, .. } => {
                let relay = stage.as_ref().and_then(|_| self.stages.relay());
                let port = relay.as_ref().map(|relay| relay.port);
                if let (Some(stage), Some(relay)) = (stage, relay) {
                    stage.update(cx, |stage, cx| {
                        stage.work(key, Some(relay), None, window, cx);
                    });
                }
                self.tell(ToDaemon::Prepared { id, port });
            }
            FromDaemon::Stage { ask, .. } => {
                let request = self.request(ask, cx);
                match self
                    .staged
                    .get(&key)
                    .and_then(|staged| staged.tools.as_ref())
                {
                    Some(tools) => {
                        let _ = tools.send(request);
                    }
                    None => self.stages.unstaged(request),
                }
            }
            _ => {}
        }
    }

    /// What the daemon's `ask` asks of a stage, as the stage takes it: a
    /// command's answer goes back to the daemon once it ends.
    fn request(&mut self, ask: StageAsk, cx: &mut Context<Self>) -> ToolRequest {
        match ask {
            StageAsk::ShowMarkdown { path, text } => ToolRequest::ShowMarkdown { path, text },
            StageAsk::ShowFile { path, text, lines } => ToolRequest::ShowFile { path, text, lines },
            StageAsk::ShowChanges { folder } => ToolRequest::ShowChanges { folder },
            StageAsk::ShowMap { map, at } => ToolRequest::ShowMap { map, at },
            StageAsk::RunCommand { id, command, cwd } => {
                let (answer, ran) = tokio::sync::oneshot::channel();
                let link = self.link.clone();
                let waits = cx.spawn(async move |this, cx| {
                    let ran = ran.await.unwrap_or_else(|_| Err(NO_TERMINAL.to_owned()));
                    link.send(ToDaemon::Ran { id, ran });
                    let _ = this.update(cx, |workspace, _| workspace.running.remove(&id));
                });
                self.running.insert(id, waits);
                ToolRequest::RunCommand {
                    command,
                    cwd,
                    answer,
                    asks_human: false,
                }
            }
            StageAsk::MacWindow { id, act } => ToolRequest::MacWindow {
                act,
                answer: self.awaiting_sight(id, cx),
            },
            StageAsk::Android { id, act } => ToolRequest::Android {
                act,
                answer: self.awaiting_sight(id, cx),
            },
        }
    }

    /// Where what the agent sees goes, for the daemon's ask `id`: back to
    /// the daemon once the stage answers.
    fn awaiting_sight(
        &mut self,
        id: u64,
        cx: &mut Context<Self>,
    ) -> tokio::sync::oneshot::Sender<Result<coxd::agent::mcp::Seen, String>> {
        let (answer, seen) = tokio::sync::oneshot::channel();
        let link = self.link.clone();
        let waits = cx.spawn(async move |this, cx| {
            let seen = seen.await.unwrap_or_else(|_| Err(NO_TAB.to_owned()));
            link.send(ToDaemon::Seen { id, seen });
            let _ = this.update(cx, |workspace, _| workspace.running.remove(&id));
        });
        self.running.insert(id, waits);
        answer
    }

    /// Does `act` on the stage in the middle of the window, which opens.
    pub(super) fn on_stage(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        act: impl FnOnce(&mut Stage, &mut Window, &mut Context<Stage>),
    ) {
        if let Some(stage) = self
            .centered()
            .and_then(|stage| stage.view.clone().downcast::<Stage>().ok())
        {
            stage.update(cx, |stage, cx| act(stage, window, cx));
        }
    }
}
