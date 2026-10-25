//! The relay between one chat's agent and the browser's `DevTools` port.
//! The agent's browser tools connect here instead of to CEF; the relay
//! passes their messages through, keeping the agent to its own chat's
//! pages, and tells that chat's stage of the input the agent sends to a
//! page, so the stage can show where the agent points, clicks and types.
//!
//! Every chat's pages are in the same browser, so the relay hides the rest
//! of the browser from the agent: what it hears of other targets, and what
//! it asks of them (see [`Scope`]).
//!
//! The agent opens and closes pages as tabs of its chat's stage: the relay
//! asks the stage instead of the browser. Whoever opens a tab, the stage
//! tells the relay before CEF makes its page (see [`Pages`]), and the relay
//! holds back what the browser says until it knows the new page, so that
//! the agent hears of its chat's new tabs, and of no one else's.
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
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use futures_util::{SinkExt as _, Stream, StreamExt as _};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{oneshot, watch};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, Message};

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

/// What the relay tells the chat's stage, or asks of it, for the agent.
/// Pages are named by their `DevTools` target ids.
#[derive(Debug)]
pub enum FromAgent {
    /// The agent worked on `page`, when the relay knows which: sent it
    /// `input`, or with `None`, navigated it or brought it to the front.
    Worked {
        page: Option<String>,
        input: Option<AgentInput>,
    },
    /// The agent asks for a new tab showing `url`, shown unless it asks for
    /// it in the `background`. The stage answers with the new page's id,
    /// or `None` when it opened none.
    Open {
        url: String,
        background: bool,
        answer: oneshot::Sender<Option<TargetId>>,
    },
    /// The agent asks to close the tab of `page`. The stage answers whether
    /// it did, or why not.
    Close {
        page: String,
        answer: oneshot::Sender<Result<(), &'static str>>,
    },
}

/// What the relay knows of one chat's pages.
#[derive(Debug, Default)]
pub struct Known {
    /// How many pages are opening that the relay does not know yet.
    opening: usize,
    /// Each page, with the tab that holds it, if the browser has tabs, in
    /// the order the relay came to know them.
    pages: Vec<(String, Option<String>)>,
}

impl Known {
    fn has(&self, page: &str) -> bool {
        self.pages.iter().any(|(known, _)| known == page)
    }
}

/// One chat's pages, which its agent sees. The stage says when it opens a
/// page, before CEF makes it, and which page it is once CEF has made it;
/// the relay finds the tab that holds it. Cloning it shares the same
/// pages.
#[derive(Clone)]
pub struct Pages {
    known: Arc<watch::Sender<Known>>,
    /// Where the relay finds the tabs of new pages.
    names: UnboundedSender<TargetId>,
}

impl Pages {
    /// Says that a page of the chat is about to open. Call it before CEF
    /// is asked for the page, and say which page it is with
    /// [`Opening::is`].
    #[must_use]
    pub fn opening(&self) -> Opening {
        self.known.send_modify(|known| known.opening += 1);
        Opening {
            pages: Some(self.clone()),
        }
    }

    fn opened(&self, page: Option<(String, Option<String>)>) {
        self.known.send_modify(|known| {
            known.opening -= 1;
            known.pages.extend(page);
        });
    }
}

/// A page about to open, until the relay knows it. Dropped before saying
/// which page it is, it opened none.
pub struct Opening {
    pages: Option<Pages>,
}

impl Opening {
    /// The page that opened is `page`.
    pub fn is(mut self, page: TargetId) {
        if let Some(pages) = self.pages.take()
            && let Err(unsent) = pages.names.send(page)
        {
            // The relay is gone, and no one is waiting for the page.
            drop(unsent);
            pages.opened(None);
        }
    }
}

impl Drop for Opening {
    fn drop(&mut self) {
        if let Some(pages) = self.pages.take() {
            pages.opened(None);
        }
    }
}

