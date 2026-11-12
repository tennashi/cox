//! What the human's keys do to a page besides typing in it: the hints
//! over what can be clicked, scrolling, and the text field the keyboard
//! is in. The scripts behind them run in a world of their own (an
//! isolated world of the Chrome `DevTools` Protocol), over the stage's
//! own `DevTools` session with the page, in process: the page sees the
//! hints drawn over it, but none of the scripts, nor can it call them.
//!
//! Clicks, and scrolls by lines or half pages, go to the page as the
//! human's own mouse would, so that the page takes them as it takes
//! theirs, and the wheel scrolls whatever part of the page is under the
//! middle of it. The top and the bottom are set by the script, on that
//! same part.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use cef::ImplBrowserHost as _;

use super::{Browser, Event, Modifiers, MouseButton, SharedRef, lock};

/// What the stage's keys do in the page: `keyboard.js`, which each call
/// runs before its own line, once per page.
const SCRIPT: &str = include_str!("keyboard.js");

/// The name of the world the scripts run in.
const WORLD: &str = "cox-keyboard";

/// How far a line scrolls, in logical pixels: as far as an arrow key
/// scrolls in Chromium.
const LINE: i32 = 40;

/// Where something is in the page, in logical pixels of its viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    fn middle(self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
}

/// Something in sight in the page that can be clicked or typed in, which
/// a hint can be drawn over (see [`Browser::hints`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// What the page knows it by until it lists its hints again.
    pub id: u32,
    pub rect: Rect,
    pub kind: Kind,
    /// Its text, or its label, as far as 80 characters.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A link to this address.
    Link(String),
    /// A field that takes text.
    Field,
    /// A button, or anything else that takes a click, as a frame of
    /// another origin, whose insides the stage cannot see.
    Other,
}

/// How a hint is taken: clicked in its middle, or, for a field, given
/// the keyboard where its text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Click,
    Focus,
}

/// Where the keys scroll the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// Down this many lines, or up as many below zero.
    Lines(i32),
    /// Down this many halves of the page's height, or up.
    HalfPages(i32),
    Top,
    Bottom,
}

/// A hint as the script lists it.
#[derive(Deserialize)]
struct Listed {
    id: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    kind: String,
    href: Option<String>,
    text: String,
}

impl From<Listed> for Hint {
    fn from(listed: Listed) -> Self {
        let kind = match (listed.kind.as_str(), listed.href) {
            ("link", Some(href)) => Kind::Link(href),
            ("field", _) => Kind::Field,
            _ => Kind::Other,
        };
        Self {
            id: listed.id,
            rect: Rect {
                x: listed.x,
                y: listed.y,
                width: listed.width,
                height: listed.height,
            },
            kind,
            text: listed.text,
        }
    }
}

/// The id of `Target.getTargetInfo`, the one call made as the page is
/// created, which the calls of [`Calls`] never take.
pub(super) const TARGET_INFO: i32 = 0;

/// What a call answers with: its result, or `None` when it failed.
type Reply = Box<dyn FnOnce(Option<Value>) + Send>;

/// The stage's `DevTools` calls to a page that wait for their answers,
/// by their ids, past [`TARGET_INFO`].
#[derive(Default)]
pub(super) struct Calls {
    last: i32,
    waiting: HashMap<i32, Reply>,
}

impl Calls {
    fn add(&mut self, reply: Reply) -> i32 {
        self.last = self.last.checked_add(1).unwrap_or(TARGET_INFO + 1);
        self.waiting.insert(self.last, reply);
        self.last
    }

    /// What waits for the answer to the call `id`, if anything does.
    pub(super) fn take(&mut self, id: i32) -> Option<Reply> {
        self.waiting.remove(&id)
    }
}

/// Calls `method` with `params` in the page, and hands its result to
/// `reply` once it comes, on CEF's UI thread.
fn call(
    shared: &SharedRef,
    method: &str,
    params: &Value,
    reply: impl FnOnce(Option<Value>) + Send + 'static,
) {
    let (host, id) = {
        let mut shared = lock(shared);
        let Some(host) = shared.host.clone() else {
            drop(shared);
            return reply(None);
        };
        (host, shared.calls.add(Box::new(reply)))
    };
    let message = json!({"id": id, "method": method, "params": params}).to_string();
    if host.send_dev_tools_message(Some(message.as_bytes())) != 1
        && let Some(reply) = lock(shared).calls.take(id)
    {
        reply(None);
    }
}

