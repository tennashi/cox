//! The maps of a project as generated from its code (see
//! [`cox_editor::map::generate`]): for a map's vocabulary, a file of the
//! project's `maps`, coxd indexes the code of each language of each of
//! the project's repositories, as the commit each has checked out has
//! it, keeps each index by that commit, and draws the map from the
//! vocabulary and the indexes whenever a page asks, as the Map page of
//! the window's stage does, telling how each index stands.
//!
//! An index is made once a page asks for a map whose code has no index
//! yet, as when the repository's head moved to a commit not indexed yet,
//! and once only, however often the page asks meanwhile; the map is drawn
//! from the index of an earlier commit until it is made. Of a checkout's
//! code, an indexer makes one index at a time: the head's, which stops
//! the one of a commit it moved on from. An indexer that fails is not run
//! again on the same commit until the page asks again after it failed. As
//! coxd quits, the indexers stop (see [`crate::process::stop_all`]).
//!
//! coxd runs the indexers, not the window: one can take minutes, as
//! rust-analyzer does on Cox's code, and coxd goes on while the window
//! closes or restarts; and the phone, which coxd serves, draws the same
//! map.
//!
//! - [`indexers`] is the indexers Cox knows, which language each reads,
//!   and how each is got.
//! - [`index`] makes one index, of one commit.
//! - [`cache`] keeps the indexes.

pub mod cache;
pub mod index;
pub mod indexers;
mod jobs;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use cox_editor::map::generated::Generated;
use cox_editor::map::vocabulary::Vocabulary;
use cox_editor::map::{self as maps, Named};
use cox_editor::scip::Index;
use serde::Serialize;

use self::cache::{Cache, Key};
use self::index::Code;
use self::indexers::{Found, Indexer};
use self::jobs::{Jobs, Slot, Standing};
use crate::process::Stop;

/// How many commits' indexes are kept, those used last, and how many
/// bytes of them at most: Cox's index is some 9 MB.
const KEEP: usize = 24;
const MOST: u64 = 1 << 30;

/// How far back the map looks for an earlier commit's index to draw from
/// while the head's is made.
const EARLIER: usize = 50;

/// How many maps, as drawn last, are kept to tell again: drawing Cox's
/// reads 30 MB of indexes, and takes a second or two.
const DRAWN: usize = 4;

/// The Map page's files, by their paths within it, built into Cox: the
/// Map plugin, which the window's stage serves at `cox://map/` and the
/// phone at `/map/`, each beside the JSON the page reads.
pub const PAGE: &[(&str, &[u8])] = &[
    (
        "plugin.json",
        include_bytes!("../../cox/plugins/map/plugin.json"),
    ),
    (
        "index.html",
        include_bytes!("../../cox/plugins/map/index.html"),
    ),
    ("map.css", include_bytes!("../../cox/plugins/map/map.css")),
    ("app.js", include_bytes!("../../cox/plugins/map/app.js")),
    (
        "canvas.js",
        include_bytes!("../../cox/plugins/map/canvas.js"),
    ),
    (
        "detail.js",
        include_bytes!("../../cox/plugins/map/detail.js"),
    ),
    ("drift.js", include_bytes!("../../cox/plugins/map/drift.js")),
    ("icons.js", include_bytes!("../../cox/plugins/map/icons.js")),
];

/// What a page is told of one of the project's maps: all it draws the
/// map from, with nothing to ask back.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Told {
    /// The project's name: its folder's.
    pub project: String,
    /// The project's folder.
    pub root: PathBuf,
    /// The project's maps, for the page to go to another.
    pub maps: Vec<Named>,
    /// The map's name, unless the project has no map of the name asked,
    /// or none at all.
    pub name: Option<String>,
    /// Its vocabulary's file.
    pub file: Option<PathBuf>,
    pub state: State,
    /// Why there is no map: each problem of its vocabulary, with where it
    /// is, or that the project has no map of the name asked.
    pub problems: Vec<String>,
    /// The index of each language of each of the project's repositories.
    pub indexes: Vec<Indexed>,
    /// The map, from its vocabulary and the indexes there are, with no
    /// drift unless every language's code is in it; none when the
    /// vocabulary does not read.
    pub map: Option<Generated>,
    /// What changed in the code of the map's nodes, by id, as the
    /// Changes tab shows each repository's changes: of a box, in the
    /// files it owns and those of the boxes inside it, all the way down;
    /// of a type, in its file. A node nothing changed in is left out.
    pub changed: BTreeMap<String, Changed>,
}