/// The relay of one chat, as its stage holds it.
pub struct Relay {
    /// The port the agent's browser tools connect to.
    pub port: u16,
    pub pages: Pages,
    pub from_agent: UnboundedReceiver<FromAgent>,
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

/// How long the relay waits to know the pages that are opening.
const OPENING_TIMEOUT: Duration = PAGE_TIMEOUT.saturating_add(TAB_TIMEOUT);

/// Starts the relay of one chat on a port of its own, in front of the
/// `DevTools` port `devtools_port`. The chat's stage tells it of the pages
/// it opens.
///
/// # Errors
///
/// Returns an error if no port is free to listen on.
pub fn start(devtools_port: u16) -> anyhow::Result<Relay> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let (seen, from_agent) = unbounded_channel();
    let (names, to_find) = unbounded_channel();
    let pages = Pages {
        known: Arc::new(watch::Sender::new(Known::default())),
        names,
    };
    let known = Arc::clone(&pages.known);
    thread::spawn(move || {
        let served = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| {
                runtime.block_on(serve(listener, devtools_port, known, to_find, seen))
            });
        if let Err(error) = served {
            log::error!("a relay to the stage stopped: {error:#}");
        }
    });
    Ok(Relay {
        port,
        pages,
        from_agent,
    })
}

/// Accepts the agent's connections, and relays each to the browser, and
/// finds the tabs of the pages the stage opens.
async fn serve(
    listener: StdTcpListener,
    devtools_port: u16,
    known: Arc<watch::Sender<Known>>,
    mut to_find: UnboundedReceiver<TargetId>,
    seen: UnboundedSender<FromAgent>,
) -> anyhow::Result<()> {
    let listener = TcpListener::from_std(listener)?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let known = known.subscribe();
                let seen = seen.clone();
                tokio::spawn(async move {
                    if let Err(error) = relay(stream, devtools_port, known, seen).await {
                        log::warn!("a connection to the stage ended: {error:#}");
                    }
                });
            }
            Some(page) = to_find.recv() => {
                let known = Arc::clone(&known);
                tokio::spawn(async move {
                    let found = find_page(devtools_port, page).await;
                    known.send_modify(|known| {
                        known.opening -= 1;
                        known.pages.extend(found);
                    });
                });
            }
        }
    }
}

/// The page `page` turns out to be once CEF has named it, with the tab
/// that holds it, or `None` when CEF named none in time.
async fn find_page(devtools_port: u16, mut page: TargetId) -> Option<(String, Option<String>)> {
    let page = match tokio::time::timeout(PAGE_TIMEOUT, page.wait_for(Option::is_some)).await {
        Ok(Ok(page)) => page.clone()?,
        Ok(Err(_)) => return None,
        Err(_) => {
            log::warn!("CEF did not name a page in time, so its agent does not see it");
            return None;
        }
    };
    let tab = tokio::time::timeout(TAB_TIMEOUT, async {
        find_tab(&browser_endpoint(devtools_port).await?, &page).await
    })
    .await;
    let tab = match tab {
        Ok(Ok(tab)) => tab,
        Ok(Err(error)) => {
            log::warn!("no tab found for a page, so its agent gets the page alone: {error:#}");
            None
        }
        Err(_) => {
            log::warn!("no tab found for a page in time, so its agent gets the page alone");
            None
        }
    };
    Some((page, tab))
}

/// No limit on a message's size: a screenshot of a long page is large, and
/// both ends of the relay are on this machine.
fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None)
}

/// Where one connection of the relay's sends what it hears from either
/// end, and the numbers of its own requests.
struct Ends {
    client: UnboundedSender<Message>,
    browser: UnboundedSender<Message>,
    /// The stage, which may be gone while the agent carries on.
    stage: UnboundedSender<FromAgent>,
    next_id: AtomicU64,
}

impl Ends {
    fn own_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn to_client(&self, message: Message) {
        // The agent's end is gone once the connection closes.
        let _ = self.client.send(message);
    }

    fn to_browser(&self, message: Message) {
        let _ = self.browser.send(message);
    }

    fn to_stage(&self, heard: FromAgent) {
        let _ = self.stage.send(heard);
    }
}

