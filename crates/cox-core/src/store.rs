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
use crate::goal::Goal;
use crate::moved::Moved;
use crate::settings::Setting;
use crate::tabs::KeptTabs;

/// The version of the tables below, as `SQLite`'s `user_version` holds it.
const VERSION: i64 = 10;

const TABLES: &str = "
CREATE TABLE projects (
    id INTEGER PRIMARY KEY,
    folder TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL
);
CREATE TABLE chats (
    id INTEGER PRIMARY KEY,
    -- NULL for a chat of Cox's own, in no project.
    project INTEGER REFERENCES projects (id),
    name TEXT NOT NULL,
    -- When the chat was opened, in milliseconds since the epoch.
    created INTEGER NOT NULL,
    -- The id its agent gave its session, once the agent has opened one.
    session TEXT,
    -- Who named it, as `Named` numbers them.
    named INTEGER NOT NULL DEFAULT 0,
    -- Whether the human put it away, out of the list.
    archived INTEGER NOT NULL DEFAULT 0,
    -- The command that checks the chat's goal, which the chat works
    -- until it passes, while the human has set one.
    goal_command TEXT,
    -- The chat whose agent opened this one, for a chat opened from a chat.
    parent INTEGER REFERENCES chats (id),
    -- How the agent of a chat opened from a chat is set up, as JSON, as
    -- the chat that opened it said.
    setup TEXT,
    -- The words of the chat's goal, which an agent judges met or not,
    -- while the human has set a goal in words. A chat has one goal, so
    -- this and goal_command are not both set.
    goal_words TEXT,
    -- What its agent lets the human choose of its session, such as the
    -- model, each as it is set, as JSON, once the agent has said.
    settings TEXT,
    -- The id of the agent the chat talks to, once the human chose one;
    -- NULL for the agent a chat starts with.
    agent TEXT
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
CREATE TABLE choices (
    -- The id of the agent whose session has the setting.
    agent TEXT NOT NULL,
    -- The id the agent knows a setting of its session by, as `model`.
    setting TEXT NOT NULL,
    -- What the human set it to last, which a new chat of the agent
    -- starts with.
    value TEXT NOT NULL,
    PRIMARY KEY (agent, setting)
);
CREATE TABLE stages (
    -- The folder whose chats work on the stage, as a project's.
    folder TEXT PRIMARY KEY,
    -- Its tabs, as JSON, each with the chat it is of.
    tabs TEXT NOT NULL
);
";

/// What makes the tables of each earlier version into those of the next:
/// the first of version 1, and so on.
const UPGRADES: &[&str] = &[
    "ALTER TABLE chats ADD COLUMN session TEXT;",
    "ALTER TABLE chats ADD COLUMN named INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE chats ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;",
    "ALTER TABLE chats ADD COLUMN goal_command TEXT;",
    "ALTER TABLE chats ADD COLUMN parent INTEGER REFERENCES chats (id);
     ALTER TABLE chats ADD COLUMN setup TEXT;",
    "ALTER TABLE chats ADD COLUMN goal_words TEXT;",
    "ALTER TABLE chats ADD COLUMN settings TEXT;
     CREATE TABLE choices (setting TEXT PRIMARY KEY, value TEXT NOT NULL);",
    // The choices of version 7 were of no agent in particular, and are
    // left behind.
    "ALTER TABLE chats ADD COLUMN agent TEXT;
     DROP TABLE choices;
     CREATE TABLE choices (
         agent TEXT NOT NULL,
         setting TEXT NOT NULL,
         value TEXT NOT NULL,
         PRIMARY KEY (agent, setting)
     );",
    "CREATE TABLE stages (folder TEXT PRIMARY KEY, tabs TEXT NOT NULL);",
    // Each chat kept a stage of its own, before the chats of a folder
    // shared the stages.
    "ALTER TABLE chats DROP COLUMN tabs;",
];

/// How long a write waits for another Cox writing the same database.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A project kept: a folder the human opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ProjectId(i64);

/// The number the project is kept by, as its icon is named after it.
impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A chat kept, which an entry of another chat's conversation can name,
/// as the number it is kept by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ChatId(i64);

/// The number the chat is kept by, as the logs say which chat a line is of.
impl std::fmt::Display for ChatId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptProject {
    pub id: ProjectId,
    pub folder: PathBuf,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptChat {
    pub id: ChatId,
    /// Its project, or `None` for Cox's own.
    pub project: Option<ProjectId>,
    pub name: String,
    pub named: Named,
    /// Whether the human put it away, out of the list.
    pub archived: bool,
    /// What it works until, if the human set a goal.
    pub goal: Option<Goal>,
    /// The chat whose agent opened it, for a chat opened from a chat.
    pub parent: Option<ChatId>,
    /// How its agent is set up, as JSON, for a chat opened from a chat
    /// whose opener said.
    pub setup: Option<String>,
}

/// Where a chat's name came from, from the name that gives way most
/// readily to the one that lasts most: a chat is named by a number when
/// it opens, then after the first line the human sends, then by the title
/// its agent gives it, until the human names it.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum Named {
    #[default]
    Numbered,
    FirstLine,
    Agent,
    Human,
}

impl Named {
    const ALL: [Self; 4] = [Self::Numbered, Self::FirstLine, Self::Agent, Self::Human];

    /// How it is kept: its place in [`Named::ALL`].
    fn number(self) -> i64 {
        self as i64
    }

    /// What `number` keeps, or a numbered name for a number a newer Cox
    /// kept.
    fn from_number(number: i64) -> Self {
        usize::try_from(number)
            .ok()
            .and_then(|number| Self::ALL.get(number).copied())
            .unwrap_or_default()
    }
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
        let mut statement = self.connection.prepare(
            "SELECT id, project, name, named, archived, goal_command, parent, setup, goal_words
             FROM chats ORDER BY id",
        )?;
        let chats = statement.query_map([], |row| {
            let command: Option<String> = row.get(5)?;
            let words: Option<String> = row.get(8)?;
            Ok(KeptChat {
                id: ChatId(row.get(0)?),
                project: row.get::<_, Option<i64>>(1)?.map(ProjectId),
                name: row.get(2)?,
                named: Named::from_number(row.get(3)?),
                archived: row.get(4)?,
                goal: command.map(Goal::Command).or(words.map(Goal::Words)),
                parent: row.get::<_, Option<i64>>(6)?.map(ChatId),
                setup: row.get(7)?,
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

    /// Keeps that `project` works in `folder` from now on, named `name`,
    /// as when the human moved its folder there, with the tabs of the
    /// stage of the folder it worked in, each path in the folder moved
    /// with it.
    ///
    /// # Errors
    ///
    /// When another project works in `folder`, or the database cannot be
    /// written.
    pub fn move_project(
        &self,
        project: ProjectId,
        folder: &Path,
        name: &str,
    ) -> anyhow::Result<()> {
        let to = folder.to_string_lossy();
        let other: Option<i64> = self
            .connection
            .query_row(
                "SELECT id FROM projects WHERE folder = ?1 AND id != ?2",
                params![to, project.0],
                |row| row.get(0),
            )
            .optional()?;
        if other.is_some() {
            bail!("another project works in {}", folder.display());
        }
        let from: String = self.connection.query_row(
            "SELECT folder FROM projects WHERE id = ?1",
            [project.0],
            |row| row.get(0),
        )?;
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE projects SET folder = ?2, name = ?3 WHERE id = ?1",
            params![project.0, to, name],
        )?;
        let tabs: Option<String> = transaction
            .query_row(
                "SELECT tabs FROM stages WHERE folder = ?1",
                [&from],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(tabs) = tabs {
            let mut tabs: serde_json::Value = serde_json::from_str(&tabs)?;
            let moved = Moved {
                from: Path::new(&from),
                to: folder,
            };
            moved.json(&mut tabs);
            transaction.execute("DELETE FROM stages WHERE folder = ?1", [&from])?;
            transaction.execute(
                "INSERT INTO stages (folder, tabs) VALUES (?1, ?2)
                 ON CONFLICT (folder) DO UPDATE SET tabs = excluded.tabs",
                params![to, serde_json::to_string(&tabs)?],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Forgets the session of `chat`'s agent, which it cannot go on with,
    /// as when its folder moved.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn forget_session(&self, chat: ChatId) -> anyhow::Result<()> {
        self.connection
            .execute("UPDATE chats SET session = NULL WHERE id = ?1", [chat.0])?;
        Ok(())
    }

    /// Keeps a new chat named `name` in `project`, or of Cox's own.
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

    /// Keeps a new chat named `name` in `project`, or of Cox's own, which
    /// the agent of `parent` opened, with its agent set up as `setup`
    /// says, if it said. The chat talks to the agent `parent` talks to.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn add_chat_under(
        &self,
        parent: ChatId,
        project: Option<ProjectId>,
        name: &str,
        setup: Option<&str>,
    ) -> anyhow::Result<ChatId> {
        self.connection.execute(
            "INSERT INTO chats (project, name, created, parent, setup, agent)
             VALUES (?1, ?2, ?3, ?4, ?5, (SELECT agent FROM chats WHERE id = ?4))",
            params![
                project.map(|project| project.0),
                name,
                now(),
                parent.0,
                setup
            ],
        )?;
        Ok(ChatId(self.connection.last_insert_rowid()))
    }

    /// Keeps `name` as that of `chat`, which `named` says where it came
    /// from.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn name_chat(&self, chat: ChatId, name: &str, named: Named) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET name = ?2, named = ?3 WHERE id = ?1",
            params![chat.0, name, named.number()],
        )?;
        Ok(())
    }

    /// Keeps whether the human put `chat` away, out of the list.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn archive_chat(&self, chat: ChatId, archived: bool) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET archived = ?2 WHERE id = ?1",
            params![chat.0, archived],
        )?;
        Ok(())
    }

    /// Keeps `goal` as what `chat` works until, or that it works toward no
    /// goal.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_goal(&self, chat: ChatId, goal: Option<&Goal>) -> anyhow::Result<()> {
        let command = goal.and_then(Goal::command);
        let words = goal.and_then(Goal::words);
        self.connection.execute(
            "UPDATE chats SET goal_command = ?2, goal_words = ?3 WHERE id = ?1",
            params![chat.0, command, words],
        )?;
        Ok(())
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

    /// The tabs of the stage of `folder`, where its chats work, if it has
    /// one.
    ///
    /// # Errors
    ///
    /// When the database cannot be read, or holds tabs this Cox cannot.
    pub fn stage(&self, folder: &Path) -> anyhow::Result<Option<KeptTabs>> {
        let tabs: Option<String> = self
            .connection
            .query_row(
                "SELECT tabs FROM stages WHERE folder = ?1",
                [folder.to_string_lossy()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(tabs.map(|tabs| serde_json::from_str(&tabs)).transpose()?)
    }

    /// Keeps `tabs` as those of the stage of `folder`.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_stage(&self, folder: &Path, tabs: &KeptTabs) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO stages (folder, tabs) VALUES (?1, ?2)
             ON CONFLICT (folder) DO UPDATE SET tabs = excluded.tabs",
            params![folder.to_string_lossy(), serde_json::to_string(tabs)?],
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

    /// Keeps `setup` as how the agent of `chat` is set up, as JSON, or
    /// that it is set up with nothing, and forgets the session of its
    /// agent, which a new setup may not go on with, as when the agent
    /// works in another folder from then on.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_setup(&self, chat: ChatId, setup: Option<&str>) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET setup = ?2, session = NULL WHERE id = ?1",
            params![chat.0, setup],
        )?;
        Ok(())
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

    /// The id of the agent `chat` talks to, once the human chose one.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn agent(&self, chat: ChatId) -> anyhow::Result<Option<String>> {
        Ok(self
            .connection
            .query_row("SELECT agent FROM chats WHERE id = ?1", [chat.0], |row| {
                row.get(0)
            })?)
    }

    /// Keeps `agent` as the id of the agent `chat` talks to from now on,
    /// and forgets the session and the settings of the agent it had,
    /// which the new one has none of.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_agent(&self, chat: ChatId, agent: &str) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET agent = ?2, session = NULL, settings = NULL WHERE id = ?1",
            params![chat.0, agent],
        )?;
        Ok(())
    }

    /// The settings of `chat`'s session, as they were last set: none until
    /// its agent said what they are.
    ///
    /// # Errors
    ///
    /// When the database cannot be read, or holds settings this Cox cannot.
    pub fn settings(&self, chat: ChatId) -> anyhow::Result<Vec<Setting>> {
        let settings: Option<String> = self.connection.query_row(
            "SELECT settings FROM chats WHERE id = ?1",
            [chat.0],
            |row| row.get(0),
        )?;
        Ok(settings
            .map(|settings| serde_json::from_str(&settings))
            .transpose()?
            .unwrap_or_default())
    }

    /// Keeps `settings` as those of `chat`'s session.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_settings(&self, chat: ChatId, settings: &[Setting]) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET settings = ?2 WHERE id = ?1",
            params![chat.0, serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    /// What the human set each setting of `agent`'s sessions to last, by
    /// the setting's id, of the settings a new chat starts with, in the
    /// order they were first set.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn choices(&self, agent: &str) -> anyhow::Result<Vec<(String, String)>> {
        let mut statement = self
            .connection
            .prepare("SELECT setting, value FROM choices WHERE agent = ?1 ORDER BY rowid")?;
        let choices = statement.query_map([agent], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(choices.collect::<Result<_, _>>()?)
    }

    /// Keeps `value` as what the human set the setting `setting` of
    /// `agent`'s sessions to last.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_choice(&self, agent: &str, setting: &str, value: &str) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO choices (agent, setting, value) VALUES (?1, ?2, ?3)
             ON CONFLICT (agent, setting) DO UPDATE SET value = excluded.value",
            params![agent, setting, value],
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

    /// The chat this keeps.
    #[must_use]
    pub fn chat(&self) -> ChatId {
        self.chat
    }

    /// Keeps `entry` as the `number`th of the chat.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn entry(&self, number: usize, entry: &Entry) -> anyhow::Result<()> {
        self.store.keep_entry(self.chat, number, entry)
    }

    /// Keeps `session` as the id of the session of the chat's agent.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn session(&self, session: &str) -> anyhow::Result<()> {
        self.store.keep_session(self.chat, session)
    }

    /// Keeps `goal` as what the chat works until, or that it works toward
    /// no goal.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn goal(&self, goal: Option<&Goal>) -> anyhow::Result<()> {
        self.store.keep_goal(self.chat, goal)
    }

    /// Keeps `settings` as those of the chat's session.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn settings(&self, settings: &[Setting]) -> anyhow::Result<()> {
        self.store.keep_settings(self.chat, settings)
    }

    /// What the human set each setting of `agent`'s sessions to last, of
    /// those a new chat starts with, as [`Store::choices`] says.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn choices(&self, agent: &str) -> anyhow::Result<Vec<(String, String)>> {
        self.store.choices(agent)
    }

    /// Keeps `value` as what the human set the setting `setting` of
    /// `agent`'s sessions to last, for a new chat to start with.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn choose(&self, agent: &str, setting: &str, value: &str) -> anyhow::Result<()> {
        self.store.keep_choice(agent, setting, value)
    }

    /// The id of the agent the chat talks to, once the human chose one.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn agent(&self) -> anyhow::Result<Option<String>> {
        self.store.agent(self.chat)
    }

    /// Keeps `agent` as the id of the agent the chat talks to from now on,
    /// forgetting the session and settings of the one it had.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_agent(&self, agent: &str) -> anyhow::Result<()> {
        self.store.keep_agent(self.chat, agent)
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
    use crate::entry::{Content, Step, StepState, ToolCallResult};
    use crate::tabs::KeptTab;

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
    fn keeps_a_chats_name_and_whether_it_is_put_away() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let other = store.add_chat(None, "Chat 2").unwrap();
        let kept = |store: &Store| {
            store
                .chats()
                .unwrap()
                .into_iter()
                .map(|chat| (chat.name, chat.named, chat.archived))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kept(&store),
            [
                ("Chat 1".to_owned(), Named::Numbered, false),
                ("Chat 2".to_owned(), Named::Numbered, false),
            ]
        );

        store
            .name_chat(chat, "Fix the tests", Named::Agent)
            .unwrap();
        store.archive_chat(other, true).unwrap();
        assert_eq!(
            kept(&store),
            [
                ("Fix the tests".to_owned(), Named::Agent, false),
                ("Chat 2".to_owned(), Named::Numbered, true),
            ]
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
    fn keeps_what_the_agent_thought_and_planned() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let said = [
            Entry::Thought("Read it first.".to_owned()),
            Entry::Plan(vec![
                Step {
                    text: "Read the code".to_owned(),
                    state: StepState::Completed,
                },
                Step {
                    text: "Fix it".to_owned(),
                    state: StepState::InProgress,
                },
            ]),
        ];
        for (number, entry) in said.iter().enumerate() {
            store.keep_entry(chat, number, entry).unwrap();
        }

        assert_eq!(store.entries(chat).unwrap(), said);
    }

    #[test]
    fn keeps_the_stage_of_a_folder_with_the_chat_of_each_tab() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let folder = Path::new("/work");
        assert_eq!(store.stage(folder).unwrap(), None);

        let tabs = KeptTabs {
            tabs: vec![
                KeptTab::Files { open: Vec::new() },
                KeptTab::Markdown {
                    path: "/work/plan.md".into(),
                },
            ],
            shown: 1,
            owners: vec![None, Some(chat)],
        };
        store.keep_stage(folder, &tabs).unwrap();
        assert_eq!(store.stage(folder).unwrap(), Some(tabs.clone()));
        let shown = KeptTabs { shown: 0, ..tabs };
        store.keep_stage(folder, &shown).unwrap();
        assert_eq!(store.stage(folder).unwrap(), Some(shown));
        assert_eq!(store.stage(Path::new("/elsewhere")).unwrap(), None);
    }

    #[test]
    fn a_project_moves_to_another_folder_with_its_stage() {
        let store = Store::in_memory().unwrap();
        let project = store.project(Path::new("/work/app"), "app").unwrap();
        let other = store.project(Path::new("/work/other"), "other").unwrap();
        let chat = store.add_chat(Some(project), "Chat 1").unwrap();
        store.keep_session(chat, "s-1").unwrap();
        let tabs = KeptTabs {
            tabs: vec![KeptTab::Markdown {
                path: "/work/app/plan.md".into(),
            }],
            shown: 0,
            owners: vec![Some(chat)],
        };
        store.keep_stage(Path::new("/work/app"), &tabs).unwrap();

        assert!(
            store
                .move_project(project, Path::new("/work/other"), "other")
                .is_err()
        );
        store
            .move_project(project, Path::new("/src/app-2"), "app-2")
            .unwrap();
        let projects = store.projects().unwrap();
        assert_eq!(projects[0].folder, Path::new("/src/app-2"));
        assert_eq!(projects[0].name, "app-2");
        assert_eq!(projects[1].id, other);
        assert_eq!(store.stage(Path::new("/work/app")).unwrap(), None);
        let moved = KeptTabs {
            tabs: vec![KeptTab::Markdown {
                path: "/src/app-2/plan.md".into(),
            }],
            ..tabs
        };
        assert_eq!(store.stage(Path::new("/src/app-2")).unwrap(), Some(moved));
        // The chat stays the project's, and its session is the daemon's
        // to forget.
        assert_eq!(store.chats().unwrap()[0].project, Some(project));
        store.forget_session(chat).unwrap();
        assert_eq!(store.session(chat).unwrap(), None);
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
    fn keeps_the_settings_of_a_chats_session_and_the_last_choices() {
        use crate::settings::{Choice, Setting};
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        assert_eq!(store.settings(chat).unwrap(), []);

        let settings = [Setting {
            id: "model".to_owned(),
            name: "Model".to_owned(),
            description: None,
            category: Some("model".to_owned()),
            value: "small".to_owned(),
            choices: vec![Choice {
                id: "small".to_owned(),
                name: "Small".to_owned(),
                description: Some("Quick".to_owned()),
                group: None,
            }],
        }];
        store.keep_settings(chat, &settings).unwrap();
        assert_eq!(store.settings(chat).unwrap(), settings);

        assert_eq!(store.choices("claude").unwrap(), []);
        store.keep_choice("claude", "model", "large").unwrap();
        store.keep_choice("claude", "effort", "high").unwrap();
        store.keep_choice("codex", "model", "fast").unwrap();
        store.keep_choice("claude", "model", "small").unwrap();
        assert_eq!(
            store.choices("claude").unwrap(),
            [
                ("model".to_owned(), "small".to_owned()),
                ("effort".to_owned(), "high".to_owned())
            ]
        );
        assert_eq!(
            store.choices("codex").unwrap(),
            [("model".to_owned(), "fast".to_owned())]
        );
    }

    #[test]
    fn keeps_the_agent_of_a_chat_and_forgets_what_the_one_before_had() {
        use crate::settings::Setting;
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        assert_eq!(store.agent(chat).unwrap(), None);
        store.keep_session(chat, "session-1").unwrap();
        let setting = Setting {
            id: "model".to_owned(),
            name: "Model".to_owned(),
            description: None,
            category: None,
            value: "small".to_owned(),
            choices: Vec::new(),
        };
        store.keep_settings(chat, &[setting]).unwrap();

        store.keep_agent(chat, "codex").unwrap();
        assert_eq!(store.agent(chat).unwrap().as_deref(), Some("codex"));
        assert_eq!(store.session(chat).unwrap(), None);
        assert_eq!(store.settings(chat).unwrap(), []);
        // A chat its agent opens talks to the same agent.
        let child = store.add_chat_under(chat, None, "Chat 2", None).unwrap();
        assert_eq!(store.agent(child).unwrap().as_deref(), Some("codex"));
    }

    #[test]
    fn keeps_a_new_setup_and_forgets_the_session() {
        let store = Store::in_memory().unwrap();
        let parent = store.add_chat(None, "Chat 1").unwrap();
        let chat = store
            .add_chat_under(
                parent,
                None,
                "Chat 2",
                Some(r#"{"folder":"/sketches/a-1"}"#),
            )
            .unwrap();
        store.keep_session(chat, "session-1").unwrap();

        store.keep_setup(chat, Some("{}")).unwrap();
        let setup = |store: &Store| {
            store
                .chats()
                .unwrap()
                .into_iter()
                .find(|kept| kept.id == chat)
                .unwrap()
                .setup
        };
        assert_eq!(setup(&store).as_deref(), Some("{}"));
        assert_eq!(store.session(chat).unwrap(), None);
        store.keep_setup(chat, None).unwrap();
        assert_eq!(setup(&store), None);
    }

    #[test]
    fn keeps_the_goal_of_a_chat_while_the_human_sets_one() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let other = store.add_chat(None, "Chat 2").unwrap();
        let goals = |store: &Store| {
            store
                .chats()
                .unwrap()
                .into_iter()
                .map(|chat| chat.goal)
                .collect::<Vec<_>>()
        };
        assert_eq!(goals(&store), [None, None]);

        let ci = Goal::Command("cargo xtask ci".to_owned());
        let words = Goal::Words("The greeting names the user.".to_owned());
        store.keep_goal(chat, Some(&ci)).unwrap();
        assert_eq!(goals(&store), [Some(ci.clone()), None]);
        store.keep_goal(other, Some(&words)).unwrap();
        store.keep_goal(chat, None).unwrap();
        assert_eq!(goals(&store), [None, Some(words.clone())]);
        // A goal in words takes the place of a command, and the other way.
        store.keep_goal(other, Some(&ci)).unwrap();
        assert_eq!(goals(&store), [None, Some(ci)]);
        store.keep_goal(other, Some(&words)).unwrap();
        assert_eq!(goals(&store), [None, Some(words)]);
    }

    #[test]
    fn keeps_which_chat_opened_a_chat_and_how_its_agent_is_set_up() {
        let store = Store::in_memory().unwrap();
        let cox = store.project(Path::new("/src/cox"), "cox").unwrap();
        let parent = store.add_chat(Some(cox), "Chat 1").unwrap();
        let child = store
            .add_chat_under(parent, Some(cox), "Chat 2", Some(r#"{"stage":"look"}"#))
            .unwrap();
        let plain = store
            .add_chat_under(parent, Some(cox), "Chat 3", None)
            .unwrap();

        let chats: Vec<_> = store
            .chats()
            .unwrap()
            .into_iter()
            .map(|chat| (chat.id, chat.parent, chat.setup))
            .collect();
        assert_eq!(
            chats,
            [
                (parent, None, None),
                (child, Some(parent), Some(r#"{"stage":"look"}"#.to_owned())),
                (plain, Some(parent), None),
            ]
        );
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
        assert_eq!(chats[0].named, Named::Numbered);
        assert!(!chats[0].archived);
        assert_eq!(chats[0].goal, None);
        assert_eq!(chats[0].parent, None);
        assert_eq!(chats[0].setup, None);
        assert_eq!(store.session(chats[0].id).unwrap(), None);
        store.keep_session(chats[0].id, "session-1").unwrap();
        store
            .keep_goal(chats[0].id, Some(&Goal::Command("make".to_owned())))
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.session(chats[0].id).unwrap().as_deref(),
            Some("session-1")
        );
        assert_eq!(
            store.chats().unwrap()[0].goal,
            Some(Goal::Command("make".to_owned()))
        );
        let child = store
            .add_chat_under(chats[0].id, None, "Chat 2", Some("{}"))
            .unwrap();
        let kept = &store.chats().unwrap()[1];
        assert_eq!((kept.id, kept.parent), (child, Some(chats[0].id)));
        assert_eq!(kept.setup.as_deref(), Some("{}"));
        assert_eq!(store.settings(child).unwrap(), []);
        assert_eq!(store.agent(child).unwrap(), None);
        store.keep_choice("claude", "model", "small").unwrap();
        assert_eq!(store.choices("claude").unwrap().len(), 1);
        // The stage each chat kept of its own is gone.
        assert!(store.connection.prepare("SELECT tabs FROM chats").is_err());
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
