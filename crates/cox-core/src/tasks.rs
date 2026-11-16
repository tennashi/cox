//! The tasks a chat's agent broke its work into, as a tree: what each is,
//! how far it went, what it came to or what blocks it, and the chat it
//! handed it to, if it handed it to one, for the human to see the plan of
//! work spread across many chats in one place.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::store::ChatId;

/// The most characters of a task's id.
const ID_LENGTH: usize = 40;

/// What the agent calls a task, unique in its tree, as `store` or
/// `store.tests`: chosen by the agent, so that a task it adds can name
/// another it adds in the same change as its parent.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TaskId(String);

impl TaskId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TaskId {
    type Error = String;

    fn try_from(id: String) -> Result<Self, String> {
        let id = id.trim();
        if id.is_empty() || id.chars().count() > ID_LENGTH || id.chars().any(char::is_whitespace) {
            return Err(format!(
                "`{id}` is no task id: give one of 1 to {ID_LENGTH} characters, without spaces"
            ));
        }
        Ok(Self(id.to_owned()))
    }
}

impl From<TaskId> for String {
    fn from(id: TaskId) -> Self {
        id.0
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// How far a task went. What it came to is only of a task done, and
/// what blocks it only of one blocked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    Pending,
    InProgress,
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<String>,
    },
    Blocked {
        blocker: String,
    },
}

impl Status {
    /// The status as the agent names it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done { .. } => "done",
            Self::Blocked { .. } => "blocked",
        }
    }

    /// The status the agent names `name`, with `note`, what the task
    /// came to or what blocks it, as the status takes one.
    ///
    /// # Errors
    ///
    /// When there is no such status, or `note` is not as it takes it.
    pub fn parse(name: &str, note: Option<String>) -> Result<Self, String> {
        let note = note
            .map(|note| note.trim().to_owned())
            .filter(|note| !note.is_empty());
        match (name, note) {
            ("pending", None) => Ok(Self::Pending),
            ("in_progress", None) => Ok(Self::InProgress),
            ("pending" | "in_progress", Some(_)) => Err(format!(
                "a task {name} takes no note: a note says what a task done came to, or what \
                 blocks a task blocked"
            )),
            ("done", result) => Ok(Self::Done { result }),
            ("blocked", Some(blocker)) => Ok(Self::Blocked { blocker }),
            ("blocked", None) => {
                Err("a task blocked takes a note saying what blocks it".to_owned())
            }
            (other, _) => Err(format!(
                "no status `{other}`: give pending, in_progress, done or blocked"
            )),
        }
    }
}

/// A status a task took, and when, in milliseconds since the epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub at: i64,
    #[serde(flatten)]
    pub status: Status,
}

/// A task of the tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    /// The task it is a part of, or none at the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TaskId>,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(flatten)]
    pub status: Status,
    /// The chat the agent handed it to, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<ChatId>,
    /// Each status it took before the one it has, the first first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Change>,
}

/// What the agent says of a task: a new one, which needs a title, or one
/// of the tree, of which it changes what it gives.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Said {
    pub id: Option<TaskId>,
    pub title: Option<String>,
    pub detail: Option<String>,
    pub parent: Option<TaskId>,
    pub status: Option<Status>,
}

/// The tasks of a chat, each after its parent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tree {
    tasks: Vec<Task>,
}

