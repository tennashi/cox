//! Where Cox keeps the projects, the chats in them, what each chat
//! showed, and the session its agent had, to go on with: a `SQLite`
//! database, written as the chats go, so that they last past their
//! closing, and past Cox quitting, however it quits.
//!
//! A chat's entries are kept one row each, written as they arrive. An
//! entry that is still changing, such as a reply still arriving or a tool
//! call still running, is written again each time it changes; once the
//! chat goes on past it, its row stays as it is.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, bail};
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::entry::Entry;
use crate::tabs::Tabs;

/// The version of the tables below, as `SQLite`'s `user_version` holds it.
const VERSION: i64 = 2;

const TABLES: &str = "
CREATE TABLE projects (
    id INTEGER PRIMARY KEY,
    folder TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL
);
CREATE TABLE chats (
    id INTEGER PRIMARY KEY,
    -- NULL for a chat in no project.
    project INTEGER REFERENCES projects (id),
    name TEXT NOT NULL,
    -- When the chat was opened, in milliseconds since the epoch.
    created INTEGER NOT NULL,
    -- Its stage's tabs, as JSON, once it has a stage.
    tabs TEXT,
    -- The id its agent gave its session, once the agent has opened one.
    session TEXT
);
CREATE TABLE entries (
    chat INTEGER NOT NULL REFERENCES chats (id),
    -- Its place in the conversation, from 0.
    number INTEGER NOT NULL,
    -- When it was first written, in milliseconds since the epoch.
    at INTEGER NOT NULL,
    -- The entry, as JSON.
    entry TEXT NOT NULL,
    PRIMARY KEY (chat, number)
) WITHOUT ROWID;
";

/// What makes the tables of each earlier version into those of the next:
/// the first of version 1, and so on.
const UPGRADES: &[&str] = &["ALTER TABLE chats ADD COLUMN session TEXT;"];

/// How long a write waits for another Cox writing the same database.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A project kept: a folder the human opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProjectId(i64);

/// A chat kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChatId(i64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptProject {
    pub id: ProjectId,
    pub folder: PathBuf,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptChat {
    pub id: ChatId,
    /// Its project, or `None` for no project.
    pub project: Option<ProjectId>,
    pub name: String,
}

pub struct Store {
    connection: Connection,
}

