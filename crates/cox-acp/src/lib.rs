//! Talking to an agent over the Agent Client Protocol (ACP), in Cox's terms.
//!
//! [`connect`] opens a [`Session`] with an agent. Each [`Session::send`]
//! starts a [`Turn`], which hands over what the agent sends as [`Update`]s
//! until the turn ends.

use std::future::Future;
use std::pin::Pin;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, InitializeRequest, NewSessionRequest, PermissionOptionKind,
    PromptRequest, PromptResponse, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionId, SessionNotification,
    SessionUpdate, StopReason as AcpStopReason, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, Client, ConnectTo, ConnectionTo, Error, Responder};
use anyhow::Context;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// A way to reach an agent: a process speaking ACP on its stdio, or, in
/// tests, an in-memory channel.
pub trait Transport: ConnectTo<Client> {}

impl<T: ConnectTo<Client>> Transport for T {}

/// Starts `command` (the program, then its arguments) as an agent process.
///
/// # Errors
///
/// Returns an error if `command` is empty.
pub fn launch(command: impl IntoIterator<Item = String>) -> anyhow::Result<impl Transport> {
    Ok(AcpAgent::from_args(command)?)
}

/// Connects to `agent`, opens a session in the current directory, and runs
/// `talk` with it. The connection closes when `talk` returns.
///
/// # Errors
///
/// Returns the error `talk` returns, or an error if the connection fails or
/// the agent refuses to open a session.
pub async fn connect<T>(
    agent: impl Transport,
    talk: impl AsyncFnOnce(Session) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let (updates, received) = unbounded_channel();
    let mut outcome = None;
    Client
        .builder()
        .name("cox")
        .on_receive_notification(
            async |notification: SessionNotification, _cx| {
                if let Some(update) = translate(notification.update) {
                    deliver(&updates, update);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async |request: RequestPermissionRequest, responder, _cx| {
                let request = PermissionRequest::new(request, responder);
                deliver(&updates, Update::PermissionRequest(request));
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, async |cx: ConnectionTo<Agent>| {
            let id = start(&cx).await?;
            let session = Session { cx, id, received };
            outcome = Some(talk(session).await);
            Ok(())
        })
        .await?;
    outcome.context("the connection closed before the session started")?
}

/// Initializes the connection and opens a session in the current directory.
async fn start(cx: &ConnectionTo<Agent>) -> agent_client_protocol::Result<SessionId> {
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

/// Hands `update` to the turn that is running. The receiver lives as long as
/// the session, so sending only fails after `talk` has returned, when there
/// is no one left to show the update to.
fn deliver(updates: &UnboundedSender<Update>, update: Update) {
    let _ = updates.send(update);
}

/// The parts of a session update Cox uses, or `None` for the rest.
fn translate(update: SessionUpdate) -> Option<Update> {
    match update {
        SessionUpdate::AgentMessageChunk(ContentChunk {
            content: ContentBlock::Text(text),
            ..
        }) => Some(Update::ReplyText(text.text)),
        _ => None,
    }
}

/// A conversation with an agent.
pub struct Session {
    cx: ConnectionTo<Agent>,
    id: SessionId,
    received: UnboundedReceiver<Update>,
}

impl Session {
    /// Sends `text` to the agent as a prompt, starting a turn.
    pub fn send(&mut self, text: String) -> Turn<'_> {
        let prompt = PromptRequest::new(
            self.id.clone(),
            vec![ContentBlock::Text(TextContent::new(text))],
        );
        Turn {
            received: &mut self.received,
            response: Box::pin(self.cx.send_request(prompt).block_task()),
            answer: None,
        }
    }
}

/// One prompt and everything the agent does in answer to it.
pub struct Turn<'s> {
    received: &'s mut UnboundedReceiver<Update>,
    response: Pin<Box<dyn Future<Output = agent_client_protocol::Result<PromptResponse>> + Send>>,
    /// The agent's answer to the prompt, once it has arrived.
    answer: Option<agent_client_protocol::Result<PromptResponse>>,
}

impl Turn<'_> {
    /// The next thing the agent sends in this turn, or `None` once the turn
    /// has ended.
    pub async fn next(&mut self) -> Option<Update> {
        if self.answer.is_some() {
            return None;
        }
        // The agent sends a turn's updates before answering the prompt, and
        // they are delivered in that order, so take every waiting update
        // before looking at the answer.
        tokio::select! {
            biased;
            Some(update) = self.received.recv() => Some(update),
            answer = &mut self.response => {
                self.answer = Some(answer);
                None
            }
        }
    }

    /// Waits for the turn to end, and returns why it ended. Updates not taken
    /// with [`Turn::next`] are dropped.
    ///
    /// # Errors
    ///
    /// Returns the error the agent answered the prompt with, or an error if
    /// the connection fails.
    pub async fn end(mut self) -> anyhow::Result<StopReason> {
        loop {
            if let Some(answer) = self.answer.take() {
                return Ok(StopReason::from(answer?.stop_reason));
            }
            self.next().await;
        }
    }
}

/// Something the agent sends during a turn.
pub enum Update {
    /// A piece of the agent's reply.
    ReplyText(String),
    /// The agent asks to be allowed to run a tool call.
    PermissionRequest(PermissionRequest),
}

/// A request from the agent for permission to run a tool call. Answer it with
/// [`PermissionRequest::answer`]; the agent waits until then.
pub struct PermissionRequest {
    // Boxed so that an `Update` stays small whichever variant it holds.
    request: Box<RequestPermissionRequest>,
    responder: Box<Responder<RequestPermissionResponse>>,
    options: Vec<PermissionOption>,
}

impl PermissionRequest {
    fn new(
        request: RequestPermissionRequest,
        responder: Responder<RequestPermissionResponse>,
    ) -> Self {
        let options = request
            .options
            .iter()
            .map(|option| PermissionOption {
                name: option.name.clone(),
                kind: PermissionKind::from(option.kind),
            })
            .collect();
        Self {
            request: Box::new(request),
            responder: Box::new(responder),
            options,
        }
    }

    /// The title of the tool call, if the agent gave one.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.request.tool_call.fields.title.as_deref()
    }

    /// The answers the agent offers.
    #[must_use]
    pub fn options(&self) -> &[PermissionOption] {
        &self.options
    }

    /// Sends `answer` to the agent.
    ///
    /// # Errors
    ///
    /// Returns an error if `answer` selects an option that does not exist,
    /// or if the connection fails.
    pub fn answer(self, answer: PermissionAnswer) -> anyhow::Result<()> {
        let outcome = match answer {
            PermissionAnswer::Select(index) => {
                let option = self
                    .request
                    .options
                    .get(index)
                    .with_context(|| format!("there is no option {index}"))?;
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                    option.option_id.clone(),
                ))
            }
            PermissionAnswer::Cancel => RequestPermissionOutcome::Cancelled,
        };
        self.responder
            .respond(RequestPermissionResponse::new(outcome))?;
        Ok(())
    }
}

