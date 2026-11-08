//! The page coxd serves the human's phone, for them to follow the chats
//! away from the Mac: the projects and chats, and a chat's conversation,
//! as they change.
//!
//! coxd serves it only when `settings.json` names a port, as
//! `{"phone": {"port": 7421}}`, and only on the Mac's own `127.0.0.1`:
//! the phone reaches it through `tailscale serve`, which hands the page
//! to the human's tailnet over HTTPS. Since any page in any browser on
//! the Mac can reach `127.0.0.1` too, every answer but the pairing asks
//! for a cookie holding the secret coxd keeps in `state/phone-token`:
//! the phone gets it by opening, once, the link `cox --phone-link`
//! prints, `/pair/<secret>`.
//!
//! The page is the files in `phone/`, built into Cox. It follows the
//! chats on `/events`, a stream of server-sent events, each a
//! [`FromDaemon`] as JSON: the [`FromDaemon::Listing`] of the projects
//! and chats whenever it changes, the [`FromDaemon::Patch`]es of the
//! chat named by `?chat=<key>`, all of it first, and the
//! [`FromDaemon::News`] of the chats that come to wait for the human.

use std::cell::Cell;
use std::convert::Infallible;
use std::io::Read as _;
use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Context as _;
use bytes::Bytes;
use http_body::Frame;
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::{Request, Response, StatusCode};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::Internal;
use super::wire::FromDaemon;
use crate::storage::{self, Kept};

/// The cookie that holds the secret, once the phone is paired.
const COOKIE: &str = "cox_phone";

/// How often the stream of events says something, even with nothing to
/// tell, so that what carries it, as `tailscale serve`, keeps it open.
const BEAT: Duration = Duration::from_secs(20);

/// What a phone that is not paired is told.
///
/// A page put on the home screen of an iPhone keeps cookies of its own,
/// apart from Safari's, so the page there pairs by the link pasted in.
const NOT_PAIRED: &str = "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" \
    content=\"width=device-width, initial-scale=1\"><title>Cox</title>\
    <link rel=\"stylesheet\" href=\"/app.css\"><main class=\"pair\"><h1>Cox</h1>\
    <p>This phone is not paired with Cox. On the Mac, run <code>cox --phone-link</code> and \
    open the link it prints here, or paste it:</p><form action=\"/pair\">\
    <input name=\"link\" autocomplete=\"off\" placeholder=\"https://…/pair/…\">\
    <button>Pair</button></form></main>";

/// The files a phone gets before it is paired: the page's look, for the
/// page that pairs it, and what puts the page on its home screen, which
/// the browser asks for without the cookie.
const PUBLIC: &[&str] = &["/app.css", "/manifest.webmanifest", "/icon.svg"];

/// The page's files, built into Cox: its path, its type, and what it
/// holds.
const FILES: &[(&str, &str, &str)] = &[
    (
        "/",
        "text/html; charset=utf-8",
        include_str!("phone/index.html"),
    ),
    (
        "/app.js",
        "text/javascript; charset=utf-8",
        include_str!("phone/app.js"),
    ),
    (
        "/app.css",
        "text/css; charset=utf-8",
        include_str!("phone/app.css"),
    ),
    (
        "/manifest.webmanifest",
        "application/manifest+json",
        include_str!("phone/manifest.webmanifest"),
    ),
    (
        "/icon.svg",
        "image/svg+xml",
        include_str!("../../assets/app-icon.svg"),
    ),
];

/// The port `settings.json` names for the page, if it names one.
#[must_use]
pub fn port() -> Option<u16> {
    let file = storage::path(Kept::Settings)?;
    let bytes = std::fs::read(&file).ok()?;
    port_in(&bytes)
        .inspect_err(|why| tracing::error!("\"phone\" in {} cannot be read: {why}", file.display()))
        .ok()
        .flatten()
}

/// The port the settings `bytes` name for the page, if they name one.
fn port_in(bytes: &[u8]) -> Result<Option<u16>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    match value.get("phone") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(phone) => match phone.get("port").and_then(serde_json::Value::as_u64) {
            Some(port) => u16::try_from(port)
                .map(Some)
                .map_err(|_| format!("{port} is no port")),
            None => Err("it names no \"port\"".to_owned()),
        },
    }
}

