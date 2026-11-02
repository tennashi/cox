//! Cox asked how it stands, and driven, by an agent that debugs it: one
//! that runs in a terminal, in a chat of another Cox, or on a machine with
//! no screen, where Cox runs on a display drawn in memory. It asks for
//! the state of the window, in words: the chats and how each stands, the
//! conversation shown, the input, the stage's tabs; takes a screenshot of
//! the window; and sends keys, text and clicks as the human's, and GPUI's
//! actions by name, which reach the window whether it has the focus of
//! the screen or not.
//!
//! The agent asks on a Unix socket, `inspect.sock` where this Cox keeps
//! what lasts (see [`crate::storage`]), which Cox opens as its window
//! does, and speaks MCP on, as the server of each chat's tools does (see
//! [`crate::agent::mcp`]). So `cox --mcp <socket>` reaches it as an MCP
//! server too, for an agent set up with one. And `cox --inspect` is a
//! client of it, for an agent that has a shell: [`client`] asks the tools
//! of the Cox that keeps what lasts where this `cox` does, or where
//! `--data` says.

use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::thread;

use base64::Engine as _;
use gpui::{
    AnyView, App, AppContext as _, AsyncApp, Bounds, Keystroke, Modifiers, Pixels, Point, Window,
    WindowHandle, px,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

use crate::agent::mcp::{self, Answered, INVALID_PARAMS, Serves, answered};
use crate::workspace::Workspace;

pub mod client;
mod state;

pub use state::describe;

/// What the socket is called, where this Cox keeps what lasts.
pub const SOCKET: &str = "inspect.sock";

/// The option that runs `cox` as the client.
pub const OPTION: &str = "--inspect";

const STATE: &str = "state";
const SCREENSHOT: &str = "screenshot";
const KEY: &str = "key";
const TYPE: &str = "type";
const CLICK: &str = "click";
const ACTION: &str = "action";

/// What a tool asks of the window.
enum Inquiry {
    /// How Cox stands, in words, with the conversation of the chat that
    /// goes by `chat`, or the chat shown.
    State {
        chat: Option<u64>,
        answer: oneshot::Sender<String>,
    },
    /// Where the window is on the screen.
    Window { answer: oneshot::Sender<Placed> },
    /// Keystrokes, as the human's; answers whether each was handled.
    Keys {
        keystrokes: Vec<Keystroke>,
        answer: oneshot::Sender<String>,
    },
    /// A GPUI action, by its name, on what has the keyboard.
    Action {
        name: String,
        answer: oneshot::Sender<Result<String, String>>,
    },
}

/// Where the window is on the screen, as the screen is captured, and
/// clicked on.
struct Placed {
    /// Its bounds, in the screen's coordinates, in points.
    bounds: Bounds<Pixels>,
    /// How many pixels of the screen a point is.
    scale: f32,
}

/// What the tools know of Cox, past the window.
struct Known {
    devtools: Option<u16>,
}

/// Opens the socket, answering for `window`, until Cox quits. Without
/// a place to keep what lasts, or with the socket in another Cox's use,
/// Cox answers nowhere, and says so.
pub fn start(window: WindowHandle<Workspace>, devtools: Option<u16>, cx: &mut App) {
    let Some(socket) = crate::storage::path(SOCKET) else {
        tracing::error!("Cox cannot be asked how it stands: it has no place for the socket");
        return;
    };
    let listener = match unused(&socket).and_then(|()| mcp::listen(&socket)) {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!("Cox cannot be asked how it stands: {error:#}");
            return;
        }
    };
    let (asks, mut inquiries) = unbounded_channel::<Inquiry>();
    let inspector = Arc::new(Inspector { asks: asks.clone() });
    let span = tracing::info_span!("inspect");
    let stops = socket.clone();
    thread::spawn(move || {
        let _entered = span.enter();
        if let Err(error) = mcp::serve(listener, inspector, asks.closed()) {
            tracing::error!("Cox stopped answering how it stands: {error:#}");
        }
        let _ = fs::remove_file(&stops);
    });
    tracing::info!(socket = %socket.display(), "Cox answers how it stands");
    // Quitting ends the process before the thread gets to it, on macOS.
    cx.on_app_quit(move |_| {
        let _ = fs::remove_file(&socket);
        async {}
    })
    .detach();
    let known = Known { devtools };
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(inquiry) = inquiries.recv().await {
            // Not through the workspace: a key's action updates it.
            let _ = cx.update_window(window.into(), |root, window, cx| {
                answer(inquiry, &known, &root, window, cx);
            });
        }
    })
    .detach();
}

