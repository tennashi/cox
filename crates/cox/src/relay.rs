//! The relay between one session's agent and the browser's `DevTools`
//! port. The agent's browser tools connect here instead of to CEF; the
//! relay passes their messages through, keeping the agent to its own
//! session's page, and tells that session's stage of the input the agent
//! sends to the page, so the stage can show where the agent points, clicks
//! and types.
//!
//! Every session's page is in the same browser, so the relay hides the
//! rest of the browser from the agent: what it hears of other targets, and
//! what it asks of them (see [`Scope`]).
//!
//! Seen from the page, the agent's clicks and the human's are the same
//! clicks: only here, on the agent's own connection, can they be told
//! apart.
//!
//! Typing says nothing of where it goes, so when the agent types, the
//! relay asks the page itself which element has the focus, and keeps the
//! answer from the agent.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use futures_util::{SinkExt as _, StreamExt as _};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::browser::TargetId;

/// Input the agent sent to the page. Positions are in CSS pixels of the
/// page's viewport, which are the stage's logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AgentInput {
    /// The pointer moved to `(x, y)`, or a button came up there.
    Move { x: f32, y: f32 },
    /// A button went down at `(x, y)`.
    Press { x: f32, y: f32 },
    /// The wheel turned at `(x, y)`.
    Wheel { x: f32, y: f32 },
    /// A key, or text, went to the focused element.
    Type,
    /// The element the agent types into has these bounds.
    Field {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
}

/// The ids of the relay's own requests, far above those the agent's tools
/// count up from, and still within the 32-bit ids the browser takes.
const OWN_IDS: u64 = 2_000_000_000;

/// How often, at most, the relay asks where the agent types: once for a
/// run of keys is enough.
const ASK_EVERY: Duration = Duration::from_millis(250);

/// Finds the focused element, inside shadow roots too, and answers its
/// bounds in the viewport, or `null` when the page has no focus.
const FOCUSED_BOUNDS: &str = "(() => {\
    let element = document.activeElement;\
    while (element?.shadowRoot?.activeElement) element = element.shadowRoot.activeElement;\
    if (!element || element === document.body) return null;\
    const bounds = element.getBoundingClientRect();\
    return [bounds.x, bounds.y, bounds.width, bounds.height];\
})()";

/// How long the relay waits for CEF to name the page, before it gives up
/// on the agent's connection.
const PAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the relay looks for the tab that holds the page.
const TAB_TIMEOUT: Duration = Duration::from_secs(2);

/// Starts the relay on a port of its own, in front of the `DevTools` port
/// `devtools_port`, for the agent of the page `page`, and returns its port
/// with the input the agent sends.
///
/// # Errors
///
/// Returns an error if no port is free to listen on.
pub fn start(
    devtools_port: u16,
    page: TargetId,
) -> anyhow::Result<(u16, UnboundedReceiver<AgentInput>)> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let (seen, received) = unbounded_channel();
    thread::spawn(move || {
        let served = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| runtime.block_on(serve(listener, devtools_port, page, seen)));
        if let Err(error) = served {
            log::error!("a relay to the stage stopped: {error:#}");
        }
    });
    Ok((port, received))
}

/// Accepts the agent's connections, and relays each to the browser.
async fn serve(
    listener: StdTcpListener,
    devtools_port: u16,
    page: TargetId,
    seen: UnboundedSender<AgentInput>,
) -> anyhow::Result<()> {
    let listener = TcpListener::from_std(listener)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let page = page.clone();
        let seen = seen.clone();
        tokio::spawn(async move {
            if let Err(error) = relay(stream, devtools_port, page, seen).await {
                log::warn!("a connection to the stage ended: {error:#}");
            }
        });
    }
}

/// No limit on a message's size: a screenshot of a long page is large, and
/// both ends of the relay are on this machine.
fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None)
}

