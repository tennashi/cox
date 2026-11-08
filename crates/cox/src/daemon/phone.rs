//! The page coxd serves the human's phone, for them to follow and answer
//! the chats away from the Mac: the projects and chats, and a chat's
//! conversation, as they change.
//!
//! coxd serves it unless `settings.json` turns it off, as
//! `{"phone": false}`, on [`PORT`], and only on the Mac's own `127.0.0.1`:
//! the phone reaches it through `tailscale serve`, which hands the page
//! to the human's tailnet over HTTPS, and which coxd sets up as it
//! starts (see [`tailnet`]). Since any page in any browser on the Mac
//! can reach `127.0.0.1` too, every answer but the pairing is for the
//! human alone: a phone of the Mac's login in the tailnet, as `tailscale
//! serve` says who it is, or else one that holds, in a cookie, the
//! secret coxd keeps in `state/phone-token`, which it gets by opening,
//! once, `/pair/<secret>`.
//!
//! The page is the files in `phone/`, built into Cox. It follows the
//! chats on `/events`, a stream of server-sent events, each a
//! [`FromDaemon`] as JSON: the [`FromDaemon::Listing`] of the projects
//! and chats whenever it changes, the [`FromDaemon::Patch`]es of the
//! chat named by `?chat=<key>`, all of it first, and the
//! [`FromDaemon::News`] of the chats that come to wait for the human.
//! What the human does on it, as sending a line or answering what the
//! agent asks, it posts to `/act`, a [`ToDaemon`] as JSON, as the window
//! says it, which coxd does as the window's (see [`done_on_the_page`]
//! for which).
//!
//! The page asks, too, to be told of the chats that need the human while
//! they are away from the Mac, with Web Push (see [`super::push`]): it
//! reads coxd's key on `/push`, and posts where to push to it there, or
//! to `/push/end` to be told no more, its service worker, `/sw.js`,
//! showing what is pushed.

use std::cell::{Cell, RefCell};
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
use hyper::{Method, Request, Response, StatusCode};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub mod mocks;
pub(crate) mod tailnet;

use super::Internal;
use super::push::{self, Notice, Pushed, Pushes, Subscription};
use super::wire::{FromDaemon, ToDaemon};
use crate::storage::{self, Kept};

/// The cookie that holds the secret, once the phone is paired.
const COOKIE: &str = "cox_phone";

/// How often the stream of events says something, even with nothing to
/// tell, so that what carries it, as `tailscale serve`, keeps it open.
const BEAT: Duration = Duration::from_secs(20);

/// What a phone that is not the human's is told.
///
/// A page put on the home screen of an iPhone keeps cookies of its own,
/// apart from Safari's, so the page there pairs by the link pasted in.
const NOT_PAIRED: &str = "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" \
    content=\"width=device-width, initial-scale=1\"><title>Cox</title>\
    <link rel=\"stylesheet\" href=\"/app.css\"><main class=\"pair\"><h1>Cox</h1>\
    <p>Cox does not know this phone. Open the page through Tailscale, logged in as the Mac \
    is, or, on the Mac, run <code>cox --phone-link</code> and open the pairing link it prints \
    here, or paste it:</p><form action=\"/pair\">\
    <input name=\"link\" autocomplete=\"off\" placeholder=\"https://…/pair/…\">\
    <button>Pair</button></form></main>";

/// The files a phone gets before it is paired: the page's look, for the
/// page that pairs it, and what puts the page on its home screen, and the
/// service worker, which the browser asks for without the cookie.
const PUBLIC: &[&str] = &["/app.css", "/manifest.webmanifest", "/icon.svg", "/sw.js"];

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
        "/sw.js",
        "text/javascript; charset=utf-8",
        include_str!("phone/sw.js"),
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

/// The port the page is served on unless `settings.json` names another:
/// what `tailscale serve` hands to the tailnet, and nothing the human
/// has to choose.
pub const PORT: u16 = 7421;

