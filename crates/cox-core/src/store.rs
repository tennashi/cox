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

use crate::entry::{Entry, Step, ToolCallResult};
use crate::goal::Goal;
use crate::moved::Moved;
use crate::settings::Setting;
use crate::tabs::KeptTabs;

/// The version of the tables below, as `SQLite`'s `user_version` holds it.
const VERSION: i64 = 27;

const TABLES: &str = "
CREATE TABLE projects (
    id INTEGER PRIMARY KEY,
    folder TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    -- Whether the human put it away, off the rail.
    archived INTEGER NOT NULL DEFAULT 0,
    -- What it is, in a sentence or two, as each chat's agent is told.
    about TEXT
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
    agent TEXT,
    -- The folder it works in, when it is not its project's: the one that
    -- was, as the chat opened, before another of the project's became it.
    folder TEXT,
    -- The steps of the plan its agent first sent after the human's last
    -- line, as JSON, which the plan is told against as it changes.
    baseline TEXT,
    -- The commands its agent runs when a line starts with `/` and one of
    -- their names, as JSON, as it last told them, for the human to be
    -- offered them while it does not run.
    commands TEXT,
    -- Where the agent of its session runs subagents, `agent` or `chats`,
    -- as the settings said when the session started: NULL for a session
    -- that started before Cox kept it, which ran them as `agent`.
    subagents TEXT,
    -- What its agent spent, as JSON, once the agent told any of it: the
    -- cost, how full the context is, and the tokens of its turns.
    spent TEXT,
    -- When it was marked done, in milliseconds since the epoch, and by
    -- whom, `human` or `agent`, while it is: both NULL, or both set.
    resolved_at INTEGER,
    resolved_by TEXT
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
-- The lines the human wrote on a step of a plan, which the Activity page
-- reads the threads of the steps by.
CREATE INDEX on_steps ON entries (chat, number) WHERE entry LIKE '{\"on_step\":%';
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
-- The chat the window showed last of each project, and when the human
-- chose the project last, for a window that starts to go back to.
CREATE TABLE shown (
    -- The project, by its id, or 0 for Cox's own.
    project INTEGER PRIMARY KEY,
    -- The chat of it shown last, by its id, once one was.
    chat INTEGER,
    -- When the human chose the project last, in milliseconds since the
    -- epoch.
    at INTEGER NOT NULL
);
-- How each turn of a chat's agent ended, and when: what the Activity
-- page tells a chat's story by, as the entries keep no end of a turn
-- that ended as turns do.
CREATE TABLE turns (
    chat INTEGER NOT NULL REFERENCES chats (id),
    -- When it ended, in milliseconds since the epoch.
    at INTEGER NOT NULL,
    -- How it ended, as `Ended` names it, such as `done` or `stopped`.
    how TEXT NOT NULL
);
CREATE INDEX turns_of ON turns (chat, at);
-- The plan each chat's agent sent last, as its entry keeps it, kept
-- again as that entry is: what the Activity page reads every two
-- seconds, without looking for it among the chat's entries.
CREATE TABLE last_plans (
    chat INTEGER PRIMARY KEY REFERENCES chats (id),
    -- The number of its entry.
    number INTEGER NOT NULL,
    -- Its steps, as JSON.
    steps TEXT NOT NULL
);
-- Each step the plans of a chat's agent had, by what coxd knows it by,
-- as the last plan that had it says, kept as each plan is: what the
-- Activity page sums up what got done by, and tells a chat's story by,
-- without reading every plan the chat kept.
CREATE TABLE steps (
    chat INTEGER NOT NULL REFERENCES chats (id),
    -- What coxd knows it by in its chat, from 1.
    id INTEGER NOT NULL,
    text TEXT NOT NULL,
    -- When coxd first saw it under way, in milliseconds since the epoch.
    started INTEGER,
    -- When coxd saw it done, while it is.
    done INTEGER,
    PRIMARY KEY (chat, id)
) WITHOUT ROWID;
CREATE INDEX steps_done ON steps (done) WHERE done IS NOT NULL;
-- The entries that tell a chat's story, past its steps: the human's
-- lines, of which the first starts it, the receipts of how its plan
-- changed, the questions, the commits, and what ran as a turn ended or
-- as something else happened, as the chat's goal and its triggers.
CREATE INDEX milestones ON entries (chat, number) WHERE entry LIKE '{\"human\":%'
    OR entry LIKE '{\"plan_revised\":%' OR entry LIKE '{\"question\":%'
    OR entry LIKE '{\"commits\":%' OR entry LIKE '{\"checks\":%'
    OR entry LIKE '{\"event\":%';
-- Each command the agents of a chat ran, kept as its entry is: when it
-- started and ended, and how, and the step of commands it is of: what the
-- Activity page tells the commands that ran long or failed by, in a
-- chat's story, without reading every command the chat ran.
CREATE TABLE commands (
    chat INTEGER NOT NULL REFERENCES chats (id),
    -- The number of its entry.
    number INTEGER NOT NULL,
    -- The number of the entry before it of what the agent replied, or of
    -- a line the agent was sent, if one is: the commands it ran one after
    -- another, with nothing said between, share it.
    step INTEGER,
    -- When it started, in milliseconds since the epoch: when coxd kept
    -- it as the command it runs last read, as an agent tells the command
    -- once it has written it.
    started INTEGER NOT NULL,
    -- When coxd saw it end; NULL while it runs, and for one an older Cox
    -- kept.
    ended INTEGER,
    -- How it ended, as `ToolCallResult` names it, once it did.
    result TEXT,
    PRIMARY KEY (chat, number)
) WITHOUT ROWID;
-- The branch the human chose to compare each directory of a project with,
-- in what changed, by the directory's name: '' for the project's folder,
-- when it is a repository itself.
CREATE TABLE bases (
    project INTEGER NOT NULL REFERENCES projects (id),
    directory TEXT NOT NULL,
    base TEXT NOT NULL,
    PRIMARY KEY (project, directory)
) WITHOUT ROWID;
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
    "ALTER TABLE chats ADD COLUMN folder TEXT;
     CREATE TABLE folders (
         project INTEGER NOT NULL REFERENCES projects (id),
         folder TEXT NOT NULL,
         UNIQUE (project, folder)
     );",
    "ALTER TABLE chats ADD COLUMN baseline TEXT;",
    // A project worked in folders beside its own, before it kept to its
    // own.
    "DROP TABLE folders;",
    "ALTER TABLE projects ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;",
    "ALTER TABLE projects ADD COLUMN about TEXT;",
    "ALTER TABLE chats ADD COLUMN commands TEXT;",
    "ALTER TABLE chats ADD COLUMN subagents TEXT;",
    "CREATE INDEX plans ON entries (chat, number) WHERE entry LIKE '{\"plan\":%';",
    "ALTER TABLE chats ADD COLUMN spent TEXT;",
    "ALTER TABLE chats ADD COLUMN resolved_at INTEGER;
     ALTER TABLE chats ADD COLUMN resolved_by TEXT;",
    "CREATE INDEX on_steps ON entries (chat, number) WHERE entry LIKE '{\"on_step\":%';",
    "CREATE TABLE shown (project INTEGER PRIMARY KEY, chat INTEGER, at INTEGER NOT NULL);",
    "CREATE TABLE turns (chat INTEGER NOT NULL REFERENCES chats (id), at INTEGER NOT NULL, how TEXT NOT NULL);
     CREATE INDEX turns_of ON turns (chat, at);",
    // Kept from the plans the entries keep, each step as the last plan
    // that had it says, which nothing then looks for among the entries.
    r#"CREATE TABLE last_plans (
         chat INTEGER PRIMARY KEY REFERENCES chats (id),
         number INTEGER NOT NULL,
         steps TEXT NOT NULL
     );
     CREATE TABLE steps (
         chat INTEGER NOT NULL REFERENCES chats (id),
         id INTEGER NOT NULL,
         text TEXT NOT NULL,
         started INTEGER,
         done INTEGER,
         PRIMARY KEY (chat, id)
     ) WITHOUT ROWID;
     CREATE INDEX steps_done ON steps (done) WHERE done IS NOT NULL;
     CREATE INDEX milestones ON entries (chat, number) WHERE entry LIKE '{"human":%'
         OR entry LIKE '{"plan_revised":%' OR entry LIKE '{"question":%'
         OR entry LIKE '{"commits":%';
     INSERT INTO last_plans (chat, number, steps)
         SELECT chat, max(number), json_extract(entry, '$.plan') FROM entries
         WHERE entry LIKE '{"plan":%' GROUP BY chat;
     INSERT INTO steps (chat, id, text, started, done)
         SELECT entries.chat, step.value ->> '$.id', step.value ->> '$.text',
             step.value ->> '$.started',
             CASE WHEN step.value ->> '$.state' = 'completed' THEN step.value ->> '$.since' END
         FROM entries, json_each(entries.entry, '$.plan') AS step
         WHERE entries.entry LIKE '{"plan":%' AND step.value ->> '$.id' IS NOT NULL
         ORDER BY entries.chat, entries.number
         ON CONFLICT (chat, id) DO UPDATE SET text = excluded.text,
             started = coalesce(excluded.started, steps.started), done = excluded.done;
     DROP INDEX plans;"#,
    // The story told what ran as a turn ended, or as something else
    // happened, too.
    r#"DROP INDEX milestones;
     CREATE INDEX milestones ON entries (chat, number) WHERE entry LIKE '{"human":%'
         OR entry LIKE '{"plan_revised":%' OR entry LIKE '{"question":%'
         OR entry LIKE '{"commits":%' OR entry LIKE '{"checks":%'
         OR entry LIKE '{"event":%';"#,
    // Kept from the commands the entries keep, with when each was first
    // kept as when it started, and no end, which an older Cox did not
    // keep.
    r#"CREATE TABLE commands (
         chat INTEGER NOT NULL REFERENCES chats (id),
         number INTEGER NOT NULL,
         step INTEGER,
         started INTEGER NOT NULL,
         ended INTEGER,
         result TEXT,
         PRIMARY KEY (chat, number)
     ) WITHOUT ROWID;
     INSERT INTO commands (chat, number, step, started, result)
         SELECT chat, number, step, at, result FROM (
             SELECT chat, number, at,
                 CASE WHEN entry LIKE '{"tool_call":%' THEN entry ->> '$.tool_call.command' END
                     AS command,
                 CASE WHEN entry LIKE '{"tool_call":%' THEN entry ->> '$.tool_call.result' END
                     AS result,
                 max(CASE WHEN entry LIKE '{"reply":%' OR entry LIKE '{"human":%'
                     OR entry LIKE '{"cox":%' THEN number END)
                     OVER (PARTITION BY chat ORDER BY number) AS step
             FROM entries)
         WHERE command;"#,
    "CREATE TABLE bases (
         project INTEGER NOT NULL REFERENCES projects (id),
         directory TEXT NOT NULL,
         base TEXT NOT NULL,
         PRIMARY KEY (project, directory)
     ) WITHOUT ROWID;",
];

