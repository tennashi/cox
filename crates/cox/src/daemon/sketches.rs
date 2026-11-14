//! The sketches of the daemon's chats: starting one beside a chat, and
//! ending it, summed up, kept as a source, or opened as a project.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cox_core::goal::Goal;
use cox_core::store::Named;

use super::{Daemon, Ends, NAME_LENGTH, Sketching, make_sketch, working};
use coxd::agent;
use coxd::agent::mcp::{OpenedChat, Waited};
use coxd::agent::setup::{Policy, Setup, keeps_in_wiki};
use coxd::chat::conversation::{Entry, State};
use coxd::chat::sketch::{self, Sketch};
use coxd::storage;
use coxd::workflow;

impl Daemon {
    /// Opens a sketch beside the chat `key`, under it in the list, that
    /// starts from `started`, to try out `what`, and answers how it goes
    /// by, or why it cannot open. The sketch's agent is set up as the
    /// chat's is, and told what the chat talked through so far, and what
    /// it tries; the chat goes on in the project, apart from it, and hears
    /// how it ends (see [`Self::finish_sketch`]).
    pub(super) fn start_sketch(
        &mut self,
        key: u64,
        started: Sketch,
        what: &str,
    ) -> Result<OpenedChat, String> {
        let slot = self
            .chats
            .iter()
            .find(|slot| slot.key == key)
            .ok_or("the chat is not in the list")?;
        let setup = slot.setup.clone().unwrap_or_default();
        if let (Some(_), Some(folder)) = (setup.sketch, &setup.folder) {
            return Err(format!(
                "the chat is a sketch already, working in {}",
                folder.display()
            ));
        }
        let from = setup
            .folder
            .clone()
            .unwrap_or_else(|| self.projects[slot.project].folder.clone());
        let talked = self
            .chat(key)
            .map(|chat| agent::talked_through(chat.conversation().entries()))
            .unwrap_or_default();
        let folder = make_sketch(started, &from)?;
        let context = [setup.context.clone(), talked]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");
        let setup = Setup {
            context: (!context.is_empty()).then_some(context),
            folder: Some(folder.clone()),
            sketch: Some(started),
            policy: sketch::allow(setup.policy, &folder, &workflow::sketch().allow),
            ..setup
        };
        let line = workflow::SKETCH_START.say(&[
            ("folder", &folder.display().to_string()),
            ("from", started.what()),
            ("what", what),
        ]);
        let opened = self.open_under(key, &line, setup, None)?;
        // Named after what it tries, not after the line Cox sends it.
        let name: String = what.chars().take(NAME_LENGTH).collect();
        let store = Rc::clone(&self.store);
        if let Some(slot) = self.slot_mut(opened.number) {
            slot.name_as(&name, Named::FirstLine, &store);
        }
        if let Some(chat) = self.chat(key) {
            chat.note(format!(
                "Sketch: chat {} opens beside this chat, in {}, to try: {what}",
                opened.number,
                folder.display()
            ));
        }
        Ok(OpenedChat { name, ..opened })
    }

    /// The sketch the chat `key` is, what it started from and the folder
    /// it works in, while the folder is there.
    pub(super) fn sketch_of(&self, key: u64) -> Option<(Sketch, PathBuf)> {
        super::sketch_of(self.slot(key)?.setup.as_ref())
    }

    /// What the human may do of sketches in the chat `key`, as the window's
    /// menu offers it: start one beside it, unless it is a sketch, or end
    /// it, a sketch that can end.
    pub(super) fn sketching(&self, key: u64) -> Option<Sketching> {
        let slot = self.slot(key)?;
        match slot.setup.as_ref().and_then(|setup| setup.sketch) {
            None => Some(Sketching::Starts {
                project: self.projects[slot.project].folder.clone(),
            }),
            Some(_) => self
                .sketch_of(key)
                .filter(|_| self.ends_sketch(key))
                .map(|(from, _)| Sketching::Ends { from }),
        }
    }

    /// Whether the sketch `key` can end: its folder is there, and was not
    /// opened as a project, which the human goes on with, and it is not on
    /// its way to an end already.
    pub(super) fn ends_sketch(&self, key: u64) -> bool {
        self.slot(key).is_some_and(|slot| {
            super::ends_sketch(slot.setup.as_ref(), slot.ending.is_some(), |folder| {
                self.project_at(folder).is_some()
            })
        })
    }

