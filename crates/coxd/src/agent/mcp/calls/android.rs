//! The tools that look at and use the Android emulators' virtual devices,
//! as the agent learns of them, and what each asks of the chat's stage:
//! the device's display shows on a tab, where the human sees what the
//! agent sees, and its pointer where it taps (see
//! `cox::stage::android`).

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::ask;
use crate::agent::mcp::{AndroidAct, Seen, ToolRequest};

pub(super) const ANDROID_DEVICES: &str = "android_devices";
pub(super) const ANDROID_LOOK: &str = "android_look";
pub(super) const ANDROID_TAP: &str = "android_tap";
pub(super) const ANDROID_SWIPE: &str = "android_swipe";
pub(super) const ANDROID_TYPE: &str = "android_type";
pub(super) const ANDROID_KEY: &str = "android_key";

/// The tools' names.
pub(super) const NAMES: [&str; 6] = [
    ANDROID_DEVICES,
    ANDROID_LOOK,
    ANDROID_TAP,
    ANDROID_SWIPE,
    ANDROID_TYPE,
    ANDROID_KEY,
];

/// The device the tools act on, as their schemas say it.
fn device_property() -> Value {
    json!({
        "type": "string",
        "description": "The device's name, as android_devices lists it.",
    })
}

/// A point of the display's picture, `way`, as the schemas say it.
fn along(way: &str) -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "description": format!(
            "Pixels {way} of the display's last picture, as android_look or the last act \
             answered it."
        ),
    })
}

/// The tools, as the agent learns of them.
pub(super) fn tools() -> impl Iterator<Item = Value> {
    seeing_tools().into_iter().chain(acting_tools())
}

/// The tools that find a device and look at it.
fn seeing_tools() -> [Value; 2] {
    [
        json!({
            "name": ANDROID_DEVICES,
            "description": "Lists the Android emulator's virtual devices, as Android \
                Studio's Device Manager keeps them, each with its name, which the other \
                android tools take, what it is, and whether its emulator runs. Use it to \
                find a device to try an app for Android on.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": ANDROID_LOOK,
            "description": "Shows the human an Android device's display, on a tab of the \
                stage beside the conversation, starting its emulator if it does not run, \
                and answers a picture of it as it is now, so that the human sees what you \
                see. Where you tap or swipe on it is in the pixels of this picture, from \
                its top left corner. An emulator just started takes a while to boot: look \
                again until its display shows.",
            "inputSchema": {
                "type": "object",
                "properties": { "device": device_property() },
                "required": ["device"],
            },
        }),
    ]
}

/// The tools that act on a device.
fn acting_tools() -> [Value; 4] {
    [
        json!({
            "name": ANDROID_TAP,
            "description": "Taps an Android device's display, where the human sees your \
                pointer on its tab, and answers a picture of the display after.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "device": device_property(),
                    "x": along("from the left edge"),
                    "y": along("down from the top edge"),
                },
                "required": ["device", "x", "y"],
            },
        }),
        json!({
            "name": ANDROID_SWIPE,
            "description": "Swipes a finger across an Android device's display, as to \
                scroll a list or pull a sheet down, and answers a picture of the display \
                after. A list scrolls down as the finger goes up.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "device": device_property(),
                    "x": along("from the left edge, where the finger lands"),
                    "y": along("down from the top edge, where the finger lands"),
                    "to_x": along("from the left edge, where the finger lifts"),
                    "to_y": along("down from the top edge, where the finger lifts"),
                },
                "required": ["device", "x", "y", "to_x", "to_y"],
            },
        }),
        json!({
            "name": ANDROID_TYPE,
            "description": "Types text on an Android device, into the field that has the \
                keyboard, as the one you tapped last, and answers a picture of the display \
                after. For Enter or Back, use android_key.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "device": device_property(),
                    "text": { "type": "string", "description": "The text to type." },
                },
                "required": ["device", "text"],
            },
        }),
        json!({
            "name": ANDROID_KEY,
            "description": "Presses a key on an Android device, and answers a picture of the \
                display after.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "device": device_property(),
                    "key": {
                        "type": "string",
                        "description": "The key, as the web names it, as `Enter`, \
                            `Backspace`, `Tab` or `ArrowDown`, or one of Android's: `GoBack`, \
                            `GoHome` or `AppSwitch`, for the overview of its apps.",
                    },
                },
                "required": ["device", "key"],
            },
        }),
    ]
}

/// The whole number `name` of `arguments`, which the tool needs.
fn needed(arguments: &Value, name: &str) -> Result<u32, String> {
    arguments
        .get(name)
        .ok_or_else(|| format!("no `{name}`"))?
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("`{name}` is a whole number, at least 0"))
}

/// What the tool `name` does, as `arguments` say.
fn act(name: &str, arguments: &Value) -> Result<AndroidAct, String> {
    if name == ANDROID_DEVICES {
        return Ok(AndroidAct::List);
    }
    let text = |field: &str| {
        arguments
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("no `{field}`"))
    };
    let device = text("device")?;
    Ok(match name {
        ANDROID_LOOK => AndroidAct::Look { device },
        ANDROID_TAP => AndroidAct::Tap {
            device,
            x: needed(arguments, "x")?,
            y: needed(arguments, "y")?,
        },
        ANDROID_SWIPE => AndroidAct::Swipe {
            device,
            x: needed(arguments, "x")?,
            y: needed(arguments, "y")?,
            to_x: needed(arguments, "to_x")?,
            to_y: needed(arguments, "to_y")?,
        },
        ANDROID_TYPE => AndroidAct::Type {
            device,
            text: text("text")?,
        },
        ANDROID_KEY => AndroidAct::Key {
            device,
            key: text("key")?,
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
    ask(requests, ToolRequest::Android { act, answer })?;
    seen.await
        .map_err(|_| "the stage went away before it answered".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_swipe_as_the_agent_gives_it() {
        let swipe = act(
            ANDROID_SWIPE,
            &json!({ "device": "Pixel_9", "x": 10, "y": 900, "to_x": 10, "to_y": 100 }),
        );
        assert_eq!(
            swipe,
            Ok(AndroidAct::Swipe {
                device: "Pixel_9".to_owned(),
                x: 10,
                y: 900,
                to_x: 10,
                to_y: 100,
            })
        );
    }

    #[test]
    fn tells_the_agent_what_it_got_wrong() {
        assert_eq!(act(ANDROID_LOOK, &json!({})), Err("no `device`".to_owned()));
        assert_eq!(
            act(ANDROID_TAP, &json!({ "device": "Pixel_9", "x": 1 })),
            Err("no `y`".to_owned())
        );
        assert!(
            act(
                ANDROID_TAP,
                &json!({ "device": "Pixel_9", "x": -1, "y": 0 })
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn asks_the_stage_and_answers_what_it_saw() {
        let (sent, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let called = tokio::spawn(async move {
            call(
                ANDROID_KEY,
                &json!({ "device": "Pixel_9", "key": "GoBack" }),
                &sent,
            )
            .await
        });
        let Some(ToolRequest::Android { act, answer }) = asked.recv().await else {
            panic!("the stage was not asked");
        };
        assert_eq!(
            act,
            AndroidAct::Key {
                device: "Pixel_9".to_owned(),
                key: "GoBack".to_owned(),
            }
        );
        let seen = Seen {
            text: "Pixel_9".to_owned(),
            png: None,
        };
        answer.send(Ok(seen.clone())).unwrap();
        assert_eq!(called.await.unwrap(), Ok(seen));
    }
}
