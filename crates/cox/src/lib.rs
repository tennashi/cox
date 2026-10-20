//! Cox: a terminal where a human talks to a coding agent.

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{InitializeRequest, NewSessionRequest, SessionId};
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo, Error, Result};

/// Talks to `agent`. For now it only opens a session and ends.
///
/// # Errors
///
/// Returns an error if the connection to the agent fails or the agent
/// answers a request with an error.
pub async fn run(agent: impl ConnectTo<Client>) -> Result<()> {
    Client
        .builder()
        .name("cox")
        .connect_with(agent, async |cx: ConnectionTo<Agent>| {
            let _session = start(&cx).await?;
            Ok(())
        })
        .await
}

/// Initializes the connection and opens a session in the current directory.
async fn start(cx: &ConnectionTo<Agent>) -> Result<SessionId> {
    cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let cwd = std::env::current_dir().map_err(Error::into_internal_error)?;
    let session = cx
        .send_request(NewSessionRequest::new(cwd))
        .block_task()
        .await?;
    Ok(session.session_id)
}
