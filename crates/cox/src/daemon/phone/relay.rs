//! The relay: how the phone reaches what is served on the Mac's own
//! address, as the web app the human works on, `http://localhost:5173`,
//! open on a stage, and the sketches' mocks (see [`super::mocks`]), each
//! at the same port of the Mac's name in the tailnet,
//! `https://<name>:5173`.
//!
//! `tailscale serve` hands each such port of the tailnet to the relay, on
//! the port after the page's (see [`super::tailnet`]), and the relay passes
//! what comes to it on to the port of the Mac's own address that it came
//! for, as its `Host` says, and what that answers back: so one relay
//! serves every port. The port keeps its number, so that a page that
//! makes its addresses from where it is, as its scripts' and its
//! WebSocket's, finds them through the relay too; a page that names
//! `localhost` itself does not.
//!
//! The relay passes on only what the human's phone sends, as Cox's page
//! takes it (see [`super::paired`]), and only to the ports the phone was
//! told of ([`Ports`]). It passes it on as made on the Mac, to
//! `localhost:<port>`, as a server that lets in only the names it knows,
//! as a dev server does, takes it; without what is Cox's own, as the
//! cookie the phone pairs with, and what Tailscale says of who sent it;
//! and a WebSocket's too, as the one a dev server reloads the page on.
//! What the relay passes back, Cox's page alone may frame.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::{COOKIE, paired};

/// What the relay answers: what the port it passed on to answered, or
/// why it did not.
pub type Body = UnsyncBoxBody<Bytes, hyper::Error>;

/// The port the relay listens on, for the page served on `page`: the one
/// after it.
#[must_use]
pub fn port(page: u16) -> Option<u16> {
    page.checked_add(1)
}

/// How long the phone waits for the tailnet to serve the ports it is
/// told of, before it is told of them anyway.
const WAITS: Duration = Duration::from_secs(8);

/// What wants a port passed on to the phone.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wanting {
    /// The sketches' mocks.
    Mocks,
    /// The stage of a folder, which has pages of the Mac's own address
    /// open, as the phone was last told.
    Stage(PathBuf),
}

/// The ports the relay passes on to, by what wants each, and those the
/// tailnet hands to it.
pub struct Ports {
    /// The port the relay listens on.
    relay: u16,
    /// The ports never passed on: the page's, and the relay's own.
    never: [u16; 2],
    wanted: watch::Sender<BTreeMap<Wanting, BTreeSet<u16>>>,
    served: watch::Sender<BTreeSet<u16>>,
    /// Whether coxd has the tailnet serve the ports, so that the phone
    /// waits for it to.
    tailnet: bool,
}

impl Ports {
    #[must_use]
    pub fn new(page: u16, relay: u16, tailnet: bool) -> Self {
        Self {
            relay,
            never: [page, relay],
            wanted: watch::Sender::new(BTreeMap::new()),
            served: watch::Sender::new(BTreeSet::new()),
            tailnet,
        }
    }

    /// The port the relay listens on.
    #[must_use]
    pub fn relay(&self) -> u16 {
        self.relay
    }

    /// Has the relay pass `ports` on, as `wanting` wants them now, in
    /// place of those it wanted before.
    pub fn want(&self, wanting: Wanting, mut ports: BTreeSet<u16>) {
        ports.retain(|port| !self.never.contains(port) && *port != 443);
        self.wanted.send_if_modified(|wanted| {
            let before = if ports.is_empty() {
                wanted.remove(&wanting)
            } else {
                wanted.insert(wanting, ports.clone())
            };
            before.unwrap_or_default() != ports
        });
    }

    /// Every port the relay passes on to.
    #[must_use]
    pub fn wanted(&self) -> BTreeSet<u16> {
        self.wanted.borrow().values().flatten().copied().collect()
    }

    /// Whether the relay passes on to `port`.
    #[must_use]
    pub fn passes(&self, port: u16) -> bool {
        self.wanted
            .borrow()
            .values()
            .any(|ports| ports.contains(&port))
    }

    /// What is told each time the ports wanted change.
    #[must_use]
    pub fn changes(&self) -> watch::Receiver<BTreeMap<Wanting, BTreeSet<u16>>> {
        self.wanted.subscribe()
    }

    /// The tailnet hands `ports` to the relay now.
    pub fn served(&self, ports: BTreeSet<u16>) {
        self.served.send_if_modified(|served| {
            let changed = *served != ports;
            *served = ports;
            changed
        });
    }

