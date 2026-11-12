//! What Cox says of itself as it runs: what went wrong, and, for whoever
//! debugs it, what it did. Cox says it through `tracing`; gpui and the
//! other crates that say it through `log` reach the same place.
//!
//! It goes to standard error, as `RUST_LOG` asks, and only the errors
//! unless it asks for more. The Cox that runs as the app also keeps all of
//! it down to `debug`, or as `COX_LOG` asks, in a file of each run's own,
//! one JSON object a line, for an agent that debugs Cox to read and search
//! once the run is over too: `logs/<time>-<process id>.jsonl` where Cox
//! keeps what lasts (see [`crate::storage`]). It keeps the files of the
//! last [`RUNS_KEPT`] runs. A panic goes there too, with its backtrace.
//! Each line says where in Cox's code it was said, so that what Cox did
//! can be followed back to the code, as [`flow`] does for a tool call.
//!
//! The targets worth asking for when debugging:
//!
//! - `cox_acp::wire`: every ACP message between Cox and an agent.
//! - `cox_acp::stderr`: what an agent writes to standard error.
//! - `cox::agent::mcp::wire`: every message between an agent and Cox's
//!   tool server, in brief; at `trace`, whole, with what it carries:
//!   pages, commands' output, screenshots.
//! - `cox::agent::mcp::asks`: what a tool asks of the stage, and
//!   `cox::stage::asked`, the stage carrying it out.
//! - `cox_browser::scheme`: how long each answer at a `cox:` address took;
//!   a warning at 100 ms or more.
//! - `cox::busy`: each time the window's thread was busy for 100 ms or more.

use std::backtrace::Backtrace;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::{EnvFilter, LevelFilter, filter_fn};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt as _;

pub mod flow;

/// How many runs' files Cox keeps, this one's with them.
pub const RUNS_KEPT: usize = 20;

/// What the file of a run keeps when `COX_LOG` does not say.
const KEPT: &str = "info,cox=debug";

/// The mode of a run's file: what the agents say is the human's user's only.
const OWNER_ONLY: u32 = 0o600;

/// The target a panic is said under, which only the file of the run takes:
/// on standard error, Rust says it already.
const PANIC: &str = "cox::panic";

/// The file this run is kept in, once [`init`] opened one.
static FILE: OnceLock<PathBuf> = OnceLock::new();

/// The file this run is kept in, if it is.
#[must_use]
pub fn file() -> Option<&'static Path> {
    FILE.get().map(PathBuf::as_path)
}

/// The directory the files of the runs are in, if this run is kept.
#[must_use]
pub fn directory() -> Option<&'static Path> {
    file().and_then(Path::parent)
}

/// Starts taking what Cox says, for the rest of the process, keeping it in
/// a new file in `directory` when given, and answers that file's path.
///
/// # Panics
///
/// When something else took it already.
pub fn init(directory: Option<&Path>) -> Option<PathBuf> {
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::ERROR.into())
                .from_env_lossy(),
        )
        .with_filter(filter_fn(|metadata| metadata.target() != PANIC));
    let file = directory.and_then(|directory| {
        open(directory)
            .inspect_err(|error| {
                eprintln!(
                    "cox: what Cox does is not kept in {}: {error}",
                    directory.display()
                );
            })
            .ok()
    });
    let path = file.as_ref().map(|(path, _)| path.clone());
    if let Some(path) = &path {
        let _ = FILE.set(path.clone());
    }
    let kept = file.map(|(_, file)| {
        let filter = EnvFilter::try_from_env("COX_LOG").unwrap_or_else(|_| EnvFilter::new(KEPT));
        kept(file).with_filter(filter)
    });
    tracing_subscriber::registry()
        .with(stderr)
        .with(kept)
        .init();
    if path.is_some() {
        keep_panics();
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            args = ?std::env::args().collect::<Vec<_>>(),
            cwd = ?std::env::current_dir().ok(),
            "Cox starts"
        );
    }
    path
}

/// What keeps what Cox says in `file`: one JSON object a line, with the
/// spans it was said in, such as its chat's, and the file and line of
/// Cox's code it was said at.
fn kept<S>(file: File) -> impl Layer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    tracing_subscriber::fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(true)
        .with_file(true)
        .with_line_number(true)
        .with_ansi(false)
        // Each line is written whole, so what is kept stays readable
        // however the run ends.
        .with_writer(Mutex::new(file))
}

/// Opens a new file for this run in `directory`, making it if need be,
/// once the files of earlier runs past the last [`RUNS_KEPT`] are gone.
fn open(directory: &Path) -> io::Result<(PathBuf, File)> {
    fs::create_dir_all(directory)?;
    prune(directory, RUNS_KEPT - 1)?;
    let time = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ");
    let path = directory.join(format!("{time}-{}.jsonl", std::process::id()));
    let file = File::options()
        .create(true)
        .append(true)
        .mode(OWNER_ONLY)
        .open(&path)?;
    Ok((path, file))
}

