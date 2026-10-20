//! An ACP agent for tests, connected to Cox in memory. It records what Cox
//! asked of it so tests can check what Cox did.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, StopReason,
};
use agent_client_protocol::{Agent, Channel, Result};

#[derive(Default)]
pub struct FakeAgent {
    sessions_opened: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl FakeAgent {
    /// Starts serving and returns the end of the connection to give Cox.
    pub fn connect(&self) -> Channel {
        let (cox_end, agent_end) = Channel::duplex();
        tokio::spawn(serve(
            agent_end,
            Arc::clone(&self.sessions_opened),
            Arc::clone(&self.prompts),
        ));
        cox_end
    }

    /// How many sessions Cox has opened.
    pub fn sessions_opened(&self) -> usize {
        self.sessions_opened.load(Ordering::SeqCst)
    }

    /// The text of each prompt Cox has sent, in order.
    pub fn prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

async fn serve(
    transport: Channel,
    sessions_opened: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
) -> Result<()> {
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
        .on_receive_request(
            async move |request: PromptRequest, responder, _cx| {
                prompts.lock().unwrap().push(text_of(&request.prompt));
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(transport)
        .await
}

/// The text blocks of a prompt, joined.
fn text_of(prompt: &[ContentBlock]) -> String {
    prompt
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}