/// Relays one connection of the agent's to the browser, whatever path it
/// asked for, until either end closes.
async fn relay(
    client: TcpStream,
    devtools_port: u16,
    mut page: TargetId,
    seen: UnboundedSender<AgentInput>,
) -> anyhow::Result<()> {
    let client = tokio_tungstenite::accept_async_with_config(client, Some(config())).await?;
    let endpoint = browser_endpoint(devtools_port).await?;
    let scope = Mutex::new(scope(&endpoint, &mut page).await?);
    let scope = || scope.lock().unwrap_or_else(PoisonError::into_inner);
    let (browser, _) =
        tokio_tungstenite::connect_async_with_config(endpoint, Some(config()), true).await?;
    let (mut to_client, mut from_client) = client.split();
    let (mut to_browser, mut from_browser) = browser.split();
    // Both ends hear from both directions: the relay answers what it
    // refuses the agent, and lets go of what it hides.
    let (for_client, mut client_queue) = unbounded_channel::<Message>();
    let (for_browser, mut browser_queue) = unbounded_channel::<Message>();
    let next_id = AtomicU64::new(OWN_IDS);
    let own_id = || next_id.fetch_add(1, Ordering::Relaxed);
    let upstream = async {
        let mut asked = None::<Instant>;
        while let Some(message) = from_client.next().await {
            let message = message?;
            let Message::Text(text) = &message else {
                if relayed(&message) {
                    let _ = for_browser.send(message);
                }
                continue;
            };
            let Ok(request) = serde_json::from_str::<Value>(text.as_str()) else {
                let _ = for_browser.send(message);
                continue;
            };
            if let Request::Refuse(answer) = scope().request(&request) {
                let _ = for_client.send(Message::text(answer));
                continue;
            }
            let typed_in = if let Some((input, session)) = observe(&request) {
                // The stage may be gone while the agent carries on.
                let _ = seen.send(input);
                session.filter(|_| input == AgentInput::Type)
            } else {
                None
            };
            let _ = for_browser.send(message);
            if let Some(session) = typed_in
                && asked.is_none_or(|asked| asked.elapsed() >= ASK_EVERY)
            {
                asked = Some(Instant::now());
                let _ = for_browser.send(Message::text(ask_focused(own_id(), &session)));
            }
        }
        anyhow::Ok(())
    };
    let downstream = async {
        while let Some(message) = from_browser.next().await {
            let message = message?;
            let Message::Text(text) = &message else {
                if relayed(&message) {
                    let _ = for_client.send(message);
                }
                continue;
            };
            if let Answer::ToRelay(field) = answer(text.as_str()) {
                if let Some(field) = field {
                    let _ = seen.send(field);
                }
                continue;
            }
            match scope().reply(text.as_str()) {
                Reply::Pass => {
                    let _ = for_client.send(message);
                }
                Reply::Replace(text) => {
                    let _ = for_client.send(Message::text(text));
                }
                Reply::Drop => {}
                Reply::Hide { session, waiting } => {
                    for request in let_go(&session, waiting, own_id) {
                        let _ = for_browser.send(Message::text(request));
                    }
                }
            }
        }
        anyhow::Ok(())
    };
    let to_client = async {
        while let Some(message) = client_queue.recv().await {
            to_client.send(message).await?;
        }
        anyhow::Ok(())
    };
    let to_browser = async {
        while let Some(message) = browser_queue.recv().await {
            to_browser.send(message).await?;
        }
        anyhow::Ok(())
    };
    tokio::select! {
        ended = upstream => ended,
        ended = downstream => ended,
        ended = to_client => ended,
        ended = to_browser => ended,
    }
}

/// What the agent of `page` sees of the browser at `endpoint`, once CEF
/// has named the page.
async fn scope(endpoint: &str, page: &mut TargetId) -> anyhow::Result<Scope> {
    let page = tokio::time::timeout(PAGE_TIMEOUT, page.wait_for(Option::is_some))
        .await
        .context("CEF did not name the page in time")?
        .context("the page is gone")?
        .clone()
        .context("the page has no name")?;
    let tab = match tokio::time::timeout(TAB_TIMEOUT, find_tab(endpoint, &page)).await {
        Ok(Ok(tab)) => tab,
        Ok(Err(error)) => {
            log::warn!("no tab found for the page, so its agent gets the page alone: {error:#}");
            None
        }
        Err(_) => {
            log::warn!("no tab found for the page in time, so its agent gets the page alone");
            None
        }
    };
    Ok(Scope::new(&page, tab.as_deref()))
}

/// The relay's requests that let go of the target attached in `session`,
/// which the agent does not see: it may be waiting to start until someone
/// lets it, as a new page does when the agent asked to attach to every
/// new target.
fn let_go(session: &str, waiting: bool, mut own_id: impl FnMut() -> u64) -> Vec<String> {
    let mut requests = Vec::new();
    if waiting {
        requests.push(
            json!({
                "id": own_id(),
                "sessionId": session,
                "method": "Runtime.runIfWaitingForDebugger",
            })
            .to_string(),
        );
    }
    requests.push(
        json!({
            "id": own_id(),
            "method": "Target.detachFromTarget",
            "params": { "sessionId": session },
        })
        .to_string(),
    );
    requests
}

