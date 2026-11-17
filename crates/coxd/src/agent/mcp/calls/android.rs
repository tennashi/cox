//! The tools that look at and use the Android emulators' virtual devices,
//! as the agent learns of them, and what each asks of the chat's stage:
//! the device's display shows on a tab, where the human sees what the
//! agent sees, and its pointer where it taps (see
//! `cox::stage::android`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use super::ask;
use crate::agent::mcp::{AndroidAct, DeviceAct, Element, Seen, Spot, ToolRequest};

pub(super) const ANDROID_DEVICES: &str = "android_devices";
pub(super) const ANDROID_LOOK: &str = "android_look";
pub(super) const ANDROID_UI: &str = "android_ui";
pub(super) const ANDROID_TAP: &str = "android_tap";
pub(super) const ANDROID_SWIPE: &str = "android_swipe";
pub(super) const ANDROID_TYPE: &str = "android_type";
pub(super) const ANDROID_KEY: &str = "android_key";

/// The tools' names.
pub(super) const NAMES: [&str; 7] = [
    ANDROID_DEVICES,
    ANDROID_LOOK,
    ANDROID_UI,
    ANDROID_TAP,
    ANDROID_SWIPE,
    ANDROID_TYPE,
    ANDROID_KEY,
];

/// How long the acting tools wait for the display to stop changing, at
/// the most, unless the agent says.
const SETTLE_MS: u32 = 2000;

/// The most of what the agent may ask: of a wait, in milliseconds, and of
/// taps or keys at once.
const MOST_MS: u32 = 10_000;
const MOST_TIMES: u32 = 100;

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
        "description": format!("Pixels {way}, in the picture android_look answers."),
    })
}

/// How long the tool waits for the display to settle, as the schemas say
/// it, `default` unless the agent says.
fn settle_property(default: u32) -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "maximum": MOST_MS,
        "description": format!(
            "Before the picture, wait up to this many ms for the display to stop changing; \
             0 answers at once. Default {default}."
        ),
    })
}

/// `properties`, with those that name an element by its text or resource
/// id, as the schemas say them, each named after `prefix`, `what` is done
/// with it.
fn with_element(mut properties: Value, prefix: &str, what: &str) -> Value {
    if let Some(properties) = properties.as_object_mut() {
        properties.insert(
            format!("{prefix}text"),
            json!({
                "type": "string",
                "description": format!(
                    "{what} the element with this text or content description, whole, or \
                     else in part, as android_ui lists it."
                ),
            }),
        );
        properties.insert(
            format!("{prefix}resource_id"),
            json!({
                "type": "string",
                "description": format!(
                    "{what} the element with this resource id, as `save` or \
                     `com.example:id/save`."
                ),
            }),
        );
    }
    properties
}

/// A whole number of `most` at the most, as the schemas say it.
fn count(most: u32, description: &str) -> Value {
    json!({ "type": "integer", "minimum": 0, "maximum": most, "description": description })
}

/// The tools, as the agent learns of them.
pub(super) fn tools() -> impl Iterator<Item = Value> {
    seeing_tools().into_iter().chain(acting_tools())
}

