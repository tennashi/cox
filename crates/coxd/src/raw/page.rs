//! A web page read whole, for `raw_keep` to keep as it was read, rather
//! than what an agent's own fetch tool makes of it: Claude Code's
//! `WebFetch` answers what a smaller model says of the page, not the
//! page.
//!
//! Cox fetches the page itself and reads its article as Readability
//! finds it, apart from the site's menus, as Markdown, as Obsidian's Web
//! Clipper does. The page is anyone's, so what reads it is kept apart:
//! `cox --read-page <url>` reads it in a process of its own, which Cox
//! stops past [`READ_TIME`], so that a page made to keep the parser busy,
//! as one nested a hundred thousand deep, which takes it minutes, or to
//! use up its stack, holds up and takes down nothing but that process.
//! It fetches only `http` and `https`, and no address of the machine's
//! own or of the networks it is in, as `localhost`, the router's or a
//! cloud's metadata, not even through a redirect, so that a page cannot
//! have Cox read what is not the web's for the agent, nor ask a service
//! there to do something by being read; only [`LARGEST`] bytes of it;
//! and only HTML and text.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::process::{ExitCode, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// The option that runs `cox` as the reader of the page that follows.
pub const OPTION: &str = "--read-page";

/// The most bytes of a page read, as it is once unpacked.
pub const LARGEST: u64 = 5 * 1024 * 1024;

/// The most redirects followed.
const REDIRECTS: usize = 5;

/// How long fetching the page may take, all told.
const FETCH_TIME: Duration = Duration::from_secs(30);

/// How long the reader may take, fetching the page and reading it, before
/// Cox stops it.
pub const READ_TIME: Duration = Duration::from_mins(1);

/// The fewest characters a page's article says for it to be kept: one
/// that says fewer is drawn by its scripts, which the reader does not
/// run.
const FEWEST: usize = 200;

/// A page as it was read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    /// Where it was read, past the redirects.
    pub url: String,
    /// Its title, as the page names itself.
    pub title: String,
    /// Its article, as Markdown, without its title.
    pub text: String,
}

/// What the reader answers, a line of JSON on its standard output.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Read {
    Page(Page),
    Error(String),
}

/// The program run as the reader, unless [`read_with`] says another.
static PROGRAM: OnceLock<PathBuf> = OnceLock::new();

/// Runs `program` as the reader from now on, in place of this one, as a
/// test runs the `cox` it built.
pub fn read_with(program: PathBuf) {
    let _ = PROGRAM.set(program);
}

/// The URL `args`, the command line without the program's name, asks to
/// be read, as `cox --read-page <url>`.
#[must_use]
pub fn reading(args: &[String]) -> Option<&str> {
    match args {
        [option, url] if option == OPTION => Some(url),
        _ => None,
    }
}

/// Whether `from`, where a source was read, is a page Cox reads itself:
/// an `http` or `https` URL.
#[must_use]
pub fn is_page(from: &str) -> bool {
    url::Url::parse(from.trim()).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// Reads the page at `url` in a reader of its own, as `cox --read-page
/// <url>`, stopping it past [`READ_TIME`].
///
/// # Errors
///
/// When the page cannot be fetched or read, as the reader says, or the
/// reader does not answer in time.
pub async fn read(url: &str) -> Result<Page, String> {
    let program = match PROGRAM.get() {
        Some(program) => program.clone(),
        None => std::env::current_exe()
            .map_err(|error| format!("cannot find Cox's program to read {url}: {error}"))?,
    };
    let reader = tokio::process::Command::new(program)
        .arg(OPTION)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("cannot start the reader of {url}: {error}"))?;
    // Past the time, the reader is dropped, which stops it.
    let read = tokio::time::timeout(READ_TIME, reader.wait_with_output())
        .await
        .map_err(|_| {
            format!(
                "{url} was not read in {} seconds, and Cox stopped reading it",
                READ_TIME.as_secs()
            )
        })?
        .map_err(|error| format!("the reader of {url} failed: {error}"))?;
    let said = String::from_utf8_lossy(&read.stdout);
    match serde_json::from_str(said.trim()) {
        Ok(Read::Page(page)) => Ok(page),
        Ok(Read::Error(error)) => Err(error),
        Err(_) => Err(format!(
            "the reader of {url} stopped without a page ({})",
            read.status
        )),
    }
}

