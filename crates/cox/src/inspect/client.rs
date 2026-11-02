//! `cox --inspect`: a client of the socket, for an agent that has a
//! shell. It calls one tool of the Cox that keeps what lasts where this
//! `cox` does, or where `--data` says, and prints what the tool answered;
//! an image, such as the screenshot, it writes to a file, and prints the
//! file's path. With no tool named, it prints the tools.

use std::io::{self, BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use base64::Engine as _;
use serde_json::{Value, json};

pub const USAGE: &str = "\
Usage: cox [--data <dir>] --inspect [<tool> [<arguments>...]]
  state [<chat>]                       how Cox stands, with one chat in full
  screenshot [<file>]                  the window as a PNG, cox-screenshot.png unless given
  key <keystroke>...                   presses keys, as ctrl-n or enter
  type <text>                          types text, a newline pressing Enter
  click <x> <y> [left|right|middle] [<count>]
  action <name>                        dispatches a GPUI action, as cox::NewChat
With no tool, the tools are listed, as the Cox asked tells them.";

/// What the client is to call: a tool, with its arguments, and where an
/// image answered goes, or the list of tools.
#[derive(Debug, PartialEq, Eq)]
pub enum Call {
    Tool {
        name: String,
        arguments: Value,
        /// Where an image the tool answers goes.
        image: PathBuf,
    },
    List,
}

impl Call {
    /// What `args`, those after `--inspect`, ask for.
    ///
    /// # Errors
    ///
    /// When they ask for nothing the client does.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let words: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut image = PathBuf::from("cox-screenshot.png");
        let (name, arguments) = match words.as_slice() {
            [] => return Ok(Self::List),
            ["state"] => ("state", json!({})),
            ["state", chat] => {
                let chat: u64 = chat
                    .parse()
                    .map_err(|_| format!("{chat} is not the number of a chat"))?;
                ("state", json!({ "chat": chat }))
            }
            ["screenshot"] => ("screenshot", json!({})),
            ["screenshot", file] => {
                image = PathBuf::from(file);
                ("screenshot", json!({}))
            }
            ["key", keys @ ..] if !keys.is_empty() => ("key", json!({ "keys": keys.join(" ") })),
            ["type", text @ ..] if !text.is_empty() => ("type", json!({ "text": text.join(" ") })),
            ["click", x, y, rest @ ..] => {
                let number = |word: &str| {
                    word.parse::<f64>()
                        .map_err(|_| format!("{word} is not a number of points"))
                };
                let mut arguments = json!({ "x": number(x)?, "y": number(y)? });
                for word in rest {
                    if let Ok(count) = word.parse::<u64>() {
                        arguments["count"] = Value::from(count);
                    } else {
                        arguments["button"] = Value::from(*word);
                    }
                }
                ("click", arguments)
            }
            ["action", name] => ("action", json!({ "name": name })),
            _ => return Err(USAGE.to_owned()),
        };
        Ok(Self::Tool {
            name: name.to_owned(),
            arguments,
            image,
        })
    }
}

/// Runs the client: calls what `args`, those after `--inspect`, ask for,
/// of the Cox at `socket`, and prints the answer.
///
/// # Errors
///
/// When the arguments ask for nothing the client does, no Cox answers at
/// `socket`, or the tool answers an error.
pub fn run(socket: &Path, args: &[String]) -> anyhow::Result<()> {
    let call = Call::parse(args).map_err(|usage| anyhow::anyhow!("{usage}"))?;
    let mut server = Server::connect(socket)?;
    match call {
        Call::List => {
            let listed = server.ask("tools/list", &json!({}))?;
            let tools = listed["tools"].as_array().cloned().unwrap_or_default();
            let mut out = io::stdout().lock();
            for tool in tools {
                let name = tool["name"].as_str().unwrap_or("?");
                let description = tool["description"].as_str().unwrap_or("");
                writeln!(out, "{name}: {description}")?;
            }
            Ok(())
        }
        Call::Tool {
            name,
            arguments,
            image,
        } => {
            let answered = server.ask(
                "tools/call",
                &json!({ "name": name, "arguments": arguments }),
            )?;
            let is_error = answered["isError"] == true;
            let mut out: Box<dyn io::Write> = if is_error {
                Box::new(io::stderr())
            } else {
                Box::new(io::stdout())
            };
            for content in answered["content"].as_array().into_iter().flatten() {
                match content["type"].as_str() {
                    Some("image") => {
                        let data = content["data"].as_str().unwrap_or_default();
                        let png = base64::engine::general_purpose::STANDARD
                            .decode(data)
                            .context("the image cannot be read")?;
                        std::fs::write(&image, png)
                            .with_context(|| format!("cannot write {}", image.display()))?;
                        writeln!(out, "{}", image.display())?;
                    }
                    _ => {
                        if let Some(text) = content["text"].as_str() {
                            writeln!(out, "{text}")?;
                        }
                    }
                }
            }
            if is_error {
                anyhow::bail!("{name} failed");
            }
            Ok(())
        }
    }
}