/// Whether a message goes on to the other end. Pings and pongs are each
/// end's business with the relay, which answers them itself.
fn relayed(message: &Message) -> bool {
    matches!(message, Message::Text(_) | Message::Binary(_))
}

/// The browser's own `DevTools` endpoint, which `/json/version` names.
async fn browser_endpoint(devtools_port: u16) -> anyhow::Result<String> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, devtools_port)).await?;
    // The endpoint the browser names is on the host this names.
    let request = format!("GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{devtools_port}\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    // The browser keeps the connection open after it answers, so the
    // answer ends where its length says.
    let mut response = Vec::new();
    let body = loop {
        if let Some(body) = body(&response) {
            break body;
        }
        let mut chunk = [0; 4096];
        let read = stream.read(&mut chunk).await?;
        anyhow::ensure!(read > 0, "the browser hung up before it answered");
        response.extend_from_slice(&chunk[..read]);
    };
    let version: Value = serde_json::from_slice(&response[body])?;
    version["webSocketDebuggerUrl"]
        .as_str()
        .map(str::to_owned)
        .context("the browser names no endpoint")
}

/// Where the body of an HTTP answer is in `response`, once all of it has
/// arrived.
fn body(response: &[u8]) -> Option<Range<usize>> {
    let headers_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    let headers = std::str::from_utf8(&response[..headers_end]).ok()?;
    let length: usize = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())?
    })?;
    let start = headers_end + 4;
    (response.len() >= start + length).then_some(start..start + length)
}

/// The input a CDP message from the agent sends to the page, if any, and
/// the session of the page it goes to.
fn observe(message: &Value) -> Option<(AgentInput, Option<String>)> {
    let params = &message["params"];
    let at = || Some((float(&params["x"])?, float(&params["y"])?));
    let input = match message["method"].as_str()? {
        "Input.dispatchMouseEvent" => {
            let (x, y) = at()?;
            match params["type"].as_str()? {
                "mousePressed" => AgentInput::Press { x, y },
                "mouseWheel" => AgentInput::Wheel { x, y },
                _ => AgentInput::Move { x, y },
            }
        }
        "Input.dispatchKeyEvent" | "Input.insertText" | "Input.imeSetComposition" => {
            AgentInput::Type
        }
        _ => return None,
    };
    let session = message["sessionId"].as_str().map(str::to_owned);
    Some((input, session))
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "positions on a page fit an f32"
)]
fn float(value: &Value) -> Option<f32> {
    value.as_f64().map(|value| value as f32)
}

/// The relay's request, numbered `id`, for the bounds of the focused
/// element in the page of `session`.
fn ask_focused(id: u64, session: &str) -> String {
    serde_json::json!({
        "id": id,
        "sessionId": session,
        "method": "Runtime.evaluate",
        "params": { "expression": FOCUSED_BOUNDS, "returnByValue": true },
    })
    .to_string()
}

/// Whom a message from the browser is for.
#[derive(Debug, PartialEq)]
enum Answer {
    ToAgent,
    /// The answer to the relay's own request, with the field it names.
    ToRelay(Option<AgentInput>),
}

fn answer(message: &str) -> Answer {
    // The relay's answers are short; the agent's screenshots are not.
    if message.len() > 1024 || !message.contains("\"id\"") {
        return Answer::ToAgent;
    }
    let Ok(message) = serde_json::from_str::<Value>(message) else {
        return Answer::ToAgent;
    };
    if message["id"].as_u64().is_none_or(|id| id < OWN_IDS) {
        return Answer::ToAgent;
    }
    let bounds = &message["result"]["result"]["value"];
    let field = || {
        Some(AgentInput::Field {
            x: float(&bounds[0])?,
            y: float(&bounds[1])?,
            width: float(&bounds[2])?,
            height: float(&bounds[3])?,
        })
    };
    Answer::ToRelay(field())
}