/// What changed in the code of a node: in how many files, and how many
/// lines they gained and lost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Changed {
    pub files: usize,
    pub added: usize,
    pub removed: usize,
}

/// How a map stands, as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// The project has no map, or none of the name asked.
    None,
    /// Its vocabulary does not read.
    Invalid,
    /// An index of the code checked out is being made: meanwhile, the
    /// map is drawn from an earlier commit's index, where there is one.
    Indexing,
    /// The map is drawn from the indexes of the code checked out, of
    /// every language.
    Ready,
    /// The code of a language, or of a repository, is not in the map
    /// (see [`Told::indexes`]).
    Incomplete,
}

/// The index of one language of one of the project's repositories.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Indexed {
    /// The repository, by its folder from the project's, as
    /// `directories/cox`, or empty for the project's folder itself.
    pub directory: String,
    /// The language, and its indexer, unless Cox knows none for the
    /// code.
    pub language: Option<String>,
    pub indexer: Option<String>,
    /// The commit checked out, which is indexed, if there is one.
    pub commit: Option<String>,
    /// The commit whose index the map is drawn from, if any: `commit`'s,
    /// or an earlier one's while `commit`'s is made.
    pub drawn: Option<String>,
    /// What the index drawn from may lack, if it was made without
    /// something.
    pub note: Option<String>,
    #[serde(flatten)]
    pub state: IndexState,
}

/// How an index stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum IndexState {
    /// Being made, since then, in seconds since the epoch.
    Indexing {
        since: u64,
    },
    Ready,
    /// It could not be made, for this reason.
    Failed {
        reason: String,
    },
    /// What its indexer needs is not there: what is missing, and how to
    /// get it.
    Missing {
        how: String,
    },
    /// Cox knows no indexer for the repository's code: it knows those of
    /// these languages.
    Unknown {
        known: Vec<String>,
    },
    /// The repository has no commit to index, or is none.
    Uncommitted {
        why: String,
    },
}

/// The maps coxd draws, and the indexes it makes for them, shared by the
/// threads that ask for maps and those that make indexes.
#[derive(Clone)]
pub struct Maps(Arc<Inner>);

struct Inner {
    cache: Cache,
    /// Where an index is made, in a folder of its own: in this run's own
    /// folder, made the first time an index is (see [`crate::scratch`]),
    /// unless given.
    scratch: Option<PathBuf>,
    indexers: Vec<Indexer>,
    /// Where Cox keeps what lasts, for the threads that make the indexes
    /// (see [`crate::storage::keep_on_this_thread`]).
    data: Option<PathBuf>,
    /// The indexes being made, and those that failed.
    jobs: Mutex<Jobs>,
    /// The maps drawn last, the last first.
    drawn: Mutex<Vec<(Drawing, Generated)>>,
}

/// What a map is drawn from: its vocabulary's text, and the indexes
/// kept, each with the folder its paths are from, from the project's, as
/// `directories/cox/`, or empty; and whether some code is not in them,
/// which leaves out the drift.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Drawing {
    vocabulary: String,
    indexes: Vec<(String, Key)>,
    partly: bool,
}

impl Maps {
    /// The maps of this Cox: their indexes kept where Cox keeps what can
    /// be made again, made in its run's own folder, by the indexers Cox
    /// knows.
    ///
    /// # Errors
    ///
    /// When Cox has nowhere to keep them.
    pub fn of_this_cox() -> anyhow::Result<Self> {
        let cache = crate::storage::path(crate::storage::Kept::Indexes)
            .ok_or_else(|| anyhow::anyhow!("Cox has nowhere to keep the indexes"))?;
        Ok(Self::new(cache, None, indexers::known()))
    }

