//! Talking to a language server, which knows a project's code as its
//! compiler does: what a name is, where it is defined, and where it is
//! used. The editor starts the server of a language it knows one for (see
//! [`Language::server`](crate::language::Language::server)), in the
//! project's folder, and talks to it over the Language Server Protocol:
//! JSON-RPC messages on the server's standard input and output.
//!
//! The server reads each file the editor shows as the editor holds it: the
//! editor opens it on the server as an [`OpenFile`], and tells the server of
//! the text each time it changes. It asks of a place in it, as a
//! [`Position`], which counts the bytes of a line in UTF-8, or in UTF-16,
//! as the server does (see [`Encoding`]).
//!
//! Two threads read and write the messages, so that waiting for the server
//! holds up nothing else: a request answers a future, which needs no
//! runtime to wait on.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};
use url::Url;

use crate::buffer::{Buffer, Point};

/// A language server, as the editor starts it.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Server {
    /// The program, found on `PATH`.
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// The files a folder that is a project of the language has, such as
    /// `Cargo.toml` for Rust.
    pub markers: &'static [&'static str],
    /// How the server is to work, as JSON: what it is told when it starts,
    /// and answered when it asks for its settings.
    pub settings: &'static str,
}

/// Why the server did not answer.
#[derive(Debug)]
pub enum Error {
    /// The server could not be started, or talked to.
    Io(std::io::Error),
    /// The server answered with an error.
    Answered { code: i64, message: String },
    /// The server stopped.
    Stopped,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Answered { message, .. } => f.write_str(message),
            Self::Stopped => f.write_str("the server stopped"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// What the server is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Nothing the editor hears of: it answers as it knows the code now.
    Ready,
    /// Work it says it is doing, such as indexing the project, which it
    /// answers better after.
    Working(String),
    /// It stopped, and answers nothing more.
    Stopped,
}

/// A place in a file as the server counts it: a line, from 0, and how far
/// into it, in the server's [`Encoding`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// What the server counts a line in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Encoding {
    /// Bytes, as the editor does.
    Utf8,
    /// UTF-16 code units, which every server knows.
    #[default]
    Utf16,
    /// Characters.
    Utf32,
}

impl Encoding {
    fn named(name: &str) -> Self {
        match name {
            "utf-8" => Self::Utf8,
            "utf-32" => Self::Utf32,
            _ => Self::Utf16,
        }
    }

    /// `point` of `buffer`, as the server counts it.
    #[must_use]
    pub fn position(self, buffer: &Buffer, point: Point) -> Position {
        let point = buffer.clip(point);
        let before = &buffer.line(point.line)[..point.column];
        let character = match self {
            Self::Utf8 => before.len(),
            Self::Utf16 => before.encode_utf16().count(),
            Self::Utf32 => before.chars().count(),
        };
        Position {
            line: count(point.line),
            character: count(character),
        }
    }

    /// The place in `buffer` that `position` is at, as the server counts
    /// it: at the end of its line if past it.
    #[must_use]
    pub fn point(self, buffer: &Buffer, position: Position) -> Point {
        let line = position.line as usize;
        let text = buffer.line(line);
        let character = position.character as usize;
        let column = match self {
            Self::Utf8 => character,
            Self::Utf16 | Self::Utf32 => {
                let mut counted = 0;
                text.char_indices()
                    .find(|&(_, c)| {
                        let past = counted >= character;
                        counted += if self == Self::Utf16 {
                            c.len_utf16()
                        } else {
                            1
                        };
                        past
                    })
                    .map_or(text.len(), |(at, _)| at)
            }
        };
        buffer.clip(Point::new(line, column))
    }
}

/// `n` as the protocol's numbers, which no line goes past.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// What the server says of a name: its type and its documentation, as
/// Markdown, and the stretch of the line it is about, if it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub markdown: String,
    pub range: Option<Range<Position>>,
}

/// A stretch of a file, such as where a name is defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// The file, as an absolute path.
    pub path: PathBuf,
    pub range: Range<Position>,
}

/// A running language server, which the editor talks to.
pub struct Client {
    shared: Arc<Shared>,
    encoding: Encoding,
    status: watch::Receiver<Status>,
    /// The server's process, if the client started it.
    child: Option<Child>,
}