/// What keeps an entry of a chat, in place of what was kept there.
const KEEP_ENTRY: &str = "INSERT INTO entries (chat, number, at, entry) VALUES (?1, ?2, ?3, ?4)
     ON CONFLICT (chat, number) DO UPDATE SET entry = excluded.entry";

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

impl ChatId {
    /// The number the chat is kept by, as an agent names it to read it.
    #[must_use]
    pub const fn number(self) -> i64 {
        self.0
    }
}

/// The number the chat is kept by, as the logs say which chat a line is of.
impl std::fmt::Display for ChatId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptProject {
    pub id: ProjectId,
    /// The folder its chats work in.
    pub folder: PathBuf,
    pub name: String,
    /// Whether the human put it away, off the rail.
    pub archived: bool,
    /// What it is, in a sentence or two, if the human said.
    pub about: Option<String>,
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
    /// The folder it works in, when it is not its project's, as one that
    /// opened before another folder of the project became the project's.
    pub folder: Option<PathBuf>,
    /// What its agent spent, as JSON, as [`Store::keep_spent`] kept it,
    /// once the agent told any of it.
    pub spent: Option<String>,
    /// When it was marked done, and by whom, while it is.
    pub resolved: Option<Resolved>,
    /// When it last moved, in milliseconds since the epoch: when the last
    /// entry of its conversation was written, or, with none, when it was
    /// opened.
    pub active: i64,
}

/// The chat the window showed last of a project, and when the human chose
/// the project last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeptShown {
    /// The project, or `None` for Cox's own.
    pub project: Option<ProjectId>,
    /// The chat of it shown last, once one was.
    pub chat: Option<ChatId>,
    /// When the human chose the project last, in milliseconds since the
    /// epoch.
    pub at: i64,
}

/// A chat marked done: it stays in the list, folded away at its end, its
/// agent closed, until something arrives in it or someone reopens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Resolved {
    /// When, in milliseconds since the epoch.
    pub at: i64,
    pub by: ResolvedBy,
}

impl Resolved {
    /// Marked done now, by `by`.
    #[must_use]
    pub fn now(by: ResolvedBy) -> Self {
        Self { at: now(), by }
    }
}

/// Who marked a chat done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedBy {
    /// The human, from the chat's menu.
    Human,
    /// Cox, for an agent's work that came to its end: a subagent whose
    /// report reached the agent that opened it, or a chat whose merge
    /// ended.
    Agent,
}

impl ResolvedBy {
    /// How it is kept.
    const fn word(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }

    /// What `word` keeps; a word a newer Cox kept is the human's.
    fn from_word(word: &str) -> Self {
        if word == "agent" {
            Self::Agent
        } else {
            Self::Human
        }
    }
}

/// How a turn of a chat's agent ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ended {
    /// The agent ended it, having done what it was asked.
    Done,
    /// The human, or Cox, stopped it.
    Stopped,
    /// The agent's reply grew too long.
    TooLong,
    /// The agent made more requests in it than it may.
    TooManyRequests,
    /// The agent refused to go on.
    Refused,
    /// It failed, as when the agent's side erred.
    Failed,
    /// The agent closed while it ran.
    Closed,
    /// For a reason Cox does not know.
    Other,
}

impl Ended {
    /// How it is kept.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Stopped => "stopped",
            Self::TooLong => "too_long",
            Self::TooManyRequests => "too_many_requests",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Closed => "closed",
            Self::Other => "other",
        }
    }

    /// What `word` keeps; a word a newer Cox kept is [`Self::Other`].
    #[must_use]
    pub fn from_word(word: &str) -> Self {
        match word {
            "done" => Self::Done,
            "stopped" => Self::Stopped,
            "too_long" => Self::TooLong,
            "too_many_requests" => Self::TooManyRequests,
            "refused" => Self::Refused,
            "failed" => Self::Failed,
            "closed" => Self::Closed,
            _ => Self::Other,
        }
    }
}

/// A turn of a chat's agent that ended: when, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TurnEnd {
    /// When, in milliseconds since the epoch.
    pub at: i64,
    pub how: Ended,
}

/// The plan a chat's agent sent last, as [`Store::plans`] reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptPlan {
    pub chat: ChatId,
    /// The number of the plan's entry in the chat, which the agent's
    /// updates to it in the same turn keep.
    pub number: usize,
    pub steps: Vec<Step>,
    /// When the last entry of the chat was written, in milliseconds
    /// since the epoch: about when the agent last did anything.
    pub at: i64,
}

/// A step of the plans of a chat's agent, as [`Store::steps`] reads it:
/// by what coxd knows it by in the chat (see [`Step::id`]), as the last
/// plan that had it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptStep {
    pub id: u64,
    pub text: String,
    /// When coxd first saw it under way, in milliseconds since the epoch.
    pub started: Option<i64>,
    /// When it was done, while it is.
    pub done: Option<i64>,
}

/// Commands a chat's agent ran one after another, with nothing said
/// between, as [`Store::ran`] reads them: those of them that ran
/// long or failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptRun {
    /// What the agent replied before it ran them, if it replied since the
    /// line it answers.
    pub said: Option<String>,
    /// How many commands it ran one after another there, those that did
    /// not run long too.
    pub of: usize,
    pub commands: Vec<KeptCommand>,
}

/// A command a chat's agent ran, which ended, as [`Store::ran`] reads
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptCommand {
    /// The number of its entry, from 0.
    pub number: usize,
    /// The command, as the agent gave it.
    pub line: String,
    /// What the agent said it is for, if it said.
    pub about: Option<String>,
    /// What it shows, as what it printed.
    pub content: Vec<crate::entry::Content>,
    pub result: ToolCallResult,
    /// When it started, in milliseconds since the epoch.
    pub started: i64,
    /// When it ended, unless an older Cox kept it, which kept no end.
    pub ended: Option<i64>,
}

/// What moved in the chats since a time, as [`Store::progress`] reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Each chat that wrote an entry since, with when it wrote its last,
    /// in milliseconds since the epoch.
    pub active: Vec<(ChatId, i64)>,
    /// The steps of their agents' plans done since, each once, as the
    /// chat knows it, by its id, however the agent reworded it, while the
    /// last plan that had it says it is done: its chat, its text, as it
    /// reads last, and when it was done.
    pub done: Vec<(ChatId, String, i64)>,
    /// The chats marked done since.
    pub resolved: Vec<(ChatId, Resolved)>,
}

/// Who said a line of the thread of a step, as [`Store::thread`] reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sayer {
    Human,
    Agent,
}