    /// Until the tailnet hands `ports` to the relay, or for a while, if
    /// it does not.
    pub async fn until_served(&self, ports: &BTreeSet<u16>) {
        if !self.tailnet || ports.is_empty() {
            return;
        }
        let mut served = self.served.subscribe();
        let _ =
            tokio::time::timeout(WAITS, served.wait_for(|served| ports.is_subset(served))).await;
    }
}

/// The port of the Mac's own address that `address` names, and the rest
/// of it, its path on, unless it is no page served there over plain
/// HTTP, as `http://localhost:5173/shop?item=2` is, at `5173`.
#[must_use]
pub fn on_the_mac(address: &str) -> Option<(u16, String)> {
    let url = url::Url::parse(address).ok()?;
    if url.scheme() != "http" {
        return None;
    }
    let local = match url.host()? {
        url::Host::Domain(name) => name == "localhost" || name.ends_with(".localhost"),
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    };
    let port = url.port_or_known_default()?;
    local.then(|| (port, url[url::Position::BeforePath..].to_owned()))
}

/// The Mac's name the request came for, as `headers` say it, without its
/// port: only one a name can be.
pub(super) fn host_name(headers: &HeaderMap) -> Option<&str> {
    came_for(headers).map(|(name, _)| name)
}

/// The Mac's name the request came for, as `headers` say it, and the
/// port, if it names one.
fn came_for(headers: &HeaderMap) -> Option<(&str, Option<u16>)> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => {
            (name, Some(port.parse().ok()?))
        }
        _ => (host, None),
    };
    (!name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-'))
    .then_some((name, port))
}

/// What the relay needs: the secret the human's phone pairs with, the
/// page's port, which Cox's page is served on, and the ports it passes on
/// to.
pub struct Relay {
    pub token: String,
    pub page: u16,
    pub ports: Rc<Ports>,
}

/// Passes on what comes on `listener`, as `relay` says.
pub async fn serve(listener: TcpListener, relay: Relay) {
    let relay = Rc::new(relay);
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::error!("a phone could not come to the relay: {error}");
                continue;
            }
        };
        let relay = Rc::clone(&relay);
        tokio::task::spawn_local(async move {
            let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                let relay = Rc::clone(&relay);
                async move { Ok::<_, Infallible>(relayed(request, &relay).await) }
            });
            if let Err(error) = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await
            {
                tracing::debug!("a phone's connection to the relay ended: {error}");
            }
        });
    }
}

/// What `request` is told: what the port it came for answers, for the
/// human's phone alone.
async fn relayed(request: Request<Incoming>, relay: &Relay) -> Response<Body> {
    let Some((name, Some(port))) = came_for(request.headers()) else {
        return told(StatusCode::BAD_REQUEST, "No port named".to_owned());
    };
    let name = name.to_owned();
    if !relay.ports.passes(port) {
        return told(
            StatusCode::NOT_FOUND,
            format!("Cox passes nothing on to port {port}"),
        );
    }
    if !paired(request.headers(), &relay.token) {
        return told(
            StatusCode::UNAUTHORIZED,
            "Cox does not know this phone: open Cox's page first.".to_owned(),
        );
    }
    let mut response = match pass_on(request, port, &name).await {
        Ok(response) => response,
        Err(why) => told(
            StatusCode::BAD_GATEWAY,
            format!("Nothing answers on the Mac at localhost:{port}: {why}"),
        ),
    };
    let ancestors = format!(
        "frame-ancestors https://{name} http://{name}:{page}",
        page = relay.page
    );
    if let Ok(policy) = HeaderValue::from_str(&ancestors) {
        // Beside the page's own policy, if it has one, which holds too.
        response
            .headers_mut()
            .append(header::CONTENT_SECURITY_POLICY, policy);
    }
    response
}

/// Passes `request` on to `port` of the Mac's own address, as made
/// there, and what it answers back, as answered at `name`: and, as a
/// WebSocket asks, what each then sends the other.
async fn pass_on(
    mut request: Request<Incoming>,
    port: u16,
    name: &str,
) -> Result<Response<Body>, String> {
    let phone = request
        .headers()
        .contains_key(header::UPGRADE)
        .then(|| hyper::upgrade::on(&mut request));
    as_made_on_the_mac(request.headers_mut(), port, name);
    // `localhost`, as a dev server may listen on its IPv6 address alone.
    let stream = TcpStream::connect(("localhost", port))
        .await
        .map_err(|error| error.to_string())?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|error| error.to_string())?;
    tokio::task::spawn_local(async move {
        if let Err(error) = connection.with_upgrades().await {
            tracing::debug!(port, "the relay's connection ended: {error}");
        }
    });
    let mut response = sender
        .send_request(request)
        .await
        .map_err(|error| error.to_string())?;
    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(phone) = phone
    {
        let there = hyper::upgrade::on(&mut response);
        tokio::task::spawn_local(async move {
            let (Ok(phone), Ok(there)) = (phone.await, there.await) else {
                return;
            };
            let _ =
                tokio::io::copy_bidirectional(&mut TokioIo::new(phone), &mut TokioIo::new(there))
                    .await;
        });
    }
    as_answered_at(response.headers_mut(), port, name);
    Ok(response.map(BodyExt::boxed_unsync))
}

