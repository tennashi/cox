//! An ACP agent for tests, connected to Cox in memory. It records what Cox
//! asked of it so tests can check what Cox did.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse,
};
use agent_client_protocol::{Agent, Channel, Result};

#[derive(Default)]
pub struct FakeAgent {
    sessions_opened: Arc<AtomicUsize>,
}

impl FakeAgent {
    /// Starts serving and returns the end of the connection to give Cox.
    pub fn connect(&self) -> Channel {
        let (cox_end, agent_end) = Channel::duplex();
        tokio::spawn(serve(agent_end, Arc::clone(&self.sessions_opened)));
        cox_end
    }

    /// How many sessions Cox has opened.
    pub fn sessions_opened(&self) -> usize {
        self.sessions_opened.load(Ordering::SeqCst)
    }
}

async fn serve(transport: Channel, sessions_opened: Arc<AtomicUsize>) -> Result<()> {
    Agent
        .builder()
        .name("fake-agent")
        .on_receive_request(
            async |request: InitializeRequest, responder, _cx| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |_request: NewSessionRequest, responder, _cx| {
                sessions_opened.fetch_add(1, Ordering::SeqCst);
                responder.respond(NewSessionResponse::new("session-1"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(transport)
        .await
}