/// The tools that find a device and look at it.
fn seeing_tools() -> [Value; 3] {
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
                and answers a picture of it, so that the human sees what you see. Every \
                android tool's x and y are pixels of this picture, from its top left \
                corner; never the device's own pixels. An emulator just started takes a \
                while to boot: look again until its display shows.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "device": device_property(),
                    "settle_ms": settle_property(0),
                    "save_to": {
                        "type": "string",
                        "description": "An absolute path to also save the picture to, as PNG.",
                    },
                },
                "required": ["device"],
            },
        }),
        json!({
            "name": ANDROID_UI,
            "description": "Lists the elements on an Android device's screen, as \
                uiautomator sees them: each one's class, text, content description, \
                resource id, middle (`at x,y`) and bounds in the picture's pixels, and \
                whether it is clickable or focused. Tap or type into one by its text or \
                resource id rather than guessing from the picture. Takes a few seconds.",
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
            "description": "Taps an Android device's display, at x and y or on an element \
                by its text or resource id, where the human sees your pointer on its tab, \
                and answers a picture of the display once it settles. For a long press, \
                give hold_ms; for quick repeated taps, as to test a button against a \
                double submit, give times and interval_ms.",
            "inputSchema": {
                "type": "object",
                "properties": with_element(json!({
                    "device": device_property(),
                    "x": along("from the left edge"),
                    "y": along("down from the top edge"),
                    "times": count(MOST_TIMES, "How many taps. Default 1."),
                    "interval_ms": count(MOST_MS, "Ms between taps. Default 100."),
                    "hold_ms": count(
                        MOST_MS,
                        "Ms the finger stays down each tap; 800 long-presses. Default 0."
                    ),
                    "settle_ms": settle_property(SETTLE_MS),
                }), "", "Tap the middle of"),
                "required": ["device"],
            },
        }),
        json!({
            "name": ANDROID_SWIPE,
            "description": "Swipes a finger across an Android device's display, as to \
                scroll a list or pull a sheet down, and answers a picture of the display \
                once it settles. A list scrolls down as the finger goes up.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "device": device_property(),
                    "x": along("from the left edge, where the finger lands"),
                    "y": along("down from the top edge, where the finger lands"),
                    "to_x": along("from the left edge, where the finger lifts"),
                    "to_y": along("down from the top edge, where the finger lifts"),
                    "settle_ms": settle_property(SETTLE_MS),
                },
                "required": ["device", "x", "y", "to_x", "to_y"],
            },
        }),
        json!({
            "name": ANDROID_TYPE,
            "description": "Types text on an Android device, into the field that has the \
                keyboard, or into the one into_text or into_resource_id names, tapped first; it \
                waits for a field to have the keyboard, and fails if none does. Answers a \
                picture of the display once it settles. For Enter or Back, use \
                android_key.",
            "inputSchema": {
                "type": "object",
                "properties": with_element(json!({
                    "device": device_property(),
                    "text": { "type": "string", "description": "The text to type." },
                    "clear_first": {
                        "type": "boolean",
                        "description": "Empty the field before typing. Default false.",
                    },
                    "settle_ms": settle_property(SETTLE_MS),
                }), "into_", "First tap"),
                "required": ["device", "text"],
            },
        }),
        json!({
            "name": ANDROID_KEY,
            "description": "Presses a key on an Android device, and answers a picture of the \
                display once it settles.",
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
                    "times": count(MOST_TIMES, "How many times to press it. Default 1."),
                    "settle_ms": settle_property(SETTLE_MS),
                },
                "required": ["device", "key"],
            },
        }),
    ]
}

/// The whole number `name` of `arguments`, which the tool needs.
fn needed(arguments: &Value, name: &str) -> Result<u32, String> {
    optional(arguments, name, u32::MAX)?.ok_or_else(|| format!("no `{name}`"))
}

/// The whole number `name` of `arguments`, `most` at the most, if given.
fn optional(arguments: &Value, name: &str, most: u32) -> Result<Option<u32>, String> {
    let Some(value) = arguments.get(name).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value <= most)
        .map(Some)
        .ok_or_else(|| format!("`{name}` is a whole number from 0 to {most}"))
}

/// The element `arguments` name by its text or resource id, those named
/// after `prefix`, if they do.
fn element(arguments: &Value, prefix: &str) -> Result<Option<Element>, String> {
    let text = |field: &str| {
        arguments
            .get(format!("{prefix}{field}"))
            .and_then(Value::as_str)
    };
    match (text("text"), text("resource_id")) {
        (Some(_), Some(_)) => Err(format!(
            "give `{prefix}text` or `{prefix}resource_id`, not both"
        )),
        (Some(text), None) => Ok(Some(Element::Text(text.to_owned()))),
        (None, Some(id)) => Ok(Some(Element::Id(id.to_owned()))),
        (None, None) => Ok(None),
    }
}

