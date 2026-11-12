//! The page's address line, as a browser's: what is typed in it is an
//! address to go to or words to search for (see [`Address`]).

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::daemon::phone::relay::on_the_mac;
use crate::settings::SearchEngine;

/// What a line typed in a browser's address line asks for, as Chrome
/// sorts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// A page, at this URL.
    Url(String),
    /// A search for these words.
    Search(String),
}

/// The schemes taken as given before a `:`. Any other word before one is
/// a host with a port, as `localhost:3000`, or else words.
const SCHEMES: [&str; 11] = [
    "about",
    "blob",
    "chrome",
    "cox",
    "data",
    "devtools",
    "file",
    "http",
    "https",
    "mailto",
    "view-source",
];

impl Address {
    /// What `typed` asks for: a URL when it has a scheme of
    /// [`SCHEMES`], or starts with a host, as `example.com`, `node.js`,
    /// `localhost`, `myhost:8080`, `192.168.1.1` or `[::1]:3000`, with
    /// no space before its path; else words to search for, as `gpui`,
    /// `1.5` or `rust gpui`. A `?` first makes the rest words, whatever
    /// they are. A host on the Mac opens over `http://`, any other over
    /// `https://`, and a space in a URL is `%20`. Blank is nothing.
    #[must_use]
    pub fn of(typed: &str) -> Option<Self> {
        let typed = typed.trim();
        if let Some(words) = typed.strip_prefix('?') {
            let words = words.trim();
            return (!words.is_empty()).then(|| Self::Search(words.to_owned()));
        }
        if typed.is_empty() {
            return None;
        }
        let url = match typed.split_once(':') {
            Some((scheme, rest)) if SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme)) => {
                let with_host = ["http", "https"]
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case(scheme));
                let host = if with_host {
                    authority(rest.trim_start_matches('/'))
                } else {
                    ""
                };
                (!host.contains(char::is_whitespace)).then(|| typed.to_owned())
            }
            _ => is_host(authority(typed)).then(|| on_the_web(typed)),
        };
        Some(url.map_or_else(
            || Self::Search(typed.to_owned()),
            |url| Self::Url(url.replace(' ', "%20")),
        ))
    }

    /// The URL to load: the page's, or the results of the search with
    /// `engine`.
    #[must_use]
    pub fn url(&self, engine: &SearchEngine) -> String {
        match self {
            Self::Url(url) => url.clone(),
            Self::Search(words) => engine.url(words),
        }
    }

    /// What Enter does with it, searching with `engine`.
    #[must_use]
    pub fn said(&self, engine: &SearchEngine) -> String {
        match self {
            Self::Url(url) => format!("Go to {url}"),
            Self::Search(words) => format!("Search {} for {words}", engine.host()),
        }
    }
}

/// The part of `typed` before its path, query or fragment, where a host
/// and its port would be.
fn authority(typed: &str) -> &str {
    typed.split(['/', '?', '#']).next().unwrap_or(typed)
}

/// Whether `authority` is a host, with a port or not: an IPv6 address in
/// brackets, or names of letters, digits, `-` and `_` between dots, which
/// are an IPv4 address, `localhost` or one of its, or have a port, or
/// else are two or more, the last no number.
fn is_host(authority: &str) -> bool {
    if let Some(bracketed) = authority.strip_prefix('[') {
        return bracketed.split_once(']').is_some_and(|(ip, port)| {
            ip.parse::<Ipv6Addr>().is_ok()
                && (port.is_empty() || port.strip_prefix(':').is_some_and(is_port))
        });
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if port.is_some_and(|port| !is_port(port)) {
        return false;
    }
    let host = host.strip_suffix('.').unwrap_or(host);
    let labels: Vec<&str> = host.split('.').collect();
    let named = labels.iter().all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    });
    if !named {
        return false;
    }
    let localhost = labels
        .last()
        .is_some_and(|last| last.eq_ignore_ascii_case("localhost"));
    let numbered = labels
        .last()
        .is_some_and(|last| last.chars().all(|c| c.is_ascii_digit()));
    host.parse::<Ipv4Addr>().is_ok()
        || localhost
        || port.is_some()
        || (labels.len() > 1 && !numbered)
}

fn is_port(port: &str) -> bool {
    !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) && port.parse::<u16>().is_ok()
}

/// `typed`, a host and what follows it, as a URL: over `http://` for a
/// server on the Mac, over `https://` for any other.
fn on_the_web(typed: &str) -> String {
    let http = format!("http://{typed}");
    if served_on_the_mac(&http) {
        http
    } else {
        format!("https://{typed}")
    }
}

