//! `cox:` addresses: pages a stage serves itself, from what Cox knows,
//! for a page of the stage to show what a tab drawn by Cox would, such
//! as the map of the project, in HTML and JavaScript instead. The
//! stage gives each browser it opens what to [`Serve`] at a `cox:`
//! address, which answers each request of a page on a thread of its own,
//! and [`Served`] hands the answer to CEF as a response. CEF asks for the
//! response on its IO thread, which carries what the browser says to
//! every page, as the wheel's scrolling and the asks for pictures: so the
//! answer is not made there, where an answer that takes a while, as
//! what goes on in a folder with many pages of the wiki, would freeze the
//! page the human scrolls. The scheme is
//! registered as a standard, secure one that pages fetch from, so a
//! page at one `cox:` address reads JSON from another, as
//! `cox://map/` reads `cox://api/map`.
//!
//! Only a page at a `cox:` address reads what is served there, and only
//! what its scripts fetch has the stage do something, as run a trigger
//! (see [`Asker`]). A page anywhere else may not fetch a `cox:` address,
//! show it as a picture, or put it in a frame; it may send its tab
//! there, as Cox, the human or an agent does, whom CEF does not tell
//! apart from it, but the tab is only shown what is there.
//!
//! A `cox://open/<path>#L<line>` address is not served: a page going
//! there asks the stage to open the file on a tab of its own, as a link
//! to a `file:` address does (see [`crate::links`]), which a page that is
//! not itself a file may not link to.
#![expect(
    clippy::transmute_ptr_to_ptr,
    reason = "the cef wrap macros cast the handler reference CEF passes back"
)]

use std::os::raw::c_int;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use cef::rc::Rc as _;
use cef::sys::{cef_return_value_t, cef_scheme_options_t};
use cef::{
    Callback, CefString, ImplCallback, ImplRequest, ImplResourceHandler,
    ImplResourceRequestHandler, ImplResponse, ImplSchemeRegistrar, Request, ResourceHandler,
    ResourceReadCallback, ResourceRequestHandler, ResourceType, Response, ReturnValue,
    SchemeRegistrar, WrapResourceHandler, WrapResourceRequestHandler, wrap_resource_handler,
    wrap_resource_request_handler,
};

pub use crate::links::percent_decoded;

/// The scheme, as an address starts.
pub const SCHEME: &str = "cox";

/// What an address starts with to be served.
const PREFIX: &str = "cox://";

/// What a page gets at a `cox:` address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    /// The media type, as `text/html` or `application/json`.
    pub mime: String,
    pub body: Vec<u8>,
}

/// Who asks for a `cox:` address, as far as the stage can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asker {
    /// A script of a page at a `cox:` address, a plugin's, fetching it:
    /// the stage does what the address asks, as save a trigger.
    Plugin,
    /// Anyone else the stage answers: a page at a `cox:` address loading
    /// it otherwise, as a stylesheet or a picture, or a tab going to it,
    /// at the word of Cox, the human, an agent or a page elsewhere, whom
    /// CEF does not tell apart. It is told what is there, and nothing is
    /// done.
    Anyone,
}

/// What a browser's pages get at `cox:` addresses: given the address
/// and who asks for it, the resource there, or `None` for nothing there.
/// It is called on a thread of its own, for each request.
pub type Serve = Arc<dyn Fn(&str, Asker) -> Option<Resource> + Send + Sync>;

/// Registers the scheme with CEF, in each of its processes: standard, so
/// that addresses have a host and a path; secure, as `https:`; and open
/// to CORS and `fetch`, so that a page at one `cox:` address reads
/// another.
pub(crate) fn register(registrar: SchemeRegistrar) {
    let options = cef_scheme_options_t::CEF_SCHEME_OPTION_STANDARD as u32
        | cef_scheme_options_t::CEF_SCHEME_OPTION_SECURE as u32
        | cef_scheme_options_t::CEF_SCHEME_OPTION_CORS_ENABLED as u32
        | cef_scheme_options_t::CEF_SCHEME_OPTION_FETCH_ENABLED as u32;
    let options = c_int::try_from(options).unwrap_or(c_int::MAX);
    let registered = registrar.add_custom_scheme(Some(&CefString::from(SCHEME)), options);
    if registered == 1 {
        tracing::debug!("the {SCHEME}: scheme is registered");
    } else {
        tracing::error!("CEF did not register the {SCHEME}: scheme, so the stage serves no pages");
    }
}

/// Whether `url` is a `cox:` address, which a browser's [`Serve`] answers.
pub(crate) fn is_served(url: &str) -> bool {
    url.starts_with(PREFIX)
}

/// The file a `cox://open/<path>#L<lines>` address asks the stage to
/// open, as the `file:` address a link to a file is (see
/// [`crate::links::file_linked`]), or `None` when `url` is no such
/// address.
pub(crate) fn opened(url: &str) -> Option<String> {
    let path = url.strip_prefix(PREFIX)?.strip_prefix("open/")?;
    if path.is_empty() {
        return None;
    }
    Some(format!("file:///{path}"))
}