impl Store {
    /// Opens the database at `path`, making it, and the directories it is
    /// in, if they are not there. Only the human reads it: it holds what
    /// they and their agents said.
    ///
    /// # Errors
    ///
    /// When it cannot be made or opened, or was made by a newer Cox.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        make_private(path).with_context(|| format!("cannot make {}", path.display()))?;
        let connection =
            Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        Self::prepare(connection).with_context(|| format!("cannot use {}", path.display()))
    }

    /// A database in memory, gone when it is dropped.
    ///
    /// # Errors
    ///
    /// When `SQLite` cannot make one.
    pub fn in_memory() -> anyhow::Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(connection: Connection) -> anyhow::Result<Self> {
        connection.busy_timeout(BUSY_TIMEOUT)?;
        // Writing as a chat goes waits on no sync to the disk; a write
        // lost to a crash of the system is at most the last few.
        connection.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                connection.execute_batch(TABLES)?;
                connection.pragma_update(None, "user_version", VERSION)?;
            }
            VERSION => {}
            1..VERSION => {
                // From the version kept on, in one go, so that a Cox
                // stopped halfway leaves the tables as they were.
                let from = usize::try_from(version - 1)?;
                let upgrades = UPGRADES[from..].concat();
                connection.execute_batch(&format!(
                    "BEGIN; {upgrades} PRAGMA user_version = {VERSION}; COMMIT;"
                ))?;
            }
            _ => bail!("the chats were kept by a newer Cox (version {version})"),
        }
        Ok(Self { connection })
    }

    /// The projects, in the order they were first opened.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn projects(&self) -> anyhow::Result<Vec<KeptProject>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, folder, name FROM projects ORDER BY id")?;
        let projects = statement.query_map([], |row| {
            Ok(KeptProject {
                id: ProjectId(row.get(0)?),
                folder: PathBuf::from(row.get::<_, String>(1)?),
                name: row.get(2)?,
            })
        })?;
        Ok(projects.collect::<Result<_, _>>()?)
    }

    /// The chats, in the order they were opened.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn chats(&self) -> anyhow::Result<Vec<KeptChat>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, project, name FROM chats ORDER BY id")?;
        let chats = statement.query_map([], |row| {
            Ok(KeptChat {
                id: ChatId(row.get(0)?),
                project: row.get::<_, Option<i64>>(1)?.map(ProjectId),
                name: row.get(2)?,
            })
        })?;
        Ok(chats.collect::<Result<_, _>>()?)
    }

    /// The project of `folder`, kept as `name` first if it is not kept.
    ///
    /// # Errors
    ///
    /// When the database cannot be read or written.
    pub fn project(&self, folder: &Path, name: &str) -> anyhow::Result<ProjectId> {
        // A folder whose name is not UTF-8 is kept as near as it can be.
        let folder = folder.to_string_lossy();
        let kept = self
            .connection
            .query_row(
                "SELECT id FROM projects WHERE folder = ?1",
                [&folder],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = kept {
            return Ok(ProjectId(id));
        }
        self.connection.execute(
            "INSERT INTO projects (folder, name) VALUES (?1, ?2)",
            params![folder, name],
        )?;
        Ok(ProjectId(self.connection.last_insert_rowid()))
    }

    /// Keeps a new chat named `name` in `project`, or in no project.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn add_chat(&self, project: Option<ProjectId>, name: &str) -> anyhow::Result<ChatId> {
        self.connection.execute(
            "INSERT INTO chats (project, name, created) VALUES (?1, ?2, ?3)",
            params![project.map(|project| project.0), name, now()],
        )?;
        Ok(ChatId(self.connection.last_insert_rowid()))
    }

    /// What `chat` showed, in order.
    ///
    /// # Errors
    ///
    /// When the database cannot be read, or holds an entry this Cox cannot.
    pub fn entries(&self, chat: ChatId) -> anyhow::Result<Vec<Entry>> {
        let mut statement = self
            .connection
            .prepare("SELECT entry FROM entries WHERE chat = ?1 ORDER BY number")?;
        let entries = statement.query_map([chat.0], |row| row.get::<_, String>(0))?;
        entries
            .map(|entry| Ok(serde_json::from_str(&entry?)?))
            .collect()
    }

    /// Keeps `entry` as the `number`th of `chat`, from 0, in place of what
    /// was kept there.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_entry(&self, chat: ChatId, number: usize, entry: &Entry) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO entries (chat, number, at, entry) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (chat, number) DO UPDATE SET entry = excluded.entry",
            params![
                chat.0,
                i64::try_from(number)?,
                now(),
                serde_json::to_string(entry)?
            ],
        )?;
        Ok(())
    }

    /// The tabs of `chat`'s stage, if it had one.
    ///
    /// # Errors
    ///
    /// When the database cannot be read, or holds tabs this Cox cannot.
    pub fn tabs(&self, chat: ChatId) -> anyhow::Result<Option<Tabs>> {
        let tabs: Option<String> =
            self.connection
                .query_row("SELECT tabs FROM chats WHERE id = ?1", [chat.0], |row| {
                    row.get(0)
                })?;
        Ok(tabs.map(|tabs| serde_json::from_str(&tabs)).transpose()?)
    }

    /// Keeps `tabs` as those of `chat`'s stage.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_tabs(&self, chat: ChatId, tabs: &Tabs) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET tabs = ?2 WHERE id = ?1",
            params![chat.0, serde_json::to_string(tabs)?],
        )?;
        Ok(())
    }

    /// The id the agent of `chat` gave its session, if it opened one.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn session(&self, chat: ChatId) -> anyhow::Result<Option<String>> {
        Ok(self.connection.query_row(
            "SELECT session FROM chats WHERE id = ?1",
            [chat.0],
            |row| row.get(0),
        )?)
    }

    /// Keeps `session` as the id of the session of `chat`'s agent.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_session(&self, chat: ChatId, session: &str) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET session = ?2 WHERE id = ?1",
            params![chat.0, session],
        )?;
        Ok(())
    }
}

