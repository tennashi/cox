//! Cox asks `claude-agent-acp` for its async tasks as the connection
//! starts, and hands over each task's start, with what it is, what it
//! does anew, and its end, as the agent tells them, while no turn runs
//! too.

use std::path::Path;
use std::time::Duration;

/// An agent that writes each request it gets to `log`, opens a session,
/// and then, with no prompt to answer, tells of a task, says something,
/// tells what the task does anew, tells of updates Cox does not know,
/// and ends the task.
fn agent(log: &Path) -> impl cox_acp::Transport {
    let update = |update: &str| {
        format!(
            r#"echo '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s","update":{update}}}}}'"#
        )
    };
    let script = format!(
        r#"while read -r request; do
  printf '%s\n' "$request" >> '{log}'
  id=$(printf '%s' "$request" | sed 's/.*"id":\("[^"]*"\).*/\1/')
  case "$request" in
    *'"method":"initialize"'*)
      echo '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}' ;;
    *'"method":"session/new"'*)
      echo '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"sessionId":"s"}}}}'
      {spawned}
      {progress}
      {described}
      {said}
      {unknown}
      {ended} ;;
  esac
done"#,
        log = log.display(),
        spawned = update(
            r#"{"sessionUpdate":"async_task_spawned","asyncTaskId":"ci","name":"Background task","taskType":"local_bash","description":"gh pr checks --watch","showInTranscript":true,"canStop":true}"#
        ),
        progress = update(
            r#"{"sessionUpdate":"async_task_progress","asyncTaskId":"ci","summary":"waiting"}"#
        ),
        described = update(
            r#"{"sessionUpdate":"async_task_progress","asyncTaskId":"ci","description":"gh pr checks 418 --watch"}"#
        ),
        said = update(
            r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"CI passed."}}"#
        ),
        unknown = update(r#"{"sessionUpdate":"something_new","what":1}"#),
        ended = update(
            r#"{"sessionUpdate":"async_task_state_update","asyncTaskId":"ci","state":"completed"}"#
        ),
    );
    cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script]).unwrap()
}

/// What an update is, in short.
fn told(update: cox_acp::Update) -> String {
    match update {
        cox_acp::Update::Task(task) => format!(
            "task {} running {} started {} {:?} {:?} {:?}",
            task.id, task.running, task.started, task.name, task.kind, task.description
        ),
        cox_acp::Update::ReplyText(text) => format!("reply {text}"),
        _ => "something else".to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tasks_the_agent_runs_by_itself_start_and_end() {
    let folder = tempfile::TempDir::new().unwrap();
    let log = folder.path().join("requests");
    let opening = cox_acp::Opening {
        cwd: folder.path().to_owned(),
        ..cox_acp::Opening::default()
    };
    let connected = cox_acp::connect(
        agent(&log),
        opening,
        |_| {},
        async |_, _| anyhow::bail!("no login"),
        async |mut session| {
            let mut updates = Vec::new();
            for _ in 0..4 {
                updates.push(told(session.next_unprompted().await.unwrap()));
            }
            Ok(updates)
        },
    );
    let updates = tokio::time::timeout(Duration::from_secs(30), connected)
        .await
        .expect("the agent's updates come in time")
        .unwrap();

    assert_eq!(
        updates,
        [
            r#"task ci running true started true Some("Background task") Some("local_bash") Some("gh pr checks --watch")"#,
            r#"task ci running true started false None None Some("gh pr checks 418 --watch")"#,
            "reply CI passed.",
            "task ci running false started false None None None"
        ]
    );
    let initialize = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|request| request["method"] == "initialize")
        .expect("an initialize request");
    assert_eq!(
        initialize["params"]["clientCapabilities"]["_meta"]["jetbrains"]["air"],
        serde_json::json!({ "version": 1, "capabilities": ["asyncTasks"] })
    );
}
