//! The tools that look at and use the windows of the Mac's other apps, as
//! the agent learns of them, and what each asks of the chat's stage: the
//! window shows on a tab, where the human sees what the agent sees, and
//! its pointer where it clicks (see [`crate::stage::mac_window`]).

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::ask;
use crate::agent::mcp::{MacAct, Seen, ToolRequest};

pub(super) const MAC_WINDOWS: &str = "mac_windows";
pub(super) const MAC_WINDOW_LOOK: &str = "mac_window_look";
pub(super) const MAC_WINDOW_CLICK: &str = "mac_window_click";
pub(super) const MAC_WINDOW_TYPE: &str = "mac_window_type";
pub(super) const MAC_WINDOW_KEY: &str = "mac_window_key";
pub(super) const MAC_WINDOW_SCROLL: &str = "mac_window_scroll";

/// The tools' names.
pub(super) const NAMES: [&str; 6] = [
    MAC_WINDOWS,
    MAC_WINDOW_LOOK,
    MAC_WINDOW_CLICK,
    MAC_WINDOW_TYPE,
    MAC_WINDOW_KEY,
    MAC_WINDOW_SCROLL,
];

/// The most clicks in a row, as a triple click selects a line.
const MOST_CLICKS: u64 = 3;

/// The window the tools act on, as their schemas say it.
fn window_property() -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "description": "The window's number, as mac_windows lists it.",
    })
}

/// A point of the window's picture, as the schemas of the tools that act
/// there say it.
fn point_properties() -> (Value, Value) {
    let along = |way: &str| {
        json!({
            "type": "integer",
            "minimum": 0,
            "description": format!(
                "Pixels {way} of the window's last picture, as mac_window_look or the \
                 last act answered it."
            ),
        })
    };
    (along("from the left edge"), along("down from the top edge"))
}

/// The tools, as the agent learns of them.
pub(super) fn tools() -> impl Iterator<Item = Value> {
    seeing_tools().into_iter().chain(acting_tools())
}

/// The tools that find a window and look at it.
fn seeing_tools() -> [Value; 2] {
    [
        json!({
            "name": MAC_WINDOWS,
            "description": "Lists the windows of the Mac's other apps that are on screen, \
                each with its number, which the other mac_window tools take, its app, its \
                title and its size. Use it to find the app you try out, such as one you \
                built and started, before you look at its window or use it.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": MAC_WINDOW_LOOK,
            "description": "Shows the human a window of another app on the Mac, on a tab of \
                the stage beside the conversation, and answers a picture of it as it is \
                now, so that the human sees what you see. The window stays where it is, \
                behind Cox. Where you click or scroll in it is in the pixels of this \
                picture, from its top left corner.",
            "inputSchema": {
                "type": "object",
                "properties": { "window": window_property() },
                "required": ["window"],
            },
        }),
    ]
}

/// The tools that act in a window.
fn acting_tools() -> [Value; 4] {
    let (x, y) = point_properties();
    [
        json!({
            "name": MAC_WINDOW_CLICK,
            "description": "Clicks in a window of another app on the Mac, where the human \
                sees your pointer on its tab, and answers a picture of the window after. \
                The app takes the click while its window stays behind Cox, and takes itself \
                for the active app until your turn ends, so that what you type next goes \
                where you clicked.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window": window_property(),
                    "x": x,
                    "y": y,
                    "button": {
                        "type": "string",
                        "enum": ["left", "right"],
                        "description": "The mouse's button: left unless given.",
                    },
                    "count": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MOST_CLICKS,
                        "description": "Clicks in a row, as 2 for a double click: 1 unless \
                            given.",
                    },
                },
                "required": ["window", "x", "y"],
            },
        }),
        json!({
            "name": MAC_WINDOW_TYPE,
            "description": "Types text into a window of another app on the Mac, where its app \
                has the keyboard, as in the field you clicked last, and answers a picture of \
                the window after. For Enter, Tab or a shortcut, use mac_window_key.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window": window_property(),
                    "text": { "type": "string", "description": "The text to type." },
                },
                "required": ["window", "text"],
            },
        }),
        json!({
            "name": MAC_WINDOW_KEY,
            "description": "Presses a key in a window of another app on the Mac, with the \
                modifiers held, and answers a picture of the window after.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window": window_property(),
                    "key": {
                        "type": "string",
                        "description": "The key, as `Enter`, `Tab`, `Escape`, `Backspace`, \
                            `ArrowDown`, `PageUp`, `F5`, a letter, a digit or a mark, after \
                            the modifiers held, each with a `+`, as `cmd+a` or \
                            `cmd+shift+z`: cmd, shift, option or ctrl.",
                    },
                },
                "required": ["window", "key"],
            },
        }),
        json!({
            "name": MAC_WINDOW_SCROLL,
            "description": "Turns the wheel over a point of a window of another app on the \
                Mac, and answers a picture of the window after.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window": window_property(),
                    "x": x,
                    "y": y,
                    "dx": {
                        "type": "integer",
                        "description": "Pixels to scroll the content right, or left when \
                            negative: 0 unless given.",
                    },
                    "dy": {
                        "type": "integer",
                        "description": "Pixels to scroll the content down, or up when \
                            negative: 0 unless given.",
                    },
                },
                "required": ["window", "x", "y"],
            },
        }),
    ]
}