/// Where one chat is kept: what its conversation and stage write to as
/// they change.
#[derive(Clone)]
pub struct Record {
    store: Rc<Store>,
    chat: ChatId,
}

impl Record {
    #[must_use]
    pub fn new(store: Rc<Store>, chat: ChatId) -> Self {
        Self { store, chat }
    }

    /// Keeps `entry` as the `number`th of the chat.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn entry(&self, number: usize, entry: &Entry) -> anyhow::Result<()> {
        self.store.keep_entry(self.chat, number, entry)
    }

    /// Keeps `tabs` as those of the chat's stage.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn tabs(&self, tabs: &Tabs) -> anyhow::Result<()> {
        self.store.keep_tabs(self.chat, tabs)
    }

    /// Keeps `session` as the id of the session of the chat's agent.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn session(&self, session: &str) -> anyhow::Result<()> {
        self.store.keep_session(self.chat, session)
    }
}

/// The time now, in milliseconds since the epoch.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or_default()
}

/// Makes the file at `path` for the human alone, and the directories it is
/// in, if they are not there. `SQLite` makes the files beside it the same.
#[cfg(unix)]
fn make_private(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};

    if let Some(directory) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_private(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(directory) => std::fs::create_dir_all(directory),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::Line;
    use crate::entry::{Content, ToolCallResult};
    use crate::tabs::{Lines, Tab};

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("cox-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn keeps_projects_and_chats_in_the_order_they_were_opened() {
        let store = Store::in_memory().unwrap();
        let cox = store.project(Path::new("/src/cox"), "cox").unwrap();
        let notes = store.project(Path::new("/notes"), "notes").unwrap();
        assert_eq!(store.project(Path::new("/src/cox"), "again").unwrap(), cox);

        let first = store.add_chat(Some(notes), "Chat 1").unwrap();
        let second = store.add_chat(None, "Chat 1").unwrap();
        let third = store.add_chat(Some(cox), "Chat 1").unwrap();

        assert_eq!(
            store.projects().unwrap(),
            [
                KeptProject {
                    id: cox,
                    folder: "/src/cox".into(),
                    name: "cox".to_owned(),
                },
                KeptProject {
                    id: notes,
                    folder: "/notes".into(),
                    name: "notes".to_owned(),
                },
            ]
        );
        let chats: Vec<_> = store
            .chats()
            .unwrap()
            .into_iter()
            .map(|chat| (chat.id, chat.project))
            .collect();
        assert_eq!(
            chats,
            [(first, Some(notes)), (second, None), (third, Some(cox))]
        );
    }

    #[test]
    fn keeps_each_entry_as_it_last_was() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let other = store.add_chat(None, "Chat 2").unwrap();
        let call = |result| Entry::ToolCall {
            id: "call-1".to_owned(),
            title: "Edit x.txt".to_owned(),
            content: vec![Content::Diff {
                path: "/work/x.txt".to_owned(),
                lines: vec![Line::Removed("one".to_owned()), Line::Added("1".to_owned())],
            }],
            result,
        };

        store
            .keep_entry(chat, 0, &Entry::Human("edit".to_owned()))
            .unwrap();
        store.keep_entry(chat, 1, &call(None)).unwrap();
        store
            .keep_entry(chat, 2, &Entry::Reply("Done".to_owned()))
            .unwrap();
        store
            .keep_entry(chat, 1, &call(Some(ToolCallResult::Completed)))
            .unwrap();
        store
            .keep_entry(other, 0, &Entry::Note("[EndTurn]".to_owned()))
            .unwrap();

        assert_eq!(
            store.entries(chat).unwrap(),
            [
                Entry::Human("edit".to_owned()),
                call(Some(ToolCallResult::Completed)),
                Entry::Reply("Done".to_owned()),
            ]
        );
    }

    #[test]
    fn keeps_the_tabs_of_a_chats_stage() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        assert_eq!(store.tabs(chat).unwrap(), None);

        let tabs = Tabs {
            tabs: vec![
                Tab::Page {
                    url: "https://example.com/".to_owned(),
                    title: "Example Domain".to_owned(),
                },
                Tab::Markdown {
                    path: "/work/plan.md".into(),
                },
                Tab::Terminal {
                    title: "cargo test".to_owned(),
                },
                Tab::File {
                    path: "/work/src/main.rs".into(),
                    lines: Some(Lines { first: 3, last: 9 }),
                },
                Tab::Files {
                    open: vec!["/work/src".into()],
                },
            ],
            shown: 1,
        };
        store.keep_tabs(chat, &tabs).unwrap();
        assert_eq!(store.tabs(chat).unwrap(), Some(tabs));
    }

    #[test]
    fn keeps_the_session_of_a_chats_agent() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        assert_eq!(store.session(chat).unwrap(), None);

        store.keep_session(chat, "session-1").unwrap();
        assert_eq!(store.session(chat).unwrap().as_deref(), Some("session-1"));
    }

    #[test]
    fn a_database_from_an_earlier_cox_is_upgraded() {
        let directory = scratch("earlier");
        let path = directory.join("chats.sqlite");
        std::fs::create_dir_all(&directory).unwrap();
        {
            // The tables as the first Cox to keep chats made them.
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE projects (
                        id INTEGER PRIMARY KEY,
                        folder TEXT NOT NULL UNIQUE,
                        name TEXT NOT NULL
                    );
                    CREATE TABLE chats (
                        id INTEGER PRIMARY KEY,
                        project INTEGER REFERENCES projects (id),
                        name TEXT NOT NULL,
                        created INTEGER NOT NULL,
                        tabs TEXT
                    );
                    CREATE TABLE entries (
                        chat INTEGER NOT NULL REFERENCES chats (id),
                        number INTEGER NOT NULL,
                        at INTEGER NOT NULL,
                        entry TEXT NOT NULL,
                        PRIMARY KEY (chat, number)
                    ) WITHOUT ROWID;
                    INSERT INTO chats (name, created) VALUES ('Chat 1', 0);
                    PRAGMA user_version = 1;",
                )
                .unwrap();
        }

        let store = Store::open(&path).unwrap();
        let chats = store.chats().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(store.session(chats[0].id).unwrap(), None);
        store.keep_session(chats[0].id, "session-1").unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.session(chats[0].id).unwrap().as_deref(),
            Some("session-1")
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn what_is_kept_lasts_past_closing_the_database() {
        let directory = scratch("lasts");
        let path = directory.join("kept").join("chats.sqlite");
        {
            let store = Store::open(&path).unwrap();
            let project = store.project(Path::new("/src/cox"), "cox").unwrap();
            let chat = store.add_chat(Some(project), "Chat 1").unwrap();
            store
                .keep_entry(chat, 0, &Entry::Human("hello".to_owned()))
                .unwrap();
        }

        let store = Store::open(&path).unwrap();
        let chats = store.chats().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].name, "Chat 1");
        assert_eq!(
            store.entries(chats[0].id).unwrap(),
            [Entry::Human("hello".to_owned())]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_database_from_a_newer_cox_is_refused() {
        let directory = scratch("newer");
        let path = directory.join("chats.sqlite");
        drop(Store::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", VERSION + 1)
            .unwrap();

        let error = Store::open(&path).err().expect("the database is refused");
        assert!(format!("{error:#}").contains("newer Cox"), "{error:#}");
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