/// Makes sure no Cox answers at `socket`, taking away what a Cox that
/// ended without taking it away left there.
fn unused(socket: &std::path::Path) -> anyhow::Result<()> {
    match UnixStream::connect(socket) {
        Ok(_) => anyhow::bail!("another Cox answers at {}", socket.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => fs::remove_file(socket)
            .or_else(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|error| anyhow::anyhow!("cannot take away {}: {error}", socket.display())),
    }
}

/// Answers `inquiry` of `window`, whose root view is `root`.
fn answer(inquiry: Inquiry, known: &Known, root: &AnyView, window: &mut Window, cx: &mut App) {
    match inquiry {
        Inquiry::State { chat, answer } => {
            let told = match root.clone().downcast::<Workspace>() {
                Ok(workspace) => {
                    state::describe(workspace.read(cx), chat, known.devtools, window, cx)
                }
                Err(_) => "The window shows no workspace.".to_owned(),
            };
            let _ = answer.send(told);
        }
        Inquiry::Window { answer } => {
            let _ = answer.send(Placed {
                bounds: window.bounds(),
                scale: window.scale_factor(),
            });
        }
        Inquiry::Keys { keystrokes, answer } => {
            let mut told = Vec::new();
            for keystroke in keystrokes {
                let handled = window.dispatch_keystroke(keystroke.clone(), cx);
                told.push(format!(
                    "{keystroke}: {}",
                    if handled { "handled" } else { "not handled" }
                ));
            }
            let _ = answer.send(told.join("\n"));
        }
        Inquiry::Action { name, answer } => {
            let _ = answer.send(act(&name, window, cx));
        }
    }
}

/// Dispatches the action named `name`, by its full name, as
/// `cox::NewChat`, or the end of it when only one action's ends so.
fn act(name: &str, window: &mut Window, cx: &mut App) -> Result<String, String> {
    let named: Vec<&str> = cx
        .all_action_names()
        .iter()
        .copied()
        .filter(|full| *full == name || full.ends_with(&format!("::{name}")))
        .collect();
    let full = match named.as_slice() {
        [full] => *full,
        [] => {
            return Err(format!(
                "no action {name}; the actions are {}",
                cx.all_action_names().join(", ")
            ));
        }
        many => return Err(format!("{name} could be {}", many.join(" or "))),
    };
    let action = cx
        .build_action(full, None)
        .map_err(|error| format!("{full} cannot be built: {error}"))?;
    window.dispatch_action(action, cx);
    Ok(format!("Dispatched {full}."))
}

/// The server's tools: each asks the window, and answers what it said.
struct Inspector {
    asks: UnboundedSender<Inquiry>,
}

impl Serves for Inspector {
    fn tools(&self) -> Value {
        tools()
    }

    fn call<'a>(
        &'a self,
        params: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Answered> + Send + 'a>> {
        Box::pin(self.call(params))
    }
}

impl Inspector {
    async fn call(&self, params: &Value) -> Answered {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").unwrap_or(&Value::Null);
        let done = match name {
            STATE => {
                let chat = arguments.get("chat").and_then(Value::as_u64);
                self.ask(|answer| Inquiry::State { chat, answer })
                    .await
                    .map(Ok)
            }
            SCREENSHOT => {
                return Ok(match self.screenshot().await? {
                    Ok(png) => json!({
                        "content": [{ "type": "image", "data": png, "mimeType": "image/png" }],
                        "isError": false,
                    }),
                    Err(text) => answered(&text, true),
                });
            }
            KEY => self.keys(arguments).await,
            TYPE => self.types(arguments).await,
            CLICK => self.click(arguments).await,
            ACTION => {
                let name = string(arguments, "name")?;
                self.ask(|answer| Inquiry::Action { name, answer }).await
            }
            _ => return Err((INVALID_PARAMS, format!("no tool {name}"))),
        }?;
        Ok(match done {
            Ok(text) => answered(&text, false),
            Err(text) => answered(&text, true),
        })
    }

    /// Asks the window what `inquiry` asks with the answer's sender, and
    /// answers what it answered, or the error of a window that is gone.
    async fn ask<T>(
        &self,
        inquiry: impl FnOnce(oneshot::Sender<T>) -> Inquiry,
    ) -> Result<T, (i64, String)> {
        let (sender, answer) = oneshot::channel();
        let gone = || (mcp::INTERNAL_ERROR, "the window is gone".to_owned());
        self.asks.send(inquiry(sender)).map_err(|_| gone())?;
        answer.await.map_err(|_| gone())
    }

    async fn keys(&self, arguments: &Value) -> Result<Result<String, String>, (i64, String)> {
        let keys = string(arguments, "keys")?;
        let mut keystrokes = Vec::new();
        for key in keys.split_whitespace() {
            match Keystroke::parse(key) {
                Ok(keystroke) => keystrokes.push(keystroke),
                Err(error) => return Ok(Err(error.to_string())),
            }
        }
        if keystrokes.is_empty() {
            return Ok(Err("no keys given".to_owned()));
        }
        self.ask(|answer| Inquiry::Keys { keystrokes, answer })
            .await
            .map(Ok)
    }