/// The tab that holds `page`, if the browser has tabs: Chromium puts each
/// page in a tab target, and the agent's tools reach pages through their
/// tabs. Nothing links a tab to its page but attaching to the tab, so the
/// relay does that on a connection of its own, which it then drops.
async fn find_tab(endpoint: &str, page: &str) -> anyhow::Result<Option<String>> {
    let (browser, _) =
        tokio_tungstenite::connect_async_with_config(endpoint, Some(config()), true).await?;
    let (mut to_browser, mut from_browser) = browser.split();
    let request = |id: usize, session: Option<&str>, method: &str, params: Value| {
        let mut request = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            request["sessionId"] = session.into();
        }
        Message::text(request.to_string())
    };
    to_browser
        .send(request(
            0,
            None,
            "Target.getTargets",
            json!({ "filter": [{ "type": "tab" }] }),
        ))
        .await?;
    // Attach to every tab; once attached, ask each for its pages.
    let mut tabs = Vec::new();
    // The tab attached in each session.
    let mut sessions = HashMap::new();
    while let Some(message) = from_browser.next().await {
        let Message::Text(text) = message? else {
            continue;
        };
        let message: Value = serde_json::from_str(text.as_str())?;
        if let Some(error) = message.get("error") {
            anyhow::bail!("the browser refused: {error}");
        }
        let result = &message["result"];
        match message["id"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
        {
            Some(0) => {
                tabs = result["targetInfos"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|info| info["targetId"].as_str().map(str::to_owned))
                    .collect();
                if tabs.is_empty() {
                    return Ok(None);
                }
                for (index, tab) in tabs.iter().enumerate() {
                    let params = json!({ "targetId": tab, "flatten": true });
                    to_browser
                        .send(request(index + 1, None, "Target.attachToTarget", params))
                        .await?;
                }
            }
            Some(attached) if attached >= 1 && attached <= tabs.len() => {
                let session = result["sessionId"].as_str().context("no session")?;
                sessions.insert(session.to_owned(), tabs[attached - 1].clone());
                let params = json!({
                    "autoAttach": true,
                    "waitForDebuggerOnStart": false,
                    "flatten": true,
                });
                let id = tabs.len() + attached;
                to_browser
                    .send(request(id, Some(session), "Target.setAutoAttach", params))
                    .await?;
            }
            _ if message["method"] == "Target.attachedToTarget"
                && message["params"]["targetInfo"]["targetId"] == page =>
            {
                // Attached to in the session of its tab.
                let tab = message["sessionId"]
                    .as_str()
                    .and_then(|session| sessions.get(session));
                return Ok(tab.cloned());
            }
            _ => {}
        }
    }
    anyhow::bail!("the browser hung up")
}
/// What one session's agent sees of the browser: its own page, the tab
/// that holds it, and the targets the agent attaches to within them, such
/// as the page's frames and workers. Other sessions' pages, and the rest
/// of the browser, stay hidden: the agent hears nothing of them, and what
/// it asks of them is refused, as if they were not there.
#[derive(Debug)]
struct Scope {
    /// The page and its tab, which the agent may not close.
    own: Vec<String>,
    /// The targets the agent sees.
    targets: HashSet<String>,
    /// The sessions, attached to those targets, that the agent sees.
    sessions: HashSet<String>,
    /// The ids of the agent's requests for the list of targets, whose
    /// answers name every target.
    listings: HashSet<u64>,
}

/// What becomes of a message from the agent.
#[derive(Debug, PartialEq)]
enum Request {
    Pass,
    /// Not for the browser: the answer goes back to the agent.
    Refuse(String),
}

/// What becomes of a message from the browser.
#[derive(Debug, PartialEq)]
enum Reply {
    Pass,
    /// The agent gets this instead.
    Replace(String),
    Drop,
    /// The browser attached to a target the agent does not see, in
    /// `session`: the agent hears nothing of it, and the relay lets go of
    /// it, starting it first if it is `waiting`.
    Hide {
        session: String,
        waiting: bool,
    },
}

/// The parts of a message from the browser that say whom it is about,
/// read without taking a screenshot's data apart.
#[derive(Deserialize)]
struct Head<'a> {
    id: Option<u64>,
    method: Option<&'a str>,
    #[serde(rename = "sessionId")]
    session: Option<&'a str>,
}

impl Scope {
    fn new(page: &str, tab: Option<&str>) -> Self {
        let own: Vec<String> = [Some(page), tab]
            .into_iter()
            .flatten()
            .map(str::to_owned)
            .collect();
        Self {
            targets: own.iter().cloned().collect(),
            own,
            sessions: HashSet::new(),
            listings: HashSet::new(),
        }
    }

