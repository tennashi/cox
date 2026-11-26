//! What goes on in a folder, as the stage's Activity page shows it, at
//! `cox://api/activity` (see [`crate::stage::plugins`]), put together by
//! coxd's own code, which tells the same to the human's phone (see
//! [`coxd::activity`]).
//!
//! The window tells this module how its chats stand as they change (see
//! [`tell`]), since the stage's pages are served apart from it; the bar
//! counts what waits on the Activity's button (see [`waiting`]). What a
//! chat does now, and what it asks, come from what the window told of its
//! conversation (see [`commands::now`]). A line the page has a chat sent,
//! as one written on a step of its plan, goes to the window, which sends
//! it (see [`listen`]).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use coxd::activity::{Chat, Of, OfCommand, Open, Pull, Say, Told, dismiss};
use serde_json::Value;

use crate::stage::commands;

/// How the chats stood when the window last told.
static TOLD: Mutex<Option<Told>> = Mutex::new(None);

/// Keeps how the window's chats stand now, for the pages to read.
pub fn tell(told: Told) {
    if let Ok(mut now) = TOLD.lock() {
        *now = Some(told);
    }
}

/// How the chats stood when the window last told, or none of them.
fn now() -> Told {
    TOLD.lock()
        .ok()
        .and_then(|now| now.clone())
        .unwrap_or_default()
}

/// The window's projects, by folder and name, as it last told.
#[must_use]
pub fn projects() -> Vec<(PathBuf, String)> {
    now().projects
}

/// The home, the folder of Cox's own chats, as the window last told, if it
/// told.
#[must_use]
pub fn home() -> Option<PathBuf> {
    Some(now().home).filter(|home| !home.as_os_str().is_empty())
}

/// How many chats wait for the human of those the page of the stage of
/// `root` tells of, for the bar.
#[must_use]
pub fn waiting(root: &Path) -> usize {
    coxd::activity::waiting(&now(), root)
}

/// What goes on in the project of the folder `root`, as its stage's page
/// tells it, as JSON, from how the chats stand now, and what they do and
/// ask, as the window told of their conversations.
#[must_use]
pub fn told(root: &Path) -> Value {
    coxd::activity::told(&now(), root, &commands::now)
}

/// The thread of the step `open` names, as JSON.
#[must_use]
pub fn thread(open: &Open) -> Value {
    coxd::activity::thread(&now(), open)
}

/// The story of the chat `of` names, as JSON.
#[must_use]
pub fn timeline(of: &Of) -> Value {
    coxd::activity::timeline(&now(), of)
}

/// The command `of` names, as JSON, laid out to read.
#[must_use]
pub fn command(of: &OfCommand) -> Value {
    coxd::activity::command(&now(), of)
}

/// Where the lines the page has a chat sent go, to the window that
/// listens.
static SAYS: Mutex<Option<tokio::sync::mpsc::UnboundedSender<Say>>> = Mutex::new(None);

/// The lines the page has the chats sent from now on, as the human
/// writes on steps of the plans, for the window to send each to its chat.
pub fn listen() -> tokio::sync::mpsc::UnboundedReceiver<Say> {
    let (says, heard) = tokio::sync::mpsc::unbounded_channel();
    if let Ok(mut now) = SAYS.lock() {
        *now = Some(says);
    }
    heard
}

/// Has the window send `say`, if there is one, and it listens.
fn send(say: Option<Say>) {
    if let Some(say) = say
        && let Ok(listening) = SAYS.lock()
        && let Some(listening) = listening.as_ref()
    {
        let _ = listening.send(say);
    }
}

/// Has the window send the line `asked` on its step, as the JSON the page
/// sent says, if the chat takes a line now and its plan has the step (see
/// [`coxd::activity::say`]); and answers the step's thread to show, if
/// the page named one.
#[must_use]
pub fn say(asked: &str) -> Option<Open> {
    let (say, open) = coxd::activity::say(&now(), asked)?;
    send(say);
    Some(open)
}

/// Has the window send the chat that opened the pull request `asked`
/// names, as the JSON the page sent says, a line that asks it to fix its
/// CI, as the human would, unless it is refused (see
/// [`coxd::activity::fix`]).
pub fn fix(asked: &str) {
    if let Ok(asked) = serde_json::from_str(asked) {
        send(coxd::activity::fix(&now(), &asked));
    }
}

/// Has the window send the chat `asked` names, as the JSON the page sent
/// says, the line that has it go on from where its turn was cut off, as
/// the human would, unless it is refused (see [`coxd::activity::resume`]).
pub fn resume(asked: &str) {
    if let Ok(asked) = serde_json::from_str(asked) {
        send(coxd::activity::resume(&now(), &asked));
    }
}
