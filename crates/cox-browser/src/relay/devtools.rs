//! The browser's `DevTools` port, as the relay talks to it on its own:
//! the endpoint the browser names, and the tab that holds a page, which
//! nothing but attaching to the tab tells.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::ops::Range;

use anyhow::Context as _;
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// No limit on a message's size: a screenshot of a long page is large, and
/// both ends of the relay are on this machine.
pub(super) fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None)
}

/// The browser's own `DevTools` endpoint, which `/json/version` names.
pub(super) async fn browser_endpoint(devtools_port: u16) -> anyhow::Result<String> {
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

/// The tab that holds `page`, if the browser has tabs: Chromium puts each
/// page in a tab target, and the agent's tools reach pages through their
/// tabs. Nothing links a tab to its page but attaching to the tab, so the
/// relay does that on a connection of its own, which it then drops.
pub(super) async fn find_tab(endpoint: &str, page: &str) -> anyhow::Result<Option<String>> {
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

#[cfg(test)]
mod tests {
    use super::body;

    #[test]
    fn an_answer_ends_where_its_length_says() {
        let answer = b"HTTP/1.1 200 OK\r\nContent-Length:7\r\n\r\n{\"a\":1}";
        assert_eq!(body(answer), Some(37..44));
        assert_eq!(body(&answer[..40]), None);
        assert_eq!(body(b"HTTP/1.1 200 OK\r\nContent-Len"), None);
    }
}