/// Makes `headers`, of a request that came for `port` at `name`, as made
/// on the Mac to `localhost:<port>`, without what is Cox's own.
fn as_made_on_the_mac(headers: &mut HeaderMap, port: u16, name: &str) {
    let local = format!("localhost:{port}");
    if let Ok(host) = HeaderValue::from_str(&local) {
        headers.insert(header::HOST, host);
    }
    let tailscale: Vec<_> = headers
        .keys()
        .filter(|key| key.as_str().starts_with("tailscale-"))
        .cloned()
        .collect();
    for key in tailscale {
        headers.remove(key);
    }
    let cookies: Vec<String> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|cookies| cookies.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .map(str::trim)
        .filter(|cookie| {
            !cookie.is_empty()
                && cookie
                    .strip_prefix(COOKIE)
                    .is_none_or(|rest| !rest.starts_with('='))
        })
        .map(str::to_owned)
        .collect();
    headers.remove(header::COOKIE);
    if let Ok(cookies) = HeaderValue::from_str(&cookies.join("; "))
        && !cookies.is_empty()
    {
        headers.insert(header::COOKIE, cookies);
    }
    let there = format!("https://{name}:{port}");
    let here = format!("http://{local}");
    for key in [header::ORIGIN, header::REFERER] {
        let moved = headers
            .get(&key)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| moved(value, &there, &here));
        if let Some(value) = moved.and_then(|value| HeaderValue::from_str(&value).ok()) {
            headers.insert(key, value);
        }
    }
}

/// Makes `headers`, of an answer from `port` of the Mac's own address,
/// as answered at `name`: where it sends the phone to, as there.
fn as_answered_at(headers: &mut HeaderMap, port: u16, name: &str) {
    let there = format!("https://{name}:{port}");
    let moved = headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            moved(value, &format!("http://localhost:{port}"), &there)
                .or_else(|| moved(value, &format!("http://127.0.0.1:{port}"), &there))
        });
    if let Some(value) = moved.and_then(|value| HeaderValue::from_str(&value).ok()) {
        headers.insert(header::LOCATION, value);
    }
}

/// `address` at `to` in place of `from`, if it is at `from`.
fn moved(address: &str, from: &str, to: &str) -> Option<String> {
    let rest = address.strip_prefix(from)?;
    (rest.is_empty() || rest.starts_with(['/', '?', '#'])).then(|| format!("{to}{rest}"))
}

