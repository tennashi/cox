//! The page's address line, as a browser's: what is typed in it is an
//! address to go to or words to search for (see [`Address`]). While the
//! human types, a list under it says which, first, then offers the pages
//! visited that fit (see [`super::history`]); Up and Down, or Control-N
//! and Control-P, choose a row for Enter, and a click opens one. Typed at
//! the end of the line, the start of a page's address is filled in to
//! its end, selected, so that typing goes on over it, Backspace takes it
//! out, and Right or End keeps it. The line gets the keyboard with its
//! whole text selected, so that typing replaces it, and Escape puts the
//! page's address back, closing the list, and gives the keyboard back to
//! the page.

use std::net::{Ipv4Addr, Ipv6Addr};

use gpui::{
    App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding, MouseButton,
    Stateful, Subscription, Window, actions, deferred, div, prelude::*, px, relative, rgb,
};

use super::ADDRESS;
use super::history::{self, Visited};
use crate::daemon::phone::relay::on_the_mac;
use crate::settings::SearchEngine;
use crate::ui::ellipsis::ellipsis;
use crate::ui::input::{self, LineInput, Submitted};
use crate::vim::hints::Hinted as _;
use crate::vim::{self, Leave, Region};
use coxd::theme::{BORDER_STRONG, DIM, FAINT, FOREGROUND, RAISED, SELECTED};

actions!(address, [Revert, Next, Previous]);

/// Binds Escape, Control-N and Control-P in the address line. Call once
/// when the app starts.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Revert, Some(ADDRESS)),
        KeyBinding::new("ctrl-n", Next, Some(ADDRESS)),
        KeyBinding::new("ctrl-p", Previous, Some(ADDRESS)),
    ]);
}

/// The most pages visited the list under the line offers.
const OFFERED: usize = 6;

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
/// server on the Mac or the local network, as Chrome takes an address, a
/// name of one word or a `.local` one, over `https://` for any other.
fn on_the_web(typed: &str) -> String {
    let http = format!("http://{typed}");
    if served_on_the_mac(&http) || on_the_local_network(&http) {
        http
    } else {
        format!("https://{typed}")
    }
}

/// Whether `http` is of a host on the local network: an address, a name
/// of one word, or one that ends in `.local`.
fn on_the_local_network(http: &str) -> bool {
    url::Url::parse(http).is_ok_and(|url| match url.host() {
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => true,
        Some(url::Host::Domain(name)) => name
            .rsplit('.')
            .next()
            .is_none_or(|last| !name.contains('.') || last == "local"),
        None => false,
    })
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

/// What the human asks of the page from its address line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// Load this URL.
    Go(String),
    /// Take the keyboard back, with nothing loaded.
    Back,
}

/// The address line of a page.
pub struct AddressLine {
    input: Entity<LineInput>,
    /// The page's address, as the browser last said.
    url: String,
    /// What words typed are searched with, as the settings said when the
    /// line last got the keyboard.
    engine: SearchEngine,
    /// Whether the mouse button went down on the line while it did not
    /// have the keyboard, which that click brings with the whole text
    /// selected.
    clicked_from_elsewhere: bool,
    offer: Offer,
    _submitted: Subscription,
    _typed: Subscription,
}

/// What the list under the line offers for what is typed, and what is
/// filled in after it.
#[derive(Default)]
struct Offer {
    /// The line as last seen, to tell when it changes.
    seen: String,
    /// What of the line was typed, without what was filled in after it,
    /// to tell typing on from taking back.
    typed: String,
    /// What was filled in, which Enter opens while the line is so.
    filled: Option<Filled>,
    /// The pages visited that fit what is typed, the best first.
    pages: Vec<Visited>,
    chosen: Row,
}

/// The line as it was filled in, from the page at `url`.
struct Filled {
    line: String,
    url: String,
}

/// A row of the list under the line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Row {
    /// What Enter does with what is typed.
    #[default]
    Line,
    /// The page at this place among those offered, from 0.
    Page(usize),
}

impl Row {
    /// The row at `place` from the top of the list.
    const fn at(place: usize) -> Self {
        match place.checked_sub(1) {
            Some(page) => Self::Page(page),
            None => Self::Line,
        }
    }

    /// The row's place from the top of the list.
    const fn place(self) -> usize {
        match self {
            Self::Line => 0,
            Self::Page(page) => page + 1,
        }
    }
}

impl EventEmitter<Asked> for AddressLine {}

impl AddressLine {
    /// A line showing `url`, the page's address.
    pub fn new(url: &str, cx: &mut Context<Self>) -> Self {
        let input = cx.new(LineInput::new);
        input.update(cx, |input, cx| input.set_text(url, cx));
        let submitted = cx.subscribe(&input, |line, _, Submitted, cx| line.submitted(cx));
        let typed = cx.observe(&input, |line, _, cx| line.typed(cx));
        Self {
            input,
            url: url.to_owned(),
            engine: crate::settings::search(),
            clicked_from_elsewhere: false,
            offer: Offer {
                seen: url.to_owned(),
                ..Offer::default()
            },
            _submitted: submitted,
            _typed: typed,
        }
    }