/// A line of the thread of a step of a plan, as [`Store::thread`] reads
/// it: what the human wrote on the step, or what the agent replied to it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Said {
    pub by: Sayer,
    pub text: String,
    /// When it was first written, in milliseconds since the epoch.
    pub at: i64,
    /// The number of its entry in the chat.
    pub number: usize,
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

    /// Opens the database at `path` to read alone, beside the Cox that
    /// writes it, as the window reads what its chats decided.
    ///
    /// # Errors
    ///
    /// When it is not there, cannot be opened, or holds the tables of
    /// another version.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("cannot open {}", path.display()))?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != VERSION {
            bail!("the chats are kept as version {version}, not {VERSION}");
        }
        Ok(Self { connection })
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
            .prepare("SELECT id, folder, name, archived, about FROM projects ORDER BY id")?;
        let projects = statement.query_map([], |row| {
            Ok(KeptProject {
                id: ProjectId(row.get(0)?),
                folder: PathBuf::from(row.get::<_, String>(1)?),
                name: row.get(2)?,
                archived: row.get(3)?,
                about: row.get(4)?,
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
            "SELECT id, project, name, named, archived, goal_command, parent, setup, goal_words,
             folder, spent, resolved_at, resolved_by,
             COALESCE((SELECT at FROM entries WHERE entries.chat = chats.id
                       ORDER BY number DESC LIMIT 1), created)
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
                folder: row.get::<_, Option<String>>(9)?.map(PathBuf::from),
                spent: row.get(10)?,
                resolved: {
                    let by: Option<String> = row.get(12)?;
                    row.get::<_, Option<i64>>(11)?.map(|at| Resolved {
                        at,
                        by: ResolvedBy::from_word(by.as_deref().unwrap_or("")),
                    })
                },
                active: row.get(13)?,
            })
        })?;
        Ok(chats.collect::<Result<_, _>>()?)
    }

    /// The chat the window showed last of each project, and when the human
    /// chose each last, the project chosen last last.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn shown(&self) -> anyhow::Result<Vec<KeptShown>> {
        let mut statement = self
            .connection
            .prepare("SELECT project, chat, at FROM shown ORDER BY at, project")?;
        let rows = statement.query_map([], |row| {
            let project: i64 = row.get(0)?;
            Ok(KeptShown {
                project: (project != 0).then_some(ProjectId(project)),
                chat: row.get::<_, Option<i64>>(1)?.map(ChatId),
                at: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Keeps that the human chose `project`, or Cox's own, now, showing
    /// `chat` of it, or, with none, keeping the chat of it shown before.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_shown(
        &self,
        project: Option<ProjectId>,
        chat: Option<ChatId>,
    ) -> anyhow::Result<()> {
        self.connection
            .prepare_cached(
                // Later than any chosen before, though the clock says the
                // same millisecond, or went back.
                "INSERT INTO shown (project, chat, at)
                 VALUES (?1, ?2, MAX(?3, COALESCE((SELECT MAX(at) + 1 FROM shown), 0)))
                 ON CONFLICT (project) DO UPDATE
                 SET chat = COALESCE(excluded.chat, chat), at = excluded.at",
            )?
            .execute(params![
                project.map_or(0, |project| project.0),
                chat.map(|chat| chat.0),
                now()
            ])?;
        Ok(())
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

    /// Keeps the tabs of the stage of `folder` naming where a folder in
    /// it moved, as `moved` says, as a directory of a project renamed.
    ///
    /// # Errors
    ///
    /// When the database cannot be read or written.
    pub fn move_in_stage(&self, folder: &Path, moved: Moved<'_>) -> anyhow::Result<()> {
        let folder = folder.to_string_lossy();
        let tabs: Option<String> = self
            .connection
            .query_row(
                "SELECT tabs FROM stages WHERE folder = ?1",
                [&folder],
                |row| row.get(0),
            )
            .optional()?;
        let Some(tabs) = tabs else {
            return Ok(());
        };
        let mut tabs: serde_json::Value = serde_json::from_str(&tabs)?;
        if moved.json(&mut tabs) {
            self.connection.execute(
                "UPDATE stages SET tabs = ?2 WHERE folder = ?1",
                params![folder, serde_json::to_string(&tabs)?],
            )?;
        }
        Ok(())
    }

    /// Keeps `folder` as the one `chat` works in, when it is not its
    /// project's, or that it works in its project's.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_folder(&self, chat: ChatId, folder: Option<&Path>) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET folder = ?2 WHERE id = ?1",
            params![chat.0, folder.map(Path::to_string_lossy)],
        )?;
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

    /// Keeps that `chat` is marked done as `resolved` says, or is not.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn resolve_chat(&self, chat: ChatId, resolved: Option<Resolved>) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET resolved_at = ?2, resolved_by = ?3 WHERE id = ?1",
            params![
                chat.0,
                resolved.map(|resolved| resolved.at),
                resolved.map(|resolved| resolved.by.word()),
            ],
        )?;
        Ok(())
    }

    /// Keeps that `chat` is of `project` now, or of none, as a chat of
    /// Cox's own.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn move_chat(&self, chat: ChatId, project: Option<ProjectId>) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET project = ?2 WHERE id = ?1",
            params![chat.0, project.map(|project| project.0)],
        )?;
        Ok(())
    }

    /// Keeps that the human put `project` away, off the rail, or brought it
    /// back.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn archive_project(&self, project: ProjectId, archived: bool) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE projects SET archived = ?2 WHERE id = ?1",
            params![project.0, archived],
        )?;
        Ok(())
    }

    /// Keeps `about` as what `project` is, or that it says nothing of what
    /// it is.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_about(&self, project: ProjectId, about: Option<&str>) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE projects SET about = ?2 WHERE id = ?1",
            params![project.0, about],
        )?;
        Ok(())
    }

    /// The branch the human chose to compare each directory of the project
    /// in `folder` with, by the directory's name, '' for the project's
    /// folder; none for a folder that is no project's.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn bases(&self, folder: &Path) -> anyhow::Result<Vec<(String, String)>> {
        let mut statement = self.connection.prepare(
            "SELECT directory, base FROM bases
             WHERE project = (SELECT id FROM projects WHERE folder = ?1)
             ORDER BY directory",
        )?;
        let bases = statement.query_map([folder.to_string_lossy()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        Ok(bases.collect::<Result<_, _>>()?)
    }

    /// Keeps the branch `base` as the one the human chose to compare the
    /// directory `directory` of `project` with, '' for the project's
    /// folder, or, with none, forgets the one they chose.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_base(
        &self,
        project: ProjectId,
        directory: &str,
        base: Option<&str>,
    ) -> anyhow::Result<()> {
        match base {
            Some(base) => self.connection.execute(
                "INSERT INTO bases (project, directory, base) VALUES (?1, ?2, ?3)
                 ON CONFLICT (project, directory) DO UPDATE SET base = excluded.base",
                params![project.0, directory, base],
            )?,
            None => self.connection.execute(
                "DELETE FROM bases WHERE project = ?1 AND directory = ?2",
                params![project.0, directory],
            )?,
        };
        Ok(())
    }

    /// Forgets `project`, its chats and what they showed, and the tabs
    /// of the stage of its folder, as when the human deletes it.
    ///
    /// # Errors
    ///
    /// When the database cannot be written, leaving it as it was.
    pub fn delete_project(&self, project: ProjectId) -> anyhow::Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let chats = "SELECT id FROM chats WHERE project = ?1";
        transaction.execute(
            &format!("DELETE FROM entries WHERE chat IN ({chats})"),
            [project.0],
        )?;
        transaction.execute(
            &format!("DELETE FROM turns WHERE chat IN ({chats})"),
            [project.0],
        )?;
        transaction.execute(
            &format!("DELETE FROM last_plans WHERE chat IN ({chats})"),
            [project.0],
        )?;
        transaction.execute(
            &format!("DELETE FROM steps WHERE chat IN ({chats})"),
            [project.0],
        )?;
        transaction.execute(
            &format!("DELETE FROM commands WHERE chat IN ({chats})"),
            [project.0],
        )?;
        // A chat of another project, or of Cox's own, that one of these
        // opened forgets which did.
        transaction.execute(
            &format!(
                "UPDATE chats SET parent = NULL
                 WHERE parent IN ({chats}) AND (project IS NOT ?1)"
            ),
            [project.0],
        )?;
        transaction.execute(
            "UPDATE chats SET parent = NULL WHERE project = ?1",
            [project.0],
        )?;
        transaction.execute("DELETE FROM chats WHERE project = ?1", [project.0])?;
        transaction.execute(
            "DELETE FROM stages WHERE folder = (SELECT folder FROM projects WHERE id = ?1)",
            [project.0],
        )?;
        transaction.execute("DELETE FROM shown WHERE project = ?1", [project.0])?;
        transaction.execute("DELETE FROM bases WHERE project = ?1", [project.0])?;
        transaction.execute("DELETE FROM projects WHERE id = ?1", [project.0])?;
        transaction.commit()?;
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
        if let Entry::ToolCall {
            command: true,
            title,
            result,
            ..
        } = entry
        {
            return self.keep_command(chat, number, entry, title, *result);
        }
        let Entry::Plan(steps) = entry else {
            self.connection
                .prepare_cached(KEEP_ENTRY)?
                .execute(params![
                    chat.0,
                    i64::try_from(number)?,
                    now(),
                    serde_json::to_string(entry)?
                ])?;
            return Ok(());
        };
        // A plan is kept with what is read of it in its place: as the
        // chat's last, and each of its steps.
        let transaction = self.connection.unchecked_transaction()?;
        let number = i64::try_from(number)?;
        transaction.prepare_cached(KEEP_ENTRY)?.execute(params![
            chat.0,
            number,
            now(),
            serde_json::to_string(entry)?
        ])?;
        transaction
            .prepare_cached(
                "INSERT INTO last_plans (chat, number, steps) VALUES (?1, ?2, ?3)
                 ON CONFLICT (chat) DO UPDATE SET number = excluded.number, steps = excluded.steps
                 WHERE excluded.number >= last_plans.number",
            )?
            .execute(params![chat.0, number, serde_json::to_string(steps)?])?;
        let mut step_kept = transaction.prepare_cached(
            "INSERT INTO steps (chat, id, text, started, done) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (chat, id) DO UPDATE SET text = excluded.text,
                 started = coalesce(excluded.started, steps.started), done = excluded.done",
        )?;
        for step in steps {
            // A step an older Cox kept is known by no id.
            let Some(id) = step.id else { continue };
            let done = step
                .since
                .filter(|_| step.state == crate::entry::StepState::Completed);
            step_kept.execute(params![
                chat.0,
                i64::try_from(id)?,
                step.text,
                step.started,
                done
            ])?;
        }
        drop(step_kept);
        transaction.commit()?;
        Ok(())
    }

    /// Keeps `entry`, a command the agent runs, `line`, which ended as
    /// `result` once it did, as the `number`th of `chat`, and when it
    /// started and ended, and its step: it starts as the command it runs
    /// is first told, and again while it runs, as it is told anew, as an
    /// agent tells a placeholder first, and the command once it wrote it.
    fn keep_command(
        &self,
        chat: ChatId,
        number: usize,
        entry: &Entry,
        line: &str,
        result: Option<ToolCallResult>,
    ) -> anyhow::Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        let number = i64::try_from(number)?;
        let at = now();
        let result = result
            .map(serde_json::to_value)
            .transpose()?
            .and_then(|result| result.as_str().map(str::to_owned));
        // Its step, of the entry before it that says something, read once.
        transaction
            .prepare_cached(
                r#"INSERT INTO commands (chat, number, step, started)
                 SELECT ?1, ?2, (SELECT number FROM entries WHERE chat = ?1 AND number < ?2
                     AND (entry LIKE '{"reply":%' OR entry LIKE '{"human":%'
                     OR entry LIKE '{"cox":%') ORDER BY number DESC LIMIT 1), ?3
                 WHERE NOT EXISTS (SELECT 1 FROM commands WHERE chat = ?1 AND number = ?2)"#,
            )?
            .execute(params![chat.0, number, at])?;
        // Read against the entry as it was kept before.
        transaction
            .prepare_cached(
                r"UPDATE commands SET
                 started = CASE WHEN ended IS NULL AND ?5 IS NULL AND ?4 IS NOT
                     (SELECT entry ->> '$.tool_call.title' FROM entries
                      WHERE chat = ?1 AND number = ?2) THEN ?3 ELSE started END,
                 ended = CASE WHEN ended IS NULL AND ?5 IS NOT NULL THEN ?3 ELSE ended END,
                 result = coalesce(?5, result)
                 WHERE chat = ?1 AND number = ?2",
            )?
            .execute(params![chat.0, number, at, line, result])?;
        transaction.prepare_cached(KEEP_ENTRY)?.execute(params![
            chat.0,
            number,
            at,
            serde_json::to_string(entry)?
        ])?;
        transaction.commit()?;
        Ok(())
    }

    /// The commands `chat`'s agent ran that ran `long` milliseconds or
    /// longer, as coxd saw them start and end, or failed, by the steps
    /// they are of, in order: each with how many commands the step ran,
    /// and what the agent said before them, if it said anything since the
    /// line it answers. A command that runs yet is not one of them, nor is
    /// one an older Cox kept that did not fail, whose end it did not keep.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn ran(&self, chat: ChatId, long: i64) -> anyhow::Result<Vec<KeptRun>> {
        // How many each step ran, of the commands' own table, which reads
        // no entry; the entries only of those that ran long or failed.
        let mut statement = self
            .connection
            .prepare_cached("SELECT step, count(*) FROM commands WHERE chat = ?1 GROUP BY step")?;
        let counts: Vec<(Option<i64>, i64)> = statement
            .query_map([chat.0], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut statement = self.connection.prepare_cached(
            "SELECT commands.number, commands.step, commands.started, commands.ended,
             entries.entry
             FROM commands JOIN entries
                 ON entries.chat = commands.chat AND entries.number = commands.number
             WHERE commands.chat = ?1 AND commands.result IS NOT NULL
                 AND (commands.result = 'failed' OR commands.ended - commands.started >= ?2)
             ORDER BY commands.number",
        )?;
        let rows = statement.query_map(params![chat.0, long], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut said = self.connection.prepare_cached(
            r#"SELECT entry FROM entries WHERE chat = ?1 AND number = ?2
             AND entry LIKE '{"reply":%'"#,
        )?;
        let mut runs: Vec<(Option<i64>, KeptRun)> = Vec::new();
        for row in rows {
            let (number, step, started, ended, entry) = row?;
            // An entry this Cox cannot read tells nothing.
            let Ok(Entry::ToolCall {
                title,
                about,
                content,
                result: Some(result),
                ..
            }) = serde_json::from_str::<Entry>(&entry)
            else {
                continue;
            };
            let command = KeptCommand {
                number: usize::try_from(number)?,
                line: title,
                about,
                content,
                result,
                started,
                ended,
            };
            if let Some((_, run)) = runs.last_mut().filter(|(of, _)| *of == step) {
                run.commands.push(command);
                continue;
            }
            let said = match step {
                Some(step) => said
                    .query_row(params![chat.0, step], |row| row.get::<_, String>(0))
                    .optional()?
                    .and_then(|entry| match serde_json::from_str(&entry) {
                        Ok(Entry::Reply(text)) => Some(text),
                        _ => None,
                    }),
                None => None,
            };
            let of = counts
                .iter()
                .find(|(of, _)| *of == step)
                .map_or(Ok(1), |(_, count)| usize::try_from(*count))?;
            runs.push((
                step,
                KeptRun {
                    said,
                    of,
                    commands: vec![command],
                },
            ));
        }
        Ok(runs.into_iter().map(|(_, run)| run).collect())
    }

    /// The `number`th entry of `chat`, from 0, if it keeps one this Cox
    /// can read.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn entry(&self, chat: ChatId, number: usize) -> anyhow::Result<Option<Entry>> {
        let entry: Option<String> = self
            .connection
            .query_row(
                "SELECT entry FROM entries WHERE chat = ?1 AND number = ?2",
                params![chat.0, i64::try_from(number)?],
                |row| row.get(0),
            )
            .optional()?;
        Ok(entry.and_then(|entry| serde_json::from_str(&entry).ok()))
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

    /// Where the agent of `chat`'s session runs subagents, as kept, if
    /// it was.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn subagents(&self, chat: ChatId) -> anyhow::Result<Option<String>> {
        Ok(self.connection.query_row(
            "SELECT subagents FROM chats WHERE id = ?1",
            [chat.0],
            |row| row.get(0),
        )?)
    }

    /// Keeps `subagents` as where the agent of `chat`'s session runs
    /// subagents, for the session it starts.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_subagents(&self, chat: ChatId, subagents: &str) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET subagents = ?2 WHERE id = ?1",
            params![chat.0, subagents],
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
            "UPDATE chats SET agent = ?2, session = NULL, settings = NULL, commands = NULL WHERE id = ?1",
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

    /// The commands `chat`'s agent runs when a line starts with `/` and
    /// one of their names, as it last told them: none until it told any.
    /// They are kept as `T` is written as JSON, whatever it is.
    ///
    /// # Errors
    ///
    /// When the database cannot be read, or holds commands that are not
    /// `T`s.
    pub fn commands<T: serde::de::DeserializeOwned>(&self, chat: ChatId) -> anyhow::Result<Vec<T>> {
        let commands: Option<String> = self.connection.query_row(
            "SELECT commands FROM chats WHERE id = ?1",
            [chat.0],
            |row| row.get(0),
        )?;
        Ok(commands
            .map(|commands| serde_json::from_str(&commands))
            .transpose()?
            .unwrap_or_default())
    }

    /// Keeps `commands` as those `chat`'s agent runs when a line starts
    /// with `/` and one of their names.
    ///
    /// # Errors
    ///
    /// When the database cannot be written, or `commands` cannot be
    /// written as JSON.
    pub fn keep_commands<T: serde::Serialize>(
        &self,
        chat: ChatId,
        commands: &[T],
    ) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET commands = ?2 WHERE id = ?1",
            params![chat.0, serde_json::to_string(commands)?],
        )?;
        Ok(())
    }

    /// Keeps `spent` as what `chat`'s agent spent, as `T` is written as
    /// JSON, whatever it is: [`KeptChat::spent`] reads it.
    ///
    /// # Errors
    ///
    /// When the database cannot be written, or `spent` cannot be written
    /// as JSON.
    pub fn keep_spent<T: serde::Serialize>(&self, chat: ChatId, spent: &T) -> anyhow::Result<()> {
        self.connection.execute(
            "UPDATE chats SET spent = ?2 WHERE id = ?1",
            params![chat.0, serde_json::to_string(spent)?],
        )?;
        Ok(())
    }

    /// Keeps that a turn of `chat`'s agent ended, as `end` says.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_turn_end(&self, chat: ChatId, end: TurnEnd) -> anyhow::Result<()> {
        self.connection.execute(
            "INSERT INTO turns (chat, at, how) VALUES (?1, ?2, ?3)",
            params![chat.0, end.at, end.how.word()],
        )?;
        Ok(())
    }

    /// How each turn of `chat`'s agent ended, the first first, of those
    /// a Cox that kept them saw end.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn turns(&self, chat: ChatId) -> anyhow::Result<Vec<TurnEnd>> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT at, how FROM turns WHERE chat = ?1 ORDER BY at, rowid")?;
        let ends = statement.query_map([chat.0], |row| {
            Ok(TurnEnd {
                at: row.get(0)?,
                how: Ended::from_word(&row.get::<_, String>(1)?),
            })
        })?;
        Ok(ends.collect::<Result<_, _>>()?)
    }

    /// The plan each chat's agent sent last, for the chats whose agent
    /// sent one.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn plans(&self) -> anyhow::Result<Vec<KeptPlan>> {
        // Kept as each plan is, rather than looked for among the entries.
        let mut statement = self.connection.prepare(
            "SELECT chat, number, steps,
             (SELECT at FROM entries WHERE entries.chat = last_plans.chat
              ORDER BY number DESC LIMIT 1)
             FROM last_plans ORDER BY chat",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                ChatId(row.get(0)?),
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get(3)?,
            ))
        })?;
        let mut plans = Vec::new();
        for row in rows {
            let (chat, number, steps, at) = row?;
            if let Ok(steps) = serde_json::from_str(&steps) {
                let number = usize::try_from(number)?;
                plans.push(KeptPlan {
                    chat,
                    number,
                    steps,
                    at,
                });
            }
        }
        Ok(plans)
    }

    /// What moved in the chats since `since`, in milliseconds since the
    /// epoch: the chats that wrote an entry, the steps of their plans
    /// done, as coxd stamped them, and the chats marked done.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn progress(&self, since: i64) -> anyhow::Result<Progress> {
        let mut moved = Progress::default();
        let mut statement = self.connection.prepare(
            "SELECT id, (SELECT at FROM entries WHERE entries.chat = chats.id
              ORDER BY number DESC LIMIT 1) AS last
             FROM chats WHERE last >= ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([since], |row| Ok((ChatId(row.get(0)?), row.get(1)?)))?;
        moved.active = rows.collect::<Result<_, _>>()?;
        // The steps done since, as each plan was kept, rather than every
        // plan the chats kept: each once, by its id, however the agent
        // reworded it, as it reads last.
        let mut statement = self.connection.prepare(
            "SELECT chat, text, done FROM steps INDEXED BY steps_done
             WHERE done IS NOT NULL AND done >= ?1 ORDER BY chat, done, id",
        )?;
        let rows = statement.query_map([since], |row| {
            Ok((ChatId(row.get(0)?), row.get::<_, String>(1)?, row.get(2)?))
        })?;
        for row in rows {
            let (chat, text, at) = row?;
            if moved
                .active
                .binary_search_by_key(&chat.0, |(active, _)| active.0)
                .is_ok()
            {
                moved.done.push((chat, text, at));
            }
        }
        let mut statement = self.connection.prepare(
            "SELECT id, resolved_at, resolved_by FROM chats WHERE resolved_at >= ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([since], |row| {
            let by: Option<String> = row.get(2)?;
            Ok((
                ChatId(row.get(0)?),
                Resolved {
                    at: row.get(1)?,
                    by: ResolvedBy::from_word(by.as_deref().unwrap_or("")),
                },
            ))
        })?;
        moved.resolved = rows.collect::<Result<_, _>>()?;
        Ok(moved)
    }

    /// When each chat was opened, in milliseconds since the epoch.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn opened(&self) -> anyhow::Result<Vec<(ChatId, i64)>> {
        let mut statement = self
            .connection
            .prepare_cached("SELECT id, created FROM chats ORDER BY id")?;
        let rows = statement.query_map([], |row| Ok((ChatId(row.get(0)?), row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// What `chat`'s story is told by, past its steps (see
    /// [`Self::steps`]), each entry with when it was first written, in
    /// milliseconds since the epoch, in order: the first line the human
    /// sent, and the receipts of how its agent's plan changed, the
    /// questions and the commits of its agent, and what ran as a turn
    /// ended or as something else happened (see [`Entry::checks`]).
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn milestones(&self, chat: ChatId) -> anyhow::Result<Vec<(i64, Entry)>> {
        // The WHERE is the milestones index's, word for word, for SQLite
        // to read that index rather than every entry of the chat.
        let mut statement = self.connection.prepare_cached(
            r#"SELECT at, entry FROM entries INDEXED BY milestones WHERE chat = ?1
             AND (entry LIKE '{"human":%' OR entry LIKE '{"plan_revised":%'
             OR entry LIKE '{"question":%' OR entry LIKE '{"commits":%'
             OR entry LIKE '{"checks":%' OR entry LIKE '{"event":%')
             ORDER BY number"#,
        )?;
        let rows = statement.query_map([chat.0], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut read = Vec::new();
        let mut first = true;
        for row in rows {
            let (at, entry) = row?;
            // An entry this Cox cannot read tells nothing.
            let Ok(entry) = serde_json::from_str::<Entry>(&entry) else {
                continue;
            };
            // Of the human's lines, the first tells the story's start.
            if matches!(entry, Entry::Human(_)) && !std::mem::take(&mut first) {
                continue;
            }
            read.push((at, entry));
        }
        Ok(read)
    }

    /// Each step the plans of `chat`'s agent had, in the order they first
    /// came, as the last plan that had it says.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn steps(&self, chat: ChatId) -> anyhow::Result<Vec<KeptStep>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT id, text, started, done FROM steps WHERE chat = ?1 ORDER BY id",
        )?;
        let rows = statement.query_map([chat.0], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                KeptStep {
                    id: 0,
                    text: row.get(1)?,
                    started: row.get(2)?,
                    done: row.get(3)?,
                },
            ))
        })?;
        let mut steps = Vec::new();
        for row in rows {
            let (id, step) = row?;
            steps.push(KeptStep {
                id: u64::try_from(id)?,
                ..step
            });
        }
        Ok(steps)
    }

    /// How many lines the human wrote on each step of the plans of
    /// `chat`'s agent, by the step's id, for the steps they wrote on.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn on_steps(&self, chat: ChatId) -> anyhow::Result<Vec<(u64, usize)>> {
        // The WHERE is the on_steps index's, word for word, for SQLite to
        // read that index rather than every entry of the chat.
        let mut statement = self.connection.prepare_cached(
            r#"SELECT entry FROM entries INDEXED BY on_steps
             WHERE chat = ?1 AND entry LIKE '{"on_step":%' ORDER BY number"#,
        )?;
        let rows = statement.query_map([chat.0], |row| row.get::<_, String>(0))?;
        let mut counts: Vec<(u64, usize)> = Vec::new();
        for entry in rows {
            let Ok(Entry::OnStep(on)) = serde_json::from_str(&entry?) else {
                continue;
            };
            match counts.iter_mut().find(|(id, _)| *id == on.id) {
                Some((_, count)) => *count += 1,
                None => counts.push((on.id, 1)),
            }
        }
        Ok(counts)
    }

    /// The thread of the step of the plans of `chat`'s agent it knows by
    /// `step`: each line the human wrote on it, and what the agent
    /// replied to it in the turn it went into, in order.
    ///
    /// # Errors
    ///
    /// When the database cannot be read.
    pub fn thread(&self, chat: ChatId, step: u64) -> anyhow::Result<Vec<Said>> {
        let mut statement = self.connection.prepare_cached(
            r#"SELECT number, entry FROM entries INDEXED BY on_steps
             WHERE chat = ?1 AND entry LIKE '{"on_step":%' ORDER BY number"#,
        )?;
        let on: Vec<i64> = statement
            .query_map([chat.0], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|row| {
                let (number, entry) = row.ok()?;
                match serde_json::from_str(&entry) {
                    Ok(Entry::OnStep(on)) if on.id == step => Some(number),
                    _ => None,
                }
            })
            .collect();
        let mut line = self.connection.prepare_cached(
            r#"SELECT number, at, entry FROM entries
             WHERE chat = ?1 AND number < ?2 AND entry LIKE '{"human":%'
             ORDER BY number DESC LIMIT 1"#,
        )?;
        let mut after = self.connection.prepare_cached(
            "SELECT number, at, entry FROM entries WHERE chat = ?1 AND number > ?2 ORDER BY number",
        )?;
        let mut thread = Vec::new();
        for number in on {
            let wrote = line
                .query_row(params![chat.0, number], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .optional()?;
            if let Some((number, at, entry)) = wrote
                && let Ok(Entry::Human(text)) = serde_json::from_str(&entry)
            {
                thread.push(Said {
                    by: Sayer::Human,
                    text,
                    at,
                    number: usize::try_from(number)?,
                });
            }
            let mut replied: Option<Said> = None;
            let mut rows = after.query(params![chat.0, number])?;
            while let Some(row) = rows.next()? {
                let entry: String = row.get(2)?;
                match serde_json::from_str(&entry) {
                    Ok(Entry::Human(_) | Entry::Cox(_)) => break,
                    Ok(Entry::Reply(text)) => match &mut replied {
                        Some(said) => {
                            said.text.push_str("\n\n");
                            said.text.push_str(&text);
                        }
                        None => {
                            replied = Some(Said {
                                by: Sayer::Agent,
                                text,
                                at: row.get(1)?,
                                number: usize::try_from(row.get::<_, i64>(0)?)?,
                            });
                        }
                    },
                    _ => {}
                }
            }
            thread.extend(replied);
        }
        Ok(thread)
    }

    /// The steps of the plan `chat`'s agent first sent after the human's
    /// last line, once it sent one.
    ///
    /// # Errors
    ///
    /// When the database cannot be read, or holds steps this Cox cannot.
    pub fn baseline(&self, chat: ChatId) -> anyhow::Result<Option<Vec<String>>> {
        let baseline: Option<String> = self.connection.query_row(
            "SELECT baseline FROM chats WHERE id = ?1",
            [chat.0],
            |row| row.get(0),
        )?;
        Ok(baseline
            .map(|baseline| serde_json::from_str(&baseline))
            .transpose()?)
    }

    /// Keeps `baseline` as the steps of the plan `chat`'s agent first sent
    /// after the human's last line, or none.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn keep_baseline(&self, chat: ChatId, baseline: Option<&[String]>) -> anyhow::Result<()> {
        let baseline = baseline.map(serde_json::to_string).transpose()?;
        self.connection.execute(
            "UPDATE chats SET baseline = ?2 WHERE id = ?1",
            params![chat.0, baseline],
        )?;
        Ok(())
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

    /// Keeps `baseline` as the steps of the plan the chat's agent first
    /// sent after the human's last line, or none.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn baseline(&self, baseline: Option<&[String]>) -> anyhow::Result<()> {
        self.store.keep_baseline(self.chat, baseline)
    }

    /// Keeps `commands` as those the chat's agent runs when a line starts
    /// with `/` and one of their names, as [`Store::keep_commands`] does.
    ///
    /// # Errors
    ///
    /// When the database cannot be written, or `commands` cannot be
    /// written as JSON.
    pub fn commands<T: serde::Serialize>(&self, commands: &[T]) -> anyhow::Result<()> {
        self.store.keep_commands(self.chat, commands)
    }

    /// Keeps `spent` as what the chat's agent spent, as
    /// [`Store::keep_spent`] does.
    ///
    /// # Errors
    ///
    /// When the database cannot be written, or `spent` cannot be written
    /// as JSON.
    pub fn spent<T: serde::Serialize>(&self, spent: &T) -> anyhow::Result<()> {
        self.store.keep_spent(self.chat, spent)
    }

    /// Keeps that a turn of the chat's agent ended, as
    /// [`Store::keep_turn_end`] does.
    ///
    /// # Errors
    ///
    /// When the database cannot be written.
    pub fn turn_ended(&self, end: TurnEnd) -> anyhow::Result<()> {
        self.store.keep_turn_end(self.chat, end)
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

/// The time now, in milliseconds since the epoch, as the store keeps
/// times.
#[must_use]
pub fn now() -> i64 {
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

    /// A folder of its own for a test, removed once dropped.
    fn scratch() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
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
                    archived: false,
                    about: None,
                },
                KeptProject {
                    id: notes,
                    folder: "/notes".into(),
                    name: "notes".to_owned(),
                    archived: false,
                    about: None,
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
    fn keeps_whether_a_chat_is_resolved_and_by_whom_past_closing() {
        let directory = scratch();
        let path = directory.path().join("chats.sqlite");
        let resolved = |path: &Path| {
            Store::open(path)
                .unwrap()
                .chats()
                .unwrap()
                .into_iter()
                .map(|chat| chat.resolved)
                .collect::<Vec<_>>()
        };
        let (chat, other) = {
            let store = Store::open(&path).unwrap();
            (
                store.add_chat(None, "Chat 1").unwrap(),
                store.add_chat(None, "Chat 2").unwrap(),
            )
        };
        assert_eq!(resolved(&path), [None, None]);

        let by_human = Resolved {
            at: 1_000,
            by: ResolvedBy::Human,
        };
        let by_agent = Resolved {
            at: 2_000,
            by: ResolvedBy::Agent,
        };
        {
            let store = Store::open(&path).unwrap();
            store.resolve_chat(chat, Some(by_human)).unwrap();
            store.resolve_chat(chat, Some(by_human)).unwrap();
            store.resolve_chat(other, Some(by_agent)).unwrap();
        }
        assert_eq!(resolved(&path), [Some(by_human), Some(by_agent)]);

        Store::open(&path)
            .unwrap()
            .resolve_chat(other, None)
            .unwrap();
        assert_eq!(resolved(&path), [Some(by_human), None]);
    }

    #[test]
    fn keeps_the_chat_shown_last_of_each_project_and_the_project_chosen_last() {
        let folder = scratch();
        let path = folder.path().join("chats.sqlite");
        let (cox, second, own) = {
            let store = Store::open(&path).unwrap();
            let cox = store.project(Path::new("/src/cox"), "cox").unwrap();
            let first = store.add_chat(Some(cox), "Chat 1").unwrap();
            let second = store.add_chat(Some(cox), "Chat 2").unwrap();
            let own = store.add_chat(None, "Chat 1").unwrap();
            assert_eq!(store.shown().unwrap(), []);
            store.keep_shown(Some(cox), Some(first)).unwrap();
            store.keep_shown(Some(cox), Some(second)).unwrap();
            store.keep_shown(None, Some(own)).unwrap();
            // Choosing the project again with no chat shown keeps the
            // chat of it shown before.
            store.keep_shown(Some(cox), None).unwrap();
            (cox, second, own)
        };
        let shown: Vec<_> = Store::open(&path)
            .unwrap()
            .shown()
            .unwrap()
            .into_iter()
            .map(|shown| (shown.project, shown.chat))
            .collect();
        assert_eq!(shown, [(None, Some(own)), (Some(cox), Some(second))]);
    }

    #[test]
    fn keeps_how_and_when_each_turn_ended_until_its_project_goes() {
        let store = Store::in_memory().unwrap();
        let app = store.project(Path::new("/src/app"), "app").unwrap();
        let chat = store.add_chat(Some(app), "Chat 1").unwrap();
        let other = store.add_chat(None, "Chat 2").unwrap();
        let stopped = TurnEnd {
            at: 20,
            how: Ended::Stopped,
        };
        let done = TurnEnd {
            at: 10,
            how: Ended::Done,
        };
        store.keep_turn_end(chat, stopped).unwrap();
        store.keep_turn_end(chat, done).unwrap();
        store
            .keep_turn_end(
                other,
                TurnEnd {
                    at: 15,
                    how: Ended::Failed,
                },
            )
            .unwrap();

        assert_eq!(store.turns(chat).unwrap(), [done, stopped]);
        assert_eq!(Ended::from_word("what a newer Cox says"), Ended::Other);

        store.delete_project(app).unwrap();
        assert_eq!(store.turns(chat).unwrap(), []);
        assert_eq!(store.turns(other).unwrap().len(), 1);
    }

    #[test]
    fn a_project_deleted_forgets_the_chat_shown_of_it() {
        let store = Store::in_memory().unwrap();
        let app = store.project(Path::new("/src/app"), "app").unwrap();
        let chat = store.add_chat(Some(app), "Chat 1").unwrap();
        store.keep_shown(Some(app), Some(chat)).unwrap();
        store.keep_shown(None, None).unwrap();

        store.delete_project(app).unwrap();

        let shown: Vec<_> = store
            .shown()
            .unwrap()
            .into_iter()
            .map(|shown| (shown.project, shown.chat))
            .collect();
        assert_eq!(shown, [(None, None)]);
    }

    #[test]
    fn a_chat_last_moved_when_its_last_entry_was_written_or_else_when_opened() {
        let store = Store::in_memory().unwrap();
        let quiet = store.add_chat(None, "Chat 1").unwrap();
        let said = store.add_chat(None, "Chat 2").unwrap();
        store
            .keep_entry(said, 0, &Entry::Human("first".to_owned()))
            .unwrap();
        store
            .connection
            .execute("UPDATE entries SET at = 5000 WHERE chat = ?1", [said.0])
            .unwrap();
        store
            .connection
            .execute("UPDATE chats SET created = 1000", [])
            .unwrap();

        let active: Vec<_> = store
            .chats()
            .unwrap()
            .into_iter()
            .map(|chat| (chat.id, chat.active))
            .collect();
        assert_eq!(active, [(quiet, 1000), (said, 5000)]);
    }

    #[test]
    fn keeps_each_entry_as_it_last_was() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let other = store.add_chat(None, "Chat 2").unwrap();
        let call = |result| Entry::ToolCall {
            id: "call-1".to_owned(),
            title: "Edit x.txt".to_owned(),
            command: false,
            about: None,
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
                    since: None,
                    started: None,
                    id: None,
                },
                Step {
                    text: "Fix it".to_owned(),
                    state: StepState::InProgress,
                    since: None,
                    started: None,
                    id: None,
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
    fn a_project_is_archived_and_deleted_with_its_chats() {
        let store = Store::in_memory().unwrap();
        let app = store.project(Path::new("/work/app"), "app").unwrap();
        let notes = store.project(Path::new("/work/notes"), "notes").unwrap();
        let opener = store.add_chat(Some(app), "Chat 1").unwrap();
        store
            .add_chat_under(opener, Some(app), "Chat 2", None)
            .unwrap();
        let elsewhere = store
            .add_chat_under(opener, Some(notes), "Chat 1", None)
            .unwrap();
        store
            .keep_entry(opener, 0, &Entry::Human("hello".to_owned()))
            .unwrap();
        store
            .keep_stage(Path::new("/work/app"), &KeptTabs::default())
            .unwrap();

        store.archive_project(app, true).unwrap();
        let archived = |store: &Store| {
            store
                .projects()
                .unwrap()
                .into_iter()
                .map(|project| (project.name, project.archived))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            archived(&store),
            [("app".to_owned(), true), ("notes".to_owned(), false)]
        );
        store.archive_project(app, false).unwrap();
        assert_eq!(
            archived(&store),
            [("app".to_owned(), false), ("notes".to_owned(), false)]
        );

        store.delete_project(app).unwrap();
        assert_eq!(archived(&store), [("notes".to_owned(), false)]);
        let chats = store.chats().unwrap();
        assert_eq!(
            chats
                .iter()
                .map(|chat| (chat.id, chat.parent))
                .collect::<Vec<_>>(),
            [(elsewhere, None)]
        );
        assert_eq!(store.entries(opener).unwrap(), []);
        assert_eq!(store.stage(Path::new("/work/app")).unwrap(), None);
        // Its folder can be a project again.
        assert_ne!(store.project(Path::new("/work/app"), "app").unwrap(), app);
    }

    #[test]
    fn keeps_what_a_project_is() {
        let store = Store::in_memory().unwrap();
        let app = store.project(Path::new("/work/app"), "app").unwrap();
        let about = |store: &Store| store.projects().unwrap()[0].about.clone();
        assert_eq!(about(&store), None);
        store
            .keep_about(app, Some("A shop that sells tea."))
            .unwrap();
        assert_eq!(about(&store).as_deref(), Some("A shop that sells tea."));
        // It moves with the project's folder.
        store
            .move_project(app, Path::new("/work/tea"), "tea")
            .unwrap();
        assert_eq!(about(&store).as_deref(), Some("A shop that sells tea."));
        store.keep_about(app, None).unwrap();
        assert_eq!(about(&store), None);
    }

    #[test]
    fn keeps_the_branch_each_directory_of_a_project_is_compared_with() {
        let store = Store::in_memory().unwrap();
        let app = store.project(Path::new("/work/app"), "app").unwrap();
        let shop = store.project(Path::new("/work/shop"), "shop").unwrap();
        let bases = |store: &Store, folder: &str| store.bases(Path::new(folder)).unwrap();
        store.keep_base(app, "web", Some("origin/main")).unwrap();
        store.keep_base(app, "web", Some("origin/release")).unwrap();
        store.keep_base(app, "", Some("develop")).unwrap();
        store.keep_base(shop, "web", Some("main")).unwrap();

        assert_eq!(
            bases(&store, "/work/app"),
            [
                (String::new(), "develop".to_owned()),
                ("web".to_owned(), "origin/release".to_owned()),
            ]
        );
        assert_eq!(bases(&store, "/elsewhere"), []);
        // They move with the project's folder.
        store
            .move_project(app, Path::new("/work/tea"), "tea")
            .unwrap();
        assert_eq!(bases(&store, "/work/tea").len(), 2);
        store.keep_base(app, "", None).unwrap();
        assert_eq!(
            bases(&store, "/work/tea"),
            [("web".to_owned(), "origin/release".to_owned())]
        );
        // And go with it.
        store.delete_project(app).unwrap();
        assert_eq!(bases(&store, "/work/tea"), []);
        assert_eq!(bases(&store, "/work/shop").len(), 1);
    }

    #[test]
    fn the_tabs_of_a_stage_follow_a_folder_in_it_that_moved() {
        let store = Store::in_memory().unwrap();
        let folder = Path::new("/work/app");
        let tabs = |path: &str| KeptTabs {
            tabs: vec![
                KeptTab::File {
                    path: path.into(),
                    lines: None,
                },
                KeptTab::Markdown {
                    path: "/work/app/directories/lib/plan.md".into(),
                },
            ],
            shown: 0,
            owners: vec![None, None],
        };
        store
            .keep_stage(folder, &tabs("/work/app/directories/app/src/main.rs"))
            .unwrap();
        let moved = Moved {
            from: Path::new("/work/app/directories/app"),
            to: Path::new("/work/app/directories/shop"),
        };
        store.move_in_stage(folder, moved).unwrap();
        assert_eq!(
            store.stage(folder).unwrap(),
            Some(tabs("/work/app/directories/shop/src/main.rs"))
        );
        // A folder with no stage kept has none still.
        store.move_in_stage(Path::new("/elsewhere"), moved).unwrap();
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
    fn keeps_where_a_chats_session_runs_subagents() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        assert_eq!(store.subagents(chat).unwrap(), None);

        store.keep_subagents(chat, "chats").unwrap();
        assert_eq!(store.subagents(chat).unwrap().as_deref(), Some("chats"));
    }

    #[test]
    fn reads_the_last_plan_of_each_chat_as_it_was_kept_with_it() {
        let store = Store::in_memory().unwrap();
        let planned = store.add_chat(None, "Chat 1").unwrap();
        let quiet = store.add_chat(None, "Chat 2").unwrap();
        let step = |text: &str, state| Step {
            text: text.to_owned(),
            state,
            since: None,
            started: None,
            id: None,
        };
        let first = vec![step("Read", StepState::Completed)];
        let last = vec![
            step("Read", StepState::Completed),
            step("Fix", StepState::InProgress),
        ];
        let entries = [
            Entry::Human("plan it".to_owned()),
            Entry::Plan(first),
            Entry::Reply("done".to_owned()),
            Entry::Human("again".to_owned()),
            Entry::Plan(last.clone()),
            Entry::Reply("on it".to_owned()),
        ];
        for (number, entry) in entries.iter().enumerate() {
            store.keep_entry(planned, number, entry).unwrap();
        }
        store
            .keep_entry(quiet, 0, &Entry::Human("hi".to_owned()))
            .unwrap();

        let plans = store.plans().unwrap();
        assert_eq!(plans.len(), 1);
        assert_eq!(
            (plans[0].chat, plans[0].number, &plans[0].steps),
            (planned, 4, &last)
        );
        // A plan before it kept again does not take its place.
        store
            .keep_entry(planned, 1, &Entry::Plan(Vec::new()))
            .unwrap();
        assert_eq!(store.plans().unwrap()[0].number, 4);
    }

    #[test]
    fn reads_each_step_of_a_chats_plans_as_the_last_plan_that_had_it_says() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let other = store.add_chat(None, "Chat 2").unwrap();
        let step = |id: u64, text: &str, state, started: Option<i64>, since: i64| Step {
            text: text.to_owned(),
            state,
            since: Some(since),
            started,
            id: Some(id),
        };
        let plans = [
            Entry::Plan(vec![
                step(1, "Read", StepState::InProgress, Some(10), 10),
                step(2, "Fix", StepState::Pending, None, 10),
            ]),
            Entry::Human("again".to_owned()),
            Entry::Plan(vec![
                // Its start is kept once a plan tells none.
                step(1, "Read the code", StepState::Completed, None, 20),
                step(3, "Lint", StepState::Completed, None, 20),
            ]),
        ];
        for (number, entry) in plans.iter().enumerate() {
            store.keep_entry(chat, number, entry).unwrap();
        }
        store
            .keep_entry(
                other,
                0,
                &Entry::Plan(vec![step(1, "Elsewhere", StepState::Pending, None, 5)]),
            )
            .unwrap();
        // The plan of the turn, kept again as it goes: it reads as it
        // stands.
        store
            .keep_entry(
                chat,
                2,
                &Entry::Plan(vec![
                    step(1, "Read the code", StepState::Completed, None, 20),
                    step(3, "Lint", StepState::InProgress, Some(30), 30),
                ]),
            )
            .unwrap();

        let steps = store.steps(chat).unwrap();

        assert_eq!(
            steps,
            [
                KeptStep {
                    id: 1,
                    text: "Read the code".to_owned(),
                    started: Some(10),
                    done: Some(20),
                },
                // Gone from the last plan, as the plan before it had it.
                KeptStep {
                    id: 2,
                    text: "Fix".to_owned(),
                    started: None,
                    done: None,
                },
                KeptStep {
                    id: 3,
                    text: "Lint".to_owned(),
                    started: Some(30),
                    done: None,
                },
            ]
        );
    }

    #[test]
    fn reads_what_moved_since_a_time() {
        let store = Store::in_memory().unwrap();
        let busy = store.add_chat(None, "Chat 1").unwrap();
        let quiet = store.add_chat(None, "Chat 2").unwrap();
        let reported = store.add_chat(None, "Chat 3").unwrap();
        let step = |text: &str, state, since| Step {
            text: text.to_owned(),
            state,
            since,
            started: None,
            id: None,
        };
        let known = |text: &str, state, since, id| Step {
            id: Some(id),
            ..step(text, state, since)
        };
        let since = now();
        // Done before, in a turn before, and kept so in the next.
        let read = known("Read", StepState::Completed, Some(since - 60_000), 1);
        let entries = [
            Entry::Plan(vec![
                read.clone(),
                known("Fix", StepState::InProgress, Some(since), 2),
            ]),
            Entry::Human("again".to_owned()),
            Entry::Plan(vec![
                read,
                // Done, then left out of the plans after: done still.
                known("Fix", StepState::Completed, Some(since + 1), 2),
                // Done as an older Cox kept it, with no id or stamp.
                step("Old", StepState::Completed, None),
                known("Test", StepState::InProgress, Some(since + 2), 4),
                known("Lint", StepState::Completed, Some(since + 2), 5),
            ]),
            Entry::Human("go on".to_owned()),
            Entry::Plan(vec![
                known("Test", StepState::Completed, Some(since + 3), 4),
                // Done before, and not now.
                known("Lint", StepState::InProgress, Some(since + 3), 5),
            ]),
            Entry::Human("reword it".to_owned()),
            // Reworded, done as it was: done once, as it reads now.
            Entry::Plan(vec![known(
                "Test the parser",
                StepState::Completed,
                Some(since + 3),
                4,
            )]),
            Entry::Reply("testing".to_owned()),
        ];
        for (number, entry) in entries.iter().enumerate() {
            store.keep_entry(busy, number, entry).unwrap();
        }
        store
            .connection
            .execute(
                "INSERT INTO entries (chat, number, at, entry) VALUES (?1, 0, ?2, '\"merge\"')",
                params![quiet.0, since - 60_000],
            )
            .unwrap();
        store
            .keep_entry(reported, 0, &Entry::Reply("done".to_owned()))
            .unwrap();
        store
            .resolve_chat(reported, Some(Resolved::now(ResolvedBy::Agent)))
            .unwrap();

        let moved = store.progress(since).unwrap();

        assert_eq!(
            moved
                .active
                .iter()
                .map(|(chat, _)| *chat)
                .collect::<Vec<_>>(),
            [busy, reported]
        );
        assert_eq!(
            moved.done,
            [
                (busy, "Fix".to_owned(), since + 1),
                (busy, "Test the parser".to_owned(), since + 3)
            ]
        );
        assert_eq!(
            moved
                .resolved
                .iter()
                .map(|(chat, resolved)| (*chat, resolved.by))
                .collect::<Vec<_>>(),
            [(reported, ResolvedBy::Agent)]
        );
        let plan: String = store
            .connection
            .query_row(
                "EXPLAIN QUERY PLAN SELECT chat FROM steps INDEXED BY steps_done
                 WHERE done IS NOT NULL AND done >= 1",
                [],
                |row| row.get(3),
            )
            .unwrap();
        assert!(plan.contains("steps_done"), "read by the index: {plan}");
    }

    #[test]
    fn reads_what_tells_a_chats_story_with_when_each_was_written() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let other = store.add_chat(None, "Chat 2").unwrap();
        let commits = Entry::Commits(Vec::new());
        let revised = Entry::PlanRevised(crate::entry::Revision::between(&[], &[], &[]));
        let check = |name: &str| crate::entry::Check {
            name: name.to_owned(),
            state: crate::entry::CheckState::Passed,
            said: None,
        };
        let checks = Entry::Checks(vec![check("goal")]);
        let event = Entry::Event {
            on: "goal.met".to_owned(),
            checks: vec![check("docs")],
        };
        let entries = [
            Entry::Note("instructions".to_owned()),
            Entry::Human("split the parser".to_owned()),
            // Its steps are read apart (see `Store::steps`).
            Entry::Plan(Vec::new()),
            revised.clone(),
            Entry::Reply("reading".to_owned()),
            Entry::Human("and the lexer".to_owned()),
            commits.clone(),
            checks.clone(),
            event.clone(),
        ];
        for (number, entry) in entries.iter().enumerate() {
            store.keep_entry(chat, number, entry).unwrap();
        }
        store
            .keep_entry(other, 0, &Entry::Human("elsewhere".to_owned()))
            .unwrap();

        let told: Vec<Entry> = store
            .milestones(chat)
            .unwrap()
            .into_iter()
            .map(|(at, entry)| {
                assert!(at > 0, "each with when");
                entry
            })
            .collect();
        assert_eq!(
            told,
            [
                Entry::Human("split the parser".to_owned()),
                revised,
                commits,
                checks,
                event
            ]
        );
        let plan: String = store
            .connection
            .query_row(
                r#"EXPLAIN QUERY PLAN SELECT number FROM entries INDEXED BY milestones
                 WHERE chat = 1 AND (entry LIKE '{"human":%' OR entry LIKE '{"plan_revised":%'
                 OR entry LIKE '{"question":%' OR entry LIKE '{"commits":%'
                 OR entry LIKE '{"checks":%' OR entry LIKE '{"event":%')"#,
                [],
                |row| row.get(3),
            )
            .unwrap();
        assert!(plan.contains("milestones"), "read by the index: {plan}");
    }

    /// A command the agent runs, `line`, as its entry keeps it, as the
    /// `id`th call, ended as `result`, once it did.
    fn command(id: &str, line: &str, result: Option<ToolCallResult>) -> Entry {
        Entry::ToolCall {
            id: id.to_owned(),
            title: line.to_owned(),
            command: true,
            about: None,
            content: vec![Content::Text(format!("ran {line}"))],
            result,
        }
    }

    /// When the command of the `number`th entry of `chat` started and
    /// ended, as kept.
    fn ran_at(store: &Store, chat: ChatId, number: i64) -> (i64, Option<i64>, Option<String>) {
        store
            .connection
            .query_row(
                "SELECT started, ended, result FROM commands WHERE chat = ?1 AND number = ?2",
                params![chat.0, number],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
    }

    #[test]
    fn a_command_starts_as_it_is_told_and_ends_as_it_ends() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let unstarted = |store: &Store| {
            store
                .connection
                .execute("UPDATE commands SET started = 0", [])
                .unwrap();
        };
        // The agent tells a placeholder first, as it writes the command.
        store
            .keep_entry(chat, 0, &command("1", "Terminal", None))
            .unwrap();
        assert_eq!(ran_at(&store, chat, 0).1, None);
        unstarted(&store);

        // The command, once written, starts it anew; told again as it
        // was, it goes on.
        store
            .keep_entry(chat, 0, &command("1", "cargo test", None))
            .unwrap();
        let started = ran_at(&store, chat, 0).0;
        assert!(started > 0, "started anew");
        unstarted(&store);
        store
            .keep_entry(chat, 0, &command("1", "cargo test", None))
            .unwrap();
        assert_eq!(ran_at(&store, chat, 0).0, 0, "went on");

        store
            .keep_entry(
                chat,
                0,
                &command("1", "cargo test", Some(ToolCallResult::Failed)),
            )
            .unwrap();
        let (started, ended, result) = ran_at(&store, chat, 0);
        assert_eq!(started, 0, "an end starts nothing");
        assert!(ended.is_some_and(|ended| ended > 0));
        assert_eq!(result.as_deref(), Some("failed"));

        // Told anew once it ended, it ended as it did.
        store
            .keep_entry(
                chat,
                0,
                &command("1", "cargo test -p a", Some(ToolCallResult::Failed)),
            )
            .unwrap();
        assert_eq!(ran_at(&store, chat, 0), (started, ended, result));
        assert_eq!(
            store.entry(chat, 0).unwrap(),
            Some(command(
                "1",
                "cargo test -p a",
                Some(ToolCallResult::Failed)
            ))
        );
    }

    #[test]
    fn reads_the_commands_that_ran_long_or_failed_by_their_steps() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let other = store.add_chat(None, "Chat 2").unwrap();
        let done = Some(ToolCallResult::Completed);
        let entries = [
            Entry::Human("fix it".to_owned()),
            Entry::Reply("Now I test.\n\nThen more.".to_owned()),
            command("1", "cargo test", done),
            Entry::Thought("hm".to_owned()),
            command("2", "ls", done),
            command("3", "false", Some(ToolCallResult::Failed)),
            Entry::Reply("Then I lint.".to_owned()),
            command("4", "cargo clippy", Some(ToolCallResult::Cancelled)),
            Entry::Human("again".to_owned()),
            command("5", "cargo build", done),
            command("6", "sleep 60", None),
        ];
        for (number, entry) in entries.iter().enumerate() {
            store.keep_entry(chat, number, entry).unwrap();
        }
        store
            .keep_entry(other, 0, &command("1", "cargo test", done))
            .unwrap();
        // How long each ran: 9 s, 1 s, 1 s, 30 s, 12 s.
        for (number, secs) in [(2, 9), (4, 1), (5, 1), (7, 30), (9, 12)] {
            store
                .connection
                .execute(
                    "UPDATE commands SET started = 1000, ended = 1000 + ?3 * 1000
                     WHERE chat = ?1 AND number = ?2",
                    params![chat.0, number, secs],
                )
                .unwrap();
        }

        let runs = store.ran(chat, 9_000).unwrap();

        let told: Vec<(Option<&str>, usize, Vec<&str>)> = runs
            .iter()
            .map(|run| {
                (
                    run.said.as_deref(),
                    run.of,
                    run.commands
                        .iter()
                        .map(|command| command.line.as_str())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            told,
            [
                (
                    Some("Now I test.\n\nThen more."),
                    3,
                    vec!["cargo test", "false"]
                ),
                (Some("Then I lint."), 1, vec!["cargo clippy"]),
                // After the human's line, before the agent said anything;
                // the one that runs yet is counted, not told.
                (None, 2, vec!["cargo build"]),
            ]
        );
        let test = &runs[0].commands[0];
        assert_eq!(
            (test.number, test.started, test.ended, test.result),
            (2, 1000, Some(10_000), ToolCallResult::Completed)
        );
        assert_eq!(test.content, [Content::Text("ran cargo test".to_owned())]);
        assert_eq!(runs[1].commands[0].result, ToolCallResult::Cancelled);
    }

    #[test]
    fn the_commands_kept_before_tell_how_they_failed_through_the_upgrade() {
        let directory = scratch();
        let path = directory.path().join("chats.sqlite");
        database_of_version(&path, 25);
        let entries = [
            Entry::Human("fix it".to_owned()),
            Entry::Reply("Now I test.".to_owned()),
            command("1", "cargo test", Some(ToolCallResult::Failed)),
            command("2", "cargo build", Some(ToolCallResult::Completed)),
            Entry::ToolCall {
                id: "3".to_owned(),
                title: "Read a.rs".to_owned(),
                command: false,
                about: None,
                content: Vec::new(),
                result: Some(ToolCallResult::Failed),
            },
            command("4", "sleep 60", None),
        ];
        {
            let connection = Connection::open(&path).unwrap();
            for (number, entry) in entries.iter().enumerate() {
                connection
                    .execute(
                        "INSERT INTO entries (chat, number, at, entry) VALUES (1, ?1, 7, ?2)",
                        params![
                            i64::try_from(number).unwrap(),
                            serde_json::to_string(entry).unwrap()
                        ],
                    )
                    .unwrap();
            }
        }

        let store = Store::open(&path).unwrap();

        let chat = store.chats().unwrap()[0].id;
        // No end was kept: of those that ended, the one that failed.
        let runs = store.ran(chat, 0).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].said.as_deref(), Some("Now I test."));
        assert_eq!(runs[0].of, 3);
        let failed = &runs[0].commands[0];
        assert_eq!(
            (failed.line.as_str(), failed.started, failed.ended),
            ("cargo test", 7, None)
        );
        // It goes on as a command kept now is, from where it was.
        store
            .keep_entry(
                chat,
                5,
                &command("4", "sleep 60", Some(ToolCallResult::Failed)),
            )
            .unwrap();
        assert_eq!(store.ran(chat, 0).unwrap()[0].commands.len(), 2);
    }

    #[test]
    fn reads_the_thread_of_a_step_the_human_wrote_on() {
        use crate::entry::OnStep;
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        let on = |id: u64, text: &str| {
            Entry::OnStep(OnStep {
                id,
                text: text.to_owned(),
            })
        };
        let entries = [
            Entry::Human("plan it".to_owned()),
            Entry::Reply("planned".to_owned()),
            Entry::Human("keep the old flag?".to_owned()),
            on(1, "Rename the setting"),
            Entry::Reply("Yes, kept.".to_owned()),
            Entry::Note("a note".to_owned()),
            Entry::Reply("Done.".to_owned()),
            Entry::Human("something else".to_owned()),
            Entry::Reply("other".to_owned()),
            Entry::Human("skip it".to_owned()),
            on(2, "Test"),
            Entry::Reply("Skipped.".to_owned()),
            Entry::Human("and say so".to_owned()),
            on(1, "Rename the setting, keeping the old"),
        ];
        for (number, entry) in entries.iter().enumerate() {
            store.keep_entry(chat, number, entry).unwrap();
        }

        assert_eq!(store.on_steps(chat).unwrap(), [(1, 2), (2, 1)]);
        let thread: Vec<(Sayer, String, usize)> = store
            .thread(chat, 1)
            .unwrap()
            .into_iter()
            .map(|said| (said.by, said.text, said.number))
            .collect();
        assert_eq!(
            thread,
            [
                (Sayer::Human, "keep the old flag?".to_owned(), 2),
                (Sayer::Agent, "Yes, kept.\n\nDone.".to_owned(), 4),
                (Sayer::Human, "and say so".to_owned(), 12),
            ]
        );
        assert_eq!(store.thread(chat, 3).unwrap(), []);
        let plan: String = store
            .connection
            .query_row(
                r#"EXPLAIN QUERY PLAN SELECT entry FROM entries INDEXED BY on_steps
                 WHERE chat = 1 AND entry LIKE '{"on_step":%' ORDER BY number"#,
                [],
                |row| row.get(3),
            )
            .unwrap();
        assert!(plan.contains("on_steps"), "read by the index: {plan}");
    }

    #[test]
    fn keeps_the_plan_a_chat_started_its_last_line_with() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        assert_eq!(store.baseline(chat).unwrap(), None);

        let steps = ["Read the code".to_owned(), "Fix the greeting".to_owned()];
        store.keep_baseline(chat, Some(&steps)).unwrap();
        assert_eq!(store.baseline(chat).unwrap().as_deref(), Some(&steps[..]));
        store.keep_baseline(chat, None).unwrap();
        assert_eq!(store.baseline(chat).unwrap(), None);
    }

    #[test]
    fn keeps_the_commands_of_a_chats_agent() {
        #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
        struct Command {
            name: String,
        }
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        assert_eq!(store.commands::<Command>(chat).unwrap(), []);

        let commands = [Command {
            name: "review".to_owned(),
        }];
        store.keep_commands(chat, &commands).unwrap();
        assert_eq!(store.commands::<Command>(chat).unwrap(), commands);
        store.keep_commands::<Command>(chat, &[]).unwrap();
        assert_eq!(store.commands::<Command>(chat).unwrap(), []);
    }

    #[test]
    fn keeps_what_a_chats_agent_spent_once_it_told() {
        let store = Store::in_memory().unwrap();
        let chat = store.add_chat(None, "Chat 1").unwrap();
        assert_eq!(store.chats().unwrap()[0].spent, None);

        store
            .keep_spent(chat, &serde_json::json!({ "session": 91_728 }))
            .unwrap();
        assert_eq!(
            store.chats().unwrap()[0].spent.as_deref(),
            Some(r#"{"session":91728}"#)
        );
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
        store.keep_commands(chat, &["review"]).unwrap();

        store.keep_agent(chat, "codex").unwrap();
        assert_eq!(store.agent(chat).unwrap().as_deref(), Some("codex"));
        assert_eq!(store.session(chat).unwrap(), None);
        assert_eq!(store.settings(chat).unwrap(), []);
        assert_eq!(
            store.commands::<String>(chat).unwrap(),
            Vec::<String>::new()
        );
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

    /// The tables as the first Cox to keep chats made them, at version 1.
    const FIRST_TABLES: &str = "
        CREATE TABLE projects (
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
        ) WITHOUT ROWID;";

    /// Makes the database at `path` as the Cox of `version` left it, with
    /// the project `/src/cox` and a chat in it.
    fn database_of_version(path: &Path, version: usize) {
        let connection = Connection::open(path).unwrap();
        connection.execute_batch(FIRST_TABLES).unwrap();
        for upgrade in &UPGRADES[..version - 1] {
            connection.execute_batch(upgrade).unwrap();
        }
        connection
            .execute_batch(&format!(
                "INSERT INTO projects (folder, name) VALUES ('/src/cox', 'cox');
                 INSERT INTO chats (project, name, created) VALUES (1, 'Chat 1', 0);
                 PRAGMA user_version = {version};"
            ))
            .unwrap();
    }

    /// Each table of the database at `path`, with its columns and the
    /// tables they refer to, as `SQLite` describes them.
    fn tables(path: &Path) -> Vec<(String, Vec<String>)> {
        let connection = Connection::open(path).unwrap();
        let names: Vec<String> = connection
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        names
            .into_iter()
            .map(|name| {
                let described = |pragma: &str| -> Vec<String> {
                    let mut statement = connection
                        .prepare(&format!("SELECT * FROM pragma_{pragma}('{name}')"))
                        .unwrap();
                    let width = statement.column_count();
                    statement
                        .query_map([], |row| {
                            (0..width)
                                .map(|at| row.get::<_, rusqlite::types::Value>(at))
                                .collect::<Result<Vec<_>, _>>()
                        })
                        .unwrap()
                        .map(|row| format!("{pragma} {:?}", row.unwrap()))
                        .collect()
                };
                let mut columns = described("table_info");
                columns.extend(described("foreign_key_list"));
                (name, columns)
            })
            .collect()
    }

    #[test]
    fn a_database_of_every_earlier_version_is_upgraded_to_the_tables_of_a_new_one() {
        let directory = scratch();
        let new = directory.path().join("new.sqlite");
        drop(Store::open(&new).unwrap());
        let version = usize::try_from(VERSION).unwrap();

        for earlier in 1..version {
            let path = directory.path().join(format!("{earlier}.sqlite"));
            database_of_version(&path, earlier);
            drop(Store::open(&path).unwrap());
            assert_eq!(
                tables(&path),
                tables(&new),
                "the tables of version {earlier}, upgraded"
            );
        }
    }

    #[test]
    fn the_projects_and_chats_of_an_earlier_cox_are_kept_through_the_upgrade() {
        let directory = scratch();
        let path = directory.path().join("chats.sqlite");
        database_of_version(&path, 1);

        let store = Store::open(&path).unwrap();
        let projects = store.projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].folder, Path::new("/src/cox"));
        assert!(!projects[0].archived);
        let chats = store.chats().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].name, "Chat 1");
        assert_eq!(chats[0].project, Some(projects[0].id));
        assert_eq!(chats[0].named, Named::Numbered);
        assert!(!chats[0].archived);
        assert_eq!(chats[0].goal, None);
        assert_eq!(chats[0].parent, None);
    }

    #[test]
    fn the_plans_kept_before_are_read_as_kept_through_the_upgrade() {
        let directory = scratch();
        let path = directory.path().join("chats.sqlite");
        database_of_version(&path, 23);
        let step = |id: Option<u64>, text: &str, state, since: i64| Step {
            text: text.to_owned(),
            state,
            since: Some(since),
            started: Some(since),
            id,
        };
        let entries = [
            Entry::Plan(vec![
                step(Some(1), "Read", StepState::InProgress, 10),
                step(Some(2), "Fix", StepState::Pending, 10),
                step(None, "Older", StepState::Completed, 10),
            ]),
            Entry::Human("again".to_owned()),
            Entry::Plan(vec![
                step(Some(1), "Read the code", StepState::Completed, 20),
                step(Some(2), "Fix", StepState::InProgress, 20),
            ]),
        ];
        {
            let connection = Connection::open(&path).unwrap();
            for (number, entry) in entries.iter().enumerate() {
                connection
                    .execute(
                        "INSERT INTO entries (chat, number, at, entry) VALUES (1, ?1, ?2, ?3)",
                        params![
                            i64::try_from(number).unwrap(),
                            now(),
                            serde_json::to_string(entry).unwrap()
                        ],
                    )
                    .unwrap();
            }
        }

        let store = Store::open(&path).unwrap();

        let plans = store.plans().unwrap();
        assert_eq!(
            plans
                .iter()
                .map(|plan| (plan.number, plan.steps.len()))
                .collect::<Vec<_>>(),
            [(2, 2)]
        );
        assert_eq!(
            store.steps(plans[0].chat).unwrap(),
            [
                KeptStep {
                    id: 1,
                    text: "Read the code".to_owned(),
                    started: Some(20),
                    done: Some(20),
                },
                KeptStep {
                    id: 2,
                    text: "Fix".to_owned(),
                    started: Some(20),
                    done: None,
                },
            ]
        );
        assert_eq!(
            store.progress(0).unwrap().done,
            [(plans[0].chat, "Read the code".to_owned(), 20)]
        );
    }

    #[test]
    fn the_checks_kept_before_tell_the_story_through_the_upgrade() {
        use crate::entry::{Check, CheckState};
        let directory = scratch();
        let path = directory.path().join("chats.sqlite");
        database_of_version(&path, 24);
        let checks = Entry::Checks(vec![Check {
            name: "goal".to_owned(),
            state: CheckState::Failed,
            said: Some("1 test failed".to_owned()),
        }]);
        let entries = [
            Entry::Human("fix it".to_owned()),
            Entry::Reply("Done.".to_owned()),
            checks.clone(),
        ];
        {
            let connection = Connection::open(&path).unwrap();
            for (number, entry) in entries.iter().enumerate() {
                connection
                    .execute(
                        "INSERT INTO entries (chat, number, at, entry) VALUES (1, ?1, ?2, ?3)",
                        params![
                            i64::try_from(number).unwrap(),
                            now(),
                            serde_json::to_string(entry).unwrap()
                        ],
                    )
                    .unwrap();
            }
        }

        let store = Store::open(&path).unwrap();

        let chat = store.chats().unwrap()[0].id;
        let told: Vec<Entry> = store
            .milestones(chat)
            .unwrap()
            .into_iter()
            .map(|(_, entry)| entry)
            .collect();
        assert_eq!(told, [Entry::Human("fix it".to_owned()), checks]);
    }

    #[test]
    fn a_database_upgraded_opens_again_as_it_is() {
        let directory = scratch();
        let path = directory.path().join("chats.sqlite");
        database_of_version(&path, 1);
        drop(Store::open(&path).unwrap());

        let again = Store::open(&path);

        assert!(again.is_ok(), "{:#}", again.err().unwrap());
        assert_eq!(again.unwrap().chats().unwrap().len(), 1);
    }

    #[test]
    fn a_write_waits_for_another_cox_writing_the_same_database() {
        let directory = scratch();
        let path = directory.path().join("chats.sqlite");
        let store = Store::open(&path).unwrap();
        let other = Connection::open(&path).unwrap();
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        // The other Cox holds its write well under the time a write waits.
        let finishing = std::thread::spawn(move || {
            // allow-sleep: the other Cox holds its write a while, under the time a write waits
            std::thread::sleep(Duration::from_millis(200));
            other.execute_batch("COMMIT").unwrap();
        });

        let kept = store.project(Path::new("/src/cox"), "cox");

        finishing.join().unwrap();
        assert!(kept.is_ok(), "{kept:?}");
        assert_eq!(store.projects().unwrap().len(), 1);
    }

    #[test]
    fn what_is_kept_lasts_past_closing_the_database() {
        let directory = scratch();
        let path = directory.path().join("kept").join("chats.sqlite");
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
    }

    #[test]
    fn a_database_from_a_newer_cox_is_refused() {
        let directory = scratch();
        let path = directory.path().join("chats.sqlite");
        drop(Store::open(&path).unwrap());
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", VERSION + 1)
            .unwrap();

        let error = Store::open(&path).err().expect("the database is refused");
        assert!(format!("{error:#}").contains("newer Cox"), "{error:#}");
    }
}
