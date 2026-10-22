//! The relay between the agent's browser tools and the stage's `DevTools`
//! port. The agent's tools connect here instead of to CEF; the relay passes
//! every message through as it is, and tells the stage of the input the
//! agent sends to the page, so the stage can show where the agent points,
//! clicks and types.
//!
//! Seen from the page, the agent's clicks and the human's are the same
//! clicks: only here, on the agent's own connection, can they be told
//! apart.
//!
//! Typing says nothing of where it goes, so when the agent types, the
//! relay asks the page itself which element has the focus, and keeps the
//! answer from the agent.

use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::ops::Range;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

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

/// Starts the relay on a port of its own, in front of the `DevTools` port
/// `devtools_port`, and returns its port with the input the agent sends.
///
/// # Errors
///
/// Returns an error if no port is free to listen on.
pub fn start(devtools_port: u16) -> anyhow::Result<(u16, UnboundedReceiver<AgentInput>)> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let (seen, received) = unbounded_channel();
    thread::spawn(move || {
        let served = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(anyhow::Error::from)
            .and_then(|runtime| runtime.block_on(serve(listener, devtools_port, seen)));
        if let Err(error) = served {
            log::error!("the relay to the stage stopped: {error:#}");
        }
    });
    Ok((port, received))
}

/// Accepts the agent's connections, and relays each to the browser.
async fn serve(
    listener: StdTcpListener,
    devtools_port: u16,
    seen: UnboundedSender<AgentInput>,
) -> anyhow::Result<()> {
    let listener = TcpListener::from_std(listener)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let seen = seen.clone();
        tokio::spawn(async move {
            if let Err(error) = relay(stream, devtools_port, seen).await {
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
    seen: UnboundedSender<AgentInput>,
) -> anyhow::Result<()> {
    let client = tokio_tungstenite::accept_async_with_config(client, Some(config())).await?;
    let endpoint = browser_endpoint(devtools_port).await?;
    let (browser, _) =
        tokio_tungstenite::connect_async_with_config(endpoint, Some(config()), true).await?;
    let (mut to_client, mut from_client) = client.split();
    let (mut to_browser, mut from_browser) = browser.split();
    let upstream = async {
        let mut asked = None::<Instant>;
        let mut next_id = OWN_IDS;
        while let Some(message) = from_client.next().await {
            let message = message?;
            let typed_in = if let Message::Text(text) = &message
                && let Some((input, session)) = observe(text.as_str())
            {
                // The stage may be gone while the agent carries on.
                let _ = seen.send(input);
                session.filter(|_| input == AgentInput::Type)
            } else {
                None
            };
            if relayed(&message) {
                to_browser.send(message).await?;
            }
            if let Some(session) = typed_in
                && asked.is_none_or(|asked| asked.elapsed() >= ASK_EVERY)
            {
                asked = Some(Instant::now());
                to_browser
                    .send(Message::text(ask_focused(next_id, &session)))
                    .await?;
                next_id += 1;
            }
        }
        anyhow::Ok(())
    };
    let downstream = async {
        while let Some(message) = from_browser.next().await {
            let message = message?;
            if let Message::Text(text) = &message
                && let Answer::ToRelay(field) = answer(text.as_str())
            {
                if let Some(field) = field {
                    let _ = seen.send(field);
                }
                continue;
            }
            if relayed(&message) {
                to_client.send(message).await?;
            }
        }
        anyhow::Ok(())
    };
    tokio::select! {
        ended = upstream => ended,
        ended = downstream => ended,
    }
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
fn observe(message: &str) -> Option<(AgentInput, Option<String>)> {
    // Most messages are not input; spare them the parsing.
    if !message.contains("\"Input.") {
        return None;
    }
    let message: Value = serde_json::from_str(message).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn input(message: &str) -> Option<AgentInput> {
        observe(message).map(|(input, _)| input)
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
            observe(&event("mousePressed")),
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

    /// A browser that serves `/json/version` and echoes on its endpoint,
    /// and the relay in front of it: what the agent sends reaches the
    /// browser as it is, the answers come back, and the input is seen. The
    /// relay's own question, after the agent types, does not come back to
    /// the agent.
    #[tokio::test]
    async fn relays_both_ways_and_sees_the_agents_input() {
        let browser = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let devtools_port = browser.local_addr().unwrap().port();
        tokio::spawn(fake_browser(browser));
        let (port, mut seen) = start(devtools_port).unwrap();

        let (mut agent, _) =
            tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/devtools/browser"))
                .await
                .unwrap();
        let sent = [
            r#"{"id":1,"sessionId":"S","method":"Input.dispatchMouseEvent","params":{"type":"mousePressed","x":10,"y":20}}"#,
            r#"{"id":2,"sessionId":"S","method":"Input.insertText","params":{"text":"Rust"}}"#,
            r#"{"id":3,"sessionId":"S","method":"Page.reload","params":{}}"#,
        ];
        for message in sent {
            agent.send(Message::text(message)).await.unwrap();
        }

        for message in sent {
            let echoed = agent.next().await.unwrap().unwrap();
            assert_eq!(echoed, Message::text(message));
        }
        assert_eq!(
            seen.recv().await,
            Some(AgentInput::Press { x: 10., y: 20. })
        );
        assert_eq!(seen.recv().await, Some(AgentInput::Type));
    }

    /// Answers the relay's `/json/version`, keeping the connection open as
    /// the browser does, then echoes each message on the endpoint it named.
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

        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            if message.is_text() {
                socket.send(message).await.unwrap();
            }
        }
    }
}
