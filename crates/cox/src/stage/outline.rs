//! What the project's language server says of a file, for the Map page
//! to show at the bottom of the map, where a box holds files (see
//! [`crate::stage::map`]): the symbols the file defines, such as its
//! functions and types, as the server lists them, how many times each is
//! used, and which files use them. The page reads it as JSON from
//! `cox://api/file/<path>` (see [`crate::stage::plugins`]), which answers
//! at once with what is known, and has the file asked of the server
//! when it is not, or not any more: the page reads again every two
//! seconds, and gets the answer once the server has given it.
//!
//! The server is the project's, as a tab of code asks it (see
//! [`crate::stage::servers`]), started if it was not, and asked once it
//! is ready; until then, the page is told what the server is doing. A
//! file in a language the editor knows no server for is told to have
//! none.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use cox_editor::buffer::Buffer;
use cox_editor::lsp::{Client, Encoding, Error, Location, OpenFile, Symbol, languages};
use gpui::{AsyncApp, Context, Entity, Task};
use serde::Serialize;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::stage::Stage;
use crate::stage::servers::{LanguageServer, Servers};

/// How long an answer is good for while the file is still looked at,
/// before it is asked of the server again: what uses it changes as other
/// files do.
pub(super) const GOOD_FOR: Duration = Duration::from_secs(90);

/// How often the server is looked at while it starts, or reads the
/// project.
pub(super) const LOOK_EVERY: Duration = Duration::from_millis(300);

/// How long the server has to have been through with the project before
/// it is asked what uses a symbol: it is through, for a moment, between
/// one piece of work it tells of and the next.
pub(super) const SETTLED: Duration = Duration::from_secs(1);

/// How long the server is waited for, at most.
pub(super) const PATIENCE: Duration = Duration::from_mins(3);

/// Of how many symbols of a file the uses are asked, at most.
const MOST_ASKED: usize = 60;

/// What is known of the files asked of, for one chat's stage, and the
/// way to have a file asked of the server.
#[derive(Clone)]
pub struct Outlines {
    known: Arc<Mutex<HashMap<PathBuf, Known>>>,
    asks: UnboundedSender<PathBuf>,
}

/// A file asked of: when, as it was then, and what came of it.
struct Known {
    asked: Instant,
    /// When the file was last written, when it was asked of.
    modified: Option<SystemTime>,
    /// Whether the whole answer is still waited for.
    waiting: bool,
    /// What the server is doing, while the answer is waited for.
    said: Option<String>,
    /// The answer, or as much of it as came: the symbols come ahead of
    /// what uses them, and the last answer stays while a new one is
    /// waited for.
    told: Option<Told>,
}

/// What the server said of a file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Told {
    /// The server's program, or `None` for a file no server knows.
    pub server: Option<String>,
    pub symbols: Vec<Outlined>,
    /// The files that use the file's symbols, most first.
    pub used_by: Vec<UsedBy>,
    /// Why the server did not answer, if it did not.
    pub error: Option<String>,
}

/// A symbol a file defines, as the page shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Outlined {
    pub name: String,
    /// The kind's name, as the protocol names it, in lower case.
    pub kind: String,
    /// What the server adds, such as a signature.
    pub detail: Option<String>,
    /// The line its name is on, from 1.
    pub line: usize,
    /// The last line of it, from 1.
    pub end: usize,
    /// How many places use it, besides where it is defined, or `None`
    /// when that was not asked.
    pub uses: Option<usize>,
    /// Whether it has no name of its own, as an `impl` in Rust, which the
    /// server names after the type it is of: the server does not say
    /// that its name is defined where it is.
    pub unnamed: bool,
    pub children: Vec<Outlined>,
}

/// A file that uses another's symbols.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct UsedBy {
    /// From the project's folder, or absolute when outside it.
    pub path: String,
    pub count: usize,
}

impl Outlines {
    /// Nothing known yet, and what hears the files to ask of, which
    /// [`answer`] takes.
    #[must_use]
    pub fn new() -> (Self, UnboundedReceiver<PathBuf>) {
        let (asks, asked) = unbounded_channel();
        (
            Self {
                known: Arc::default(),
                asks,
            },
            asked,
        )
    }