    /// Gives the line the keyboard, with its whole text selected, so that
    /// typing replaces it.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.engine = crate::settings::search();
        window.focus(&self.input.focus_handle(cx));
        self.input.update(cx, LineInput::select_whole);
        vim::typing(Region::Address, window, cx);
    }

    /// What is typed in the line.
    #[must_use]
    pub fn text<'a>(&self, cx: &'a App) -> &'a str {
        self.input.read(cx).text()
    }

    /// What Enter does with what is typed, while the human types: while
    /// the line has the keyboard and holds something else than the page's
    /// address.
    #[must_use]
    pub fn said(&self, window: &Window, cx: &App) -> Option<String> {
        if !self.is_typed_in(window, cx) {
            return None;
        }
        if let Some(url) = self.filled_url(cx) {
            return Some(format!("Go to {url}"));
        }
        Some(Address::of(self.text(cx))?.said(&self.engine))
    }

    /// The URLs of the pages visited the list offers under the line, the
    /// best first, while the human types.
    #[must_use]
    pub fn offered(&self, window: &Window, cx: &App) -> Vec<String> {
        if !self.is_typed_in(window, cx) {
            return Vec::new();
        }
        self.offer
            .pages
            .iter()
            .map(|page| page.url.clone())
            .collect()
    }

    /// The row of the list chosen for Enter.
    #[must_use]
    pub fn chosen(&self) -> Row {
        self.offer.chosen
    }

    /// Whether the line has the keyboard and holds something else than
    /// the page's address.
    fn is_typed_in(&self, window: &Window, cx: &App) -> bool {
        self.input.focus_handle(cx).is_focused(window) && self.text(cx) != self.url
    }

    /// The URL of the page the line was filled in from, while it is so.
    fn filled_url<'a>(&'a self, cx: &App) -> Option<&'a str> {
        let filled = self.offer.filled.as_ref()?;
        (filled.line == self.text(cx)).then_some(filled.url.as_str())
    }

    /// The line changed, or its cursor moved: the list offers the pages
    /// that fit it now, and an address typed on at the end of the line is
    /// filled in. Nothing is filled in after taking back, as with
    /// Backspace, so that the fill goes and stays gone. What was filled in
    /// counts as typed once it is no longer selected, as after Right.
    fn typed(&mut self, cx: &mut Context<Self>) {
        let input = self.input.read(cx);
        let text = input.text().to_owned();
        if text == self.offer.seen {
            if self.filled_url(cx).is_some() && input.selected().is_empty() {
                self.offer.typed = text;
            }
            return;
        }
        let at_end = input.cursor() == text.len() && !input.is_composing();
        let took_back = self.offer.typed.starts_with(&text) && self.offer.seen != self.url;
        text.clone_into(&mut self.offer.seen);
        text.clone_into(&mut self.offer.typed);
        self.offer.chosen = Row::Line;
        if self
            .offer
            .filled
            .as_ref()
            .is_some_and(|filled| filled.line != text)
        {
            self.offer.filled = None;
        }
        if text == self.url {
            self.offer.pages.clear();
            cx.notify();
            return;
        }
        let now = history::now();
        let history = history::history(cx);
        let fill = (at_end && !took_back)
            .then(|| history.fill(&text, now))
            .flatten();
        let mut pages: Vec<Visited> = history
            .offered(&text, now, OFFERED + 1)
            .into_iter()
            .filter(|page| fill.as_ref().is_none_or(|fill| fill.url != page.url))
            .take(OFFERED)
            .cloned()
            .collect();
        std::mem::swap(&mut self.offer.pages, &mut pages);
        if let Some(fill) = fill {
            let line = format!("{text}{}", fill.rest);
            line.clone_into(&mut self.offer.seen);
            self.offer.filled = Some(Filled {
                line: line.clone(),
                url: fill.url,
            });
            self.input.update(cx, |input, cx| {
                input.set_text_selected(&line, text.len()..line.len(), cx);
            });
        }
        cx.notify();
    }

    /// Up or Down, Control-P or Control-N, by `step`: chooses the row
    /// above or below, round from one end to the other, while the list
    /// is open. Answers whether the list took the key.
    fn step(&mut self, step: isize, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.is_typed_in(window, cx) {
            return false;
        }
        let rows = self.offer.pages.len() + 1;
        let place = self.offer.chosen.place().checked_add_signed(step);
        self.offer.chosen = Row::at(place.unwrap_or(rows - 1) % rows);
        cx.notify();
        true
    }

    /// Where Enter goes: the page chosen in the list, the page the line
    /// was filled in from, or what the line says.
    fn going(&self, cx: &App) -> Option<String> {
        if let Row::Page(page) = self.offer.chosen {
            return self.offer.pages.get(page).map(|page| page.url.clone());
        }
        if let Some(url) = self.filled_url(cx) {
            return Some(url.to_owned());
        }
        Some(Address::of(self.text(cx))?.url(&self.engine))
    }

    /// The page went to `url`: the line shows it, unless the human is in
    /// the middle of typing in it.
    pub fn navigated(&mut self, url: &str, window: &Window, cx: &mut Context<Self>) {
        let typing =
            self.input.focus_handle(cx).is_focused(window) && self.text(cx) != self.url.as_str();
        url.clone_into(&mut self.url);
        if !typing {
            self.show_url(cx);
        }
    }

    /// The page is loading `url`, as the human or Cox asked.
    pub fn went(&mut self, url: &str, cx: &mut Context<Self>) {
        url.clone_into(&mut self.url);
        self.show_url(cx);
    }

    fn show_url(&mut self, cx: &mut Context<Self>) {
        self.offer.chosen = Row::Line;
        self.offer.filled = None;
        if self.text(cx) != self.url.as_str() {
            let url = self.url.clone();
            self.input.update(cx, |input, cx| input.set_text(&url, cx));
        }
        cx.notify();
    }

    fn submitted(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = self.going(cx) {
            cx.emit(Asked::Go(url));
        }
    }

    /// Escape: the page's address comes back, and the keyboard goes back
    /// to the page.
    fn revert(&mut self, cx: &mut Context<Self>) {
        self.show_url(cx);
        cx.emit(Asked::Back);
    }

    /// The list under the line: what Enter does with it, then the pages
    /// visited that fit it, the row chosen marked.
    fn render_offer(&self, said: String, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = self.offer.chosen;
        let pages = self.offer.pages.iter().enumerate().map(|(place, page)| {
            let url = page.url.clone();
            let title = (!page.title.is_empty()).then(|| {
                div()
                    .flex_shrink_0()
                    .max_w(relative(0.6))
                    .overflow_hidden()
                    .text_color(rgb(FOREGROUND))
                    .child(ellipsis(page.title.clone()))
            });
            Self::render_row(Row::Page(place), chosen)
                .children(title)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_color(rgb(DIM))
                        .child(ellipsis(page.url.clone())),
                )
                .on_hint_click(cx.listener(move |_, _, _, cx| cx.emit(Asked::Go(url.clone()))))
                .into_any_element()
        });
        let said = Self::render_row(Row::Line, chosen)
            .text_color(rgb(FAINT))
            .child(div().min_w_0().overflow_hidden().child(ellipsis(said)))
            .on_hint_click(cx.listener(|line, _, _, cx| line.submitted(cx)))
            .into_any_element();
        deferred(
            div()
                .id("address-offer")
                .occlude()
                .child(crate::vim::hints::cover())
                .absolute()
                .top(relative(1.))
                .left_0()
                .right_0()
                .mt(px(4.))
                .p(px(4.))
                .bg(rgb(RAISED))
                .border_1()
                .border_color(rgb(BORDER_STRONG))
                .rounded(px(10.))
                .shadow_lg()
                .flex()
                .flex_col()
                .child(said)
                .children(pages),
        )
        .with_priority(1)
    }

    /// The row `row` of the list, marked if it is the one `chosen`.
    fn render_row(row: Row, chosen: Row) -> Stateful<Div> {
        div()
            .id(("address-row", row.place()))
            .debug_selector(move || match row {
                Row::Line => "address-says".to_owned(),
                Row::Page(page) => format!("address-page-{page}"),
            })
            .flex_shrink_0()
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(6.))
            .text_size(px(12.))
            .cursor_pointer()
            .when(row == chosen, |row| row.bg(rgb(SELECTED)))
            .hover(|style| style.bg(rgb(SELECTED)))
    }
}