/// One of the answers to a permission request.
pub struct PermissionOption {
    pub name: String,
    pub kind: PermissionKind,
}

/// What choosing a permission option means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    /// A kind newer than this version of Cox knows.
    Other,
}

impl From<PermissionOptionKind> for PermissionKind {
    fn from(kind: PermissionOptionKind) -> Self {
        match kind {
            PermissionOptionKind::AllowOnce => Self::AllowOnce,
            PermissionOptionKind::AllowAlways => Self::AllowAlways,
            PermissionOptionKind::RejectOnce => Self::RejectOnce,
            PermissionOptionKind::RejectAlways => Self::RejectAlways,
            _ => Self::Other,
        }
    }
}

/// An answer to a permission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionAnswer {
    /// Choose the option at this index in [`PermissionRequest::options`].
    Select(usize),
    /// Decline to choose; the agent treats the tool call as not allowed.
    Cancel,
}

/// Why a turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    /// A reason newer than this version of Cox knows.
    Other,
}

impl From<AcpStopReason> for StopReason {
    fn from(reason: AcpStopReason) -> Self {
        match reason {
            AcpStopReason::EndTurn => Self::EndTurn,
            AcpStopReason::MaxTokens => Self::MaxTokens,
            AcpStopReason::MaxTurnRequests => Self::MaxTurnRequests,
            AcpStopReason::Refusal => Self::Refusal,
            AcpStopReason::Cancelled => Self::Cancelled,
            _ => Self::Other,
        }
    }
}