    /// What is known of the file at `path`, an absolute path, in the
    /// project in `root`, as JSON, or `None` for a path outside the
    /// project: the file and its lines, `pending` while any of the
    /// server's answer is waited for, and `said`, what the server is
    /// doing then, with the answer (see [`Told`]), or as much of it as
    /// came. A file not asked of yet, or written since, or asked of a
    /// while ago, is asked of again.
    pub fn told(&self, root: &Path, path: &Path) -> Option<serde_json::Value> {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        if !path.starts_with(&root) {
            return None;
        }
        let name = path
            .strip_prefix(&root)
            .map_or_else(|_| path.display().to_string(), |p| p.display().to_string());
        let modified = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok();
        let text = std::fs::read_to_string(&path).ok();
        let lines = text.as_ref().map(|text| text.lines().count());
        let language = languages::id_of(&path);

        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let ask = match known.get(&path) {
            None => true,
            Some(known) => {
                known.modified != modified
                    || (!known.waiting && now.duration_since(known.asked) > GOOD_FOR)
            }
        };
        if ask && text.is_some() {
            let told = known.get(&path).and_then(|known| known.told.clone());
            known.insert(
                path.clone(),
                Known {
                    asked: now,
                    modified,
                    waiting: true,
                    said: None,
                    told,
                },
            );
            // Nothing answers once the stage is gone, which the page is too.
            let _ = self.asks.send(path.clone());
        }
        let (pending, said, told) = match known.get(&path) {
            Some(known) => (known.waiting, known.said.clone(), known.told.clone()),
            None => (false, None, None),
        };
        drop(known);

        let mut json = serde_json::json!({
            "path": path.display().to_string(),
            "name": name,
            "exists": text.is_some(),
            "lines": lines,
            "language": language,
            "pending": pending,
            "said": said,
            "server": serde_json::Value::Null,
            "symbols": serde_json::Value::Null,
            "used_by": serde_json::Value::Null,
            "error": serde_json::Value::Null,
        });
        if let Some(told) = told {
            json["server"] = serde_json::json!(told.server);
            json["symbols"] = serde_json::to_value(&told.symbols).unwrap_or_default();
            json["used_by"] = serde_json::to_value(&told.used_by).unwrap_or_default();
            json["error"] = serde_json::json!(told.error);
        }
        Some(json)
    }

    /// The server says `said` of itself, while the file at `path` waits
    /// for it.
    fn saying(&self, path: &Path, said: Option<String>) {
        if let Some(known) = self
            .known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(path)
        {
            known.said = said;
        }
    }

    /// The server answered `told` of the file at `path` so far, with more
    /// to come.
    fn partly(&self, path: &Path, told: Told) {
        self.heard(path, told, true);
    }

    /// The server answered `told` of the file at `path`.
    pub fn learned(&self, path: &Path, told: Told) {
        self.heard(path, told, false);
    }

    fn heard(&self, path: &Path, told: Told, waiting: bool) {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        match known.get_mut(path) {
            Some(known) => {
                known.waiting = waiting;
                known.said = None;
                known.told = Some(told);
                if !waiting {
                    // Good for a while from now, not from when it was asked.
                    known.asked = Instant::now();
                }
            }
            None => {
                known.insert(
                    path.to_owned(),
                    Known {
                        asked: Instant::now(),
                        modified: None,
                        waiting,
                        said: None,
                        told: Some(told),
                    },
                );
            }
        }
    }
}

/// Answers the files `asked` names, one after another, for a chat working
/// in `cwd`: asks the project's server of each (see [`learn`]), and tells
/// `outlines` what it said.
pub fn answer(
    mut asked: UnboundedReceiver<PathBuf>,
    outlines: Outlines,
    cwd: PathBuf,
    cx: &mut Context<Stage>,
) -> Task<()> {
    cx.spawn(async move |_, cx| {
        while let Some(path) = asked.recv().await {
            let told = learn(&path, &cwd, &outlines, cx).await;
            outlines.learned(&path, told);
        }
    })
}