impl Focusable for AddressLine {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for AddressLine {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "address".to_owned())
            .key_context(ADDRESS)
            .relative()
            .flex_1()
            .px_2()
            .rounded_sm()
            .bg(rgb(SELECTED))
            .on_action(cx.listener(|line, _: &Revert, _, cx| line.revert(cx)))
            .on_action(cx.listener(|line, _: &Leave, _, cx| line.revert(cx)))
            .capture_action(cx.listener(|line, _: &input::Up, window, cx| {
                if line.step(-1, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|line, _: &input::Down, window, cx| {
                if line.step(1, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|line, _: &Previous, window, cx| {
                line.step(-1, window, cx);
            }))
            .on_action(cx.listener(|line, _: &Next, window, cx| {
                line.step(1, window, cx);
            }))
            .capture_any_mouse_down(cx.listener(|line, _, window, cx| {
                line.clicked_from_elsewhere = !line.input.focus_handle(cx).is_focused(window);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|line, _, window, cx| {
                    if std::mem::take(&mut line.clicked_from_elsewhere) {
                        line.focus(window, cx);
                    }
                }),
            )
            .child(self.input.clone())
            .children(
                self.said(window, cx)
                    .map(|said| self.render_offer(said, cx).into_any_element()),
            )
    }
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
            ("foo.local", Some(url("http://foo.local"))),
            ("日本.jp", Some(url("https://日本.jp"))),
            ("myhost:8080/x", Some(url("http://myhost:8080/x"))),
            ("192.168.1.1:8080", Some(url("http://192.168.1.1:8080"))),
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