/// What the tool `name` does, as `arguments` say.
fn act(name: &str, arguments: &Value) -> Result<AndroidAct, String> {
    if name == ANDROID_DEVICES {
        return Ok(AndroidAct::List);
    }
    let string = |field: &str| {
        arguments
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("no `{field}`"))
    };
    let times = || Ok::<_, String>(optional(arguments, "times", MOST_TIMES)?.unwrap_or(1));
    let device = string("device")?;
    let (act, settle) = match name {
        ANDROID_LOOK => (DeviceAct::Look, 0),
        ANDROID_UI => (DeviceAct::Ui, 0),
        ANDROID_TAP => {
            let at = match (
                element(arguments, "")?,
                arguments.get("x"),
                arguments.get("y"),
            ) {
                (Some(element), None, None) => Spot::On(element),
                (Some(_), _, _) => {
                    return Err("give x and y, or an element, not both".to_owned());
                }
                (None, _, _) => Spot::At {
                    x: needed(arguments, "x")?,
                    y: needed(arguments, "y")?,
                },
            };
            let tap = DeviceAct::Tap {
                at,
                times: times()?,
                interval_ms: optional(arguments, "interval_ms", MOST_MS)?.unwrap_or(100),
                hold_ms: optional(arguments, "hold_ms", MOST_MS)?.unwrap_or(0),
            };
            (tap, SETTLE_MS)
        }
        ANDROID_SWIPE => {
            let swipe = DeviceAct::Swipe {
                x: needed(arguments, "x")?,
                y: needed(arguments, "y")?,
                to_x: needed(arguments, "to_x")?,
                to_y: needed(arguments, "to_y")?,
            };
            (swipe, SETTLE_MS)
        }
        ANDROID_TYPE => {
            let clear_first = match arguments.get("clear_first") {
                None | Some(Value::Null) => false,
                Some(value) => value.as_bool().ok_or("`clear_first` is true or false")?,
            };
            let typed = DeviceAct::Type {
                text: string("text")?,
                into: element(arguments, "into_")?,
                clear_first,
            };
            (typed, SETTLE_MS)
        }
        ANDROID_KEY => {
            let key = DeviceAct::Key {
                key: string("key")?,
                times: times()?,
            };
            (key, SETTLE_MS)
        }
        _ => return Err(format!("no tool {name}")),
    };
    Ok(AndroidAct::On {
        device,
        act,
        settle_ms: optional(arguments, "settle_ms", MOST_MS)?.unwrap_or(settle),
    })
}

/// Asks the stage to do what the tool `name` does, as `arguments` say,
/// and answers what the agent sees then: a look saves the picture where
/// `save_to` says too, if it does.
pub(super) async fn call(
    name: &str,
    arguments: &Value,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<Seen, String> {
    let act = act(name, arguments)?;
    let save_to = match arguments.get("save_to").and_then(Value::as_str) {
        Some(path) if name == ANDROID_LOOK => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err("`save_to` is an absolute path".to_owned());
            }
            Some(path)
        }
        _ => None,
    };
    let (answer, seen) = oneshot::channel();
    ask(requests, ToolRequest::Android { act, answer })?;
    let mut seen = seen
        .await
        .map_err(|_| "the stage went away before it answered".to_owned())??;
    if let (Some(path), Some(png)) = (save_to, &seen.png) {
        save(&path, png)?;
        let _ = write!(seen.text, " Saved to {}.", path.display());
    }
    Ok(seen)
}