/// What the project's server says of the file at `path`: its symbols, as
/// soon as the server reads the file, which `outlines` hears ahead of the
/// rest, and how many places use each, once the server has read the
/// project; `outlines` hears what the server is doing meanwhile.
async fn learn(path: &Path, cwd: &Path, outlines: &Outlines, cx: &mut AsyncApp) -> Told {
    let saying = |said: Option<String>| outlines.saying(path, said);
    let (server, name) = match running(path, cwd, &saying, cx).await {
        Ok(Some(server)) => server,
        Ok(None) => return Told::default(),
        Err((name, error)) => {
            return Told {
                server: Some(name),
                error: Some(error),
                ..Told::default()
            };
        }
    };
    let failed = |error: String| Told {
        server: Some(name.clone()),
        error: Some(error),
        ..Told::default()
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return failed("the file cannot be read".to_owned());
    };
    let opened = server.read_with(cx, |server, _| {
        let client = server.client()?;
        client.open(path, server.language_of(path)?, &text)
    });
    let Ok(Some(document)) = opened else {
        return failed(format!("{name} stopped"));
    };
    let began = Instant::now();
    let symbols = match symbols_of(&server, &document, &name, began, &saying, cx).await {
        Ok(symbols) => symbols,
        Err(error) => return failed(error),
    };
    let mut nodes: Vec<Outlined> = symbols.iter().map(|symbol| outline(symbol, 0)).collect();
    outlines.partly(
        path,
        Told {
            server: Some(name.clone()),
            symbols: nodes.clone(),
            used_by: Vec::new(),
            error: None,
        },
    );
    if let Err(error) = settled(&server, began, &saying, cx).await {
        return failed(error);
    }
    outlines.saying(path, Some(format!("{name}: finding what uses them")));
    let unnamed = unnamed(&server, &document, path, &text, &symbols, cx).await;
    for (place, node) in nodes.iter_mut().enumerate() {
        node.unnamed = unnamed.contains(&vec![place]);
        for (within, child) in node.children.iter_mut().enumerate() {
            child.unnamed = unnamed.contains(&vec![place, within]);
        }
    }
    let used_by = uses(&server, &document, path, cwd, &symbols, &mut nodes, cx).await;
    drop(document);
    Told {
        server: Some(name),
        symbols: nodes,
        used_by,
        error: None,
    }
}

/// The project's server for the file at `path`, and its name, once it
/// runs, or `None` when the editor knows none for the file; or its name
/// and why it will not run. Until it runs, `saying` hears what it is
/// doing.
pub(super) async fn running(
    path: &Path,
    cwd: &Path,
    saying: &dyn Fn(Option<String>),
    cx: &mut AsyncApp,
) -> Result<Option<(Entity<LanguageServer>, String)>, (String, String)> {
    let server: Option<Entity<LanguageServer>> = cx
        .update(|cx| {
            cx.try_global::<Servers>()
                .cloned()
                .and_then(|servers| servers.of(cwd, path, cx))
        })
        .ok()
        .flatten();
    let Some(server) = server else {
        return Ok(None);
    };
    let name = server
        .read_with(cx, |server, _| server.name().to_owned())
        .unwrap_or_default();
    let began = Instant::now();
    loop {
        let Ok((runs, said)) =
            server.read_with(cx, |server, _| (server.client().is_some(), server.said()))
        else {
            return Err((name, "Cox is quitting".to_owned()));
        };
        if runs {
            return Ok(Some((server, name)));
        }
        if let Some(said) = &said
            && (said.contains("did not start") || said.ends_with("stopped"))
        {
            return Err((name, said.clone()));
        }
        if began.elapsed() > PATIENCE {
            return Err((
                name.clone(),
                format!(
                    "{name} did not start in {} minutes",
                    PATIENCE.as_secs() / 60
                ),
            ));
        }
        saying(said);
        cx.background_executor().timer(LOOK_EVERY).await;
    }
}

