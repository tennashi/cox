//! A map of the chat's project, as a page of the stage draws it: coxd
//! draws it from the map's vocabulary, a file of the project's `maps`,
//! or of one of its repositories', and the indexes of the project's code
//! (see [`coxd::map`]), and the Map page reads it as JSON from
//! `cox://api/map/<name>` (see [`crate::stage::plugins`]), which asks
//! coxd for it through the window (see [`told`]). The agent's `show_map`
//! tool opens the page where it talks of.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// How long a page waits for coxd to draw a map: reading the indexes of
/// a large project's code takes a second or two.
pub const PATIENCE: Duration = Duration::from_secs(60);

/// A map a page asks coxd for, through the window: the project's
/// folder, the map's name, or none for its first, whether to make again
/// the indexes that failed, and where the JSON coxd answers goes.
#[derive(Debug)]
pub struct Ask {
    pub folder: PathBuf,
    pub name: Option<String>,
    pub again: bool,
    pub answer: std::sync::mpsc::Sender<String>,
}

/// Where the pages' asks for maps go, to the window that listens.
static ASKS: Mutex<Option<UnboundedSender<Ask>>> = Mutex::new(None);

/// The maps the pages ask for from now on, for the window to ask coxd
/// for each.
pub fn listen() -> UnboundedReceiver<Ask> {
    let (asks, heard) = unbounded_channel();
    if let Ok(mut now) = ASKS.lock() {
        *now = Some(asks);
    }
    heard
}

/// The map of the project in `root` named `name`, or its first, as coxd
/// draws it from the indexes of the project's code, as the JSON of
/// [`coxd::map::Told`], making again the indexes that failed when
/// `again`. When coxd does not answer, the JSON says so, as
/// `{"state": "unanswered", "problems": [<why>]}`.
#[must_use]
pub fn told(root: &Path, name: Option<&str>, again: bool) -> String {
    let unanswered =
        |why: &str| serde_json::json!({ "state": "unanswered", "problems": [why] }).to_string();
    let (answer, answered) = std::sync::mpsc::channel();
    let ask = Ask {
        folder: root.to_owned(),
        name: name.map(str::to_owned),
        again,
        answer,
    };
    let asked = ASKS
        .lock()
        .ok()
        .and_then(|asks| asks.as_ref()?.send(ask).ok());
    if asked.is_none() {
        return unanswered("no window asks coxd for maps");
    }
    answered
        .recv_timeout(PATIENCE)
        .unwrap_or_else(|_| unanswered("coxd did not answer in time"))
}
