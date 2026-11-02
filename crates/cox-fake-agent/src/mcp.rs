//! A client of an MCP server Cox hands the agent, as ACP's `session/new`
//! gives it: the agent starts the server's command and speaks MCP on its
//! standard input and output, a JSON object a line.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

pub struct Server {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    asked: u64,
}

impl Server {
    /// Starts the server `server`, as `session/new` describes it, and
    /// tells it who the agent is.
    pub fn start(server: &Value) -> Result<Self, String> {
        let command = server["command"]
            .as_str()
            .ok_or("the server has no command")?;
        let args = server["args"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str);
        let env = server["env"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|variable| Some((variable["name"].as_str()?, variable["value"].as_str()?)));
        let mut child = Command::new(command)
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot start {command}: {error}"))?;
        let input = child.stdin.take().ok_or("no input to the server")?;
        let output = BufReader::new(child.stdout.take().ok_or("no output from the server")?);
        let mut started = Self {
            child,
            input,
            output,
            asked: 0,
        };
        started.ask(
            "initialize",
            &json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "cox-fake-agent", "version": env!("CARGO_PKG_VERSION") },
            }),
        )?;
        started.tell("notifications/initialized")?;
        Ok(started)
    }

    /// Calls `tool` with `arguments`, and answers the text it answered
    /// with, as `Err` when the tool says it failed.
    pub fn call(
        &mut self,
        tool: &str,
        arguments: &Value,
    ) -> Result<Result<String, String>, String> {
        let result = self.ask(
            "tools/call",
            &json!({ "name": tool, "arguments": arguments }),
        )?;
        let text = result["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|content| content["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(if result["isError"] == true {
            Err(text)
        } else {
            Ok(text)
        })
    }

    fn ask(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        self.asked += 1;
        let id = self.asked;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let mut line = String::new();
            let read = self
                .output
                .read_line(&mut line)
                .map_err(|error| format!("the server went: {error}"))?;
            if read == 0 {
                return Err("the server went".to_owned());
            }
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message["id"] != id {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!("the server answered {method} with {error}"));
            }
            return Ok(message["result"].clone());
        }
    }

    fn tell(&mut self, method: &str) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method }))
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        writeln!(self.input, "{message}")
            .and_then(|()| self.input.flush())
            .map_err(|error| format!("the server went: {error}"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