/// Removes the files of runs in `directory` but the last `keep`. Their
/// names start with the time the run started, so they sort by it.
fn prune(directory: &Path, keep: usize) -> io::Result<()> {
    let mut runs: Vec<PathBuf> = fs::read_dir(directory)?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .collect();
    runs.sort();
    let past = runs.len().saturating_sub(keep);
    for run in &runs[..past] {
        fs::remove_file(run)?;
    }
    Ok(())
}

/// Says a panic, with its backtrace, before Rust says it as it does.
fn keep_panics() {
    let rust = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(
            target: PANIC,
            backtrace = %Backtrace::force_capture(),
            "{info}"
        );
        rust(info);
    }));
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixStream;
    use std::pin::Pin;
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::*;
    use crate::agent::mcp::{self, Answered, Serves};

    /// A directory of its own for a test.
    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("cox-logs-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn only_the_last_runs_are_kept() {
        let directory = scratch("prune");
        for name in [
            "20261101T100000Z-3.jsonl",
            "20261101T090000Z-2.jsonl",
            "20261031T235959Z-1.jsonl",
            "notes.txt",
        ] {
            fs::write(directory.join(name), "").unwrap();
        }

        prune(&directory, 2).unwrap();

        assert_eq!(
            names(&directory),
            [
                "20261101T090000Z-2.jsonl",
                "20261101T100000Z-3.jsonl",
                "notes.txt"
            ]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_run_gets_a_file_of_its_own() {
        let directory = scratch("open").join("logs");

        let (path, _) = open(&directory).unwrap();

        assert_eq!(path.parent(), Some(directory.as_path()));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.ends_with(&format!("-{}.jsonl", std::process::id())),
            "{name}"
        );
        fs::remove_dir_all(directory.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_run_file_is_the_owners_only() {
        let directory = scratch("mode");

        let (path, _) = open(&directory).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
        fs::remove_dir_all(directory).unwrap();
    }

    /// A tool server whose every call answers a screenshot of `bytes`
    /// bytes of base64.
    struct Screenshot {
        bytes: usize,
    }

    impl Serves for Screenshot {
        fn tools(&self) -> Value {
            json!([])
        }

        fn call<'a>(
            &'a self,
            _params: &'a Value,
        ) -> Pin<Box<dyn Future<Output = Answered> + Send + 'a>> {
            let data = "iVBORw0K".repeat(self.bytes / 8);
            Box::pin(async move {
                Ok(json!({
                    "content": [{ "type": "image", "data": data, "mimeType": "image/png" }],
                    "isError": false,
                }))
            })
        }
    }

    #[test]
    fn a_screenshot_over_the_tools_socket_is_kept_in_brief() {
        let directory = scratch("wire");
        let socket = directory.join("mcp");
        let listener = mcp::listen(&socket).unwrap();
        let path = directory.join("run.jsonl");
        let file = File::create(&path).unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = std::thread::spawn(move || {
            let subscriber =
                tracing_subscriber::registry().with(kept(file).with_filter(EnvFilter::new(KEPT)));
            tracing::subscriber::with_default(subscriber, || {
                mcp::serve(listener, Arc::new(Screenshot { bytes: 1 << 20 }), async {
                    let _ = stopped.await;
                })
            })
        });

        let mut stream = UnixStream::connect(&socket).unwrap();
        let call = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "android_look", "arguments": {} } });
        writeln!(stream, "{call}").unwrap();
        let mut answer = String::new();
        BufReader::new(&stream).read_line(&mut answer).unwrap();
        assert!(answer.len() > 1 << 20);
        stop.send(()).unwrap();
        server.join().unwrap().unwrap();

        let kept = fs::read_to_string(&path).unwrap();
        let wire: Vec<Value> = kept
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|line| line["target"] == mcp::WIRE)
            .collect();
        let froms: Vec<&str> = wire
            .iter()
            .map(|line| line["fields"]["from"].as_str().unwrap())
            .collect();
        assert_eq!(froms, ["agent", "cox"]);
        let answered: Value =
            serde_json::from_str(wire[1]["fields"]["message"].as_str().unwrap()).unwrap();
        assert_eq!(answered["id"], 3);
        assert_eq!(answered["result"]["content"][0]["mimeType"], "image/png");
        assert!(kept.len() < 4096, "{} bytes kept", kept.len());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn what_is_said_is_kept_as_json_with_its_spans() {
        let directory = scratch("kept");
        let path = directory.join("run.jsonl");
        let file = File::create(&path).unwrap();
        let subscriber = tracing_subscriber::registry().with(kept(file));

        tracing::subscriber::with_default(subscriber, || {
            let chat = tracing::info_span!("chat", id = 7);
            chat.in_scope(|| tracing::warn!(target: "cox::test", "the agent said {}", "hi"));
        });

        let line = fs::read_to_string(&path).unwrap();
        let kept: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(kept["level"], "WARN");
        assert_eq!(kept["target"], "cox::test");
        assert_eq!(kept["fields"]["message"], "the agent said hi");
        assert_eq!(kept["spans"][0]["name"], "chat");
        assert_eq!(kept["spans"][0]["id"], 7);
        assert!(kept["timestamp"].is_string());
        assert_eq!(kept["filename"], file!());
        assert!(kept["line_number"].is_u64());
        fs::remove_dir_all(directory).unwrap();
    }
}