/// The secret a phone is paired with, made and kept the first time.
///
/// # Errors
///
/// When there is no place to keep it, or it cannot be read or kept.
pub fn token() -> anyhow::Result<String> {
    let file =
        storage::path(Kept::PhoneToken).context("there is no place for the phone's secret")?;
    token_at(&file)
}

fn token_at(file: &Path) -> anyhow::Result<String> {
    match std::fs::read_to_string(file) {
        Ok(token) if !token.trim().is_empty() => return Ok(token.trim().to_owned()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", file.display())),
    }
    let mut random = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut random))
        .context("cannot make a secret")?;
    let token = random.iter().fold(String::new(), |mut token, byte| {
        let _ = std::fmt::Write::write_fmt(&mut token, format_args!("{byte:02x}"));
        token
    });
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(&mut options.open(file)?, token.as_bytes())
        .with_context(|| format!("cannot keep {}", file.display()))?;
    Ok(token)
}

/// What `cox --phone-link` prints: the link that pairs a phone, through
/// the tailnet when `tailscale` tells the Mac's name there, and how to
/// serve the page there.
///
/// # Errors
///
/// When the settings name no port, or the secret cannot be had.
pub fn link() -> anyhow::Result<String> {
    let port = port().context(
        "settings.json names no port for the phone's page, as {\"phone\": {\"port\": 7421}}",
    )?;
    let token = token()?;
    Ok(match tailnet_name() {
        Some(name) => format!(
            "Serve the page to your tailnet, once:\n\n  tailscale serve --bg {port}\n\n\
             Then open this on your phone:\n\n  https://{name}/pair/{token}\n"
        ),
        None => format!(
            "tailscale does not say the Mac's name in a tailnet. On this machine, open:\n\n  \
             http://127.0.0.1:{port}/pair/{token}\n"
        ),
    })
}

/// The Mac's name in the tailnet, as `tailscale status` says it.
fn tailnet_name() -> Option<String> {
    [
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ]
    .into_iter()
    .find_map(|program| {
        let output = std::process::Command::new(program)
            .args(["status", "--json"])
            .output()
            .ok()?;
        let status: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let name = status.pointer("/Self/DNSName")?.as_str()?;
        let name = name.trim_end_matches('.');
        (!name.is_empty()).then(|| name.to_owned())
    })
}

/// Listens for the phone on `port` of the Mac's own address.
///
/// # Errors
///
/// When the port cannot be listened on.
pub async fn listen(port: u16) -> anyhow::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("cannot listen for the phone on port {port}"))
}

/// Serves the page on `listener` to the phones paired with `token`,
/// telling the daemon, on `internal`, of the phones that follow the
/// chats.
pub async fn serve(listener: TcpListener, token: String, internal: UnboundedSender<Internal>) {
    let page = Rc::new(Page {
        token,
        internal,
        last: Cell::new(0),
    });
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::error!("a phone could not come: {error}");
                continue;
            }
        };
        let page = Rc::clone(&page);
        tokio::task::spawn_local(async move {
            let service = hyper::service::service_fn(move |request| {
                let answer = page.answer(&request);
                async move { Ok::<_, Infallible>(answer) }
            });
            if let Err(error) = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await
            {
                tracing::debug!("a phone's connection ended: {error}");
            }
        });
    }
}

/// What answers the phones.
struct Page {
    token: String,
    internal: UnboundedSender<Internal>,
    /// The number of the last phone that followed the chats.
    last: Cell<u64>,
}

impl Page {
    fn answer(&self, request: &Request<Incoming>) -> Response<Body> {
        let path = request.uri().path();
        if let Some(token) = path.strip_prefix("/pair/") {
            return self.pair(token);
        }
        if path == "/pair" {
            let link = request.uri().query().unwrap_or_default();
            return self.pair(pasted(link));
        }
        if !PUBLIC.contains(&path) && !self.paired(request) {
            return answer(
                StatusCode::UNAUTHORIZED,
                "text/html; charset=utf-8",
                NOT_PAIRED,
            );
        }
        if path == "/events" {
            let chat = request.uri().query().and_then(|query| {
                query
                    .split('&')
                    .find_map(|pair| pair.strip_prefix("chat="))
                    .and_then(|key| key.parse().ok())
            });
            return self.events(chat);
        }
        match FILES.iter().find(|(at, _, _)| *at == path) {
            Some((_, kind, text)) => answer(StatusCode::OK, kind, text),
            None => answer(
                StatusCode::NOT_FOUND,
                "text/plain; charset=utf-8",
                "Not found",
            ),
        }
    }