/// Writes `png`, as base64, to `path`, as PNG.
fn save(path: &Path, png: &str) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png)
        .map_err(|error| format!("the picture is not whole: {error}"))?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("cannot make {}: {error}", folder.display()))?;
    }
    std::fs::write(path, bytes)
        .map_err(|error| format!("cannot save to {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(act: DeviceAct, settle_ms: u32) -> AndroidAct {
        AndroidAct::On {
            device: "Pixel_9".to_owned(),
            act,
            settle_ms,
        }
    }

    #[test]
    fn reads_a_swipe_as_the_agent_gives_it() {
        let swipe = act(
            ANDROID_SWIPE,
            &json!({ "device": "Pixel_9", "x": 10, "y": 900, "to_x": 10, "to_y": 100 }),
        );
        let expected = DeviceAct::Swipe {
            x: 10,
            y: 900,
            to_x: 10,
            to_y: 100,
        };
        assert_eq!(swipe, Ok(on(expected, SETTLE_MS)));
    }

    #[test]
    fn a_tap_is_one_short_one_unless_the_agent_says_how_many_and_how_long() {
        let tap = act(ANDROID_TAP, &json!({ "device": "Pixel_9", "x": 1, "y": 2 }));
        let once = DeviceAct::Tap {
            at: Spot::At { x: 1, y: 2 },
            times: 1,
            interval_ms: 100,
            hold_ms: 0,
        };
        assert_eq!(tap, Ok(on(once, SETTLE_MS)));
        let held = act(
            ANDROID_TAP,
            &json!({
                "device": "Pixel_9", "text": "保存", "times": 2, "interval_ms": 30,
                "hold_ms": 800, "settle_ms": 0,
            }),
        );
        let twice = DeviceAct::Tap {
            at: Spot::On(Element::Text("保存".to_owned())),
            times: 2,
            interval_ms: 30,
            hold_ms: 800,
        };
        assert_eq!(held, Ok(on(twice, 0)));
    }

    #[test]
    fn a_tap_is_at_a_point_or_on_an_element_not_both() {
        let both = act(
            ANDROID_TAP,
            &json!({ "device": "Pixel_9", "x": 1, "y": 2, "resource_id": "save" }),
        );
        assert!(both.is_err());
        let two = act(
            ANDROID_TAP,
            &json!({ "device": "Pixel_9", "text": "a", "resource_id": "save" }),
        );
        assert!(two.is_err());
    }

    #[test]
    fn typing_names_its_field_apart_from_its_text() {
        let typed = act(
            ANDROID_TYPE,
            &json!({
                "device": "Pixel_9", "text": "S04", "into_resource_id": "staff_id",
                "clear_first": true,
            }),
        );
        let expected = DeviceAct::Type {
            text: "S04".to_owned(),
            into: Some(Element::Id("staff_id".to_owned())),
            clear_first: true,
        };
        assert_eq!(typed, Ok(on(expected, SETTLE_MS)));
    }

    #[test]
    fn a_look_does_not_wait_unless_the_agent_says() {
        let look = act(ANDROID_LOOK, &json!({ "device": "Pixel_9" }));
        assert_eq!(look, Ok(on(DeviceAct::Look, 0)));
        let key = act(
            ANDROID_KEY,
            &json!({ "device": "Pixel_9", "key": "Backspace", "times": 14 }),
        );
        let expected = DeviceAct::Key {
            key: "Backspace".to_owned(),
            times: 14,
        };
        assert_eq!(key, Ok(on(expected, SETTLE_MS)));
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
        assert!(
            act(
                ANDROID_KEY,
                &json!({ "device": "Pixel_9", "key": "Enter", "times": 1000 })
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
        let expected = DeviceAct::Key {
            key: "GoBack".to_owned(),
            times: 1,
        };
        assert_eq!(act, on(expected, SETTLE_MS));
        let seen = Seen {
            text: "Pixel_9".to_owned(),
            png: None,
        };
        answer.send(Ok(seen.clone())).unwrap();
        assert_eq!(called.await.unwrap(), Ok(seen));
    }

    #[tokio::test]
    async fn a_look_saves_its_picture_where_the_agent_says() {
        let folder = crate::testing::TempDir::new();
        let path = folder.path().join("shots/look.png");
        let (sent, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let arguments = json!({ "device": "Pixel_9", "save_to": path });
        let called = tokio::spawn(async move { call(ANDROID_LOOK, &arguments, &sent).await });
        let Some(ToolRequest::Android { answer, .. }) = asked.recv().await else {
            panic!("the stage was not asked");
        };
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG");
        let seen = Seen {
            text: "Pixel_9's display.".to_owned(),
            png: Some(png),
        };
        answer.send(Ok(seen)).unwrap();
        let seen = called.await.unwrap().unwrap();
        assert!(
            seen.text
                .ends_with(&format!("Saved to {}.", path.display()))
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"\x89PNG");
    }

    #[tokio::test]
    async fn a_look_saves_only_to_an_absolute_path() {
        let (sent, _asked) = tokio::sync::mpsc::unbounded_channel();
        let arguments = json!({ "device": "Pixel_9", "save_to": "look.png" });
        assert!(call(ANDROID_LOOK, &arguments, &sent).await.is_err());
    }
}
