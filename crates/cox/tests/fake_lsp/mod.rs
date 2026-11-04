//! A language server for tests, connected to Cox over pipes, which knows a
//! project of two files: `main.rs` calls `helper`, which `lib.rs`
//! defines.
//!
//! It says the same of every name: that it is `fn helper()`, which helps.
//! It finds `helper` defined in `lib.rs`, and used in both files.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cox::stage::servers::Starter;
use cox_editor::lsp::Client;
use serde_json::{Value, json};

pub const MAIN: &str = "fn main() {\n    helper();\n}\n";
pub const LIB: &str = "pub fn helper() {}\n";

/// What the server says of every name.
pub const HOVER: &str = "```rust\nfn helper()\n```\n\nHelps.";

/// Writes the project in `directory`.
pub fn write_project(directory: &Path) {
    std::fs::write(directory.join("main.rs"), MAIN).unwrap();
    std::fs::write(directory.join("lib.rs"), LIB).unwrap();
}

/// What starts the server, counting in `started` how many it started.
pub fn start(started: Arc<AtomicUsize>) -> Starter {
    Rc::new(move |server, root| {
        started.fetch_add(1, Ordering::Relaxed);
        let (output, writes) = std::io::pipe().unwrap();
        let (reads, input) = std::io::pipe().unwrap();
        let project = root.clone();
        std::thread::spawn(move || serve(&project, reads, writes));
        Box::pin(async move { Client::over(output, input, &root, &server).await })
    })
}

/// Answers what it reads from `reads` on `writes`, for the project in
/// `root`.
fn serve(root: &Path, reads: impl Read, mut writes: impl Write) {
    let uri = |name: &str| format!("file://{}", root.join(name).display());
    let range = |line: u32, start: u32, end: u32| {
        json!({
            "start": { "line": line, "character": start },
            "end": { "line": line, "character": end },
        })
    };
    let mut reads = BufReader::new(reads);
    while let Some(message) = read(&mut reads) {
        let result = match message.get("method").and_then(Value::as_str) {
            Some("initialize") => json!({ "capabilities": {
                "positionEncoding": "utf-8",
                "hoverProvider": true,
                "definitionProvider": true,
                "referencesProvider": true,
            } }),
            Some("textDocument/hover") => {
                json!({ "contents": { "kind": "markdown", "value": HOVER } })
            }
            Some("textDocument/definition") => {
                json!({ "uri": uri("lib.rs"), "range": range(0, 7, 13) })
            }
            Some("textDocument/references") => json!([
                { "uri": uri("main.rs"), "range": range(1, 4, 10) },
                { "uri": uri("lib.rs"), "range": range(0, 7, 13) },
            ]),
            Some("exit") => break,
            _ => continue,
        };
        let body = json!({ "jsonrpc": "2.0", "id": message["id"], "result": result }).to_string();
        if write!(writes, "Content-Length: {}\r\n\r\n{body}", body.len()).is_err() {
            break;
        }
    }
}

/// The next message from `reads`, until there is none.
fn read(reads: &mut impl BufRead) -> Option<Value> {
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reads.read_line(&mut header).ok()? == 0 {
            return None;
        }
        match header.trim_end().split_once(": ") {
            Some(("Content-Length", value)) => length = value.parse().ok()?,
            None => break,
            Some(_) => {}
        }
    }
    let mut body = vec![0; length];
    reads.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}