    /// What becomes of `request`, from the agent.
    fn request(&mut self, request: &Value) -> Request {
        let params = &request["params"];
        let session = request["sessionId"].as_str();
        let refused = match request["method"].as_str().unwrap_or_default() {
            _ if session.is_some_and(|session| !self.sessions.contains(session)) => {
                Some("No session with given id")
            }
            "Target.createTarget" => {
                Some("Cox gives each session one page: navigate it instead of opening another")
            }
            "Browser.close" | "Browser.crash" | "Browser.crashGpuProcess" => {
                Some("The browser is Cox's, and stays open")
            }
            "Target.closeTarget" if self.own.iter().any(|own| params["targetId"] == **own) => {
                Some("The page is the stage's, and stays open")
            }
            _ if params["targetId"]
                .as_str()
                .is_some_and(|target| !self.targets.contains(target)) =>
            {
                Some("No target with given id found")
            }
            "Target.detachFromTarget" | "Target.sendMessageToTarget"
                if params["sessionId"]
                    .as_str()
                    .is_some_and(|session| !self.sessions.contains(session)) =>
            {
                Some("No session with given id")
            }
            "Target.getTargets" => {
                if let Some(id) = request["id"].as_u64() {
                    self.listings.insert(id);
                }
                None
            }
            _ => None,
        };
        let Some(refused) = refused else {
            return Request::Pass;
        };
        let mut answer = json!({
            "id": request["id"],
            "error": { "code": -32000, "message": refused },
        });
        if let Some(session) = session {
            answer["sessionId"] = session.into();
        }
        Request::Refuse(answer.to_string())
    }