/// An answer of the relay's own, as why it passes nothing on.
fn told(status: StatusCode, text: String) -> Response<Body> {
    let mut response = Response::new(
        Full::new(Bytes::from(text))
            .map_err(|never: Infallible| match never {})
            .boxed_unsync(),
    );
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_name_and_port_the_request_came_for() {
        let headers = |host: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, host.parse().unwrap());
            headers
        };
        assert_eq!(
            came_for(&headers("mac.tail1234.ts.net:5173")),
            Some(("mac.tail1234.ts.net", Some(5173)))
        );
        assert_eq!(
            came_for(&headers("mac.tail1234.ts.net")),
            Some(("mac.tail1234.ts.net", None))
        );
        assert_eq!(host_name(&headers("127.0.0.1:7421")), Some("127.0.0.1"));
        assert_eq!(host_name(&headers("evil;x")), None);
        assert_eq!(came_for(&headers("mac:99999")), None);
    }

    #[test]
    fn finds_the_pages_of_the_macs_own_address() {
        assert_eq!(
            on_the_mac("http://localhost:5173/shop?item=2#top"),
            Some((5173, "/shop?item=2#top".to_owned()))
        );
        assert_eq!(
            on_the_mac("http://127.0.0.1:3000"),
            Some((3000, "/".to_owned()))
        );
        assert_eq!(
            on_the_mac("http://[::1]:8080/"),
            Some((8080, "/".to_owned()))
        );
        assert_eq!(
            on_the_mac("http://app.localhost:4000/"),
            Some((4000, "/".to_owned()))
        );
        assert_eq!(on_the_mac("http://localhost/"), Some((80, "/".to_owned())));
        assert_eq!(on_the_mac("https://localhost:5173/"), None);
        assert_eq!(on_the_mac("http://example.com:5173/"), None);
        assert_eq!(on_the_mac("cox://activity/"), None);
        assert_eq!(on_the_mac("not an address"), None);
    }

    #[test]
    fn passes_ports_on_as_wanted() {
        let ports = Ports::new(7421, 7422, false);
        ports.want(Wanting::Mocks, BTreeSet::from([7423]));
        ports.want(
            Wanting::Stage(PathBuf::from("/app")),
            BTreeSet::from([5173, 7421, 7422, 443]),
        );
        assert_eq!(ports.wanted(), BTreeSet::from([5173, 7423]));
        assert!(ports.passes(5173) && !ports.passes(7421) && !ports.passes(3000));
        ports.want(Wanting::Stage(PathBuf::from("/app")), BTreeSet::new());
        assert_eq!(ports.wanted(), BTreeSet::from([7423]));
    }

    #[test]
    fn makes_a_request_as_made_on_the_mac() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "mac.ts.net:5173".parse().unwrap());
        headers.insert("tailscale-user-login", "me@github".parse().unwrap());
        headers.insert("tailscale-user-name", "Me".parse().unwrap());
        headers.insert(
            header::COOKIE,
            "theme=dark; cox_phone=secret; cox_phone_x=1"
                .parse()
                .unwrap(),
        );
        headers.insert(header::ORIGIN, "https://mac.ts.net:5173".parse().unwrap());
        headers.insert(
            header::REFERER,
            "https://mac.ts.net:5173/shop".parse().unwrap(),
        );

        as_made_on_the_mac(&mut headers, 5173, "mac.ts.net");

        assert_eq!(headers[header::HOST], "localhost:5173");
        assert!(!headers.contains_key("tailscale-user-login"));
        assert!(!headers.contains_key("tailscale-user-name"));
        assert_eq!(headers[header::COOKIE], "theme=dark; cox_phone_x=1");
        assert_eq!(headers[header::ORIGIN], "http://localhost:5173");
        assert_eq!(headers[header::REFERER], "http://localhost:5173/shop");

        let mut alone = HeaderMap::new();
        alone.insert(header::COOKIE, "cox_phone=secret".parse().unwrap());
        alone.insert(header::ORIGIN, "https://mac.ts.net".parse().unwrap());
        as_made_on_the_mac(&mut alone, 5173, "mac.ts.net");
        assert!(!alone.contains_key(header::COOKIE));
        assert_eq!(alone[header::ORIGIN], "https://mac.ts.net");
    }

    #[test]
    fn sends_the_phone_where_the_mac_would_go() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::LOCATION,
            "http://localhost:5173/login?next=/".parse().unwrap(),
        );
        as_answered_at(&mut headers, 5173, "mac.ts.net");
        assert_eq!(
            headers[header::LOCATION],
            "https://mac.ts.net:5173/login?next=/"
        );
        headers.insert(header::LOCATION, "http://localhost:51730/".parse().unwrap());
        as_answered_at(&mut headers, 5173, "mac.ts.net");
        assert_eq!(headers[header::LOCATION], "http://localhost:51730/");
        headers.insert(header::LOCATION, "/elsewhere".parse().unwrap());
        as_answered_at(&mut headers, 5173, "mac.ts.net");
        assert_eq!(headers[header::LOCATION], "/elsewhere");
    }

    /// What `stream` sends up to the blank line that ends its head.
    async fn head(stream: &mut TcpStream) -> String {
        use tokio::io::AsyncReadExt as _;
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            if stream.read_exact(&mut byte).await.is_err() {
                break;
            }
            head.push(byte[0]);
        }
        String::from_utf8_lossy(&head).into_owned()
    }

    /// What `request` is answered by the relay on `relay`, whole.
    async fn asked(relay: u16, request: String) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut stream = TcpStream::connect(("127.0.0.1", relay)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut told = String::new();
        stream.read_to_string(&mut told).await.unwrap();
        told
    }

    /// An app on a port of the Mac's own, which tells on what it is sent
    /// each request's head, and a relay that passes that port on: the
    /// app's port and the relay's. A WebSocket's, it echoes five bytes.
    async fn relayed_app() -> (u16, u16, tokio::sync::mpsc::UnboundedReceiver<String>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let app = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let at = app.local_addr().unwrap().port();
        let (saw, seen) = tokio::sync::mpsc::unbounded_channel();
        tokio::task::spawn_local(async move {
            loop {
                let (mut stream, _) = app.accept().await.unwrap();
                let saw = saw.clone();
                tokio::task::spawn_local(async move {
                    let made = head(&mut stream).await;
                    let websocket = made.contains("upgrade: websocket");
                    saw.send(made).unwrap();
                    if websocket {
                        stream
                            .write_all(
                                b"HTTP/1.1 101 Switching Protocols\r\n\
                                  connection: upgrade\r\nupgrade: websocket\r\n\r\n",
                            )
                            .await
                            .unwrap();
                        let mut said = [0; 5];
                        stream.read_exact(&mut said).await.unwrap();
                        stream.write_all(&said).await.unwrap();
                    } else {
                        let answer = format!(
                            "HTTP/1.1 302 Found\r\nlocation: http://localhost:{at}/in\r\n\
                             content-length: 2\r\nconnection: close\r\n\r\nhi"
                        );
                        stream.write_all(answer.as_bytes()).await.unwrap();
                    }
                });
            }
        });
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let relay = listener.local_addr().unwrap().port();
        let ports = Rc::new(Ports::new(7421, relay, false));
        ports.want(Wanting::Stage(PathBuf::from("/app")), BTreeSet::from([at]));
        tokio::task::spawn_local(serve(
            listener,
            Relay {
                token: "secret".to_owned(),
                page: 7421,
                ports,
            },
        ));
        (at, relay, seen)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passes_a_request_on_to_the_port_it_came_for() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (at, relay, mut seen) = relayed_app().await;
                let told = asked(
                    relay,
                    format!(
                        "GET /shop HTTP/1.1\r\nhost: mac.ts.net:{at}\r\n\
                         tailscale-user-login: me@github\r\ncookie: cox_phone=secret; a=1\r\n\
                         connection: close\r\n\r\n"
                    ),
                )
                .await;
                assert!(told.starts_with("HTTP/1.1 302"), "{told}");
                assert!(told.contains(&format!("location: https://mac.ts.net:{at}/in")));
                assert!(told.contains("frame-ancestors https://mac.ts.net http://mac.ts.net:7421"));
                assert!(told.ends_with("hi"));
                let made = seen.recv().await.unwrap();
                assert!(made.starts_with("GET /shop HTTP/1.1"), "{made}");
                assert!(made.contains(&format!("host: localhost:{at}")), "{made}");
                assert!(made.contains("cookie: a=1\r\n"), "{made}");
                assert!(!made.contains("tailscale-user-login"), "{made}");

                let stranger = asked(
                    relay,
                    format!("GET / HTTP/1.1\r\nhost: mac.ts.net:{at}\r\nconnection: close\r\n\r\n"),
                )
                .await;
                assert!(stranger.starts_with("HTTP/1.1 401"), "{stranger}");
                let elsewhere = asked(
                    relay,
                    "GET / HTTP/1.1\r\nhost: mac.ts.net:22\r\ncookie: cox_phone=secret\r\n\
                     connection: close\r\n\r\n"
                        .to_owned(),
                )
                .await;
                assert!(elsewhere.starts_with("HTTP/1.1 404"), "{elsewhere}");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passes_nothing_on_for_what_tailscale_would_say_alone() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (at, relay, mut seen) = relayed_app().await;
                let told = asked(
                    relay,
                    format!(
                        "GET / HTTP/1.1\r\nhost: mac.ts.net:{at}\r\n\
                         tailscale-user-login: me@github\r\nconnection: close\r\n\r\n"
                    ),
                )
                .await;
                assert!(told.starts_with("HTTP/1.1 401"), "{told}");
                assert!(seen.try_recv().is_err(), "the app was sent the request");
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn passes_a_websocket_on() {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (at, relay, _seen) = relayed_app().await;
                let mut stream = TcpStream::connect(("127.0.0.1", relay)).await.unwrap();
                stream
                    .write_all(
                        format!(
                            "GET /hmr HTTP/1.1\r\nhost: mac.ts.net:{at}\r\n\
                             cookie: cox_phone=secret\r\nconnection: Upgrade\r\n\
                             upgrade: websocket\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                let switched = head(&mut stream).await;
                assert!(switched.starts_with("HTTP/1.1 101"), "{switched}");
                stream.write_all(b"hello").await.unwrap();
                let mut echoed = [0; 5];
                stream.read_exact(&mut echoed).await.unwrap();
                assert_eq!(&echoed, b"hello");
            })
            .await;
    }
}
