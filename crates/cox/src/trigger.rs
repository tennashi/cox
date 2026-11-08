//! Triggers: what runs, unasked, when something happens in a folder, each
//! a Markdown file whose properties, the front matter between `---` lines
//! at its top, say when it runs (`on`, `when`) and who does it (`by`), and
//! whose rest is what is done:
//!
//! ```markdown
//! ---
//! on: turn.end
//! when:
//!   changed: "**/*.rs"
//!   sketch: skip
//! by: new
//! allow: [read:*, search:*, execute:git diff*]
//! ---
//!
//! Read this turn's diff. If anything could be simpler, say where and how.
//! Change nothing.
//! ```
//!
//! The human's triggers are in `triggers` in their workflow's folder (see
//! [`crate::settings::workflow`]), and run in every project; a project's
//! own are in `.cox/triggers` in its folder, which the project's
//! repository can keep.
//!
//! As a chat's turn ends, as its agent ends it, its goal is checked first,
//! then the triggers of the folder it works in, `turn.end`, run in order:
//! the project's, then the human's, each by name. The first that fails
//! stops them, and the chat's agent is told what it found, as the next
//! turn, as a goal that is not met tells it (see
//! [`crate::chat::conversation`]).
//!
//! Which triggers run in a folder, and in which order, the human chooses
//! on the stage's Triggers tab, which Cox keeps apart from the files (see
//! [`Arranged`]), so that a trigger the human turns off in one project
//! still runs in the others, and the files stay as they are written.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use cox_core::entry::CheckState;

use crate::agent::setup::{Policy, Setup, StageUse};
use crate::schedule::Cron;

pub mod card;
pub mod discuss;

/// The folder of the human's workflow that holds their triggers.
pub const TRIGGERS: &str = "triggers";

/// The folder of a project that holds its own triggers.
pub const PROJECT_TRIGGERS: &str = ".cox/triggers";

/// Where a trigger is kept, and so where it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The project's own, in its folder.
    Project,
    /// The human's, in their workflow's folder, which runs in every
    /// project.
    Global,
}

/// What a trigger runs on.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum On {
    /// A chat's turn ends, as its agent ends it: `turn.end`.
    #[serde(rename = "turn.end")]
    TurnEnd,
    /// A chat's goal is met: `goal.met`.
    #[serde(rename = "goal.met")]
    GoalMet,
    /// A chat's agent wrote a page of a wiki: `wiki.written`, or, for
    /// some pages, `wiki.written: {page: "research/*"}`, globs of the
    /// pages' names. Fired, it names the page written.
    #[serde(rename = "wiki.written")]
    WikiWritten {
        #[serde(skip_serializing_if = "Option::is_none")]
        pages: Option<Vec<String>>,
    },
    /// A sketch ended, in the chat that goes on with it: `sketch.end`.
    #[serde(rename = "sketch.end")]
    SketchEnd,
    /// A chat opened: `chat.open`.
    #[serde(rename = "chat.open")]
    ChatOpen,
    /// The times a cron says, in no chat, as `schedule: {cron: "0 9 * * *"}`
    /// (see [`crate::schedule`]).
    #[serde(rename = "schedule")]
    Schedule { cron: Cron },
}

impl On {
    /// The events `value`, as `on` says them: one, a list, or, for those
    /// that say more, as `schedule`, each with what it says under it.
    fn read(value: &Value) -> Result<Vec<Self>, String> {
        let by_name = |name: &str| match name {
            "turn.end" => Ok(Self::TurnEnd),
            "goal.met" => Ok(Self::GoalMet),
            "wiki.written" => Ok(Self::WikiWritten { pages: None }),
            "sketch.end" => Ok(Self::SketchEnd),
            "chat.open" => Ok(Self::ChatOpen),
            "schedule" => {
                Err("schedule says when under it, as schedule: {cron: \"0 9 * * *\"}".to_owned())
            }
            other => Err(format!(
                "{other:?} is no event Cox knows: turn.end, goal.met, wiki.written, \
                 sketch.end, chat.open or schedule"
            )),
        };
        match value {
            Value::One(name) => Ok(vec![by_name(name)?]),
            Value::List(names) => names.iter().map(|name| by_name(name)).collect(),
            Value::Under(events) => events
                .iter()
                .map(|(name, under)| match (name.as_str(), under) {
                    ("schedule", Value::One(cron)) => Self::schedule(cron),
                    ("schedule", Value::Under(under)) => {
                        let cron = under
                            .get("cron")
                            .ok_or("schedule says nothing of when, as cron: \"0 9 * * *\"")?
                            .one()
                            .map_err(|why| format!("cron {why}"))?;
                        if let Some(other) = under.keys().find(|key| *key != "cron") {
                            return Err(format!("schedule: {other:?} is nothing Cox knows"));
                        }
                        Self::schedule(cron)
                    }
                    ("wiki.written", Value::Under(under)) => {
                        let pages = under
                            .get("page")
                            .map(|pages| pages.values().map_err(|why| format!("page {why}")))
                            .transpose()?;
                        if let Some(other) = under.keys().find(|key| *key != "page") {
                            return Err(format!("wiki.written: {other:?} is nothing Cox knows"));
                        }
                        Ok(Self::WikiWritten { pages })
                    }
                    (name, Value::Under(under)) if under.is_empty() => by_name(name),
                    (name, _) => Err(format!("{name} says nothing more under it")),
                })
                .collect(),
        }
    }

    /// Whether a trigger that runs on this runs on `fired`, the event as
    /// it happened: the same event, and, for a page of the wiki, one of
    /// the pages it names, if it names any.
    fn takes(&self, fired: &Self) -> bool {
        match (self, fired) {
            (
                Self::WikiWritten { pages: Some(globs) },
                Self::WikiWritten {
                    pages: Some(written),
                },
            ) => {
                let mut set = globset::GlobSetBuilder::new();
                for glob in globs {
                    match globset::GlobBuilder::new(glob)
                        .literal_separator(true)
                        .build()
                    {
                        Ok(glob) => {
                            set.add(glob);
                        }
                        Err(error) => tracing::error!("page: {glob:?} is no glob: {error}"),
                    }
                }
                set.build()
                    .is_ok_and(|set| written.iter().any(|page| set.is_match(page)))
            }
            (Self::WikiWritten { .. }, Self::WikiWritten { .. }) => true,
            (mine, fired) => mine == fired,
        }
    }

    /// The name of the event, as `on` writes it.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::TurnEnd => "turn.end",
            Self::GoalMet => "goal.met",
            Self::WikiWritten { .. } => "wiki.written",
            Self::SketchEnd => "sketch.end",
            Self::ChatOpen => "chat.open",
            Self::Schedule { .. } => "schedule",
        }
    }

    fn schedule(cron: &str) -> Result<Self, String> {
        Ok(Self::Schedule {
            cron: Cron::parse(cron)?,
        })
    }
}

