//! A turn hands over what the agent tells of what its session spends: the
//! context and the session's cost as `usage_update`s come, and the tokens
//! of the turn last, from the answer to its prompt, as
//! `claude-agent-acp` 0.107.3 sends them.

use std::time::Duration;

use cox_acp::{Microdollars, Tokens, Update, Usage};

/// An agent that opens a session, and answers a prompt with a reply, the
/// context as the reply streams, the context and cost as the turn ends,
/// and the turn's tokens in its answer.
fn agent() -> impl cox_acp::Transport {
    let update = |update: &str| {
        format!(
            r#"echo '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s","update":{update}}}}}'"#
        )
    };
    let script = format!(
        r#"while read -r request; do
  id=$(printf '%s' "$request" | sed 's/.*"id":\([^,}}]*\).*/\1/')
  case "$request" in
    *'"method":"initialize"'*)
      echo '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}' ;;
    *'"method":"session/new"'*)
      echo '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"sessionId":"s"}}}}' ;;
    *'"method":"session/prompt"'*)
      {reply}
      {streaming}
      {ended}
      echo '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"stopReason":"end_turn","usage":{{"inputTokens":2,"outputTokens":3,"cachedReadTokens":11219,"cachedWriteTokens":11177,"totalTokens":22401}}}}}}' ;;
  esac
done"#,
        reply = update(
            r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"one"}}"#
        ),
        streaming = update(
            r#"{"sessionUpdate":"usage_update","used":22401,"size":1000000,"_meta":{"_claude/model":"claude-opus-5-5"}}"#
        ),
        ended = update(
            r#"{"sessionUpdate":"usage_update","used":22401,"size":1000000,"cost":{"amount":0.0917278,"currency":"USD"},"_meta":{"_claude/origin":{"kind":"human"},"_claude/model":"claude-opus-5-5"}}"#
        ),
    );
    cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script]).unwrap()
}

/// What an update tells of what the session spends, if anything.
#[derive(Debug, PartialEq, Eq)]
enum Spent {
    Usage(Usage),
    Tokens(Tokens),
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_tells_the_context_the_cost_and_last_its_tokens() {
    let folder = tempfile::TempDir::new().unwrap();
    let opening = cox_acp::Opening {
        cwd: folder.path().to_owned(),
        ..cox_acp::Opening::default()
    };
    let connected = cox_acp::connect(
        agent(),
        opening,
        |_| {},
        async |_, _| anyhow::bail!("no login"),
        async |mut session| {
            let mut turn = session.send([cox_acp::Block::Text("Reply with one".to_owned())]);
            let mut spent = Vec::new();
            while let Some(update) = turn.next().await {
                match update {
                    Update::Usage(usage) => spent.push(Spent::Usage(usage)),
                    Update::TurnTokens(tokens) => spent.push(Spent::Tokens(tokens)),
                    _ => {}
                }
            }
            turn.end().await?;
            Ok(spent)
        },
    );
    let spent = tokio::time::timeout(Duration::from_secs(30), connected)
        .await
        .expect("the turn ends in time")
        .unwrap();

    assert_eq!(
        spent,
        [
            Spent::Usage(Usage {
                used: 22_401,
                size: 1_000_000,
                cost: None,
            }),
            Spent::Usage(Usage {
                used: 22_401,
                size: 1_000_000,
                cost: Some(Microdollars(91_728)),
            }),
            Spent::Tokens(Tokens {
                input: 2,
                output: 3,
                cache_read: 11_219,
                cache_write: 11_177,
            }),
        ]
    );
}
