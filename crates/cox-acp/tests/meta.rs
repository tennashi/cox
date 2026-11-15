//! The `_meta` a session is asked for with goes to the agent with each
//! way of asking for it: a new session, one resumed, and one loaded.

use std::path::Path;
use std::time::Duration;

/// An agent that writes each request it gets to `log`, and answers as
/// one whose capabilities are `capabilities`: it opens, resumes and
/// loads sessions.
fn agent(log: &Path, capabilities: &str) -> impl cox_acp::Transport {
    let script = format!(
        r#"while read -r request; do
  printf '%s\n' "$request" >> '{log}'
  id=$(printf '%s' "$request" | sed 's/.*"id":\("[^"]*"\).*/\1/')
  case "$request" in
    *'"method":"initialize"'*)
      echo '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"protocolVersion":1,"agentCapabilities":{capabilities}}}}}' ;;
    *'"method":"session/new"'*)
      echo '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"sessionId":"s"}}}}' ;;
    *'"method":"session/resume"'*|*'"method":"session/load"'*)
      echo '{{"jsonrpc":"2.0","id":'"$id"',"result":{{}}}}' ;;
  esac
done"#,
        log = log.display()
    );
    cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script]).unwrap()
}

/// The request of `method` the agent got, when Cox opens a session with
/// the `_meta` of [`disallowed`], going on with `resume` if given, with an
/// agent whose capabilities are `capabilities`.
async fn asked(capabilities: &str, resume: Option<&str>, method: &str) -> serde_json::Value {
    let folder = tempfile::TempDir::new().unwrap();
    let log = folder.path().join("requests");
    let opening = cox_acp::Opening {
        cwd: folder.path().to_owned(),
        mcp_servers: Vec::new(),
        resume: resume.map(str::to_owned),
        meta: disallowed().as_object().cloned(),
    };
    let connected = cox_acp::connect(
        agent(&log, capabilities),
        opening,
        |_| {},
        async |_, _| anyhow::bail!("no login"),
        async |_| Ok(()),
    );
    tokio::time::timeout(Duration::from_secs(30), connected)
        .await
        .expect("the session opens in time")
        .unwrap();
    std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|request| request["method"] == method)
        .unwrap_or_else(|| panic!("no {method} request"))
}

/// A `_meta` as Claude Code's agent reads it, turning its own subagents
/// off.
fn disallowed() -> serde_json::Value {
    serde_json::json!({ "claudeCode": { "options": { "disallowedTools": ["Agent", "Task"] } } })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_session_is_asked_for_with_its_meta() {
    let request = asked("{}", None, "session/new").await;
    assert_eq!(request["params"]["_meta"], disallowed());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_resumed_or_loaded_is_asked_for_with_its_meta() {
    let resumed = asked(
        r#"{"sessionCapabilities":{"resume":{}}}"#,
        Some("s"),
        "session/resume",
    )
    .await;
    assert_eq!(resumed["params"]["_meta"], disallowed());
    let loaded = asked(r#"{"loadSession":true}"#, Some("s"), "session/load").await;
    assert_eq!(loaded["params"]["_meta"], disallowed());
}