/// Whether a trigger runs in a sketch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InSketch {
    /// Not in a sketch.
    Skip,
    /// In a sketch only.
    Only,
}

/// What has to hold, besides the event, for a trigger to run.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct When {
    /// The files that, changed since the trigger last ran, make it run, as
    /// globs from the folder, those that start `!` leaving files out;
    /// `None` runs it whatever changed.
    pub changed: Option<Vec<String>>,
    /// Whether it runs in a sketch, or in either.
    pub sketch: Option<InSketch>,
}

/// Who does what a trigger says.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "by")]
pub enum By {
    /// A shell, in a terminal on the stage, which runs the trigger's text
    /// as a command, passing when it exits 0.
    Shell,
    /// The chat's own agent, told the trigger's text in the human's place,
    /// as the next turn: once for a line of the human's, on a turn's end,
    /// so that the turn it starts does not tell it again.
    This,
    /// An agent Cox starts for it, as the judge of a goal: not a chat, so
    /// not in the list, which ends its reply with `PASS`, or `FAIL: ` and
    /// why.
    ///
    /// On a schedule, it is a chat that opens, which the human sees in
    /// the list, its first line the trigger's text.
    New {
        /// The patterns of the permission requests it goes on with, as
        /// `open_chat`'s `allow` takes them, or, when empty, what the judge
        /// of a goal may; for a schedule's chat, whose human answers the
        /// rest, none.
        allow: Vec<String>,
        /// The patterns of those it is refused.
        reject: Vec<String>,
        /// Where a schedule of the human's works, from `~` or whole, or,
        /// when `None`, Cox's home; a project's works in the project.
        folder: Option<PathBuf>,
    },
}

/// A trigger, as its file has it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Trigger {
    /// What it goes by: its file's name, less `.md`.
    pub name: String,
    /// Its file.
    pub file: PathBuf,
    pub scope: Scope,
    pub on: Vec<On>,
    pub when: When,
    pub by: By,
    /// What is done: the command, for a shell, or what the agent is asked.
    pub text: String,
}

/// A property of a trigger's front matter: a value, a list, or, as
/// `when`, the properties under it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    One(String),
    List(Vec<String>),
    Under(BTreeMap<String, Value>),
}

impl Value {
    /// The values it holds: one, or those of the list.
    fn values(&self) -> Result<Vec<String>, String> {
        match self {
            Self::One(value) => Ok(vec![value.clone()]),
            Self::List(values) => Ok(values.clone()),
            Self::Under(_) => Err("is no value or list".to_owned()),
        }
    }

    fn one(&self) -> Result<&str, String> {
        match self {
            Self::One(value) => Ok(value),
            _ => Err("is no single value".to_owned()),
        }
    }
}

impl Trigger {
    /// The trigger of the file at `path`, kept as `scope` says, unless it
    /// is no Markdown file.
    ///
    /// # Errors
    ///
    /// When Cox cannot read it, or it says what Cox does not know.
    pub fn read(path: &Path, scope: Scope) -> Result<Option<Self>, String> {
        if path.extension().is_none_or(|extension| extension != "md") {
            return Ok(None);
        }
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
            return Ok(None);
        };
        let said = |why: String| format!("{}: {why}", path.display());
        let text = std::fs::read_to_string(path).map_err(|error| said(error.to_string()))?;
        Self::parse(name, path, scope, &text)
            .map(Some)
            .map_err(said)
    }

    /// The trigger `name`, of `file`, whose text is `text`.
    fn parse(name: &str, file: &Path, scope: Scope, text: &str) -> Result<Self, String> {
        let (lines, rest) = crate::wiki::front(text);
        let mut properties = properties(&lines)?;
        let key = |properties: &mut BTreeMap<String, Value>, key: &str| properties.remove(key);
        let on = match key(&mut properties, "on") {
            Some(on) => On::read(&on)?,
            None => {
                return Err("it says nothing of when it runs, as on: turn.end".to_owned());
            }
        };
        let scheduled = on.iter().any(|on| matches!(on, On::Schedule { .. }));
        if scheduled && on.len() > 1 {
            return Err("a schedule runs in no chat, apart from the events of one: \
                 give it a file of its own"
                .to_owned());
        }
        let when = match key(&mut properties, "when") {
            None => When::default(),
            Some(_) if scheduled => {
                return Err(
                    "when is for the events of a chat, and a schedule runs in none".to_owned(),
                );
            }
            Some(Value::Under(mut when)) => {
                let changed = when
                    .remove("changed")
                    .map(|changed| changed.values().map_err(|why| format!("changed {why}")))
                    .transpose()?;
                let sketch = when
                    .remove("sketch")
                    .map(|sketch| match sketch.one()? {
                        "skip" => Ok(InSketch::Skip),
                        "only" => Ok(InSketch::Only),
                        other => Err(format!("sketch: {other:?} is neither skip nor only")),
                    })
                    .transpose()?;
                if let Some(other) = when.keys().next() {
                    return Err(format!("when: {other:?} is no condition Cox knows"));
                }
                When { changed, sketch }
            }
            Some(_) => return Err("when holds no conditions, as changed: \"**/*.rs\"".to_owned()),
        };
        let list = |properties: &mut BTreeMap<String, Value>, name: &str| {
            properties
                .remove(name)
                .map(|value| value.values().map_err(|why| format!("{name} {why}")))
                .transpose()
                .map(Option::unwrap_or_default)
        };
        let allow = list(&mut properties, "allow")?;
        let reject = list(&mut properties, "reject")?;
        let folder = match key(&mut properties, "folder") {
            None => None,
            Some(_) if !scheduled => {
                return Err(
                    "folder is where a schedule's chat works, and only a schedule \
                     has one"
                        .to_owned(),
                );
            }
            // A project's works in the project, whatever it says.
            Some(_) if scope == Scope::Project => None,
            Some(folder) => {
                let user = std::env::var_os("HOME").map(PathBuf::from);
                let folder = folder.one().map_err(|why| format!("folder {why}"))?;
                Some(
                    crate::settings::folder_of(folder, user.as_deref())
                        .map_err(|why| format!("folder: {why}"))?,
                )
            }
        };
        let by = match key(&mut properties, "by") {
            Some(by) => by.one().map_err(|why| format!("by {why}"))?.to_owned(),
            // A schedule opens a chat.
            None if scheduled => "new".to_owned(),
            None => return Err("it says nothing of who runs it, as by: shell".to_owned()),
        };
        let by = match by.as_str() {
            "shell" if scheduled => {
                return Err("a schedule opens a chat, by: new, and no shell runs it".to_owned());
            }
            "shell" if allow.is_empty() && reject.is_empty() => By::Shell,
            "shell" => {
                return Err("allow and reject are for an agent's permission requests, \
                     by: new, and a shell asks none"
                    .to_owned());
            }
            "new" => By::New {
                allow,
                reject,
                folder,
            },
            "this" if scheduled => {
                return Err(
                    "a schedule runs in no chat, so no chat's agent does it: by: new".to_owned(),
                );
            }
            "this" if allow.is_empty() && reject.is_empty() => By::This,
            "this" => {
                return Err("allow and reject are for an agent of the trigger's own, \
                     by: new, and the chat's agent is asked as the chat asks it"
                    .to_owned());
            }
            other => return Err(format!("by: {other:?} is none of shell, this and new")),
        };
        if let Some(other) = properties.keys().next() {
            return Err(format!("{other:?} is no property Cox knows"));
        }
        let text = rest.trim().to_owned();
        if text.is_empty() {
            return Err(match by {
                By::Shell => "it has no command to run under its properties".to_owned(),
                By::New { .. } | By::This => "it asks nothing under its properties".to_owned(),
            });
        }
        Ok(Self {
            name: name.to_owned(),
            file: file.to_owned(),
            scope,
            on,
            when,
            by,
            text,
        })
    }

    /// The times it runs, when it runs on a schedule.
    #[must_use]
    pub fn cron(&self) -> Option<&Cron> {
        self.on.iter().find_map(|on| match on {
            On::Schedule { cron } => Some(cron),
            _ => None,
        })
    }

    /// Where a schedule of the human's works: the folder it says, or else
    /// `home`.
    #[must_use]
    pub fn works_in(&self, home: &Path) -> PathBuf {
        match &self.by {
            By::New {
                folder: Some(folder),
                ..
            } => folder.clone(),
            _ => home.to_owned(),
        }
    }

    /// Whether it runs on `on`, in a chat that is a sketch as `sketch`
    /// says, whatever changed.
    #[must_use]
    pub fn runs_on(&self, on: &On, sketch: bool) -> bool {
        self.on.iter().any(|mine| mine.takes(on))
            && match self.when.sketch {
                None => true,
                Some(InSketch::Skip) => !sketch,
                Some(InSketch::Only) => sketch,
            }
    }

    /// Whether `files`, the paths from the folder that changed since it
    /// last ran, make it run, as its `changed` says.
    #[must_use]
    pub fn runs_for(&self, files: &[String]) -> bool {
        let Some(globs) = &self.when.changed else {
            return true;
        };
        let (out, of): (Vec<&String>, Vec<&String>) =
            globs.iter().partition(|glob| glob.starts_with('!'));
        let set = |globs: &[&String], strip: bool| {
            let mut set = globset::GlobSetBuilder::new();
            for glob in globs {
                let glob = if strip { &glob[1..] } else { glob.as_str() };
                match globset::GlobBuilder::new(glob)
                    .literal_separator(true)
                    .build()
                {
                    Ok(glob) => {
                        set.add(glob);
                    }
                    Err(error) => tracing::warn!("{glob:?} is no glob: {error}"),
                }
            }
            set.build().unwrap_or_else(|_| globset::GlobSet::empty())
        };
        let (of, out) = (set(&of, false), set(&out, true));
        files
            .iter()
            .any(|file| (of.is_empty() || of.is_match(file)) && !out.is_match(file))
    }
}