/// Reads the page at `url`, as the reader, and says what it read, or why
/// not, as a line of JSON on standard output.
#[must_use]
pub fn run(url: &str) -> ExitCode {
    let read = match fetch(url).and_then(|fetched| page(&fetched)) {
        Ok(page) => Read::Page(page),
        Err(error) => Read::Error(error),
    };
    let failed = matches!(read, Read::Error(_));
    match serde_json::to_string(&read) {
        Ok(line) => {
            use std::io::Write as _;
            // Cox may have stopped waiting for it.
            let _ = writeln!(std::io::stdout(), "{line}");
        }
        Err(error) => eprintln!("cox: {error}"),
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// A page as it was fetched.
struct Fetched {
    /// Where it was fetched, past the redirects.
    url: String,
    /// Its media type, as `text/html`, lower case.
    kind: String,
    /// What it says it is written in, if it says.
    charset: Option<String>,
    body: Vec<u8>,
}

/// Fetches the page at `url`, following its redirects, from the web
/// alone.
fn fetch(url: &str) -> Result<Fetched, String> {
    let proxy = ureq::Proxy::try_from_env();
    let config = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(FETCH_TIME))
        .proxy(proxy.clone())
        .user_agent(concat!(
            "Mozilla/5.0 (compatible; Cox/",
            env!("CARGO_PKG_VERSION"),
            ")"
        ))
        .build();
    let agent = ureq::Agent::with_parts(
        config,
        DefaultConnector::new(),
        OfTheWeb {
            proxy: proxy.as_ref().map(|proxy| proxy.host().to_owned()),
        },
    );
    let mut url =
        url::Url::parse(url.trim()).map_err(|error| format!("{url} is no URL: {error}"))?;
    for _ in 0..=REDIRECTS {
        of_the_web(&url)?;
        let response = agent
            .get(url.as_str())
            .header(
                "Accept",
                "text/html,application/xhtml+xml,text/markdown;q=0.9,text/plain;q=0.8",
            )
            .call()
            .map_err(|error| format!("cannot fetch {url}: {error}"))?;
        let status = response.status();
        if status.is_redirection() {
            let to = response
                .headers()
                .get("location")
                .and_then(|to| to.to_str().ok())
                .ok_or_else(|| format!("{url} redirects nowhere ({status})"))?;
            url = url
                .join(to)
                .map_err(|error| format!("{url} redirects to {to}, which is no URL: {error}"))?;
            continue;
        }
        if !status.is_success() {
            return Err(format!("{url} answered {status}"));
        }
        let mut body = response.into_body();
        let kind = body.mime_type().unwrap_or("").to_ascii_lowercase();
        let charset = body.charset().map(str::to_owned);
        let body =
            body.with_config()
                .limit(LARGEST)
                .read_to_vec()
                .map_err(|error| match error {
                    ureq::Error::BodyExceedsLimit(_) => format!(
                        "{url} is larger than {} MB, which Cox does not read",
                        LARGEST / 1024 / 1024
                    ),
                    error => format!("cannot read {url}: {error}"),
                })?;
        return Ok(Fetched {
            url: url.into(),
            kind,
            charset,
            body,
        });
    }
    Err(format!("{url} redirects more than {REDIRECTS} times"))
}

/// Refuses `url` unless it is of the web: `http` or `https`, and not
/// named as the machine itself or by an address that is not the web's.
/// A host named otherwise is refused as it resolves, by [`OfTheWeb`].
fn of_the_web(url: &url::Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "{url} is not http or https, which Cox does not read"
        ));
    }
    let not = || {
        format!(
            "{url} is not on the web, but on this machine or its network, which Cox does not read"
        )
    };
    match url.host() {
        Some(url::Host::Ipv4(ip)) if !public(IpAddr::V4(ip)) => Err(not()),
        Some(url::Host::Ipv6(ip)) if !public(IpAddr::V6(ip)) => Err(not()),
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            if name == "localhost" || name.ends_with(".localhost") {
                Err(not())
            } else {
                Ok(())
            }
        }
        Some(_) => Ok(()),
        None => Err(format!("{url} names no host")),
    }
}

