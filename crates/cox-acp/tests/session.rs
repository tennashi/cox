//! A session with an agent, as Cox holds it, talking to `cox-fake-agent`
//! as it plays scenarios.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use cox_acp::{
    Block, Login, LoginMethod, McpServer, News, PermissionAnswer, PermissionKind, PlanEntry,
    PlanState, Session, Steered, StopReason, ToolCallContent, ToolCallResult, ToolKind, Turn,
    Update,
};
use serde_json::{Value, json};
use tempfile::TempDir;

/// The fake agent's executable, built once for all the tests here, in a
/// folder of its own, so that the build does not wait on the one running
/// the tests.
fn fake_agent() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let output = Command::new(cargo)
            .args([
                "build",
                "--message-format=json",
                "--package",
                "cox-fake-agent",
            ])
            .arg("--manifest-path")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"))
            .arg("--target-dir")
            .arg(Path::new(env!("CARGO_TARGET_TMPDIR")).join("cox-fake-agent"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "cox-fake-agent builds: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|message| {
                (message["reason"] == "compiler-artifact"
                    && message["target"]["name"] == "cox-fake-agent")
                    .then(|| message["executable"].as_str().map(PathBuf::from))
                    .flatten()
            })
            .expect("cargo names the fake agent's executable")
    })
}

/// A scenario written to a folder of its own, which the session works in.
struct Played {
    folder: TempDir,
}

impl Played {
    fn new(scenario: &Value) -> Self {
        let folder = TempDir::new().unwrap();
        std::fs::write(folder.path().join("scenario.json"), scenario.to_string()).unwrap();
        Self { folder }
    }

    fn scenario(&self) -> PathBuf {
        self.folder.path().join("scenario.json")
    }

    /// The fake agent playing the scenario, launched as Cox launches an
    /// agent.
    fn agent(&self) -> impl cox_acp::Transport {
        cox_acp::launch([
            fake_agent().display().to_string(),
            self.scenario().display().to_string(),
        ])
        .unwrap()
    }

    /// Opens a session with the agent and runs `talk` with it, with no
    /// servers, no session to resume, and no way to log in.
    async fn talk<T>(
        &self,
        talk: impl AsyncFnOnce(Session) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        within(cox_acp::connect(
            self.agent(),
            self.folder.path().to_owned(),
            Vec::new(),
            None,
            |_| {},
            async |_, _| anyhow::bail!("the test logs in no way"),
            talk,
        ))
        .await
    }
}

/// `future`, which is to end well within a generous deadline.
async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), future)
        .await
        .expect("the session ends in time")
}

/// Everything the agent sends in `turn`, until it ends.
async fn all(turn: &mut Turn<'_>) -> Vec<Update> {
    let mut updates = Vec::new();
    while let Some(update) = turn.next().await {
        updates.push(update);
    }
    updates
}