    /// Ends the sketch `key` as `ending` says: asks its agent to sum it up
    /// in the conversation first, if the ending says to, once its turn
    /// ends if one goes on, and ends it once that is done (see
    /// [`Self::finish_sketch`]).
    pub(super) fn end_sketch_as(&mut self, key: u64, ending: workflow::Ending) {
        let ends = self.ends_sketch(key)
            && self
                .sketch_of(key)
                .is_some_and(|(from, _)| ending.ends(from));
        if !ends {
            return;
        }
        self.reveals.push(key);
        self.read(key);
        let Some(slot) = self.slot_mut(key) else {
            return;
        };
        let waits = ending.sum_up.is_some() && working(slot.state);
        let name = ending.name.clone();
        slot.ending = Some((Ends::Waiting, ending));
        if waits {
            if let Some(chat) = self.chat(key) {
                chat.note(format!(
                    "The sketch ends as \"{name}\" once this turn ends, and its agent has \
                     summed it up."
                ));
            }
        } else {
            self.sum_up(key);
        }
    }

    /// Asks the agent of the sketch `key`, which the human ends, to sum it
    /// up in the conversation, as its ending says, starting it again if it
    /// is closed; the sketch ends once it is done, or at once if the ending
    /// asks for none, or the agent cannot be asked.
    pub(super) fn sum_up(&mut self, key: u64) {
        let Some((_, ending)) = self.slot(key).and_then(|slot| slot.ending.clone()) else {
            return;
        };
        let Some(text) = ending.sum_up.clone() else {
            self.finish_sketch(key);
            return;
        };
        let Some((_, folder)) = self.sketch_of(key) else {
            if let Some(slot) = self.slot_mut(key) {
                slot.ending = None;
            }
            return;
        };
        let left_out = storage::path(storage::Kept::Sketches)
            .ok_or_else(|| "Cox cannot tell where the sketches are".to_owned())
            .and_then(|sketches| sketch::left_out(&folder, &sketches))
            .unwrap_or_else(|error| {
                tracing::warn!("cannot tell what the sketch changed: {error}");
                Vec::new()
            });
        let Some(line) = workflow::say(&text, &[("left_out", &sketch::left_out_lines(&left_out))])
        else {
            if let Some(chat) = self.chat(key) {
                chat.note(format!(
                    "The sketch is not summed up: the workflow has no {text}."
                ));
            }
            self.finish_sketch(key);
            return;
        };
        if let Some(slot) = self.slot_mut(key) {
            slot.ending = Some((Ends::Summing, ending));
        }
        self.go_on(key, &line);
        let asked = self
            .chat(key)
            .is_some_and(|chat| working(chat.conversation().state()));
        if !asked {
            self.finish_sketch(key);
        }
    }