/// What the client shares with its threads and its documents.
struct Shared {
    /// Each message to write, whole.
    outgoing: Mutex<mpsc::Sender<Vec<u8>>>,
    next_id: AtomicI64,
    /// What answers each request the server has not answered yet, or
    /// `None` once the server stopped.
    waiting: Mutex<Option<HashMap<i64, Answer>>>,
    /// The documents open, by their address.
    documents: Mutex<HashMap<Url, Opened>>,
}

/// What answers a request.
type Answer = oneshot::Sender<Result<Value, Error>>;

/// A document open on the server.
struct Opened {
    /// How many [`OpenFile`]s hold it open.
    holders: usize,
    version: i64,
    text: String,
}

impl Shared {
    fn send(&self, message: &Value) {
        let body = message.to_string();
        let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        framed.extend_from_slice(body.as_bytes());
        // Once the writer stopped, the server did too, which the reader
        // tells of.
        let _ = lock(&self.outgoing).send(framed);
    }

    fn notify(&self, method: &str, params: &Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Asks the server `method`, and answers what it answers.
    fn request(
        &self,
        method: &str,
        params: &Value,
    ) -> impl Future<Output = Result<Value, Error>> + use<> {
        let (answer, answered) = oneshot::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        match lock(&self.waiting).as_mut() {
            Some(waiting) => {
                waiting.insert(id, answer);
                self.send(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": method,
                    "params": params,
                }));
            }
            None => {
                let _ = answer.send(Err(Error::Stopped));
            }
        }
        async move { answered.await.unwrap_or(Err(Error::Stopped)) }
    }
}