/// The agent's reply among `updates`.
fn replied(updates: &[Update]) -> String {
    updates
        .iter()
        .filter_map(|update| match update {
            Update::ReplyText(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// A scenario of one turn of `steps`.
fn one_turn(steps: &Value) -> Value {
    json!({ "turns": [{ "steps": steps }] })
}

fn text(text: &str) -> Block {
    Block::Text(text.to_owned())
}

#[tokio::test]
async fn a_session_has_the_id_the_agent_gave_it() {
    let played = Played::new(&json!({ "turns": [] }));
    let (id, resumed) = played
        .talk(async |session| Ok((session.id(), session.resumed())))
        .await
        .unwrap();
    assert_eq!((id.as_str(), resumed), ("fake-1", false));
}

#[tokio::test]
async fn an_agent_that_cannot_resume_a_session_opens_a_new_one() {
    let played = Played::new(&json!({ "turns": [] }));
    let (id, resumed) = within(cox_acp::connect(
        played.agent(),
        played.folder.path().to_owned(),
        Vec::new(),
        Some("an-old-session".to_owned()),
        |_| {},
        async |_, _| anyhow::bail!("no login"),
        async |session| Ok((session.id(), session.resumed())),
    ))
    .await
    .unwrap();
    assert_eq!((id.as_str(), resumed), ("fake-1", false));
}

/// The model setting of a scenario: `large` of the group `Big`, or
/// `small` of the group `Little`.
fn model() -> Value {
    json!({
        "id": "model", "name": "Model", "category": "model", "type": "select",
        "currentValue": "large",
        "options": [
            { "group": "big", "name": "Big", "options": [{ "value": "large", "name": "Large" }] },
            { "group": "little", "name": "Little", "options": [{ "value": "small", "name": "Small" }] },
        ],
    })
}

#[tokio::test]
async fn a_session_has_the_settings_the_agent_offers() {
    let played = Played::new(&json!({ "configOptions": [model()], "turns": [] }));
    let settings = played
        .talk(async |session| Ok(session.settings().to_vec()))
        .await
        .unwrap();
    assert_eq!(settings.len(), 1);
    let model = &settings[0];
    assert_eq!(
        (model.id.as_str(), model.name.as_str(), model.value.as_str()),
        ("model", "Model", "large")
    );
    assert_eq!(model.category.as_deref(), Some("model"));
    let choices: Vec<_> = model
        .choices
        .iter()
        .map(|choice| (choice.id.as_str(), choice.group.as_deref()))
        .collect();
    assert_eq!(choices, [("large", Some("Big")), ("small", Some("Little"))]);
}

#[tokio::test]
async fn setting_a_setting_answers_the_settings_as_they_are_then() {
    let played = Played::new(&json!({ "configOptions": [model()], "turns": [] }));
    let (answered, kept) = played
        .talk(async |mut session| {
            let answered = session.set("model", "small").await?;
            Ok((answered, session.settings().to_vec()))
        })
        .await
        .unwrap();
    assert_eq!(answered[0].value, "small");
    assert_eq!(kept, answered);
}

#[tokio::test]
async fn a_setting_the_agent_refuses_is_an_error() {
    let played = Played::new(&json!({ "configOptions": [model()], "turns": [] }));
    let refused = played
        .talk(async |mut session| Ok(session.set("model", "huge").await.is_err()))
        .await
        .unwrap();
    assert!(refused);
}

/// What the agent told of the session as a whole, as `heard` keeps it.
type Heard = Arc<Mutex<Vec<News>>>;

/// Opens a session with the agent of `played`, keeping what it tells of
/// the session in `heard`, and runs `talk` with it.
async fn talk_hearing<T>(
    played: &Played,
    heard: &Heard,
    talk: impl AsyncFnOnce(Session) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    within(cox_acp::connect(
        played.agent(),
        played.folder.path().to_owned(),
        Vec::new(),
        None,
        |news| heard.lock().unwrap().push(news),
        async |_, _| anyhow::bail!("no login"),
        talk,
    ))
    .await
}

#[tokio::test]
async fn the_commands_the_agent_runs_are_told_as_the_session_opens() {
    let played = Played::new(&json!({
        "commands": [
            { "name": "/compact", "description": "Compacts the chat" },
            { "name": "review", "description": "Reviews", "input": { "hint": "what to review" } },
        ],
        "turns": [{ "steps": [{ "say": "Hi." }] }],
    }));
    let heard = Heard::default();
    // A turn taken, the agent has told what it told as the session opened.
    talk_hearing(&played, &heard, async |mut session| {
        session.send([text("hi")]).end().await
    })
    .await
    .unwrap();

    let heard = heard.lock().unwrap();
    let Some(News::Commands(commands)) = heard.first() else {
        panic!("the commands are told: {heard:?}");
    };
    let told: Vec<_> = commands
        .iter()
        .map(|command| (command.name.as_str(), command.hint.as_deref()))
        .collect();
    assert_eq!(
        told,
        [("compact", None), ("review", Some("what to review"))]
    );
}

#[tokio::test]
async fn a_title_the_agent_gives_the_session_is_told() {
    let played = Played::new(&one_turn(&json!([
        { "update": { "sessionUpdate": "session_info_update", "title": "Greeting" } },
    ])));
    let heard = Heard::default();
    talk_hearing(&played, &heard, async |mut session| {
        session.send([text("hi")]).end().await
    })
    .await
    .unwrap();
    assert_eq!(
        *heard.lock().unwrap(),
        [News::Titled("Greeting".to_owned())]
    );
}

#[tokio::test]
async fn settings_the_agent_changes_on_its_own_are_told() {
    let mut small = model();
    small["currentValue"] = json!("small");
    let played = Played::new(&one_turn(&json!([
        { "update": { "sessionUpdate": "config_option_update", "configOptions": [small] } },
    ])));
    let heard = Heard::default();
    talk_hearing(&played, &heard, async |mut session| {
        session.send([text("hi")]).end().await
    })
    .await
    .unwrap();
    let heard = heard.lock().unwrap();
    let Some(News::Settings(settings)) = heard.first() else {
        panic!("the settings are told: {heard:?}");
    };
    assert_eq!(settings[0].value, "small");
}

#[tokio::test]
async fn a_turn_hands_over_what_the_agent_thinks_and_replies_in_order() {
    let played = Played::new(&one_turn(&json!([
        { "think": "Read first." },
        { "say": "It prints hello." },
    ])));
    let (updates, stop) = played
        .talk(async |mut session| {
            let mut turn = session.send([text("what does it print?")]);
            let updates = all(&mut turn).await;
            Ok((updates, turn.end().await?))
        })
        .await
        .unwrap();
    assert!(
        matches!(&updates[0], Update::ThoughtText(thought) if thought == "Read first."),
        "the thought comes first"
    );
    assert_eq!(replied(&updates), "It prints hello.");
    assert_eq!(stop, StopReason::EndTurn);
}

#[tokio::test]
async fn a_turn_ends_for_the_reason_the_agent_says() {
    for (said, reason) in [
        ("end_turn", StopReason::EndTurn),
        ("max_tokens", StopReason::MaxTokens),
        ("max_turn_requests", StopReason::MaxTurnRequests),
        ("refusal", StopReason::Refusal),
    ] {
        let played = Played::new(&json!({ "turns": [{ "steps": [], "stop": said }] }));
        let stop = played
            .talk(async |mut session| session.send([text("go")]).end().await)
            .await
            .unwrap();
        assert_eq!(stop, reason, "{said}");
    }
}

#[tokio::test]
async fn a_turn_the_agent_fails_ends_with_its_error() {
    let played = Played::new(&one_turn(&json!([{ "fail": "the model is down" }])));
    let error = played
        .talk(async |mut session| Ok(session.send([text("go")]).end().await.unwrap_err()))
        .await
        .unwrap();
    assert!(
        format!("{error:#}").contains("the model is down"),
        "{error:#}"
    );
    assert!(!cox_acp::needs_login(&error));
}

#[tokio::test]
async fn the_plan_the_agent_works_to_is_handed_over_whole() {
    let played = Played::new(&one_turn(&json!([{ "update": {
        "sessionUpdate": "plan",
        "entries": [
            { "content": "Read", "priority": "high", "status": "completed" },
            { "content": "Write", "priority": "high", "status": "in_progress" },
            { "content": "Test", "priority": "low", "status": "pending" },
        ],
    } }])));
    let updates = played
        .talk(async |mut session| Ok(all(&mut session.send([text("go")])).await))
        .await
        .unwrap();
    let [Update::Plan(plan)] = updates.as_slice() else {
        panic!("one plan is handed over");
    };
    assert_eq!(
        *plan,
        [
            PlanEntry {
                text: "Read".to_owned(),
                state: PlanState::Completed,
            },
            PlanEntry {
                text: "Write".to_owned(),
                state: PlanState::InProgress,
            },
            PlanEntry {
                text: "Test".to_owned(),
                state: PlanState::Pending,
            },
        ]
    );
}

#[tokio::test]
async fn a_tool_call_is_handed_over_as_it_starts_shows_more_and_ends() {
    let played = Played::new(&one_turn(&json!([
        { "tool": { "id": "e1", "title": "Edit a.rs", "kind": "edit", "status": "in_progress" } },
        { "tool_update": { "id": "e1", "content": [
            { "type": "diff", "path": "/w/a.rs", "oldText": "a", "newText": "b" },
        ] } },
        { "tool_update": { "id": "e1", "status": "completed" } },
    ])));
    let updates = played
        .talk(async |mut session| Ok(all(&mut session.send([text("go")])).await))
        .await
        .unwrap();
    let calls: Vec<_> = updates
        .iter()
        .map(|update| match update {
            Update::ToolCall(call) => call,
            _ => panic!("only the call is handed over"),
        })
        .collect();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|call| call.id == "e1"));
    assert_eq!(calls[0].title.as_deref(), Some("Edit a.rs"));
    assert_eq!(calls[0].result, None);
    assert_eq!(
        calls[1].content,
        Some(vec![ToolCallContent::Diff {
            path: "/w/a.rs".into(),
            old_text: Some("a".to_owned()),
            new_text: "b".to_owned(),
        }])
    );
    assert_eq!(calls[2].title.as_deref(), Some("Edit a.rs"));
    assert_eq!(calls[2].result, Some(ToolCallResult::Completed));
}

/// A turn that starts an edit of `/w/a.rs` and asks to be allowed it,
/// replying `Edited.` when allowed and `Left as it was.` when not.
fn asks_to_edit() -> Value {
    one_turn(&json!([
        { "tool": {
            "id": "e1", "title": "Edit a.rs", "kind": "edit", "status": "pending",
            "locations": [{ "path": "/w/a.rs" }],
        } },
        { "ask": {
            "toolCall": { "toolCallId": "e1" },
            "options": [
                { "optionId": "allow", "name": "Yes", "kind": "allow_once" },
                { "optionId": "reject", "name": "No", "kind": "reject_once" },
            ],
            "then": { "allow": [{ "say": "Edited." }], "reject": [{ "say": "Left as it was." }] },
        } },
    ]))
}

/// Takes `turn` until the agent asks permission, and answers it as
/// `answer` says of the request; answers the rest of the turn.
async fn answering(
    turn: &mut Turn<'_>,
    answer: impl FnOnce(&cox_acp::PermissionRequest) -> PermissionAnswer,
) -> Vec<Update> {
    loop {
        if let Update::PermissionRequest(request) =
            turn.next().await.expect("the agent asks permission")
        {
            let answer = answer(&request);
            request.answer(answer).unwrap();
            return all(turn).await;
        }
    }
}

#[tokio::test]
async fn a_permission_request_tells_of_the_call_it_is_for() {
    let played = Played::new(&asks_to_edit());
    let (title, kind, locations, options) = played
        .talk(async |mut session| {
            let mut turn = session.send([text("edit it")]);
            let mut seen = None;
            answering(&mut turn, |request| {
                seen = Some((
                    request.title().map(str::to_owned),
                    request.kind(),
                    request.locations().to_vec(),
                    request
                        .options()
                        .iter()
                        .map(|option| (option.name.clone(), option.kind))
                        .collect::<Vec<_>>(),
                ));
                PermissionAnswer::Cancel
            })
            .await;
            Ok(seen.unwrap())
        })
        .await
        .unwrap();
    assert_eq!(title.as_deref(), Some("Edit a.rs"));
    assert_eq!(kind, Some(ToolKind::Edit));
    assert_eq!(locations, [PathBuf::from("/w/a.rs")]);
    assert_eq!(
        options,
        [
            ("Yes".to_owned(), PermissionKind::AllowOnce),
            ("No".to_owned(), PermissionKind::RejectOnce),
        ]
    );
}

#[tokio::test]
async fn the_agent_goes_on_as_the_option_chosen_says() {
    for (chosen, said) in [(0, "Edited."), (1, "Left as it was.")] {
        let played = Played::new(&asks_to_edit());
        let (updates, stop) = played
            .talk(async |mut session| {
                let mut turn = session.send([text("edit it")]);
                let updates = answering(&mut turn, |_| PermissionAnswer::Select(chosen)).await;
                Ok((updates, turn.end().await?))
            })
            .await
            .unwrap();
        assert_eq!(replied(&updates), said);
        assert_eq!(stop, StopReason::EndTurn);
    }
}

#[tokio::test]
async fn an_option_that_is_not_offered_cannot_be_chosen() {
    let played = Played::new(&asks_to_edit());
    let refused = played
        .talk(async |mut session| {
            let mut turn = session.send([text("edit it")]);
            loop {
                if let Some(Update::PermissionRequest(request)) = turn.next().await {
                    return Ok(request.answer(PermissionAnswer::Select(2)).is_err());
                }
            }
        })
        .await
        .unwrap();
    assert!(refused);
}

#[tokio::test]
async fn a_permission_request_answered_cancel_ends_the_turn_cancelled() {
    let played = Played::new(&asks_to_edit());
    let (updates, stop) = played
        .talk(async |mut session| {
            let mut turn = session.send([text("edit it")]);
            let updates = answering(&mut turn, |_| PermissionAnswer::Cancel).await;
            Ok((updates, turn.end().await?))
        })
        .await
        .unwrap();
    assert_eq!(replied(&updates), "");
    assert_eq!(stop, StopReason::Cancelled);
}

#[tokio::test]
async fn a_cancelled_turn_ends_cancelled_with_its_running_calls() {
    let played = Played::new(&one_turn(&json!([
        { "tool": { "id": "x1", "title": "Run the tests", "kind": "execute", "status": "in_progress" } },
        { "sleep": 60_000 },
        { "say": "Never said." },
    ])));
    let (updates, stop) = played
        .talk(async |mut session| {
            let mut turn = session.send([text("test it")]);
            let started = turn.next().await;
            assert!(matches!(started, Some(Update::ToolCall(_))));
            turn.cancel()?;
            let updates = all(&mut turn).await;
            Ok((updates, turn.end().await?))
        })
        .await
        .unwrap();
    let [Update::ToolCall(call)] = updates.as_slice() else {
        panic!("the running call ends, and nothing else comes");
    };
    assert_eq!(call.id, "x1");
    assert_eq!(call.title.as_deref(), Some("Run the tests"));
    assert_eq!(call.result, Some(ToolCallResult::Cancelled));
    assert_eq!(stop, StopReason::Cancelled);
}

/// A turn that says `Working.` and waits for a line steered into it, then
/// says `Done.`, and a turn for a line that says `also`.
fn steerable() -> Value {
    json!({ "turns": [
        { "steps": [{ "say": "Working." }, { "wait_for": "steer" }, { "say": "Done." }] },
        { "when": "also", "steps": [{ "say": " And that too." }] },
    ] })
}

#[tokio::test]
async fn a_line_steered_into_a_turn_is_taken_into_it() {
    let played = Played::new(&steerable());
    let (steers, steered, updates) = played
        .talk(async |mut session| {
            let steers = session.steers();
            let mut turn = session.send([text("go")]);
            let mut updates = vec![turn.next().await.unwrap()];
            let steered = turn.steer([text("also this")]).await?;
            updates.extend(all(&mut turn).await);
            Ok((steers, steered, updates))
        })
        .await
        .unwrap();
    assert!(steers);
    assert_eq!(steered, Steered::Taken);
    assert_eq!(replied(&updates), "Working.Done. And that too.");
}

#[tokio::test]
async fn a_line_steered_once_the_turn_has_ended_is_too_late() {
    let played = Played::new(&one_turn(&json!([{ "say": "Done." }])));
    let steered = played
        .talk(async |mut session| {
            let mut turn = session.send([text("go")]);
            all(&mut turn).await;
            turn.steer([text("also this")]).await
        })
        .await
        .unwrap();
    assert_eq!(steered, Steered::TooLate);
}

/// A turn that ends, then goes on by itself, saying `Finished.` once
/// `wait` is over, and a turn that says `Next.`.
fn goes_on_after(wait: u64) -> Value {
    json!({ "turns": [
        { "steps": [{ "say": "Started." }], "after": [{ "sleep": wait }, { "say": "Finished." }] },
        { "steps": [{ "say": "Next." }] },
    ] })
}

#[tokio::test]
async fn what_the_agent_sends_after_a_turn_comes_unprompted() {
    let played = Played::new(&goes_on_after(0));
    let unprompted = played
        .talk(async |mut session| {
            session
                .send([text("run it in the background")])
                .end()
                .await?;
            Ok(session.next_unprompted().await)
        })
        .await
        .unwrap();
    assert!(
        matches!(&unprompted, Some(Update::ReplyText(text)) if text == "Finished."),
        "the agent finishes on its own"
    );
}

#[tokio::test]
async fn what_the_agent_sent_unprompted_and_was_not_taken_comes_first_in_the_next_turn() {
    let played = Played::new(&goes_on_after(0));
    let updates = played
        .talk(async |mut session| {
            session
                .send([text("run it in the background")])
                .end()
                .await?;
            Ok(all(&mut session.send([text("and then?")])).await)
        })
        .await
        .unwrap();
    assert_eq!(replied(&updates), "Finished.Next.");
}

#[tokio::test]
async fn cancelling_between_turns_stops_what_the_agent_does_on_its_own() {
    let played = Played::new(&goes_on_after(60_000));
    let updates = played
        .talk(async |mut session| {
            session
                .send([text("run it in the background")])
                .end()
                .await?;
            session.cancel()?;
            Ok(all(&mut session.send([text("and then?")])).await)
        })
        .await
        .unwrap();
    assert_eq!(replied(&updates), "Next.");
}

#[tokio::test]
async fn a_file_goes_whole_to_an_agent_that_reads_files_in_the_prompt() {
    let played = Played::new(&json!({ "turns": [
        { "when": "the magic word", "steps": [{ "say": "Read it." }] },
    ] }));
    let file = played.folder.path().join("notes.txt");
    std::fs::write(&file, "the magic word").unwrap();
    let updates = played
        .talk(async |mut session| Ok(all(&mut session.send([Block::File(file.clone())])).await))
        .await
        .unwrap();
    assert_eq!(replied(&updates), "Read it.");
}

#[tokio::test]
async fn a_quote_goes_whole_with_where_it_is_from_to_an_agent_that_reads_files() {
    let played = Played::new(&json!({ "turns": [
        { "when": "<context ref=\"file:///w/a.rs#L2-3\">\nfn b() {}", "steps": [{ "say": "Quoted." }] },
    ] }));
    let updates = played
        .talk(async |mut session| {
            let quote = Block::Quote {
                uri: "file:///w/a.rs#L2-3".to_owned(),
                text: "fn b() {}".to_owned(),
            };
            Ok(all(&mut session.send([quote])).await)
        })
        .await
        .unwrap();
    assert_eq!(replied(&updates), "Quoted.");
}

#[tokio::test]
async fn the_servers_handed_over_are_the_agents_to_call() {
    // An MCP server that answers each call with the arguments it got.
    let server = r#"while read -r line; do
  id=$(printf '%s' "$line" | sed 's/.*"id":\([0-9]*\).*/\1/')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"t","version":"0"}}}\n' "$id" ;;
    *'"method":"tools/call"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"called"}]}}\n' "$id" ;;
  esac
