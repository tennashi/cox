//! The page's icon, which its tab shows. CEF names the icons the page
//! links to, or the site's `/favicon.ico` when it links to none, and the
//! browser fetches the first of them that loads, as Chromium fetches a
//! tab's icon. The last page's icon stays while the next loads, so that
//! a tab going from page to page of one site does not flicker; a page
//! that names none, or none of whose icons load, has none.
#![expect(
    clippy::transmute_ptr_to_ptr,
    reason = "the cef wrap macros cast the handler reference CEF passes back"
)]

use std::os::raw::c_int;
use std::sync::Arc;

use cef::rc::Rc as _;
use cef::{
    CefString, DownloadImageCallback, Image, ImplBinaryValue, ImplBrowserHost,
    ImplDownloadImageCallback, ImplImage, WrapDownloadImageCallback, wrap_download_image_callback,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{Event, SharedRef, lock};

/// How big an icon is, in logical pixels, as Chromium asks for a tab's:
/// CEF fetches it this big, and twice that for a screen of twice the
/// pixels.
const SIZE: u32 = 16;

/// The scale the icon is taken at, of those CEF fetched: the sharpest a
/// screen of twice the pixels needs, which the stage scales down on
/// another.
const SCALE: f32 = 2.;

/// What the browser knows of the page's icon.
#[derive(Debug, Default)]
pub(crate) struct Favicons {
    /// The addresses of the icons the page names, still to try, the one
    /// being fetched first.
    wanted: Vec<String>,
    /// The icon fetched: its address, and its picture as PNG.
    fetched: Option<(String, Arc<[u8]>)>,
    /// Whether the page loading now named its icons yet, so that a page
    /// that names none shows none, rather than the last page's.
    named: bool,
}

/// What to do once the icons changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Next {
    /// Whether the icon shown changed.
    changed: bool,
    /// The icon to fetch, if any.
    fetch: Option<String>,
}

impl Favicons {
    /// The picture of the icon, as PNG, once one is fetched.
    pub(crate) fn png(&self) -> Option<Arc<[u8]>> {
        self.fetched.as_ref().map(|(_, png)| Arc::clone(png))
    }

    /// Another page started to load: it shows the last page's icon until
    /// it names its own.
    pub(crate) fn loading(&mut self) {
        self.named = false;
    }

    /// The page loaded: one that named no icon has none.
    pub(crate) fn loaded(&mut self) -> Next {
        if self.named {
            return Next::default();
        }
        self.wanted.clear();
        Next {
            changed: self.fetched.take().is_some(),
            fetch: None,
        }
    }

    /// The page names `urls` as its icons, the best first: the first is
    /// fetched, unless it is the icon fetched already.
    pub(crate) fn named(&mut self, urls: Vec<String>) -> Next {
        self.named = true;
        if let (Some((fetched, _)), Some(first)) = (&self.fetched, urls.first())
            && fetched == first
        {
            self.wanted.clear();
            return Next::default();
        }
        self.wanted = urls;
        match self.wanted.first() {
            Some(first) => Next {
                changed: false,
                fetch: Some(first.clone()),
            },
            None => Next {
                changed: self.fetched.take().is_some(),
                fetch: None,
            },
        }
    }

    /// The icon at `url` was fetched, as `png`, or could not be: the next
    /// the page names is tried then, and with none left, the page has no
    /// icon. An icon of a page the browser has left since is let be.
    pub(crate) fn fetched(&mut self, url: &str, png: Option<Arc<[u8]>>) -> Next {
        if self.wanted.first().map(String::as_str) != Some(url) {
            return Next::default();
        }
        self.wanted.remove(0);
        if let Some(png) = png {
            self.wanted.clear();
            self.fetched = Some((url.to_owned(), png));
            return Next {
                changed: true,
                fetch: None,
            };
        }
        match self.wanted.first() {
            Some(next) => Next {
                changed: false,
                fetch: Some(next.clone()),
            },
            None => Next {
                changed: self.fetched.take().is_some(),
                fetch: None,
            },
        }
    }
}

/// Does what `next` says: tells the stage the icon changed, and fetches
/// the next.
pub(crate) fn follow(shared: &SharedRef, events: &UnboundedSender<Event>, next: Next) {
    if next.changed {
        let _ = events.send(Event::Favicon);
    }
    if let Some(url) = next.fetch {
        fetch(shared, events, url);
    }
}