/// The symbols of `document` on `server`, named `name`: the server
/// answers them as soon as it has the file, but may answer nothing, or
/// that it is busy, while it still reads the project; then it is asked
/// again, `saying` hearing what it is doing, until `began` is a while
/// ago; or why it did not answer.
pub(super) async fn symbols_of(
    server: &Entity<LanguageServer>,
    document: &OpenFile,
    name: &str,
    began: Instant,
    saying: &dyn Fn(Option<String>),
    cx: &mut AsyncApp,
) -> Result<Vec<Symbol>, String> {
    loop {
        let asked = server.read_with(cx, |server, _| {
            server.client().map(|client| client.symbols(document))
        });
        let Ok(Some(asked)) = asked else {
            return Err(format!("{name} stopped"));
        };
        let busy = server
            .read_with(cx, |server, _| server.said())
            .unwrap_or_default();
        match asked.await {
            Ok(symbols) if !symbols.is_empty() || busy.is_none() => return Ok(symbols),
            Ok(_) => {}
            Err(error @ Error::Unsupported(_)) => return Err(error.to_string()),
            Err(error) if busy.is_some() => tracing::debug!("{name} said: {error}"),
            Err(error) => return Err(error.to_string()),
        }
        if began.elapsed() > PATIENCE {
            return Err(format!(
                "{name} is not ready after {} minutes",
                PATIENCE.as_secs() / 60
            ));
        }
        saying(busy);
        cx.background_executor().timer(LOOK_EVERY).await;
    }
}

/// Waits until `server` has been through with the project for a moment,
/// as it is between one piece of work it tells of and the next, which
/// `saying` hears meanwhile, or until `began` is a while ago; or why it
/// cannot wait.
pub(super) async fn settled(
    server: &Entity<LanguageServer>,
    began: Instant,
    saying: &dyn Fn(Option<String>),
    cx: &mut AsyncApp,
) -> Result<(), String> {
    let mut through_since: Option<Instant> = None;
    loop {
        let Ok(said) = server.read_with(cx, |server, _| server.said()) else {
            return Err("Cox is quitting".to_owned());
        };
        if said.is_none() {
            let since = *through_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= SETTLED {
                return Ok(());
            }
        } else {
            through_since = None;
            saying(said);
        }
        if began.elapsed() > PATIENCE {
            return Ok(());
        }
        cx.background_executor().timer(LOOK_EVERY).await;
    }
}

/// Which of the `symbols` of `document`, the file at `path`, which says
/// `text`, the top ones and those within them, have no name of its own,
/// by their places among them. A symbol whose name is the text at its
/// name's place, as most are, has; another, as an `impl` in Rust named
/// `impl Store` at `Store`, or a method in Go named `(*Store).Add` at
/// `Add`, has if the server says its name is defined there, up to a
/// limit, and is taken to have past it, or when the server does not say.
async fn unnamed(
    server: &Entity<LanguageServer>,
    document: &OpenFile,
    path: &Path,
    text: &str,
    symbols: &[Symbol],
    cx: &mut AsyncApp,
) -> Vec<Vec<usize>> {
    let Ok(Some(encoding)) =
        server.read_with(cx, |server, _| server.client().map(Client::encoding))
    else {
        return Vec::new();
    };
    let buffer = Buffer::new(text);
    let places = symbols.iter().enumerate().flat_map(|(index, symbol)| {
        std::iter::once((vec![index], symbol)).chain(
            symbol
                .children
                .iter()
                .enumerate()
                .map(move |(within, child)| (vec![index, within], child)),
        )
    });
    let mut unnamed = Vec::new();
    let unsure = places
        .filter(|(_, symbol)| !named_at(symbol, &buffer, encoding))
        .take(MOST_ASKED);
    for (place, symbol) in unsure {
        let definitions = server.read_with(cx, |server, _| {
            server
                .client()
                .map(|client| client.definition(document, symbol.selection.start))
        });
        let Ok(Some(definitions)) = definitions else {
            break;
        };
        let Ok(definitions) = definitions.await else {
            continue;
        };
        let at = symbol.selection.start;
        if !definitions.iter().any(|defined| {
            defined.path == path && defined.range.start <= at && at <= defined.range.end
        }) {
            unnamed.push(place);
        }
    }
    unnamed
}

/// Whether `symbol`'s name is the text of `buffer` at its name's place,
/// which the server counts in `encoding`.
fn named_at(symbol: &Symbol, buffer: &Buffer, encoding: Encoding) -> bool {
    let Range { start, end } = symbol.selection;
    if start.line != end.line {
        return false;
    }
    let line = buffer.line(start.line as usize);
    let (start, end) = (
        encoding.point(buffer, start).column,
        encoding.point(buffer, end).column,
    );
    line.get(start..end) == Some(symbol.name.as_str())
}