    async fn types(&self, arguments: &Value) -> Result<Result<String, String>, (i64, String)> {
        let text = string(arguments, "text")?;
        let keystrokes: Vec<Keystroke> = text
            .chars()
            .map(|character| Keystroke {
                modifiers: Modifiers::none(),
                key: if character == '\n' {
                    "enter".to_owned()
                } else {
                    character.to_string()
                },
                key_char: Some(character.to_string()),
            })
            .collect();
        let count = keystrokes.len();
        self.ask(|answer| Inquiry::Keys { keystrokes, answer })
            .await?;
        Ok(Ok(format!("Typed {count} characters.")))
    }

    /// Clicks on the window where it is on the screen, as the screen's
    /// own tools do.
    async fn click(&self, arguments: &Value) -> Result<Result<String, String>, (i64, String)> {
        let number = |name: &str| {
            arguments
                .get(name)
                .and_then(Value::as_f64)
                .ok_or_else(|| (INVALID_PARAMS, format!("{name} is a number")))
        };
        #[expect(clippy::cast_possible_truncation, reason = "a point of the window")]
        let position = Point::new(px(number("x")? as f32), px(number("y")? as f32));
        let button = match arguments.get("button").and_then(Value::as_str) {
            None | Some("left") => 1,
            Some("middle") => 2,
            Some("right") => 3,
            Some(other) => return Ok(Err(format!("no button {other}: left, right or middle"))),
        };
        let count = arguments
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .clamp(1, 3);
        let placed = self.ask(|answer| Inquiry::Window { answer }).await?;
        let clicked = tokio::task::spawn_blocking(move || click(&placed, position, button, count))
            .await
            .map_err(|error| (mcp::INTERNAL_ERROR, error.to_string()))?;
        Ok(clicked.map(|()| {
            format!(
                "Clicked at {}, {}.",
                f32::from(position.x),
                f32::from(position.y)
            )
        }))
    }

    /// Captures the window, as the screen shows it, and answers it as an
    /// image.
    async fn screenshot(&self) -> Result<Result<String, String>, (i64, String)> {
        let placed = self.ask(|answer| Inquiry::Window { answer }).await?;
        let captured = tokio::task::spawn_blocking(move || capture(&placed))
            .await
            .map_err(|error| (mcp::INTERNAL_ERROR, error.to_string()))?;
        Ok(captured.map(|png| base64::engine::general_purpose::STANDARD.encode(png)))
    }
}