    /// Ends the sketch `key`, whose agent has summed it up if its ending
    /// asked, as the ending says. Its folder moves into a project of its
    /// own, as a directory named as the project is, to go on with what the
    /// sketch made, for an ending that opens it (see
    /// [`Self::open_as_project`]); or else it goes, what the sketch changed kept apart first if
    /// the ending says to (see [`sketch::discard`]), and the sketch ends,
    /// its chat going on in the project toward the ending's goal, if it
    /// has one, its agent told what the ending says then.
    pub(super) fn finish_sketch(&mut self, key: u64) {
        let Some((_, ending)) = self.slot_mut(key).and_then(|slot| slot.ending.take()) else {
            return;
        };
        let Some((started, folder)) = self.sketch_of(key).filter(|_| self.ends_sketch(key)) else {
            return;
        };
        if ending.open {
            self.open_as_project(key, &folder);
            return;
        }
        let Some(slot) = self.slot(key) else {
            return;
        };
        let title = slot.name.clone();
        let project = slot.project;
        let sketch_kept = slot.kept;
        let sketch_keeps = keeps_in_wiki(slot.setup.as_ref());
        let parent = slot.parent;
        // What the sketch's agent said last, as its summing up.
        let summary = self
            .chat(key)
            .and_then(|chat| chat.last_reply())
            .unwrap_or_default();
        let discarded = storage::path(storage::Kept::Sketches)
            .ok_or_else(|| "Cox cannot tell where to keep discarded sketches".to_owned())
            .and_then(|sketches| sketch::discard(started, &folder, &title, &sketches, ending.keep));
        let kept = discarded
            .as_ref()
            .ok()
            .and_then(|discarded| discarded.kept.as_ref())
            .map(ToString::to_string)
            .unwrap_or_default();
        let (sources, of) = self.sources_of(project);
        let (ended, source) = match discarded.as_ref().map(sketch::Discarded::note) {
            Ok(note) => {
                let (note, source) = self.with_sketch_kept(key, &sources, of.as_deref(), &note);
                (Ok(note), source)
            }
            Err(error) => (Err(error.clone()), None),
        };
        let note = ended.as_ref().ok().cloned();
        let ended = self.end_sketch(key, &folder, ended);
        if !ended {
            return;
        }
        // The chat the sketch was opened beside goes on with it, in the
        // project; a sketch opened beside none goes on itself.
        let goes_on = parent
            .filter(|parent| self.chats.iter().any(|slot| slot.key == *parent))
            .unwrap_or(key);
        if goes_on != key {
            // The human goes on there.
            self.reveals.push(goes_on);
            if let (Some(chat), Some(note)) = (self.chat(goes_on), note.as_ref()) {
                chat.note(format!("The sketch \"{title}\" (chat {key}) ended: {note}"));
            }
            // Its triggers run where the human goes on, in the project.
            if let Some(chat) = self.chat(goes_on) {
                chat.fire(
                    &crate::trigger::On::SketchEnd,
                    format!(
                        "The sketch \"{title}\" ended{}",
                        note.as_ref()
                            .map_or(".".to_owned(), |note| format!(": {note}"))
                    ),
                    false,
                );
            }
        }
        // What the sketch found, and why it ended, is the sketch's agent's
        // to keep, as the human chooses where they go on.
        let keeps = (goes_on != key).then_some(sketch_kept);
        if sketch_keeps
            && keeps.is_none_or(|kept| kept.is_some())
            && let Some(chat) = self.chat(goes_on)
        {
            chat.ask_to_keep(format!("The sketch \"{title}\" ended."), keeps.flatten());
        }
        if let Some(goal) = &ending.goal
            && let Some(chat) = self.chat(goes_on)
        {
            chat.set_goal(Goal::parse(goal));
        }
        if let Some(text) = &ending.then {
            let source = source.unwrap_or_default();
            let values = [
                ("kept", kept.as_str()),
                ("summary", summary.as_str()),
                ("source", source.as_str()),
            ];
            match workflow::say(text, &values) {
                Some(line) => self.go_on(goes_on, &line),
                None => {
                    if let Some(chat) = self.chat(goes_on) {
                        chat.note(format!(
                            "The agent is told nothing: the workflow has no {text}."
                        ));
                    }
                }
            }
        }
    }

    /// Ends the sketch `key`, which worked in `folder`, as `ended` says, or
    /// says why it could not, and answers whether it ended. Ended, the
    /// sketch's chat goes on in the project, as do the chats it opened,
    /// which worked in its folder too: each is closed, its agent stopped,
    /// and set up to work in the project, asking the human as any chat
    /// does, so that a line sent starts its agent again there, with the
    /// conversation so far.
    pub(super) fn end_sketch(
        &mut self,
        key: u64,
        folder: &Path,
        ended: Result<String, String>,
    ) -> bool {
        self.reveals.push(key);
        let note = match ended {
            Ok(note) => note,
            Err(error) => {
                if let Some(chat) = self.chat(key) {
                    chat.note(format!("The sketch could not end: {error}"));
                }
                return false;
            }
        };
        let goes_on = "The chat goes on in the project: a line sent starts its agent again \
                       there, with the conversation so far.";
        let mut ended = Vec::new();
        let store = Rc::clone(&self.store);
        for slot in self.chats.iter_mut().chain(&mut self.archived) {
            let worked_there = slot.setup.as_ref().is_some_and(|setup| {
                setup
                    .folder
                    .as_ref()
                    .is_some_and(|works| works.starts_with(folder))
            });
            if !worked_there {
                continue;
            }
            let told = if slot.key == key {
                format!("{note} {goes_on}")
            } else {
                format!(
                    "The sketch this chat worked in has ended, and its folder is gone. {goes_on}"
                )
            };
            if let Some(chat) = &mut slot.chat {
                chat.close(told.clone());
            } else {
                // Read once it is shown, it says so.
                if let Some(kept) = slot.kept {
                    let entries = store.entries(kept).map(|entries| entries.len());
                    if let Ok(number) = entries
                        && let Err(error) =
                            store.keep_entry(kept, number, &Entry::Note(told.clone()))
                    {
                        tracing::error!("the chat is not kept as it goes: {error:#}");
                    }
                }
            }
            slot.state = State::Closed;
            slot.setup = slot.setup.take().map(|setup| Setup {
                folder: None,
                sketch: None,
                policy: Policy::default(),
                ..setup
            });
            if let Some(kept) = slot.kept
                && let Err(error) =
                    store.keep_setup(kept, slot.setup.as_ref().map(Setup::to_json).as_deref())
            {
                tracing::error!("the chat's setup is not kept: {error:#}");
            }
            ended.push((slot.key, told));
        }
        // What waits for them hears how they ended.
        for (ended, told) in &ended {
            for (_, answer) in self.waiting.extract_if(.., |(waited, _)| waited == ended) {
                let stands = Waited {
                    asking: None,
                    said: Some(told.clone()),
                };
                answer.send(Ok(stands)).ok();
            }
        }
        // Each is read again, closed, to go on with in the project.
        for (ended, _) in ended {
            self.read_again(ended);
        }
        true
    }