/// The properties of `lines`, the front matter of a trigger, as YAML
/// writes them: `key: value`, `key: [one, two]`, or a list a line each
/// under it, as `- one`, or, as `when`, properties of their own, indented
/// under it.
fn properties(lines: &[&str]) -> Result<BTreeMap<String, Value>, String> {
    let mut found = BTreeMap::new();
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        at += 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            return Err(format!("{:?} is indented under nothing", line.trim()));
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("{:?} is no property, as on: turn.end", line.trim()))?;
        let value = without_comment(value);
        let under: Vec<&str> = lines[at..]
            .iter()
            .take_while(|line| line.trim().is_empty() || line.starts_with([' ', '\t']))
            .copied()
            .collect();
        at += under.len();
        let value = if !value.is_empty() {
            flow(value)?
        } else if under.iter().any(|line| line.trim_start().starts_with("- ")) {
            Value::List(
                under
                    .iter()
                    .filter_map(|line| line.trim_start().strip_prefix("- "))
                    .map(|item| unquoted(without_comment(item)).to_owned())
                    .filter(|item| !item.is_empty())
                    .collect(),
            )
        } else {
            // Under it, as far in as the least indented line, so that what
            // is under that stays under it.
            let lines: Vec<&str> = under
                .iter()
                .copied()
                .filter(|line| !line.trim().is_empty())
                .collect();
            let indent = lines
                .iter()
                .map(|line| line.len() - line.trim_start().len())
                .min()
                .unwrap_or(0);
            let dedented: Vec<&str> = lines.iter().map(|line| &line[indent..]).collect();
            Value::Under(properties(&dedented)?)
        };
        found.insert(key.trim().to_owned(), value);
    }
    Ok(found)
}

/// `value`, as its line has it after the key: a list on the line, as
/// `[one, two]`, properties, as `{cron: "0 9 * * *"}`, or one value.
fn flow(value: &str) -> Result<Value, String> {
    let value = value.trim();
    if let Some(items) = value
        .strip_prefix('[')
        .and_then(|items| items.strip_suffix(']'))
    {
        return Ok(Value::List(
            apart(items)
                .into_iter()
                .map(|item| unquoted(item).to_owned())
                .filter(|item| !item.is_empty())
                .collect(),
        ));
    }
    if let Some(items) = value
        .strip_prefix('{')
        .and_then(|items| items.strip_suffix('}'))
    {
        let mut found = BTreeMap::new();
        for item in apart(items) {
            if item.trim().is_empty() {
                continue;
            }
            let (key, value) = item.split_once(':').unwrap_or((item, ""));
            let value = if value.trim().is_empty() {
                Value::Under(BTreeMap::new())
            } else {
                flow(value)?
            };
            found.insert(unquoted(key).to_owned(), value);
        }
        return Ok(Value::Under(found));
    }
    if value.starts_with(['[', '{']) {
        return Err(format!("{value:?} is not closed"));
    }
    Ok(Value::One(unquoted(value).to_owned()))
}