impl Tree {
    #[must_use]
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Whether every task is done.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.tasks
            .iter()
            .all(|task| matches!(task.status, Status::Done { .. }))
    }

    #[must_use]
    pub fn task(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.iter().find(|task| &task.id == id)
    }

    /// Adds or changes each task of `said`, in order, at `at`, in
    /// milliseconds since the epoch, all of them or none.
    ///
    /// # Errors
    ///
    /// When a task is not as the tree takes it: a new one with no title,
    /// or a parent the tree has not, or that is the task or under it.
    pub fn apply(&mut self, said: Vec<Said>, at: i64) -> Result<(), String> {
        let mut tree = self.clone();
        for said in said {
            tree.apply_one(said, at)?;
        }
        *self = tree;
        Ok(())
    }

    fn apply_one(&mut self, said: Said, at: i64) -> Result<(), String> {
        let Said {
            id,
            title,
            detail,
            parent,
            status,
        } = said;
        let title = title
            .map(|title| title.trim().to_owned())
            .filter(|title| !title.is_empty());
        let detail = detail.map(|detail| detail.trim().to_owned());
        let id = match id {
            Some(id) => id,
            None => self.free_id(title.as_deref().ok_or("a new task needs a `title`")?),
        };
        if let Some(parent) = &parent {
            if self.task(parent).is_none() {
                return Err(format!("no task `{parent}` to be the parent of `{id}`"));
            }
            if self.is_under(parent, &id) {
                return Err(format!(
                    "`{parent}` cannot be the parent of `{id}`: it is under it"
                ));
            }
        }
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            if let Some(title) = title {
                task.title = title;
            }
            if let Some(detail) = detail {
                task.detail = Some(detail).filter(|detail| !detail.is_empty());
            }
            if let Some(status) = status {
                task.set(status, at);
            }
            if let Some(parent) = parent {
                task.parent = Some(parent);
                self.order();
            }
            return Ok(());
        }
        let title = title.ok_or_else(|| format!("no task `{id}`: a new task needs a `title`"))?;
        self.tasks.push(Task {
            id,
            parent,
            title,
            detail: detail.filter(|detail| !detail.is_empty()),
            status: status.unwrap_or(Status::Pending),
            chat: None,
            history: Vec::new(),
        });
        self.order();
        Ok(())
    }

    /// Hands the task `id` to `chat`, which works on it from `at`.
    ///
    /// # Errors
    ///
    /// When the tree has no task `id`.
    pub fn hand(&mut self, id: &TaskId, chat: ChatId, at: i64) -> Result<(), String> {
        let task = self
            .tasks
            .iter_mut()
            .find(|task| &task.id == id)
            .ok_or_else(|| format!("no task `{id}`"))?;
        task.chat = Some(chat);
        if !matches!(task.status, Status::InProgress) {
            task.set(Status::InProgress, at);
        }
        Ok(())
    }

    /// Whether `task` is `of`, or under it.
    fn is_under(&self, task: &TaskId, of: &TaskId) -> bool {
        let mut at = Some(task);
        while let Some(id) = at {
            if id == of {
                return true;
            }
            at = self.task(id).and_then(|task| task.parent.as_ref());
        }
        false
    }

    /// An id for a new task titled `title`, made of its first word,
    /// which no task of the tree has.
    fn free_id(&self, title: &str) -> TaskId {
        let word: String = title
            .split_whitespace()
            .next()
            .unwrap_or("task")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(ID_LENGTH - 4)
            .collect::<String>()
            .to_lowercase();
        let word = if word.is_empty() {
            "task".to_owned()
        } else {
            word
        };
        let mut id = word.clone();
        let mut n = 2;
        while self.tasks.iter().any(|task| task.id.0 == id) {
            id = format!("{word}-{n}");
            n += 1;
        }
        TaskId(id)
    }

    /// Puts each task after its parent, and after the tasks before it
    /// with the same parent, as they were.
    fn order(&mut self) {
        fn take(tasks: &mut Vec<Task>, parent: Option<&TaskId>, ordered: &mut Vec<Task>) {
            let (children, rest): (Vec<Task>, Vec<Task>) = std::mem::take(tasks)
                .into_iter()
                .partition(|task| task.parent.as_ref() == parent);
            *tasks = rest;
            for child in children {
                let id = child.id.clone();
                ordered.push(child);
                take(tasks, Some(&id), ordered);
            }
        }
        let mut ordered = Vec::with_capacity(self.tasks.len());
        let mut tasks = std::mem::take(&mut self.tasks);
        take(&mut tasks, None, &mut ordered);
        self.tasks = ordered;
    }
}

