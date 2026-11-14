//! The projects the daemon holds: renaming them, their folders with them,
//! what the agents propose of them, making one of a chat in none,
//! moving, putting away and deleting them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cox_core::entry::Answer;
use cox_core::store::ProjectId;

use super::wire::FromDaemon;
use super::{Daemon, working};
use crate::chat::conversation::{Entry, Proposed, State};
use crate::chat::sketch::{self, Sketch};
use crate::storage;

impl Daemon {
    /// Renames the project kept as `project` `name`, and its folder with
    /// it, in the folder it is in, unless the name is no folder's, a
    /// folder there has it already, or one of its chats is at work.
    pub(super) fn rename_project(&mut self, project: ProjectId, name: &str) {
        let Some(index) = self.index_of(project) else {
            return;
        };
        if let Err(why) = self.rename(index, name) {
            tracing::error!(name, "the project is not renamed: {why}");
        }
    }

    /// The folder the project at `index` would have, renamed `name`, or
    /// why it cannot be renamed so.
    fn renamed(&self, index: usize, name: &str) -> Result<PathBuf, String> {
        if index == 0 {
            return Err("Cox's own chats are in no project.".to_owned());
        }
        let name = name.trim();
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
            return Err("The name is its folder's: no / in it, nor a . first.".to_owned());
        }
        let from = &self.projects[index].folder;
        let to = from.with_file_name(name);
        if cox_core::folder::taken(&to, from) {
            return Err("There is a folder of that name there already.".to_owned());
        }
        if self.at_work(index) {
            return Err(
                "A chat of the project is at work: rename it once the turn ends.".to_owned(),
            );
        }
        Ok(to)
    }

    /// Renames the project at `index` `name`, and its folder with it, or
    /// answers why not. Its directory named as it was is renamed with it,
    /// unless it has one of the new name already.
    fn rename(&mut self, index: usize, name: &str) -> Result<(), String> {
        let to = self.renamed(index, name)?;
        let from = self.projects[index].folder.clone();
        if to == from {
            return Ok(());
        }
        let directory = self.rename_directory(&from, &to);
        if let Err(error) = std::fs::rename(&from, &to) {
            if let Some(directory) = directory {
                self.name_back(&from, &directory);
            }
            return Err(format!("{} is not renamed: {error}", from.display()));
        }
        if !self.relocate(index, &to) {
            if let Err(error) = std::fs::rename(&to, &from) {
                tracing::error!("{} is not named back: {error}", to.display());
            } else if let Some(directory) = directory {
                self.name_back(&from, &directory);
            }
            return Err("The project does not move.".to_owned());
        }
        Ok(())
    }

    /// Renames the directory of the project in `from` named as it is,
    /// as the project is renamed, its folder going to `to`, and has what
    /// Cox keeps of the stage there name where it is now; answers the
    /// directory renamed, if one is.
    fn rename_directory(&self, from: &Path, to: &Path) -> Option<Renamed> {
        let directories = from.join(crate::settings::DIRECTORIES);
        let was = directories.join(from.file_name()?);
        let now = directories.join(to.file_name()?);
        if !was.is_dir() || cox_core::folder::taken(&now, &was) {
            return None;
        }
        let renamed = Renamed { was, now };
        self.move_directory(from, &renamed.was, &renamed.now)
            .then_some(renamed)
    }

    /// Names the directory of the project in `project` that `renamed`
    /// says was renamed as it was again, as when the project is not
    /// renamed after all.
    fn name_back(&self, project: &Path, renamed: &Renamed) {
        self.move_directory(project, &renamed.now, &renamed.was);
    }

    /// Moves the directory `from` of the project in `project` to `to`,
    /// with what Cox keeps of the stage there; answers whether it moved.
    fn move_directory(&self, project: &Path, from: &Path, to: &Path) -> bool {
        if let Err(error) = std::fs::rename(from, to) {
            tracing::error!("{} is not renamed: {error}", from.display());
            return false;
        }
        let moved = cox_core::moved::Moved { from, to };
        if let Err(error) = self.store.move_in_stage(project, moved) {
            tracing::error!("the stage's tabs do not follow the directory: {error:#}");
        }
        if let Err(error) = storage::moved(moved) {
            tracing::error!("what Cox keeps of the directory does not move: {error}");
        }
        true
    }

    /// Shows `name`, which the agent of the chat `key` proposes for its
    /// project, on a card in the chat's conversation, and answers the
    /// project's name now, or why it cannot be named so.
    pub(super) fn propose_name(
        &mut self,
        key: u64,
        name: &str,
        why: Option<String>,
    ) -> Result<String, String> {
        let project = self.slot(key).ok_or("the chat has closed")?.project;
        if project == 0 {
            return Err(
                "This chat is in no project: Cox's own chats have no name to propose.".to_owned(),
            );
        }
        let name = name.trim();
        if name.starts_with('.') || name.contains(['/', '\\']) {
            return Err(
                "The name is the project's folder's: no / or \\ in it, nor a . first.".to_owned(),
            );
        }
        let now = self.projects[project]
            .folder
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        self.chat(key)
            .ok_or("the chat has closed")?
            .propose(Proposed::Name(name.to_owned()), why);
        Ok(now)
    }

    /// Keeps `about` as what the project at `index` is, or, blank, that it
    /// says nothing of what it is, for the agents of its chats that start
    /// from now on to be told.
    pub(super) fn set_about(&mut self, index: usize, about: &str) {
        let about = Some(about.trim())
            .filter(|about| !about.is_empty())
            .map(str::to_owned);
        let project = &mut self.projects[index];
        let Some(id) = project.id else {
            return;
        };
        if project.about == about {
            return;
        }
        if let Err(error) = self.store.keep_about(id, about.as_deref()) {
            tracing::error!("what the project is is not kept: {error:#}");
            return;
        }
        project.about = about;
        self.tell_listing();
    }

    /// Shows `about`, what the agent of the chat `key` proposes its
    /// project is, on a card in the chat's conversation, and answers what
    /// the project is now, if it says, or why it cannot be proposed.
    pub(super) fn propose_about(
        &mut self,
        key: u64,
        about: &str,
        why: Option<String>,
    ) -> Result<Option<String>, String> {
        let project = self.slot(key).ok_or("the chat has closed")?.project;
        if project == 0 {
            return Err("This chat is in no project: Cox's own chats have none to say.".to_owned());
        }
        let now = self.projects[project].about.clone();
        self.chat(key)
            .ok_or("the chat has closed")?
            .propose(Proposed::About(about.trim().to_owned()), why);
        Ok(now)
    }

    /// Shows the icon kept as `file`, made from the picture at `from`,
    /// which the agent of the chat `key` proposes for its project, on a
    /// card in the chat's conversation, and answers whether the project
    /// has an icon now, or why it cannot be proposed.
    pub(super) fn propose_icon(
        &mut self,
        key: u64,
        file: &str,
        from: &str,
        why: Option<String>,
    ) -> Result<bool, String> {
        let project = self.slot(key).ok_or("the chat has closed")?.project;
        let id = self.projects[project]
            .id
            .ok_or("This chat is in no project: Cox's own chats have no icon to propose.")?;
        let has = crate::project_icon::path(id).is_some_and(|path| path.is_file());
        let proposed = Proposed::Icon {
            file: file.to_owned(),
            from: from.to_owned(),
        };
        self.chat(key)
            .ok_or("the chat has closed")?
            .propose(proposed, why);
        Ok(has)
    }

    /// The human answered what was proposed at `index` in the chat `key`:
    /// lets it be, or carries it out, as its kind says.
    pub(super) fn answer_proposal(&mut self, key: u64, index: usize, answer: Answer) {
        let Some(chat) = self.chat(key) else {
            return;
        };
        let Some(proposed) = chat.proposed(index) else {
            return;
        };
        if answer == Answer::Dismissed {
            chat.answer_proposal(index, answer);
            return;
        }
        match proposed {
            Proposed::Goal(_) => {
                let Some(goal) = proposed.goal() else {
                    return;
                };
                chat.answer_proposal(index, answer);
                if answer == Answer::Set {
                    chat.set_goal(Some(goal));
                }
            }
            Proposed::Name(name) => self.rename_as_proposed(key, index, &name),
            Proposed::About(about) => {
                chat.answer_proposal(index, Answer::Set);
                let project = self.slot(key).map_or(0, |slot| slot.project);
                if project != 0 {
                    self.set_about(project, &about);
                }
            }
            Proposed::Icon { file, .. } => self.set_icon_as_proposed(key, index, &file),
            Proposed::Project { name, sketch, .. } => {
                self.make_project_as_proposed(key, index, &name, sketch);
            }
        }
    }

    /// Gives the project of the chat `key` the icon proposed at `index`,
    /// kept as `file`, in place of the one it had, and has the window draw
    /// it; the card stays when it cannot.
    fn set_icon_as_proposed(&mut self, key: u64, index: usize, file: &str) {
        let Some(project) = self.slot(key).map(|slot| slot.project) else {
            return;
        };
        let Some(id) = self.projects[project].id else {
            return;
        };
        if let Err(error) = crate::project_icon::set_proposed(id, file) {
            tracing::error!("the project's icon is not set: {error:#}");
            return;
        }
        self.tell(FromDaemon::Icon { project: id });
        if let Some(chat) = self.chat(key) {
            chat.answer_proposal(index, Answer::Set);
        }
    }

    /// Renames the project of the chat `key` `name`, as proposed at
    /// `index`; or, when it cannot be renamed so, notes why, the card
    /// staying for the human to answer.
    fn rename_as_proposed(&mut self, key: u64, index: usize, name: &str) {
        let Some(project) = self.slot(key).map(|slot| slot.project) else {
            return;
        };
        if let Err(why) = self.renamed(project, name) {
            if let Some(chat) = self.chat(key) {
                chat.note(format!("The project is not renamed {name}: {why}"));
            }
            return;
        }
        if let Some(chat) = self.chat(key) {
            chat.answer_proposal(index, Answer::Set);
        }
        if let Err(why) = self.rename(project, name)
            && let Some(chat) = self.chat(key)
        {
            chat.note(format!("The project is not renamed {name}: {why}"));
        }
    }

    /// Shows the project named `name`, which the agent of the chat `key`,
    /// in no project, proposes to make of it, on a card in the chat's
    /// conversation, for what the sketch `sketch` names tried, if given, or
    /// else the chat itself, when it is a sketch from nothing; and answers
    /// where the project is to be, or why it cannot be proposed.
    pub(super) fn propose_project(
        &mut self,
        key: u64,
        name: &str,
        why: Option<String>,
        sketch: Option<String>,
    ) -> Result<PathBuf, String> {
        let name = name.trim();
        let folder = self.made_project(key, name)?;
        let sketch = match sketch {
            Some(sketch) => Some(self.sketch_named(key, &sketch)?),
            // A sketch proposes a project of what it made.
            None => self
                .sketch_of_project(key, key)
                .zip(self.slot(key))
                .map(|(folder, slot)| (slot.name.clone(), folder)),
        };
        let (sketch_name, sketch) = sketch.unzip();
        let proposed = Proposed::Project {
            name: name.to_owned(),
            sketch,
            sketch_name,
        };
        self.chat(key)
            .ok_or("the chat has closed")?
            .propose(proposed, why);
        Ok(folder)
    }

    /// The folder of the project named `name` to be made of the chat `key`,
    /// in no project, or why it cannot be.
    fn made_project(&self, key: u64, name: &str) -> Result<PathBuf, String> {
        let slot = self.slot(key).ok_or("the chat has closed")?;
        if slot.project != 0 {
            return Err("This chat is in a project already.".to_owned());
        }
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
            return Err(
                "The name is the project's folder's: no / or \\ in it, nor a . first.".to_owned(),
            );
        }
        let folder = self.projects[0]
            .folder
            .join(crate::settings::PROJECTS)
            .join(name);
        if folder.exists() || self.project_at(&folder).is_some() {
            return Err(format!("There is a project {name} already."));
        }
        Ok(folder)
    }

    /// The chat `key` and the chats opened under it, and under those, by
    /// their keys.
    fn under(&self, key: u64) -> Vec<u64> {
        let mut keys = vec![key];
        let mut at = 0;
        while let Some(&parent) = keys.get(at) {
            keys.extend(
                self.chats
                    .iter()
                    .chain(&self.archived)
                    .filter(|slot| slot.parent == Some(parent) && !keys.contains(&slot.key))
                    .map(|slot| slot.key)
                    .collect::<Vec<_>>(),
            );
            at += 1;
        }
        keys
    }

    /// The chats a project made of the chat `key` takes: the chat it was
    /// opened under, and that one under, up to the first, and the chats
    /// opened under that, by their keys.
    fn family(&self, key: u64) -> Vec<u64> {
        let mut first = key;
        while let Some(parent) = self
            .slot(first)
            .and_then(|slot| slot.parent)
            .filter(|&parent| parent != key && self.slot(parent).is_some())
        {
            first = parent;
        }
        self.under(first)
    }

    /// The folder of the sketch `sketch`, when it is one from nothing,
    /// of the chats a project made of the chat `key` takes, that can end.
    fn sketch_of_project(&self, key: u64, sketch: u64) -> Option<PathBuf> {
        if !self.family(key).contains(&sketch) || !self.ends_sketch(sketch) {
            return None;
        }
        self.sketch_of(sketch)
            .filter(|(from, _)| *from == Sketch::Empty)
            .map(|(_, folder)| folder)
    }

    /// The sketch from nothing of the chats a project made of the chat
    /// `key` takes (see [`Self::family`]), that can end, that
    /// `given` names, as the human's list shows it, or by its chat's
    /// number, as Cox says it: its name and folder, or why there is none.
    fn sketch_named(&self, key: u64, given: &str) -> Result<(String, PathBuf), String> {
        let sketches: Vec<(u64, &str, PathBuf)> = self
            .family(key)
            .into_iter()
            .filter_map(|sketch| {
                let folder = self.sketch_of_project(key, sketch)?;
                Some((sketch, self.slot(sketch)?.name.as_str(), folder))
            })
            .collect();
        let number = given.trim_start_matches("chat ").parse::<u64>().ok();
        sketches
            .iter()
            .find(|(_, name, _)| name.trim() == given)
            .or_else(|| {
                sketches
                    .iter()
                    .find(|(sketch, _, _)| Some(*sketch) == number)
            })
            .map(|(_, name, folder)| ((*name).to_owned(), folder.clone()))
            .ok_or_else(|| {
                let names: Vec<String> = sketches
                    .iter()
                    .map(|(_, name, _)| format!("\"{name}\""))
                    .collect();
                if names.is_empty() {
                    "No sketch from nothing of this chat's can go into the project: give no \
                     sketch."
                        .to_owned()
                } else {
                    format!(
                        "No sketch from nothing of this chat's is named \"{given}\"; those \
                         that can go into the project: {}.",
                        names.join(", ")
                    )
                }
            })
    }

    /// Makes the project named `name` proposed at `index` in the chat
    /// `key`, for the sketch in `sketch`, if any, the chats it takes moving
    /// into it (see [`Self::family`]); or, when it cannot be made, notes
    /// why, the card staying for the human to answer.
    fn make_project_as_proposed(
        &mut self,
        key: u64,
        index: usize,
        name: &str,
        sketch: Option<PathBuf>,
    ) {
        let made = self.made_project(key, name).and_then(|folder| {
            let sketch = match sketch {
                None => None,
                Some(folder) => Some(
                    self.chats
                        .iter()
                        .find(|slot| {
                            slot.setup
                                .as_ref()
                                .is_some_and(|setup| setup.folder.as_ref() == Some(&folder))
                        })
                        .map(|slot| slot.key)
                        .filter(|&sketch| {
                            self.sketch_of_project(key, sketch).as_ref() == Some(&folder)
                        })
                        .map(|sketch| (sketch, folder))
                        .ok_or("the sketch has ended")?,
                ),
            };
            let at_work = self
                .family(key)
                .into_iter()
                .any(|key| self.slot(key).is_some_and(|slot| working(slot.state)));
            if at_work {
                return Err(
                    "A chat that moves into the project is at work: make it once the turn ends."
                        .to_owned(),
                );
            }
            Ok((folder, sketch))
        });
        let (folder, sketch) = match made {
            Ok(made) => made,
            Err(why) => {
                if let Some(chat) = self.chat(key) {
                    chat.note(format!("The project {name} is not made: {why}"));
                }
                return;
            }
        };
        if let Some(chat) = self.chat(key) {
            chat.answer_proposal(index, Answer::Set);
        }
        if let Err(why) = self.make_project_of(key, name, &folder, sketch)
            && let Some(chat) = self.chat(key)
        {
            chat.note(format!("The project {name} is not made: {why}"));
        }
    }

    /// Makes the project named `name` in `folder` of the chat `key`, in no
    /// project, for what the sketch `sketch`, its key and folder, tried, if
    /// given, what it made kept apart (see [`Self::keep_apart`]), and the
    /// sketch ending; the chats it takes (see [`Self::family`]) move into
    /// the project, each
    /// closed, its agent stopped, to go on there from the next line sent,
    /// with the conversation so far.
    fn make_project_of(
        &mut self,
        key: u64,
        name: &str,
        folder: &Path,
        sketch: Option<(u64, PathBuf)>,
    ) -> Result<(), String> {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("Cox cannot make {}: {error}", folder.display()))?;
        // The chats the sketch ends are told so as it ends.
        let mut told = Vec::new();
        if let Some((sketch, from)) = sketch {
            told = self
                .chats
                .iter()
                .chain(&self.archived)
                .filter(|slot| {
                    slot.setup
                        .as_ref()
                        .and_then(|setup| setup.folder.as_ref())
                        .is_some_and(|works| works.starts_with(&from))
                })
                .map(|slot| slot.key)
                .collect();
            let kept = self.keep_apart(sketch, &from).map_err(|error| {
                let _ = std::fs::remove_dir_all(folder);
                format!("Cox cannot keep what the sketch made: {error}")
            })?;
            let (note, _) = self.with_sketch_kept(
                sketch,
                &folder.join(crate::raw::PROJECT_RAW),
                Some(name),
                &format!(
                    "The sketch moved into the project {name}, to make there what it tried; what \
                     it made is not the project's.{}",
                    kept.where_kept()
                ),
            );
            self.end_sketch(sketch, &from, Ok(note));
        }
        let index = self.project(folder.to_owned());
        let project = self.projects[index].id;
        let note = format!(
            "The chat moved into the project {name}, in {}. A line sent starts the agent again \
             there, with the conversation so far.",
            folder.display()
        );
        let moving = self.family(key);
        let store = Rc::clone(&self.store);
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            if !moving.contains(&slot.key) {
                continue;
            }
            slot.project = index;
            let kept_to = slot.folder.take();
            if let Some(kept) = slot.kept {
                if let Err(error) = store.move_chat(kept, project) {
                    tracing::error!("the chat's project is not kept: {error:#}");
                }
                // It works in the project's folder now.
                if kept_to.is_some()
                    && let Err(error) = store.keep_folder(kept, None)
                {
                    tracing::error!("the chat's folder is not kept: {error:#}");
                }
                // The agent's session was of the folder it worked in; a
                // sketch's goes on working in its own.
                let sketch = slot
                    .setup
                    .as_ref()
                    .is_some_and(|setup| setup.folder.is_some());
                if !sketch && let Err(error) = store.forget_session(kept) {
                    tracing::error!("the chat's session is not forgotten: {error:#}");
                }
            }
            slot.state = State::Closed;
            if told.contains(&slot.key) {
                continue;
            }
            if let Some(chat) = &mut slot.chat {
                chat.close(note.clone());
            } else if let Some(kept) = slot.kept {
                let entries = store.entries(kept).map(|entries| entries.len());
                if let Ok(number) = entries
                    && let Err(error) = store.keep_entry(kept, number, &Entry::Note(note.clone()))
                {
                    tracing::error!("the chat is not kept as it goes: {error:#}");
                }
            }
        }
        // The window hears of the project before it hears the chats again.
        self.tell_listing();
        for moved in moving {
            self.read_again(moved);
        }
        self.reveals.push(key);
        Ok(())
    }

    /// Whether a chat of the project at `index` is at work.
    fn at_work(&self, index: usize) -> bool {
        self.chats
            .iter()
            .chain(&self.archived)
            .any(|slot| slot.project == index && working(slot.state))
    }

    /// Has the project at `index`, whose folder is now `folder`, work
    /// there, its chats noting it and starting again there, and answers
    /// whether it does, which it does not while one of its chats is at
    /// work.
    fn relocate(&mut self, index: usize, folder: &Path) -> bool {
        let Some(project) = self.projects[index].id else {
            return false;
        };
        let from = self.projects[index].folder.clone();
        if self.at_work(index) {
            tracing::error!("the project does not move while a chat of it is at work");
            return false;
        }
        let name = folder.file_name().map_or_else(
            || folder.to_string_lossy().into_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        if let Err(error) = self.store.move_project(project, folder, &name) {
            tracing::error!("the project does not move: {error:#}");
            return false;
        }
        let moved = cox_core::moved::Moved {
            from: &from,
            to: folder,
        };
        if let Err(error) = storage::moved(moved) {
            tracing::error!("what Cox keeps of the folder's triggers does not move: {error}");
        }
        self.scheduled = std::mem::take(&mut self.scheduled)
            .into_iter()
            .map(|(id, key)| (moved.text(&id).unwrap_or(id), key))
            .collect();
        tracing::info!(from = %from.display(), to = %folder.display(), "the project moved");
        folder.clone_into(&mut self.projects[index].folder);
        self.projects[index].name = name;
        let note = format!(
            "The project's folder moved from {} to {}. A line sent starts the agent again there, \
             with the conversation so far.",
            from.display(),
            folder.display()
        );
        let mut moving = Vec::new();
        let mut sketches = Vec::new();
        let store = Rc::clone(&self.store);
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            // A chat that keeps to another folder of the project stays
            // there.
            if slot.project != index || slot.folder.is_some() {
                continue;
            }
            // A chat of a sketch works in the sketch's folder, which
            // stays where it is; a sketch from the project is a worktree
            // of its repository, which Git is told moved.
            if let Some(setup) = &slot.setup
                && let Some(works) = &setup.folder
            {
                if setup.sketch == Some(Sketch::Project) && works.exists() {
                    sketches.push(works.clone());
                }
                continue;
            }
            if let Some(kept) = slot.kept {
                // The agent's session was of the folder it worked in.
                if let Err(error) = store.forget_session(kept) {
                    tracing::error!("the chat's session is not forgotten: {error:#}");
                }
            }
            if let Some(chat) = &mut slot.chat {
                chat.note(note.clone());
            } else if let Some(kept) = slot.kept {
                let entries = store.entries(kept).map(|entries| entries.len());
                if let Ok(number) = entries
                    && let Err(error) = store.keep_entry(kept, number, &Entry::Note(note.clone()))
                {
                    tracing::error!("the chat is not kept as it goes: {error:#}");
                }
            }
            slot.state = State::Closed;
            moving.push(slot.key);
        }
        if !sketches.is_empty() {
            sketch::repair(folder, &sketches);
        }
        // The window hears where the project is before it hears the
        // chats again, with the tabs of its stage there.
        self.tell_listing();
        for key in moving {
            self.read_again(key);
        }
        true
    }

    /// The project kept as `project`, by its place in the projects.
    pub(super) fn index_of(&self, project: ProjectId) -> Option<usize> {
        self.projects
            .iter()
            .position(|kept| kept.id == Some(project) && !kept.deleted)
    }

    /// Puts the project at `index` away, off the rail, its chats going on
    /// as they were, or brings it back.
    pub(super) fn put_away(&mut self, index: usize, archived: bool) {
        let Some(id) = self.projects[index].id else {
            return;
        };
        if let Err(error) = self.store.archive_project(id, archived) {
            tracing::error!("whether the project is archived is not kept: {error:#}");
            return;
        }
        self.projects[index].archived = archived;
    }

    /// Deletes the project at `index`, unless one of its chats is at
    /// work: its folder goes to the trash, whole, for the human to take
    /// back from there; its chats stop, and are forgotten with what they
    /// showed, the tabs of its stage and its icon.
    pub(super) fn delete_project(&mut self, index: usize) -> Result<(), String> {
        let Some(id) = self.projects[index].id else {
            return Err("Cox's own chats are in no project.".to_owned());
        };
        if self.at_work(index) {
            return Err(
                "A chat of the project is at work: delete it once the turn ends.".to_owned(),
            );
        }
        let folder = self.projects[index].folder.clone();
        if folder.exists()
            && let Err(why) = crate::trash::put(&folder)
        {
            // The trash can fail after it took the folder, as for a name
            // it was given beside it; the folder gone is what counts.
            if folder.exists() {
                return Err(why);
            }
            tracing::warn!("the trash took the folder, but said: {why}");
        }
        tracing::info!(folder = %folder.display(), "the project is deleted");
        if let Err(error) = self.store.delete_project(id) {
            tracing::error!("the project's chats are not forgotten: {error:#}");
        }
        if let Err(error) = crate::project_icon::remove(id) {
            tracing::error!("the project's icon is not removed: {error:#}");
        }
        let mut gone = HashSet::new();
        for chats in [&mut self.chats, &mut self.archived] {
            for mut slot in chats.extract_if(.., |slot| slot.project == index) {
                if let Some(chat) = &mut slot.chat {
                    chat.close("The project was deleted.".to_owned());
                }
                gone.insert(slot.key);
            }
        }
        // An agent that waits for one of them waits no more.
        for (_, answer) in self.waiting.extract_if(.., |(key, _)| gone.contains(key)) {
            answer
                .send(Err("the chat's project was deleted".to_owned()))
                .ok();
        }
        let project = &mut self.projects[index];
        project.deleted = true;
        project.archived = false;
        Ok(())
    }
}

/// A directory of a project renamed with it, from `was` to `now`.
struct Renamed {
    was: PathBuf,
    now: PathBuf,
}
