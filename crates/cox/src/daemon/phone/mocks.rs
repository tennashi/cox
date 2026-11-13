//! The pages of the sketches, served on the Mac's own address for the
//! human to try a mock as it is, HTML and all, apart from Cox's own page.
//!
//! An agent makes a mock as a sketch from nothing, as plain HTML, for
//! the human to try what the work is to be before they agree on it. coxd
//! serves the sketches on the port two after the page's, as a dev server
//! serves a web app, and the phone reaches them as it reaches any page
//! served on the Mac, through the relay (see [`super::relay`]), which
//! passes on what the human's phone alone sends: so a mock, another
//! origin than Cox's page, reads nothing of it, and does nothing on it,
//! since what the page posts has to say it comes from the page itself
//! (see [`super::from_the_page`]).
//!
//! What a sketch's folder holds can be the project's code, so a request
//! is answered only as made to the Mac's own address, by its name, as a
//! page whose own name was turned to `127.0.0.1` does not make it.

use std::convert::Infallible;
use std::path::{Component, Path, PathBuf};

use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use tokio::net::TcpListener;

use super::{Body, TEXT, answer};
use crate::storage::{self, Kept};

/// The port the mocks are served on, for the page served on `page`: two
/// after it, the one after it being the relay's.
#[must_use]
pub fn port(page: u16) -> Option<u16> {
    page.checked_add(2)
}

/// The folder the sketches keep what they discarded in, which is no
/// sketch's.
const DISCARDED: &str = "discarded";

/// How many pages of a sketch the phone lists, at most.
const PAGES: usize = 40;

/// How deep in a sketch's folder its pages are looked for.
const DEEP: usize = 4;

/// The biggest file of a sketch served.
const MOST: u64 = 50 * 1024 * 1024;

/// The folders a sketch's pages are not looked for in, beside those
/// whose name starts with a dot.
const NOT_LOOKED_IN: &[&str] = &["node_modules", "target"];

/// The name of the sketch whose folder is `folder` and the paths of its
/// HTML pages there, the shallowest first, an `index.html` first of
/// those in a folder, unless `folder` is no sketch's, in `sketches`.
#[must_use]
pub fn pages(sketches: &Path, folder: &Path) -> Option<(String, Vec<String>)> {
    let sketches = sketches.canonicalize().ok()?;
    let folder = folder.canonicalize().ok()?;
    if folder.parent()? != sketches {
        return None;
    }
    let name = folder.file_name()?.to_str()?.to_owned();
    if name == DISCARDED || name.starts_with('.') {
        return None;
    }
    let mut pages = Vec::new();
    find(&folder, "", 0, &mut pages);
    pages.sort_by_key(|page: &String| {
        let index = page == "index.html" || page.ends_with("/index.html");
        (page.matches('/').count(), !index, page.clone())
    });
    pages.truncate(PAGES);
    Some((name, pages))
}

/// Adds the pages in `at` of `folder`, `deep` folders down, to `pages`.
fn find(folder: &Path, at: &str, deep: usize, pages: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(folder.join(at)) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let path = if at.is_empty() {
            name.clone()
        } else {
            format!("{at}/{name}")
        };
        if kind.is_dir() && deep < DEEP && !NOT_LOOKED_IN.contains(&name.as_str()) {
            find(folder, &path, deep + 1, pages);
        } else if kind.is_file() && matches!(extension(&name).as_deref(), Some("html" | "htm")) {
            pages.push(path);
        }
    }
}

/// The extension of `name`, in lowercase.
fn extension(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

/// The type of a file of a sketch, by its extension, as a browser takes
/// it.
fn kind(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    match extension(name).as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("txt" | "md") => TEXT,
        _ => "application/octet-stream",
    }
}

/// What a path of a sketch names.
#[derive(Debug, PartialEq, Eq)]
enum Found {
    File(PathBuf),
    /// A folder named without the `/` that ends it, which the browser is
    /// sent to, so that the links of its `index.html` lead where they
    /// say.
    Folder,
}

/// The file of a sketch in `sketches` that `path`, `/<sketch>/<path>`,
/// names, a folder's `index.html` for the folder: none that is out of
/// the sketch's folder, as through `..` or a link, nor hidden in it, as
/// its `.git`.
fn file(sketches: &Path, path: &str) -> Option<Found> {
    let path = cox_browser::scheme::percent_decoded(path.trim_start_matches('/'));
    let mut parts = path.split('/');
    let name = parts.next()?;
    let rest: Vec<&str> = parts.filter(|part| !part.is_empty()).collect();
    if name.is_empty()
        || name == DISCARDED
        || std::iter::once(name)
            .chain(rest.iter().copied())
            .any(|part| part.starts_with('.') || part.contains('\\'))
    {
        return None;
    }
    let sketches = sketches.canonicalize().ok()?;
    let sketch = sketches.join(name).canonicalize().ok()?;
    // A sketch is a folder: a file beside the sketches is none of theirs,
    // though macOS finds `<file>/` as the file itself.
    if sketch.parent()? != sketches || !sketch.is_dir() {
        return None;
    }
    let mut at = sketch.join(rest.join("/")).canonicalize().ok()?;
    if at.is_dir() {
        if !path.ends_with('/') {
            return Some(Found::Folder);
        }
        at = at.join("index.html").canonicalize().ok()?;
    }
    let within = at.strip_prefix(&sketch).ok()?;
    let hidden = within.components().any(|part| match part {
        Component::Normal(part) => part.to_string_lossy().starts_with('.'),
        _ => true,
    });
    (!hidden && at.is_file()).then_some(Found::File(at))
}

