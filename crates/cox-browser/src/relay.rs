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
//!
//! - [`scope`] decides, message by message, what the agent sees and what
//!   is refused.
//! - [`seen`] sees the agent's input to a page in its messages, and
//!   asks the page where the agent types.
//! - [`devtools`] talks to the browser's `DevTools` port on the relay's
//!   own: its endpoint, and the tab that holds a page.

mod devtools;
mod scope;
mod seen;

use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use futures_util::{SinkExt as _, Stream, StreamExt as _};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{oneshot, watch};
use tokio_tungstenite::tungstenite::{self, Message};

use self::devtools::{browser_endpoint, config, find_tab};
use self::scope::{Ask, Reply, Request, Scope, let_go};
use self::seen::{Answer, OWN_IDS, answer, ask_focused, observe};
use crate::TargetId;

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
    pub(super) opening: usize,
    /// Each page, with the tab that holds it, if the browser has tabs, in
    /// the order the relay came to know them.
    pub(super) pages: Vec<(String, Option<String>)>,
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

/// How often, at most, the relay asks where the agent types: once for a
/// run of keys is enough.
const ASK_EVERY: Duration = Duration::from_millis(250);

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

/// Whether a message goes on to the other end. Pings and pongs are each
/// end's business with the relay, which answers them itself.
fn relayed(message: &Message) -> bool {
    matches!(message, Message::Text(_) | Message::Binary(_))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

    use super::*;

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