/// The Cox asked, on its socket.
struct Server {
    input: UnixStream,
    output: BufReader<UnixStream>,
    asked: u64,
}

impl Server {
    fn connect(socket: &Path) -> anyhow::Result<Self> {
        let input = UnixStream::connect(socket)
            .with_context(|| format!("no Cox answers at {}", socket.display()))?;
        let output = BufReader::new(input.try_clone()?);
        let mut server = Self {
            input,
            output,
            asked: 0,
        };
        server.ask(
            "initialize",
            &json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "cox --inspect", "version": env!("CARGO_PKG_VERSION") },
            }),
        )?;
        server.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok(server)
    }

    /// Asks `method` with `params`, and answers its result.
    fn ask(&mut self, method: &str, params: &Value) -> anyhow::Result<Value> {
        self.asked += 1;
        let id = self.asked;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let mut line = String::new();
            if self.output.read_line(&mut line)? == 0 {
                anyhow::bail!("Cox hung up");
            }
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message["id"] != id {
                continue;
            }
            if let Some(error) = message.get("error") {
                anyhow::bail!(
                    "{}",
                    error["message"].as_str().unwrap_or("Cox answered an error")
                );
            }
            return Ok(message["result"].clone());
        }
    }

    fn send(&mut self, message: &Value) -> anyhow::Result<()> {
        writeln!(self.input, "{message}")?;
        self.input.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Call, String> {
        Call::parse(
            &words
                .iter()
                .map(|&word| word.to_owned())
                .collect::<Vec<_>>(),
        )
    }

    fn tool(name: &str, arguments: Value) -> Call {
        Call::Tool {
            name: name.to_owned(),
            arguments,
            image: PathBuf::from("cox-screenshot.png"),
        }
    }

    #[test]
    fn each_tool_takes_its_arguments_as_words() {
        assert_eq!(parse(&[]), Ok(Call::List));
        assert_eq!(parse(&["state"]), Ok(tool("state", json!({}))));
        assert_eq!(
            parse(&["state", "3"]),
            Ok(tool("state", json!({ "chat": 3 })))
        );
        assert_eq!(
            parse(&["key", "ctrl-n", "enter"]),
            Ok(tool("key", json!({ "keys": "ctrl-n enter" })))
        );
        assert_eq!(
            parse(&["type", "hello", "there"]),
            Ok(tool("type", json!({ "text": "hello there" })))
        );
        assert_eq!(
            parse(&["click", "10", "20.5", "right", "2"]),
            Ok(tool(
                "click",
                json!({ "x": 10.0, "y": 20.5, "button": "right", "count": 2 })
            ))
        );
        assert_eq!(
            parse(&["action", "cox::NewChat"]),
            Ok(tool("action", json!({ "name": "cox::NewChat" })))
        );
        assert_eq!(
            parse(&["screenshot", "shot.png"]),
            Ok(Call::Tool {
                name: "screenshot".to_owned(),
                arguments: json!({}),
                image: PathBuf::from("shot.png"),
            })
        );
    }

    #[test]
    fn what_is_not_a_tool_gets_the_usage() {
        assert_eq!(parse(&["dance"]), Err(USAGE.to_owned()));
        assert_eq!(parse(&["key"]), Err(USAGE.to_owned()));
        assert_eq!(
            parse(&["state", "x"]),
            Err("x is not the number of a chat".to_owned())
        );
    }
}
