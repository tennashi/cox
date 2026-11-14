//! A trigger as the stage's Triggers tab edits it on its card: a
//! [`Draft`], what the card says, which Cox writes to the trigger's file
//! once the human saves it (see [`Save`]), and which the human can try on
//! the last turn of a chat in the folder first (see [`crate::trigger::discuss`]).

use std::path::{Path, PathBuf};

use super::{Arranged, By, Change, InSketch, Scope, Trigger};

/// What the card of a trigger says, field by field, as the page edits it.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Draft {
    /// Its name, which its file is named after.
    pub name: String,
    /// Where it is kept: the project's, or the human's.
    pub scope: Option<Scope>,
    /// The event it runs on, as `on` names it: `turn.end`, `goal.met`,
    /// `wiki.written`, `sketch.end`, `chat.open`, `plan.changed`,
    /// `permission.asked` or `schedule`.
    pub on: String,
    /// The globs of the pages whose writing runs it, on `wiki.written`,
    /// or, empty, any.
    #[serde(default)]
    pub pages: Vec<String>,
    /// The times a schedule runs, as cron writes them.
    #[serde(default)]
    pub cron: String,
    /// The project a schedule's chat works in, by its name, or else,
    /// empty, the home.
    #[serde(default)]
    pub project: String,
    /// The globs whose files, changed, run it, or, empty, anything.
    #[serde(default)]
    pub changed: Vec<String>,
    /// Whether it runs in a sketch, or, `None`, in either.
    #[serde(default)]
    pub sketch: Option<InSketch>,
    /// Who runs it: `shell` or `new`.
    pub by: String,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub reject: Vec<String>,
    /// What is done.
    #[serde(default)]
    pub text: String,
}

impl Draft {
    /// The card of `trigger`, as its file has it.
    #[must_use]
    pub fn of(trigger: &Trigger) -> Self {
        let cron = trigger.cron().map(ToString::to_string).unwrap_or_default();
        let (by, allow, reject, project) = match &trigger.by {
            By::Shell => ("shell", Vec::new(), Vec::new(), None),
            By::This => ("this", Vec::new(), Vec::new(), None),
            By::New {
                allow,
                reject,
                project,
            } => ("new", allow.clone(), reject.clone(), project.clone()),
        };
        Self {
            name: trigger.name.clone(),
            scope: Some(trigger.scope),
            on: trigger
                .on
                .first()
                .map_or("turn.end", super::On::name)
                .to_owned(),
            pages: trigger
                .on
                .iter()
                .find_map(|on| match on {
                    super::On::WikiWritten { pages } => pages.clone(),
                    _ => None,
                })
                .unwrap_or_default(),
            cron,
            project: project.unwrap_or_default(),
            changed: trigger.when.changed.clone().unwrap_or_default(),
            sketch: trigger.when.sketch,
            by: by.to_owned(),
            allow,
            reject,
            text: trigger.text.clone(),
        }
    }

    /// The text of its file, its properties between `---` lines at its
    /// top, as YAML writes them, and what is done under them.
    #[must_use]
    pub fn written(&self) -> String {
        let mut lines = vec!["---".to_owned()];
        if self.on == "schedule" {
            lines.push("on:".to_owned());
            lines.push("  schedule:".to_owned());
            lines.push(format!("    cron: {}", quoted(self.cron.trim())));
        } else if self.on == "wiki.written" && self.pages.iter().any(|page| !page.trim().is_empty())
        {
            let pages: Vec<&String> = self
                .pages
                .iter()
                .filter(|page| !page.trim().is_empty())
                .collect();
            lines.push("on:".to_owned());
            lines.push("  wiki.written:".to_owned());
            lines.push(format!("    page: {}", listed(&pages)));
        } else {
            lines.push(format!("on: {}", self.on.trim()));
        }
        let changed: Vec<&String> = self
            .changed
            .iter()
            .filter(|glob| !glob.trim().is_empty())
            .collect();
        if !changed.is_empty() || self.sketch.is_some() {
            lines.push("when:".to_owned());
            if !changed.is_empty() {
                lines.push(format!("  changed: {}", listed(&changed)));
            }
            match self.sketch {
                Some(InSketch::Skip) => lines.push("  sketch: skip".to_owned()),
                Some(InSketch::Only) => lines.push("  sketch: only".to_owned()),
                None => {}
            }
        }
        lines.push(format!("by: {}", self.by.trim()));
        if !self.project.trim().is_empty() {
            lines.push(format!("project: {}", self.project.trim()));
        }
        for (key, list) in [("allow", &self.allow), ("reject", &self.reject)] {
            let list: Vec<&String> = list.iter().filter(|item| !item.trim().is_empty()).collect();
            if !list.is_empty() {
                lines.push(format!("{key}: {}", listed(&list)));
            }
        }
        lines.push("---".to_owned());
        format!("{}\n\n{}\n", lines.join("\n"), self.text.trim())
    }