/// Relays one connection of the agent's to the browser, whatever path it
/// asked for, until either end closes.
async fn relay(
    client: TcpStream,
    devtools_port: u16,
    mut known: watch::Receiver<Known>,
    stage: UnboundedSender<FromAgent>,
) -> anyhow::Result<()> {
    let client = tokio_tungstenite::accept_async_with_config(client, Some(config())).await?;
    let endpoint = browser_endpoint(devtools_port).await?;
    // The agent sees the pages the stage has opened so far.
    if tokio::time::timeout(OPENING_TIMEOUT, known.wait_for(|known| known.opening == 0))
        .await
        .is_err()
    {
        log::warn!("the stage's pages did not open in time, so its agent sees those that did");
    }
    let learned = known.clone();
    let opened = known.clone();
    let scope = Mutex::new(Scope::default());
    let scope = || {
        let mut scope = scope.lock().unwrap_or_else(PoisonError::into_inner);
        scope.learn(&learned.borrow());
        scope
    };
    let (browser, _) =
        tokio_tungstenite::connect_async_with_config(endpoint, Some(config()), true).await?;
    let (mut to_client, mut from_client) = client.split();
    let (mut to_browser, from_browser) = browser.split();
    // Both ends hear from both directions: the relay answers what it
    // refuses the agent, and lets go of what it hides.
    let (for_client, mut client_queue) = unbounded_channel::<Message>();
    let (for_browser, mut browser_queue) = unbounded_channel::<Message>();
    // What the agent asks of the stage, with the ids of its requests.
    let (for_stage, stage_queue) = unbounded_channel::<(Value, Ask)>();
    let ends = Ends {
        client: for_client,
        browser: for_browser,
        stage,
        next_id: AtomicU64::new(OWN_IDS),
    };
    let upstream = async {
        let mut asked = None::<Instant>;
        while let Some(message) = from_client.next().await {
            let message = message?;
            let Message::Text(text) = &message else {
                if relayed(&message) {
                    ends.to_browser(message);
                }
                continue;
            };
            let Ok(request) = serde_json::from_str::<Value>(text.as_str()) else {
                ends.to_browser(message);
                continue;
            };
            let decided = scope().request(&request);
            match decided {
                Request::Pass => {}
                Request::Refuse(answer) => {
                    ends.to_client(Message::text(answer));
                    continue;
                }
                Request::Ask(ask) => {
                    let _ = for_stage.send((request["id"].clone(), ask));
                    continue;
                }
            }
            let typed_in = if let Some((input, session)) = observe(&request) {
                let page = session
                    .as_deref()
                    .and_then(|session| scope().page_of(session));
                ends.to_stage(FromAgent::Worked { page, input });
                session.filter(|_| input == Some(AgentInput::Type))
            } else {
                None
            };
            ends.to_browser(message);
            if let Some(session) = typed_in
                && asked.is_none_or(|asked| asked.elapsed() >= ASK_EVERY)
            {
                asked = Some(Instant::now());
                ends.to_browser(Message::text(ask_focused(ends.own_id(), &session)));
            }
        }
        anyhow::Ok(())
    };
    let downstream = hear_browser(from_browser, known, |message| {
        deliver(message, &mut scope(), &ends);
    });
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
        ended = ask_stage(stage_queue, &ends, opened) => ended,
        ended = to_client => ended,
        ended = to_browser => ended,
    }
}

/// Delivers what the browser says, from `browser`, in order, holding it
/// back while a page is opening: it may be of that page, which the agent is
/// to hear of once the relay knows it.
async fn hear_browser(
    mut browser: impl Stream<Item = Result<Message, tungstenite::Error>> + Unpin,
    mut known: watch::Receiver<Known>,
    mut deliver: impl FnMut(Message),
) -> anyhow::Result<()> {
    let mut held = Vec::new();
    let mut watching = true;
    loop {
        tokio::select! {
            message = browser.next() => {
                let Some(message) = message else {
                    return Ok(());
                };
                let message = message?;
                if watching && known.borrow().opening > 0 {
                    held.push(message);
                } else {
                    held.drain(..).chain([message]).for_each(&mut deliver);
                }
            }
            changed = known.changed(), if watching => {
                // Once the stage is gone, no page opens any more.
                watching = changed.is_ok();
                if !watching || known.borrow().opening == 0 {
                    held.drain(..).for_each(&mut deliver);
                }
            }
        }
    }
}

/// Passes on `message`, from the browser, as `scope` has it.
fn deliver(message: Message, scope: &mut Scope, ends: &Ends) {
    let Message::Text(text) = &message else {
        if relayed(&message) {
            ends.to_client(message);
        }
        return;
    };
    if let Answer::ToRelay { field, session } = answer(text.as_str()) {
        if let Some(field) = field {
            let page = session
                .as_deref()
                .and_then(|session| scope.page_of(session));
            ends.to_stage(FromAgent::Worked {
                page,
                input: Some(field),
            });
        }
        return;
    }
    match scope.reply(text.as_str()) {
        Reply::Pass => ends.to_client(message),
        Reply::Replace(text) => ends.to_client(Message::text(text)),
        Reply::Drop => {}
        Reply::Hide { session, waiting } => {
            for request in let_go(&session, waiting, || ends.own_id()) {
                ends.to_browser(Message::text(request));
            }
        }
    }
}