/// Whether `http` is a page of a server on the Mac: one the phone's relay
/// reaches, or one at `0.0.0.0` or `[::]`, every address of the Mac, where
/// a server that listens on all of them says it is.
fn served_on_the_mac(http: &str) -> bool {
    let every_address = url::Url::parse(http).is_ok_and(|url| match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_unspecified(),
        Some(url::Host::Ipv6(ip)) => ip.is_unspecified(),
        _ => false,
    });
    every_address || on_the_mac(http).is_some()
}

#[cfg(test)]
mod tests {
    use super::Address;

    fn url(typed: &str) -> Address {
        Address::Url(typed.to_owned())
    }

    fn search(words: &str) -> Address {
        Address::Search(words.to_owned())
    }

    #[test]
    fn a_line_is_an_address_or_words_as_chrome_sorts_it() {
        let table = [
            ("example.com", Some(url("https://example.com"))),
            (" example.com/path ", Some(url("https://example.com/path"))),
            ("example.com/a b", Some(url("https://example.com/a%20b"))),
            ("https://example.com/a", Some(url("https://example.com/a"))),
            ("HTTPS://example.com", Some(url("HTTPS://example.com"))),
            ("about:blank", Some(url("about:blank"))),
            (
                "file:///Users/me/a.html",
                Some(url("file:///Users/me/a.html")),
            ),
            ("cox://plugin/x", Some(url("cox://plugin/x"))),
            ("chrome://version", Some(url("chrome://version"))),
            (
                "localhost.example.com",
                Some(url("https://localhost.example.com")),
            ),
            ("node.js", Some(url("https://node.js"))),
            ("a.b", Some(url("https://a.b"))),
            ("foo.local", Some(url("https://foo.local"))),
            ("日本.jp", Some(url("https://日本.jp"))),
            ("myhost:8080/x", Some(url("https://myhost:8080/x"))),
            ("192.168.1.1:8080", Some(url("https://192.168.1.1:8080"))),
            ("example.com?q=a", Some(url("https://example.com?q=a"))),
            ("gpui", Some(search("gpui"))),
            ("rust gpui", Some(search("rust gpui"))),
            ("what is x?", Some(search("what is x?"))),
            ("what?", Some(search("what?"))),
            ("1.5", Some(search("1.5"))),
            ("v1.2.3", Some(search("v1.2.3"))),
            ("C++ tutorial", Some(search("C++ tutorial"))),
            ("C++", Some(search("C++"))),
            ("検索 する", Some(search("検索 する"))),
            ("検索", Some(search("検索"))),
            ("foo:bar", Some(search("foo:bar"))),
            ("me@example.com", Some(search("me@example.com"))),
            ("foo/bar", Some(search("foo/bar"))),
            ("https:// example.com", Some(search("https:// example.com"))),
            ("curl example.com/a", Some(search("curl example.com/a"))),
            ("?example.com", Some(search("example.com"))),
            ("? rust gpui", Some(search("rust gpui"))),
            ("?", None),
            ("  ", None),
        ];
        for (typed, expected) in table {
            assert_eq!(Address::of(typed), expected, "{typed}");
        }
    }

    #[test]
    fn a_host_of_the_mac_is_plain_http() {
        for typed in [
            "localhost",
            "localhost:3000",
            "localhost:5173/shop?item=2",
            "app.localhost:4000/",
            "127.0.0.1:8080",
            "127.1.2.3",
            "[::1]:3000/",
            "[::1]",
            "0.0.0.0:8000",
        ] {
            assert_eq!(
                Address::of(typed),
                Some(url(&format!("http://{typed}"))),
                "{typed}"
            );
        }
        assert_eq!(
            Address::of("http://localhost:3000/"),
            Some(url("http://localhost:3000/"))
        );
    }

    #[test]
    fn enter_goes_to_the_url_or_searches_the_words() {
        let engine = crate::settings::SearchEngine::default();
        let rust_gpui = Address::Search("rust gpui".to_owned());
        assert_eq!(rust_gpui.said(&engine), "Search google.com for rust gpui");
        assert!(rust_gpui.url(&engine).contains("q=rust+gpui"));
        let page = Address::Url("https://example.com".to_owned());
        assert_eq!(page.said(&engine), "Go to https://example.com");
        assert_eq!(page.url(&engine), "https://example.com");
    }
}