done"#;
    let played = Played::new(&one_turn(&json!([
        { "call": { "server": "tools", "tool": "anything" } },
        { "say": "It said {result}." },
    ])));
    let tools = McpServer {
        name: "tools".to_owned(),
        command: "/bin/sh".into(),
        args: vec!["-c".to_owned(), server.to_owned()],
    };
    let updates = within(cox_acp::connect(
        played.agent(),
        played.folder.path().to_owned(),
        vec![tools],
        None,
        |_| {},
        async |_, _| anyhow::bail!("no login"),
        async |mut session| Ok(all(&mut session.send([text("call it")])).await),
    ))
    .await
    .unwrap();
    assert_eq!(replied(&updates), "It said called.");
}

/// Ways to log in: one the agent carries out, and one in a terminal.
fn ways_to_log_in() -> Value {
    json!([
        { "id": "key", "name": "API key", "description": "Uses the key" },
        { "id": "terminal", "name": "Terminal", "type": "terminal", "args": ["--log-in"] },
    ])
}

/// How each time the agent asked to log in went: the ways it offered,
/// and why the way tried before failed.
type Asked = Arc<Mutex<Vec<(Vec<LoginMethod>, Option<String>)>>>;

/// Opens a session with the agent of `played`, logging in each time it
/// asks as `ways` says in turn, noting each ask in `asked`, and answers
/// the session's id.
async fn logging_in(played: &Played, asked: &Asked, ways: &[Login]) -> anyhow::Result<String> {
    let ways = Mutex::new(ways.iter().cloned());
    within(cox_acp::connect(
        played.agent(),
        played.folder.path().to_owned(),
        Vec::new(),
        None,
        |_| {},
        async |offered, failed| {
            asked.lock().unwrap().push((offered, failed));
            ways.lock()
                .unwrap()
                .next()
                .ok_or_else(|| anyhow::anyhow!("the human gives up"))
        },
        async |session| Ok(session.id()),
    ))
    .await
}