    /// What becomes of `reply`, from the browser.
    fn reply(&mut self, reply: &str) -> Reply {
        let Ok(head) = serde_json::from_str::<Head<'_>>(reply) else {
            return Reply::Pass;
        };
        if head
            .session
            .is_some_and(|session| !self.sessions.contains(session))
        {
            return Reply::Drop;
        }
        if let Some(id) = head.id {
            if head.session.is_none() && self.listings.remove(&id) {
                return self.listing(reply);
            }
            return Reply::Pass;
        }
        match head.method {
            Some(method) if method.starts_with("Target.") => {
                let Ok(event) = serde_json::from_str::<Value>(reply) else {
                    return Reply::Pass;
                };
                self.target_event(method, head.session, &event["params"])
            }
            _ => Reply::Pass,
        }
    }

    /// The answer to the agent's request for the list of targets, naming
    /// those it sees only.
    fn listing(&self, reply: &str) -> Reply {
        let Ok(mut reply) = serde_json::from_str::<Value>(reply) else {
            return Reply::Pass;
        };
        if let Some(infos) = reply["result"]["targetInfos"].as_array_mut() {
            infos.retain(|info| self.sees(info));
        }
        Reply::Replace(reply.to_string())
    }

    /// Whether the agent sees the target `info` describes. The browser
    /// itself has nothing of other sessions in it.
    fn sees(&self, info: &Value) -> bool {
        info["type"] == "browser"
            || info["targetId"]
                .as_str()
                .is_some_and(|target| self.targets.contains(target))
    }

    /// What becomes of an event of the `Target` domain, sent in `session`
    /// (`None` for the browser's own) with `params`.
    fn target_event(&mut self, method: &str, session: Option<&str>, params: &Value) -> Reply {
        let shown = |seen: bool| if seen { Reply::Pass } else { Reply::Drop };
        match method {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                shown(self.sees(&params["targetInfo"]))
            }
            "Target.targetDestroyed" | "Target.targetCrashed" => shown(
                params["targetId"]
                    .as_str()
                    .is_some_and(|target| self.targets.contains(target)),
            ),
            "Target.attachedToTarget" => {
                let Some(attached) = params["sessionId"].as_str() else {
                    return Reply::Drop;
                };
                let info = &params["targetInfo"];
                // What attaches within a session the agent sees is part of
                // its page.
                if session.is_some() || self.sees(info) {
                    if let Some(target) = info["targetId"].as_str() {
                        self.targets.insert(target.to_owned());
                    }
                    self.sessions.insert(attached.to_owned());
                    Reply::Pass
                } else {
                    Reply::Hide {
                        session: attached.to_owned(),
                        waiting: params["waitingForDebugger"] == true,
                    }
                }
            }
            "Target.detachedFromTarget" | "Target.receivedMessageFromTarget" => shown(
                params["sessionId"]
                    .as_str()
                    .is_some_and(|attached| self.sessions.contains(attached)),
            ),
            _ => Reply::Pass,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(message: &str) -> Option<(AgentInput, Option<String>)> {
        observe(&serde_json::from_str(message).unwrap())
    }

    fn input(message: &str) -> Option<AgentInput> {
        observed(message).map(|(input, _)| input)
    }

    #[test]
    fn mouse_events_are_seen_where_they_happen() {
        let event = |kind: &str| {
            format!(
                r#"{{"id":7,"method":"Input.dispatchMouseEvent","sessionId":"S","params":{{"type":"{kind}","x":120.5,"y":40,"button":"left"}}}}"#
            )
        };
        assert_eq!(
            input(&event("mouseMoved")),
            Some(AgentInput::Move { x: 120.5, y: 40. })
        );
        assert_eq!(
            observed(&event("mousePressed")),
            Some((AgentInput::Press { x: 120.5, y: 40. }, Some("S".to_owned())))
        );
        assert_eq!(
            input(&event("mouseReleased")),
            Some(AgentInput::Move { x: 120.5, y: 40. })
        );
        assert_eq!(
            input(&event("mouseWheel")),
            Some(AgentInput::Wheel { x: 120.5, y: 40. })
        );
    }

    #[test]
    fn keys_and_text_are_typing() {
        assert_eq!(
            input(r#"{"id":8,"method":"Input.insertText","params":{"text":"Rust"}}"#),
            Some(AgentInput::Type)
        );
        assert_eq!(
            input(
                r#"{"id":9,"method":"Input.dispatchKeyEvent","params":{"type":"keyDown","key":"Enter"}}"#
            ),
            Some(AgentInput::Type)
        );
    }

    #[test]
    fn other_messages_are_not_input() {
        assert_eq!(
            input(r#"{"id":1,"method":"Page.navigate","params":{"url":"https://example.com"}}"#),
            None
        );
        assert_eq!(
            input(r#"{"id":2,"method":"Runtime.evaluate","params":{"expression":"\"Input.x\""}}"#),
            None
        );
    }

    #[test]
    fn the_relay_keeps_its_own_answers() {
        assert_eq!(
            answer(
                r#"{"id":2000000003,"result":{"result":{"type":"object","value":[10,20.5,300,24]}},"sessionId":"S"}"#
            ),
            Answer::ToRelay(Some(AgentInput::Field {
                x: 10.,
                y: 20.5,
                width: 300.,
                height: 24.
            }))
        );
        assert_eq!(
            answer(
                r#"{"id":2000000004,"result":{"result":{"type":"object","subtype":"null","value":null}}}"#
            ),
            Answer::ToRelay(None)
        );
        assert_eq!(
            answer(r#"{"id":12,"result":{"result":{"type":"number","value":3}}}"#),
            Answer::ToAgent
        );
        assert_eq!(
            answer(r#"{"method":"Page.loadEventFired","params":{"timestamp":1}}"#),
            Answer::ToAgent
        );
    }

    #[test]
    fn an_answer_ends_where_its_length_says() {
        let answer = b"HTTP/1.1 200 OK\r\nContent-Length:7\r\n\r\n{\"a\":1}";
        assert_eq!(body(answer), Some(37..44));
        assert_eq!(body(&answer[..40]), None);
        assert_eq!(body(b"HTTP/1.1 200 OK\r\nContent-Len"), None);
    }

    fn page_scope() -> Scope {
        Scope::new("PAGE", Some("TAB"))
    }

    fn request(scope: &mut Scope, request: &str) -> Request {
        scope.request(&serde_json::from_str(request).unwrap())
    }

    #[test]
    fn the_agent_sees_its_tab_and_page_and_what_attaches_in_them() {
        let mut scope = page_scope();
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"T1","targetInfo":{"targetId":"TAB","type":"tab"},"waitingForDebugger":false}}"#
            ),
            Reply::Pass
        );
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":5,"sessionId":"T1","method":"Target.setAutoAttach","params":{}}"#
            ),
            Request::Pass
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","sessionId":"T1","params":{"sessionId":"P1","targetInfo":{"targetId":"PAGE","type":"page"},"waitingForDebugger":false}}"#
            ),
            Reply::Pass
        );
        // A frame of the page, attached within it.
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","sessionId":"P1","params":{"sessionId":"F1","targetInfo":{"targetId":"FRAME","type":"iframe"},"waitingForDebugger":true}}"#
            ),
            Reply::Pass
        );
        assert_eq!(
            scope.reply(r#"{"method":"Page.loadEventFired","params":{},"sessionId":"F1"}"#),
            Reply::Pass
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"FRAME","type":"iframe"}}}"#
            ),
            Reply::Pass
        );
    }

    #[test]
    fn other_pages_are_hidden_and_let_go() {
        let mut scope = page_scope();
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"OTHER","type":"page"}}}"#
            ),
            Reply::Drop
        );
        // Another session opened its page while the agent asked to attach
        // to every new tab: it waits to start until someone lets it.
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"T2","targetInfo":{"targetId":"OTHER-TAB","type":"tab"},"waitingForDebugger":true}}"#
            ),
            Reply::Hide {
                session: "T2".to_owned(),
                waiting: true
            }
        );
        assert_eq!(
            scope.reply(r#"{"method":"Target.detachedFromTarget","params":{"sessionId":"T2"}}"#),
            Reply::Drop
        );
        assert_eq!(
            scope.reply(r#"{"method":"Runtime.consoleAPICalled","params":{},"sessionId":"T2"}"#),
            Reply::Drop
        );
        let mut id = 9;
        assert_eq!(
            let_go("T2", true, || {
                id += 1;
                id
            }),
            [
                r#"{"id":10,"sessionId":"T2","method":"Runtime.runIfWaitingForDebugger"}"#,
                r#"{"id":11,"method":"Target.detachFromTarget","params":{"sessionId":"T2"}}"#,
            ]
        );
    }

    #[test]
    fn the_list_of_targets_names_only_those_the_agent_sees() {
        let mut scope = page_scope();
        assert_eq!(
            request(&mut scope, r#"{"id":7,"method":"Target.getTargets"}"#),
            Request::Pass
        );
        let Reply::Replace(listing) = scope.reply(
            r#"{"id":7,"result":{"targetInfos":[{"targetId":"PAGE","type":"page"},{"targetId":"OTHER","type":"page"},{"targetId":"B","type":"browser"}]}}"#,
        ) else {
            panic!("the list is cut down");
        };
        assert_eq!(
            listing,
            r#"{"id":7,"result":{"targetInfos":[{"targetId":"PAGE","type":"page"},{"targetId":"B","type":"browser"}]}}"#
        );
        // Other answers go through as they are.
        assert_eq!(scope.reply(r#"{"id":8,"result":{}}"#), Reply::Pass);
    }

    #[test]
    fn what_the_agent_asks_of_other_targets_is_refused() {
        let mut scope = page_scope();
        let refused = |scope: &mut Scope, message: &str| {
            matches!(request(scope, message), Request::Refuse(_))
        };
        assert!(refused(
            &mut scope,
            r#"{"id":1,"method":"Target.attachToTarget","params":{"targetId":"OTHER","flatten":true}}"#
        ));
        assert!(refused(
            &mut scope,
            r#"{"id":2,"sessionId":"T2","method":"Runtime.evaluate","params":{"expression":"1"}}"#
        ));
        assert!(refused(
            &mut scope,
            r#"{"id":3,"method":"Target.createTarget","params":{"url":"about:blank"}}"#
        ));
        assert!(refused(
            &mut scope,
            r#"{"id":4,"method":"Target.closeTarget","params":{"targetId":"PAGE"}}"#
        ));
        assert!(refused(&mut scope, r#"{"id":5,"method":"Browser.close"}"#));
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":6,"method":"Target.attachToTarget","params":{"targetId":"PAGE","flatten":true}}"#
            ),
            Request::Pass
        );
        assert_eq!(
            request(&mut scope, r#"{"id":8,"method":"Target.createTarget","params":{}}"#),
            Request::Refuse(
                r#"{"id":8,"error":{"code":-32000,"message":"Cox gives each session one page: navigate it instead of opening another"}}"#
                    .to_owned()
            )
        );
    }

    /// A browser with two tabs, each holding a page, and the relay in
    /// front of it, kept to one page: the agent hears of its page only,
    /// what it sends that page reaches the browser as it is, the answers
    /// come back, and the input is seen. The relay's own question, after
    /// the agent types, does not come back to the agent.
    #[tokio::test]
    async fn relays_both_ways_for_the_agents_page_and_sees_its_input() {
        let browser = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let devtools_port = browser.local_addr().unwrap().port();
        tokio::spawn(fake_browser(browser));
        let (_page_named, page) = tokio::sync::watch::channel(Some("PAGE".to_owned()));
        let (port, mut seen) = start(devtools_port, page).unwrap();

        let (mut agent, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/devtools/browser"))
                .await
                .unwrap();
        let mut ask = async |request: &str, answers: usize| {
            agent.send(Message::text(request)).await.unwrap();
            let mut heard = Vec::new();
            for _ in 0..answers {
                heard.push(
                    agent
                        .next()
                        .await
                        .unwrap()
                        .unwrap()
                        .into_text()
                        .unwrap()
                        .to_string(),
                );
            }
            heard
        };

        assert_eq!(
            ask(
                r#"{"id":0,"method":"Target.setDiscoverTargets","params":{"discover":true}}"#,
                2
            )
            .await,
            [
                r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"PAGE","type":"page"}}}"#,
                r#"{"id":0,"result":{}}"#,
            ]
        );
        assert_eq!(
            ask(r#"{"id":1,"method":"Target.attachToTarget","params":{"targetId":"OTHER","flatten":true}}"#, 1).await,
            [r#"{"id":1,"error":{"code":-32000,"message":"No target with given id found"}}"#]
        );
        assert_eq!(
            ask(r#"{"id":2,"method":"Target.attachToTarget","params":{"targetId":"PAGE","flatten":true}}"#, 2).await,
            [
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"s-PAGE","targetInfo":{"targetId":"PAGE","type":"page"},"waitingForDebugger":false}}"#,
                r#"{"id":2,"result":{"sessionId":"s-PAGE"}}"#,
            ]
        );
        for message in [
            r#"{"id":3,"sessionId":"s-PAGE","method":"Input.dispatchMouseEvent","params":{"type":"mousePressed","x":10,"y":20}}"#,
            r#"{"id":4,"sessionId":"s-PAGE","method":"Input.insertText","params":{"text":"Rust"}}"#,
            r#"{"id":5,"sessionId":"s-PAGE","method":"Page.reload","params":{}}"#,
        ] {
            assert_eq!(ask(message, 1).await, [message]);
        }
        assert_eq!(
            seen.recv().await,
            Some(AgentInput::Press { x: 10., y: 20. })
        );
        assert_eq!(seen.recv().await, Some(AgentInput::Type));
    }

    /// Answers the relay's `/json/version`, keeping the connection open as
    /// the browser does, then serves each connection to the endpoint it
    /// named: the tabs `TAB` and `OTHER-TAB` hold the pages `PAGE` and
    /// `OTHER`. It answers what the relay and the agent ask about them,
    /// and echoes the rest.
    async fn fake_browser(listener: TcpListener) {
        let port = listener.local_addr().unwrap().port();
        let (mut version, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let _ = version.read(&mut request).await.unwrap();
        let body = format!(
            r#"{{"Browser":"Fake","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/browser/fake"}}"#
        );
        let answer = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        version.write_all(answer.as_bytes()).await.unwrap();

        loop {
            let (socket, _) = listener.accept().await.unwrap();
            tokio::spawn(fake_endpoint(socket));
        }
    }

    async fn fake_endpoint(socket: TcpStream) {
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let info = |target: &str| {
            let kind = if target.ends_with("TAB") {
                "tab"
            } else {
                "page"
            };
            json!({ "targetId": target, "type": kind })
        };
        while let Some(Ok(message)) = socket.next().await {
            let Message::Text(text) = &message else {
                continue;
            };
            let request: Value = serde_json::from_str(text.as_str()).unwrap();
            let params = &request["params"];
            let mut sent = Vec::new();
            match request["method"].as_str().unwrap_or_default() {
                "Target.getTargets" => sent.push(json!({
                    "id": request["id"],
                    "result": { "targetInfos": [info("TAB"), info("OTHER-TAB")] },
                })),
                "Target.setDiscoverTargets" => {
                    for target in ["OTHER", "PAGE"] {
                        sent.push(json!({
                            "method": "Target.targetCreated",
                            "params": { "targetInfo": info(target) },
                        }));
                    }
                    sent.push(json!({ "id": request["id"], "result": {} }));
                }
                "Target.attachToTarget" => {
                    let target = params["targetId"].as_str().unwrap();
                    let session = format!("s-{target}");
                    sent.push(json!({
                        "method": "Target.attachedToTarget",
                        "params": {
                            "sessionId": session,
                            "targetInfo": info(target),
                            "waitingForDebugger": false,
                        },
                    }));
                    sent.push(json!({ "id": request["id"], "result": { "sessionId": session } }));
                }
                "Target.setAutoAttach" => {
                    let session = request["sessionId"].as_str().unwrap();
                    let page = session.trim_start_matches("s-").trim_end_matches("-TAB");
                    let page = if page == "TAB" { "PAGE" } else { page };
                    sent.push(json!({
                        "method": "Target.attachedToTarget",
                        "sessionId": session,
                        "params": {
                            "sessionId": format!("s-{page}"),
                            "targetInfo": info(page),
                            "waitingForDebugger": false,
                        },
                    }));
                    sent.push(json!({ "id": request["id"], "sessionId": session, "result": {} }));
                }
                _ => {
                    socket.send(message).await.unwrap();
                    continue;
                }
            }
            for message in sent {
                socket
                    .send(Message::text(message.to_string()))
                    .await
                    .unwrap();
            }
        }
    }
}