    /// The trigger it makes, as it would be read from `file`, or why it
    /// makes none.
    ///
    /// # Errors
    ///
    /// When its name is no file's, or what it says is no trigger.
    pub fn trigger(&self, file: &Path) -> Result<Trigger, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Give it a name.".to_owned());
        }
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            || name.starts_with('.')
        {
            return Err(format!(
                "{name:?} is no file's name: letters, digits, - and _ only"
            ));
        }
        Trigger::parse(
            name,
            file,
            self.scope.unwrap_or(Scope::Project),
            &self.written(),
        )
    }
}

/// `items` as a list on one line, as `["one", "two"]`.
fn listed(items: &[&String]) -> String {
    let items: Vec<String> = items.iter().map(|item| quoted(item.trim())).collect();
    format!("[{}]", items.join(", "))
}

/// `value` in quotes, as YAML takes any text so.
fn quoted(value: &str) -> String {
    if value.contains('"') {
        format!("'{value}'")
    } else {
        format!("\"{value}\"")
    }
}

/// What the card saves: the trigger of `file`, if it has one yet, shown
/// in `folder`, as `draft` says.
#[derive(Debug, serde::Deserialize)]
pub struct Save {
    pub folder: PathBuf,
    #[serde(default)]
    pub file: Option<PathBuf>,
    pub draft: Draft,
}

impl Save {
    /// Writes the trigger to its file, in `triggers` in the folder
    /// for the project's, or in `triggers` in the human's `workflow`: the
    /// file it has, while it stays where it is and is named so, or else
    /// another, the one it had taken away. Answers the file. A project's
    /// own runs in the folder as written, as `arranged` keeps it (see
    /// [`Arranged`]), when the card made it or the human had turned it on.
    ///
    /// # Errors
    ///
    /// When the draft makes no trigger, another has its name, or the file
    /// cannot be written.
    pub fn make(self, workflow: &Path, arranged: Option<&Path>) -> Result<PathBuf, String> {
        let scope = self.draft.scope.unwrap_or(Scope::Project);
        let name = self.draft.name.trim().to_owned();
        let kept = match &self.file {
            Some(file)
                if file.file_stem().and_then(|stem| stem.to_str()) == Some(name.as_str())
                    && Self::scope_of(file, &self.folder, workflow) == Some(scope) =>
            {
                file.clone()
            }
            _ => {
                let folder = match scope {
                    Scope::Project => self.folder.join(super::PROJECT_TRIGGERS),
                    Scope::Global => workflow.join(super::TRIGGERS),
                };
                let file = folder.join(format!("{name}.md"));
                if file.exists() {
                    return Err(format!("A trigger named {name} is there already."));
                }
                file
            }
        };
        let mut draft = self.draft;
        draft.name = name;
        draft.scope = Some(scope);
        let trigger = draft.trigger(&kept)?;
        crate::storage::write_whole(&kept, draft.written())
            .map_err(|error| format!("cannot write {}: {error}", kept.display()))?;
        let runs = self
            .file
            .as_ref()
            .is_none_or(|file| Arranged::read(arranged, &self.folder).on.contains_key(file));
        if let Some(file) = self.file.filter(|file| *file != kept) {
            let _ = std::fs::remove_file(file);
        }
        if scope == Scope::Project && runs {
            Change::Switch {
                folder: self.folder,
                file: kept.clone(),
                on: true,
                sum: Some(trigger.sum),
            }
            .make(arranged)?;
        }
        Ok(kept)
    }

    /// Where `file`, a trigger shown in `folder`, is kept.
    fn scope_of(file: &Path, folder: &Path, workflow: &Path) -> Option<Scope> {
        if file.starts_with(folder.join(super::PROJECT_TRIGGERS)) {
            Some(Scope::Project)
        } else if file.starts_with(workflow) {
            Some(Scope::Global)
        } else {
            None
        }
    }
}

/// Keeps what a chat in `folder` said last as its turn ended, `said`, in
/// `kept`, for a trigger to be tried on.
///
/// # Errors
///
/// When `kept` cannot be written.
pub fn keep_last_turn(
    kept: Option<&Path>,
    folder: &Path,
    said: Option<&str>,
) -> Result<(), String> {
    let path = kept.ok_or("Cox cannot tell where it keeps its state")?;
    let mut all = super::read_kept::<Option<String>>(Some(path));
    all.insert(folder.to_owned(), said.map(str::to_owned));
    super::write_kept(path, &all)
}