#[tokio::test]
async fn an_agent_that_asks_to_log_in_first_opens_the_session_once_logged_in() {
    let played = Played::new(&json!({ "authMethods": ways_to_log_in(), "turns": [] }));
    let asked = Asked::default();
    let id = logging_in(&played, &asked, &[Login::Authenticate("key".to_owned())])
        .await
        .unwrap();
    assert_eq!(id, "fake-1");
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    let (offered, failed) = &asked[0];
    assert_eq!(*failed, None);
    assert_eq!(offered[0].id, "key");
    assert_eq!(offered[0].description.as_deref(), Some("Uses the key"));
    assert_eq!(offered[0].terminal, None);
    assert_eq!(
        offered[1]
            .terminal
            .as_ref()
            .map(|terminal| &terminal.args[..]),
        Some(&["--log-in".to_owned()][..])
    );
}

#[tokio::test]
async fn a_way_to_log_in_that_failed_is_told_on_the_next_ask() {
    let played = Played::new(&json!({ "authMethods": ways_to_log_in(), "turns": [] }));
    let asked = Asked::default();
    let id = logging_in(
        &played,
        &asked,
        &[
            // The agent does not carry out a way of the terminal's.
            Login::Authenticate("terminal".to_owned()),
            Login::Authenticate("key".to_owned()),
        ],
    )
    .await
    .unwrap();
    assert_eq!(id, "fake-1");
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 2);
    assert!(
        asked[1]
            .1
            .as_deref()
            .is_some_and(|failed| failed.contains("terminal")),
        "{:?}",
        asked[1].1
    );
}