/// Serves the sketches' pages on `listener`, which listens on `port` of
/// the Mac's own address.
pub async fn serve(listener: TcpListener, port: u16) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::error!("a page could not come for a sketch: {error}");
                continue;
            }
        };
        tokio::task::spawn_local(async move {
            let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                let told = told(&request, port);
                async move { Ok::<_, Infallible>(told) }
            });
            if let Err(error) = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await
            {
                tracing::debug!("a connection to the sketches ended: {error}");
            }
        });
    }
}

/// Whether a request with `headers` was made to `port` of the Mac's own
/// address, by its name.
fn on_the_mac(headers: &HeaderMap, port: u16) -> bool {
    headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| {
            host == format!("localhost:{port}") || host == format!("127.0.0.1:{port}")
        })
}

/// What `request`, made to `port`, is told: the file of a sketch it asks
/// for.
fn told(request: &Request<Incoming>, port: u16) -> Response<Body> {
    if request.method() != Method::GET {
        return answer(StatusCode::METHOD_NOT_ALLOWED, TEXT, "Only read");
    }
    if !on_the_mac(request.headers(), port) {
        return answer(
            StatusCode::MISDIRECTED_REQUEST,
            TEXT,
            "The sketches are served at the Mac's own address alone.",
        );
    }
    let path = request.uri().path();
    let at = match storage::path(Kept::Sketches).and_then(|sketches| file(&sketches, path)) {
        Some(Found::File(at)) => at,
        Some(Found::Folder) => {
            let mut response = answer(StatusCode::MOVED_PERMANENTLY, TEXT, "");
            if let Ok(there) = HeaderValue::from_str(&format!("{path}/")) {
                response.headers_mut().insert(header::LOCATION, there);
            }
            return response;
        }
        None => return answer(StatusCode::NOT_FOUND, TEXT, "Not found"),
    };
    let read = std::fs::metadata(&at)
        .ok()
        .filter(|metadata| metadata.len() <= MOST)
        .and_then(|_| std::fs::read(&at).ok());
    let Some(bytes) = read else {
        return answer(StatusCode::NOT_FOUND, TEXT, "Not found");
    };
    let mut response = answer(StatusCode::OK, kind(&at), bytes);
    // A mock runs what it holds, as the scripts it loads from elsewhere;
    // the relay says who may frame it.
    response
        .headers_mut()
        .remove(header::CONTENT_SECURITY_POLICY);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn lists_the_pages_of_a_sketch_alone() {
        let sketches = TempDir::new();
        let sketch = sketches.join("scratch-1");
        for file in [
            "about.html",
            "index.html",
            "shop/index.html",
            "shop/cart.HTM",
            "style.css",
            ".git/info.html",
            "node_modules/x/index.html",
        ] {
            let path = sketch.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "<p>hi</p>").unwrap();
        }

        let (name, found) = pages(&sketches, &sketch).unwrap();

        assert_eq!(name, "scratch-1");
        assert_eq!(
            found,
            [
                "index.html",
                "about.html",
                "shop/index.html",
                "shop/cart.HTM"
            ]
        );
        assert_eq!(pages(&sketches, &sketches), None);
        assert_eq!(pages(&sketches, &sketch.join("shop")), None);
        std::fs::create_dir_all(sketches.join(DISCARDED)).unwrap();
        assert_eq!(pages(&sketches, &sketches.join(DISCARDED)), None);
    }

    #[test]
    fn serves_only_what_is_in_a_sketch() {
        let sketches = TempDir::new();
        let sketch = sketches.join("scratch-2");
        std::fs::create_dir_all(sketch.join(".git")).unwrap();
        std::fs::create_dir_all(sketch.join("app")).unwrap();
        std::fs::write(sketch.join("index.html"), "<p>mock</p>").unwrap();
        std::fs::write(sketch.join("app/index.html"), "<p>app</p>").unwrap();
        std::fs::write(sketch.join(".git/config"), "secret").unwrap();
        std::fs::write(sketches.join("outside.html"), "no").unwrap();

        assert_eq!(
            file(&sketches, "/scratch-2/"),
            Some(Found::File(sketch.join("index.html")))
        );
        assert_eq!(
            file(&sketches, "/scratch-2/app/"),
            Some(Found::File(sketch.join("app/index.html")))
        );
        assert_eq!(file(&sketches, "/scratch-2/app"), Some(Found::Folder));
        assert_eq!(
            file(&sketches, "/scratch%2D2/index.html"),
            Some(Found::File(sketch.join("index.html")))
        );
        assert_eq!(file(&sketches, "/scratch-2/.git/config"), None);
        assert_eq!(file(&sketches, "/scratch-2/../outside.html"), None);
        assert_eq!(file(&sketches, "/scratch-2/%2E%2E/outside.html"), None);
        assert_eq!(file(&sketches, "/outside.html"), None);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(sketches.join("outside.html"), sketch.join("out.html"))
                .unwrap();
            std::os::unix::fs::symlink(sketch.join(".git"), sketch.join("git")).unwrap();
            assert_eq!(file(&sketches, "/scratch-2/out.html"), None);
            assert_eq!(file(&sketches, "/scratch-2/git/config"), None);
        }
    }

    #[test]
    fn answers_only_as_made_to_the_macs_own_address() {
        let headers = |host: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, host.parse().unwrap());
            headers
        };
        assert!(on_the_mac(&headers("localhost:7423"), 7423));
        assert!(on_the_mac(&headers("127.0.0.1:7423"), 7423));
        assert!(!on_the_mac(&headers("localhost:7424"), 7423));
        assert!(!on_the_mac(&headers("evil.example:7423"), 7423));
        assert!(!on_the_mac(&HeaderMap::new(), 7423));
    }
}