/// The port the page is served on, or none when `settings.json` turns it
/// off, as `{"phone": false}`.
#[must_use]
pub fn port() -> Option<u16> {
    let Some(file) = storage::path(Kept::Settings) else {
        return Some(PORT);
    };
    let Ok(bytes) = std::fs::read(&file) else {
        return Some(PORT);
    };
    port_in(&bytes)
        .inspect_err(|why| tracing::error!("\"phone\" in {} cannot be read: {why}", file.display()))
        .unwrap_or(Some(PORT))
}

/// The port the settings `bytes` say the page is served on: [`PORT`],
/// unless they turn it off, or name another, as `{"phone": {"port":
/// 7422}}`.
fn port_in(bytes: &[u8]) -> Result<Option<u16>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    match value.get("phone") {
        None | Some(serde_json::Value::Null | serde_json::Value::Bool(true)) => Ok(Some(PORT)),
        Some(serde_json::Value::Bool(false)) => Ok(None),
        Some(phone) => match phone.get("port") {
            None => Ok(Some(PORT)),
            Some(port) => port
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .map(Some)
                .ok_or_else(|| format!("{port} is no port")),
        },
    }
}

/// Whether coxd has the tailnet hand the page to the phone, and takes a
/// phone of the Mac's login there as the human's: unless `settings.json`
/// says not, as `{"phone": {"tailscale": false}}`.
#[must_use]
pub fn tailscale() -> bool {
    storage::path(Kept::Settings)
        .and_then(|file| std::fs::read(file).ok())
        .is_none_or(|bytes| tailscale_in(&bytes))
}

/// Whether the settings `bytes` leave the tailnet to coxd.
fn tailscale_in(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.pointer("/phone/tailscale")?.as_bool())
        .unwrap_or(true)
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

/// What `cox --phone-link` prints: where the phone opens the page,
/// through the tailnet when `tailscale` tells the Mac's name there, with
/// what the tailnet lacks for it, and the link that pairs a phone of
/// another login, or one that comes some other way.
///
/// # Errors
///
/// When the settings turn the page off, or the secret cannot be had.
pub fn link() -> anyhow::Result<String> {
    use std::fmt::Write as _;

    let port =
        port().context("settings.json turns the phone's page off, with {\"phone\": false}")?;
    let token = token()?;
    let status = tailscale().then(tailnet::status).flatten();
    let Some((
        program,
        tailnet::Status {
            owner: Some(owner),
            magic_dns,
        },
    )) = status
    else {
        return Ok(format!(
            "tailscale does not say the Mac's name in a tailnet. On this machine, open:\n\n  \
             http://127.0.0.1:{port}/pair/{token}\n"
        ));
    };
    let mut told = String::new();
    if let Err(why) = tailnet::serve(program, port, 443) {
        let _ = write!(told, "The tailnet does not serve the page yet: {why}\n\n");
    }
    if !magic_dns {
        told += "Turn MagicDNS on for the phone to find the Mac by its name: \
                 https://login.tailscale.com/admin/dns\n\n";
    }
    let _ = write!(
        told,
        "On a phone logged in to Tailscale as {login}, open:\n\n  https://{host}/\n\n\
         and add it to the Home Screen to be told of the chats there.\n\n\
         A phone of another login pairs once with:\n\n  https://{host}/pair/{token}\n",
        login = owner.login,
        host = owner.host,
    );
    Ok(told)
}