#[tokio::test]
async fn a_session_is_not_opened_once_the_human_gives_up_logging_in() {
    let played = Played::new(&json!({ "authMethods": ways_to_log_in(), "turns": [] }));
    let asked = Asked::default();
    let opened = logging_in(&played, &asked, &[]).await;
    assert!(opened.is_err());
    assert_eq!(asked.lock().unwrap().len(), 1);
}

/// A file removed once dropped, even when a test fails.
struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
async fn logging_in_in_a_terminal_runs_the_agents_command_with_the_ways_arguments() {
    let played = Played::new(&json!({ "authMethods": ways_to_log_in(), "turns": [] }));
    let _login = Removed(
        std::env::temp_dir()
            .join("cox-fake-agent-logins")
            .join(played.scenario().to_string_lossy().replace('/', "%")),
    );
    let asked = Asked::default();
    let opened = within(cox_acp::connect(
        played.agent(),
        played.folder.path().to_owned(),
        Vec::new(),
        None,
        |_| {},
        async |offered: Vec<LoginMethod>, failed| {
            asked.lock().unwrap().push((offered.clone(), failed));
            let terminal = offered
                .iter()
                .find_map(|way| way.terminal.clone())
                .expect("a way in a terminal");
            let ran = Command::new(fake_agent())
                .arg(played.scenario())
                .args(&terminal.args)
                .envs(&terminal.env)
                .output()?;
            anyhow::ensure!(ran.status.success(), "the human logged in");
            Ok(Login::Done)
        },
        async |session| Ok(session.id()),
    ))
    .await;
    assert_eq!(opened.unwrap(), "fake-1");
}

