//! The page coxd serves the human's phone, for them to follow and answer
//! the chats away from the Mac: what goes on in them, which is its top,
//! the projects and chats, and a chat's conversation, as they change.
//!
//! An installed coxd serves it unless `settings.json` turns it off, as
//! `{"phone": false}`; any other, as a test's or `cargo xtask run`'s,
//! only when it turns it on, so that a Cox being built takes neither the
//! installed one's port nor its place in the tailnet. It serves it on
//! [`PORT`], and only on the Mac's own `127.0.0.1`:
//! the phone reaches it through `tailscale serve`, which hands the page
//! to the human's tailnet over HTTPS, and which coxd sets up as it
//! starts (see [`tailnet`]). Since any page in any browser on the Mac,
//! and anything else there, can reach `127.0.0.1` too, every answer but
//! the pairing is for the human alone: a phone that holds, in a cookie,
//! the secret coxd keeps in `state/phone-token`, which it gets by
//! opening, once, `/pair/<secret>`, until the human unpairs every phone,
//! as coxd makes the secret again (see [`Phones`]).
//!
//! The page is the files in `phone/`, built into Cox. It follows the
//! chats on `/events`, a stream of server-sent events, each a
//! [`FromDaemon`] as JSON: the [`FromDaemon::Listing`] of the projects
//! and chats whenever it changes, the [`FromDaemon::Patch`]es of the
//! chat named by `?kept=<number>`, all of it first, and the
//! [`FromDaemon::News`] of the chats that come to wait for the human;
//! but for what the conversation's tool calls show, which it asks for on
//! `/content` as the human opens one (see [`for_the_phone`]), and, of a
//! long conversation, for the entries before its last [`TAIL`], which it
//! asks for on `/entries` as the human reads back to them (see
//! [`opened`]), and which a page that holds them already, as it follows
//! the chat again, names on `/events` by the first it holds, `&from=<n>`,
//! to be told from there.
//! What the human does on it, as sending a line or answering what the
//! agent asks, it posts to `/act?kept=<number>`, a [`ToDaemon`] as JSON,
//! as the window says it, which coxd does as the window's (see
//! [`done_on_the_page`] for which).
//!
//! The top of the page is what goes on in every project, as the window's
//! Activity page of the home tells it, beside the list of chats: the page
//! reads it on `/activity`, as JSON, which coxd puts together from the
//! chats as it lists them itself, with the code the window's page is told
//! by (see [`crate::activity`]), so that it is there while no window is
//! open; and a chat's story, what it did when, on
//! `/activity/timeline?kept=<number>`, as the human opens it. It posts to
//! `/activity/resume?kept=<number>` to have a chat whose turn was cut off
//! go on, and to `/activity/fix?kept=<number>`, with the pull request, as
//! `{"repo": ..., "number": ...}`, to have the chat that opened it fix its
//! CI, each a line coxd sends the chat as the human's, as the window's
//! page has it sent, and refused where that page's is (see
//! [`crate::activity::resume`] and [`crate::activity::fix`]).
//!
//! The page names a chat by the number it is kept by, `kept`, never by
//! its key: its address, and the notices pushed to it, outlast coxd,
//! which numbers the keys anew each time it starts, and a key the page
//! held could name another chat then.
//!
//! The page lists, too, the pages a chat has for the human to try, on
//! `/pages`: the mocks of the sketch it works in, and the pages of its
//! stage served on the Mac's own address, as the web app the agent makes
//! is, and shows one in a frame over the chat, as the relay passes it on
//! at the same port of the Mac's name (see [`relay`]).
//!
//! The page shows, too, a map of the project the agent showed on the
//! stage, where it showed it, in a frame over the chat: the window's own
//! Map page, whose files coxd serves at `/map/` (see [`crate::map::PAGE`]),
//! with the window's colors beside them, and which reads the map of the
//! project where the chat its query names works, `?kept=<number>`, beside
//! itself, at `/map/api/map`, as the window's stage tells it at
//! `cox://api/map` (see [`map_asked`]). The human opens the same page
//! whenever they will, at the project's first map: from the head of a
//! chat of a project, and from a project's shelf on the list of chats,
//! which asks by the project, `?project=<number>`, for the map of the
//! project's own folder, not of a worktree one of its chats works in.
//!
//! What coxd answers the phone as text, as the page's files and the map,
//! megabytes of JSON, it packs with gzip for a browser that reads it so
//! (see [`packed`]): over the tailnet, the phone waits for each byte.
//!
//! The page asks, too, to be told of the chats that need the human while
//! they are away from the Mac, with Web Push (see [`super::push`]): it
//! reads coxd's key on `/push`, and posts where to push to it there, or
//! to `/push/end` to be told no more, its service worker, `/sw.js`,
//! showing what is pushed.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::io::Read as _;
use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Context as _;
use bytes::Bytes;
use cox_core::store::{ChatId, ProjectId};
use http_body::Frame;
use hyper::body::Incoming;
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub mod mocks;
pub mod relay;
pub(crate) mod tailnet;

use super::push::{self, Notice, Pushed, Pushes, Subscription};
use super::wire::{FromDaemon, ToDaemon};
use super::{Internal, MapOf, Sketching};
use crate::activity;
use crate::chat::conversation::{Content, Entry};
use crate::storage::{self, Kept};
use crate::theme;

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
    <link rel=\"stylesheet\" href=\"/theme.css\"><link rel=\"stylesheet\" href=\"/app.css\">\
    <main class=\"pair\"><h1>Cox</h1>\
    <p>Cox does not know this phone. On the Mac, scan the pairing code in Cox's settings, or \
    run <code>cox --phone-link</code> and open the pairing link it prints here, or paste \
    it:</p><form action=\"/pair\">\
    <input name=\"link\" autocomplete=\"off\" placeholder=\"https://…/pair/…\">\
    <button>Pair</button></form></main>";

/// The files a phone gets before it is paired: the page's look, for the
/// page that pairs it, and what puts the page on its home screen, and the
/// service worker, which the browser asks for without the cookie.
const PUBLIC: &[&str] = &[
    "/app.css",
    "/theme.css",
    "/manifest.webmanifest",
    "/icon.svg",
    "/sw.js",
];

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
/// off, as `{"phone": false}`, or, for a coxd that is not installed, when
/// it does not turn it on.
#[must_use]
pub fn port() -> Option<u16> {
    let unsaid = storage::installed().then_some(PORT);
    let Some(file) = storage::path(Kept::Settings) else {
        return unsaid;
    };
    let Ok(bytes) = std::fs::read(&file) else {
        return unsaid;
    };
    port_in(&bytes, unsaid)
        .inspect_err(|why| tracing::error!("\"phone\" in {} cannot be read: {why}", file.display()))
        .unwrap_or(unsaid)
}

/// The port the page is served on, or why there is none.
fn served_port() -> anyhow::Result<u16> {
    port().with_context(|| {
        let turned_off = storage::path(Kept::Settings)
            .and_then(|file| std::fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|value| value.get("phone") == Some(&serde_json::Value::Bool(false)));
        if turned_off || storage::installed() {
            "settings.json turns the phone's page off, with {\"phone\": false}"
        } else {
            "a Cox that is not installed serves the phone's page only when settings.json \
             turns it on, as with {\"phone\": true}"
        }
    })
}

/// The port the settings `bytes` say the page is served on: `unsaid`
/// when they say nothing of the phone, else [`PORT`], unless they turn it
/// off, or name another, as `{"phone": {"port": 7422}}`.
fn port_in(bytes: &[u8], unsaid: Option<u16>) -> Result<Option<u16>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    match value.get("phone") {
        None | Some(serde_json::Value::Null) => Ok(unsaid),
        Some(serde_json::Value::Bool(true)) => Ok(Some(PORT)),
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

/// Whether coxd has the tailnet hand the page to the phone: an installed
/// one unless `settings.json` says not, as `{"phone": {"tailscale":
/// false}}`, and any other only when it says so, as `{"phone":
/// {"tailscale": true}}`.
#[must_use]
pub fn tailscale() -> bool {
    let unsaid = storage::installed();
    storage::path(Kept::Settings)
        .and_then(|file| std::fs::read(file).ok())
        .map_or(unsaid, |bytes| tailscale_in(&bytes, unsaid))
}

/// Whether the settings `bytes` leave the tailnet to coxd, or `unsaid`
/// when they do not say.
fn tailscale_in(bytes: &[u8], unsaid: bool) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.pointer("/phone/tailscale")?.as_bool())
        .unwrap_or(unsaid)
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
    new_token_at(file)
}

/// A new secret for a phone to pair with, kept in place of the one
/// before, so that no phone paired with that one is let in any more,
/// once coxd takes it up (see [`Phones::unpaired`]).
///
/// # Errors
///
/// When there is no place to keep it, or it cannot be made or kept.
pub fn new_token() -> anyhow::Result<String> {
    let file =
        storage::path(Kept::PhoneToken).context("there is no place for the phone's secret")?;
    new_token_at(&file)
}

/// A new secret, kept in `file` in place of the one before.
fn new_token_at(file: &Path) -> anyhow::Result<String> {
    let mut random = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut random))
        .context("cannot make a secret")?;
    let token = random.iter().fold(String::new(), |mut token, byte| {
        let _ = std::fmt::Write::write_fmt(&mut token, format_args!("{byte:02x}"));
        token
    });
    storage::keep_secret(file, token.as_bytes())
        .with_context(|| format!("cannot keep {}", file.display()))?;
    Ok(token)
}

/// Where a phone opens the page, and the link that pairs it, as the
/// settings show them: through the tailnet when `tailscale` tells the
/// Mac's name there, or else on the Mac itself. It has no `Debug`, so
/// that the secret is not logged by mistake.
#[derive(Clone, PartialEq, Eq)]
pub struct Pairing {
    /// The page, as a paired phone opens it.
    pub page: String,
    /// The link that pairs a phone, with the secret in it.
    pub pair: String,
    /// The Mac's login in the tailnet; none when `tailscale` does not say
    /// the Mac's name there.
    pub login: Option<String>,
    /// Whether the tailnet's devices find each other by name, which the
    /// phone needs to find the Mac by its name.
    pub magic_dns: bool,
}

/// Where a phone opens the page, and the link that pairs it, without
/// having the tailnet serve it, which coxd does as it starts.
///
/// # Errors
///
/// When the settings turn the page off, or the secret cannot be had.
pub fn pairing() -> anyhow::Result<Pairing> {
    let port = served_port()?;
    let token = token()?;
    let status = tailscale().then(tailnet::status).flatten();
    Ok(match status {
        Some((
            _,
            tailnet::Status {
                owner: Some(owner),
                magic_dns,
            },
        )) => Pairing {
            page: format!("https://{}/", owner.host),
            pair: format!("https://{}/pair/{token}", owner.host),
            login: Some(owner.login),
            magic_dns,
        },
        _ => Pairing {
            page: format!("http://127.0.0.1:{port}/"),
            pair: format!("http://127.0.0.1:{port}/pair/{token}"),
            login: None,
            magic_dns: false,
        },
    })
}

/// What `cox --phone-link` prints: the link that pairs a phone, and
/// where it opens the page then, through the tailnet when `tailscale`
/// tells the Mac's name there, with what the tailnet lacks for it.
///
/// # Errors
///
/// When the settings turn the page off, or the secret cannot be had.
pub fn link() -> anyhow::Result<String> {
    use std::fmt::Write as _;

    let port = served_port()?;
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
        "On a phone logged in to Tailscale as {login}, pair once with:\n\n  \
         https://{host}/pair/{token}\n\n\
         and add the page it opens, https://{host}/, to the Home Screen to be told of \
         the chats there.\n",
        login = owner.login,
        host = owner.host,
    );
    Ok(told)
}

/// Has the tailnet hand the page on `port` to the phone, at the Mac's
/// name there, and each port the relay passes on to, as `ports` wants
/// them, at that port of it, to the relay, asking Tailscale again every
/// few minutes.
pub async fn follow_tailnet(port: u16, ports: Option<Rc<relay::Ports>>) {
    let mut page = (false, None);
    // What the tailnet hands to the relay, once read, as an earlier coxd
    // may have left it, and why it does not hand each port it does not.
    let mut relayed: Option<BTreeSet<u16>> = None;
    let mut why_not: BTreeMap<u16, String> = BTreeMap::new();
    let mut changes = ports.as_ref().map(|ports| ports.changes());
    let mut told_dns = false;
    loop {
        let status = tokio::task::spawn_blocking(tailnet::status)
            .await
            .ok()
            .flatten();
        if let Some((program, status)) = status {
            if !status.magic_dns && !told_dns {
                told_dns = true;
                tracing::warn!(
                    "MagicDNS is off in the tailnet: a phone finds the Mac by its name only with it on"
                );
            }
            // Again until it does, as once Serve is on in the
            // tailnet, saying why not only when that changes.
            if !page.0 {
                match serve_in_the_tailnet(program, port, 443).await {
                    Ok(done) => {
                        page.0 = true;
                        tracing::info!("{done}");
                    }
                    Err(why) => {
                        if page.1.as_ref() != Some(&why) {
                            tracing::warn!("the tailnet does not serve the phone: {why}");
                        }
                        page.1 = Some(why);
                    }
                }
            }
            if let Some(ports) = &ports {
                let relay = ports.relay();
                if relayed.is_none() {
                    relayed = tokio::task::spawn_blocking(move || tailnet::relayed(program, relay))
                        .await
                        .ok();
                }
                if let Some(relayed) = &mut relayed {
                    relay_in_the_tailnet(program, ports, relayed, &mut why_not).await;
                }
            }
        }
        match &mut changes {
            Some(changes) => {
                tokio::select! {
                    () = tokio::time::sleep(LOOKS_AT_THE_TAILNET) => {}
                    told = changes.changed() => if told.is_err() {
                        tokio::time::sleep(LOOKS_AT_THE_TAILNET).await;
                    },
                }
            }
            None => tokio::time::sleep(LOOKS_AT_THE_TAILNET).await,
        }
    }
}