/// Asks `server` where the `symbols` of `document`, the file at `path`,
/// are used: the top ones and those within them, as the methods of an
/// `impl`, up to a limit, but for those `nodes` has as unnamed, which the
/// server would answer for what they are named after. Each count goes on
/// its node of `nodes`, and the files that use them, besides the file
/// itself, are answered, most first, from `cwd`.
async fn uses(
    server: &Entity<LanguageServer>,
    document: &OpenFile,
    path: &Path,
    cwd: &Path,
    symbols: &[Symbol],
    nodes: &mut [Outlined],
    cx: &mut AsyncApp,
) -> Vec<UsedBy> {
    let mut asked: Vec<(Vec<usize>, &Symbol)> = Vec::new();
    for (index, symbol) in symbols.iter().enumerate() {
        if asked.len() >= MOST_ASKED {
            break;
        }
        let node = &nodes[index];
        if !node.unnamed {
            asked.push((vec![index], symbol));
        }
        for (within, child) in symbol.children.iter().enumerate() {
            if asked.len() >= MOST_ASKED {
                break;
            }
            if !node.children.get(within).is_some_and(|child| child.unnamed) {
                asked.push((vec![index, within], child));
            }
        }
    }
    let mut used_by: HashMap<PathBuf, usize> = HashMap::new();
    for (place, symbol) in asked {
        let references = server.read_with(cx, |server, _| {
            server
                .client()
                .map(|client| client.references(document, symbol.selection.start))
        });
        let Ok(Some(references)) = references else {
            break;
        };
        let Ok(references) = references.await else {
            continue;
        };
        let found: Vec<&Location> = references
            .iter()
            .filter(|at| at.path != path || at.range.start != symbol.selection.start)
            .collect();
        for at in &found {
            if at.path != path {
                *used_by.entry(at.path.clone()).or_default() += 1;
            }
        }
        let mut node = &mut nodes[place[0]];
        if let Some(within) = place.get(1) {
            node = &mut node.children[*within];
        }
        node.uses = Some(found.len());
    }
    let mut used_by: Vec<UsedBy> = used_by
        .into_iter()
        .map(|(file, count)| UsedBy {
            path: file
                .strip_prefix(cwd)
                .map_or_else(|_| file.display().to_string(), |p| p.display().to_string()),
            count,
        })
        .collect();
    used_by.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.path.cmp(&b.path)));
    used_by
}