    /// Pairs the phone that opened `/pair/<token>`, when it is the secret.
    fn pair(&self, token: &str) -> Response<Body> {
        if !same(token.as_bytes(), self.token.as_bytes()) {
            return answer(
                StatusCode::FORBIDDEN,
                "text/html; charset=utf-8",
                NOT_PAIRED,
            );
        }
        let mut response = answer(StatusCode::SEE_OTHER, "text/plain; charset=utf-8", "Paired");
        let headers = response.headers_mut();
        headers.insert(header::LOCATION, HeaderValue::from_static("/"));
        // Browsers keep a cookie 400 days at most.
        let cookie = format!(
            "{COOKIE}={}; Path=/; Max-Age=34560000; HttpOnly; Secure; SameSite=Strict",
            self.token
        );
        if let Ok(cookie) = HeaderValue::from_str(&cookie) {
            headers.insert(header::SET_COOKIE, cookie);
        }
        response
    }

    /// Whether `request` holds the secret in its cookie.
    fn paired(&self, request: &Request<Incoming>) -> bool {
        request
            .headers()
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|cookies| cookies.to_str().ok())
            .flat_map(|cookies| cookies.split(';'))
            .filter_map(|cookie| cookie.trim().strip_prefix(COOKIE)?.strip_prefix('='))
            .any(|token| same(token.as_bytes(), self.token.as_bytes()))
    }

    /// The stream of events of a phone that follows the chats, watching
    /// the conversation of `chat`, if it names one.
    fn events(&self, chat: Option<u64>) -> Response<Body> {
        let id = self.last.get() + 1;
        self.last.set(id);
        let (phone, told) = unbounded_channel();
        let came = self.internal.send(Internal::View { id, phone }).is_ok()
            && self
                .internal
                .send(Internal::ViewChat { id, key: chat })
                .is_ok();
        if !came {
            return answer(
                StatusCode::SERVICE_UNAVAILABLE,
                "text/plain; charset=utf-8",
                "coxd is stopping",
            );
        }
        let events = Events {
            told,
            beat: tokio::time::interval(BEAT),
            id,
            internal: self.internal.clone(),
        };
        let mut response = Response::new(Body::Events(events));
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

/// The secret in the query `link=<link>` of the form a phone pastes the
/// link in: its last 64 characters, as they end the link.
fn pasted(query: &str) -> &str {
    let link = query
        .strip_prefix("link=")
        .unwrap_or(query)
        .trim_end_matches(['+', '/']);
    link.get(link.len().saturating_sub(64)..)
        .unwrap_or_default()
}

/// Whether `a` and `b` are the same, taking as long whatever byte they
/// differ at, so that the time an answer takes tells nothing of the
/// secret.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |differ, (a, b)| differ | (a ^ b)) == 0
}

/// An answer of `status`, holding `text` of the type `kind`.
fn answer(status: StatusCode, kind: &'static str, text: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::Whole(Some(Bytes::from_static(text.as_bytes()))));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    // What the agents write shows as text: the page runs only its own
    // script.
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; \
             frame-ancestors 'none'",
        ),
    );
    response
}

/// What an answer holds: a file whole, or the stream of events.
enum Body {
    Whole(Option<Bytes>),
    Events(Events),
}

impl http_body::Body for Body {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.get_mut() {
            Self::Whole(bytes) => Poll::Ready(bytes.take().map(|bytes| Ok(Frame::data(bytes)))),
            Self::Events(events) => events
                .poll(context)
                .map(|bytes| bytes.map(|bytes| Ok(Frame::data(bytes)))),
        }
    }
}

/// The stream of events a phone follows the chats on, until it goes.
struct Events {
    told: UnboundedReceiver<FromDaemon>,
    beat: tokio::time::Interval,
    id: u64,
    internal: UnboundedSender<Internal>,
}

impl Events {
    fn poll(&mut self, context: &mut Context<'_>) -> Poll<Option<Bytes>> {
        if let Poll::Ready(told) = self.told.poll_recv(context) {
            return Poll::Ready(told.map(|told| event(&told)));
        }
        if self.beat.poll_tick(context).is_ready() {
            return Poll::Ready(Some(Bytes::from_static(b": beat\n\n")));
        }
        Poll::Pending
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        let _ = self.internal.send(Internal::Unview { id: self.id });
    }
}