    /// The sources of the project at `index`, in its folder, and its name;
    /// or, for Cox's own, of no project, the human's: where the settings
    /// say, or else in the home this Cox works in.
    fn sources_of(&self, index: usize) -> (PathBuf, Option<String>) {
        let project = &self.projects[index];
        if project.id.is_none() {
            let saved = coxd::settings::Saved::read().raw;
            (
                saved.unwrap_or_else(|| project.folder.join(coxd::raw::RAW)),
                None,
            )
        } else {
            (
                project.folder.join(coxd::raw::PROJECT_RAW),
                Some(project.name.clone()),
            )
        }
    }

    /// `ended`, how the sketch `key` ends, with where what it did is kept
    /// (see [`Self::keep_sketch`]), in the sources at `root` of the
    /// project `project`, if it is one, or why it is not; and the source's
    /// name, once it is kept.
    pub(super) fn with_sketch_kept(
        &self,
        key: u64,
        root: &Path,
        project: Option<&str>,
        ended: &str,
    ) -> (String, Option<String>) {
        match self.keep_sketch(key, root, project, ended) {
            Ok(name) => (
                format!(
                    "{ended} What it did is kept as the source {name}, for what is made of it \
                     to name."
                ),
                Some(name),
            ),
            Err(error) => {
                tracing::error!("what the sketch did is not kept: {error}");
                (
                    format!("{ended} What it did is not kept as a source: {error}"),
                    None,
                )
            }
        }
    }

    /// Keeps what the sketch `key` did as a source, in the sources at
    /// `root`, of the project `project`, if it is one, under
    /// [`sketch::SOURCES`]: how it ended, `ended`, and all that was said in
    /// it, as Cox kept it, whatever the human's workflow says of how it
    /// ends; answers the source's name.
    fn keep_sketch(
        &self,
        key: u64,
        root: &Path,
        project: Option<&str>,
        ended: &str,
    ) -> Result<String, String> {
        let slot = self.slot(key).ok_or("the sketch has closed")?;
        let entries = match &slot.chat {
            Some(chat) => chat.conversation().entries().to_vec(),
            None => slot
                .kept
                .map(|kept| self.store.entries(kept))
                .transpose()
                .map_err(|error| format!("its conversation cannot be read: {error:#}"))?
                .unwrap_or_default(),
        };
        let said =
            agent::said_in_sketch(&entries).unwrap_or_else(|| "Nothing was said in it.".to_owned());
        let number = slot.kept.map(|kept| kept.number().to_string());
        let from = number.as_ref().map_or_else(
            || format!("sketch \"{}\"", slot.name),
            |number| format!("chat {number}"),
        );
        let showed = storage::path(storage::Kept::Media).map_or_else(String::new, |kept| {
            showed_in_sketch(&entries, &kept, &root.join(sketch::SOURCES))
        });
        let text = format!(
            "## How it ended\n\n{ended}\n\n{showed}## What was said in it\n\nAs Cox kept it, \
             where [you] stands for the sketch's agent.\n\n{said}\n"
        );
        let day = coxd::wiki::today();
        coxd::raw::keep(
            root,
            sketch::SOURCES,
            coxd::raw::Source {
                from: &from,
                title: &slot.name,
                text: &text,
            },
            coxd::raw::Kept {
                day: &day,
                chat: &slot.name,
                chat_id: number.as_deref(),
                project,
            },
        )
    }