/// `symbol` as the page shows it, with what is within it, `depth` down.
fn outline(symbol: &Symbol, depth: usize) -> Outlined {
    Outlined {
        name: symbol.name.clone(),
        kind: symbol.kind.clone(),
        detail: symbol.detail.clone(),
        line: symbol.selection.start.line as usize + 1,
        end: symbol.range.end.line as usize + 1,
        uses: None,
        unnamed: false,
        children: if depth < 2 {
            symbol
                .children
                .iter()
                .map(|child| outline(child, depth + 1))
                .collect()
        } else {
            Vec::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use cox_editor::lsp::Position;

    use super::*;
    use coxd::testing::TempDir;

    #[test]
    fn tells_a_symbol_named_at_its_place() {
        let buffer =
            Buffer::new("impl Store {\n    fn add(&mut self) {}\n}\nfunc (s *Store) Add() {}\n");
        let symbol = |name: &str, line: u32, start: u32, end: u32| Symbol {
            name: name.to_owned(),
            kind: String::new(),
            detail: None,
            range: Position { line, character: 0 }..Position {
                line,
                character: end,
            },
            selection: Position {
                line,
                character: start,
            }..Position {
                line,
                character: end,
            },
            children: Vec::new(),
        };
        let named = |symbol: &Symbol| named_at(symbol, &buffer, Encoding::Utf16);
        assert!(named(&symbol("add", 1, 7, 10)));
        // Named after the type it is of.
        assert!(!named(&symbol("impl Store", 0, 5, 10)));
        // Named with its receiver, which the server is asked about.
        assert!(!named(&symbol("(*Store).Add", 3, 16, 19)));
    }

    #[test]
    fn a_file_is_asked_of_once_and_told_once_learned() {
        let root = TempDir::new();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let main = root.join("src/main.rs");
        std::fs::write(&main, "fn main() {}\n").unwrap();
        let (outlines, mut asked) = Outlines::new();

        let json = outlines.told(&root, &main).unwrap();
        assert_eq!(json["name"], "src/main.rs");
        assert_eq!(json["lines"], 1);
        assert_eq!(json["language"], "rust");
        assert_eq!(json["pending"], true);
        assert_eq!(json["symbols"], serde_json::Value::Null);
        assert_eq!(asked.try_recv().unwrap(), main);
        // Asked of again before it is answered, it is not asked twice.
        let json = outlines.told(&root, &main).unwrap();
        assert_eq!(json["pending"], true);
        assert!(asked.try_recv().is_err());

        outlines.saying(&main, Some("rust-analyzer: Indexing".to_owned()));
        assert_eq!(
            outlines.told(&root, &main).unwrap()["said"],
            "rust-analyzer: Indexing"
        );
        // The symbols come first, with more to wait for.
        outlines.partly(
            &main,
            Told {
                server: Some("rust-analyzer".to_owned()),
                symbols: vec![Outlined {
                    name: "main".to_owned(),
                    ..Outlined::default()
                }],
                ..Told::default()
            },
        );
        let json = outlines.told(&root, &main).unwrap();
        assert_eq!(json["pending"], true);
        assert_eq!(json["symbols"][0]["name"], "main");
        assert_eq!(json["symbols"][0]["uses"], serde_json::Value::Null);
        outlines.learned(
            &main,
            Told {
                server: Some("rust-analyzer".to_owned()),
                symbols: vec![Outlined {
                    name: "main".to_owned(),
                    kind: "function".to_owned(),
                    line: 1,
                    end: 1,
                    uses: Some(0),
                    ..Outlined::default()
                }],
                used_by: vec![UsedBy {
                    path: "src/lib.rs".to_owned(),
                    count: 2,
                }],
                error: None,
            },
        );
        let json = outlines.told(&root, &main).unwrap();
        assert_eq!(json["pending"], false);
        assert_eq!(json["said"], serde_json::Value::Null);
        assert_eq!(json["server"], "rust-analyzer");
        assert_eq!(json["symbols"][0]["name"], "main");
        assert_eq!(json["symbols"][0]["uses"], 0);
        assert_eq!(json["used_by"][0]["path"], "src/lib.rs");
        assert!(asked.try_recv().is_err());

        // Written since, it is asked of again, and tells the old answer
        // meanwhile.
        std::fs::write(&main, "fn main() {}\nfn run() {}\n").unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(2);
        std::fs::File::options()
            .write(true)
            .open(&main)
            .unwrap()
            .set_modified(later)
            .unwrap();
        let json = outlines.told(&root, &main).unwrap();
        assert_eq!(json["pending"], true);
        assert_eq!(json["lines"], 2);
        assert_eq!(json["symbols"][0]["name"], "main");
        assert_eq!(asked.try_recv().unwrap(), main);

        // A file outside the project is not told; one that is not there
        // is told so, and not asked of.
        assert_eq!(outlines.told(&root, Path::new("/etc/hostname")), None);
        let json = outlines.told(&root, &root.join("src/gone.rs")).unwrap();
        assert_eq!(json["exists"], false);
        assert_eq!(json["pending"], false);
        assert!(asked.try_recv().is_err());
    }

    #[test]
    fn a_symbol_is_outlined_with_what_is_within_it() {
        use cox_editor::lsp::Position;
        let at = |line: u32, character: u32| Position { line, character };
        let field = Symbol {
            name: "cwd".to_owned(),
            kind: "field".to_owned(),
            range: at(12, 4)..at(12, 16),
            selection: at(12, 4)..at(12, 7),
            ..Symbol::default()
        };
        let stage = Symbol {
            name: "Stage".to_owned(),
            kind: "struct".to_owned(),
            detail: Some("pub struct Stage".to_owned()),
            range: at(10, 0)..at(40, 1),
            selection: at(10, 11)..at(10, 16),
            children: vec![field],
        };
        let outlined = outline(&stage, 0);
        assert_eq!(outlined.line, 11);
        assert_eq!(outlined.end, 41);
        assert_eq!(outlined.uses, None);
        assert_eq!(outlined.children[0].name, "cwd");
        assert_eq!(outlined.children[0].line, 13);
    }
}