/// The items of `items`, between the commas that are in no quotes,
/// brackets or braces.
fn apart(items: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut quote = None;
    let mut start = 0;
    for (at, character) in items.char_indices() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (None, '"' | '\'') => quote = Some(character),
            (None, '[' | '{') => depth += 1,
            (None, ']' | '}') => depth = depth.saturating_sub(1),
            (None, ',') if depth == 0 => {
                found.push(&items[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    found.push(&items[start..]);
    found
}

/// `value` without a comment after it, `#` after a space, as YAML has it.
fn without_comment(value: &str) -> &str {
    let value = value.trim();
    match value.find(" #") {
        Some(at) if !value.starts_with(['"', '\'']) => value[..at].trim(),
        _ => value,
    }
}

/// `value` without the quotes around it, if it has them.
fn unquoted(value: &str) -> &str {
    let value = value.trim();
    ['"', '\'']
        .iter()
        .find_map(|&quote| value.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(value)
}

/// The triggers that run in `folder`, as its chats find them: the
/// project's own, in `.cox/triggers` there, then the human's, in their
/// `workflow`, each by name; and why each of those Cox cannot read does
/// not run.
#[must_use]
pub fn of(folder: &Path, workflow: &Path) -> (Vec<Trigger>, Vec<String>) {
    let (mut triggers, mut unread) = of_project(folder);
    let (global, also) = read_all(&workflow.join(TRIGGERS), Scope::Global);
    triggers.extend(global);
    unread.extend(also);
    (triggers, unread)
}

/// The project's own triggers of `folder`, and why each of those Cox
/// cannot read does not run.
fn of_project(folder: &Path) -> (Vec<Trigger>, Vec<String>) {
    read_all(&folder.join(PROJECT_TRIGGERS), Scope::Project)
}

/// The triggers that run on a schedule: the human's, in their
/// `workflow`, each with the folder its chat works in, as it says, or
/// else `home`, and the own of each of `projects`, which work there, saying
/// in the log those Cox cannot read.
#[must_use]
pub fn scheduled(workflow: &Path, home: &Path, projects: &[PathBuf]) -> Vec<(Trigger, PathBuf)> {
    let (global, mut unread) = read_all(&workflow.join(TRIGGERS), Scope::Global);
    let mut found: Vec<(Trigger, PathBuf)> = global
        .into_iter()
        .filter(|trigger| trigger.cron().is_some())
        .map(|trigger| {
            let works = trigger.works_in(home);
            (trigger, works)
        })
        .collect();
    for project in projects {
        let (own, also) = of_project(project);
        unread.extend(also);
        found.extend(
            own.into_iter()
                .filter(|trigger| trigger.cron().is_some())
                .map(|trigger| (trigger, project.clone())),
        );
    }
    for why in unread {
        tracing::warn!("a trigger does not run: {why}");
    }
    found
}

/// What the agent of a trigger `by: new` is told it is, ahead of what
/// the trigger asks.
const STANCE: &str = "You run as a trigger, as another chat's turn ends: an agent of its \
    own, which the human does not talk to. The chat works in this same folder. Each time \
    its turn ends, do what you are asked below, as it says, and unless it says otherwise, \
    change nothing.";

/// How the agent of a trigger ends its reply, so that Cox reads whether
/// it found something.
const PROTOCOL: &str = "End every reply with one line of its own: `PASS` when you found \
    nothing the chat has to act on, or `FAIL: ` and what it has to act on, in one \
    sentence. The chat is told your whole reply when it fails.";

/// How the agent of a trigger that asks `text` is set up: allowed what
/// `allow` names, or, with none, what the judge of a goal is, and refused
/// what `reject` names.
#[must_use]
pub fn setup(text: &str, allow: &[String], reject: &[String]) -> Setup {
    let allow = if allow.is_empty() {
        crate::chat::judge::ALLOWED
            .iter()
            .map(|&title| title.to_owned())
            .collect()
    } else {
        allow.to_vec()
    };
    Setup {
        context: Some(format!(
            "{STANCE}\n\n{PROTOCOL}\n\nWhat you are asked:\n\n{}\n",
            text.trim()
        )),
        tools: Some(Vec::new()),
        stage: StageUse::Look,
        policy: Policy {
            allow,
            reject: reject.to_vec(),
            within: None,
        },
        folder: None,
        sketch: None,
    }
}

/// What the agent of a trigger is asked as a turn of the chat ended,
/// which last `said` this.
#[must_use]
pub fn question(said: Option<&str>) -> String {
    let said = match said {
        Some(said) if !said.trim().is_empty() => {
            format!(
                "The chat's turn ended. It last said:\n\n{}\n\n",
                said.trim()
            )
        }
        _ => "The chat's turn ended. It said nothing this turn.\n\n".to_owned(),
    };
    format!("{said}Do what you were asked, and end with PASS or FAIL: what to act on.")
}

/// What the agent of a trigger is asked as `about` says something
/// happened in the chat, besides a turn's end.
#[must_use]
pub fn asked_of(about: &str) -> String {
    format!(
        "{}\n\nDo what you were asked, and end with PASS or FAIL: what to act on.",
        about.trim()
    )
}

/// Which triggers the human turned off in a folder, and the order they
/// run in there, by their files, as the stage's Triggers tab sets them:
/// kept in Cox's state (see [`crate::storage::Kept::Triggers`]), a folder
/// each.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Arranged {
    /// The files of the triggers that do not run in the folder.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub off: BTreeSet<PathBuf>,
    /// The files of the triggers in the order they run in, those it does
    /// not name after them, as [`of`] orders them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<PathBuf>,
}

impl Arranged {
    /// How the triggers of `folder` are arranged, as `kept`, the file Cox
    /// keeps it in (see [`crate::storage::Kept::Triggers`]), has it.
    #[must_use]
    pub fn read(kept: Option<&Path>, folder: &Path) -> Self {
        read_arranged(kept).remove(folder).unwrap_or_default()
    }

    /// Keeps how the triggers of `folder` are arranged, in `kept`.
    ///
    /// # Errors
    ///
    /// When `kept` cannot be written.
    pub fn keep(self, kept: Option<&Path>, folder: &Path) -> Result<(), String> {
        let path = kept.ok_or("Cox cannot tell where it keeps its state")?;
        let mut all = read_arranged(Some(path));
        if self == Self::default() {
            all.remove(folder);
        } else {
            all.insert(folder.to_owned(), self);
        }
        write_kept(path, &all)
    }

    /// `triggers`, in the order they run in, each with whether it runs.
    #[must_use]
    pub fn apply(&self, mut triggers: Vec<Trigger>) -> Vec<(Trigger, bool)> {
        let place = |trigger: &Trigger| {
            self.order
                .iter()
                .position(|file| *file == trigger.file)
                .unwrap_or(usize::MAX)
        };
        // A stable sort keeps the triggers not ordered as they were.
        triggers.sort_by_key(place);
        triggers
            .into_iter()
            .map(|trigger| {
                let on = !self.off.contains(&trigger.file);
                (trigger, on)
            })
            .collect()
    }
}

/// How a trigger last ran in a folder, for the Triggers tab to show.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ran {
    /// How it went: passed, failed, as when it found something, or
    /// broken, when it could not run.
    pub state: CheckState,
    /// When, in seconds since the Unix epoch.
    pub at: i64,
    /// What it said, at the end, as the chip of the turn's end keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
}

impl Ran {
    /// How the triggers of `folder` last ran, by their files, as `kept`,
    /// the file Cox keeps it in (see [`crate::storage::Kept::TriggerRuns`]),
    /// has it.
    #[must_use]
    pub fn read(kept: Option<&Path>, folder: &Path) -> BTreeMap<PathBuf, Self> {
        read_kept::<BTreeMap<PathBuf, Self>>(kept)
            .remove(folder)
            .unwrap_or_default()
    }

    /// Keeps that the trigger of `file` ran in `folder` so, in `kept`.
    ///
    /// # Errors
    ///
    /// When `kept` cannot be written.
    pub fn keep(self, kept: Option<&Path>, folder: &Path, file: &Path) -> Result<(), String> {
        let path = kept.ok_or("Cox cannot tell where it keeps its state")?;
        let mut all = read_kept::<BTreeMap<PathBuf, Self>>(Some(path));
        all.entry(folder.to_owned())
            .or_default()
            .insert(file.to_owned(), self);
        write_kept(path, &all)
    }
}

/// Where the stage's Triggers tab is told the triggers are, and how
/// they ran: the human's `workflow` and Cox's `home`, and the files Cox
/// keeps how the triggers are arranged in, how each ran in a chat in, and
/// when each schedule last ran in.
pub struct Kept<'a> {
    pub workflow: &'a Path,
    pub home: &'a Path,
    pub arranged: Option<&'a Path>,
    pub runs: Option<&'a Path>,
    pub schedules: &'a crate::schedule::Ran,
}

/// What the stage's Triggers tab is told, as `cox://api/triggers`: the
/// triggers of each folder it shows, `folders`, by its path and name, in
/// the order they run in there, each with what its file says, whether it
/// runs there, and how it last ran there; and why each Cox cannot read
/// does not run. A schedule of the human's shows where its chat works,
/// and in the home, if that is no folder shown, there; it runs there, or
/// not, and when, as of `now`.
#[must_use]
pub fn told(
    folders: &[(PathBuf, Option<String>)],
    kept: &Kept<'_>,
    now: &jiff::Zoned,
) -> serde_json::Value {
    let mut unread = BTreeSet::new();
    let shown = |folder: &Path| folders.iter().any(|(shown, _)| shown == folder);
    let folders: Vec<serde_json::Value> = folders
        .iter()
        .map(|(folder, name)| {
            let (triggers, why) = of(folder, kept.workflow);
            unread.extend(why);
            let ran = Ran::read(kept.runs, folder);
            let arranged = Arranged::read(kept.arranged, folder);
            // A schedule of the human's runs where its chat works, once.
            let triggers = triggers.into_iter().filter(|trigger| {
                trigger.cron().is_none()
                    || trigger.scope == Scope::Project
                    || trigger.works_in(kept.home) == *folder
                    || (folder == kept.home && !shown(&trigger.works_in(kept.home)))
            });
            let triggers: Vec<serde_json::Value> = arranged
                .apply(triggers.collect())
                .into_iter()
                .map(|(trigger, on)| {
                    let (by, allow, reject) = match &trigger.by {
                        By::Shell => ("shell", Vec::new(), Vec::new()),
                        By::This => ("this", Vec::new(), Vec::new()),
                        By::New { allow, reject, .. } => ("new", allow.clone(), reject.clone()),
                    };
                    let mut told = serde_json::json!({
                        "name": trigger.name,
                        "file": trigger.file,
                        "scope": trigger.scope,
                        "on": trigger.on,
                        "changed": trigger.when.changed,
                        "sketch": trigger.when.sketch,
                        "by": by,
                        "allow": allow,
                        "reject": reject,
                        "text": trigger.text.trim(),
                        "enabled": on,
                        "at": folder,
                        "ran": ran.get(&trigger.file),
                        "draft": card::Draft::of(&trigger),
                    });
                    if let Some(cron) = trigger.cron() {
                        let works = match trigger.scope {
                            Scope::Project => folder.clone(),
                            Scope::Global => trigger.works_in(kept.home),
                        };
                        let last = kept.schedules.last(&trigger.file.display().to_string());
                        if works != *folder {
                            told["at"] = serde_json::json!(works);
                            told["enabled"] = (!Arranged::read(kept.arranged, &works)
                                .off
                                .contains(&trigger.file))
                            .into();
                        }
                        if let Some(last) = last {
                            told["ran"] = serde_json::json!({
                                "state": CheckState::Passed,
                                "at": last.as_second(),
                            });
                        }
                        told["schedule"] = serde_json::json!({
                            "cron": cron,
                            "every": cron.words(),
                            "works": works,
                            "last": last.map(|last| last
                                .to_zoned(now.time_zone().clone())
                                .strftime("%Y-%m-%d %H:%M")
                                .to_string()),
                            "next": cron.next(last, now).map(|next| next.timestamp().as_second()),
                            "due": cron.due(last, now),
                        });
                    }
                    told
                })
                .collect();
            serde_json::json!({
                "folder": folder,
                "name": name,
                "triggers": triggers,
            })
        })
        .collect();
    serde_json::json!({
        "workflow": kept.workflow.join(TRIGGERS),
        "now": now.timestamp().as_second(),
        "folders": folders,
        "unread": unread,
    })
}

/// What the Triggers tab changes of how the triggers of a folder are
/// arranged: one turned on or off, or the order they run in.
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
pub enum Change {
    /// The trigger of `file` turned on, or off, in `folder`.
    Switch {
        folder: PathBuf,
        file: PathBuf,
        on: bool,
    },
    /// The triggers of `folder` ordered so, by their files.
    Order {
        folder: PathBuf,
        order: Vec<PathBuf>,
    },
}

impl Change {
    /// The folder it changes.
    #[must_use]
    pub fn folder(&self) -> &Path {
        match self {
            Self::Switch { folder, .. } | Self::Order { folder, .. } => folder,
        }
    }

    /// Makes the change, keeping it in `kept`.
    ///
    /// # Errors
    ///
    /// When `kept` cannot be written.
    pub fn make(self, kept: Option<&Path>) -> Result<(), String> {
        let folder = self.folder().to_owned();
        let mut arranged = Arranged::read(kept, &folder);
        match self {
            Self::Switch { file, on, .. } => {
                if on {
                    arranged.off.remove(&file);
                } else {
                    arranged.off.insert(file);
                }
            }
            Self::Order { order, .. } => arranged.order = order,
        }
        arranged.keep(kept, &folder)
    }
}

/// How the triggers of each folder are arranged, as `kept` has it.
fn read_arranged(kept: Option<&Path>) -> BTreeMap<PathBuf, Arranged> {
    read_kept(kept)
}

/// Writes `all` to `path`, as JSON, a folder's each.
fn write_kept<T: serde::Serialize>(path: &Path, all: &BTreeMap<PathBuf, T>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(all).map_err(|error| error.to_string())?;
    path.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(path, text + "\n"))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// What `kept` holds, a folder's each, or nothing when it holds nothing
/// Cox reads.
fn read_kept<T: serde::de::DeserializeOwned>(kept: Option<&Path>) -> BTreeMap<PathBuf, T> {
    let Some(path) = kept else {
        return BTreeMap::new();
    };
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            tracing::error!("cannot read {}: {error}", path.display());
            BTreeMap::new()
        }),
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::error!("cannot read {}: {error}", path.display());
            }
            BTreeMap::new()
        }
    }
}