/// Has the tailnet hand the page on `port` to the phone, at the Mac's
/// name there, and the sketches' mocks on `mocks`, at that port of it,
/// and keeps `owner`, the Mac's login there, as Tailscale says it, every
/// few minutes, so that a phone of that login comes in.
pub async fn follow_tailnet(
    port: u16,
    mocks: Option<u16>,
    owner: Rc<RefCell<Option<tailnet::Owner>>>,
) {
    let mut serving: Vec<(u16, u16, bool, Option<String>)> = std::iter::once((port, 443))
        .chain(mocks.map(|mocks| (mocks, mocks)))
        .map(|(port, https)| (port, https, false, None))
        .collect();
    let mut told_dns = false;
    loop {
        let status = tokio::task::spawn_blocking(tailnet::status)
            .await
            .ok()
            .flatten();
        match status {
            Some((program, status)) => {
                if !status.magic_dns && !told_dns {
                    told_dns = true;
                    tracing::warn!(
                        "MagicDNS is off in the tailnet: a phone finds the Mac by its name only with it on"
                    );
                }
                // Again until it does, as once Serve is on in the
                // tailnet, saying why not only when that changes.
                for (port, https, served, why_not) in serving.iter_mut().filter(|one| !one.2) {
                    let (port, https) = (*port, *https);
                    match tokio::task::spawn_blocking(move || tailnet::serve(program, port, https))
                        .await
                    {
                        Ok(Ok(done)) => {
                            *served = true;
                            tracing::info!("{done}");
                        }
                        Ok(Err(why)) => {
                            if why_not.as_ref() != Some(&why) {
                                tracing::warn!("the tailnet does not serve the phone: {why}");
                            }
                            *why_not = Some(why);
                        }
                        Err(error) => tracing::error!("cannot ask tailscale: {error}"),
                    }
                }
                if *owner.borrow() != status.owner {
                    tracing::info!(login = ?status.owner.as_ref().map(|owner| &owner.login), "the phone of the Mac's login in the tailnet comes in");
                }
                *owner.borrow_mut() = status.owner;
            }
            None => {
                if owner.borrow_mut().take().is_some() {
                    tracing::info!("tailscale says nothing of the tailnet any more");
                }
            }
        }
        tokio::time::sleep(LOOKS_AT_THE_TAILNET).await;
    }
}

/// How often coxd asks Tailscale again of the Mac's login.
const LOOKS_AT_THE_TAILNET: Duration = Duration::from_mins(5);

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