/// Carries out what the agent asks of the stage, from `queue`, one at a
/// time, and answers the agent.
async fn ask_stage(
    mut queue: UnboundedReceiver<(Value, Ask)>,
    ends: &Ends,
    mut known: watch::Receiver<Known>,
) -> anyhow::Result<()> {
    while let Some((id, ask)) = queue.recv().await {
        let done = match ask {
            Ask::Open { url, background } => {
                open_tab(&ends.stage, &mut known, url, background).await
            }
            Ask::Close { page } => close_tab(&ends.stage, page).await,
        };
        let answer = match done {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(message) => json!({
                "id": id,
                "error": { "code": -32000, "message": message },
            }),
        };
        ends.to_client(Message::text(answer.to_string()));
    }
    Ok(())
}

/// Asks the stage for a new tab showing `url`, and answers with its page's
/// id once the relay knows the page, when the agent has heard of it.
async fn open_tab(
    stage: &UnboundedSender<FromAgent>,
    known: &mut watch::Receiver<Known>,
    url: String,
    background: bool,
) -> Result<Value, &'static str> {
    let (answer, answered) = oneshot::channel();
    stage
        .send(FromAgent::Open {
            url,
            background,
            answer,
        })
        .map_err(|_| "The stage is closed")?;
    let mut page = answered
        .await
        .ok()
        .flatten()
        .ok_or("The stage opened no tab")?;
    let page = tokio::time::timeout(PAGE_TIMEOUT, page.wait_for(Option::is_some))
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|page| page.clone())
        .ok_or("The new tab has no page")?;
    tokio::time::timeout(OPENING_TIMEOUT, known.wait_for(|known| known.has(&page)))
        .await
        .ok()
        .and_then(Result::ok)
        .map(drop)
        .ok_or("The new tab did not open in time")?;
    Ok(json!({ "targetId": page }))
}

/// Asks the stage to close the tab of `page`.
async fn close_tab(
    stage: &UnboundedSender<FromAgent>,
    page: String,
) -> Result<Value, &'static str> {
    let (answer, answered) = oneshot::channel();
    stage
        .send(FromAgent::Close { page, answer })
        .map_err(|_| "The stage is closed")?;
    answered.await.map_err(|_| "The stage is closed")??;
    Ok(json!({ "success": true }))
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