/// The triggers in `folder`, by name, and why each Cox cannot read does
/// not run.
fn read_all(folder: &Path, scope: Scope) -> (Vec<Trigger>, Vec<String>) {
    let mut triggers = Vec::new();
    let mut unread = Vec::new();
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.filter_map(Result::ok) {
            match Trigger::read(&entry.path(), scope) {
                Ok(trigger) => triggers.extend(trigger),
                Err(why) => unread.push(why),
            }
        }
    }
    triggers.sort_by(|one, other| one.name.cmp(&other.name));
    unread.sort();
    (triggers, unread)
}

/// The tree of what `folder`'s files hold now, as Git would commit them
/// all, what it ignores aside, without touching its index: a tree's hash,
/// to tell later what changed since. `None` when the folder is in no Git
/// repository.
#[must_use]
pub fn tree(folder: &Path) -> Option<String> {
    let index = git(folder, &["rev-parse", "--git-path", "index"])?;
    let index = folder.join(index.trim());
    let scratch = tempfile_path();
    // Starting from the index, Git hashes again only the files that
    // changed since it was last written.
    if index.exists() {
        std::fs::copy(&index, &scratch).ok()?;
    }
    let with = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(folder)
            .env("GIT_INDEX_FILE", &scratch)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let tree = with(&["add", "--all", "--", "."]).and_then(|_| with(&["write-tree"]));
    let _ = std::fs::remove_file(&scratch);
    tree
}

