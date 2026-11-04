//! Talking to a language server, which knows a project's code as its
//! compiler does: what a name is, where it is defined, where it is used,
//! and what a function calls. The editor starts the server that knows the
//! language of a file (see [`languages`] and [`servers`]), in the
//! project's folder, and talks to it over the Language Server Protocol:
//! JSON-RPC messages on the server's standard input and output. It asks
//! the server only what the server says it answers (see [`Feature`]).
//!
//! The server reads each file the editor shows as the editor holds it: the
//! editor opens it on the server as an [`OpenFile`], and tells the server of
//! the text each time it changes. It asks of a place in it, as a
//! [`Position`], which counts the bytes of a line in UTF-8, or in UTF-16,
//! as the server does (see [`Encoding`]).
//!
//! Two threads read and write the messages (see [`wire`]), so that waiting
//! for the server holds up nothing else: a request answers a future, which
//! needs no runtime to wait on.
//!
//! - [`languages`] is which language a file is in, as the protocol names
//!   it.
//! - [`servers`] is the servers Cox knows, and those a project names.
//! - [`position`] is a place in a file as the server counts it.
//! - [`wire`] reads and writes the messages, and acts on the server's own.
//! - [`answers`] is what the server answers about a name, and the symbols
//!   of a file, read from its JSON.

mod answers;
pub mod languages;
mod position;
pub mod servers;
mod wire;

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};
use url::Url;

pub use self::answers::{Call, Callable, Hover, Location, Symbol};
use self::answers::{callable, calls, hover, locations, symbols};
pub use self::position::{Encoding, Position};
pub use self::servers::{Server, Servers};
use self::wire::{Reader, write};

/// Why the server did not answer.
#[derive(Debug)]
pub enum Error {
    /// The server could not be started, or talked to.
    Io(std::io::Error),
    /// The server answered with an error.
    Answered { code: i64, message: String },
    /// The server stopped.
    Stopped,
    /// The server does not answer what it was to be asked.
    Unsupported(Feature),
}

/// What a server can be asked, which it says it answers, or not, when it
/// starts, or later as it registers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Hover,
    Definition,
    References,
    Symbols,
    Calls,
}