/// The work a CDP message from the agent does on a page, if any, and the
/// session of the page it goes to: the input it sends the page, or `None`
/// when it navigates the page or brings it to the front.
fn observe(message: &Value) -> Option<(Option<AgentInput>, Option<String>)> {
    let params = &message["params"];
    let at = || Some((float(&params["x"])?, float(&params["y"])?));
    let input = match message["method"].as_str()? {
        "Input.dispatchMouseEvent" => {
            let (x, y) = at()?;
            Some(match params["type"].as_str()? {
                "mousePressed" => AgentInput::Press { x, y },
                "mouseWheel" => AgentInput::Wheel { x, y },
                _ => AgentInput::Move { x, y },
            })
        }
        "Input.dispatchKeyEvent" | "Input.insertText" | "Input.imeSetComposition" => {
            Some(AgentInput::Type)
        }
        "Page.navigate" | "Page.navigateToHistoryEntry" | "Page.reload" | "Page.bringToFront" => {
            None
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
    /// The answer to the relay's own request, with the field it names, in
    /// the page of `session`.
    ToRelay {
        field: Option<AgentInput>,
        session: Option<String>,
    },
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
    Answer::ToRelay {
        field: field(),
        session: message["sessionId"].as_str().map(str::to_owned),
    }
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

/// What one chat's agent sees of the browser: its chat's pages, the tabs
/// that hold them, and the targets the agent attaches to within them, such
/// as the pages' frames and workers. Other chats' pages, and the rest of
/// the browser, stay hidden: the agent hears nothing of them, and what it
/// asks of them is refused, as if they were not there.
#[derive(Debug, Default)]
struct Scope {
    /// The chat's pages and the tabs that hold them, each with its page:
    /// the agent closes them through the stage.
    own: HashMap<String, String>,
    /// How many of the chat's pages are in `own`.
    learned: usize,
    /// The targets the agent sees.
    targets: HashSet<String>,
    /// The sessions, attached to those targets, that the agent sees, each
    /// with the page it is in, if it is in one.
    sessions: HashMap<String, Option<String>>,
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
    /// For the stage instead of the browser.
    Ask(Ask),
}

/// What the agent asks of the stage.
#[derive(Debug, PartialEq)]
enum Ask {
    Open { url: String, background: bool },
    Close { page: String },
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
    /// Takes in the chat's pages it has not seen yet.
    fn learn(&mut self, known: &Known) {
        for (page, tab) in known.pages.iter().skip(self.learned) {
            for target in [Some(page), tab.as_ref()].into_iter().flatten() {
                self.targets.insert(target.clone());
                self.own.insert(target.clone(), page.clone());
            }
        }
        self.learned = known.pages.len();
    }

    /// The page that `session` is in, if any.
    fn page_of(&self, session: &str) -> Option<String> {
        self.sessions.get(session).cloned().flatten()
    }

    /// What becomes of `request`, from the agent.
    fn request(&mut self, request: &Value) -> Request {
        let params = &request["params"];
        let session = request["sessionId"].as_str();
        let target = params["targetId"].as_str();
        let refused = match request["method"].as_str().unwrap_or_default() {
            _ if session.is_some_and(|session| !self.sessions.contains_key(session)) => {
                Some("No session with given id")
            }
            "Target.createTarget" => {
                let url = params["url"].as_str().filter(|url| !url.is_empty());
                return Request::Ask(Ask::Open {
                    url: url.unwrap_or("about:blank").to_owned(),
                    background: params["background"] == true,
                });
            }
            "Browser.close" | "Browser.crash" | "Browser.crashGpuProcess" => {
                Some("The browser is Cox's, and stays open")
            }
            "Target.closeTarget"
                if let Some(page) = target.and_then(|target| self.own.get(target)) =>
            {
                return Request::Ask(Ask::Close { page: page.clone() });
            }
            _ if target.is_some_and(|target| !self.targets.contains(target)) => {
                Some("No target with given id found")
            }
            "Target.detachFromTarget" | "Target.sendMessageToTarget"
                if params["sessionId"]
                    .as_str()
                    .is_some_and(|session| !self.sessions.contains_key(session)) =>
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
            .is_some_and(|session| !self.sessions.contains_key(session))
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
    /// itself has nothing of other chats in it. A page CEF has just made is
    /// a target of the type `other` until it turns into a page: the agent
    /// sees it once it has, through its tab, or its tools never take it
    /// for a page.
    fn sees(&self, info: &Value) -> bool {
        info["type"] == "browser"
            || (info["type"] != "other"
                && info["targetId"]
                    .as_str()
                    .is_some_and(|target| self.targets.contains(target)))
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
                // its pages.
                if session.is_some() || self.sees(info) {
                    let target = info["targetId"].as_str();
                    if let Some(target) = target {
                        self.targets.insert(target.to_owned());
                    }
                    // A page's frames and workers are in the page.
                    let page = if info["type"] == "page" {
                        target.map(str::to_owned)
                    } else {
                        session.and_then(|session| self.page_of(session))
                    };
                    self.sessions.insert(attached.to_owned(), page);
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
                    .is_some_and(|attached| self.sessions.contains_key(attached)),
            ),
            _ => Reply::Pass,
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

    use super::*;

    fn observed(message: &str) -> Option<(Option<AgentInput>, Option<String>)> {
        observe(&serde_json::from_str(message).unwrap())
    }

    fn input(message: &str) -> Option<AgentInput> {
        observed(message).and_then(|(input, _)| input)
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
            Some((
                Some(AgentInput::Press { x: 120.5, y: 40. }),
                Some("S".to_owned())
            ))
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
    fn navigating_is_work_on_the_page_with_no_input() {
        assert_eq!(
            observed(
                r#"{"id":1,"sessionId":"S","method":"Page.navigate","params":{"url":"https://example.com"}}"#
            ),
            Some((None, Some("S".to_owned())))
        );
        assert_eq!(
            observed(r#"{"id":2,"sessionId":"S","method":"Page.bringToFront"}"#),
            Some((None, Some("S".to_owned())))
        );
    }

    #[test]
    fn reading_the_page_is_no_work_on_it() {
        assert_eq!(
            observed(
                r#"{"id":2,"method":"Runtime.evaluate","params":{"expression":"\"Input.x\""}}"#
            ),
            None
        );
        assert_eq!(
            observed(r#"{"id":3,"sessionId":"S","method":"Page.captureScreenshot"}"#),
            None
        );
    }

    #[test]
    fn the_relay_keeps_its_own_answers() {
        assert_eq!(
            answer(
                r#"{"id":2000000003,"result":{"result":{"type":"object","value":[10,20.5,300,24]}},"sessionId":"S"}"#
            ),
            Answer::ToRelay {
                field: Some(AgentInput::Field {
                    x: 10.,
                    y: 20.5,
                    width: 300.,
                    height: 24.
                }),
                session: Some("S".to_owned()),
            }
        );
        assert_eq!(
            answer(
                r#"{"id":2000000004,"result":{"result":{"type":"object","subtype":"null","value":null}}}"#
            ),
            Answer::ToRelay {
                field: None,
                session: None
            }
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

    /// The scope of a chat with the page `PAGE` in the tab `TAB`.
    fn page_scope() -> Scope {
        let mut scope = Scope::default();
        scope.learn(&Known {
            opening: 0,
            pages: vec![("PAGE".to_owned(), Some("TAB".to_owned()))],
        });
        scope
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
            r#"{"id":3,"method":"Target.closeTarget","params":{"targetId":"OTHER"}}"#
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
            request(
                &mut scope,
                r#"{"id":7,"method":"Target.attachToTarget","params":{"targetId":"OTHER"}}"#
            ),
            Request::Refuse(
                r#"{"id":7,"error":{"code":-32000,"message":"No target with given id found"}}"#
                    .to_owned()
            )
        );
    }

    #[test]
    fn the_agent_opens_and_closes_its_tabs_through_the_stage() {
        let mut scope = page_scope();
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":1,"method":"Target.createTarget","params":{"url":"https://example.com/"}}"#
            ),
            Request::Ask(Ask::Open {
                url: "https://example.com/".to_owned(),
                background: false
            })
        );
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":2,"method":"Target.createTarget","params":{"url":"","background":true}}"#
            ),
            Request::Ask(Ask::Open {
                url: "about:blank".to_owned(),
                background: true
            })
        );
        // A page closes by its own id, or its tab's.
        for target in ["PAGE", "TAB"] {
            assert_eq!(
                request(
                    &mut scope,
                    &format!(
                        r#"{{"id":3,"method":"Target.closeTarget","params":{{"targetId":"{target}"}}}}"#
                    )
                ),
                Request::Ask(Ask::Close {
                    page: "PAGE".to_owned()
                })
            );
        }
    }

    #[test]
    fn the_agent_sees_the_pages_its_chat_opens_later() {
        let mut scope = page_scope();
        let mut known = Known {
            opening: 0,
            pages: vec![("PAGE".to_owned(), Some("TAB".to_owned()))],
        };
        let created = r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"NEW-TAB","type":"tab"}}}"#;
        assert_eq!(scope.reply(created), Reply::Drop);

        known
            .pages
            .push(("NEW".to_owned(), Some("NEW-TAB".to_owned())));
        scope.learn(&known);
        assert_eq!(scope.reply(created), Reply::Pass);
        // CEF makes the page as a target of another type first, which the
        // browser attaches the agent to; it attaches again as a page in
        // its tab.
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"NEW","type":"other"}}}"#
            ),
            Reply::Drop
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"O1","targetInfo":{"targetId":"NEW","type":"other"},"waitingForDebugger":true}}"#
            ),
            Reply::Hide {
                session: "O1".to_owned(),
                waiting: true
            }
        );
        assert_eq!(
            scope.reply(
                r#"{"method":"Target.targetInfoChanged","params":{"targetInfo":{"targetId":"NEW","type":"page"}}}"#
            ),
            Reply::Pass
        );
        assert_eq!(
            request(
                &mut scope,
                r#"{"id":1,"method":"Target.closeTarget","params":{"targetId":"NEW-TAB"}}"#
            ),
            Request::Ask(Ask::Close {
                page: "NEW".to_owned()
            })
        );
    }

    #[test]
    fn a_session_is_in_the_page_it_is_attached_to_or_within() {
        let mut scope = page_scope();
        for attached in [
            r#"{"method":"Target.attachedToTarget","params":{"sessionId":"T1","targetInfo":{"targetId":"TAB","type":"tab"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"T1","params":{"sessionId":"P1","targetInfo":{"targetId":"PAGE","type":"page"},"waitingForDebugger":false}}"#,
            r#"{"method":"Target.attachedToTarget","sessionId":"P1","params":{"sessionId":"F1","targetInfo":{"targetId":"FRAME","type":"iframe"},"waitingForDebugger":false}}"#,
        ] {
            assert_eq!(scope.reply(attached), Reply::Pass);
        }
        assert_eq!(scope.page_of("T1"), None);
        assert_eq!(scope.page_of("P1"), Some("PAGE".to_owned()));
        assert_eq!(scope.page_of("F1"), Some("PAGE".to_owned()));
    }

    /// The relay of a chat whose stage opened the page `PAGE`, in front of
    /// the fake browser, with the agent connected to it.
    async fn connect() -> (Agent, Relay, watch::Sender<Option<String>>) {
        let browser = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let devtools_port = browser.local_addr().unwrap().port();
        tokio::spawn(fake_browser(browser));
        let relay = start(devtools_port).unwrap();
        let (named, page) = watch::channel(Some("PAGE".to_owned()));
        relay.pages.opening().is(page);
        let (agent, _) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{}/devtools/browser",
            relay.port
        ))
        .await
        .unwrap();
        (agent, relay, named)
    }

    type Agent = WebSocketStream<MaybeTlsStream<TcpStream>>;

    /// Sends `request` as the agent, and returns the next `answers`
    /// messages the agent hears.
    async fn ask(agent: &mut Agent, request: &str, answers: usize) -> Vec<String> {
        agent.send(Message::text(request)).await.unwrap();
        hear(agent, answers).await
    }

    /// The next `answers` messages the agent hears.
    async fn hear(agent: &mut Agent, answers: usize) -> Vec<String> {
        let mut heard = Vec::new();
        for _ in 0..answers {
            let message = tokio::time::timeout(Duration::from_secs(5), agent.next())
                .await
                .expect("the relay answers");
            heard.push(message.unwrap().unwrap().into_text().unwrap().to_string());
        }
        heard
    }

    /// Takes what the relay tells the stage next, of the agent's work.
    async fn worked(relay: &mut Relay) -> (Option<String>, Option<AgentInput>) {
        match relay.from_agent.recv().await {
            Some(FromAgent::Worked { page, input }) => (page, input),
            other => panic!("the agent worked on a page, not {other:?}"),
        }
    }

    /// A browser with two tabs, each holding a page, and the relay in
    /// front of it, kept to one page: the agent hears of its page only,
    /// what it sends that page reaches the browser as it is, the answers
    /// come back, and the work is seen. The relay's own question, after
    /// the agent types, does not come back to the agent.
    #[tokio::test]
    async fn relays_both_ways_for_the_agents_page_and_sees_its_work() {
        let (mut agent, mut relay, _named) = connect().await;

        assert_eq!(
            ask(
                &mut agent,
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
            ask(&mut agent, r#"{"id":1,"method":"Target.attachToTarget","params":{"targetId":"OTHER","flatten":true}}"#, 1).await,
            [r#"{"id":1,"error":{"code":-32000,"message":"No target with given id found"}}"#]
        );
        assert_eq!(
            ask(&mut agent, r#"{"id":2,"method":"Target.attachToTarget","params":{"targetId":"PAGE","flatten":true}}"#, 2).await,
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
            assert_eq!(ask(&mut agent, message, 1).await, [message]);
        }
        let page = Some("PAGE".to_owned());
        assert_eq!(
            worked(&mut relay).await,
            (page.clone(), Some(AgentInput::Press { x: 10., y: 20. }))
        );
        assert_eq!(
            worked(&mut relay).await,
            (page.clone(), Some(AgentInput::Type))
        );
        assert_eq!(worked(&mut relay).await, (page, None));
    }

    /// The agent asks for a tab, which the stage opens; the agent hears of
    /// the new tab, which the browser told of while it was opening, and
    /// works on its page. Then it closes the tab, but not the last.
    #[tokio::test]
    async fn the_agent_opens_and_closes_tabs_on_the_stage() {
        let (mut agent, mut relay, _named) = connect().await;

        agent
            .send(Message::text(
                r#"{"id":1,"method":"Target.createTarget","params":{"url":"https://example.com/"}}"#,
            ))
            .await
            .unwrap();
        let Some(FromAgent::Open {
            url,
            background: false,
            answer,
        }) = relay.from_agent.recv().await
        else {
            panic!("the stage is asked for a tab");
        };
        assert_eq!(url, "https://example.com/");
        let opening = relay.pages.opening();
        // The fake browser echoes this back as if it told of the new tab.
        let created = r#"{"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"NEW-TAB","type":"tab"}}}"#;
        agent.send(Message::text(created)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (_new_named, new) = watch::channel(Some("NEW".to_owned()));
        opening.is(new.clone());
        answer.send(Some(new)).unwrap();
        // In either order.
        let mut heard = hear(&mut agent, 2).await;
        heard.sort();
        assert_eq!(heard, [r#"{"id":1,"result":{"targetId":"NEW"}}"#, created]);

        assert_eq!(
            ask(&mut agent, r#"{"id":2,"method":"Target.attachToTarget","params":{"targetId":"NEW","flatten":true}}"#, 2).await,
            [
                r#"{"method":"Target.attachedToTarget","params":{"sessionId":"s-NEW","targetInfo":{"targetId":"NEW","type":"page"},"waitingForDebugger":false}}"#,
                r#"{"id":2,"result":{"sessionId":"s-NEW"}}"#,
            ]
        );
        let click = r#"{"id":3,"sessionId":"s-NEW","method":"Input.dispatchMouseEvent","params":{"type":"mousePressed","x":1,"y":2}}"#;
        assert_eq!(ask(&mut agent, click, 1).await, [click]);
        assert_eq!(
            worked(&mut relay).await,
            (
                Some("NEW".to_owned()),
                Some(AgentInput::Press { x: 1., y: 2. })
            )
        );

        for (request, page, closed) in [
            (
                r#"{"id":4,"method":"Target.closeTarget","params":{"targetId":"NEW-TAB"}}"#,
                "NEW",
                Ok(()),
            ),
            (
                r#"{"id":5,"method":"Target.closeTarget","params":{"targetId":"PAGE"}}"#,
                "PAGE",
                Err("The stage keeps its last tab open"),
            ),
        ] {
            agent.send(Message::text(request)).await.unwrap();
            let Some(FromAgent::Close {
                page: asked,
                answer,
            }) = relay.from_agent.recv().await
            else {
                panic!("the stage is asked to close a tab");
            };
            assert_eq!(asked, page);
            answer.send(closed).unwrap();
        }
        assert_eq!(
            hear(&mut agent, 2).await,
            [
                r#"{"id":4,"result":{"success":true}}"#,
                r#"{"id":5,"error":{"code":-32000,"message":"The stage keeps its last tab open"}}"#,
            ]
        );
    }

    /// Answers each `/json/version`, keeping the connection open as the
    /// browser does, and serves each connection to the endpoint it named:
    /// the tabs `TAB`, `OTHER-TAB` and `NEW-TAB` hold the pages `PAGE`,
    /// `OTHER` and `NEW`. It answers what the relay and the agent ask about
    /// them, and echoes the rest.
    async fn fake_browser(listener: TcpListener) {
        let port = listener.local_addr().unwrap().port();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let peeked = socket.peek(&mut request).await.unwrap();
            if !request[..peeked].starts_with(b"GET /json/version") {
                tokio::spawn(fake_endpoint(socket));
                continue;
            }
            let _ = socket.read(&mut request).await.unwrap();
            let body = format!(
                r#"{{"Browser":"Fake","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/browser/fake"}}"#
            );
            let answer = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(answer.as_bytes()).await.unwrap();
            tokio::spawn(async move {
                // Open until the relay hangs up.
                let _ = socket.read(&mut request).await;
            });
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
                    "result": { "targetInfos": [info("TAB"), info("OTHER-TAB"), info("NEW-TAB")] },
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