/// Whether `ip` is an address of the web, and not of the machine itself,
/// of the networks it is in, or of none.
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let [first, second, ..] = ip.segments();
            // NAT64, `64:ff9b::/96`, carries an IPv4 address at its end.
            if ip.segments()[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [.., a, b, c, d] = ip.octets();
                return public_v4(Ipv4Addr::new(a, b, c, d));
            }
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                // Unique local, `fc00::/7`.
                || first & 0xfe00 == 0xfc00
                // Link-local, `fe80::/10`.
                || first & 0xffc0 == 0xfe80
                // Site-local, as was, `fec0::/10`.
                || first & 0xffc0 == 0xfec0
                // IPv4-compatible, as was, `::/96`.
                || ip.segments()[..6] == [0; 6]
                // Documentation, `2001:db8::/32`.
                || (first == 0x2001 && second == 0x0db8))
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_unspecified()
        || a == 0
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // Shared, as carriers' NAT uses, `100.64.0.0/10`.
        || (a == 100 && b & 0xc0 == 64)
        // Benchmarking, `198.18.0.0/15`.
        || (a == 198 && b & 0xfe == 18)
        // The IETF's, `192.0.0.0/24`.
        || (a == 192 && b == 0 && ip.octets()[2] == 0)
        // Reserved, `240.0.0.0/4`.
        || a >= 240)
}

/// Resolves the hosts Cox connects to as the system does, keeping the
/// addresses of the web alone, so that a name that resolves to the
/// machine's own or its network's is refused where it is connected to,
/// whatever it resolved to before. The proxy, which resolves the hosts
/// it connects to itself, is the human's, wherever it is.
#[derive(Debug)]
struct OfTheWeb {
    proxy: Option<String>,
}

impl Resolver for OfTheWeb {
    fn resolve(
        &self,
        uri: &Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let resolved = DefaultResolver::default().resolve(uri, config, timeout)?;
        if self.proxy.is_some() && uri.host() == self.proxy.as_deref() {
            return Ok(resolved);
        }
        let mut kept = self.empty();
        for address in resolved.iter().filter(|address| public(address.ip())) {
            kept.push(*address);
        }
        if kept.is_empty() {
            return Err(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "{} is not on the web, but on this machine or its network, which Cox does not read",
                    uri.host().unwrap_or("the host")
                ),
            )));
        }
        Ok(kept)
    }
}

/// The page `fetched` holds, its article read out of its HTML, or its
/// text as it is.
fn page(fetched: &Fetched) -> Result<Page, String> {
    let url = &fetched.url;
    let text = decoded(&fetched.body, fetched.charset.as_deref());
    let html = match fetched.kind.as_str() {
        "text/html" | "application/xhtml+xml" => true,
        "text/plain" | "text/markdown" | "text/x-markdown" => false,
        "" => text.trim_start().starts_with('<'),
        kind => {
            return Err(format!(
                "{url} is {kind}, not a page Cox reads: it reads HTML and text"
            ));
        }
    };
    let (title, text) = if html {
        article(&text, url).unwrap_or_default()
    } else {
        let title = text
            .lines()
            .find_map(|line| line.strip_prefix("# "))
            .map_or_else(|| last_part(url), |title| title.trim().to_owned());
        (title, text.trim().to_owned())
    };
    let said = text.chars().filter(|c| !c.is_whitespace()).count();
    if said < FEWEST {
        return Err(format!(
            "Cox found no article in {url} ({said} characters), as on a page its scripts \
             draw, which Cox does not run: read it on the stage with the browser's tools, \
             and keep what it says as `text`"
        ));
    }
    Ok(Page {
        url: url.clone(),
        title,
        text,
    })
}