/// What a chat in `folder` said last as its turn ended, as `kept` has it:
/// `None` when no turn ended there, and `Some(None)` when it said nothing.
#[must_use]
pub fn last_turn(kept: Option<&Path>, folder: &Path) -> Option<Option<String>> {
    super::read_kept::<Option<String>>(kept).remove(folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_card_writes_the_trigger_it_reads() {
        let text = "---\non: turn.end\nwhen:\n  changed: [\"**/*.rs\", \"!target/**\"]\n  sketch: skip\nby: new\nallow: [\"read:*\", \"execute:git diff*\"]\n---\n\nRead the diff.\n";
        let trigger =
            Trigger::parse("simplify", Path::new("/p/s.md"), Scope::Global, text).unwrap();
        let draft = Draft::of(&trigger);
        assert_eq!(draft.on, "turn.end");
        assert_eq!(draft.changed, ["**/*.rs", "!target/**"]);
        assert_eq!(draft.written(), text);
        assert_eq!(draft.trigger(Path::new("/p/s.md")).unwrap(), trigger);

        let schedule = Draft {
            name: "news".to_owned(),
            scope: Some(Scope::Global),
            on: "schedule".to_owned(),
            cron: "0 9 * * 1-5".to_owned(),
            project: "notes".to_owned(),
            by: "new".to_owned(),
            text: "News since {last}.".to_owned(),
            ..Draft::default()
        };
        assert_eq!(
            schedule.written(),
            "---\non:\n  schedule:\n    cron: \"0 9 * * 1-5\"\nby: new\nproject: notes\n---\n\nNews since {last}.\n"
        );
        let trigger = schedule.trigger(Path::new("/w/triggers/news.md")).unwrap();
        assert_eq!(Draft::of(&trigger), schedule);

        let wiki = Draft {
            name: "sources".to_owned(),
            scope: Some(Scope::Global),
            on: "wiki.written".to_owned(),
            pages: vec!["research/*".to_owned()],
            by: "this".to_owned(),
            text: "Check the page names its sources.".to_owned(),
            ..Draft::default()
        };
        assert_eq!(
            wiki.written(),
            "---\non:\n  wiki.written:\n    page: [\"research/*\"]\nby: this\n---\n\nCheck the page names its sources.\n"
        );
        let trigger = wiki.trigger(Path::new("/w/triggers/sources.md")).unwrap();
        assert_eq!(Draft::of(&trigger), wiki);

        let bad = Draft {
            name: "../x".to_owned(),
            ..schedule.clone()
        };
        assert!(
            bad.trigger(Path::new("/x.md"))
                .unwrap_err()
                .contains("name")
        );
        let bad = Draft {
            by: "shell".to_owned(),
            ..schedule
        };
        assert!(
            bad.trigger(Path::new("/x.md"))
                .unwrap_err()
                .contains("by: new")
        );
    }

    #[test]
    fn saving_a_card_writes_its_file_where_it_is_kept() {
        let root = TempDir::new();
        let (project, workflow) = (root.join("app"), root.join("workflow"));
        let draft = Draft {
            name: "ci".to_owned(),
            scope: Some(Scope::Project),
            on: "turn.end".to_owned(),
            by: "shell".to_owned(),
            text: "cargo test".to_owned(),
            ..Draft::default()
        };
        let arranged = root.join("triggers.json");
        let save = |file: Option<PathBuf>, draft: Draft| {
            Save {
                folder: project.clone(),
                file,
                draft,
            }
            .make(&workflow, Some(&arranged))
        };
        let runs = |file: &Path| {
            let trigger = Trigger::read(file, Scope::Project).unwrap().unwrap();
            Arranged::read(Some(&arranged), &project).runs(&trigger)
        };
        let ci = save(None, draft.clone()).unwrap();
        assert_eq!(ci, project.join("triggers/ci.md"));
        assert!(runs(&ci));
        assert!(save(None, draft.clone()).unwrap_err().contains("already"));
        // Changed where it stays.
        let changed = Draft {
            text: "cargo xtask ci".to_owned(),
            ..draft.clone()
        };
        assert_eq!(save(Some(ci.clone()), changed.clone()).unwrap(), ci);
        assert!(
            std::fs::read_to_string(&ci)
                .unwrap()
                .contains("cargo xtask ci")
        );
        assert!(runs(&ci));
        let never_turned_on = project.join("triggers/cloned.md");
        std::fs::write(
            &never_turned_on,
            "---\non: turn.end\nby: shell\n---\nmake\n",
        )
        .unwrap();
        let cloned = Draft {
            name: "cloned".to_owned(),
            text: "make all".to_owned(),
            ..draft.clone()
        };
        assert_eq!(
            save(Some(never_turned_on.clone()), cloned).unwrap(),
            never_turned_on
        );
        assert!(!runs(&never_turned_on));
        // Made the human's, it moves to their workflow.
        let global = Draft {
            scope: Some(Scope::Global),
            ..changed
        };
        let moved = save(Some(ci.clone()), global).unwrap();
        assert_eq!(moved, workflow.join("triggers/ci.md"));
        assert!(!ci.exists());
        // One that makes no trigger is not written.
        let empty = Draft {
            name: "empty".to_owned(),
            text: String::new(),
            ..draft
        };
        assert!(save(None, empty).is_err());
        assert!(!project.join("triggers/empty.md").exists());

        let kept = root.join("last-turns.json");
        assert_eq!(last_turn(Some(&kept), &project), None);
        keep_last_turn(Some(&kept), &project, Some("Done.")).unwrap();
        assert_eq!(
            last_turn(Some(&kept), &project),
            Some(Some("Done.".to_owned()))
        );
    }
}