/// The chat a `cox://chat/<key>` address asks the window to show, by the
/// number the window goes by for it, or `None` when `url` is no such
/// address.
pub(crate) fn chat_shown(url: &str) -> Option<u64> {
    url.strip_prefix(PREFIX)?
        .strip_prefix("chat/")?
        .trim_end_matches('/')
        .parse()
        .ok()
}

/// Who asks for a `cox:` address in a request for `resource`, made by
/// the page whose origin is `initiator`, as `cox://map`; or `None` for a
/// request the stage does not answer: a page elsewhere fetching the
/// address, showing it as a picture, or putting it in a frame. CEF names
/// `null` as the origin both of a tab Cox, the human or an agent sends
/// there and of a `data:` or sandboxed page that does.
pub(crate) fn asker(initiator: &str, resource: ResourceType) -> Option<Asker> {
    if is_served(initiator) {
        Some(if resource == ResourceType::XHR {
            Asker::Plugin
        } else {
            Asker::Anyone
        })
    } else {
        (resource == ResourceType::MAIN_FRAME).then_some(Asker::Anyone)
    }
}

/// Answers a request for a `cox:` address, for `resource`, made by the
/// page whose origin is `initiator`, from `serve`, as [`asker`] lets
/// it; or `None` when it does not, which leaves the request to CEF, as
/// for an address of a scheme no one serves.
pub(crate) fn handler(
    serve: Serve,
    initiator: &str,
    resource: ResourceType,
) -> Option<ResourceRequestHandler> {
    let asker = asker(initiator, resource)?;
    let reader = is_served(initiator).then(|| Arc::from(initiator));
    Some(Serving::new(serve, asker, reader))
}

wrap_resource_request_handler! {
    struct Serving {
        serve: Serve,
        asker: Asker,
        reader: Option<Arc<str>>,
    }

    impl ResourceRequestHandler {
        /// Every request goes on to [`Self::resource_handler`]; left to
        /// CEF's default, it would be cancelled.
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            ReturnValue::from(cef_return_value_t::RV_CONTINUE)
        }

        fn resource_handler(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            request: Option<&mut Request>,
        ) -> Option<ResourceHandler> {
            let url = CefString::from(&request?.url()).to_string();
            Some(Served::new(
                self.serve.clone(),
                Arc::from(url),
                self.asker,
                self.reader.clone(),
                Arc::new(OnceLock::new()),
                Arc::new(AtomicUsize::new(0)),
            ))
        }
    }
}

// One response to `url`, for `asker`, which the page at `reader`, the
// origin of a `cox:` page, may read: the resource, or nothing there,
// once `serve` answered, read out as CEF asks for it, from `read` bytes
// in.
wrap_resource_handler! {
    struct Served {
        serve: Serve,
        url: Arc<str>,
        asker: Asker,
        reader: Option<Arc<str>>,
        answer: Arc<OnceLock<Option<Resource>>>,
        read: Arc<AtomicUsize>,
    }

    impl ResourceHandler {
        /// The response is ready once `serve` answered, on a thread of
        /// its own, which tells CEF so.
        fn open(
            &self,
            _request: Option<&mut Request>,
            handle_request: Option<&mut c_int>,
            callback: Option<&mut Callback>,
        ) -> c_int {
            let Some(handle_request) = handle_request else {
                return 0;
            };
            let Some(callback) = callback.map(|callback| callback.clone()) else {
                // CEF gives no way to say later: answered here.
                answer(&self.serve, &self.url, self.asker, &self.answer);
                *handle_request = 1;
                return 1;
            };
            *handle_request = 0;
            let (serve, url, told) = (self.serve.clone(), self.url.clone(), self.answer.clone());
            let asker = self.asker;
            let serving = std::thread::Builder::new()
                .name("cox-serve".to_owned())
                .spawn(move || {
                    answer(&serve, &url, asker, &told);
                    callback.cont();
                });
            if let Err(error) = serving {
                tracing::error!("cannot answer {}: {error}", self.url);
                // Cancelled at once.
                *handle_request = 1;
                return 0;
            }
            1
        }

        fn response_headers(
            &self,
            response: Option<&mut Response>,
            response_length: Option<&mut i64>,
            _redirect_url: Option<&mut CefString>,
        ) {
            let Some(response) = response else { return };
            let Some(resource) = self.answer.get().and_then(Option::as_ref) else {
                response.set_status(404);
                response.set_mime_type(Some(&CefString::from("text/plain")));
                if let Some(length) = response_length {
                    *length = 0;
                }
                return;
            };
            response.set_status(200);
            response.set_mime_type(Some(&CefString::from(resource.mime.as_str())));
            if resource.mime.starts_with("text/") || resource.mime.ends_with("json") {
                response.set_charset(Some(&CefString::from("utf-8")));
            }
            // A page at one `cox:` address reads another's.
            if let Some(reader) = &self.reader {
                response.set_header_by_name(
                    Some(&CefString::from("Access-Control-Allow-Origin")),
                    Some(&CefString::from(&**reader)),
                    1,
                );
            }
            if let Some(length) = response_length {
                *length = i64::try_from(resource.body.len()).unwrap_or(-1);
            }
        }

        /// Hands CEF the next `bytes_to_read` bytes of the body, or fewer
        /// at its end: 0 bytes read ends the response.
        fn read(
            &self,
            data_out: *mut u8,
            bytes_to_read: c_int,
            bytes_read: Option<&mut c_int>,
            _callback: Option<&mut ResourceReadCallback>,
        ) -> c_int {
            let copied = self.answer.get().and_then(Option::as_ref).map_or(0, |resource| {
                let from = self.read.load(Ordering::Acquire);
                let rest = &resource.body[from.min(resource.body.len())..];
                let count = rest.len().min(usize::try_from(bytes_to_read).unwrap_or(0));
                if count > 0 && !data_out.is_null() {
                    // SAFETY: CEF hands `data_out` as a buffer of
                    // `bytes_to_read` bytes to fill, and `count` is at most
                    // that; `rest` is a separate allocation.
                    unsafe { std::ptr::copy_nonoverlapping(rest.as_ptr(), data_out, count) };
                }
                self.read.store(from + count, Ordering::Release);
                count
            });
            if let Some(bytes_read) = bytes_read {
                *bytes_read = c_int::try_from(copied).unwrap_or(c_int::MAX);
            }
            c_int::from(copied > 0)
        }
    }
}