/// Serves the page on `listener` to the phones of the Mac's login in the
/// tailnet, as `owner` has it, and those paired with `token`, telling the
/// daemon, on `internal`, of the phones that follow the chats, and, with
/// `pushes`, pushing to them what it says of the chats; the sketches'
/// mocks it frames being served on `mocks`, if they are (see
/// [`mocks`]).
pub async fn serve(
    listener: TcpListener,
    token: String,
    owner: Rc<RefCell<Option<tailnet::Owner>>>,
    (pushes, mocks): (Option<Pushes>, Option<u16>),
    internal: UnboundedSender<Internal>,
) {
    let pushes = pushes.map(|pushes| {
        let pushes = Rc::new(RefCell::new(pushes));
        let (to, notices) = unbounded_channel();
        if internal.send(Internal::Push { to }).is_ok() {
            tokio::task::spawn_local(push_all(Rc::clone(&pushes), notices));
        }
        pushes
    });
    let page = Rc::new(Page {
        token,
        owner,
        internal,
        last: Cell::new(0),
        pushes,
        mocks,
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
            let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                let page = Rc::clone(&page);
                async move {
                    let answer = if request.method() == Method::POST {
                        page.posted(request).await
                    } else {
                        page.answer(&request)
                    };
                    Ok::<_, Infallible>(answer)
                }
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
    /// The Mac in the tailnet, whose login's phones come in, unless
    /// Tailscale says nothing of it.
    owner: Rc<RefCell<Option<tailnet::Owner>>>,
    internal: UnboundedSender<Internal>,
    /// The number of the last phone that followed the chats.
    last: Cell<u64>,
    /// coxd's key and the phones it pushes to, unless it pushes nothing.
    pushes: Option<Rc<RefCell<Pushes>>>,
    /// The port the sketches' mocks are served on, if they are.
    mocks: Option<u16>,
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
        if path == "/push" {
            return match &self.pushes {
                Some(pushes) => answer(
                    StatusCode::OK,
                    "application/json",
                    serde_json::json!({ "key": pushes.borrow().public_key() }).to_string(),
                ),
                None => answer(StatusCode::NOT_FOUND, TEXT, "coxd pushes nothing"),
            };
        }
        if let Some(file) = path.strip_prefix("/media/") {
            return media(file, request.headers().get(header::RANGE));
        }
        if path == "/mocks" {
            return self.mocks(request.uri().query().unwrap_or_default());
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
            Some((at, kind, text)) => {
                let mut response = answer(StatusCode::OK, kind, *text);
                if *at == "/" {
                    self.frame_mocks(request.headers(), &mut response);
                }
                response
            }
            None => answer(StatusCode::NOT_FOUND, TEXT, "Not found"),
        }
    }

    /// Lets the page frame the sketches' mocks, served at the Mac's name
    /// the page came for, with `headers`, on their port.
    fn frame_mocks(&self, headers: &hyper::HeaderMap, response: &mut Response<Body>) {
        let (Some(mocks), Some(name)) = (self.mocks, mocks::host_name(headers)) else {
            return;
        };
        let policy =
            format!("{PAGE_POLICY}; frame-src https://{name}:{mocks} http://{name}:{mocks}");
        if let Ok(policy) = HeaderValue::from_str(&policy) {
            response
                .headers_mut()
                .insert(header::CONTENT_SECURITY_POLICY, policy);
        }
    }

    /// The mocks of the sketch whose folder `query`, `folder=<path>`,
    /// names, as JSON: the port they are served on, the sketch's name,
    /// and the paths of its pages.
    fn mocks(&self, query: &str) -> Response<Body> {
        let folder = url::form_urlencoded::parse(query.as_bytes())
            .find_map(|(name, value)| (name == "folder").then(|| value.into_owned()));
        let told = self.mocks.zip(folder).and_then(|(port, folder)| {
            let sketches = storage::path(Kept::Sketches)?;
            let (sketch, pages) = mocks::pages(&sketches, Path::new(&folder))?;
            Some(serde_json::json!({ "port": port, "sketch": sketch, "pages": pages }))
        });
        match told {
            Some(told) => answer(StatusCode::OK, "application/json", told.to_string()),
            None => answer(StatusCode::NOT_FOUND, TEXT, "No sketch's mocks"),
        }
    }

    /// Does what the page posted, as `request` says it, from the page
    /// itself: at `/act`, what the human did on it; at `/push`, where to
    /// push to the phone; and at `/push/end`, where to push no more.
    async fn posted(&self, request: Request<Incoming>) -> Response<Body> {
        if !self.paired(&request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        if !from_the_page(&request) {
            return answer(StatusCode::FORBIDDEN, TEXT, "Not from the page");
        }
        let path = request.uri().path().to_owned();
        let body = http_body_util::Limited::new(request.into_body(), MOST);
        let Ok(body) = http_body_util::BodyExt::collect(body).await else {
            return answer(StatusCode::PAYLOAD_TOO_LARGE, TEXT, "Too long");
        };
        let body = body.to_bytes();
        match path.as_str() {
            "/act" => self.act(&body),
            "/push" => self.subscribe(&body),
            "/push/end" => self.unsubscribe(&body),
            _ => answer(StatusCode::NOT_FOUND, TEXT, "Not found"),
        }
    }

    /// Pushes to the phone where `body` says, a `PushSubscription` as
    /// JSON, from now on.
    fn subscribe(&self, body: &[u8]) -> Response<Body> {
        let Some(pushes) = &self.pushes else {
            return answer(StatusCode::NOT_FOUND, TEXT, "coxd pushes nothing");
        };
        let Ok(subscription) = serde_json::from_slice::<Subscription>(body) else {
            return answer(StatusCode::BAD_REQUEST, TEXT, "Not understood");
        };
        let service = push::service(&subscription.endpoint);
        match pushes.borrow_mut().subscribe(subscription) {
            Ok(()) => {
                tracing::info!(service, "a phone asked to be told of the chats");
                answer(StatusCode::NO_CONTENT, TEXT, "")
            }
            Err(why) => answer(StatusCode::BAD_REQUEST, TEXT, why),
        }
    }

    /// Pushes no more where `body` says, `{"endpoint": ...}`.
    fn unsubscribe(&self, body: &[u8]) -> Response<Body> {
        #[derive(serde::Deserialize)]
        struct Ended {
            endpoint: String,
        }
        let Some(pushes) = &self.pushes else {
            return answer(StatusCode::NOT_FOUND, TEXT, "coxd pushes nothing");
        };
        let Ok(ended) = serde_json::from_slice::<Ended>(body) else {
            return answer(StatusCode::BAD_REQUEST, TEXT, "Not understood");
        };
        pushes.borrow_mut().unsubscribe(&ended.endpoint);
        answer(StatusCode::NO_CONTENT, TEXT, "")
    }

    /// Does what the human did on the page, as `body` says it, a
    /// [`ToDaemon`] as JSON, as the window would: only what the page
    /// does.
    fn act(&self, body: &[u8]) -> Response<Body> {
        let Ok(message) = serde_json::from_slice::<ToDaemon>(body) else {
            return answer(StatusCode::BAD_REQUEST, TEXT, "Not understood");
        };
        if !done_on_the_page(&message) {
            return answer(StatusCode::FORBIDDEN, TEXT, "Not done from the phone");
        }
        if self.internal.send(Internal::Act { message }).is_err() {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        answer(StatusCode::NO_CONTENT, TEXT, "")
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
        let mut response = answer(StatusCode::SEE_OTHER, TEXT, "Paired");
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

    /// Whether `request` is the human's (see [`paired`]).
    fn paired(&self, request: &Request<Incoming>) -> bool {
        paired(request.headers(), &self.owner, &self.token, 443)
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

/// How long what the page does may be, as a line sent, at most.
const MOST: usize = 256 * 1024;

/// The type of the answers in words.
const TEXT: &str = "text/plain; charset=utf-8";

/// Pushes each of the `notices` to the phones that asked, forgetting a
/// phone whose push service says it is gone.
async fn push_all(pushes: Rc<RefCell<Pushes>>, mut notices: UnboundedReceiver<Notice>) {
    while let Some(notice) = notices.recv().await {
        let ready = pushes.borrow().pushes(&notice);
        let chat = notice.key;
        for ready in ready {
            let pushes = Rc::clone(&pushes);
            tokio::task::spawn_local(async move {
                let endpoint = ready.endpoint.clone();
                let service = push::service(&endpoint);
                match tokio::task::spawn_blocking(move || push::send(&ready)).await {
                    Ok(Pushed::Done) => tracing::info!(service, chat, "pushed"),
                    Ok(Pushed::Gone) => {
                        tracing::info!(service, "a phone takes pushes no more");
                        pushes.borrow_mut().unsubscribe(&endpoint);
                    }
                    Ok(Pushed::Failed(why)) => tracing::error!(service, "cannot push: {why}"),
                    Err(error) => tracing::error!(service, "cannot push: {error}"),
                }
            });
        }
    }
}

/// Whether a request with `headers` is the human's: of the Mac's login
/// in the tailnet, for its port `https` there, as `owner` has it, or
/// holding the secret, `token`, in its cookie.
fn paired(
    headers: &hyper::HeaderMap,
    owner: &RefCell<Option<tailnet::Owner>>,
    token: &str,
    https: u16,
) -> bool {
    if let Some(owner) = &*owner.borrow()
        && tailnet::of_the_owner(headers, owner, https)
    {
        return true;
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|cookies| cookies.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|cookie| cookie.trim().strip_prefix(COOKIE)?.strip_prefix('='))
        .any(|given| same(given.as_bytes(), token.as_bytes()))
}

/// Whether `request` comes from the page itself, and not from another
/// page that the phone's browser, or the Mac's, has open: the page says
/// so in a header of its own, which another page could send only after
/// asking, as browsers have it, and which nothing here answers; and
/// where the browser says which page it comes from, it is this one.
fn from_the_page(request: &Request<Incoming>) -> bool {
    let headers = request.headers();
    let said = headers.get("x-cox").is_some_and(|said| said == "phone");
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|origin| origin.to_str().ok());
    let host = headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok());
    let same_origin = match origin {
        None => true,
        Some(origin) => origin
            .split_once("://")
            .is_some_and(|(_, at)| Some(at) == host),
    };
    said && same_origin
}

/// Whether the phone may do `message`: what its page does, and nothing
/// of the window's own, as what the stage carried out.
fn done_on_the_page(message: &ToDaemon) -> bool {
    match message {
        ToDaemon::Submit { attached, .. } => attached.is_empty(),
        ToDaemon::Choose { .. }
        | ToDaemon::Stop { .. }
        | ToDaemon::SetGoal { .. }
        | ToDaemon::AnswerProposal { .. }
        | ToDaemon::ChangeProposal { .. }
        | ToDaemon::AnswerKeep { .. }
        | ToDaemon::Merge { .. } => true,
        _ => false,
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
/// The picture or the video shown in a conversation kept as `file`, or
/// the part of it `range` asks for, as a phone's browser asks to play a
/// video.
fn media(file: &str, range: Option<&HeaderValue>) -> Response<Body> {
    let Some((path, kind)) = crate::media::kept(file) else {
        return answer(StatusCode::NOT_FOUND, TEXT, "Not found");
    };
    let Ok(bytes) = std::fs::read(path) else {
        return answer(StatusCode::NOT_FOUND, TEXT, "Not found");
    };
    let length = bytes.len();
    let asked = range
        .and_then(|range| range.to_str().ok())
        .and_then(|range| ranged(range, length));
    let mut response = match asked {
        Some((start, end)) => {
            let mut response = answer(
                StatusCode::PARTIAL_CONTENT,
                kind,
                Bytes::from(bytes).slice(start..=end),
            );
            if let Ok(range) = HeaderValue::from_str(&format!("bytes {start}-{end}/{length}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, range);
            }
            response
        }
        None => answer(StatusCode::OK, kind, bytes),
    };
    let headers = response.headers_mut();
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    // What a copy holds never changes: its name is what it holds.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response
}

/// The first and last byte a `Range` header asks for, of a file of
/// `length` bytes, if it asks for one range Cox serves.
fn ranged(range: &str, length: usize) -> Option<(usize, usize)> {
    let (start, end) = range.strip_prefix("bytes=")?.split_once('-')?;
    let last = length.checked_sub(1)?;
    let (start, end) = match (start.trim(), end.trim()) {
        ("", suffix) => {
            let suffix: usize = suffix.parse().ok()?;
            (length.checked_sub(suffix.min(length))?, last)
        }
        (start, "") => (start.parse().ok()?, last),
        (start, end) => (start.parse().ok()?, end.parse::<usize>().ok()?.min(last)),
    };
    (start <= end).then_some((start, end))
}

fn answer(status: StatusCode, kind: &'static str, text: impl Into<Bytes>) -> Response<Body> {
    let mut response = Response::new(Body::Whole(Some(text.into())));
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
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PAGE_POLICY),
    );
    response
}

/// What the page may do: what the agents write shows as text, so the
/// page runs only its own script, and no page frames it, a sketch's mock
/// included.
const PAGE_POLICY: &str = "default-src 'self'; img-src 'self' data:; object-src 'none'; \
    base-uri 'none'; frame-ancestors 'none'";

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

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            Self::Whole(bytes) => http_body::SizeHint::with_exact(
                bytes.as_ref().map_or(0, |bytes| bytes.len() as u64),
            ),
            Self::Events(_) => http_body::SizeHint::default(),
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
    fn serves_the_part_of_a_video_a_browser_asks_for() {
        assert_eq!(ranged("bytes=0-1", 10), Some((0, 1)));
        assert_eq!(ranged("bytes=4-", 10), Some((4, 9)));
        assert_eq!(ranged("bytes=-3", 10), Some((7, 9)));
        assert_eq!(ranged("bytes=2-99", 10), Some((2, 9)));
        assert_eq!(ranged("bytes=9-2", 10), None);
        assert_eq!(ranged("bytes=0-1,4-5", 10), None);
        assert_eq!(ranged("bytes=0-", 0), None);
    }

    #[test]
    fn reads_the_port_from_the_settings() {
        assert_eq!(port_in(br#"{"home": "~/cox"}"#), Ok(Some(PORT)));
        assert_eq!(port_in(br#"{"phone": false}"#), Ok(None));
        assert_eq!(port_in(br#"{"phone": true}"#), Ok(Some(PORT)));
        assert_eq!(port_in(br#"{"phone": {}}"#), Ok(Some(PORT)));
        assert_eq!(port_in(br#"{"phone": {"port": 7422}}"#), Ok(Some(7422)));
        assert!(port_in(br#"{"phone": {"port": 70000}}"#).is_err());
    }

    #[test]
    fn leaves_the_tailnet_to_coxd_unless_told_not() {
        assert!(tailscale_in(br#"{"home": "~/cox"}"#));
        assert!(tailscale_in(br#"{"phone": {"port": 7422}}"#));
        assert!(!tailscale_in(br#"{"phone": {"tailscale": false}}"#));
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
                let owner = Rc::new(RefCell::new(Some(tailnet::Owner {
                    host: "mac.tail1234.ts.net".to_owned(),
                    login: "me@github".to_owned(),
                })));
                tokio::task::spawn_local(serve(listener, "secret".to_owned(), owner, (None, None), internal));

                let unpaired = ask(port, "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", "").await;
                assert!(unpaired.starts_with("HTTP/1.1 401"), "{unpaired}");
                let through = |login: &str| {
                    format!(
                        "GET / HTTP/1.1\r\nHost: mac.tail1234.ts.net\r\n\
                         Tailscale-User-Login: {login}\r\nConnection: close\r\n\r\n"
                    )
                };
                let mine = ask(port, &through("me@github"), "").await;
                assert!(mine.starts_with("HTTP/1.1 200"), "{mine}");
                let shared = ask(port, &through("other@github"), "").await;
                assert!(shared.starts_with("HTTP/1.1 401"), "{shared}");
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

                let line = r#"{"Submit":{"key":7,"text":"go on"}}"#;
                let posted = |headers: &str| {
                    format!(
                        "POST /act HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\n{headers}\
                         Content-Type: application/json\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{line}",
                        line.len()
                    )
                };
                let elsewhere = ask(port, &posted("Origin: https://evil\r\nX-Cox: phone\r\n"), "").await;
                assert!(elsewhere.starts_with("HTTP/1.1 403"), "{elsewhere}");
                let unsaid = ask(port, &posted(""), "").await;
                assert!(unsaid.starts_with("HTTP/1.1 403"), "{unsaid}");
                let done = ask(port, &posted("Origin: http://x\r\nX-Cox: phone\r\n"), "").await;
                assert!(done.starts_with("HTTP/1.1 204"), "{done}");
                assert!(matches!(
                    heard.recv().await,
                    Some(Internal::Act { message: ToDaemon::Submit { key: 7, ref text, .. } }) if text == "go on"
                ));
            });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tells_the_phone_coxd_key_and_keeps_where_to_push() {
        let folder = std::env::temp_dir().join(format!("cox-phone-push-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let pushes = Pushes::open_at(&folder.join("push-key"), folder.join("pushes.json")).unwrap();
        let key = pushes.public_key();
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                "secret".to_owned(),
                Rc::new(RefCell::new(None)),
                (Some(pushes), None),
                internal,
            ));
            assert!(
                matches!(heard.recv().await, Some(Internal::Push { .. })),
                "the daemon is not told where to push"
            );

            let worker = ask(port, "GET /sw.js HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", "").await;
            assert!(worker.starts_with("HTTP/1.1 200"), "{worker}");
            let unpaired = ask(port, "GET /push HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n", "").await;
            assert!(unpaired.starts_with("HTTP/1.1 401"), "{unpaired}");
            let told = ask(
                port,
                "GET /push HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\nConnection: close\r\n\r\n",
                "",
            )
            .await;
            assert!(told.ends_with(&format!(r#"{{"key":"{key}"}}"#)), "{told}");

            let posting = |path: &str, headers: &str, body: &str| {
                format!(
                    "POST {path} HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\n{headers}\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let point = format!("BA{}", "A".repeat(85));
            let subscription = |endpoint: &str| {
                format!(
                    r#"{{"endpoint":"{endpoint}","expirationTime":null,"keys":{{"p256dh":"{point}","auth":"AAAAAAAAAAAAAAAAAAAAAA"}}}}"#
                )
            };
            let ours = subscription("https://web.push.apple.com/QAbc");
            let unsaid = ask(port, &posting("/push", "", &ours), "").await;
            assert!(unsaid.starts_with("HTTP/1.1 403"), "{unsaid}");
            let elsewhere = ask(
                port,
                &posting("/push", "X-Cox: phone\r\n", &subscription("https://127.0.0.1/a")),
                "",
            )
            .await;
            assert!(elsewhere.starts_with("HTTP/1.1 400"), "{elsewhere}");
            let done = ask(port, &posting("/push", "X-Cox: phone\r\n", &ours), "").await;
            assert!(done.starts_with("HTTP/1.1 204"), "{done}");
            let kept = std::fs::read_to_string(folder.join("pushes.json")).unwrap();
            assert!(kept.contains("https://web.push.apple.com/QAbc"), "{kept}");

            let ended = ask(
                port,
                &posting("/push/end", "X-Cox: phone\r\n", r#"{"endpoint":"https://web.push.apple.com/QAbc"}"#),
                "",
            )
            .await;
            assert!(ended.starts_with("HTTP/1.1 204"), "{ended}");
            let kept = std::fs::read_to_string(folder.join("pushes.json")).unwrap();
            assert!(!kept.contains("web.push.apple.com"), "{kept}");
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
        std::fs::remove_dir_all(&folder).unwrap();
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
    fn does_only_what_the_page_does() {
        let line = |attached: Vec<cox_core::entry::Attachment>| ToDaemon::Submit {
            key: 1,
            text: "go on".to_owned(),
            attached,
        };
        assert!(done_on_the_page(&line(Vec::new())));
        assert!(done_on_the_page(&ToDaemon::Choose { key: 1, index: 0 }));
        assert!(done_on_the_page(&ToDaemon::Stop { key: 1 }));
        assert!(done_on_the_page(&ToDaemon::Merge { key: 1 }));
        assert!(!done_on_the_page(&ToDaemon::Open { folder: None }));
        assert!(!done_on_the_page(&ToDaemon::Archive {
            key: 1,
            archived: true
        }));
        let picture = cox_core::entry::Attachment {
            label: "region".to_owned(),
            uri: "cox://region".to_owned(),
            text: String::new(),
            picture: Some("/etc/passwd".into()),
        };
        assert!(!done_on_the_page(&line(vec![picture])));
    }

    #[test]
    fn tells_the_secret_only_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
    }
}
