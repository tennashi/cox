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

/// The folder of the human's workflow that holds their triggers.
pub const TRIGGERS: &str = "triggers";

/// The folder of a project that holds its own triggers.
pub const PROJECT_TRIGGERS: &str = ".cox/triggers";

/// Where a trigger is kept, and so where it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The project's own, in its folder.
    Project,
    /// The human's, in their workflow's folder, which runs in every
    /// project.
    Global,
}

/// What a trigger runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum On {
    /// A chat's turn ends, as its agent ends it: `turn.end`.
    #[serde(rename = "turn.end")]
    TurnEnd,
}

impl On {
    fn parse(text: &str) -> Result<Self, String> {
        match text {
            "turn.end" => Ok(Self::TurnEnd),
            other => Err(format!("{other:?} is no event Cox knows, as turn.end")),
        }
    }
}

/// Whether a trigger runs in a sketch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
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
    /// An agent Cox starts for it, as the judge of a goal: not a chat, so
    /// not in the list, which ends its reply with `PASS`, or `FAIL: ` and
    /// why.
    New {
        /// The patterns of the permission requests it goes on with, as
        /// `open_chat`'s `allow` takes them, or, when empty, what the judge
        /// of a goal may.
        allow: Vec<String>,
        /// The patterns of those it is refused.
        reject: Vec<String>,
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
        let on = key(&mut properties, "on")
            .ok_or("it says nothing of when it runs, as on: turn.end")?
            .values()
            .map_err(|why| format!("on {why}"))?
            .iter()
            .map(|on| On::parse(on))
            .collect::<Result<Vec<_>, _>>()?;
        let when = match key(&mut properties, "when") {
            None => When::default(),
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
        let by = match key(&mut properties, "by")
            .ok_or("it says nothing of who runs it, as by: shell")?
            .one()
            .map_err(|why| format!("by {why}"))?
        {
            "shell" if allow.is_empty() && reject.is_empty() => By::Shell,
            "shell" => {
                return Err("allow and reject are for an agent's permission requests, \
                     by: new, and a shell asks none"
                    .to_owned());
            }
            "new" => By::New { allow, reject },
            "this" => {
                return Err("by: this does not run on turn.end, which tells the agent \
                     what the triggers found already"
                    .to_owned());
            }
            other => return Err(format!("by: {other:?} is none of shell and new")),
        };
        if let Some(other) = properties.keys().next() {
            return Err(format!("{other:?} is no property Cox knows"));
        }
        let text = rest.trim().to_owned();
        if text.is_empty() {
            return Err(match by {
                By::Shell => "it has no command to run under its properties".to_owned(),
                By::New { .. } => "it asks nothing under its properties".to_owned(),
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

    /// Whether it runs on `on`, in a chat that is a sketch as `sketch`
    /// says, whatever changed.
    #[must_use]
    pub fn runs_on(&self, on: On, sketch: bool) -> bool {
        self.on.contains(&on)
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
            one_or_list(value)
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
            let dedented: Vec<&str> = under
                .iter()
                .filter(|line| !line.trim().is_empty())
                .map(|line| line.trim_start())
                .collect();
            Value::Under(properties(&dedented)?)
        };
        found.insert(key.trim().to_owned(), value);
    }
    Ok(found)
}

/// `value`, as its line has it after the key: a list on the line, as
/// `[one, two]`, or one value.
fn one_or_list(value: &str) -> Value {
    match value
        .strip_prefix('[')
        .and_then(|items| items.strip_suffix(']'))
    {
        Some(items) => Value::List(
            items
                .split(',')
                .map(|item| unquoted(item).to_owned())
                .filter(|item| !item.is_empty())
                .collect(),
        ),
        None => Value::One(unquoted(value).to_owned()),
    }
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
    let (mut triggers, mut unread) = read_all(&folder.join(PROJECT_TRIGGERS), Scope::Project);
    let (global, also) = read_all(&workflow.join(TRIGGERS), Scope::Global);
    triggers.extend(global);
    unread.extend(also);
    (triggers, unread)
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

/// What the stage's Triggers tab is told, as `cox://api/triggers`: the
/// triggers of each folder it shows, `folders`, by its path and name, in
/// the order they run in there, each with what its file says, whether it
/// runs there, and how it last ran there; and why each Cox cannot read
/// does not run. `arranged` and `runs` are the files Cox keeps how the
/// triggers are arranged and how they ran in.
#[must_use]
pub fn told(
    folders: &[(PathBuf, Option<String>)],
    workflow: &Path,
    arranged: Option<&Path>,
    runs: Option<&Path>,
) -> serde_json::Value {
    let mut unread = BTreeSet::new();
    let folders: Vec<serde_json::Value> = folders
        .iter()
        .map(|(folder, name)| {
            let (triggers, why) = of(folder, workflow);
            unread.extend(why);
            let ran = Ran::read(runs, folder);
            let triggers: Vec<serde_json::Value> = Arranged::read(arranged, folder)
                .apply(triggers)
                .into_iter()
                .map(|(trigger, on)| {
                    let (by, allow, reject) = match &trigger.by {
                        By::Shell => ("shell", Vec::new(), Vec::new()),
                        By::New { allow, reject } => ("new", allow.clone(), reject.clone()),
                    };
                    serde_json::json!({
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
                        "ran": ran.get(&trigger.file),
                    })
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
        "workflow": workflow.join(TRIGGERS),
        "now": jiff::Timestamp::now().as_second(),
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
    let Ok(entries) = std::fs::read_dir(folder) else {
        return (Vec::new(), Vec::new());
    };
    let mut triggers = Vec::new();
    let mut unread = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        match Trigger::read(&entry.path(), scope) {
            Ok(trigger) => triggers.extend(trigger),
            Err(why) => unread.push(why),
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

        let told = told(
            &[(project.clone(), None)],
            &workflow,
            Some(&arranged),
            Some(&runs),
        );
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
        assert!(
            parse("---\non: turn.end\nby: this\n---\nhi\n")
                .unwrap_err()
                .contains("this")
        );
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
        assert!(
            parse("---\non: turn.end\nby: shell\nevery: daily\n---\nmake\n")
                .unwrap_err()
                .contains("every")
        );
    }

    #[test]
    fn runs_in_a_sketch_as_it_says() {
        let skip =
            parse("---\non: turn.end\nwhen:\n  sketch: skip\nby: shell\n---\nmake\n").unwrap();
        assert!(skip.runs_on(On::TurnEnd, false));
        assert!(!skip.runs_on(On::TurnEnd, true));
        let only =
            parse("---\non: turn.end\nwhen:\n  sketch: only\nby: shell\n---\nmake\n").unwrap();
        assert!(!only.runs_on(On::TurnEnd, false));
        assert!(only.runs_on(On::TurnEnd, true));
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