/// How long an answer takes before the log warns of it: a page that
/// reads it every two seconds, as the plugins do, waits that long each
/// time.
const SLOW: Duration = Duration::from_millis(100);

/// Has `serve` answer `url` for `asker`, keeping the answer in `told`,
/// and says in the log how long it took, so that what answers slowly is
/// found there.
fn answer(serve: &Serve, url: &str, asker: Asker, told: &OnceLock<Option<Resource>>) {
    let started = Instant::now();
    let resource = serve(url, asker);
    let took = started.elapsed();
    let took_ms = took.as_millis();
    if took >= SLOW {
        tracing::warn!(url, took_ms, "{url} took {took_ms} ms to answer");
    } else {
        tracing::debug!(url, took_ms, "answered {url}");
    }
    if resource.is_none() {
        tracing::debug!("nothing is served at {url}");
    }
    let _ = told.set(resource);
}

#[cfg(test)]
mod tests {
    use cef::ResourceType;

    use super::{Asker, asker, chat_shown, is_served, opened};

    #[test]
    fn only_a_cox_page_reads_and_only_its_scripts_have_things_done() {
        assert_eq!(asker("cox://map", ResourceType::XHR), Some(Asker::Plugin));
        assert_eq!(
            asker("cox://map", ResourceType::STYLESHEET),
            Some(Asker::Anyone)
        );
        assert_eq!(
            asker("cox://wiki", ResourceType::IMAGE),
            Some(Asker::Anyone)
        );
        assert_eq!(
            asker("cox://activity", ResourceType::MAIN_FRAME),
            Some(Asker::Anyone)
        );
        assert_eq!(asker("null", ResourceType::MAIN_FRAME), Some(Asker::Anyone));
        assert_eq!(
            asker("https://example.com", ResourceType::MAIN_FRAME),
            Some(Asker::Anyone)
        );
        assert_eq!(asker("https://example.com", ResourceType::XHR), None);
        assert_eq!(asker("http://127.0.0.1:8000", ResourceType::IMAGE), None);
        assert_eq!(asker("https://example.com", ResourceType::SUB_FRAME), None);
        assert_eq!(asker("null", ResourceType::XHR), None);
        assert_eq!(asker("null", ResourceType::SUB_FRAME), None);
        assert_eq!(asker("file://", ResourceType::XHR), None);
        assert_eq!(asker("", ResourceType::XHR), None);
    }

    #[test]
    fn cox_addresses_are_served() {
        assert!(is_served("cox://map/"));
        assert!(is_served("cox://api/map"));
        assert!(!is_served("https://example.com/cox://"));
        assert!(!is_served("file:///home/me/cox"));
    }

    #[test]
    fn an_open_address_names_a_file() {
        assert_eq!(
            opened("cox://open/home/me/src/main.rs#L12").as_deref(),
            Some("file:///home/me/src/main.rs#L12")
        );
        assert_eq!(opened("cox://open/"), None);
        assert_eq!(opened("cox://map/open/x"), None);
        assert_eq!(opened("file:///home/me/open/x"), None);
    }

    #[test]
    fn a_chat_address_names_a_chat() {
        assert_eq!(chat_shown("cox://chat/12"), Some(12));
        assert_eq!(chat_shown("cox://chat/12/"), Some(12));
        assert_eq!(chat_shown("cox://chat/"), None);
        assert_eq!(chat_shown("cox://chat/twelve"), None);
        assert_eq!(chat_shown("cox://open/12"), None);
    }
}
