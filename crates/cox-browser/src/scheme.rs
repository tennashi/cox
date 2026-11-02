//! `cox:` addresses: pages a stage serves itself, from what Cox knows,
//! for a page of the stage to show what a tab drawn by Cox would, such
//! as the parts a chat runs with, in HTML and JavaScript instead. The
//! stage gives each browser it opens what to [`Serve`] at a `cox:`
//! address; CEF asks it on its own thread for each request of a page,
//! and [`Served`] hands the answer to CEF as a response. The scheme is
//! registered as a standard, secure one that pages fetch from, so a
//! page at one `cox:` address reads JSON from another, as
//! `cox://parts/` reads `cox://api/parts`.
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
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cef::rc::Rc as _;
use cef::sys::{cef_return_value_t, cef_scheme_options_t};
use cef::{
    Callback, CefString, ImplRequest, ImplResourceHandler, ImplResourceRequestHandler,
    ImplResponse, ImplSchemeRegistrar, Request, ResourceHandler, ResourceReadCallback,
    ResourceRequestHandler, Response, ReturnValue, SchemeRegistrar, WrapResourceHandler,
    WrapResourceRequestHandler, wrap_resource_handler, wrap_resource_request_handler,
};

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

/// What a browser's pages get at `cox:` addresses: given the address,
/// the resource there, or `None` for nothing there. CEF calls it on a
/// thread of its own, for each request.
pub type Serve = Arc<dyn Fn(&str) -> Option<Resource> + Send + Sync>;

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

/// Answers the requests of one browser's pages at `cox:` addresses from
/// its [`Serve`].
pub(crate) fn handler(serve: Serve) -> ResourceRequestHandler {
    Serving::new(serve)
}

wrap_resource_request_handler! {
    struct Serving {
        serve: Serve,
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
            let resource = (self.serve)(&url);
            if resource.is_none() {
                tracing::debug!("nothing is served at {url}");
            }
            Some(Served::new(resource.map(Arc::new), Arc::new(AtomicUsize::new(0))))
        }
    }
}

// One response: the resource, or nothing there, read out as CEF asks for
// it, from `read` bytes in.
wrap_resource_handler! {
    struct Served {
        resource: Option<Arc<Resource>>,
        read: Arc<AtomicUsize>,
    }

    impl ResourceHandler {
        /// The response is ready at once.
        fn open(
            &self,
            _request: Option<&mut Request>,
            handle_request: Option<&mut c_int>,
            _callback: Option<&mut Callback>,
        ) -> c_int {
            if let Some(handle_request) = handle_request {
                *handle_request = 1;
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
            let Some(resource) = &self.resource else {
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
            response.set_header_by_name(
                Some(&CefString::from("Access-Control-Allow-Origin")),
                Some(&CefString::from("*")),
                1,
            );
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
            let copied = self.resource.as_ref().map_or(0, |resource| {
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

#[cfg(test)]
mod tests {
    use super::{is_served, opened};

    #[test]
    fn cox_addresses_are_served() {
        assert!(is_served("cox://parts/"));
        assert!(is_served("cox://api/parts"));
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
        assert_eq!(opened("cox://parts/open/x"), None);
        assert_eq!(opened("file:///home/me/open/x"), None);
    }
}