/// Locks `mutex`, whatever a thread that panicked holding it left in it.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Client {
    /// Starts `server` working in `root`, a project's folder, as an
    /// absolute path, and answers once it is ready to be asked.
    ///
    /// # Errors
    ///
    /// When the server cannot be started, or stops or fails before it is
    /// ready.
    pub async fn start(server: &Server, root: &Path) -> Result<Self, Error> {
        let mut child = std::process::Command::new(server.program)
            .args(server.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let (Some(input), Some(output), Some(errors)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            unreachable!("the server's standard streams are piped");
        };
        let program = server.program;
        std::thread::spawn(move || {
            for line in BufReader::new(errors).lines().map_while(Result::ok) {
                log::debug!("{program}: {line}");
            }
        });
        let settings = serde_json::from_str(server.settings).unwrap_or(Value::Null);
        match Self::over(output, input, root, settings).await {
            Ok(mut client) => {
                client.child = Some(child);
                Ok(client)
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    /// Talks to a server that writes to `output` and reads from `input`,
    /// working in `root`, and answers once it is ready to be asked. It is
    /// told `settings` when it starts, and when it asks for them.
    ///
    /// # Errors
    ///
    /// When the server stops or fails before it is ready.
    pub async fn over(
        output: impl Read + Send + 'static,
        input: impl Write + Send + 'static,
        root: &Path,
        settings: Value,
    ) -> Result<Self, Error> {
        let (outgoing, to_write) = mpsc::channel();
        let shared = Arc::new(Shared {
            outgoing: Mutex::new(outgoing),
            next_id: AtomicI64::new(1),
            waiting: Mutex::new(Some(HashMap::new())),
            documents: Mutex::new(HashMap::new()),
        });
        let (status, status_heard) = watch::channel(Status::Ready);
        std::thread::spawn(move || write(input, &to_write));
        let reader = Reader {
            shared: Arc::clone(&shared),
            status,
            progress: Vec::new(),
            settings: settings.clone(),
            root: root.to_owned(),
        };
        std::thread::spawn(move || reader.read(output));

        let root_uri = Url::from_directory_path(root).ok();
        let name = root
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let initialized = shared.request(
            "initialize",
            &json!({
                "processId": std::process::id(),
                "clientInfo": { "name": "Cox" },
                "rootPath": root,
                "rootUri": root_uri,
                "workspaceFolders": root_uri.as_ref().map(|uri| json!([{ "uri": uri, "name": name }])),
                "initializationOptions": settings,
                "capabilities": {
                    "general": { "positionEncodings": ["utf-8", "utf-16"] },
                    "window": { "workDoneProgress": true },
                    "workspace": { "workspaceFolders": true, "configuration": true },
                    "textDocument": {
                        "synchronization": { "dynamicRegistration": false },
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "definition": { "linkSupport": true },
                        "references": {},
                    },
                },
            }),
        );
        let answer = initialized.await?;
        let encoding = answer
            .pointer("/capabilities/positionEncoding")
            .and_then(Value::as_str)
            .map_or(Encoding::Utf16, Encoding::named);
        shared.notify("initialized", &json!({}));
        Ok(Self {
            shared,
            encoding,
            status: status_heard,
            child: None,
        })
    }

    /// What the server counts a line in.
    #[must_use]
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// What the server is doing, which changes as it says.
    #[must_use]
    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    /// Opens the file at `path`, an absolute path, in the language the
    /// protocol names `language`, as saying `text`. It stays open on the
    /// server for as long as an [`OpenFile`] of it is held.
    #[must_use]
    pub fn open(&self, path: &Path, language: &str, text: &str) -> Option<OpenFile> {
        let uri = Url::from_file_path(path).ok()?;
        let mut documents = lock(&self.shared.documents);
        if let Some(opened) = documents.get_mut(&uri) {
            opened.holders += 1;
        } else {
            documents.insert(
                uri.clone(),
                Opened {
                    holders: 1,
                    version: 0,
                    text: text.to_owned(),
                },
            );
            self.shared.notify(
                "textDocument/didOpen",
                &json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": language,
                        "version": 0,
                        "text": text,
                    },
                }),
            );
        }
        drop(documents);
        let document = OpenFile {
            shared: Arc::clone(&self.shared),
            uri,
        };
        // Held open already, it may have said something else.
        document.change(text);
        Some(document)
    }

    /// What the server says of the name at `position` of `document`, if
    /// anything.
    ///
    /// # Errors
    ///
    /// When the server fails to answer, or stops.
    pub fn hover(
        &self,
        document: &OpenFile,
        position: Position,
    ) -> impl Future<Output = Result<Option<Hover>, Error>> + use<> {
        let answered = self
            .shared
            .request("textDocument/hover", &document.at(position));
        async move { Ok(hover(&answered.await?)) }
    }

    /// Where the name at `position` of `document` is defined.
    ///
    /// # Errors
    ///
    /// When the server fails to answer, or stops.
    pub fn definition(
        &self,
        document: &OpenFile,
        position: Position,
    ) -> impl Future<Output = Result<Vec<Location>, Error>> + use<> {
        let answered = self
            .shared
            .request("textDocument/definition", &document.at(position));
        async move { Ok(locations(&answered.await?)) }
    }

    /// Where the name at `position` of `document` is used, and defined.
    ///
    /// # Errors
    ///
    /// When the server fails to answer, or stops.
    pub fn references(
        &self,
        document: &OpenFile,
        position: Position,
    ) -> impl Future<Output = Result<Vec<Location>, Error>> + use<> {
        let mut params = document.at(position);
        params["context"] = json!({ "includeDeclaration": true });
        let answered = self.shared.request("textDocument/references", &params);
        async move { Ok(locations(&answered.await?)) }
    }
}

