//! A human talking to an agent through `cox::run`, with the agent faked in
//! memory.

mod fake_agent;

use fake_agent::FakeAgent;

#[tokio::test]
async fn opens_a_session_with_the_agent_and_ends() {
    let agent = FakeAgent::default();

    cox::run(agent.connect())
        .await
        .expect("cox ends without an error");

    assert_eq!(agent.sessions_opened(), 1);
}
