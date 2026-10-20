//! A human talking to an agent through `cox::run`, with the agent faked in
//! memory.

mod fake_agent;

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