/// A whole number of `arguments`, `name`, at least 0, if given.
fn number(arguments: &Value, name: &str) -> Result<Option<u32>, String> {
    arguments.get(name).map_or(Ok(None), |value| {
        value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| format!("`{name}` is a whole number, at least 0"))
    })
}

/// The whole number `name` of `arguments`, which the tool needs.
fn needed(arguments: &Value, name: &str) -> Result<u32, String> {
    number(arguments, name)?.ok_or_else(|| format!("no `{name}`"))
}

/// A number of pixels to scroll, `name`, 0 unless given.
fn pixels(arguments: &Value, name: &str) -> Result<i32, String> {
    arguments.get(name).map_or(Ok(0), |value| {
        value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| format!("`{name}` is a whole number of pixels"))
    })
}

/// What the tool `name` does, as `arguments` say.
fn act(name: &str, arguments: &Value) -> Result<MacAct, String> {
    if name == MAC_WINDOWS {
        return Ok(MacAct::List);
    }
    let window = needed(arguments, "window")?;
    let text = |field: &str| {
        arguments
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("no `{field}`"))
    };
    Ok(match name {
        MAC_WINDOW_LOOK => MacAct::Look { window },
        MAC_WINDOW_CLICK => {
            let right = match arguments.get("button").and_then(Value::as_str) {
                None | Some("left") => false,
                Some("right") => true,
                Some(other) => return Err(format!("no button {other:?}: left or right")),
            };
            let count = number(arguments, "count")?.unwrap_or(1);
            let count = u8::try_from(count)
                .ok()
                .filter(|count| (1..=MOST_CLICKS).contains(&u64::from(*count)))
                .ok_or_else(|| format!("`count` is 1 to {MOST_CLICKS}"))?;
            MacAct::Click {
                window,
                x: needed(arguments, "x")?,
                y: needed(arguments, "y")?,
                right,
                count,
            }
        }
        MAC_WINDOW_TYPE => MacAct::Type {
            window,
            text: text("text")?,
        },
        MAC_WINDOW_KEY => {
            let key = text("key")?;
            cox_mac::Stroke::parse(&key)?;
            MacAct::Key { window, key }
        }
        MAC_WINDOW_SCROLL => MacAct::Scroll {
            window,
            x: needed(arguments, "x")?,
            y: needed(arguments, "y")?,
            dx: pixels(arguments, "dx")?,
            dy: pixels(arguments, "dy")?,
        },
        _ => return Err(format!("no tool {name}")),
    })
}

/// Asks the stage to do what the tool `name` does, as `arguments` say,
/// and answers what the agent sees then.
pub(super) async fn call(
    name: &str,
    arguments: &Value,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<Seen, String> {
    let act = act(name, arguments)?;
    let (answer, seen) = oneshot::channel();
    ask(requests, ToolRequest::MacWindow { act, answer })?;
    seen.await
        .map_err(|_| "the stage went away before it answered".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_click_as_the_agent_gives_it() {
        let click = act(
            MAC_WINDOW_CLICK,
            &json!({ "window": 7, "x": 10, "y": 20, "button": "right", "count": 2 }),
        );
        assert_eq!(
            click,
            Ok(MacAct::Click {
                window: 7,
                x: 10,
                y: 20,
                right: true,
                count: 2,
            })
        );
        let plain = act(MAC_WINDOW_CLICK, &json!({ "window": 7, "x": 1, "y": 2 }));
        assert!(matches!(
            plain,
            Ok(MacAct::Click {
                right: false,
                count: 1,
                ..
            })
        ));
    }

    #[test]
    fn tells_the_agent_what_it_got_wrong() {
        assert_eq!(
            act(MAC_WINDOW_LOOK, &json!({})),
            Err("no `window`".to_owned())
        );
        assert!(act(MAC_WINDOW_CLICK, &json!({ "window": 1, "x": -1, "y": 0 })).is_err());
        assert!(
            act(
                MAC_WINDOW_CLICK,
                &json!({ "window": 1, "x": 0, "y": 0, "count": 9 })
            )
            .is_err()
        );
        assert!(act(MAC_WINDOW_KEY, &json!({ "window": 1, "key": "hyper+q" })).is_err());
        assert_eq!(
            act(
                MAC_WINDOW_SCROLL,
                &json!({ "window": 1, "x": 0, "y": 0, "dy": -40 })
            ),
            Ok(MacAct::Scroll {
                window: 1,
                x: 0,
                y: 0,
                dx: 0,
                dy: -40,
            })
        );
    }

    #[tokio::test]
    async fn asks_the_stage_and_answers_what_it_saw() {
        let (sent, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let called =
            tokio::spawn(
                async move { call(MAC_WINDOW_LOOK, &json!({ "window": 3 }), &sent).await },
            );
        let Some(ToolRequest::MacWindow { act, answer }) = asked.recv().await else {
            panic!("the stage was not asked");
        };
        assert_eq!(act, MacAct::Look { window: 3 });
        let seen = Seen {
            text: "Mac window 3".to_owned(),
            png: Some("iVBO".to_owned()),
        };
        answer.send(Ok(seen.clone())).unwrap();
        assert_eq!(called.await.unwrap(), Ok(seen));
    }
}
