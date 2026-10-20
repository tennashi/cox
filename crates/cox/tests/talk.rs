//! A human talking to an agent through `cox::run`, with the agent faked in
//! memory.

mod fake_agent;

use agent_client_protocol::schema::v1::{RequestPermissionOutcome, SelectedPermissionOutcome};
use fake_agent::FakeAgent;

#[tokio::test]
async fn opens_a_session_with_the_agent_and_ends() {
    let agent = FakeAgent::default();

    cox::run(agent.connect(), "".as_bytes(), &mut Vec::new())
        .await
        .expect("cox ends without an error");

    assert_eq!(agent.sessions_opened(), 1);
}

#[tokio::test]
async fn sends_a_line_and_shows_how_the_turn_ended() {
    let agent = FakeAgent::default();
    let mut screen = Vec::new();

    cox::run(agent.connect(), "hello\n".as_bytes(), &mut screen)
        .await
        .expect("cox ends without an error");

    assert_eq!(agent.prompts(), ["hello"]);
    assert!(String::from_utf8(screen).unwrap().ends_with("[end_turn]\n"));
}

#[tokio::test]
async fn shows_the_reply_as_it_arrives() {
    let agent = FakeAgent::default();
    let mut screen = Vec::new();

    cox::run(agent.connect(), "hello\n".as_bytes(), &mut screen)
        .await
        .expect("cox ends without an error");

    assert_eq!(
        String::from_utf8(screen).unwrap(),
        "echo: hello\n[end_turn]\n"
    );
}

#[tokio::test]
async fn takes_a_turn_for_each_line_until_the_input_ends() {
    let agent = FakeAgent::default();
    let mut screen = Vec::new();

    cox::run(
        agent.connect(),
        "hello\nhow are you\n".as_bytes(),
        &mut screen,
    )
    .await
    .expect("cox ends without an error");

    assert_eq!(agent.prompts(), ["hello", "how are you"]);
    assert_eq!(
        String::from_utf8(screen).unwrap(),
        "echo: hello\n[end_turn]\necho: how are you\n[end_turn]\n"
    );
}

#[tokio::test]
async fn rejects_permission_requests_and_shows_it() {
    let agent = FakeAgent::default();
    let mut screen = Vec::new();

    cox::run(agent.connect(), "ask permission\n".as_bytes(), &mut screen)
        .await
        .expect("cox ends without an error");

    assert_eq!(
        agent.answers(),
        [RequestPermissionOutcome::Selected(
            SelectedPermissionOutcome::new("reject")
        )]
    );
    assert_eq!(
        String::from_utf8(screen).unwrap(),
        "\n[rejected: touch x.txt]\necho: ask permission\n[end_turn]\n"
    );
}