impl Drop for Client {
    /// Asks the server to stop, which it does once it has read that, and
    /// ends its process if it has not after a while.
    fn drop(&mut self) {
        let shutdown = self.shared.request("shutdown", &Value::Null);
        drop(shutdown);
        self.shared.notify("exit", &Value::Null);
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                for _ in 0..20 {
                    if child.try_wait().is_ok_and(|ended| ended.is_some()) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

/// A file open on the server, which reads it as the editor holds it.
pub struct OpenFile {
    shared: Arc<Shared>,
    uri: Url,
}

impl OpenFile {
    /// Tells the server the file says `text` now.
    pub fn change(&self, text: &str) {
        let mut documents = lock(&self.shared.documents);
        let Some(opened) = documents.get_mut(&self.uri) else {
            return;
        };
        if opened.text == text {
            return;
        }
        opened.version += 1;
        text.clone_into(&mut opened.text);
        self.shared.notify(
            "textDocument/didChange",
            &json!({
                "textDocument": { "uri": self.uri, "version": opened.version },
                "contentChanges": [{ "text": text }],
            }),
        );
    }

    /// The parameters of a request about `position` of the file.
    fn at(&self, position: Position) -> Value {
        json!({
            "textDocument": { "uri": self.uri },
            "position": { "line": position.line, "character": position.character },
        })
    }
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        let mut documents = lock(&self.shared.documents);
        let Some(opened) = documents.get_mut(&self.uri) else {
            return;
        };
        opened.holders -= 1;
        if opened.holders == 0 {
            documents.remove(&self.uri);
            self.shared.notify(
                "textDocument/didClose",
                &json!({ "textDocument": { "uri": self.uri } }),
            );
        }
    }
}

/// Writes each message of `to_write` to `input`, the server's, until the
/// client is gone or the server stopped reading.
fn write(mut input: impl Write, to_write: &mpsc::Receiver<Vec<u8>>) {
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
struct Reader {
    shared: Arc<Shared>,
    status: watch::Sender<Status>,
    /// The work the server said it began and has not ended, in order, by
    /// its token, with its title, and what the status says of it.
    progress: Vec<(Value, String, String)>,
    settings: Value,
    root: PathBuf,
}

impl Reader {
    fn read(mut self, output: impl Read) {
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
            // Each setting asked for is the server's own.
            "workspace/configuration" => {
                let items = params
                    .and_then(|params| params.get("items"))
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                Ok(Value::Array(vec![self.settings.clone(); items]))
            }
            "workspace/workspaceFolders" => Ok(Url::from_directory_path(&self.root)
                .map_or(Value::Null, |uri| json!([{ "uri": uri, "name": "" }]))),
            "window/workDoneProgress/create"
            | "client/registerCapability"
            | "client/unregisterCapability"
            | "window/showMessageRequest" => Ok(Value::Null),
            _ => Err(format!("Cox does not answer {method}")),
        }
    }

    /// The server's notification `method`, with `params`.
    fn notified(&mut self, method: &str, params: &Value) {
        match method {
            "$/progress" => self.progressed(params),
            "window/showMessage" | "window/logMessage" => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    log::debug!("{message}");
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
fn read_message(output: &mut impl BufRead) -> Option<Value> {
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

/// The position `value` says.
fn position(value: &Value) -> Option<Position> {
    Some(Position {
        line: u32::try_from(value.get("line")?.as_u64()?).ok()?,
        character: u32::try_from(value.get("character")?.as_u64()?).ok()?,
    })
}

/// The stretch `value` says.
fn range(value: &Value) -> Option<Range<Position>> {
    Some(position(value.get("start")?)?..position(value.get("end")?)?)
}

/// The hover `value` says, if it says anything.
fn hover(value: &Value) -> Option<Hover> {
    let markdown = markdown(value.get("contents")?);
    if markdown.trim().is_empty() {
        return None;
    }
    Some(Hover {
        markdown,
        range: value.get("range").and_then(range),
    })
}

/// The contents of a hover, as Markdown: markup, or a string of the older
/// form, or a list of them.
fn markdown(contents: &Value) -> String {
    match contents {
        Value::String(markdown) => markdown.clone(),
        Value::Array(contents) => contents
            .iter()
            .map(markdown)
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(contents) => {
            let value = contents
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match (
                contents.get("kind").and_then(Value::as_str),
                contents.get("language").and_then(Value::as_str),
            ) {
                (Some("markdown"), _) => value.to_owned(),
                (_, Some(language)) => format!("```{language}\n{value}\n```"),
                // Plain text, which Markdown draws as it is in a code block.
                _ => format!("```\n{value}\n```"),
            }
        }
        _ => String::new(),
    }
}

/// The locations `value` says: one, a list of them, or of links to them,
/// leaving out those not in a file.
fn locations(value: &Value) -> Vec<Location> {
    let one = |value: &Value| {
        // A link goes to the name, within the whole of what it links to.
        let (uri, stretch) = match value.get("targetUri") {
            Some(uri) => (
                uri,
                value
                    .get("targetSelectionRange")
                    .or_else(|| value.get("targetRange"))?,
            ),
            None => (value.get("uri")?, value.get("range")?),
        };
        Some(Location {
            path: Url::parse(uri.as_str()?).ok()?.to_file_path().ok()?,
            range: range(stretch)?,
        })
    };
    match value {
        Value::Array(values) => values.iter().filter_map(one).collect(),
        Value::Object(_) => one(value).into_iter().collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_a_line_as_the_server_does() {
        let buffer = Buffer::new("let 日本 = \"😀\";\n");
        let after_emoji = Point::new(0, "let 日本 = \"😀".len());
        for (encoding, character) in [
            (Encoding::Utf8, 18),
            (Encoding::Utf16, 12),
            (Encoding::Utf32, 11),
        ] {
            let position = encoding.position(&buffer, after_emoji);
            assert_eq!(position, Position { line: 0, character });
            assert_eq!(encoding.point(&buffer, position), after_emoji);
        }
        // Past the end of the line, and within a character.
        let end = Point::new(0, buffer.line(0).len());
        let past = Position {
            line: 0,
            character: 99,
        };
        assert_eq!(Encoding::Utf16.point(&buffer, past), end);
        assert_eq!(
            Encoding::Utf8.point(
                &buffer,
                Position {
                    line: 0,
                    character: 5
                }
            ),
            Point::new(0, 4)
        );
    }

    #[test]
    fn reads_hovers_of_every_form() {
        let markup = json!({
            "contents": { "kind": "markdown", "value": "```rust\nfn main()\n```" },
            "range": {
                "start": { "line": 1, "character": 3 },
                "end": { "line": 1, "character": 7 },
            },
        });
        assert_eq!(
            hover(&markup),
            Some(Hover {
                markdown: "```rust\nfn main()\n```".to_owned(),
                range: Some(
                    Position {
                        line: 1,
                        character: 3
                    }..Position {
                        line: 1,
                        character: 7
                    }
                ),
            })
        );
        let marked = json!({
            "contents": [{ "language": "rust", "value": "fn main()" }, "Runs."],
        });
        assert_eq!(
            hover(&marked).unwrap().markdown,
            "```rust\nfn main()\n```\n\nRuns."
        );
        assert_eq!(hover(&json!({ "contents": "" })), None);
        assert_eq!(hover(&Value::Null), None);
    }

    #[test]
    fn reads_locations_and_links_to_them() {
        let range = json!({
            "start": { "line": 4, "character": 7 },
            "end": { "line": 4, "character": 11 },
        });
        let whole = json!({
            "start": { "line": 3, "character": 0 },
            "end": { "line": 9, "character": 1 },
        });
        let expected = Location {
            path: PathBuf::from("/src/main.rs"),
            range: Position {
                line: 4,
                character: 7,
            }..Position {
                line: 4,
                character: 11,
            },
        };
        let location = json!({ "uri": "file:///src/main.rs", "range": range });
        assert_eq!(locations(&location), std::slice::from_ref(&expected));
        let links = json!([
            { "targetUri": "file:///src/main.rs", "targetRange": whole, "targetSelectionRange": range },
            { "targetUri": "untitled:Untitled-1", "targetRange": whole },
        ]);
        assert_eq!(locations(&links), [expected]);
        assert_eq!(locations(&Value::Null), []);
    }

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

    /// Serves as a language server would, over pipes, keeping what it
    /// hears in `heard`, until it hears `exit`, or `stop`, which it does
    /// not answer: answers the client's ends of the pipes, and the server's
    /// thread.
    fn serve(
        heard: Arc<Mutex<Vec<Value>>>,
        stop: &'static str,
    ) -> (
        std::io::PipeReader,
        std::io::PipeWriter,
        std::thread::JoinHandle<()>,
    ) {
        let (output, mut writes) = std::io::pipe().unwrap();
        let (reads, input) = std::io::pipe().unwrap();
        let server = std::thread::spawn(move || {
            let mut reads = BufReader::new(reads);
            let mut write = |message: Value| {
                let body = message.to_string();
                write!(writes, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            };
            while let Some(message) = read_message(&mut reads) {
                lock(&heard).push(message.clone());
                let id = message.get("id").cloned();
                let method = message.get("method").and_then(Value::as_str);
                if method == Some(stop) || method == Some("exit") {
                    break;
                }
                let answer =
                    |result: Value| json!({ "jsonrpc": "2.0", "id": id, "result": result });
                match method {
                    Some("initialize") => {
                        write(answer(
                            json!({ "capabilities": { "positionEncoding": "utf-8" } }),
                        ));
                    }
                    Some("initialized") => {
                        write(json!({
                            "jsonrpc": "2.0",
                            "id": "settings",
                            "method": "workspace/configuration",
                            "params": { "items": [{ "section": "fake" }] },
                        }));
                        for value in [
                            json!({ "kind": "begin", "title": "Indexing" }),
                            json!({ "kind": "report", "message": "1/2" }),
                        ] {
                            write(json!({
                                "jsonrpc": "2.0",
                                "method": "$/progress",
                                "params": { "token": "index", "value": value },
                            }));
                        }
                    }
                    Some("textDocument/hover") => {
                        write(answer(
                            json!({ "contents": { "kind": "markdown", "value": "fn main()" } }),
                        ));
                    }
                    Some("textDocument/definition") => {
                        write(answer(json!({
                            "uri": "file:///src/lib.rs",
                            "range": {
                                "start": { "line": 2, "character": 7 },
                                "end": { "line": 2, "character": 10 },
                            },
                        })));
                    }
                    Some("textDocument/references") => write(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32801, "message": "content modified" },
                    })),
                    Some("shutdown") => write(answer(Value::Null)),
                    _ => {}
                }
            }
        });
        (output, input, server)
    }

    /// What `heard` holds: each message's method, or `answer`.
    fn methods(heard: &Mutex<Vec<Value>>) -> Vec<String> {
        lock(heard)
            .iter()
            .map(|message| {
                message
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("answer")
                    .to_owned()
            })
            .collect()
    }

    #[tokio::test]
    async fn asks_a_server_of_the_files_it_opens() {
        let heard = Arc::default();
        let (output, input, server) = serve(Arc::clone(&heard), "");
        let client = Client::over(output, input, Path::new("/src"), json!({ "fake": true }))
            .await
            .unwrap();
        assert_eq!(client.encoding(), Encoding::Utf8);
        client
            .status()
            .wait_for(|status| *status == Status::Working("Indexing: 1/2".to_owned()))
            .await
            .unwrap();

        // Opened twice, the file is opened on the server once, and closed
        // once neither holds it.
        let main = Path::new("/src/main.rs");
        let document = client.open(main, "rust", "fn main() {}").unwrap();
        let again = client.open(main, "rust", "fn main() {}").unwrap();
        again.change("fn main() { run(); }");
        again.change("fn main() { run(); }");
        let at = Position {
            line: 0,
            character: 3,
        };
        let hover = client.hover(&document, at).await.unwrap().unwrap();
        assert_eq!(hover.markdown, "fn main()");
        let defined = client.definition(&document, at).await.unwrap();
        assert_eq!(defined[0].path, Path::new("/src/lib.rs"));
        assert!(matches!(
            client.references(&document, at).await,
            Err(Error::Answered { code: -32801, .. })
        ));
        drop(again);
        drop(document);
        drop(client);
        server.join().unwrap();

        assert_eq!(
            methods(&heard),
            [
                "initialize",
                "initialized",
                "answer",
                "textDocument/didOpen",
                "textDocument/didChange",
                "textDocument/hover",
                "textDocument/definition",
                "textDocument/references",
                "textDocument/didClose",
                "shutdown",
                "exit",
            ]
        );
        let heard = lock(&heard);
        // The settings it asked for are those it was told when it started.
        assert_eq!(
            heard[0]["params"]["initializationOptions"],
            json!({ "fake": true })
        );
        assert_eq!(heard[2]["result"], json!([{ "fake": true }]));
        assert_eq!(heard[4]["params"]["textDocument"]["version"], 1);
        assert_eq!(
            heard[5]["params"]["textDocument"]["uri"],
            "file:///src/main.rs"
        );
    }

    #[tokio::test]
    async fn a_server_that_stopped_answers_nothing() {
        let heard = Arc::default();
        let (output, input, server) = serve(Arc::clone(&heard), "textDocument/hover");
        let client = Client::over(output, input, Path::new("/src"), Value::Null)
            .await
            .unwrap();
        let document = client.open(Path::new("/src/main.rs"), "rust", "").unwrap();
        let at = Position::default();
        assert!(matches!(
            client.hover(&document, at).await,
            Err(Error::Stopped)
        ));
        server.join().unwrap();
        client
            .status()
            .wait_for(|status| *status == Status::Stopped)
            .await
            .unwrap();
        assert!(matches!(
            client.definition(&document, at).await,
            Err(Error::Stopped)
        ));
    }
}
