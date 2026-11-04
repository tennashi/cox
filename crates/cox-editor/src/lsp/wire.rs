//! The messages on the server's standard input and output: JSON-RPC, each
//! after a header saying its length. One thread writes what the client
//! sends, and another reads what the server writes: the answers to the
//! client's requests, which it hands on, and the server's own requests
//! and notifications, which it answers and acts on.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use serde_json::{Value, json};
use tokio::sync::watch;
use url::Url;

use super::{Error, Server, Shared, Status, lock};

/// Writes each message of `to_write` to `input`, the server's, until the
/// client is gone or the server stopped reading.
pub(super) fn write(mut input: impl Write, to_write: &mpsc::Receiver<Vec<u8>>) {
    while let Ok(message) = to_write.recv() {
        if input
            .write_all(&message)
            .and_then(|()| input.flush())
            .is_err()
        {
            break;
        }
    }
}

/// Reads what the server writes: answers to the client's requests, which
/// it hands on, and its own requests and notifications.
pub(super) struct Reader {
    pub(super) shared: Arc<Shared>,
    pub(super) status: watch::Sender<Status>,
    /// The work the server said it began and has not ended, in order, by
    /// its token, with its title, and what the status says of it.
    pub(super) progress: Vec<(Value, String, String)>,
    /// The server, whose settings it answers when it asks for them.
    pub(super) server: Server,
    pub(super) root: PathBuf,
}

impl Reader {
    pub(super) fn read(mut self, output: impl Read) {
        let mut output = BufReader::new(output);
        while let Some(message) = read_message(&mut output) {
            self.heard(message);
        }
        // Once the server stopped, nothing more is answered.
        let waiting = lock(&self.shared.waiting).take();
        for (_, answer) in waiting.into_iter().flatten() {
            let _ = answer.send(Err(Error::Stopped));
        }
        self.status.send_replace(Status::Stopped);
    }

    fn heard(&mut self, mut message: Value) {
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id").cloned();
        match (method, id) {
            // An answer.
            (None, Some(id)) => {
                let Some(id) = id.as_i64() else {
                    return;
                };
                let answer = lock(&self.shared.waiting)
                    .as_mut()
                    .and_then(|waiting| waiting.remove(&id));
                let Some(answer) = answer else {
                    return;
                };
                let answered = match message.get_mut("error") {
                    Some(error) => Err(Error::Answered {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("the server failed")
                            .to_owned(),
                    }),
                    None => Ok(message.get_mut("result").map_or(Value::Null, Value::take)),
                };
                let _ = answer.send(answered);
            }
            (Some(method), Some(id)) => {
                let answer = self.answer(method, message.get("params"));
                let mut answered = json!({ "jsonrpc": "2.0", "id": id });
                match answer {
                    Ok(result) => answered["result"] = result,
                    Err(message) => {
                        answered["error"] = json!({ "code": -32601, "message": message });
                    }
                }
                self.shared.send(&answered);
            }
            (Some(method), None) => {
                let method = method.to_owned();
                self.notified(&method, message.get("params").unwrap_or(&Value::Null));
            }
            (None, None) => {}
        }
    }

    /// The answer to the server's request `method`, with `params`.
    fn answer(&self, method: &str, params: Option<&Value>) -> Result<Value, String> {
        match method {
            // The settings of each section asked for.
            "workspace/configuration" => {
                let items = params
                    .and_then(|params| params.get("items"))
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                Ok(items
                    .iter()
                    .map(|item| {
                        self.server
                            .settings_of(item.get("section").and_then(Value::as_str))
                    })
                    .collect())
            }
            "client/registerCapability" => {
                let mut registered = lock(&self.shared.registered);
                for method in methods(params, "registrations") {
                    registered.insert(method);
                }
                Ok(Value::Null)
            }
            "client/unregisterCapability" => {
                let mut registered = lock(&self.shared.registered);
                // The protocol spells it so.
                for method in methods(params, "unregisterations") {
                    registered.remove(&method);
                }
                Ok(Value::Null)
            }
            "workspace/workspaceFolders" => Ok(Url::from_directory_path(&self.root)
                .map_or(Value::Null, |uri| json!([{ "uri": uri, "name": "" }]))),
            "window/workDoneProgress/create" | "window/showMessageRequest" => Ok(Value::Null),
            _ => Err(format!("Cox does not answer {method}")),
        }
    }

    /// The server's notification `method`, with `params`.
    fn notified(&mut self, method: &str, params: &Value) {
        match method {
            "$/progress" => self.progressed(params),
            "window/showMessage" | "window/logMessage" => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    tracing::debug!("{message}");
                }
            }
            _ => {}
        }
    }

    /// The server says how its work goes, as `params`.
    fn progressed(&mut self, params: &Value) {
        let Some(token) = params.get("token") else {
            return;
        };
        let value = &params["value"];
        let at = self.progress.iter().position(|(begun, ..)| begun == token);
        match (value.get("kind").and_then(Value::as_str), at) {
            (Some("begin"), _) => {
                let title = value["title"].as_str().unwrap_or_default().to_owned();
                let said = said(&title, value);
                self.progress.push((token.clone(), title, said));
            }
            (Some("report"), Some(at)) => {
                let (_, title, said_now) = &mut self.progress[at];
                *said_now = said(title, value);
            }
            (Some("end"), Some(at)) => {
                self.progress.remove(at);
            }
            _ => return,
        }
        let status = match self.progress.last() {
            Some((.., said)) => Status::Working(said.clone()),
            None => Status::Ready,
        };
        self.status.send_if_modified(|now| {
            let changed = *now != status;
            *now = status;
            changed
        });
    }
}

/// The methods of the registrations `params` lists under `key`.
fn methods(params: Option<&Value>, key: &str) -> Vec<String> {
    params
        .and_then(|params| params.get(key))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|registration| registration.get("method")?.as_str())
        .map(str::to_owned)
        .collect()
}

/// What the status says of work called `title`, as `value` says it goes.
fn said(title: &str, value: &Value) -> String {
    let message = value.get("message").and_then(Value::as_str);
    let percentage = value.get("percentage").and_then(Value::as_u64);
    match (message, percentage) {
        (Some(message), _) => format!("{title}: {message}"),
        (None, Some(percentage)) => format!("{title}: {percentage}%"),
        (None, None) => title.to_owned(),
    }
}

/// Reads the next message from `output`, or `None` once the server
/// stopped writing.
pub(super) fn read_message(output: &mut impl BufRead) -> Option<Value> {
    loop {
        let mut length = None;
        let mut header = String::new();
        loop {
            header.clear();
            if output.read_line(&mut header).ok()? == 0 {
                return None;
            }
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let Some(length) = length else {
            continue;
        };
        let mut body = vec![0; length];
        output.read_exact(&mut body).ok()?;
        // A message that is not JSON is skipped.
        if let Ok(message) = serde_json::from_slice(&body) {
            return Some(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use serde_json::json;

    use super::read_message;

    #[test]
    fn reads_messages_after_their_headers() {
        let written = "Content-Length: 2\r\nContent-Type: x\r\n\r\n{}\
                       content-length: 8\r\n\r\nnot json\
                       Content-Length: 9\r\n\r\n[1, 2, 3]";
        let mut output = BufReader::new(written.as_bytes());
        assert_eq!(read_message(&mut output), Some(json!({})));
        // The message that is not JSON is skipped.
        assert_eq!(read_message(&mut output), Some(json!([1, 2, 3])));
        assert_eq!(read_message(&mut output), None);
    }
}
