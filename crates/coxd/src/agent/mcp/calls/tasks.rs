//! `task_tree`: the tasks the agent broke its work into, as a tree it
//! keeps as it goes, which the human sees on the Activity page, each task
//! with how far it went and the chat it was handed to.

use cox_core::tasks::{Said, Status, TaskId};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::agent::mcp::{ChatRequest, TasksChange};

pub(super) const TASK_TREE: &str = "task_tree";

pub(super) fn tool() -> Value {
    json!({
        "name": TASK_TREE,
        "description": "Keeps the plan of your work as a tree of tasks the human sees on the \
            Activity page, beside the chats at work: what you broke the work into, which \
            subagent's chat each part was handed to, what is done, what is blocked, and what \
            has not started. Each task in `tasks` is added, or changed if its id is in the \
            tree already: a new one needs a `title`; give only what changes of one there. \
            All of them change, or none, and the answer is the tree as it is then. Call it \
            with no tasks to read the tree. To hand a task to a subagent, give its id as \
            `task` to agent, which puts it in progress and links it to the subagent's chat; \
            once the subagent reports, mark the task done with what it came to, or blocked \
            with what blocks it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "A short name for the task, unique in the \
                                    tree, without spaces, as `store` or `store-tests`: made \
                                    from the title when a new task has none.",
                            },
                            "title": { "type": "string", "description": "What the task is, in a line." },
                            "detail": { "type": "string", "description": "More on it, if needed." },
                            "parent": {
                                "type": "string",
                                "description": "The id of the task it is a part of, added \
                                    before it, in this call or an earlier one.",
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "done", "blocked"],
                                "description": "How far it went: pending for a new task \
                                    unless given.",
                            },
                            "note": {
                                "type": "string",
                                "description": "With done, what the task came to, in a \
                                    line; with blocked, what blocks it, which it needs. \
                                    Not given with the others.",
                            },
                        },
                    },
                },
            },
        },
    })
}

/// Changes the chat's tree as `arguments` say, and tells the agent the
/// tree then.
pub(super) async fn call(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let said = match arguments.get("tasks") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(tasks)) => tasks.iter().map(said).collect::<Result<_, _>>()?,
        Some(_) => return Err("`tasks` is not an array".to_owned()),
    };
    change(TasksChange::Say(said), chats).await
}

/// Hands the task `task` to the chat that goes by `chat`, which the agent
/// opened for it, and answers the tree then.
pub(super) async fn hand(
    task: TaskId,
    chat: u64,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    change(TasksChange::Hand { task, chat }, chats).await
}

async fn change(
    change: TasksChange,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let (answer, tree) = oneshot::channel();
    chats
        .send(ChatRequest::Tasks { change, answer })
        .map_err(|_| "the chat has closed".to_owned())?;
    tree.await
        .map_err(|_| "the chat closed before it kept the tasks".to_owned())?
}

/// The id `value` gives, if it gives one.
pub(super) fn id(value: Option<&Value>, name: &str) -> Result<Option<TaskId>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => TaskId::try_from(id.clone()).map(Some),
        Some(_) => Err(format!("`{name}` is not a string")),
    }
}

/// What the agent says of a task, as `task` gives it.
fn said(task: &Value) -> Result<Said, String> {
    let text = |name: &str| match task.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(format!("a task's `{name}` is not a string")),
    };
    let note = text("note")?;
    let status = match text("status")? {
        Some(status) => Some(Status::parse(status.trim(), note)?),
        None if note.is_some() => {
            return Err("a note goes with a status, done or blocked".to_owned());
        }
        None => None,
    };
    Ok(Said {
        id: id(task.get("id"), "id")?,
        title: text("title")?,
        detail: text("detail")?,
        parent: id(task.get("parent"), "parent")?,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calls the tool with `arguments`, answering its request with the
    /// change it asked for, as the daemon gets it, and `tree`.
    async fn call_with(
        arguments: Value,
        tree: &str,
    ) -> (Result<String, String>, Option<TasksChange>) {
        let (chats, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let tree = tree.to_owned();
        let answering = tokio::spawn(async move {
            let Some(ChatRequest::Tasks { change, answer }) = requests.recv().await else {
                return None;
            };
            answer.send(Ok(tree)).ok();
            Some(change)
        });
        let told = call(&arguments, &chats).await;
        drop(chats);
        (told, answering.await.unwrap())
    }

    #[tokio::test]
    async fn asks_the_chat_to_add_and_change_tasks_and_tells_the_tree() {
        let (told, change) = call_with(
            json!({ "tasks": [
                { "id": "ui", "title": "Show it" },
                { "id": "store", "status": "blocked", "note": "no disk" },
            ] }),
            "[ ] ui: Show it\n",
        )
        .await;
        assert_eq!(told.unwrap(), "[ ] ui: Show it\n");
        let Some(TasksChange::Say(said)) = change else {
            panic!("asked {change:?}");
        };
        assert_eq!(said[0].title.as_deref(), Some("Show it"));
        assert_eq!(
            said[1].status,
            Some(Status::Blocked {
                blocker: "no disk".to_owned()
            })
        );
    }

    #[tokio::test]
    async fn reads_the_tree_when_given_no_tasks() {
        let (told, change) = call_with(json!({}), "The tree has no tasks.").await;
        assert_eq!(told.unwrap(), "The tree has no tasks.");
        assert_eq!(change, Some(TasksChange::Say(Vec::new())));
    }

    #[tokio::test]
    async fn refuses_a_task_blocked_without_what_blocks_it_before_asking() {
        for task in [
            json!({ "id": "a", "status": "blocked" }),
            json!({ "id": "a", "note": "why" }),
            json!({ "id": "a b" }),
            json!({ "id": "a", "status": "finished" }),
        ] {
            let (told, change) = call_with(json!({ "tasks": [task] }), "").await;
            assert!(told.is_err(), "{task}");
            assert_eq!(change, None, "{task}");
        }
    }
}