/// `told` as a server-sent event: a line of JSON.
fn event(told: &FromDaemon) -> Bytes {
    let json = serde_json::to_string(told).unwrap_or_else(|error| {
        tracing::error!("the phone cannot be told: {error}");
        "null".to_owned()
    });
    Bytes::from(format!("data: {json}\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_port_from_the_settings() {
        assert_eq!(port_in(br#"{"home": "~/cox"}"#), Ok(None));
        assert_eq!(port_in(br#"{"phone": {"port": 7421}}"#), Ok(Some(7421)));
        assert!(port_in(br#"{"phone": {"port": 70000}}"#).is_err());
        assert!(port_in(br#"{"phone": {}}"#).is_err());
    }

    #[test]
    fn keeps_the_secret_it_makes() {
        let folder = std::env::temp_dir().join(format!("cox-phone-token-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("phone-token");
        let secret = token_at(&file).unwrap();
        assert_eq!(secret.len(), 64);
        assert_eq!(token_at(&file).unwrap(), secret);
        let mode = std::fs::metadata(&file).unwrap().permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
            0o600
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// What the page answers `request`, written whole on a connection
    /// of its own to `port`, up to `until` in what it answers, or all of
    /// it when `until` is empty.
    async fn ask(port: u16, request: &str, until: &str) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        let mut chunk = [0_u8; 4096];
        while until.is_empty() || !String::from_utf8_lossy(&answer).contains(until) {
            let read = stream.read(&mut chunk).await.unwrap();
            if read == 0 {
                break;
            }
            answer.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8_lossy(&answer).into_owned()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn answers_only_a_paired_phone_and_tells_it_the_chats() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
                let listener = listen(0).await.unwrap();
                let port = listener.local_addr().unwrap().port();
                let (internal, mut heard) = unbounded_channel();
                tokio::task::spawn_local(serve(listener, "secret".to_owned(), internal));

                let unpaired = ask(port, "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", "").await;
                assert!(unpaired.starts_with("HTTP/1.1 401"), "{unpaired}");
                let look = ask(
                    port,
                    "GET /app.css HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
                    "",
                )
                .await;
                assert!(look.starts_with("HTTP/1.1 200"), "{look}");
                let wrong = ask(port, "GET /pair/guess HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", "").await;
                assert!(wrong.starts_with("HTTP/1.1 403"), "{wrong}");
                let paired = ask(
                    port,
                    "GET /pair/secret HTTP/1.1\r\nHost: x\r\n\r\n",
                    "Paired",
                )
                .await;
                assert!(paired.starts_with("HTTP/1.1 303"), "{paired}");
                assert!(
                    paired.contains("set-cookie: cox_phone=secret; Path=/;"),
                    "{paired}"
                );
                let page = ask(
                    port,
                    "GET / HTTP/1.1\r\nHost: x\r\nCookie: a=b; cox_phone=secret\r\nConnection: close\r\n\r\n",
                    "",
                )
                .await;
                assert!(page.starts_with("HTTP/1.1 200"), "{page}");

                let events = tokio::task::spawn_local(ask(
                    port,
                    "GET /events?chat=7 HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\n\r\n",
                    "}}\n\n",
                ));
                let Some(Internal::View { id, phone }) = heard.recv().await else {
                    panic!("the phone did not come");
                };
                assert!(matches!(
                    heard.recv().await,
                    Some(Internal::ViewChat { id: chat_of, key: Some(7) }) if chat_of == id
                ));
                phone
                    .send(FromDaemon::Listing(super::super::wire::Listing::default()))
                    .unwrap();
                let told = events.await.unwrap();
                assert!(told.contains("text/event-stream"), "{told}");
                assert!(
                    told.contains(r#"data: {"Listing":{"projects":[],"chats":[],"archived":[],"agents":{}}}"#),
                    "{told}"
                );
                assert!(
                    matches!(heard.recv().await, Some(Internal::Unview { id: gone }) if gone == id),
                    "the phone that went is not said to have gone"
                );
            });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[test]
    fn finds_the_secret_in_the_link_pasted() {
        let secret = "0123456789abcdef".repeat(4);
        assert_eq!(
            pasted(&format!(
                "link=https%3A%2F%2Fmac.tail.ts.net%2Fpair%2F{secret}"
            )),
            secret
        );
        assert_eq!(pasted(&format!("link={secret}+")), secret);
        assert_eq!(pasted("link=short"), "short");
    }

    #[test]
    fn tells_the_secret_only_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
    }
}