/// The screen as it shows the window placed as `placed`, as a PNG: on
/// macOS, the part of the screen where the window is, through
/// `screencapture`, which has to be allowed to record the screen.
#[cfg(target_os = "macos")]
fn capture(placed: &Placed) -> Result<Vec<u8>, String> {
    let bounds = placed.bounds;
    let area = format!(
        "{},{},{},{}",
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height)
    );
    let file = std::env::temp_dir().join(format!("cox-{}-screenshot.png", std::process::id()));
    let ran = std::process::Command::new("screencapture")
        .args(["-x", "-R", &area])
        .arg(&file)
        .output()
        .map_err(|error| format!("cannot run screencapture: {error}"))?;
    if !ran.status.success() {
        return Err(format!(
            "screencapture failed: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        ));
    }
    let png = fs::read(&file).map_err(|error| format!("cannot read the screenshot: {error}"));
    let _ = fs::remove_file(&file);
    png
}

/// The screen as it shows the window placed as `placed`, as a PNG: on
/// X11, the part of the screen where the window is, through `import`, of
/// `ImageMagick`.
#[cfg(not(target_os = "macos"))]
fn capture(placed: &Placed) -> Result<Vec<u8>, String> {
    // On a display drawn in memory, as Xvfb's, what the window drew
    // shows only once the pointer has moved: a pixel and back does, and
    // the window has drawn again by the time it is captured.
    for step in ["1", "-1"] {
        let _ = std::process::Command::new("xdotool")
            .args(["mousemove_relative", "--sync", "--", step, "0"])
            .output();
    }
    thread::sleep(std::time::Duration::from_millis(200));
    let bounds = placed.bounds;
    let pixels = |points: Pixels| (f32::from(points) * placed.scale).round();
    let area = format!(
        "{}x{}+{}+{}",
        pixels(bounds.size.width),
        pixels(bounds.size.height),
        pixels(bounds.origin.x),
        pixels(bounds.origin.y)
    );
    let ran = std::process::Command::new("import")
        .args(["-window", "root", "-crop", &area, "+repage", "png:-"])
        .output()
        .map_err(|error| format!("cannot run import, of ImageMagick: {error}"))?;
    if !ran.status.success() {
        return Err(format!(
            "import failed: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        ));
    }
    Ok(ran.stdout)
}

/// Clicks `button`, as X11 numbers them, `count` times at `position` of
/// the window placed as `placed`, through `xdotool`, which moves the
/// pointer there and presses, as the human would.
#[cfg(not(target_os = "macos"))]
fn click(placed: &Placed, position: Point<Pixels>, button: u8, count: u64) -> Result<(), String> {
    let x = (f32::from(placed.bounds.origin.x) + f32::from(position.x)) * placed.scale;
    let y = (f32::from(placed.bounds.origin.y) + f32::from(position.y)) * placed.scale;
    let ran = std::process::Command::new("xdotool")
        .args(["mousemove", "--sync"])
        .arg(format!("{}", x.round()))
        .arg(format!("{}", y.round()))
        .args(["click", "--repeat", &count.to_string(), "--delay", "80"])
        .arg(button.to_string())
        .output()
        .map_err(|error| format!("cannot run xdotool: {error}"))?;
    if !ran.status.success() {
        return Err(format!(
            "xdotool failed: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        ));
    }
    Ok(())
}

/// On macOS, nothing clicks for the agent yet.
#[cfg(target_os = "macos")]
fn click(
    _placed: &Placed,
    _position: Point<Pixels>,
    _button: u8,
    _count: u64,
) -> Result<(), String> {
    Err("clicking is not done on macOS yet: use the keys, or the screen's own tools".to_owned())
}

/// The string `arguments` gives as `name`.
fn string(arguments: &Value, name: &str) -> Result<String, (i64, String)> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| (INVALID_PARAMS, format!("{name} is a string")))
}

/// The tools, as a client learns of them.
fn tools() -> Value {
    json!([
        {
            "name": STATE,
            "description": "How Cox stands, in words: where it keeps what lasts and this run's \
                log, the window, the chats as the list shows them, and one chat in full: the \
                line above its input, its input, its stage's tabs, and its conversation, entry \
                by entry.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat": {
                        "type": "integer",
                        "description": "The chat to tell in full, by the number it goes by in \
                            the list of chats; the chat shown unless given.",
                    },
                },
            },
        },
        {
            "name": SCREENSHOT,
            "description": "A screenshot of the window, as the screen shows it, as a PNG image.",
            "inputSchema": { "type": "object", "properties": {} },
        },
        {
            "name": KEY,
            "description": "Presses keys, as the human would, on what has the keyboard in the \
                window: one or more keystrokes separated by spaces, each as GPUI writes them, \
                such as \"ctrl-n\", \"enter\", \"escape\", \"cmd-shift-tab\" or \"a\". \
                \"secondary\" is cmd on macOS and ctrl elsewhere. Answers whether each was \
                handled.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keys": { "type": "string", "description": "The keystrokes, separated by spaces." },
                },
                "required": ["keys"],
            },
        },
        {
            "name": TYPE,
            "description": "Types text, as the human would, into what has the keyboard in the \
                window, a character at a time; a newline presses Enter.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The text to type." },
                },
                "required": ["text"],
            },
        },
        {
            "name": CLICK,
            "description": "Clicks at a point of the window, in points from its top left corner, \
                as the screenshot shows it (divide by the scale the state names on a screen that \
                draws a point with several pixels), moving the pointer there as the human \
                would. Not on macOS yet.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "button": { "type": "string", "enum": ["left", "right", "middle"], "description": "Left unless given." },
                    "count": { "type": "integer", "description": "How many times: 2 for a double click." },
                },
                "required": ["x", "y"],
            },
        },
        {
            "name": ACTION,
            "description": "Dispatches one of GPUI's actions by name, as a key bound to it would, \
                on what has the keyboard: by its full name, such as \"cox::NewChat\", or the \
                end of it when only one action's ends so.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                },
                "required": ["name"],
            },
        },
    ])
}

/// Where the socket is, where this Cox keeps what lasts.
#[must_use]
pub fn socket() -> Option<PathBuf> {
    crate::storage::path(SOCKET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_are_those_the_client_knows() {
        let tools = tools();
        let names: Vec<&str> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, [STATE, SCREENSHOT, KEY, TYPE, CLICK, ACTION]);
        for name in names {
            let words: &[&str] = match name {
                STATE | SCREENSHOT => &[name],
                KEY => &[name, "ctrl-n"],
                TYPE => &[name, "hello"],
                CLICK => &[name, "1", "2"],
                _ => &[name, "cox::NewChat"],
            };
            let words: Vec<String> = words.iter().map(|&word| word.to_owned()).collect();
            assert!(
                matches!(client::Call::parse(&words), Ok(client::Call::Tool { name: called, .. }) if called == name),
                "{name}"
            );
        }
    }
}