/// Asks CEF for the icon at `url`, without cookies, as a browser fetches
/// a tab's icon.
fn fetch(shared: &SharedRef, events: &UnboundedSender<Event>, url: String) {
    let Some(host) = lock(shared).host.clone() else {
        return;
    };
    tracing::debug!(%url, "fetching the page's icon");
    let address = CefString::from(url.as_str());
    let mut fetched = FaviconFetched::new(Arc::clone(shared), events.clone(), url);
    host.download_image(Some(&address), 1, SIZE, 0, Some(&mut fetched));
}

// The macro takes no doc comments on fields: `url` is the address asked
// for, as the page named it.
wrap_download_image_callback! {
    struct FaviconFetched {
        shared: SharedRef,
        events: UnboundedSender<Event>,
        url: String,
    }

    impl DownloadImageCallback {
        fn on_download_image_finished(
            &self,
            _image_url: Option<&CefString>,
            status: c_int,
            image: Option<&mut Image>,
        ) {
            let png = image.and_then(|image| png(image));
            if png.is_none() {
                tracing::debug!(url = %self.url, status, "the page's icon did not load");
            }
            let next = lock(&self.shared).favicons.fetched(&self.url, png);
            follow(&self.shared, &self.events, next);
        }
    }
}

/// The picture CEF fetched, as PNG, if it has one.
fn png(image: &Image) -> Option<Arc<[u8]>> {
    if image.is_empty() == 1 {
        return None;
    }
    // CEF answers nothing unless asked for the size too.
    let (mut width, mut height) = (0, 0);
    let value = image.as_png(SCALE, 1, Some(&mut width), Some(&mut height))?;
    let mut bytes = vec![0; value.size()];
    let read = value.data(Some(&mut bytes), 0);
    bytes.truncate(read);
    (!bytes.is_empty()).then(|| bytes.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(byte: u8) -> Arc<[u8]> {
        Arc::from([byte])
    }

    fn fetch(url: &str) -> Next {
        Next {
            changed: false,
            fetch: Some(url.to_owned()),
        }
    }

    const CHANGED: Next = Next {
        changed: true,
        fetch: None,
    };

    #[test]
    fn the_first_icon_named_is_fetched() {
        let mut icons = Favicons::default();
        icons.loading();
        assert_eq!(icons.named(vec!["a".into(), "b".into()]), fetch("a"));
        assert_eq!(icons.fetched("a", Some(png(1))), CHANGED);
        assert_eq!(icons.png(), Some(png(1)));
        assert_eq!(icons.loaded(), Next::default());
        assert_eq!(icons.png(), Some(png(1)));
    }

    #[test]
    fn an_icon_that_does_not_load_gives_way_to_the_next() {
        let mut icons = Favicons::default();
        icons.named(vec!["a".into(), "b".into()]);
        assert_eq!(icons.fetched("a", None), fetch("b"));
        assert_eq!(icons.fetched("b", Some(png(2))), CHANGED);
        assert_eq!(icons.png(), Some(png(2)));
    }

    #[test]
    fn a_page_none_of_whose_icons_load_has_none() {
        let mut icons = Favicons::default();
        icons.named(vec!["a".into()]);
        icons.fetched("a", Some(png(1)));
        icons.loading();
        icons.named(vec!["b".into()]);
        assert_eq!(
            icons.png(),
            Some(png(1)),
            "the last page's icon stays meanwhile"
        );
        assert_eq!(icons.fetched("b", None), CHANGED);
        assert_eq!(icons.png(), None);
    }

    #[test]
    fn a_page_that_names_no_icon_has_none() {
        let mut icons = Favicons::default();
        icons.named(vec!["a".into()]);
        icons.fetched("a", Some(png(1)));
        icons.loading();
        assert_eq!(icons.loaded(), CHANGED);
        assert_eq!(icons.png(), None);
    }

    #[test]
    fn the_icon_fetched_already_is_not_fetched_again() {
        let mut icons = Favicons::default();
        icons.named(vec!["a".into()]);
        icons.fetched("a", Some(png(1)));
        icons.loading();
        assert_eq!(icons.named(vec!["a".into()]), Next::default());
        assert_eq!(icons.png(), Some(png(1)));
    }

    #[test]
    fn an_icon_of_a_page_left_is_let_be() {
        let mut icons = Favicons::default();
        icons.named(vec!["a".into()]);
        icons.loading();
        icons.named(vec!["b".into()]);
        assert_eq!(icons.fetched("a", Some(png(1))), Next::default());
        assert_eq!(icons.png(), None);
        assert_eq!(icons.fetched("b", Some(png(2))), CHANGED);
    }
}