    /// Makes a project with nothing set yet (see [`Self::untitled`]) for
    /// what the sketch `key`, in `folder`, tried, what it made kept apart
    /// (see [`Self::keep_apart`]), and opens a chat in the project; the
    /// sketch ends, its chats going on where they were opened.
    fn open_as_project(&mut self, key: u64, folder: &Path) {
        let project = self.untitled();
        let name = project
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kept = std::fs::create_dir_all(&project)
            .map_err(|error| format!("Cox cannot make {}: {error}", project.display()))
            .and_then(|()| {
                self.keep_apart(key, folder).inspect_err(|_| {
                    let _ = std::fs::remove_dir_all(&project);
                })
            });
        let ended = kept.map(|kept| {
            let note = format!(
                "The sketch was opened as the project {name}, to make there what it tried; \
                 what it made is not the project's.{}",
                kept.where_kept()
            );
            self.with_sketch_kept(
                key,
                &project.join(coxd::raw::PROJECT_RAW),
                Some(&name),
                &note,
            )
            .0
        });
        if self.end_sketch(key, folder, ended) {
            self.open(Some(project));
        }
    }

    /// Keeps apart what the sketch `key` from nothing, in `folder`, made,
    /// as a discarded sketch's is kept (see [`sketch::discard`]), taking
    /// its folder away, for a project made for what it tried to start
    /// with none of it.
    pub(super) fn keep_apart(&self, key: u64, folder: &Path) -> Result<sketch::Discarded, String> {
        let title = self
            .slot(key)
            .map(|slot| slot.name.clone())
            .unwrap_or_default();
        let sketches = storage::path(storage::Kept::Sketches)
            .ok_or("Cox cannot tell where to keep what the sketch made")?;
        sketch::discard(Sketch::Empty, folder, &title, &sketches, true)
    }
}

/// What the sketch of `entries` showed the human, as pictures and
/// recordings, copied from `kept`, where Cox keeps them, into
/// [`sketch::MEDIA`] in `sources`, the sources of the sketches, as its
/// source tells them: a section of links to them, by their paths from the
/// source, or nothing when it showed none.
fn showed_in_sketch(entries: &[Entry], kept: &Path, sources: &Path) -> String {
    let into = sources.join(sketch::MEDIA);
    let mut showed = String::new();
    for entry in entries {
        let Entry::Media { file, caption, .. } = entry else {
            continue;
        };
        let copied = std::fs::create_dir_all(&into)
            .and_then(|()| std::fs::copy(kept.join(file), into.join(file)));
        if let Err(error) = copied {
            tracing::warn!("{file}, which the sketch showed, is not kept with it: {error}");
            continue;
        }
        let caption = caption.as_deref().unwrap_or(file);
        let _ = write!(showed, "![{caption}]({}/{file})\n\n", sketch::MEDIA);
    }
    if showed.is_empty() {
        showed
    } else {
        format!(
            "## What it showed\n\nThe pictures and recordings it showed the human, in `{}` \
             beside this source, as they are to be made again; not its code.\n\n{showed}",
            sketch::MEDIA
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coxd::testing::TempDir;

    #[test]
    fn a_sketch_keeps_the_pictures_it_showed_beside_its_source() {
        let scratch = TempDir::new();
        let media = scratch.join("media-kept");
        std::fs::create_dir_all(&media).unwrap();
        let file = "shown.png";
        std::fs::write(media.join(file), b"png").unwrap();
        let sources = scratch.join("sources");
        let shown = |file: &str| Entry::Media {
            file: file.to_owned(),
            kind: cox_core::entry::MediaKind::Image,
            from: "/tmp/list.png".to_owned(),
            caption: Some("The list".to_owned()),
            size: None,
        };
        let entries = [
            Entry::Human("Make the list".to_owned()),
            shown(file),
            shown("gone.png"),
        ];

        let told = showed_in_sketch(&entries, &media, &sources);
        assert!(told.starts_with("## What it showed\n\n"), "{told}");
        assert!(
            told.ends_with(&format!("![The list](media/{file})\n\n")),
            "{told}"
        );
        assert!(!told.contains("gone.png"), "{told}");
        assert_eq!(
            std::fs::read(sources.join("media").join(file)).unwrap(),
            b"png"
        );
        assert_eq!(showed_in_sketch(&entries[..1], &media, &sources), "");
    }
}