/// Has `tailscale serve`, through `program`, hand what coxd serves on
/// `port` to the tailnet, at its port `https`.
async fn serve_in_the_tailnet(
    program: &'static str,
    port: u16,
    https: u16,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || tailnet::serve(program, port, https))
        .await
        .map_err(|error| format!("cannot ask tailscale: {error}"))?
}

/// Has the tailnet, through `program`, hand to the relay each port
/// `ports` wants, and no other, where `relayed` are those it does: saying
/// why it does not hand a port only when that changes, in `why_not`.
async fn relay_in_the_tailnet(
    program: &'static str,
    ports: &relay::Ports,
    relayed: &mut BTreeSet<u16>,
    why_not: &mut BTreeMap<u16, String>,
) {
    let wanted = ports.wanted();
    for &port in wanted.difference(&relayed.clone()) {
        match serve_in_the_tailnet(program, ports.relay(), port).await {
            Ok(done) => {
                relayed.insert(port);
                why_not.remove(&port);
                tracing::info!(port, "{done}, to the relay");
            }
            Err(why) => {
                if why_not.get(&port) != Some(&why) {
                    tracing::warn!(
                        port,
                        "the tailnet does not hand the port to the phone: {why}"
                    );
                }
                why_not.insert(port, why);
            }
        }
    }
    for &port in relayed.clone().difference(&wanted) {
        match tokio::task::spawn_blocking(move || tailnet::unserve(program, port)).await {
            Ok(Ok(())) => {
                relayed.remove(&port);
                tracing::info!(port, "the tailnet hands the port to the phone no more");
            }
            Ok(Err(why)) => {
                tracing::warn!(port, "the tailnet still hands the port to the phone: {why}");
            }
            Err(error) => tracing::error!("cannot ask tailscale: {error}"),
        }
    }
    why_not.retain(|port, _| wanted.contains(port));
    ports.served(relayed.clone());
}

/// How often coxd asks Tailscale again of the tailnet.
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

/// The human's phones, as coxd holds them: the secret they are paired
/// with, which the page and the relay let them in by, and where coxd
/// pushes to them, unless it pushes nothing. One secret for them all, so
/// that the human unpairs them all at once.
pub struct Phones {
    /// The secret, as the cookie of a paired phone holds it.
    secret: RefCell<String>,
    /// coxd's key and the phones it pushes to, unless it pushes nothing.
    pushes: Option<Rc<RefCell<Pushes>>>,
}

impl Phones {
    /// The phones paired with `secret`, pushed to as `pushes` says.
    #[must_use]
    pub fn new(secret: String, pushes: Option<Pushes>) -> Self {
        Self {
            secret: RefCell::new(secret),
            pushes: pushes.map(|pushes| Rc::new(RefCell::new(pushes))),
        }
    }

    /// Lets in only the phones paired with `secret` from now on, the
    /// human having unpaired every phone, and pushes to none of them
    /// until it asks again.
    pub fn unpaired(&self, secret: String) {
        *self.secret.borrow_mut() = secret;
        if let Some(pushes) = &self.pushes {
            pushes.borrow_mut().forget_all();
        }
    }

    /// Whether a request with `headers` is the human's: holding the
    /// secret in its cookie.
    fn paired(&self, headers: &hyper::HeaderMap) -> bool {
        let secret = self.secret.borrow();
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|cookies| cookies.to_str().ok())
            .flat_map(|cookies| cookies.split(';'))
            .filter_map(|cookie| cookie.trim().strip_prefix(COOKIE)?.strip_prefix('='))
            .any(|given| same(given.as_bytes(), secret.as_bytes()))
    }
}