/// Runs `line` in the stage's world of the page's main frame, after the
/// script, and hands what it comes to to `reply`.
fn evaluate(shared: &SharedRef, line: String, reply: impl FnOnce(Option<Value>) + Send + 'static) {
    let page = Arc::clone(shared);
    call(shared, "Page.getFrameTree", &json!({}), move |tree| {
        let Some(frame) =
            tree.and_then(|tree| tree["frameTree"]["frame"]["id"].as_str().map(str::to_owned))
        else {
            return reply(None);
        };
        let world = json!({"frameId": frame, "worldName": WORLD});
        let then = Arc::clone(&page);
        call(&page, "Page.createIsolatedWorld", &world, move |created| {
            let Some(context) = created.and_then(|created| created["executionContextId"].as_i64())
            else {
                return reply(None);
            };
            let params = json!({
                "expression": format!("{SCRIPT}\n{line}"),
                "contextId": context,
                "returnByValue": true,
            });
            call(&then, "Runtime.evaluate", &params, move |answer| {
                let value = answer.and_then(|answer| {
                    if let Some(error) = answer.get("exceptionDetails") {
                        tracing::warn!(%error, "the stage's script failed in the page");
                        return None;
                    }
                    Some(answer["result"]["value"].clone())
                });
                reply(value);
            });
        });
    });
}

impl Browser {
    /// Lists what can be clicked or typed in, in sight in the page and its
    /// frames of the same origin: they come as [`Event::Hints`].
    pub fn hints(&self) {
        let events = self.events.clone();
        evaluate(&self.shared, "coxVim.hints()".to_owned(), move |listed| {
            let hints = listed
                .and_then(|listed| serde_json::from_value::<Vec<Listed>>(listed).ok())
                .unwrap_or_default()
                .into_iter()
                .map(Hint::from)
                .collect();
            let _ = events.send(Event::Hints(hints));
        });
    }

    /// Draws each label over the hint of its id, of those listed last, in
    /// place of what was drawn before.
    pub fn show_hints(&self, labels: &[(u32, String)]) {
        evaluate(
            &self.shared,
            format!("coxVim.show({})", json!(labels)),
            |_| {},
        );
    }

    pub fn clear_hints(&self) {
        evaluate(&self.shared, "coxVim.clear()".to_owned(), |_| {});
    }

    /// Takes `hint`, of those listed last, as `how` says.
    pub fn activate(&self, hint: &Hint, how: How) {
        match how {
            How::Click => {
                let (x, y) = hint.rect.middle();
                let none = Modifiers::default();
                self.mouse_move(x, y, None, none);
                self.mouse_down(x, y, MouseButton::Left, 1, none);
                self.mouse_up(x, y, MouseButton::Left, 1, none);
            }
            How::Focus => evaluate(&self.shared, format!("coxVim.focus({})", hint.id), |_| {}),
        }
    }

    /// Gives the keyboard to the page's first field that takes text, as
    /// far down the page as it is.
    pub fn focus_field(&self) {
        evaluate(&self.shared, "coxVim.field()".to_owned(), |_| {});
    }

    /// Takes the keyboard from the field that has it, leaving it with the
    /// page.
    pub fn blur_field(&self) {
        evaluate(&self.shared, "coxVim.blur()".to_owned(), |_| {});
    }

    pub fn scroll(&self, scroll: Scroll) {
        let (width, height) = {
            let shared = self.shared();
            (shared.reported.width, shared.reported.height)
        };
        let down = match scroll {
            Scroll::Lines(lines) => lines.saturating_mul(LINE),
            Scroll::HalfPages(halves) => halves.saturating_mul(height / 2),
            Scroll::Top | Scroll::Bottom => {
                let bottom = scroll == Scroll::Bottom;
                return evaluate(&self.shared, format!("coxVim.scroll({bottom})"), |_| {});
            }
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "a scroll is far below what f32 keeps exactly"
        )]
        let down = down as f32;
        // CEF's wheel goes down for less than zero.
        self.wheel(width / 2, height / 2, 0.0, -down, Modifiers::default());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Calls, Hint, Kind, Listed, Rect};

    #[test]
    fn a_listed_hint_is_a_link_a_field_or_else_something_to_click() {
        let hint = |kind: &str, href: Option<&str>| {
            Hint::from(
                serde_json::from_value::<Listed>(json!({
                    "id": 3, "x": 1, "y": 2, "width": 30, "height": 10,
                    "kind": kind, "href": href, "text": "go",
                }))
                .unwrap(),
            )
        };
        let link = hint("link", Some("https://example.com/"));
        assert_eq!(link.kind, Kind::Link("https://example.com/".to_owned()));
        assert_eq!(
            link.rect,
            Rect {
                x: 1,
                y: 2,
                width: 30,
                height: 10
            }
        );
        assert_eq!(link.rect.middle(), (16, 7));
        assert_eq!(hint("field", None).kind, Kind::Field);
        assert_eq!(hint("other", None).kind, Kind::Other);
        assert_eq!(hint("link", None).kind, Kind::Other);
    }

    #[test]
    fn each_call_waits_under_an_id_of_its_own_past_zero() {
        let mut calls = Calls::default();
        let first = calls.add(Box::new(|_| {}));
        let second = calls.add(Box::new(|_| {}));
        assert_eq!((first, second), (1, 2));
        assert!(calls.take(first).is_some());
        assert!(calls.take(first).is_none());
        calls.last = i32::MAX;
        assert_eq!(calls.add(Box::new(|_| {})), 1);
    }
}