    /// The maps whose indexes are kept in `cache`, made in `scratch`, or
    /// in this run's own folder, by `indexers`.
    #[must_use]
    pub fn new(cache: PathBuf, scratch: Option<PathBuf>, indexers: Vec<Indexer>) -> Self {
        Self(Arc::new(Inner {
            cache: Cache::new(cache),
            scratch,
            indexers,
            data: crate::storage::kept_on_this_thread(),
            jobs: Mutex::default(),
            drawn: Mutex::default(),
        }))
    }

    /// The map of the project in `root` named `name`, or its first unless
    /// named, as it can be drawn now, starting to make each index of the
    /// code checked out that is not made yet, and again one that failed,
    /// when `again`.
    #[must_use]
    pub fn told(&self, root: &Path, name: Option<&str>, again: bool) -> Told {
        crate::storage::keep_on_this_thread(self.0.data.clone());
        let maps = maps::list(root);
        let named = match name {
            None => maps.first().cloned(),
            Some(name) => maps.iter().find(|named| named.name == name).cloned(),
        };
        let mut told = Told {
            project: root.file_name().map_or_else(
                || root.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            root: root.to_owned(),
            maps,
            name: named.as_ref().map(|named| named.name.clone()),
            file: named.as_ref().map(|named| root.join(&named.file)),
            state: State::None,
            problems: Vec::new(),
            indexes: Vec::new(),
            map: None,
            changed: BTreeMap::new(),
        };
        let Some(named) = named else {
            if let Some(name) = name {
                told.problems
                    .push(format!("the project has no map `{name}`"));
            }
            return told;
        };
        let read = std::fs::read_to_string(root.join(&named.file))
            .map_err(|error| vec![format!("{}: {error}", named.file)])
            .and_then(|text| match Vocabulary::parse(&text) {
                Ok(vocabulary) => Ok((text, vocabulary)),
                Err(error) => Err(error.problems),
            });
        let (text, vocabulary) = match read {
            Ok(read) => read,
            Err(problems) => {
                told.state = State::Invalid;
                told.problems = problems;
                return told;
            }
        };
        let mut indexes = Vec::new();
        for (_, folder) in crate::repositories::of(root) {
            self.index_all(root, &folder, again, &mut told.indexes, &mut indexes);
        }
        let drawing = Drawing {
            vocabulary: text,
            indexes,
            partly: told.indexes.iter().any(|indexed| indexed.drawn.is_none()),
        };
        let (map, lost) = self.draw(drawing, &vocabulary);
        for indexed in &mut told.indexes {
            let of = |key: &Key| {
                indexed.drawn.as_ref() == Some(&key.commit)
                    && indexed.indexer.as_ref() == Some(&key.indexer)
            };
            if lost.iter().any(of) {
                indexed.drawn = None;
                indexed.note = None;
                indexed.state = IndexState::Failed {
                    reason: "its index went as it was read: it is made again as the map is \
                             asked for next"
                        .to_owned(),
                };
            }
        }
        told.state = State::of(&told.indexes);
        told.changed = changed(&map, &changed_files(root));
        told.map = Some(map);
        told
    }

    /// The map `drawing` says, of `vocabulary`: as it was drawn last, if
    /// it was drawn of the same; and what the indexes that could not be
    /// read, as one taken away meanwhile, are kept by.
    fn draw(&self, drawing: Drawing, vocabulary: &Vocabulary) -> (Generated, Vec<Key>) {
        if let Some(map) = self.drawn_as(&drawing) {
            return (map, Vec::new());
        }
        let mut documents = Vec::new();
        let mut lost = Vec::new();
        for (prefix, key) in &drawing.indexes {
            let Some(index) = self.0.cache.read(key) else {
                lost.push(key.clone());
                continue;
            };
            documents.extend(index.documents.into_iter().map(|mut document| {
                document.relative_path.insert_str(0, prefix);
                document
            }));
        }
        // The text of each file is in its index.
        let mut map =
            cox_editor::map::generate::generate(&Index { documents }, vocabulary, &BTreeMap::new());
        if drawing.partly || !lost.is_empty() {
            // Drift against code that is not in the map is no drift.
            map.drift.clear();
        }
        if lost.is_empty() {
            let mut drawn = self.0.drawn.lock().unwrap_or_else(PoisonError::into_inner);
            drawn.insert(0, (drawing, map.clone()));
            drawn.truncate(DRAWN);
        }
        (map, lost)
    }

    /// The map drawn last of `drawing`, if it is kept.
    fn drawn_as(&self, drawing: &Drawing) -> Option<Generated> {
        let mut drawn = self.0.drawn.lock().unwrap_or_else(PoisonError::into_inner);
        let at = drawn.iter().position(|(of, _)| of == drawing)?;
        let found = drawn.remove(at);
        let map = found.1.clone();
        drawn.insert(0, found);
        Some(map)
    }

    /// Tells into `indexes` how the index of each language of the
    /// repository checked out in `folder` stands, for the project in
    /// `root`, and adds to `drawn` those the map is drawn from, each with
    /// the folder its paths are from.
    fn index_all(
        &self,
        root: &Path,
        folder: &Path,
        again: bool,
        indexes: &mut Vec<Indexed>,
        drawn: &mut Vec<(String, Key)>,
    ) {
        let directory = folder
            .strip_prefix(root)
            .map_or_default(|directory| directory.to_string_lossy().into_owned());
        let prefix = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let blank = Indexed {
            directory,
            language: None,
            indexer: None,
            commit: None,
            drawn: None,
            note: None,
            state: IndexState::Ready,
        };
        let code = match Code::checked_out(folder) {
            Ok(code) => code,
            Err(why) => {
                indexes.push(Indexed {
                    state: IndexState::Uncommitted { why },
                    ..blank
                });
                return;
            }
        };
        let blank = Indexed {
            commit: Some(code.commit.clone()),
            ..blank
        };
        let top = match code.top() {
            Ok(top) => top,
            Err(reason) => {
                indexes.push(Indexed {
                    state: IndexState::Failed { reason },
                    ..blank
                });
                return;
            }
        };
        let found = indexers::of(&self.0.indexers, &top);
        if found.is_empty() {
            let mut known: Vec<String> = self
                .0
                .indexers
                .iter()
                .map(|indexer| indexer.language.clone())
                .collect();
            known.dedup();
            indexes.push(Indexed {
                state: IndexState::Unknown { known },
                ..blank
            });
            return;
        }
        for indexer in found {
            let blank = Indexed {
                language: Some(indexer.language.clone()),
                indexer: Some(indexer.name.clone()),
                ..blank.clone()
            };
            let (told, kept) = self.index(root, &code, indexer, again, blank);
            indexes.push(told);
            if let Some(kept) = kept {
                drawn.push((prefix.clone(), kept));
            }
        }
    }

    /// How the index of `code` that `indexer` writes stands, for the
    /// project in `root`, told over `blank`, with what the index the map
    /// is drawn from is kept by, if there is one: starting to make it
    /// when it is not made yet.
    fn index(
        &self,
        root: &Path,
        code: &Code,
        indexer: &Indexer,
        again: bool,
        blank: Indexed,
    ) -> (Indexed, Option<Key>) {
        let found = match indexers::find(indexer, root, std::env::var_os("PATH").as_deref()) {
            Ok(found) => found,
            Err(how) => {
                let missing = Indexed {
                    state: IndexState::Missing { how },
                    ..blank
                };
                return (missing, None);
            }
        };
        let key = Key {
            commit: code.commit.clone(),
            within: code.within.clone(),
            indexer: indexer.name.clone(),
            version: found.version.clone(),
            without: indexer
                .borrows
                .as_ref()
                .is_some_and(|(borrows, _)| !code.checkout.join(borrows).is_dir()),
        };
        let state = self.made(&key, code, indexer, &found, again);
        let kept = if state == IndexState::Ready {
            Some(key)
        } else {
            self.earlier(&key, code)
        };
        if let Some(kept) = &kept {
            self.0.cache.used(kept);
        }
        let note = kept
            .as_ref()
            .filter(|kept| kept.without)
            .and(indexer.borrows.as_ref())
            .map(|(_, lacks)| lacks.clone());
        let told = Indexed {
            drawn: kept.as_ref().map(|kept| kept.commit.clone()),
            note,
            state,
            ..blank
        };
        (told, kept)
    }

    /// How the index of `key` stands: starting to make it, with
    /// `indexer` as `found`, unless it is kept, or being made, or failed
    /// and is not asked `again`, and stopping the one made of the same
    /// code meanwhile, of an older commit.
    fn made(
        &self,
        key: &Key,
        code: &Code,
        indexer: &Indexer,
        found: &Found,
        again: bool,
    ) -> IndexState {
        if self.0.cache.has(key) {
            return IndexState::Ready;
        }
        let mut jobs = self.jobs();
        match jobs.standing(key) {
            Some(Standing::Indexing { since, .. }) => {
                return IndexState::Indexing { since: *since };
            }
            Some(Standing::Failed { reason }) if !again => {
                return IndexState::Failed {
                    reason: reason.clone(),
                };
            }
            _ => {}
        }
        // Made between the look above and the lock: a job keeps the index
        // before it lets go of its key.
        if self.0.cache.has(key) {
            return IndexState::Ready;
        }
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let slot = (
            code.checkout.clone(),
            code.within.clone(),
            indexer.name.clone(),
        );
        let (job, stop) = jobs.start(slot.clone(), key.clone(), since);
        drop(jobs);
        let ending = Ending {
            maps: self.clone(),
            slot,
            key: key.clone(),
            job,
            ended: None,
        };
        let (code, indexer, found) = (code.clone(), indexer.clone(), found.clone());
        let started = std::thread::Builder::new()
            .name(format!("index {}", indexer.name))
            .spawn(move || {
                let made = ending
                    .maps
                    .make(&ending.key, &code, &indexer, &found, job, &stop);
                ending.end(made);
            });
        match started {
            Ok(_) => IndexState::Indexing { since },
            // The job's `Ending` went with the closure: it is kept failed.
            Err(error) => IndexState::Failed {
                reason: format!("no thread to index on: {error}"),
            },
        }
    }

    /// Makes the index of `key`, of `code`, with `indexer` as `found`, as
    /// the job `job`, until `stop` stops it, and keeps it.
    ///
    /// # Errors
    ///
    /// Why it was not made.
    fn make(
        &self,
        key: &Key,
        code: &Code,
        indexer: &Indexer,
        found: &Found,
        job: u64,
        stop: &Stop,
    ) -> Result<(), String> {
        crate::storage::keep_on_this_thread(self.0.data.clone());
        let command = found
            .got()
            .map_err(|error| format!("{} cannot be got: {error:#}", indexer.name))?;
        let scratch = match &self.0.scratch {
            Some(scratch) => scratch.clone(),
            None => crate::scratch::folder()
                .map_err(|error| format!("no folder to index in: {error}"))?,
        };
        let work = scratch.join(format!("index-{job}"));
        let index = index::make(code, indexer, key.without, &command, &work, stop)?;
        if stop.stopped() {
            return Err(format!("{} was stopped", indexer.name));
        }
        self.0
            .cache
            .write(key, &index)
            .map_err(|error| format!("the index cannot be kept: {error}"))?;
        self.0.cache.prune(KEEP, MOST);
        Ok(())
    }

    /// What the index of the last commit before `key`'s is kept by,
    /// whose index of the same code, by the same indexer, is kept, if any.
    fn earlier(&self, key: &Key, code: &Code) -> Option<Key> {
        code.earlier(EARLIER)
            .into_iter()
            .map(|commit| Key {
                commit,
                ..key.clone()
            })
            .find(|earlier| self.0.cache.has(earlier))
    }

    fn jobs(&self) -> MutexGuard<'_, Jobs> {
        self.0.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A job that makes an index, which says how it ended as it goes, even
/// when it goes as its thread panics, or never starts.
struct Ending {
    maps: Maps,
    slot: Slot,
    key: Key,
    job: u64,
    /// How it ended, once it did.
    ended: Option<Result<(), String>>,
}

impl Ending {
    /// It ended so.
    fn end(mut self, ended: Result<(), String>) {
        self.ended = Some(ended);
    }
}

impl Drop for Ending {
    fn drop(&mut self) {
        let ended = self
            .ended
            .take()
            .unwrap_or_else(|| Err("the indexing broke off".to_owned()));
        if let Err(reason) = &ended {
            tracing::warn!(commit = %self.key.commit, indexer = %self.key.indexer, "{reason}");
        }
        self.maps.jobs().end(&self.slot, &self.key, self.job, ended);
    }
}

/// The files that changed in the project in `root`, by their paths from
/// its folder, with the lines each gained and lost, as the Changes tab
/// shows each of its repositories' changes (see [`cox_git::changes`]).
fn changed_files(root: &Path) -> BTreeMap<String, (usize, usize)> {
    let roots = [Some(root.to_owned()), std::fs::canonicalize(root).ok()];
    let mut files = BTreeMap::new();
    for (_, folder) in crate::repositories::of(root) {
        let Ok(diff) = cox_git::changes::read(&folder) else {
            continue;
        };
        for (path, lines) in cox_git::changes::changed_files(&diff) {
            let relative = roots
                .iter()
                .flatten()
                .find_map(|root| path.strip_prefix(root).ok());
            if let Some(relative) = relative {
                files.insert(relative.to_string_lossy().into_owned(), lines);
            }
        }
    }
    files
}

/// What changed in the code of each node of `map`, of `files`, the
/// files of the project that changed (see [`Told::changed`]).
fn changed(map: &Generated, files: &BTreeMap<String, (usize, usize)>) -> BTreeMap<String, Changed> {
    // The level each node is a part of, by the node that opens it.
    let mut parents: BTreeMap<&str, &str> = BTreeMap::new();
    for (opens, level) in &map.levels {
        for part in level.parts.keys() {
            parents.entry(part.as_str()).or_insert(opens.as_str());
        }
    }
    let mut of: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for file in files.keys() {
        let mut node = map.files.get(file).map(String::as_str);
        // A box, and each box it is inside of, once each, however the
        // levels nest.
        let mut seen = Vec::new();
        while let Some(at) = node.filter(|at| !seen.contains(at)) {
            seen.push(at);
            of.entry(at).or_default().push(file);
            node = parents.get(at).copied();
        }
    }
    for ty in map.types.values() {
        if files.contains_key(&ty.file) {
            of.entry(ty.id.as_str()).or_default().push(&ty.file);
        }
    }
    of.into_iter()
        .map(|(node, changed)| {
            let lines = changed.iter().filter_map(|file| files.get(*file));
            let told = Changed {
                files: changed.len(),
                added: lines.clone().map(|(added, _)| added).sum(),
                removed: lines.map(|(_, removed)| removed).sum(),
            };
            (node.to_owned(), told)
        })
        .collect()
}

impl State {
    /// How a map drawn from `indexes` stands.
    fn of(indexes: &[Indexed]) -> Self {
        if indexes
            .iter()
            .any(|indexed| matches!(indexed.state, IndexState::Indexing { .. }))
        {
            Self::Indexing
        } else if indexes
            .iter()
            .all(|indexed| indexed.state == IndexState::Ready)
        {
            Self::Ready
        } else {
            Self::Incomplete
        }
    }
}

impl Told {
    /// It as JSON, for a page.
    #[must_use]
    pub fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|error| {
            serde_json::json!({ "state": "invalid", "problems": [error.to_string()] }).to_string()
        })
    }
}

#[cfg(test)]
mod tests;