impl Feature {
    /// The request the feature starts with.
    fn method(self) -> &'static str {
        match self {
            Self::Hover => "textDocument/hover",
            Self::Definition => "textDocument/definition",
            Self::References => "textDocument/references",
            Self::Symbols => "textDocument/documentSymbol",
            Self::Calls => "textDocument/prepareCallHierarchy",
        }
    }

    /// Where the server's capabilities say it answers the feature.
    fn provider(self) -> &'static str {
        match self {
            Self::Hover => "hoverProvider",
            Self::Definition => "definitionProvider",
            Self::References => "referencesProvider",
            Self::Symbols => "documentSymbolProvider",
            Self::Calls => "callHierarchyProvider",
        }
    }

    /// What the server tells, when it answers the feature.
    fn tells(self) -> &'static str {
        match self {
            Self::Hover => "what a name is",
            Self::Definition => "where a name is defined",
            Self::References => "where a name is used",
            Self::Symbols => "the symbols of a file",
            Self::Calls => "what a function calls",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Answered { message, .. } => f.write_str(message),
            Self::Stopped => f.write_str("the server stopped"),
            Self::Unsupported(feature) => {
                write!(f, "the server does not tell {}", feature.tells())
            }
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

/// A running language server, which the editor talks to.
pub struct Client {
    shared: Arc<Shared>,
    encoding: Encoding,
    /// What the server said it answers, when it started.
    capabilities: Value,
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
    /// The requests the server registered that it answers since it
    /// started, by their method.
    registered: Mutex<HashSet<String>>,
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
        let Some((program, args)) = server.command.split_first() else {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} has no command", server.name),
            )));
        };
        let mut child = std::process::Command::new(program)
            .args(args)
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
        let name = server.name.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(errors).lines().map_while(Result::ok) {
                tracing::debug!("{name}: {line}");
            }
        });
        match Self::over(output, input, root, server).await {
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

    /// Talks to `server`, which writes to `output` and reads from `input`,
    /// working in `root`, and answers once it is ready to be asked. It is
    /// told the server's options when it starts, and its settings when it
    /// asks for them.
    ///
    /// # Errors
    ///
    /// When the server stops or fails before it is ready.
    pub async fn over(
        output: impl Read + Send + 'static,
        input: impl Write + Send + 'static,
        root: &Path,
        server: &Server,
    ) -> Result<Self, Error> {
        let (outgoing, to_write) = mpsc::channel();
        let shared = Arc::new(Shared {
            outgoing: Mutex::new(outgoing),
            next_id: AtomicI64::new(1),
            waiting: Mutex::new(Some(HashMap::new())),
            documents: Mutex::new(HashMap::new()),
            registered: Mutex::new(HashSet::new()),
        });
        let (status, status_heard) = watch::channel(Status::Ready);
        std::thread::spawn(move || write(input, &to_write));
        let reader = Reader {
            shared: Arc::clone(&shared),
            status,
            progress: Vec::new(),
            server: server.clone(),
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
                "initializationOptions": server.initialization_options,
                "capabilities": {
                    "general": { "positionEncodings": ["utf-8", "utf-16"] },
                    "window": { "workDoneProgress": true },
                    "workspace": { "workspaceFolders": true, "configuration": true },
                    "textDocument": {
                        "synchronization": { "dynamicRegistration": false },
                        "hover": { "contentFormat": ["markdown", "plaintext"] },
                        "definition": { "linkSupport": true },
                        "references": {},
                        "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
                        "callHierarchy": { "dynamicRegistration": false },
                    },
                },
            }),
        );
        let answer = initialized.await?;
        let encoding = answer
            .pointer("/capabilities/positionEncoding")
            .and_then(Value::as_str)
            .map_or(Encoding::Utf16, Encoding::named);
        let capabilities = answer.get("capabilities").cloned().unwrap_or_default();
        shared.notify("initialized", &json!({}));
        Ok(Self {
            shared,
            encoding,
            capabilities,
            status: status_heard,
            child: None,
        })
    }

    /// What the server counts a line in.
    #[must_use]
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Whether the server answers `feature`: as it said when it started,
    /// or registered since.
    #[must_use]
    pub fn supports(&self, feature: Feature) -> bool {
        let said = self
            .capabilities
            .get(feature.provider())
            .is_some_and(|provider| !matches!(provider, Value::Null | Value::Bool(false)));
        said || lock(&self.shared.registered).contains(feature.method())
    }

    /// Asks the server `params` of `feature`, if it answers it.
    fn ask(
        &self,
        feature: Feature,
        params: &Value,
    ) -> impl Future<Output = Result<Value, Error>> + use<> {
        let answered = self
            .supports(feature)
            .then(|| self.shared.request(feature.method(), params));
        async move {
            match answered {
                Some(answered) => answered.await,
                None => Err(Error::Unsupported(feature)),
            }
        }
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
    /// When the server fails to answer, or stops, or does not answer
    /// that.
    pub fn hover(
        &self,
        document: &OpenFile,
        position: Position,
    ) -> impl Future<Output = Result<Option<Hover>, Error>> + use<> {
        let answered = self.ask(Feature::Hover, &document.at(position));
        async move { Ok(hover(&answered.await?)) }
    }

    /// Where the name at `position` of `document` is defined.
    ///
    /// # Errors
    ///
    /// When the server fails to answer, or stops, or does not answer
    /// that.
    pub fn definition(
        &self,
        document: &OpenFile,
        position: Position,
    ) -> impl Future<Output = Result<Vec<Location>, Error>> + use<> {
        let answered = self.ask(Feature::Definition, &document.at(position));
        async move { Ok(locations(&answered.await?)) }
    }

    /// Where the name at `position` of `document` is used, and defined.
    ///
    /// # Errors
    ///
    /// When the server fails to answer, or stops, or does not answer
    /// that.
    pub fn references(
        &self,
        document: &OpenFile,
        position: Position,
    ) -> impl Future<Output = Result<Vec<Location>, Error>> + use<> {
        let mut params = document.at(position);
        params["context"] = json!({ "includeDeclaration": true });
        let answered = self.ask(Feature::References, &params);
        async move { Ok(locations(&answered.await?)) }
    }

    /// The symbols `document` defines, such as its functions and types,
    /// as the server lists them (see [`Symbol`]).
    ///
    /// # Errors
    ///
    /// When the server fails to answer, or stops, or does not answer
    /// that.
    pub fn symbols(
        &self,
        document: &OpenFile,
    ) -> impl Future<Output = Result<Vec<Symbol>, Error>> + use<> {
        let answered = self.ask(
            Feature::Symbols,
            &json!({ "textDocument": { "uri": document.uri } }),
        );
        async move { Ok(symbols(&answered.await?)) }
    }

    /// The function at `position` of `document`, as the server's call
    /// hierarchy has it, and the functions it calls, with where it calls
    /// each; `None` when no function is there. The server answers as it
    /// knows the code: asked before it has read the project, it may know
    /// of no calls yet.
    ///
    /// # Errors
    ///
    /// When the server fails to answer, or stops, or does not answer
    /// that.
    pub fn calls(
        &self,
        document: &OpenFile,
        position: Position,
    ) -> impl Future<Output = Result<Option<(Callable, Vec<Call>)>, Error>> + use<> {
        let shared = Arc::clone(&self.shared);
        let prepared = self.ask(Feature::Calls, &document.at(position));
        async move {
            let items = prepared.await?;
            let Some(item) = items.as_array().and_then(|items| items.first()) else {
                return Ok(None);
            };
            let Some(function) = callable(item) else {
                return Ok(None);
            };
            let answered = shared
                .request("callHierarchy/outgoingCalls", &json!({ "item": item }))
                .await?;
            Ok(Some((function, calls(&answered))))
        }
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

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Write as _};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::wire::read_message;
    use super::{Client, Encoding, Error, Feature, Position, Server, Status, lock};

    /// Serves as a language server would, over pipes, keeping what it
    /// hears in `heard`, until it hears `exit`, or `stop`, which it does
    /// not answer: answers the client's ends of the pipes, and the server's
    /// thread.
    fn serve(
        heard: Arc<Mutex<Vec<Value>>>,
        stop: &'static str,
        capabilities: Value,
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
                        write(answer(json!({ "capabilities": capabilities })));
                    }
                    Some("initialized") => {
                        write(json!({
                            "jsonrpc": "2.0",
                            "id": "settings",
                            "method": "workspace/configuration",
                            "params": { "items": [{ "section": "fake.on" }, { "section": "else" }] },
                        }));
                        // It answers its call hierarchy after all.
                        write(json!({
                            "jsonrpc": "2.0",
                            "id": "register",
                            "method": "client/registerCapability",
                            "params": { "registrations": [{
                                "id": "calls",
                                "method": "textDocument/prepareCallHierarchy",
                            }] },
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
                    Some("textDocument/documentSymbol") => {
                        write(answer(json!([{
                            "name": "main",
                            "kind": 12,
                            "detail": "fn main()",
                            "range": {
                                "start": { "line": 0, "character": 0 },
                                "end": { "line": 0, "character": 12 },
                            },
                            "selectionRange": {
                                "start": { "line": 0, "character": 3 },
                                "end": { "line": 0, "character": 7 },
                            },
                        }])));
                    }
                    Some(
                        method @ ("textDocument/prepareCallHierarchy"
                        | "callHierarchy/outgoingCalls"),
                    ) => {
                        write(answer(hierarchy(method)));
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

    /// What the fake server says it answers when it starts: all but its
    /// call hierarchy, which it registers later.
    fn answering() -> Value {
        json!({
            "positionEncoding": "utf-8",
            "hoverProvider": true,
            "definitionProvider": { "workDoneProgress": false },
            "referencesProvider": true,
            "documentSymbolProvider": true,
            "callHierarchyProvider": false,
        })
    }

    /// What the fake server answers of its call hierarchy: `main`, which
    /// calls `run` in another file.
    fn hierarchy(method: &str) -> Value {
        let item = |name: &str, uri: &str, line: u64| {
            json!({
                "name": name,
                "kind": 12,
                "uri": uri,
                "range": {
                    "start": { "line": line, "character": 0 },
                    "end": { "line": line + 2, "character": 1 },
                },
                "selectionRange": {
                    "start": { "line": line, "character": 3 },
                    "end": { "line": line, "character": 3 + name.len() },
                },
                "data": 7,
            })
        };
        if method == "textDocument/prepareCallHierarchy" {
            json!([item("main", "file:///src/main.rs", 0)])
        } else {
            json!([{
                "to": item("run", "file:///src/lib.rs", 2),
                "fromRanges": [{
                    "start": { "line": 0, "character": 12 },
                    "end": { "line": 0, "character": 15 },
                }],
            }])
        }
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
        let (output, input, server) = serve(Arc::clone(&heard), "", answering());
        let fake = Server {
            initialization_options: json!({ "fake": true }),
            settings: json!({ "fake": { "on": true } }),
            ..Server::default()
        };
        let client = Client::over(output, input, Path::new("/src"), &fake)
            .await
            .unwrap();
        assert_eq!(client.encoding(), Encoding::Utf8);
        assert!(client.supports(Feature::Hover));
        assert!(client.supports(Feature::Definition));
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
        let symbols = client.symbols(&document).await.unwrap();
        assert!(client.supports(Feature::Calls));
        assert_eq!(symbols[0].name, "main");
        assert_eq!(symbols[0].kind, "function");
        assert_eq!(symbols[0].selection.start.character, 3);
        let (function, calls) = client.calls(&document, at).await.unwrap().unwrap();
        assert_eq!(function.name, "main");
        assert_eq!(calls[0].to.name, "run");
        assert_eq!(calls[0].to.path, Path::new("/src/lib.rs"));
        assert_eq!(calls[0].from[0].start.line, 0);
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
                "answer",
                "textDocument/didOpen",
                "textDocument/didChange",
                "textDocument/hover",
                "textDocument/definition",
                "textDocument/references",
                "textDocument/documentSymbol",
                "textDocument/prepareCallHierarchy",
                "callHierarchy/outgoingCalls",
                "textDocument/didClose",
                "shutdown",
                "exit",
            ]
        );
        let heard = lock(&heard);
        // It is told its options when it starts, and the settings of each
        // section it asks for.
        assert_eq!(
            heard[0]["params"]["initializationOptions"],
            json!({ "fake": true })
        );
        assert_eq!(heard[2]["result"], json!([true, null]));
        assert_eq!(heard[5]["params"]["textDocument"]["version"], 1);
        // The item the server prepared goes back to it as it was.
        assert_eq!(heard[11]["params"]["item"]["data"], 7);
        assert_eq!(
            heard[6]["params"]["textDocument"]["uri"],
            "file:///src/main.rs"
        );
    }

    #[tokio::test]
    async fn a_server_is_asked_only_what_it_answers() {
        let heard = Arc::default();
        let (output, input, server) =
            serve(Arc::clone(&heard), "", json!({ "hoverProvider": true }));
        let client = Client::over(output, input, Path::new("/src"), &Server::default())
            .await
            .unwrap();
        assert_eq!(client.encoding(), Encoding::Utf16);
        let document = client.open(Path::new("/src/main.rs"), "rust", "").unwrap();
        let at = Position::default();
        assert!(client.hover(&document, at).await.unwrap().is_some());
        let error = client.references(&document, at).await.unwrap_err();
        assert!(matches!(error, Error::Unsupported(Feature::References)));
        assert_eq!(
            error.to_string(),
            "the server does not tell where a name is used"
        );
        drop(document);
        drop(client);
        server.join().unwrap();
        assert!(!methods(&heard).contains(&"textDocument/references".to_owned()));
    }

    #[tokio::test]
    async fn a_server_that_stopped_answers_nothing() {
        let heard = Arc::default();
        let (output, input, server) = serve(Arc::clone(&heard), "textDocument/hover", answering());
        let client = Client::over(output, input, Path::new("/src"), &Server::default())
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