/// `body` as text, in the charset it says, or its HTML's `<meta>` says,
/// or else UTF-8.
fn decoded(body: &[u8], charset: Option<&str>) -> String {
    let named = charset.or_else(|| meta_charset(body));
    let encoding = named
        .and_then(|name| encoding_rs::Encoding::for_label(name.trim().as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(body);
    text.into_owned()
}

/// The charset the `<meta>` at the top of the HTML `body` names, as
/// `<meta charset="shift_jis">` or `content="text/html; charset=euc-jp"`.
fn meta_charset(body: &[u8]) -> Option<&str> {
    let head = &body[..body.len().min(2048)];
    let at = head
        .windows(8)
        .position(|window| window.eq_ignore_ascii_case(b"charset="))?;
    let rest = &head[at + 8..];
    let rest = rest
        .strip_prefix(b"\"")
        .or_else(|| rest.strip_prefix(b"'"))
        .unwrap_or(rest);
    let end = rest
        .iter()
        .position(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
        })
        .unwrap_or(rest.len());
    std::str::from_utf8(&rest[..end])
        .ok()
        .filter(|name| !name.is_empty())
}

/// The title and the article of the page at `url`, whose HTML is `html`,
/// as Markdown, its links made whole from `url`, if Readability finds
/// one.
fn article(html: &str, url: &str) -> Option<(String, String)> {
    let config = dom_smoothie::Config {
        text_mode: dom_smoothie::TextMode::Markdown,
        ..Default::default()
    };
    let document = dom_query::Document::from(html);
    let article = dom_smoothie::Readability::with_document(document, Some(url), Some(config))
        .and_then(|mut readability| readability.parse())
        .ok()?;
    let title = article.title.trim();
    let title = if title.is_empty() {
        last_part(url)
    } else {
        title.to_owned()
    };
    Some((title, unescaped_links(article.text_content.trim())))
}

/// `markdown` with the `&amp;` of its links' URLs as `&`, as the
/// Markdown is written with its HTML's attributes as they were escaped.
fn unescaped_links(markdown: &str) -> String {
    let mut unescaped = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(at) = rest.find("](") {
        let (before, link) = rest.split_at(at + 2);
        unescaped.push_str(before);
        let end = link.find([')', '\n']).unwrap_or(link.len());
        unescaped.push_str(&link[..end].replace("&amp;", "&"));
        rest = &link[end..];
    }
    unescaped.push_str(rest);
    unescaped
}

/// The last part of the path of `url`, or its host, to name a page that
/// names itself nothing.
fn last_part(url: &str) -> String {
    let parsed = url::Url::parse(url).ok();
    parsed
        .as_ref()
        .and_then(|url| url.path_segments()?.rev().find(|part| !part.is_empty()))
        .map(str::to_owned)
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|url| url.host_str().map(str::to_owned))
        })
        .unwrap_or_else(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_web_is_read() {
        let refused = |url: &str| of_the_web(&url::Url::parse(url).unwrap()).is_err();
        assert!(!refused("https://blog.rust-lang.org/"));
        assert!(!refused("http://93.184.215.14/page"));
        assert!(!refused("http://[2606:2800:21f:cb07:6820:80da:af6b:8b2c]/"));
        for url in [
            "file:///etc/passwd",
            "ftp://example.com/",
            "http://localhost:8080/",
            "http://LOCALHOST./",
            "http://admin.localhost/",
            "http://127.0.0.1/",
            "http://0.0.0.0/",
            "http://10.0.0.1/",
            "http://172.16.5.4/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/",
            "http://[::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:192.168.1.1]/",
            "http://[64:ff9b::a9fe:a9fe]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            // As a browser reads them, which `url` does too.
            "http://2130706433/",
            "http://0x7f.1/",
        ] {
            assert!(refused(url), "{url} is refused");
        }
    }

    #[test]
    fn a_page_is_read_as_its_article() {
        let html = format!(
            "<html><head><title>Announcing Rust 1.92.0</title></head><body>\
             <nav><a href=\"/\">Home</a> <a href=\"/blog\">Blog</a></nav>\
             <article><h1>Announcing Rust 1.92.0</h1>{}\
             <p>See <a href=\"notes?a=1&amp;b=2\">the notes</a>.</p></article>\
             <footer>Menu</footer></body></html>",
            "<p>The Rust team is happy to announce a new version of Rust, 1.92.0. \
             Rust is a programming language empowering everyone to build reliable \
             and efficient software.</p>"
                .repeat(3)
        );
        let fetched = Fetched {
            url: "https://blog.rust-lang.org/2026/11/06/Rust-1.92.0/".to_owned(),
            kind: "text/html".to_owned(),
            charset: None,
            body: html.into_bytes(),
        };
        let page = page(&fetched).unwrap();
        assert_eq!(page.title, "Announcing Rust 1.92.0");
        assert!(page.text.contains("The Rust team is happy to announce"));
        assert!(
            page.text.contains(
                "[the notes](https://blog.rust-lang.org/2026/11/06/Rust-1.92.0/notes?a=1&b=2)"
            ),
            "{}",
            page.text
        );
        assert!(!page.text.contains("Home"), "{}", page.text);
    }

    #[test]
    fn a_page_drawn_by_its_scripts_is_not_kept() {
        let fetched = Fetched {
            url: "https://app.example.com/".to_owned(),
            kind: "text/html".to_owned(),
            charset: None,
            body: b"<html><head><title>App</title></head><body><div id=root></div>\
                    <script src=app.js></script></body></html>"
                .to_vec(),
        };
        let error = page(&fetched).unwrap_err();
        assert!(
            error.starts_with("Cox found no article in https://app.example.com/ (0 characters)"),
            "{error}"
        );
    }

    #[test]
    fn text_is_kept_as_it_is_and_other_kinds_are_not_read() {
        let text = format!(
            "# Notes\n\n{}",
            "Plain text, as a release note is. ".repeat(10)
        );
        let fetched = Fetched {
            url: "https://example.com/NOTES.md".to_owned(),
            kind: "text/markdown".to_owned(),
            charset: None,
            body: text.clone().into_bytes(),
        };
        let page = page(&fetched).unwrap();
        assert_eq!(page.title, "Notes");
        assert_eq!(page.text, text.trim());
        let pdf = Fetched {
            kind: "application/pdf".to_owned(),
            ..fetched
        };
        assert!(super::page(&pdf).unwrap_err().contains("application/pdf"));
    }

    #[test]
    fn a_page_is_read_in_the_charset_it_names() {
        let (body, _, _) = encoding_rs::SHIFT_JIS.encode("<p>日本語の本文</p>");
        let mut page = b"<html><head><meta charset=\"Shift_JIS\"></head><body>".to_vec();
        page.extend_from_slice(&body);
        assert!(decoded(&page, None).contains("日本語の本文"));
        assert!(decoded(&body, Some("shift_jis")).contains("日本語の本文"));
    }

    #[test]
    fn the_reader_is_asked_for_with_the_page() {
        let args = |args: &[&str]| args.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(
            reading(&args(&["--read-page", "https://example.com/"])),
            Some("https://example.com/")
        );
        assert_eq!(reading(&args(&["--read-page"])), None);
        assert!(is_page("https://example.com/a"));
        assert!(!is_page("cargo xtask ci"));
        assert!(!is_page("src/main.rs"));
    }
}