/// The tree of `folder`'s last commit, if it has one.
#[must_use]
pub fn committed(folder: &Path) -> Option<String> {
    git(folder, &["rev-parse", "--verify", "--quiet", "HEAD^{tree}"])
        .map(|tree| tree.trim().to_owned())
}

/// The paths, from `folder`, of the files that differ between the trees
/// `from` and `to`.
#[must_use]
pub fn changed_between(folder: &Path, from: &str, to: &str) -> Option<Vec<String>> {
    let prefix = git(folder, &["rev-parse", "--show-prefix"])?;
    let prefix = prefix.trim();
    let names = git(folder, &["diff", "--name-only", "--no-renames", from, to])?;
    Some(
        names
            .lines()
            .filter_map(|name| name.strip_prefix(prefix))
            .map(str::to_owned)
            .collect(),
    )
}

/// What `git args` printed in `folder`, if it ran and passed.
fn git(folder: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(folder)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A path for an index of Git's own, among the system's temporary files.
fn tempfile_path() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "cox-trigger-index-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Trigger, String> {
        Trigger::parse("t", Path::new("t.md"), Scope::Global, text)
    }

    fn named(name: &str) -> Trigger {
        let file = PathBuf::from(format!("/t/{name}.md"));
        Trigger::parse(
            name,
            &file,
            Scope::Project,
            "---\non: turn.end\nby: shell\n---\ntrue",
        )
        .unwrap()
    }

    #[test]
    fn the_triggers_tab_is_told_the_triggers_of_each_folder_and_changes_them() {
        let root = std::env::temp_dir().join(format!("cox-told-triggers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("project");
        let workflow = root.join("workflow");
        std::fs::create_dir_all(project.join(PROJECT_TRIGGERS)).unwrap();
        std::fs::create_dir_all(workflow.join(TRIGGERS)).unwrap();
        let ci = project.join(PROJECT_TRIGGERS).join("ci.md");
        std::fs::write(&ci, "---\non: turn.end\nby: shell\n---\ncargo test\n").unwrap();
        let simplify = workflow.join(TRIGGERS).join("simplify.md");
        std::fs::write(
            &simplify,
            "---\non: turn.end\nwhen:\n  changed: \"**/*.rs\"\nby: new\n---\nSay what could be simpler.\n",
        )
        .unwrap();
        std::fs::write(
            workflow.join(TRIGGERS).join("bad.md"),
            "---\non: never\n---\nx\n",
        )
        .unwrap();
        let arranged = root.join("triggers.json");
        let runs = root.join("trigger-runs.json");
        Ran {
            state: CheckState::Failed,
            at: 100,
            said: Some("FAIL: x".to_owned()),
        }
        .keep(Some(&runs), &project, &simplify)
        .unwrap();
        Change::Order {
            folder: project.clone(),
            order: vec![simplify.clone()],
        }
        .make(Some(&arranged))
        .unwrap();
        Change::Switch {
            folder: project.clone(),
            file: ci.clone(),
            on: false,
        }
        .make(Some(&arranged))
        .unwrap();

        let schedules = crate::schedule::Ran::default();
        let kept = Kept {
            workflow: &workflow,
            home: &root.join("home"),
            arranged: Some(&arranged),
            runs: Some(&runs),
            schedules: &schedules,
        };
        let told = told(&[(project.clone(), None)], &kept, &jiff::Zoned::now());
        let triggers = &told["folders"][0]["triggers"];
        assert_eq!(triggers[0]["name"], "simplify");
        assert_eq!(triggers[0]["scope"], "global");
        assert_eq!(triggers[0]["by"], "new");
        assert_eq!(triggers[0]["changed"], serde_json::json!(["**/*.rs"]));
        assert_eq!(triggers[0]["enabled"], true);
        assert_eq!(triggers[0]["ran"]["state"], "failed");
        assert_eq!(triggers[0]["ran"]["said"], "FAIL: x");
        assert_eq!(triggers[1]["name"], "ci");
        assert_eq!(triggers[1]["by"], "shell");
        assert_eq!(triggers[1]["text"], "cargo test");
        assert_eq!(triggers[1]["enabled"], false);
        assert_eq!(triggers[1]["ran"], serde_json::Value::Null);
        assert_eq!(told["unread"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The schedules of a workflow in `root`, and of a project: one that
    /// works in the project, one that works in the home, one that works
    /// in a folder no project has, and the project's own. Answers the home, the
    /// project, that folder and the workflow.
    fn schedules_in(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let _ = std::fs::remove_dir_all(root);
        let (home, project, elsewhere) = (root.join("home"), root.join("app"), root.join("notes"));
        let workflow = root.join("workflow");
        for folder in [workflow.join(TRIGGERS), project.join(PROJECT_TRIGGERS)] {
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::write(
            workflow.join(TRIGGERS).join("news.md"),
            format!(
                "---\non:\n  schedule:\n    cron: \"0 9 * * *\"\nby: new\nfolder: {}\n---\nNews since {{last}}.\n",
                project.display()
            ),
        )
        .unwrap();
        std::fs::write(
            workflow.join(TRIGGERS).join("tidy.md"),
            "---\non: {schedule: {cron: \"0 */6 * * *\"}}\nallow: [read:*]\n---\nTidy.\n",
        )
        .unwrap();
        std::fs::write(
            workflow.join(TRIGGERS).join("away.md"),
            format!(
                "---\non: {{schedule: {{cron: \"0 9 * * 1-5\"}}}}\nfolder: {}\n---\nAway.\n",
                elsewhere.display()
            ),
        )
        .unwrap();
        std::fs::write(
            project.join(PROJECT_TRIGGERS).join("triage.md"),
            "---\non: {schedule: {cron: \"0 9 * * *\"}}\nfolder: ~/ignored\n---\nTriage.\n",
        )
        .unwrap();
        (home, project, elsewhere, workflow)
    }

    #[test]
    fn a_schedule_shows_where_its_chat_works() {
        let root = std::env::temp_dir().join(format!("cox-told-schedules-{}", std::process::id()));
        let (home, project, elsewhere, workflow) = schedules_in(&root);
        let mut schedules = crate::schedule::Ran::default();
        let now = jiff::civil::date(2026, 11, 3)
            .at(10, 0, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap();
        schedules.ran(
            &workflow
                .join(TRIGGERS)
                .join("tidy.md")
                .display()
                .to_string(),
            now.timestamp(),
        );
        let arranged = root.join("triggers.json");
        let kept = Kept {
            workflow: &workflow,
            home: &home,
            arranged: Some(&arranged),
            runs: None,
            schedules: &schedules,
        };
        let names = |told: &serde_json::Value, at: usize| -> Vec<String> {
            told["folders"][at]["triggers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|trigger| trigger["name"].as_str().unwrap().to_owned())
                .collect()
        };

        // The project shows its own, and the human's that works there.
        let told = told(&[(project.clone(), None)], &kept, &now);
        assert_eq!(names(&told, 0), ["triage", "news"]);
        let triage = &told["folders"][0]["triggers"][0];
        assert_eq!(triage["schedule"]["cron"], "0 9 * * *");
        assert_eq!(triage["schedule"]["every"], "Daily 09:00");
        assert_eq!(triage["schedule"]["due"], true);
        assert_eq!(triage["on"][0]["schedule"]["cron"], "0 9 * * *");

        // The home shows its own, and those that work in no folder shown,
        // each turned on or off where it works.
        Change::Switch {
            folder: elsewhere.clone(),
            file: workflow.join(TRIGGERS).join("away.md"),
            on: false,
        }
        .make(Some(&arranged))
        .unwrap();
        let told = super::told(
            &[
                (home.clone(), None),
                (project.clone(), Some("app".to_owned())),
            ],
            &kept,
            &now,
        );
        assert_eq!(names(&told, 0), ["away", "tidy"]);
        assert_eq!(names(&told, 1), ["triage", "news"]);
        let away = &told["folders"][0]["triggers"][0];
        assert_eq!(away["at"], serde_json::json!(elsewhere));
        assert_eq!(away["enabled"], false);
        assert_eq!(away["schedule"]["every"], "Weekdays 09:00");
        let tidy = &told["folders"][0]["triggers"][1];
        assert_eq!(tidy["schedule"]["cron"], "0 */6 * * *");
        assert_eq!(tidy["schedule"]["due"], false);
        assert_eq!(tidy["schedule"]["last"], "2026-11-03 10:00");
        assert_eq!(tidy["ran"]["state"], "passed");
        assert_eq!(
            tidy["schedule"]["next"],
            now.timestamp().as_second() + 2 * 3600
        );

        // coxd runs each where it works.
        let found: Vec<(String, PathBuf)> =
            scheduled(&workflow, &home, std::slice::from_ref(&project))
                .into_iter()
                .map(|(trigger, works)| (trigger.name, works))
                .collect();
        assert_eq!(
            found,
            [
                ("away".to_owned(), elsewhere.clone()),
                ("news".to_owned(), project.clone()),
                ("tidy".to_owned(), home.clone()),
                ("triage".to_owned(), project.clone()),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_human_turns_triggers_off_and_orders_them_in_a_folder() {
        let kept = std::env::temp_dir()
            .join(format!("cox-arranged-{}", std::process::id()))
            .join("triggers.json");
        let work = Path::new("/work");
        assert_eq!(Arranged::read(Some(&kept), work), Arranged::default());

        let arranged = Arranged {
            off: BTreeSet::from([PathBuf::from("/t/b.md")]),
            order: vec![PathBuf::from("/t/c.md")],
        };
        arranged.clone().keep(Some(&kept), work).unwrap();
        assert_eq!(Arranged::read(Some(&kept), work), arranged);
        assert_eq!(
            Arranged::read(Some(&kept), Path::new("/other")),
            Arranged::default()
        );
        let applied: Vec<(String, bool)> = arranged
            .apply(vec![named("a"), named("b"), named("c")])
            .into_iter()
            .map(|(trigger, on)| (trigger.name, on))
            .collect();
        assert_eq!(
            applied,
            [
                ("c".to_owned(), true),
                ("a".to_owned(), true),
                ("b".to_owned(), false),
            ]
        );

        // Arranged as at first, the folder is forgotten.
        Arranged::default().keep(Some(&kept), work).unwrap();
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "{}\n");
        let _ = std::fs::remove_dir_all(kept.parent().unwrap());
    }

    #[test]
    fn reads_a_trigger_an_agent_runs() {
        let trigger = parse(
            "---\non: turn.end\nwhen:\n  changed: \"**/*.rs\"\n  sketch: skip\nby: new\n\
             allow:\n  - read:*\n  - execute:git diff*\n---\n\nRead the diff.\n",
        )
        .unwrap();
        assert_eq!(trigger.on, vec![On::TurnEnd]);
        assert_eq!(
            trigger.when,
            When {
                changed: Some(vec!["**/*.rs".to_owned()]),
                sketch: Some(InSketch::Skip),
            }
        );
        assert_eq!(
            trigger.by,
            By::New {
                allow: vec!["read:*".to_owned(), "execute:git diff*".to_owned()],
                reject: Vec::new(),
                folder: None,
            }
        );
        assert_eq!(trigger.text, "Read the diff.");
    }

    #[test]
    fn reads_a_trigger_a_shell_runs() {
        let trigger =
            parse("---\non: [turn.end]\nby: shell # in the stage\n---\ncargo xtask ci\n").unwrap();
        assert_eq!(trigger.by, By::Shell);
        assert_eq!(trigger.when, When::default());
        assert_eq!(trigger.text, "cargo xtask ci");
    }

    #[test]
    fn says_what_it_does_not_know() {
        assert!(
            parse("---\nby: shell\n---\nmake\n")
                .unwrap_err()
                .contains("on: turn.end")
        );
        assert!(
            parse("---\non: turn.end\n---\nmake\n")
                .unwrap_err()
                .contains("by: shell")
        );
        assert!(
            parse("---\non: lunch\nby: shell\n---\nmake\n")
                .unwrap_err()
                .contains("lunch")
        );
        assert_eq!(
            parse("---\non: [goal.met, turn.end]\nby: this\n---\nWrite the commit message.\n")
                .unwrap()
                .by,
            By::This
        );
        assert!(
            parse("---\non: goal.met\nby: this\nallow: [read:*]\n---\nhi\n")
                .unwrap_err()
                .contains("by: new")
        );
        let wiki =
            parse("---\non:\n  wiki.written:\n    page: \"research/*\"\nby: new\n---\nCheck it.\n")
                .unwrap();
        let written = |page: &str| On::WikiWritten {
            pages: Some(vec![page.to_owned()]),
        };
        assert!(wiki.runs_on(&written("research/rust"), false));
        assert!(!wiki.runs_on(&written("research/rust/1.92"), false));
        assert!(!wiki.runs_on(&written("gpui-layout"), false));
        assert!(!wiki.runs_on(&On::TurnEnd, false));
        let any = parse("---\non: [wiki.written, sketch.end, chat.open]\nby: shell\n---\nmake\n")
            .unwrap();
        assert!(any.runs_on(&written("gpui-layout"), false));
        assert!(any.runs_on(&On::SketchEnd, false));
        assert!(any.runs_on(&On::ChatOpen, false));
        assert!(!any.runs_on(&On::GoalMet, false));
        assert!(
            parse("---\non: turn.end\nby: shell\n---\n\n")
                .unwrap_err()
                .contains("command")
        );
        assert!(
            parse("---\non: turn.end\nwhen:\n  sketch: maybe\nby: shell\n---\nmake\n")
                .unwrap_err()
                .contains("maybe")
        );
        let scheduled = |rest: &str| {
            parse(&format!(
                "---\non: {{schedule: {{cron: \"0 9 * * *\"}}}}\n{rest}---\nhi\n"
            ))
        };
        assert!(scheduled("by: shell\n").unwrap_err().contains("by: new"));
        assert!(scheduled("by: this\n").unwrap_err().contains("by: new"));
        assert!(
            scheduled("when:\n  sketch: skip\n")
                .unwrap_err()
                .contains("when")
        );
        assert!(
            parse("---\non: turn.end\nby: new\nfolder: ~/x\n---\nhi\n")
                .unwrap_err()
                .contains("folder")
        );
        assert!(
            parse("---\non: {schedule: {cron: \"0 9 * *\"}}\n---\nhi\n")
                .unwrap_err()
                .contains("five fields")
        );
        assert!(
            parse("---\non:\n  turn.end:\n  schedule: \"0 9 * * *\"\nby: new\n---\nhi\n")
                .unwrap_err()
                .contains("file of its own")
        );
        assert!(
            parse("---\non: schedule\n---\nhi\n")
                .unwrap_err()
                .contains("cron")
        );
    }

    #[test]
    fn reads_a_schedule_in_the_ways_yaml_writes_it() {
        let cron = |text: &str| parse(text).map(|trigger| trigger.cron().map(ToString::to_string));
        let want = Ok(Some("0 9 * * 1,3".to_owned()));
        assert_eq!(
            cron("---\non:\n  schedule:\n    cron: \"0 9 * * 1,3\"\nby: new\n---\nhi\n"),
            want
        );
        assert_eq!(
            cron("---\non: {schedule: {cron: '0 9 * * 1,3'}}\n---\nhi\n"),
            want
        );
        assert_eq!(cron("---\non:\n  schedule: 0 9 * * 1,3\n---\nhi\n"), want);
        let trigger = parse(
            "---\non: {schedule: {cron: \"0 9 * * *\"}}\nfolder: /notes\nallow:\n  - fetch:*\n---\nhi\n",
        )
        .unwrap();
        assert_eq!(
            trigger.by,
            By::New {
                allow: vec!["fetch:*".to_owned()],
                reject: Vec::new(),
                folder: Some("/notes".into()),
            }
        );
        assert_eq!(trigger.works_in(Path::new("/home")), Path::new("/notes"));
        assert!(!trigger.runs_on(&On::TurnEnd, false));
    }

    #[test]
    fn runs_in_a_sketch_as_it_says() {
        let skip =
            parse("---\non: turn.end\nwhen:\n  sketch: skip\nby: shell\n---\nmake\n").unwrap();
        assert!(skip.runs_on(&On::TurnEnd, false));
        assert!(!skip.runs_on(&On::TurnEnd, true));
        let only =
            parse("---\non: turn.end\nwhen:\n  sketch: only\nby: shell\n---\nmake\n").unwrap();
        assert!(!only.runs_on(&On::TurnEnd, false));
        assert!(only.runs_on(&On::TurnEnd, true));
    }

    #[test]
    fn runs_for_the_files_it_names() {
        let trigger = parse(
            "---\non: turn.end\nwhen:\n  changed: [\"**/*.rs\", \"!tests/**\"]\nby: shell\n---\nmake\n",
        )
        .unwrap();
        let files = |files: &[&str]| {
            files
                .iter()
                .map(|&file| file.to_owned())
                .collect::<Vec<_>>()
        };
        assert!(trigger.runs_for(&files(&["src/main.rs"])));
        assert!(!trigger.runs_for(&files(&["tests/chats.rs"])));
        assert!(!trigger.runs_for(&files(&["README.md"])));
        assert!(!trigger.runs_for(&[]));
        let any = parse("---\non: turn.end\nby: shell\n---\nmake\n").unwrap();
        assert!(any.runs_for(&[]));
    }

    #[test]
    fn tells_what_changed_since_a_tree() {
        let folder = std::env::temp_dir().join(format!("cox-trigger-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(folder.as_path())
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "--quiet"]);
        std::fs::write(folder.as_path().join("a.rs"), "fn a() {}").unwrap();
        std::fs::write(folder.as_path().join(".gitignore"), "target/\n").unwrap();
        let before = tree(folder.as_path()).unwrap();
        std::fs::write(folder.as_path().join("b.rs"), "fn b() {}").unwrap();
        std::fs::create_dir(folder.as_path().join("target")).unwrap();
        std::fs::write(folder.as_path().join("target/x.rs"), "").unwrap();
        let after = tree(folder.as_path()).unwrap();
        assert_eq!(
            changed_between(folder.as_path(), &before, &after).unwrap(),
            vec!["b.rs".to_owned()]
        );
        // The index is the folder's as it was.
        let status = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(folder.as_path())
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&status.stdout), "");
        assert_eq!(committed(folder.as_path()), None);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