impl Task {
    fn set(&mut self, status: Status, at: i64) {
        if self.status == status {
            return;
        }
        let was = std::mem::replace(&mut self.status, status);
        self.history.push(Change { at, status: was });
    }

    /// How deep it is in `tree`: 0 at the top.
    #[must_use]
    pub fn depth(&self, tree: &Tree) -> usize {
        let mut depth = 0;
        let mut at = self.parent.as_ref();
        while let Some(parent) = at {
            depth += 1;
            at = tree.task(parent).and_then(|task| task.parent.as_ref());
        }
        depth
    }
}

/// The tree as the agent reads it: each task on a line, indented under
/// its parent, with its id, its status, and what it came to or what
/// blocks it, the chat it was handed to as `chat` names it.
#[must_use]
pub fn told(tree: &Tree, chat: &dyn Fn(ChatId) -> String) -> String {
    if tree.is_empty() {
        return "The tree has no tasks.".to_owned();
    }
    let mut told = String::new();
    for task in tree.tasks() {
        let indent = "  ".repeat(task.depth(tree));
        let mark = match task.status {
            Status::Pending => "[ ]",
            Status::InProgress => "[~]",
            Status::Done { .. } => "[x]",
            Status::Blocked { .. } => "[!]",
        };
        let _ = write!(told, "{indent}{mark} {}: {}", task.id, task.title);
        if let Some(handed) = task.chat {
            let _ = write!(told, " (chat {})", chat(handed));
        }
        match &task.status {
            Status::Done {
                result: Some(result),
            } => {
                let _ = write!(told, " → {result}");
            }
            Status::Blocked { blocker } => {
                let _ = write!(told, " — blocked: {blocker}");
            }
            _ => {}
        }
        told.push('\n');
    }
    told
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(id: &str) -> TaskId {
        TaskId::try_from(id.to_owned()).unwrap()
    }

    fn new(task: &str, title: &str, parent: Option<&str>) -> Said {
        Said {
            id: Some(id(task)),
            title: Some(title.to_owned()),
            parent: parent.map(id),
            ..Said::default()
        }
    }

    fn status(task: &str, status: Status) -> Said {
        Said {
            id: Some(id(task)),
            status: Some(status),
            ..Said::default()
        }
    }

    #[test]
    fn a_task_added_under_another_follows_it_and_starts_pending() {
        let mut tree = Tree::default();
        tree.apply(
            vec![
                new("store", "Keep the tree", None),
                new("ui", "Show the tree", None),
                new("tests", "Test the store", Some("store")),
            ],
            1,
        )
        .unwrap();
        let order: Vec<&str> = tree.tasks().iter().map(|task| task.id.as_str()).collect();
        assert_eq!(order, ["store", "tests", "ui"]);
        assert_eq!(tree.task(&id("tests")).unwrap().status, Status::Pending);
        assert_eq!(tree.task(&id("tests")).unwrap().depth(&tree), 1);
    }

    #[test]
    fn a_change_that_fails_leaves_the_tree_as_it_was() {
        let mut tree = Tree::default();
        tree.apply(vec![new("a", "A", None)], 1).unwrap();
        let was = tree.clone();
        let error = tree
            .apply(vec![new("b", "B", None), new("c", "C", Some("nope"))], 2)
            .unwrap_err();
        assert!(error.contains("nope"), "{error}");
        assert_eq!(tree, was);
    }

    #[test]
    fn a_task_cannot_go_under_itself() {
        let mut tree = Tree::default();
        tree.apply(vec![new("a", "A", None), new("b", "B", Some("a"))], 1)
            .unwrap();
        let moved = Said {
            id: Some(id("a")),
            parent: Some(id("b")),
            ..Said::default()
        };
        assert!(tree.apply(vec![moved], 2).is_err());
    }

    #[test]
    fn keeps_each_status_a_task_left_and_saying_one_again_changes_nothing() {
        let mut tree = Tree::default();
        tree.apply(vec![new("a", "A", None)], 1).unwrap();
        tree.apply(vec![status("a", Status::InProgress)], 2)
            .unwrap();
        let done = Status::Done {
            result: Some("merged".to_owned()),
        };
        tree.apply(vec![status("a", done.clone())], 3).unwrap();
        tree.apply(vec![status("a", done.clone())], 4).unwrap();
        let task = tree.task(&id("a")).unwrap();
        assert_eq!(task.status, done);
        assert_eq!(
            task.history,
            [
                Change {
                    at: 2,
                    status: Status::Pending
                },
                Change {
                    at: 3,
                    status: Status::InProgress
                },
            ]
        );
        assert!(tree.is_done());
    }

    #[test]
    fn a_new_task_without_an_id_gets_one_from_its_title() {
        let mut tree = Tree::default();
        let untitled = |title: &str| Said {
            title: Some(title.to_owned()),
            ..Said::default()
        };
        tree.apply(vec![untitled("Build it"), untitled("Build again")], 1)
            .unwrap();
        let ids: Vec<&str> = tree.tasks().iter().map(|task| task.id.as_str()).collect();
        assert_eq!(ids, ["build", "build-2"]);
        assert!(tree.apply(vec![Said::default()], 2).is_err());
    }

    #[test]
    fn handing_a_task_to_a_chat_puts_it_in_progress() {
        let mut tree = Tree::default();
        tree.apply(vec![new("a", "A", None)], 1).unwrap();
        let chat: ChatId = serde_json::from_str("7").unwrap();
        tree.hand(&id("a"), chat, 2).unwrap();
        let task = tree.task(&id("a")).unwrap();
        assert_eq!(task.chat, Some(chat));
        assert_eq!(task.status, Status::InProgress);
        assert!(tree.hand(&id("b"), chat, 3).is_err());
    }

    #[test]
    fn a_note_goes_only_with_done_or_blocked() {
        assert_eq!(
            Status::parse("blocked", Some(" no disk ".to_owned())),
            Ok(Status::Blocked {
                blocker: "no disk".to_owned()
            })
        );
        assert!(Status::parse("blocked", None).is_err());
        assert!(Status::parse("pending", Some("x".to_owned())).is_err());
        assert_eq!(
            Status::parse("done", None),
            Ok(Status::Done { result: None })
        );
        assert!(Status::parse("finished", None).is_err());
    }

    #[test]
    fn reads_back_as_it_was_written() {
        let mut tree = Tree::default();
        tree.apply(
            vec![
                new("a", "A", None),
                status(
                    "a",
                    Status::Blocked {
                        blocker: "waits on CI".to_owned(),
                    },
                ),
            ],
            1,
        )
        .unwrap();
        let json = serde_json::to_string(&tree).unwrap();
        assert_eq!(serde_json::from_str::<Tree>(&json).unwrap(), tree);
        assert!(
            serde_json::from_str::<Tree>(r#"[{"id":"a","title":"A","status":"blocked"}]"#).is_err()
        );
    }

    #[test]
    fn tells_the_agent_the_tree_indented_with_notes() {
        let mut tree = Tree::default();
        tree.apply(
            vec![
                new("a", "A", None),
                new("b", "B", Some("a")),
                status(
                    "b",
                    Status::Done {
                        result: Some("ok".to_owned()),
                    },
                ),
            ],
            1,
        )
        .unwrap();
        let chat: ChatId = serde_json::from_str("9").unwrap();
        tree.hand(&id("a"), chat, 2).unwrap();
        assert_eq!(
            told(&tree, &|chat| format!("#{chat}")),
            "[~] a: A (chat #9)\n  [x] b: B → ok\n"
        );
    }
}