#[tokio::test]
async fn a_prompt_refused_until_the_human_logs_in_needs_login_and_goes_once_logged_in() {
    let played = Played::new(&json!({
        "authMethods": ways_to_log_in(),
        "authRequired": "prompt",
        "turns": [{ "steps": [{ "say": "Hello." }] }],
    }));
    let (needed, updates) = played
        .talk(async |mut session| {
            let refused = session.send([text("hi")]).end().await.unwrap_err();
            session
                .log_in(Login::Authenticate("key".to_owned()))
                .await?;
            Ok((
                cox_acp::needs_login(&refused),
                all(&mut session.send([text("hi")])).await,
            ))
        })
        .await
        .unwrap();
    assert!(needed);
    assert_eq!(replied(&updates), "Hello.");
}

#[tokio::test]
async fn an_agent_that_goes_before_a_session_opens_opens_none() {
    let agent = cox_acp::launch(["true".to_owned()]).unwrap();
    let folder = TempDir::new().unwrap();
    let opened = within(cox_acp::connect(
        agent,
        folder.path().to_owned(),
        Vec::new(),
        None,
        |_| {},
        async |_, _| anyhow::bail!("no login"),
        async |session| Ok(session.id()),
    ))
    .await;
    assert!(opened.is_err());
}

#[test]
fn an_agent_with_no_command_is_not_launched() {
    assert!(cox_acp::launch(Vec::<String>::new()).is_err());
}