/// Serves the page on `listener` to the `phones`, telling the daemon, on
/// `internal`, of the phones that follow the chats, and pushing to them
/// what it says of the chats; the pages it frames being passed on by the
/// relay, as `relayed` has it, if they are (see [`relay`]).
pub async fn serve(
    listener: TcpListener,
    phones: Rc<Phones>,
    relayed: Option<Relayed>,
    internal: UnboundedSender<Internal>,
) {
    if let Some(pushes) = &phones.pushes {
        let (to, notices) = unbounded_channel();
        if internal.send(Internal::Push { to }).is_ok() {
            tokio::task::spawn_local(push_all(Rc::clone(pushes), notices));
        }
    }
    let page = Rc::new(Page {
        phones,
        internal,
        last: Cell::new(0),
        relayed,
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
                    let packs = accepts_gzip(request.headers());
                    let answer = if request.method() == Method::POST {
                        page.posted(request).await
                    } else if request.uri().path() == "/content" {
                        page.content(&request).await
                    } else if request.uri().path() == "/entries" {
                        page.entries(&request).await
                    } else if request.uri().path() == "/changes" {
                        page.changes(&request).await
                    } else if request.uri().path() == "/pages" {
                        page.pages(&request).await
                    } else if request.uri().path() == "/sketch" {
                        page.sketch(&request).await
                    } else if request.uri().path() == "/diagram" {
                        page.diagram(&request).await
                    } else if map_asked(request.uri().path()).is_some() {
                        page.map(&request).await
                    } else if matches!(request.uri().path(), "/activity" | "/activity/timeline") {
                        page.activity(&request).await
                    } else if request.uri().path() == "/events" {
                        page.events(&request).await
                    } else {
                        page.answer(&request)
                    };
                    let answer = if packs { packed(answer).await } else { answer };
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
    phones: Rc<Phones>,
    internal: UnboundedSender<Internal>,
    /// The number of the last phone that followed the chats.
    last: Cell<u64>,
    /// What the relay passes on to the phone, if it does.
    relayed: Option<Relayed>,
}

/// What the relay passes on to the phone: the sketches' mocks, served on
/// a port of the Mac's own, if they are, and the ports it passes on to.
pub struct Relayed {
    pub mocks: Option<u16>,
    pub ports: Rc<relay::Ports>,
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
            return match &self.phones.pushes {
                Some(pushes) => answer(
                    StatusCode::OK,
                    "application/json",
                    serde_json::json!({ "key": pushes.borrow().public_key() }).to_string(),
                ),
                None => answer(StatusCode::NOT_FOUND, TEXT, "coxd pushes nothing"),
            };
        }
        if path == "/theme.css" {
            return answer(StatusCode::OK, "text/css; charset=utf-8", theme::css());
        }
        if let Some(file) = path.strip_prefix("/media/") {
            return media(file, request.headers().get(header::RANGE));
        }
        if let Some(project) = path.strip_prefix("/project-icon/") {
            return project_icon(project);
        }
        if let Some(file) = path.strip_prefix("/map/") {
            return map_page(file);
        }
        match FILES.iter().find(|(at, _, _)| *at == path) {
            Some((at, kind, text)) => {
                let mut response = answer(StatusCode::OK, kind, *text);
                if *at == "/" {
                    self.frame_relayed(request.headers(), &mut response);
                }
                response
            }
            None => answer(StatusCode::NOT_FOUND, TEXT, "Not found"),
        }
    }

    /// Lets the page frame what the relay passes on, at the Mac's name
    /// the page came for, with `headers`, on any port of it.
    fn frame_relayed(&self, headers: &hyper::HeaderMap, response: &mut Response<Body>) {
        let (Some(_), Some(name)) = (&self.relayed, relay::host_name(headers)) else {
            return;
        };
        let policy = format!("{PAGE_POLICY}; frame-src https://{name}:* http://{name}:*");
        if let Ok(policy) = HeaderValue::from_str(&policy) {
            response
                .headers_mut()
                .insert(header::CONTENT_SECURITY_POLICY, policy);
        }
    }

    /// The pages the chat `request` names, `?kept=<number>`, has for the
    /// human to try, as JSON, each its name, the port of the Mac's own
    /// address it is served on, which the relay passes on at the same
    /// port of the Mac's name, and its path there: the mocks of the
    /// sketch it works in, if it works in one, and the pages of its
    /// stage served on the Mac's own address, as a web app being made
    /// is. Told once the tailnet hands their ports to the relay, or after
    /// a while, if it does not.
    async fn pages(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        let key = match self.chat_named(request).await {
            Ok(key) => key,
            Err((status, why)) => return answer(status, TEXT, why),
        };
        let Some(relayed) = &self.relayed else {
            return answer(
                StatusCode::OK,
                "application/json",
                serde_json::json!({ "pages": [] }).to_string(),
            );
        };
        let (asked, staged) = tokio::sync::oneshot::channel();
        if self
            .internal
            .send(Internal::Pages { key, answer: asked })
            .is_err()
        {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        let Ok(Some((folder, staged))) = staged.await else {
            return answer(StatusCode::NOT_FOUND, TEXT, "No such chat");
        };
        let mut pages = Vec::new();
        let mut ports = BTreeSet::new();
        if let Some(mocks) = relayed.mocks
            && let Some(sketches) = storage::path(Kept::Sketches)
            && let Some((sketch, found)) = mocks::pages(&sketches, &folder)
        {
            ports.insert(mocks);
            for page in found {
                let path = path_of(std::iter::once(sketch.as_str()).chain(page.split('/')));
                pages.push(serde_json::json!({ "name": page, "port": mocks, "path": path }));
            }
        }
        let mut stage = BTreeSet::new();
        for (address, title) in staged {
            let Some((port, path)) = relay::on_the_mac(&address) else {
                continue;
            };
            stage.insert(port);
            let name = if title.is_empty() { address } else { title };
            pages.push(serde_json::json!({ "name": name, "port": port, "path": path }));
        }
        relayed
            .ports
            .want(relay::Wanting::Stage(folder), stage.clone());
        ports.extend(stage);
        ports.retain(|port| relayed.ports.passes(*port));
        pages.retain(|page| {
            page["port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .is_some_and(|port| ports.contains(&port))
        });
        relayed.ports.until_served(&ports).await;
        answer(
            StatusCode::OK,
            "application/json",
            serde_json::json!({ "pages": pages }).to_string(),
        )
    }

    /// What the tool call, or the permission request, `request` names
    /// shows, `?kept=<number>&at=<place>`, as JSON, `{"content": [...]}`,
    /// which the stream of events leaves out (see [`for_the_phone`]).
    async fn content(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        let Some(at) = place_asked(request) else {
            return answer(StatusCode::BAD_REQUEST, TEXT, "No place");
        };
        let key = match self.chat_named(request).await {
            Ok(key) => key,
            Err((status, why)) => return answer(status, TEXT, why),
        };
        let (asked, content) = tokio::sync::oneshot::channel();
        if self
            .internal
            .send(Internal::Content {
                key,
                at,
                answer: asked,
            })
            .is_err()
        {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        let Ok(Some(content)) = content.await else {
            return answer(StatusCode::NOT_FOUND, TEXT, "Nothing there");
        };
        answer(
            StatusCode::OK,
            "application/json",
            serde_json::json!({ "content": content }).to_string(),
        )
    }

    /// The entries before `before=<n>` of the conversation of the chat
    /// `request` names, `?kept=<number>`, as JSON, the last [`TAIL`] of them
    /// at most, as the phone reads back through a long one: where they
    /// start, `from`, and each with its place, as [`for_the_phone`] tells
    /// them.
    async fn entries(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        let Some(before) = number_asked(request, "before") else {
            return answer(StatusCode::BAD_REQUEST, TEXT, "No place");
        };
        let key = match self.chat_named(request).await {
            Ok(key) => key,
            Err((status, why)) => return answer(status, TEXT, why),
        };
        let from = before.saturating_sub(TAIL);
        let (asked, entries) = tokio::sync::oneshot::channel();
        if self
            .internal
            .send(Internal::Entries {
                key,
                from,
                to: before,
                answer: asked,
            })
            .is_err()
        {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        let Ok(Some(entries)) = entries.await else {
            return answer(StatusCode::NOT_FOUND, TEXT, "No such chat");
        };
        let told = entries
            .into_iter()
            .zip(from..)
            .map(|(entry, at)| Ok(serde_json::json!([at, told_entry(entry)?])))
            .collect::<serde_json::Result<Vec<_>>>();
        match told {
            Ok(told) => answer(
                StatusCode::OK,
                "application/json",
                serde_json::json!({ "from": from, "entries": told }).to_string(),
            ),
            Err(error) => answer(
                StatusCode::INTERNAL_SERVER_ERROR,
                TEXT,
                format!("The entries cannot be told: {error}"),
            ),
        }
    }

    /// What changed where the chat `request` names works, `?kept=<number>`,
    /// as JSON, as the window's "Changes" tab shows it (see
    /// [`told_changes`]).
    async fn changes(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        let key = match self.chat_named(request).await {
            Ok(key) => key,
            Err((status, why)) => return answer(status, TEXT, why),
        };
        let (asked, folder) = tokio::sync::oneshot::channel();
        if self
            .internal
            .send(Internal::Folder { key, answer: asked })
            .is_err()
        {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        let Ok(Some(folder)) = folder.await else {
            return answer(StatusCode::NOT_FOUND, TEXT, "No such chat");
        };
        match tokio::task::spawn_blocking(move || cox_git::changes::read(&folder)).await {
            Ok(Ok(diff)) => answer(
                StatusCode::OK,
                "application/json",
                told_changes(&diff).to_string(),
            ),
            Ok(Err(error)) => answer(
                StatusCode::OK,
                "application/json",
                serde_json::json!({ "error": error }).to_string(),
            ),
            Err(error) => answer(StatusCode::INTERNAL_SERVER_ERROR, TEXT, error.to_string()),
        }
    }

    /// What the human may do of sketches in the chat `request` names,
    /// `?kept=<number>`, as JSON, as the window's menu offers it (see
    /// [`told_sketching`]).
    async fn sketch(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        let key = match self.chat_named(request).await {
            Ok(key) => key,
            Err((status, why)) => return answer(status, TEXT, why),
        };
        let (asked, sketching) = tokio::sync::oneshot::channel();
        if self
            .internal
            .send(Internal::Sketching { key, answer: asked })
            .is_err()
        {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        let Ok(sketching) = sketching.await else {
            return answer(StatusCode::NOT_FOUND, TEXT, "No such chat");
        };
        // Whether the project is in a Git repository asks Git, and the
        // workflow's endings are read from its folder.
        let told = tokio::task::spawn_blocking(move || {
            told_sketching(sketching.as_ref(), &crate::workflow::sketch().endings)
        });
        match told.await {
            Ok(told) => answer(StatusCode::OK, "application/json", told.to_string()),
            Err(error) => answer(StatusCode::INTERNAL_SERVER_ERROR, TEXT, error.to_string()),
        }
    }

    /// The Mermaid diagram `request` holds, `?source=<source>`, as an SVG
    /// in the window's colors, as the window draws it (see
    /// [`crate::diagram`]).
    async fn diagram(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        let query = request.uri().query().unwrap_or_default();
        let Some(source) = url::form_urlencoded::parse(query.as_bytes())
            .find_map(|(name, value)| (name == "source").then(|| value.into_owned()))
        else {
            return answer(StatusCode::BAD_REQUEST, TEXT, "No diagram");
        };
        // Drawing it reads the system's fonts the first time.
        match tokio::task::spawn_blocking(move || crate::diagram::svg(&source)).await {
            Ok(Ok(svg)) => {
                let mut response = answer(StatusCode::OK, "image/svg+xml", svg);
                // The same source draws the same diagram.
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("private, max-age=86400"),
                );
                response
            }
            Ok(Err(error)) => answer(StatusCode::UNPROCESSABLE_ENTITY, TEXT, format!("{error:#}")),
            Err(error) => answer(StatusCode::INTERNAL_SERVER_ERROR, TEXT, error.to_string()),
        }
    }

    /// The map of the project `request` names, `?project=<number>`, or
    /// of where the chat it names works, `?kept=<number>`, as JSON, as
    /// the window's Map page is told it:
    /// the project's first, or the one the path names after `api/map/`,
    /// drawn again after a failed index when the path says `again/`
    /// before the name (see [`map_asked`]); without the map when the
    /// query gives the tag of the map the page has, `seen=<tag>`, and it
    /// is the same (see [`crate::map::since`]).
    async fn map(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        let Some((name, again)) = map_asked(request.uri().path()) else {
            return answer(StatusCode::NOT_FOUND, TEXT, "Not found");
        };
        let of = match project_asked(request) {
            Some(project) => MapOf::Project(project),
            None => match self.chat_named(request).await {
                Ok(key) => MapOf::Chat(key),
                Err((status, why)) => return answer(status, TEXT, why),
            },
        };
        let (asked, drawn) = tokio::sync::oneshot::channel();
        if self
            .internal
            .send(Internal::Map {
                of,
                name,
                again,
                answer: asked,
            })
            .is_err()
        {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        let Ok(Some(told)) = drawn.await else {
            return answer(StatusCode::NOT_FOUND, TEXT, "No such chat or project");
        };
        let seen = text_asked(request, "seen");
        let told = match crate::map::since(&told, seen.as_deref()) {
            std::borrow::Cow::Borrowed(_) => told,
            std::borrow::Cow::Owned(rest) => rest,
        };
        answer(StatusCode::OK, "application/json", told)
    }

    /// How the chats stand, as coxd lists them, and what the agent of
    /// each at work, or asking, does now and asks, by its key, or nothing
    /// while coxd stops.
    async fn standing(&self) -> Option<(activity::Told, Vec<(u64, crate::commands::Now)>)> {
        let (asked, standing) = tokio::sync::oneshot::channel();
        self.internal
            .send(Internal::Activity { answer: asked })
            .ok()?;
        standing.await.ok()
    }

    /// What goes on in every project, as JSON, as the window's Activity
    /// page of the home is told it (see [`activity::told`]), from the
    /// chats as coxd lists them, whether a window is open or not; or, at
    /// `/activity/timeline`, the story of the chat `request` names,
    /// `?kept=<number>` (see [`activity::timeline`]), null when coxd
    /// keeps nothing of it.
    async fn activity(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        let of = if request.uri().path() == "/activity/timeline" {
            match self.chat_named(request).await {
                Ok(key) => Some(key),
                Err((status, why)) => return answer(status, TEXT, why),
            }
        } else {
            None
        };
        let Some((told, now)) = self.standing().await else {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        };
        let read = move || match of {
            None => activity::told(&told, &told.home, &|key| {
                now.iter()
                    .find(|(chat, _)| *chat == key)
                    .map(|(_, now)| now.clone())
            }),
            Some(key) => told
                .chats
                .iter()
                .find(|chat| chat.key == key)
                .and_then(|chat| chat.kept)
                .map_or(serde_json::Value::Null, |chat| {
                    activity::timeline(&told, &activity::Of { chat })
                }),
        };
        match blocking(read).await {
            Ok(told) => answer(StatusCode::OK, "application/json", told.to_string()),
            Err(error) => answer(StatusCode::INTERNAL_SERVER_ERROR, TEXT, error.to_string()),
        }
    }

    /// Has the chat kept as `kept` go on from where its turn was cut off,
    /// or, with a pull request in `body`, as `{"repo": ..., "number":
    /// ...}`, fix the CI of that pull request, which its agent opened: by
    /// the line the window's Activity page would have it sent, as the
    /// human's, unless that page would send none (see
    /// [`activity::resume`] and [`activity::fix`]).
    async fn say(&self, kept: Option<i64>, body: Option<&[u8]>) -> Response<Body> {
        #[derive(serde::Deserialize)]
        struct Pull {
            repo: String,
            number: u64,
        }
        let pull = match body.map(serde_json::from_slice::<Pull>) {
            None => None,
            Some(Ok(pull)) => Some(pull),
            Some(Err(_)) => return answer(StatusCode::BAD_REQUEST, TEXT, "Not understood"),
        };
        let Some(kept) = kept else {
            return answer(StatusCode::BAD_REQUEST, TEXT, "No chat");
        };
        let Some(key) = self.key_of(kept).await else {
            return answer(StatusCode::NOT_FOUND, TEXT, "No such chat");
        };
        let Some((told, _)) = self.standing().await else {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        };
        let (say, refused) = match pull {
            Some(Pull { repo, number }) => (
                activity::fix(&told, &activity::Fix { key, repo, number }),
                "Its CI did not fail, or another chat opened it",
            ),
            None => (
                activity::resume(&told, &activity::Resume { key }),
                "Its turn was not cut off, or it is at work, or asks you something",
            ),
        };
        let Some(say) = say else {
            return answer(StatusCode::CONFLICT, TEXT, refused);
        };
        let message = ToDaemon::Submit {
            key: say.key,
            text: say.text,
            attached: Vec::new(),
            step: say.step,
        };
        if self.internal.send(Internal::Act { message }).is_err() {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        answer(StatusCode::NO_CONTENT, TEXT, "")
    }

    /// Does what the page posted, as `request` says it, from the page
    /// itself: at `/act`, what the human did on it, in the chat the query
    /// names, if it is done in one; at `/activity/resume` and
    /// `/activity/fix`, what its Activity has a chat do (see
    /// [`Self::say`]); at `/push`, where to
    /// push to the phone; and at `/push/end`, where to push no more.
    async fn posted(&self, request: Request<Incoming>) -> Response<Body> {
        if !self.paired(&request) {
            return answer(StatusCode::UNAUTHORIZED, TEXT, "Not paired");
        }
        if !from_the_page(&request) {
            return answer(StatusCode::FORBIDDEN, TEXT, "Not from the page");
        }
        let path = request.uri().path().to_owned();
        let kept = chat_asked(&request);
        let body = http_body_util::Limited::new(request.into_body(), MOST);
        let Ok(body) = http_body_util::BodyExt::collect(body).await else {
            return answer(StatusCode::PAYLOAD_TOO_LARGE, TEXT, "Too long");
        };
        let body = body.to_bytes();
        match path.as_str() {
            "/act" => self.act(kept, &body).await,
            "/activity/resume" => self.say(kept, None).await,
            "/activity/fix" => self.say(kept, Some(&body)).await,
            "/push" => self.subscribe(&body),
            "/push/end" => self.unsubscribe(&body),
            _ => answer(StatusCode::NOT_FOUND, TEXT, "Not found"),
        }
    }

    /// Pushes to the phone where `body` says, a `PushSubscription` as
    /// JSON, from now on.
    fn subscribe(&self, body: &[u8]) -> Response<Body> {
        let Some(pushes) = &self.phones.pushes else {
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
        let Some(pushes) = &self.phones.pushes else {
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
    /// does, and what it does in a chat, in the chat kept as `kept`,
    /// whichever key the page says.
    async fn act(&self, kept: Option<i64>, body: &[u8]) -> Response<Body> {
        let Ok(mut message) = serde_json::from_slice::<ToDaemon>(body) else {
            return answer(StatusCode::BAD_REQUEST, TEXT, "Not understood");
        };
        match done_on_the_page(&mut message) {
            None => return answer(StatusCode::FORBIDDEN, TEXT, "Not done from the phone"),
            Some(Done::Apart) => {}
            Some(Done::In(key)) => {
                let Some(kept) = kept else {
                    return answer(StatusCode::BAD_REQUEST, TEXT, "No chat");
                };
                let Some(found) = self.key_of(kept).await else {
                    return answer(StatusCode::NOT_FOUND, TEXT, "No such chat");
                };
                *key = found;
            }
        }
        // A sketch ends only in a way the human's workflow says, as the
        // window's menu offers it.
        if let ToDaemon::EndSketch { ending, .. } = &message
            && !crate::workflow::sketch().endings.contains(ending)
        {
            return answer(
                StatusCode::FORBIDDEN,
                TEXT,
                "Not a way the workflow ends a sketch",
            );
        }
        if self.internal.send(Internal::Act { message }).is_err() {
            return answer(StatusCode::SERVICE_UNAVAILABLE, TEXT, "coxd is stopping");
        }
        answer(StatusCode::NO_CONTENT, TEXT, "")
    }

    /// Pairs the phone that opened `/pair/<token>`, when it is the secret.
    fn pair(&self, token: &str) -> Response<Body> {
        let secret = self.phones.secret.borrow();
        if !same(token.as_bytes(), secret.as_bytes()) {
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
            "{COOKIE}={secret}; Path=/; Max-Age=34560000; HttpOnly; Secure; SameSite=Strict"
        );
        if let Ok(cookie) = HeaderValue::from_str(&cookie) {
            headers.insert(header::SET_COOKIE, cookie);
        }
        response
    }

    /// Whether `request` is the human's (see [`Phones::paired`]).
    fn paired(&self, request: &Request<Incoming>) -> bool {
        self.phones.paired(request.headers())
    }

    /// The key of the chat named, in the query of `request`, by the number
    /// it is kept by, or the answer to a request that names none, or a
    /// chat there is none of.
    async fn chat_named(
        &self,
        request: &Request<Incoming>,
    ) -> Result<u64, (StatusCode, &'static str)> {
        let Some(kept) = chat_asked(request) else {
            return Err((StatusCode::BAD_REQUEST, "No chat"));
        };
        self.key_of(kept)
            .await
            .ok_or((StatusCode::NOT_FOUND, "No such chat"))
    }

    /// The key of the chat kept as `kept`, as coxd says it, or nothing of
    /// a chat there is none of.
    async fn key_of(&self, kept: i64) -> Option<u64> {
        let (asked, key) = tokio::sync::oneshot::channel();
        self.internal
            .send(Internal::Key {
                number: kept,
                answer: asked,
            })
            .ok()?;
        key.await.ok().flatten()
    }

    /// The stream of events of a phone that follows the chats, watching
    /// the conversation of the chat `request` names, `?kept=<number>`, if
    /// it names one there is.
    async fn events(&self, request: &Request<Incoming>) -> Response<Body> {
        if !self.paired(request) {
            return answer(
                StatusCode::UNAUTHORIZED,
                "text/html; charset=utf-8",
                NOT_PAIRED,
            );
        }
        let chat = match chat_asked(request) {
            Some(kept) => self.key_of(kept).await,
            None => None,
        };
        let holds = number_asked(request, "from").unwrap_or(usize::MAX);
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
            opened: false,
            holds,
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

/// Runs `read`, which reads what coxd keeps, on a thread that may wait
/// for it, keeping what lasts where this thread was told to, if it was
/// (see [`storage::keep_on_this_thread`]).
async fn blocking<T: Send + 'static>(
    read: impl FnOnce() -> T + Send + 'static,
) -> Result<T, tokio::task::JoinError> {
    let data = storage::kept_on_this_thread();
    tokio::task::spawn_blocking(move || {
        if data.is_none() {
            return read();
        }
        storage::keep_on_this_thread(data);
        let read = read();
        storage::keep_on_this_thread(None);
        read
    })
    .await
}

/// The most lines of what changed the phone is told, in all: the files
/// past them are told by their paths and counts alone, as the window's
/// tab would scroll to them.
const MOST_LINES: usize = 4000;

/// What changed, as `diff` says it, as the page is told it: what the
/// files changed since, and each file, with how it changed, the lines it
/// gained and lost, and its lines, but past [`MOST_LINES`], where a file
/// is `cut` instead.
fn told_changes(diff: &cox_git::changes::Diff) -> serde_json::Value {
    use cox_git::changes::Kind;
    let mut left = MOST_LINES;
    let files: Vec<_> = diff
        .files
        .iter()
        .map(|file| {
            let cut = file.lines.len() > left;
            let lines: Vec<_> = if cut {
                Vec::new()
            } else {
                left -= file.lines.len();
                file.lines
                    .iter()
                    .map(|line| {
                        let kind = match line.kind {
                            Kind::Stretch => "stretch",
                            Kind::Same => "same",
                            Kind::Added => "added",
                            Kind::Removed => "removed",
                        };
                        serde_json::json!({
                            "kind": kind,
                            "old": line.old,
                            "new": line.new,
                            "text": line.text,
                        })
                    })
                    .collect()
            };
            serde_json::json!({
                "path": file.path,
                "was": file.was,
                "how": file.how.letter().to_string(),
                "binary": file.binary,
                "added": file.added,
                "removed": file.removed,
                "cut": cut,
                "lines": lines,
            })
        })
        .collect();
    serde_json::json!({ "since": diff.since.said(), "files": files })
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
        let chat = notice.kept.map(ChatId::number);
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

/// What the phone does, as its page does it.
enum Done<'a> {
    /// In the chat whose key it holds.
    In(&'a mut u64),
    /// In no chat, as opening one.
    Apart,
}

/// What the phone does in `message`, if it may: what its page does, and
/// nothing of the window's own, as what the stage carried out.
fn done_on_the_page(message: &mut ToDaemon) -> Option<Done<'_>> {
    match message {
        ToDaemon::Submit { key, attached, .. } => attached.is_empty().then_some(Done::In(key)),
        ToDaemon::Choose { key, .. }
        | ToDaemon::AnswerQuestion { key, .. }
        | ToDaemon::Stop { key }
        | ToDaemon::SetGoal { key, .. }
        | ToDaemon::AnswerProposal { key, .. }
        | ToDaemon::ChangeProposal { key, .. }
        | ToDaemon::AnswerKeep { key, .. }
        | ToDaemon::Merge { key }
        | ToDaemon::Set { key, .. }
        | ToDaemon::StartSketch { key, .. }
        | ToDaemon::EndSketch { key, .. } => Some(Done::In(key)),
        ToDaemon::NewChat { .. } | ToDaemon::NewProjectAside => Some(Done::Apart),
        _ => None,
    }
}

/// The path of `parts`, each as a URL has it, as `/a%20b/c.html`.
fn path_of<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut url = url::Url::parse("http://mac/").expect("an address");
    if let Ok(mut path) = url.path_segments_mut() {
        path.clear().extend(parts);
    }
    url.path().to_owned()
}

/// The chat the query of `request` names by the number it is kept by,
/// `kept=<number>`.
fn chat_asked(request: &Request<Incoming>) -> Option<i64> {
    let query = request.uri().query()?;
    url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(name, value)| (name == "kept").then(|| value.parse::<i64>().ok()))
        .flatten()
}

/// The project the query of `request` names, `project=<number>`, by the
/// number it is kept by, as the page lists it.
fn project_asked(request: &Request<Incoming>) -> Option<ProjectId> {
    serde_json::from_str(&text_asked(request, "project")?).ok()
}

/// The place in a conversation the query of `request` names, `at=<n>`.
fn place_asked(request: &Request<Incoming>) -> Option<usize> {
    number_asked(request, "at")
}

/// The number the query of `request` gives `name`, as `before=<n>`.
fn number_asked(request: &Request<Incoming>, name: &str) -> Option<usize> {
    let query = request.uri().query()?;
    url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(one, value)| (one == name).then(|| value.parse::<usize>().ok()))
        .flatten()
}

/// The text the query of `request` gives `name`, as `seen=<tag>`.
fn text_asked(request: &Request<Incoming>, name: &str) -> Option<String> {
    let query = request.uri().query()?;
    url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(one, value)| (one == name).then(|| value.into_owned()))
}

/// What the human may do of sketches in a chat, as the page offers it:
/// `{"start": "project"}`, or `"empty"`, what a sketch started beside the
/// chat starts from at first, as the window's input has it; or
/// `{"endings": [...]}`, the ways the sketch ends, of `endings`, as the
/// window's menu offers them; or `{}`, neither.
fn told_sketching(
    sketching: Option<&Sketching>,
    endings: &[crate::workflow::Ending],
) -> serde_json::Value {
    match sketching {
        Some(Sketching::Starts { project }) => {
            serde_json::json!({ "start": crate::chat::sketch::from_at_first(project) })
        }
        Some(Sketching::Ends { from }) => serde_json::json!({
            "endings": endings.iter().filter(|ending| ending.ends(*from)).collect::<Vec<_>>(),
        }),
        None => serde_json::json!({}),
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

/// The map the phone's Map page asks for at `path`, under [`MAP_API`]:
/// the map's name, if the path names one, and whether to draw it again
/// after a failed index, as the window's stage has them at
/// `cox://api/map[/again]/<name>`; or nothing of a path that is not of
/// a map.
fn map_asked(path: &str) -> Option<(Option<String>, bool)> {
    let rest = path.strip_prefix(MAP_API)?;
    if rest.is_empty() {
        return Some((None, false));
    }
    let rest = rest.strip_prefix('/')?;
    let (again, name) = match rest.strip_prefix("again/") {
        Some(name) => (true, name),
        None => (false, rest),
    };
    if name.contains('/') {
        return None;
    }
    let name = (!name.is_empty()).then(|| cox_core::address::percent_decoded(name));
    Some((name, again))
}

/// Where the Map page reads its map, beside itself, at `/map/`.
const MAP_API: &str = "/map/api/map";

/// The file `file` of the Map page, as the window's stage serves it at
/// `cox://map/` (see [`crate::map::PAGE`]), and the window's colors
/// beside it, as `theme.css`; the page itself for none. The phone's page
/// shows it in a frame, over the chat.
fn map_page(file: &str) -> Response<Body> {
    let file = if file.is_empty() { "index.html" } else { file };
    let mut response = if file == "theme.css" {
        answer(StatusCode::OK, "text/css; charset=utf-8", theme::css())
    } else {
        match crate::map::PAGE.iter().find(|(at, _)| *at == file) {
            Some((at, bytes)) => answer(StatusCode::OK, kind_of(at), Bytes::from_static(bytes)),
            None => return answer(StatusCode::NOT_FOUND, TEXT, "Not found"),
        }
    };
    frameable(&mut response);
    response
}

/// The type of a file of a page, by the end of its name.
fn kind_of(file: &str) -> &'static str {
    match file.rsplit_once('.').map(|(_, end)| end) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// Lets the phone's own page frame `response`, as it shows the Map page
/// over the chat; still no other page.
fn frameable(response: &mut Response<Body>) {
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(FRAMED_POLICY),
    );
}

/// The icon the human gave the project kept by the number `project`, as
/// the rail draws it, if it has one.
fn project_icon(project: &str) -> Response<Body> {
    match crate::project_icon::named(project).and_then(|path| std::fs::read(path).ok()) {
        Some(bytes) => answer(StatusCode::OK, "image/png", bytes),
        None => answer(StatusCode::NOT_FOUND, TEXT, "Not found"),
    }
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

/// Whether the phone that sent `headers` reads an answer packed with
/// gzip, as every browser does.
fn accepts_gzip(headers: &header::HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|codings| codings.to_str().ok())
        .flat_map(|codings| codings.split(','))
        .any(|coding| {
            let (name, weight) = coding.split_once(';').unwrap_or((coding, ""));
            let refused = weight
                .trim()
                .strip_prefix("q=")
                .and_then(|weight| weight.trim().parse::<f32>().ok())
                == Some(0.0);
            name.trim().eq_ignore_ascii_case("gzip") && !refused
        })
}

/// The fewest bytes of an answer worth packing.
const PACKED_FROM: usize = 1024;

/// `response` packed with gzip, for a phone that reads it so (see
/// [`accepts_gzip`]), when it is text whole, as a page's file or JSON:
/// a map of megabytes, as Cox's own, is a tenth of itself packed, and
/// the phone waits for each byte of it. A picture or a video, packed
/// already, a part of one, and the stream of events go as they are.
async fn packed(mut response: Response<Body>) -> Response<Body> {
    let text = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|kind| kind.to_str().ok())
        .is_some_and(|kind| {
            kind.starts_with("text/") || kind.contains("json") || kind == "image/svg+xml"
        });
    if !text
        || response.status() != StatusCode::OK
        || response.headers().contains_key(header::CONTENT_ENCODING)
    {
        return response;
    }
    let Body::Whole(Some(plain)) = response.body() else {
        return response;
    };
    if plain.len() < PACKED_FROM {
        return response;
    }
    let plain = plain.clone();
    // Off the thread coxd answers every phone and window on: megabytes
    // take a tenth of a second.
    let gzip = tokio::task::spawn_blocking(move || {
        use std::io::Write as _;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&plain)?;
        gzip.finish()
    });
    let Ok(Ok(gzip)) = gzip.await else {
        return response;
    };
    *response.body_mut() = Body::Whole(Some(gzip.into()));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    response
}

/// What the page may do: what the agents write shows as text, so the
/// page runs only its own script, and no page frames it, a sketch's mock
/// included.
const PAGE_POLICY: &str = "default-src 'self'; img-src 'self' data:; object-src 'none'; \
    base-uri 'none'; frame-ancestors 'none'";

/// What a page the phone's page frames, as the Map page, may do: as
/// [`PAGE_POLICY`], but framed by the phone's page.
const FRAMED_POLICY: &str = "default-src 'self'; img-src 'self' data:; object-src 'none'; \
    base-uri 'none'; frame-ancestors 'self'";

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
    /// Whether the phone was told the conversation it watches, which the
    /// first patch it is told is of, whole (see [`opened`]).
    opened: bool,
    /// The first place of that conversation the page holds already, as
    /// it follows the chat again, from which it is told it, or
    /// `usize::MAX` when it holds none.
    holds: usize,
    beat: tokio::time::Interval,
    id: u64,
    internal: UnboundedSender<Internal>,
}

impl Events {
    fn poll(&mut self, context: &mut Context<'_>) -> Poll<Option<Bytes>> {
        if let Poll::Ready(told) = self.told.poll_recv(context) {
            let opens = matches!(told, Some(FromDaemon::Patch { .. })) && !self.opened;
            self.opened |= opens;
            let opens = opens.then_some(self.holds);
            return Poll::Ready(told.map(|told| event(told, opens)));
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

/// `told` as a server-sent event: a line of JSON, as [`for_the_phone`]
/// has it, of the conversation the phone `opens`, if it opens one.
fn event(told: FromDaemon, opens: Option<usize>) -> Bytes {
    let json = for_the_phone(told, opens)
        .and_then(|told| serde_json::to_string(&told))
        .unwrap_or_else(|error| {
            tracing::error!("the phone cannot be told: {error}");
            "null".to_owned()
        });
    Bytes::from(format!("data: {json}\n\n"))
}

/// `told` as the phone is told it: as the window is, but for what the
/// tool calls of a conversation show, and the permission requests
/// answered, of which it is told only how many things they show, as
/// `shows`, and asks for them on `/content` as the human opens one. In a
/// long chat, what they show, as the files the agent read and what its
/// commands printed, is the most of the conversation, which a phone
/// would wait for, and the human opens few of them.
///
/// The conversation the phone `opens`, of which it is told the whole
/// first, it is told the last of, as [`opened`] says, and where they
/// start, as `from`: `opens` is the first place of it the page holds
/// already, from which it is told it all, or `usize::MAX`.
fn for_the_phone(told: FromDaemon, opens: Option<usize>) -> serde_json::Result<serde_json::Value> {
    let FromDaemon::Patch { key, mut patch } = told else {
        return serde_json::to_value(&told);
    };
    let mut entries = std::mem::take(&mut patch.entries);
    let from = opens.map(|holds| {
        let (from, last) = opened(std::mem::take(&mut entries), patch.len, holds);
        entries = last;
        from
    });
    let mut value = serde_json::to_value(FromDaemon::Patch { key, patch })?;
    let entries = entries
        .into_iter()
        .map(|(at, entry)| Ok(serde_json::json!([at, told_entry(entry)?])))
        .collect::<serde_json::Result<Vec<_>>>()?;
    value["Patch"]["patch"]["entries"] = entries.into();
    if let Some(from) = from {
        value["Patch"]["patch"]["from"] = from.into();
    }
    Ok(value)
}

/// The most entries of a conversation a phone is told as it opens it,
/// the last: about what it draws first, the last 40 places of it, as
/// many tool calls in a row fold into one. A long chat's thousands of
/// entries, told whole, the phone would wait seconds for, and draw few.
const TAIL: usize = 200;

/// Of the `entries` of a conversation that holds `len`, those a phone is
/// told as it opens it, and where the last of them start: the last
/// [`TAIL`], or all from `holds`, the first the page holds already, as
/// it follows the chat again after it read back to there, so that what
/// it holds is told afresh, as it may have changed meanwhile; and,
/// before them, those the page reads all the conversation for, which it
/// would miss: the last line of the human's, the work they close after
/// it, as what the agent showed, and the last of what the agent asked;
/// and the agents the chat switched between.
fn opened(entries: Vec<(usize, Entry)>, len: usize, holds: usize) -> (usize, Vec<(usize, Entry)>) {
    let from = len.saturating_sub(TAIL).min(holds);
    if from == 0 {
        return (0, entries);
    }
    let last = |kind: fn(&Entry) -> bool| entries.iter().rposition(|(_, entry)| kind(entry));
    let human = last(|entry| matches!(entry, Entry::Human(_)));
    let mut read: BTreeSet<usize> = [
        human,
        last(|entry| matches!(entry, Entry::Merge)),
        last(|entry| matches!(entry, Entry::Plan(_))),
        last(|entry| matches!(entry, Entry::Permission { .. })),
        last(|entry| matches!(entry, Entry::Question { .. })),
    ]
    .into_iter()
    .flatten()
    .collect();
    for (place, (_, entry)) in entries.iter().enumerate() {
        let closed = human.is_some_and(|human| place > human)
            && matches!(
                entry,
                Entry::Media { .. }
                    | Entry::Shown { .. }
                    | Entry::MapShown { .. }
                    | Entry::PlanChanged { .. }
                    | Entry::Checks(_)
                    | Entry::Event { .. }
            );
        if closed || matches!(entry, Entry::Switched { .. }) {
            read.insert(place);
        }
    }
    let entries = entries
        .into_iter()
        .enumerate()
        .filter(|(place, (at, _))| *at >= from || read.contains(place))
        .map(|(_, entry)| entry)
        .collect();
    (from, entries)
}

/// `entry` as the phone is told it (see [`for_the_phone`]).
fn told_entry(mut entry: Entry) -> serde_json::Result<serde_json::Value> {
    let content = match &mut entry {
        Entry::ToolCall { content, .. }
        | Entry::Permission {
            content,
            chosen: Some(_),
            ..
        } => Some(std::mem::take(content)),
        _ => None,
    };
    let mut value = serde_json::to_value(&entry)?;
    let fields = value
        .as_object_mut()
        .and_then(|value| value.values_mut().next())
        .and_then(serde_json::Value::as_object_mut);
    if let (Some(content), Some(fields)) = (content, fields) {
        let shows = content.iter().filter(|one| shows(one)).count();
        fields.remove("content");
        fields.insert("shows".to_owned(), shows.into());
    }
    Ok(value)
}

/// Whether the page shows `content`: what is not blank.
fn shows(content: &Content) -> bool {
    match content {
        Content::Text(text) => !text.trim().is_empty(),
        Content::Diff { .. } => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

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
        let installed = Some(PORT);
        assert_eq!(port_in(br#"{"home": "~/cox"}"#, installed), Ok(Some(PORT)));
        assert_eq!(port_in(br#"{"phone": false}"#, installed), Ok(None));
        assert_eq!(port_in(br#"{"phone": true}"#, installed), Ok(Some(PORT)));
        assert_eq!(port_in(br#"{"phone": {}}"#, installed), Ok(Some(PORT)));
        assert_eq!(
            port_in(br#"{"phone": {"port": 7422}}"#, installed),
            Ok(Some(7422))
        );
        assert!(port_in(br#"{"phone": {"port": 70000}}"#, installed).is_err());
    }

    #[test]
    fn a_cox_not_installed_serves_the_phone_only_when_told() {
        assert_eq!(port_in(br#"{"home": "~/cox"}"#, None), Ok(None));
        assert_eq!(port_in(br#"{"phone": true}"#, None), Ok(Some(PORT)));
        assert_eq!(
            port_in(br#"{"phone": {"port": 7422}}"#, None),
            Ok(Some(7422))
        );
    }

    #[test]
    fn leaves_the_tailnet_to_coxd_unless_told_not() {
        assert!(tailscale_in(br#"{"home": "~/cox"}"#, true));
        assert!(tailscale_in(br#"{"phone": {"port": 7422}}"#, true));
        assert!(!tailscale_in(br#"{"phone": {"tailscale": false}}"#, true));
    }

    #[test]
    fn a_cox_not_installed_leaves_the_tailnet_be_unless_told() {
        assert!(!tailscale_in(br#"{"phone": {"port": 7422}}"#, false));
        assert!(tailscale_in(br#"{"phone": {"tailscale": true}}"#, false));
    }

    #[test]
    fn a_coxd_of_a_test_takes_no_port_and_leaves_the_tailnet_be() {
        let folder = TempDir::new();
        storage::keep_on_this_thread(Some(folder.to_path_buf()));
        let (port, tailscale) = (port(), tailscale());
        storage::keep_on_this_thread(None);
        assert_eq!(port, None);
        assert!(!tailscale);
    }

    #[test]
    fn keeps_the_secret_it_makes() {
        let folder = TempDir::new();
        let file = folder.join("phone-token");
        let secret = token_at(&file).unwrap();
        assert_eq!(secret.len(), 64);
        assert_eq!(token_at(&file).unwrap(), secret);
        let mode = std::fs::metadata(&file).unwrap().permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
            0o600
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lets_in_no_phone_paired_before_the_human_unpairs_them() {
        let folder = TempDir::new();
        let file = folder.join("phone-token");
        let mut pushes =
            Pushes::open_at(&folder.join("push-key"), folder.join("pushes.json")).unwrap();
        pushes
            .subscribe(Subscription {
                endpoint: "https://web.push.apple.com/QAbc".to_owned(),
                keys: push::Keys {
                    p256dh: format!("BA{}", "A".repeat(85)),
                    auth: "AAAAAAAAAAAAAAAAAAAAAA".to_owned(),
                },
            })
            .unwrap();
        let before = token_at(&file).unwrap();
        let phones = Rc::new(Phones::new(before.clone(), Some(pushes)));
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, _heard) = unbounded_channel();
            tokio::task::spawn_local(serve(listener, Rc::clone(&phones), None, internal));
            let get = |path: &str, secret: &str| {
                format!(
                    "GET {path} HTTP/1.1\r\nHost: x\r\nCookie: cox_phone={secret}\r\n\
                     Connection: close\r\n\r\n"
                )
            };

            let paired = ask(port, &get("/", &before), "").await;
            assert!(paired.starts_with("HTTP/1.1 200"), "{paired}");

            let after = new_token_at(&file).unwrap();
            assert_ne!(after, before);
            assert_eq!(std::fs::read_to_string(&file).unwrap(), after);
            phones.unpaired(after.clone());

            let lost = ask(port, &get("/", &before), "").await;
            assert!(lost.starts_with("HTTP/1.1 401"), "{lost}");
            let leaked = ask(port, &get(&format!("/pair/{before}"), ""), "").await;
            assert!(leaked.starts_with("HTTP/1.1 403"), "{leaked}");
            let paired_again = ask(port, &get(&format!("/pair/{after}"), ""), "").await;
            assert!(paired_again.starts_with("HTTP/1.1 303"), "{paired_again}");
            assert!(
                paired_again.contains(&format!("cox_phone={after};")),
                "{paired_again}"
            );
            let new = ask(port, &get("/", &after), "").await;
            assert!(new.starts_with("HTTP/1.1 200"), "{new}");
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");

        let kept = std::fs::read_to_string(folder.join("pushes.json")).unwrap();
        assert_eq!(
            kept.trim(),
            "[]",
            "nothing is pushed to a phone paired before"
        );
        let reopened =
            Pushes::open_at(&folder.join("push-key"), folder.join("pushes.json")).unwrap();
        assert!(!reopened.any());
        for kept in ["phone-token", "pushes.json"] {
            let mode = std::fs::metadata(folder.join(kept)).unwrap().permissions();
            assert_eq!(
                std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
                0o600,
                "{kept} is the human's alone"
            );
        }
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
                tokio::task::spawn_local(serve(listener, Rc::new(Phones::new("secret".to_owned(), None)), None, internal));

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
                    "GET /events?kept=12 HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\n\r\n",
                    "}}\n\n",
                ));
                let Some(Internal::Key { number: 12, answer }) = heard.recv().await else {
                    panic!("coxd is not asked the key of the chat kept as 12");
                };
                answer.send(Some(7)).unwrap();
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
                    told.contains(r#"data: {"Listing":{"projects":[],"chats":[],"archived":[],"agents":{},"chosen":0}}"#),
                    "{told}"
                );
                assert!(
                    matches!(heard.recv().await, Some(Internal::Unview { id: gone }) if gone == id),
                    "the phone that went is not said to have gone"
                );

                let line = r#"{"Submit":{"key":7,"text":"go on"}}"#;
                let posted = |headers: &str| {
                    format!(
                        "POST /act?kept=12 HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\n{headers}\
                         Content-Type: application/json\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{line}",
                        line.len()
                    )
                };
                let elsewhere = ask(port, &posted("Origin: https://evil\r\nX-Cox: phone\r\n"), "").await;
                assert!(elsewhere.starts_with("HTTP/1.1 403"), "{elsewhere}");
                let unsaid = ask(port, &posted(""), "").await;
                assert!(unsaid.starts_with("HTTP/1.1 403"), "{unsaid}");
            });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn takes_no_process_on_the_mac_for_the_human_by_what_tailscale_would_say() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let as_tailscale_says =
                "Host: mac.tail1234.ts.net\r\nTailscale-User-Login: me@github\r\n";

            let line = r#"{"Submit":{"key":7,"text":"go on"}}"#;
            let posted = |cookie: &str| {
                format!(
                    "POST /act?kept=12 HTTP/1.1\r\n{as_tailscale_says}{cookie}X-Cox: phone\r\n\
                     Origin: https://mac.tail1234.ts.net\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{line}",
                    line.len()
                )
            };
            let page = ask(
                port,
                &format!("GET / HTTP/1.1\r\n{as_tailscale_says}Connection: close\r\n\r\n"),
                "",
            )
            .await;
            let act = ask(port, &posted(""), "").await;
            let sent = heard.try_recv().is_ok();
            let first = |answer: &str| answer.lines().next().unwrap_or_default().to_owned();
            assert_eq!(
                (first(&page), first(&act), sent),
                (
                    "HTTP/1.1 401 Unauthorized".to_owned(),
                    "HTTP/1.1 401 Unauthorized".to_owned(),
                    false
                ),
                "the page, what it posts, and whether a line was sent as the human's"
            );

            let with_the_secret = posted("Cookie: cox_phone=secret\r\n");
            let paired =
                tokio::task::spawn_local(async move { ask(port, &with_the_secret, "").await });
            let Some(Internal::Key { number: 12, answer }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat");
            };
            answer.send(Some(7)).unwrap();
            let paired = paired.await.unwrap();
            assert!(paired.starts_with("HTTP/1.1 204"), "{paired}");
            assert!(matches!(
                heard.recv().await,
                Some(Internal::Act {
                    message: ToDaemon::Submit { key: 7, .. }
                })
            ));
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[test]
    fn tells_the_phone_how_many_things_a_call_shows_but_not_them() {
        use crate::chat::conversation::{Patch, State};
        let call = Entry::ToolCall {
            id: "1".into(),
            title: "Read a.rs".into(),
            command: false,
            about: None,
            content: vec![
                Content::Text("fn a() {}".into()),
                Content::Text(" \n".into()),
                Content::Diff {
                    path: "/work/a.rs".into(),
                    lines: Vec::new(),
                },
            ],
            result: None,
        };
        let asked = |chosen| Entry::Permission {
            id: "2".into(),
            title: "Edit a.rs".into(),
            content: vec![Content::Text("fn b() {}".into())],
            options: vec!["Allow".into()],
            chosen,
            by_setup: false,
            gathered: Vec::new(),
        };
        let patch = Patch {
            len: 4,
            entries: vec![
                (0, Entry::Reply("Done.".into())),
                (1, call),
                (2, asked(Some(0))),
                (3, asked(None)),
            ],
            state: State::Asking,
            title: None,
            goal: None,
            settings: Vec::new(),
            commands: Vec::new(),
            before_asking: State::Idle,
            keeps: true,
            steers: false,
        };
        let told = for_the_phone(FromDaemon::Patch { key: 7, patch }, None).unwrap();
        let told = &told["Patch"];
        assert_eq!(told["key"], 7);
        assert_eq!(told["patch"]["len"], 4);
        assert_eq!(told["patch"]["state"], "Asking");
        let entries = &told["patch"]["entries"];
        assert_eq!(entries[0], serde_json::json!([0, {"reply": "Done."}]));
        assert_eq!(
            entries[1],
            serde_json::json!([1, {"tool_call": {"id": "1", "title": "Read a.rs", "result": null, "shows": 2}}])
        );
        // An answered request is a line, which opens to what it showed;
        // the one the human is asked shows it all.
        assert_eq!(entries[2][1]["permission"]["shows"], 1);
        assert!(entries[2][1]["permission"].get("content").is_none());
        assert_eq!(
            entries[3][1]["permission"]["content"],
            serde_json::json!([{"text": "fn b() {}"}])
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tells_a_paired_phone_what_a_call_shows_as_it_asks() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let get = |path: &str, cookie: &str| {
                format!("GET {path} HTTP/1.1\r\nHost: x\r\n{cookie}Connection: close\r\n\r\n")
            };
            let cookie = "Cookie: cox_phone=secret\r\n";

            let unpaired = ask(port, &get("/content?kept=12&at=3", ""), "").await;
            assert!(unpaired.starts_with("HTTP/1.1 401"), "{unpaired}");
            let nowhere = ask(port, &get("/content?kept=12", cookie), "").await;
            assert!(nowhere.starts_with("HTTP/1.1 400"), "{nowhere}");

            let request = get("/content?kept=12&at=3", cookie);
            let asked = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Key { number: 12, answer }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat kept as 12");
            };
            answer.send(Some(7)).unwrap();
            let Some(Internal::Content {
                key: 7,
                at: 3,
                answer,
            }) = heard.recv().await
            else {
                panic!("coxd is not asked what the call shows");
            };
            answer
                .send(Some(vec![Content::Text("printed".into())]))
                .unwrap();
            let told = asked.await.unwrap();
            assert!(told.starts_with("HTTP/1.1 200"), "{told}");
            assert!(
                told.ends_with(r#"{"content":[{"text":"printed"}]}"#),
                "{told}"
            );

            let request = get("/content?kept=12&at=4", cookie);
            let asked = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Key { answer, .. }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat");
            };
            answer.send(Some(7)).unwrap();
            let Some(Internal::Content { answer, .. }) = heard.recv().await else {
                panic!("coxd is not asked what the call shows");
            };
            answer.send(None).unwrap();
            let none = asked.await.unwrap();
            assert!(none.starts_with("HTTP/1.1 404"), "{none}");
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[test]
    fn tells_a_phone_opening_a_long_chat_its_last_entries_and_what_the_page_reads_before() {
        use crate::chat::conversation::{Patch, State};
        let len = TAIL + 10;
        let mut entries = vec![
            (0, Entry::Human("first".into())),
            (
                1,
                Entry::Switched {
                    from: "Claude".into(),
                    to: "Codex".into(),
                },
            ),
            (2, Entry::Thought("why".into())),
            (3, Entry::Human("last".into())),
            (4, Entry::Checks(Vec::new())),
            (
                5,
                Entry::MapShown {
                    map: None,
                    at: vec!["shop".into()],
                },
            ),
        ];
        entries.extend((6..len).map(|at| (at, Entry::Reply(format!("{at}")))));
        let patch = |entries| Patch {
            len,
            entries,
            state: State::Idle,
            title: None,
            goal: None,
            settings: Vec::new(),
            commands: Vec::new(),
            before_asking: State::Idle,
            keeps: true,
            steers: false,
        };
        let places = |told: &serde_json::Value| -> Vec<u64> {
            told["Patch"]["patch"]["entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry[0].as_u64().unwrap())
                .collect()
        };

        let opened = for_the_phone(
            FromDaemon::Patch {
                key: 7,
                patch: patch(entries.clone()),
            },
            Some(usize::MAX),
        )
        .unwrap();
        assert_eq!(opened["Patch"]["patch"]["len"], len);
        assert_eq!(opened["Patch"]["patch"]["from"], 10);
        let told = places(&opened);
        assert_eq!(told[..4], [1, 3, 4, 5], "{told:?}");
        assert_eq!(told[4..], (10..len as u64).collect::<Vec<_>>()[..]);

        // A page that read back to 6 is told all from there as it follows
        // the chat again, and what it reads before.
        let again = for_the_phone(
            FromDaemon::Patch {
                key: 7,
                patch: patch(entries.clone()),
            },
            Some(6),
        )
        .unwrap();
        assert_eq!(again["Patch"]["patch"]["from"], 6);
        let told = places(&again);
        assert_eq!(told[..4], [1, 3, 4, 5], "{told:?}");
        assert_eq!(told[4..], (6..len as u64).collect::<Vec<_>>()[..]);

        // And a page that holds more than the conversation does, as it
        // was cut short meanwhile, is told its last.
        let shorter = for_the_phone(
            FromDaemon::Patch {
                key: 7,
                patch: patch(entries.clone()),
            },
            Some(len + 50),
        )
        .unwrap();
        assert_eq!(shorter["Patch"]["patch"]["from"], 10);

        // A patch after it, as the conversation goes on, is told whole.
        let after = for_the_phone(
            FromDaemon::Patch {
                key: 7,
                patch: patch(entries),
            },
            None,
        )
        .unwrap();
        assert!(after["Patch"]["patch"].get("from").is_none());
        assert_eq!(places(&after).len(), len);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tells_a_paired_phone_the_entries_before_those_it_was_told() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let get = |path: &str, cookie: &str| {
                format!("GET {path} HTTP/1.1\r\nHost: x\r\n{cookie}Connection: close\r\n\r\n")
            };
            let cookie = "Cookie: cox_phone=secret\r\n";

            let unpaired = ask(port, &get("/entries?kept=12&before=250", ""), "").await;
            assert!(unpaired.starts_with("HTTP/1.1 401"), "{unpaired}");

            let request = get("/entries?kept=12&before=250", cookie);
            let asked = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Key { number: 12, answer }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat kept as 12");
            };
            answer.send(Some(7)).unwrap();
            let Some(Internal::Entries {
                key: 7,
                from: 50,
                to: 250,
                answer,
            }) = heard.recv().await
            else {
                panic!("coxd is not asked the entries before those told");
            };
            let call = Entry::ToolCall {
                id: "1".into(),
                title: "Read a.rs".into(),
                command: false,
                about: None,
                content: vec![Content::Text("fn a() {}".into())],
                result: None,
            };
            answer
                .send(Some(vec![Entry::Reply("Done.".into()), call]))
                .unwrap();
            let told = asked.await.unwrap();
            assert!(told.starts_with("HTTP/1.1 200"), "{told}");
            let body: serde_json::Value =
                serde_json::from_str(told.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(
                body,
                serde_json::json!({
                    "from": 50,
                    "entries": [
                        [50, {"reply": "Done."}],
                        [51, {"tool_call": {"id": "1", "title": "Read a.rs", "result": null, "shows": 1}}],
                    ],
                })
            );
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    /// A chat as coxd's Activity is told of it, kept as `kept`, in the
    /// project of `project`, standing by with nothing to say.
    fn standing_by(key: u64, kept: ChatId, project: &Path) -> activity::Chat {
        activity::Chat {
            key,
            name: format!("chat {key}"),
            cwd: project.to_owned(),
            project: project.to_owned(),
            asking: false,
            unread: false,
            goal: None,
            working: false,
            background: Vec::new(),
            parent: None,
            kept: Some(kept),
            pulls: Vec::new(),
            spent: None,
            resolved: false,
            reported: false,
            archived: false,
            steers: false,
            on_steps: Vec::new(),
            active: 0,
            cut_off: None,
        }
    }

    /// Answers the page as coxd would, with no window: the key of each
    /// chat of `told`, by the number it is kept by, and how the chats
    /// stand, with what each does `now`; and keeps what the page has it
    /// do in `done`.
    async fn play_coxd(
        mut heard: UnboundedReceiver<Internal>,
        told: activity::Told,
        now: Vec<(u64, crate::commands::Now)>,
        done: Rc<RefCell<Vec<ToDaemon>>>,
    ) {
        while let Some(asked) = heard.recv().await {
            match asked {
                Internal::Key { number, answer } => {
                    let key = told
                        .chats
                        .iter()
                        .find(|chat| chat.kept.is_some_and(|kept| kept.number() == number))
                        .map(|chat| chat.key);
                    answer.send(key).ok();
                }
                Internal::Activity { answer } => {
                    answer.send((told.clone(), now.clone())).ok();
                }
                Internal::Act { message } => done.borrow_mut().push(message),
                _ => {}
            }
        }
    }

    /// What `answer`, of the page, holds after its head, as JSON.
    fn json_of(answer: &str) -> serde_json::Value {
        let (_, body) = answer
            .split_once("\r\n\r\n")
            .expect("an answer with a head");
        serde_json::from_str(body).unwrap_or_else(|error| panic!("{error}: {answer}"))
    }

    /// A chat that asks the human something, and one at work on a plan
    /// coxd keeps, half done, in a project of `folder`, as coxd tells its
    /// Activity of them, with what each does now, and the chats.
    fn asking_and_at_work(
        folder: &Path,
    ) -> (
        activity::Told,
        Vec<(u64, crate::commands::Now)>,
        [ChatId; 2],
    ) {
        use cox_core::entry::{Step, StepState};
        let kept = cox_core::store::Store::open(&storage::path(Kept::Chats).unwrap()).unwrap();
        let asking = kept.add_chat(None, "asks").unwrap();
        let working = kept.add_chat(None, "works").unwrap();
        let step = |text: &str, state| Step {
            text: text.to_owned(),
            state,
            since: None,
            started: None,
            id: None,
        };
        kept.keep_entry(working, 0, &Entry::Human("fix it".to_owned()))
            .unwrap();
        let plan = vec![
            step("Read", StepState::Completed),
            step("Fix", StepState::InProgress),
        ];
        kept.keep_entry(working, 1, &Entry::Plan(plan)).unwrap();
        let (home, work) = (folder.join("home"), folder.join("work"));
        let told = activity::Told {
            chats: vec![
                activity::Chat {
                    asking: true,
                    ..standing_by(1, asking, &work)
                },
                activity::Chat {
                    working: true,
                    ..standing_by(2, working, &work)
                },
            ],
            projects: vec![(home.clone(), "Cox".to_owned()), (work, "work".to_owned())],
            home,
            ..activity::Told::default()
        };
        let now = vec![
            (
                1,
                crate::commands::Now {
                    asks: Some("Which way?".to_owned()),
                    ..Default::default()
                },
            ),
            (
                2,
                crate::commands::Now {
                    said: Some("Fixing it.".to_owned()),
                    running: Some(("Runs the tests".to_owned(), "cargo test".to_owned())),
                    asks: None,
                },
            ),
        ];
        (told, now, [asking, working])
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tells_a_paired_phone_what_goes_on_with_no_window_open() {
        let folder = TempDir::new();
        storage::keep_on_this_thread(Some(folder.join("data")));
        let (told, now, [asking, working]) = asking_and_at_work(&folder);
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            // No window: only the page, and coxd behind it.
            let (internal, heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            tokio::task::spawn_local(play_coxd(heard, told, now, Rc::default()));
            let get = |path: &str, cookie: &str| {
                format!("GET {path} HTTP/1.1\r\nHost: x\r\n{cookie}Connection: close\r\n\r\n")
            };
            let cookie = "Cookie: cox_phone=secret\r\n";
            let timeline = format!("/activity/timeline?kept={}", working.number());

            for path in ["/activity", timeline.as_str()] {
                let unpaired = ask(port, &get(path, ""), "").await;
                assert!(unpaired.starts_with("HTTP/1.1 401"), "{path}: {unpaired}");
                assert!(!unpaired.contains("Which way?"), "{unpaired}");
            }

            let answered = ask(port, &get("/activity", cookie), "").await;
            assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
            assert!(answered.contains("application/json"), "{answered}");
            let told = json_of(&answered);
            // Of every project, as the home's page, no window showing a chat.
            assert_eq!(told["home"], true, "{told}");
            assert_eq!(told["shown"], serde_json::Value::Null, "{told}");
            assert_eq!(told["busy"], 1, "{told}");
            let waiting = &told["waiting"][0];
            assert_eq!(waiting["chat"], asking.number(), "{told}");
            assert_eq!(waiting["why"], "asks you", "{told}");
            assert_eq!(waiting["asks"], "Which way?", "{told}");
            assert_eq!(waiting["where"], "work", "{told}");
            // The plan is read from what coxd keeps, and what the chat
            // does now from its conversation.
            let planned = &told["plans"][0];
            assert_eq!(planned["chat"], working.number(), "{told}");
            assert_eq!(planned["working"], true, "{told}");
            assert_eq!(planned["said"], "Fixing it.", "{told}");
            assert_eq!(planned["running"]["called"], "Runs the tests", "{told}");
            assert_eq!(planned["plan"][1]["text"], "Fix", "{told}");
            assert_eq!(
                planned["progress"],
                serde_json::json!({ "done": 1, "of": 2 }),
                "{told}"
            );
            assert_eq!(
                told["summaries"].as_array().map(Vec::len),
                Some(3),
                "{told}"
            );

            let answered = ask(port, &get(&timeline, cookie), "").await;
            assert!(answered.starts_with("HTTP/1.1 200"), "{answered}");
            let did = json_of(&answered);
            assert_eq!(did["chat"], working.number(), "{did}");
            let events = did["events"].as_array().expect("what it did when");
            assert!(
                events
                    .iter()
                    .any(|event| event["kind"] == "start" && event["text"] == "fix it"),
                "{did}"
            );

            let unnamed = ask(port, &get("/activity/timeline", cookie), "").await;
            assert!(unnamed.starts_with("HTTP/1.1 400"), "{unnamed}");
            let none = ask(port, &get("/activity/timeline?kept=999", cookie), "").await;
            assert!(none.starts_with("HTTP/1.1 404"), "{none}");
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
        storage::keep_on_this_thread(None);
    }

    /// The chats kept as 11, cut off, 12, which opened the pull request
    /// #8, whose CI failed, and 13, cut off, but at work again, as coxd
    /// tells its Activity of them.
    fn cut_off_and_failed() -> activity::Told {
        use crate::github::{Check, CheckState, Mergeable, Pull, PullState, Status, Times};
        use cox_core::store::{Ended, TurnEnd};
        let kept =
            |number: i64| serde_json::from_value::<ChatId>(serde_json::json!(number)).unwrap();
        let work = Path::new("/work");
        let cut_off = Some(TurnEnd {
            at: 5,
            how: Ended::Limited,
            why: Some("resets 3pm".to_owned()),
        });
        activity::Told {
            chats: vec![
                activity::Chat {
                    cut_off: cut_off.clone(),
                    ..standing_by(1, kept(11), work)
                },
                standing_by(2, kept(12), work),
                activity::Chat {
                    cut_off,
                    working: true,
                    ..standing_by(3, kept(13), work)
                },
            ],
            projects: vec![(work.to_owned(), "work".to_owned())],
            home: "/home".into(),
            pulls: vec![activity::Pull {
                project: work.to_owned(),
                pull: Pull {
                    repo: "tennashi/cox".to_owned(),
                    number: 8,
                },
                status: Some(Status {
                    title: "pull 8".to_owned(),
                    state: PullState::Open,
                    head: "a".to_owned(),
                    checks: vec![Check {
                        name: "ci".to_owned(),
                        state: CheckState::Failed,
                        link: None,
                        ended: None,
                    }],
                    times: Times::default(),
                    review: None,
                    mergeable: Mergeable::Unknown,
                    size: None,
                }),
                author: Some(2),
            }],
            ..activity::Told::default()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn has_a_chat_go_on_or_fix_its_ci_only_where_the_windows_activity_would() {
        let told = cut_off_and_failed();
        let done = Rc::new(RefCell::new(Vec::new()));
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            tokio::task::spawn_local(play_coxd(heard, told, Vec::new(), Rc::clone(&done)));
            let posted = |path: &str, head: &str, body: &str| {
                format!(
                    "POST {path} HTTP/1.1\r\nHost: x\r\n{head}Content-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let ours = "Cookie: cox_phone=secret\r\nX-Cox: phone\r\n";

            let unpaired = ask(
                port,
                &posted("/activity/resume?kept=11", "X-Cox: phone\r\n", "{}"),
                "",
            )
            .await;
            assert!(unpaired.starts_with("HTTP/1.1 401"), "{unpaired}");
            let elsewhere = ask(
                port,
                &posted(
                    "/activity/resume?kept=11",
                    "Cookie: cox_phone=secret\r\n",
                    "{}",
                ),
                "",
            )
            .await;
            assert!(elsewhere.starts_with("HTTP/1.1 403"), "{elsewhere}");
            assert!(done.borrow().is_empty(), "nothing is sent for them");

            // A chat cut off goes on, by the line the human would send.
            let resumed = ask(port, &posted("/activity/resume?kept=11", ours, "{}"), "").await;
            assert!(resumed.starts_with("HTTP/1.1 204"), "{resumed}");
            // Not one that was not cut off, nor one at work again, nor
            // one there is none of.
            for (path, status) in [
                ("/activity/resume?kept=12", "409"),
                ("/activity/resume?kept=13", "409"),
                ("/activity/resume?kept=99", "404"),
                ("/activity/resume", "400"),
            ] {
                let refused = ask(port, &posted(path, ours, "{}"), "").await;
                assert!(
                    refused.starts_with(&format!("HTTP/1.1 {status}")),
                    "{path}: {refused}"
                );
            }

            // The chat that opened a pull request whose CI failed is
            // asked to fix it; not another chat, nor for another pull
            // request.
            let pull = r#"{"repo":"tennashi/cox","number":8}"#;
            let fixed = ask(port, &posted("/activity/fix?kept=12", ours, pull), "").await;
            assert!(fixed.starts_with("HTTP/1.1 204"), "{fixed}");
            for (path, body, status) in [
                ("/activity/fix?kept=11", pull, "409"),
                (
                    "/activity/fix?kept=12",
                    r#"{"repo":"tennashi/cox","number":9}"#,
                    "409",
                ),
                ("/activity/fix?kept=12", "{}", "400"),
            ] {
                let refused = ask(port, &posted(path, ours, body), "").await;
                assert!(
                    refused.starts_with(&format!("HTTP/1.1 {status}")),
                    "{path} {body}: {refused}"
                );
            }
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");

        assert_eq!(
            *done.borrow(),
            [
                ToDaemon::Submit {
                    key: 1,
                    text: activity::RESUME.to_owned(),
                    attached: Vec::new(),
                    step: None,
                },
                ToDaemon::Submit {
                    key: 2,
                    text: "CI failed on #8 (https://github.com/tennashi/cox/pull/8): ci. Fix it."
                        .to_owned(),
                    attached: Vec::new(),
                    step: None,
                },
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reaches_a_chat_by_the_number_it_is_kept_by_alone() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let posted = |path: &str, line: &str| {
                format!(
                    "POST {path} HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\n\
                     Origin: http://x\r\nX-Cox: phone\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{line}",
                    line.len()
                )
            };
            let line = r#"{"Submit":{"key":7,"text":"go on"}}"#;

            let unpaired = ask(
                port,
                "GET /events?kept=12 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
                "",
            )
            .await;
            assert!(unpaired.starts_with("HTTP/1.1 401"), "{unpaired}");
            let by_key = ask(port, &posted("/act", line), "").await;
            assert!(by_key.starts_with("HTTP/1.1 400"), "{by_key}");
            let old = ask(port, &posted("/act?chat=7", line), "").await;
            assert!(old.starts_with("HTTP/1.1 400"), "{old}");

            let request = posted("/act?kept=13", line);
            let gone = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Key { number: 13, answer }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat kept as 13");
            };
            answer.send(None).unwrap();
            let gone = gone.await.unwrap();
            assert!(gone.starts_with("HTTP/1.1 404"), "{gone}");

            let request = posted("/act?kept=12", line);
            let done = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Key { number: 12, answer }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat kept as 12");
            };
            answer.send(Some(3)).unwrap();
            let done = done.await.unwrap();
            assert!(done.starts_with("HTTP/1.1 204"), "{done}");
            assert!(
                matches!(
                    heard.recv().await,
                    Some(Internal::Act { message: ToDaemon::Submit { key: 3, ref text, .. } }) if text == "go on"
                ),
                "the line goes to the chat kept as 12, not to the key the page held"
            );

            let opened = ask(port, &posted("/act", r#"{"NewChat":{"folder":null}}"#), "").await;
            assert!(opened.starts_with("HTTP/1.1 204"), "{opened}");
            assert!(
                matches!(
                    heard.recv().await,
                    Some(Internal::Act {
                        message: ToDaemon::NewChat { folder: None }
                    })
                ),
                "nothing done in another chat on the way"
            );

            let events = tokio::task::spawn_local(ask(
                port,
                "GET /events?chat=7 HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\n\r\n",
                "",
            ));
            let Some(Internal::View { id, .. }) = heard.recv().await else {
                panic!("the phone did not come");
            };
            assert!(
                matches!(
                    heard.recv().await,
                    Some(Internal::ViewChat { id: of, key: None }) if of == id
                ),
                "an address of an older coxd watches no chat"
            );
            events.abort();
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn draws_in_the_window_colors_and_shows_the_icons_to_a_paired_phone() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, _heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let get = |path: &str, cookie: &str| {
                format!("GET {path} HTTP/1.1\r\nHost: x\r\n{cookie}Connection: close\r\n\r\n")
            };
            // The pairing page is drawn in them too.
            let theme = ask(port, &get("/theme.css", ""), "").await;
            assert!(theme.starts_with("HTTP/1.1 200"), "{theme}");
            assert!(theme.contains("--asking-ground: #1e1b15;"), "{theme}");
            let unpaired = ask(port, &get("/project-icon/1", ""), "").await;
            assert!(unpaired.starts_with("HTTP/1.1 401"), "{unpaired}");
            let cookie = "Cookie: cox_phone=secret\r\n";
            let none = ask(port, &get("/project-icon/1", cookie), "").await;
            assert!(none.starts_with("HTTP/1.1 404"), "{none}");
            let elsewhere = ask(port, &get("/project-icon/..%2fphone-token", cookie), "").await;
            assert!(elsewhere.starts_with("HTTP/1.1 404"), "{elsewhere}");
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn serves_the_map_page_and_the_chats_map_to_a_paired_phone_alone() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let get = |path: &str, cookie: &str| {
                format!("GET {path} HTTP/1.1\r\nHost: x\r\n{cookie}Connection: close\r\n\r\n")
            };
            let cookie = "Cookie: cox_phone=secret\r\n";

            for path in [
                "/map/",
                "/map/app.js",
                "/map/theme.css",
                "/map/api/map?kept=12",
            ] {
                let unpaired = ask(port, &get(path, ""), "").await;
                assert!(unpaired.starts_with("HTTP/1.1 401"), "{path}: {unpaired}");
            }

            // The files the window's stage serves, and the theme beside
            // them, which the phone's page may frame, and no other.
            let page = ask(port, &get("/map/", cookie), "").await;
            assert!(page.starts_with("HTTP/1.1 200"), "{page}");
            assert!(
                page.contains("content-type: text/html; charset=utf-8"),
                "{page}"
            );
            assert!(
                page.contains(&format!("content-security-policy: {FRAMED_POLICY}")),
                "{page}"
            );
            assert!(FRAMED_POLICY.contains("default-src 'self'"));
            assert!(FRAMED_POLICY.contains("frame-ancestors 'self'"));
            assert!(page.contains("href=\"./theme.css\""), "{page}");
            assert!(
                !page.contains("\"cox:"),
                "nothing the phone refuses: {page}"
            );
            for (file, bytes) in crate::map::PAGE {
                let served = ask(port, &get(&format!("/map/{file}"), cookie), "").await;
                assert!(served.starts_with("HTTP/1.1 200"), "{file}: {served}");
                assert!(served.contains(kind_of(file)), "{file}: {served}");
                assert!(
                    served.ends_with(std::str::from_utf8(bytes).unwrap()),
                    "{file} is served as it is"
                );
            }
            let theme = ask(port, &get("/map/theme.css", cookie), "").await;
            assert!(theme.contains("text/css"), "{theme}");
            assert!(theme.contains("--asking-ground: #1e1b15;"), "{theme}");
            let missing = ask(port, &get("/map/../app.js", cookie), "").await;
            assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

            // The map of the project where the chat kept as 12 works, as
            // the window's Map page asks for it.
            let unnamed = ask(port, &get("/map/api/map", cookie), "").await;
            assert!(unnamed.starts_with("HTTP/1.1 400"), "{unnamed}");
            for (path, asked, again) in [
                ("/map/api/map?kept=12", None, false),
                ("/map/api/map/use%3Aflow?kept=12", Some("use:flow"), false),
                ("/map/api/map/again/use?kept=12", Some("use"), true),
            ] {
                let request = get(path, cookie);
                let told = tokio::task::spawn_local(async move { ask(port, &request, "").await });
                let Some(Internal::Key { number: 12, answer }) = heard.recv().await else {
                    panic!("coxd is not asked the key of the chat kept as 12");
                };
                answer.send(Some(7)).unwrap();
                let Some(Internal::Map {
                    of: MapOf::Chat(7),
                    name,
                    again: drawn_again,
                    answer,
                }) = heard.recv().await
                else {
                    panic!("coxd is not asked the map of the chat 7");
                };
                assert_eq!((name.as_deref(), drawn_again), (asked, again), "{path}");
                answer.send(Some(r#"{"state":"none"}"#.to_owned())).unwrap();
                let told = told.await.unwrap();
                assert!(told.starts_with("HTTP/1.1 200"), "{path}: {told}");
                assert!(told.contains("content-type: application/json"), "{told}");
                assert!(told.ends_with(r#"{"state":"none"}"#), "{told}");
            }
            let request = get("/map/api/map?kept=13", cookie);
            let gone = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Key { number: 13, answer }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat kept as 13");
            };
            answer.send(None).unwrap();
            let gone = gone.await.unwrap();
            assert!(gone.starts_with("HTTP/1.1 404"), "{gone}");
            let deeper = ask(port, &get("/map/api/map/a/b?kept=12", cookie), "").await;
            assert!(deeper.starts_with("HTTP/1.1 404"), "{deeper}");
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    /// The head of what `request` is answered, and what it holds, as it
    /// was sent.
    async fn ask_bytes(port: u16, request: &str) -> (String, Vec<u8>) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).await.unwrap();
        let end = answer
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .expect("the answer has a head");
        let head = String::from_utf8_lossy(&answer[..end]).into_owned();
        (head, answer[end + 4..].to_vec())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn packs_a_map_and_a_pages_file_for_a_phone_that_reads_gzip() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let get = |path: &str, accepts: &str| {
                format!(
                    "GET {path} HTTP/1.1\r\nHost: x\r\nCookie: cox_phone=secret\r\n\
                     {accepts}Connection: close\r\n\r\n"
                )
            };
            let unpacked = |bytes: &[u8]| {
                let mut plain = Vec::new();
                flate2::read::GzDecoder::new(bytes)
                    .read_to_end(&mut plain)
                    .expect("it is gzip");
                plain
            };
            let gzip = "Accept-Encoding: gzip, deflate, br\r\n";

            let map = format!(r#"{{"state":"ready","map":"{}"}}"#, "type ".repeat(4000));
            let request = get("/map/api/map?kept=12", gzip);
            let told = tokio::task::spawn_local(async move { ask_bytes(port, &request).await });
            let Some(Internal::Key { number: 12, answer }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat kept as 12");
            };
            answer.send(Some(7)).unwrap();
            let Some(Internal::Map { answer, .. }) = heard.recv().await else {
                panic!("coxd is not asked the map of the chat 7");
            };
            answer.send(Some(map.clone())).unwrap();
            let (top, body) = told.await.unwrap();
            assert!(top.starts_with("HTTP/1.1 200"), "{top}");
            assert!(top.contains("content-encoding: gzip"), "{top}");
            assert!(top.contains("vary: Accept-Encoding"), "{top}");
            assert!(top.contains("content-type: application/json"), "{top}");
            assert!(
                body.len() * 10 < map.len(),
                "{} bytes of {}",
                body.len(),
                map.len()
            );
            assert_eq!(unpacked(&body), map.as_bytes());

            let (_, canvas) = crate::map::PAGE
                .iter()
                .find(|(file, _)| *file == "canvas.js")
                .unwrap();
            let (top, body) = ask_bytes(port, &get("/map/canvas.js", gzip)).await;
            assert!(top.contains("content-encoding: gzip"), "{top}");
            assert!(top.contains(FRAMED_POLICY), "{top}");
            assert_eq!(unpacked(&body), *canvas);

            // As it is: for a phone that does not read gzip, and what is
            // too short to gain by it.
            for accepts in ["", "Accept-Encoding: br, gzip;q=0\r\n"] {
                let (top, body) = ask_bytes(port, &get("/map/canvas.js", accepts)).await;
                assert!(!top.contains("content-encoding"), "{accepts}: {top}");
                assert_eq!(body, *canvas);
            }
            let (top, body) = ask_bytes(port, &get("/map/plugin.json", gzip)).await;
            assert!(top.starts_with("HTTP/1.1 200"), "{top}");
            assert!(!top.contains("content-encoding"), "{top}");
            assert!(body.len() < PACKED_FROM);
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn asks_the_map_of_a_project_by_the_project_whatever_chat_is_named() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let cookie = "Cookie: cox_phone=secret\r\n";
            let get = |path: &str| {
                format!("GET {path} HTTP/1.1\r\nHost: x\r\n{cookie}Connection: close\r\n\r\n")
            };
            let request = get("/map/api/map/use?project=3&kept=12");
            let told = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Map {
                of: MapOf::Project(project),
                name,
                answer,
                ..
            }) = heard.recv().await
            else {
                panic!("coxd is not asked the map of a project");
            };
            assert_eq!(project.to_string(), "3");
            assert_eq!(name.as_deref(), Some("use"));
            answer.send(Some(r#"{"state":"none"}"#.to_owned())).unwrap();
            let told = told.await.unwrap();
            assert!(told.starts_with("HTTP/1.1 200"), "{told}");
            assert!(told.ends_with(r#"{"state":"none"}"#), "{told}");

            let request = get("/map/api/map?project=4");
            let gone = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Map { answer, .. }) = heard.recv().await else {
                panic!("coxd is not asked the map of a project");
            };
            answer.send(None).unwrap();
            let gone = gone.await.unwrap();
            assert!(gone.starts_with("HTTP/1.1 404"), "{gone}");
            assert!(gone.ends_with("No such chat or project"), "{gone}");
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tells_a_phone_that_has_the_map_the_rest_alone() {
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), None)),
                None,
                internal,
            ));
            let cookie = "Cookie: cox_phone=secret\r\n";
            let get = |path: &str| {
                format!("GET {path} HTTP/1.1\r\nHost: x\r\n{cookie}Connection: close\r\n\r\n")
            };
            let request = get("/map/api/map?kept=12&seen=0123456789abcdef");
            let told = tokio::task::spawn_local(async move { ask(port, &request, "").await });
            let Some(Internal::Key { number: 12, answer }) = heard.recv().await else {
                panic!("coxd is not asked the key of the chat kept as 12");
            };
            answer.send(Some(7)).unwrap();
            let Some(Internal::Map { answer, .. }) = heard.recv().await else {
                panic!("coxd is not asked the map of the chat 7");
            };
            answer
                .send(Some(
                    r#"{"state":"ready","tag":"0123456789abcdef","map":{"nodes":{}}}"#.to_owned(),
                ))
                .unwrap();
            let told = told.await.unwrap();
            assert!(
                told.ends_with(r#"{"state":"ready","tag":"0123456789abcdef"}"#),
                "{told}"
            );
        });
        tokio::time::timeout(Duration::from_secs(30), test)
            .await
            .expect("the page answers in time");
    }

    #[test]
    fn reads_which_map_the_map_page_asks_for_as_the_stage_has_it() {
        assert_eq!(map_asked("/map/api/map"), Some((None, false)));
        assert_eq!(map_asked("/map/api/map/"), Some((None, false)));
        assert_eq!(
            map_asked("/map/api/map/use"),
            Some((Some("use".to_owned()), false))
        );
        // A repository's map, named after it, has its `/` encoded.
        assert_eq!(
            map_asked("/map/api/map/again/a%2Fb"),
            Some((Some("a/b".to_owned()), true))
        );
        assert_eq!(map_asked("/map/api/map/again/"), Some((None, true)));
        assert_eq!(map_asked("/map/api/maps"), None);
        assert_eq!(map_asked("/map/api/map/a/b"), None);
        assert_eq!(map_asked("/map/app.js"), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tells_the_phone_coxd_key_and_keeps_where_to_push() {
        let folder = TempDir::new();
        let pushes = Pushes::open_at(&folder.join("push-key"), folder.join("pushes.json")).unwrap();
        let key = pushes.public_key();
        let local = tokio::task::LocalSet::new();
        let test = local.run_until(async {
            let listener = listen(0).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let (internal, mut heard) = unbounded_channel();
            tokio::task::spawn_local(serve(
                listener,
                Rc::new(Phones::new("secret".to_owned(), Some(pushes))),
                None,
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
    fn tells_what_changed_up_to_as_many_lines_as_it_shows() {
        use cox_git::changes::{Diff, File, How, Kind, Line, Since};
        let line = |kind, text: &str| Line {
            kind,
            old: None,
            new: Some(1),
            at: 1,
            text: text.to_owned(),
        };
        let file = |path: &str, lines: Vec<Line>| File {
            path: path.into(),
            was: None,
            how: How::Changed,
            added: lines.iter().filter(|line| line.kind == Kind::Added).count(),
            removed: 0,
            lines,
            binary: false,
        };
        let many = (0..MOST_LINES).map(|_| line(Kind::Same, "x")).collect();
        let diff = Diff {
            top: "/work".into(),
            since: Since::Head("abc1234".to_owned()),
            files: vec![
                file(
                    "a.rs",
                    vec![line(Kind::Stretch, "@@"), line(Kind::Added, "fn a() {}")],
                ),
                file("b.rs", many),
            ],
        };
        let told = told_changes(&diff);
        assert_eq!(told["since"], "since the last commit, abc1234");
        assert_eq!(told["files"][0]["path"], "a.rs");
        assert_eq!(told["files"][0]["how"], "M");
        assert_eq!(told["files"][0]["added"], 1);
        assert_eq!(told["files"][0]["cut"], false);
        assert_eq!(told["files"][0]["lines"][1]["kind"], "added");
        assert_eq!(told["files"][0]["lines"][1]["text"], "fn a() {}");
        assert_eq!(told["files"][1]["cut"], true);
        assert_eq!(told["files"][1]["lines"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn tells_how_a_sketch_starts_or_the_ways_it_ends() {
        use crate::chat::sketch::Sketch;
        let ending = |name: &str, from: Option<Sketch>, open: bool| crate::workflow::Ending {
            name: name.to_owned(),
            from,
            sum_up: None,
            keep: false,
            open,
            then: None,
            goal: None,
        };
        let endings = [
            ending("Open as project", None, true),
            ending("Rewrite in project", Some(Sketch::Project), false),
            ending("Discard sketch", None, false),
        ];
        let names = |told: &serde_json::Value| -> Vec<String> {
            told["endings"].as_array().map_or_default(|endings| {
                endings
                    .iter()
                    .map(|ending| ending["name"].to_string())
                    .collect()
            })
        };
        let ends = |from| told_sketching(Some(&Sketching::Ends { from }), &endings);
        assert_eq!(
            names(&ends(Sketch::Project)),
            [r#""Rewrite in project""#, r#""Discard sketch""#]
        );
        assert_eq!(
            names(&ends(Sketch::Empty)),
            [r#""Open as project""#, r#""Discard sketch""#]
        );
        assert_eq!(ends(Sketch::Project)["endings"][0]["from"], "project");
        // A folder in no Git repository starts its sketches from nothing.
        let project = TempDir::new();
        let starts = told_sketching(
            Some(&Sketching::Starts {
                project: project.to_path_buf(),
            }),
            &endings,
        );
        assert_eq!(starts, serde_json::json!({ "start": "empty" }));
        assert_eq!(told_sketching(None, &endings), serde_json::json!({}));
    }

    #[test]
    fn does_only_what_the_page_does() {
        /// What the phone may do with a message.
        #[derive(Debug, PartialEq)]
        enum May {
            InChat(u64),
            Apart,
            Not,
        }
        let done = |mut message: ToDaemon| match done_on_the_page(&mut message) {
            Some(Done::In(key)) => May::InChat(*key),
            Some(Done::Apart) => May::Apart,
            None => May::Not,
        };
        let line = |attached: Vec<cox_core::entry::Attachment>| ToDaemon::Submit {
            key: 1,
            text: "go on".to_owned(),
            attached,
            step: None,
        };
        assert_eq!(done(line(Vec::new())), May::InChat(1));
        assert_eq!(done(ToDaemon::Choose { key: 1, index: 0 }), May::InChat(1));
        assert_eq!(done(ToDaemon::Stop { key: 1 }), May::InChat(1));
        assert_eq!(done(ToDaemon::Merge { key: 1 }), May::InChat(1));
        assert_eq!(
            done(ToDaemon::Set {
                key: 1,
                id: "model".to_owned(),
                value: "opus".to_owned(),
            }),
            May::InChat(1)
        );
        assert_eq!(
            done(ToDaemon::StartSketch {
                key: 1,
                sketch: crate::chat::sketch::Sketch::Empty,
                what: "a mock".to_owned(),
            }),
            May::InChat(1)
        );
        assert_eq!(done(ToDaemon::NewChat { folder: None }), May::Apart);
        assert_eq!(done(ToDaemon::NewProjectAside), May::Apart);
        assert_eq!(done(ToDaemon::NewProject), May::Not);
        assert_eq!(done(ToDaemon::Open { folder: None }), May::Not);
        assert_eq!(
            done(ToDaemon::Archive {
                key: 1,
                archived: true
            }),
            May::Not
        );
        let picture = cox_core::entry::Attachment {
            label: "region".to_owned(),
            uri: "cox://region".to_owned(),
            text: String::new(),
            picture: Some("/etc/passwd".into()),
        };
        assert_eq!(done(line(vec![picture])), May::Not);
    }

    #[test]
    fn tells_the_secret_only_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
    }
}