#[tokio::test]
async fn what_the_talk_returns_is_what_connecting_returns() {
    let played = Played::new(&json!({ "turns": [] }));
    let failed = played
        .talk(async |_| -> anyhow::Result<()> { anyhow::bail!("the chat went") })
        .await;
    assert_eq!(format!("{:#}", failed.unwrap_err()), "the chat went");
}

#[tokio::test]
async fn a_turn_whose_agent_goes_ends_with_an_error() {
    // An agent that opens a session, and goes as it is prompted.
    let script = r#"while read -r line; do
  id=$(printf '%s' "$line" | sed 's/.*"id":\("[^"]*"\).*/\1/')
  case "$line" in
    *'"method":"initialize"'*)
      echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":1,"agentCapabilities":{}}}' ;;
    *'"method":"session/new"'*)
      echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"sessionId":"s"}}' ;;
    *'"method":"session/prompt"'*)
      exit 0 ;;
  esac
done"#;
    let agent = cox_acp::launch(["sh".to_owned(), "-c".to_owned(), script.to_owned()]).unwrap();
    let folder = TempDir::new().unwrap();
    let ended = within(cox_acp::connect(
        agent,
        folder.path().to_owned(),
        Vec::new(),
        None,
        |_| {},
        async |_, _| anyhow::bail!("no login"),
        async |mut session| Ok(session.send([text("go")]).end().await.is_err()),
    ))
    .await;
    assert!(ended.unwrap(), "the turn ends with an error");
}
