//! What the project's language server says a step of a flow calls, for
//! the Map page to name a step by the function it runs where the boxes
//! hold the code (see [`crate::stage::map`]): at the line the step is
//! made at, its `at` in `.cox/map.json`, the function the line is in,
//! and the functions it calls on that line, as the server's call
//! hierarchy has them, or, where that names none, as the server says
//! each name called on the line is defined. The page reads it as JSON from
//! `cox://api/call/<path>:<line>` (see [`crate::stage::plugins`]), which
//! answers at once with what is known, and has the place asked of the
//! server when it is not, or not any more, as `cox://api/file/<path>`
//! does (see [`crate::stage::outline`]): the page reads again every two
//! seconds, and gets the answer once the server has given it.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime};

use cox_editor::buffer::{Buffer, Point};
use cox_editor::language::Language;
use cox_editor::lsp::{Call, Client, Error, OpenFile, Position, Symbol};
use cox_editor::syntax::Syntax;
use gpui::{AsyncApp, Context, Entity, Task};
use serde::Serialize;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::stage::Stage;
use crate::stage::outline::{GOOD_FOR, running, settled, symbols_of};
use crate::stage::servers::LanguageServer;

/// A place asked of: a file, as an absolute path, and a line in it, from
/// 1.
type At = (PathBuf, usize);

/// What is known of the places asked of, for one chat's stage, and the
/// way to have a place asked of the server.
#[derive(Clone)]
pub struct Calls {
    known: Arc<Mutex<HashMap<At, Known>>>,
    asks: UnboundedSender<At>,
}

/// A place asked of: when, as its file was then, and what came of it.
struct Known {
    asked: Instant,
    /// When the file was last written, when it was asked of.
    modified: Option<SystemTime>,
    /// Whether the answer is still waited for.
    waiting: bool,
    /// What the server is doing, while the answer is waited for.
    said: Option<String>,
    /// The answer; the last one stays while a new one is waited for.
    told: Option<Told>,
}

/// What the server said of a place.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Told {
    /// The server's program, or `None` for a file no server knows.
    pub server: Option<String>,
    /// The function the line is in, if one is.
    pub function: Option<Named>,
    /// The functions it calls on the line, in the order the server
    /// lists them.
    pub calls: Vec<Named>,
    /// Why the server did not answer, if it did not.
    pub error: Option<String>,
}

/// A function, as the page names it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Named {
    pub name: String,
    /// The kind's name, as the protocol names it, in lower case.
    pub kind: String,
    /// What the server adds, such as a signature.
    pub detail: Option<String>,
    /// The file it is in, from the project's folder, or absolute when
    /// outside it.
    pub path: String,
    /// The line its name is on, from 1.
    pub line: usize,
    /// Where on the line asked of it is called, from 1, for a function
    /// called there; for the function the line is in, where its name is
    /// on its own line.
    pub column: usize,
}

impl Calls {
    /// Nothing known yet, and what hears the places to ask of, which
    /// [`answer`] takes.
    #[must_use]
    pub fn new() -> (Self, UnboundedReceiver<At>) {
        let (asks, asked) = unbounded_channel();
        (
            Self {
                known: Arc::default(),
                asks,
            },
            asked,
        )
    }

    /// What is known of `line` of the file at `path`, an absolute path, in
    /// the project in `root`, as JSON, or `None` for a path outside the
    /// project: the file and the line, `pending` while the server's
    /// answer is waited for, and `said`, what the server is doing then,
    /// with the answer (see [`Told`]). A place not asked of yet, or whose
    /// file was written since, or asked of a while ago, is asked of
    /// again.
    pub fn told(&self, root: &Path, path: &Path, line: usize) -> Option<serde_json::Value> {
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
        let exists = std::fs::read_to_string(&path).is_ok();
        let at = (path.clone(), line);

        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let ask = match known.get(&at) {
            None => true,
            Some(known) => {
                known.modified != modified
                    || (!known.waiting && now.duration_since(known.asked) > GOOD_FOR)
            }
        };
        if ask && exists {
            let told = known.get(&at).and_then(|known| known.told.clone());
            known.insert(
                at.clone(),
                Known {
                    asked: now,
                    modified,
                    waiting: true,
                    said: None,
                    told,
                },
            );
            // Nothing answers once the stage is gone, which the page is too.
            let _ = self.asks.send(at.clone());
        }
        let (pending, said, told) = match known.get(&at) {
            Some(known) => (known.waiting, known.said.clone(), known.told.clone()),
            None => (false, None, None),
        };
        drop(known);

        let mut json = serde_json::json!({
            "path": path.display().to_string(),
            "name": name,
            "line": line,
            "exists": exists,
            "pending": pending,
            "said": said,
            "server": serde_json::Value::Null,
            "function": serde_json::Value::Null,
            "calls": serde_json::Value::Null,
            "error": serde_json::Value::Null,
        });
        if let Some(told) = told {
            json["server"] = serde_json::json!(told.server);
            json["function"] = serde_json::to_value(&told.function).unwrap_or_default();
            json["calls"] = serde_json::to_value(&told.calls).unwrap_or_default();
            json["error"] = serde_json::json!(told.error);
        }
        Some(json)
    }

    /// The server says `said` of itself, while the place `at` waits for
    /// it.
    fn saying(&self, at: &At, said: Option<String>) {
        if let Some(known) = self
            .known
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(at)
        {
            known.said = said;
        }
    }

    /// The server answered `told` of the place `at`.
    pub fn learned(&self, at: &At, told: Told) {
        let mut known = self.known.lock().unwrap_or_else(PoisonError::into_inner);
        match known.get_mut(at) {
            Some(known) => {
                known.waiting = false;
                known.said = None;
                known.told = Some(told);
                // Good for a while from now, not from when it was asked.
                known.asked = Instant::now();
            }
            None => {
                known.insert(
                    at.clone(),
                    Known {
                        asked: Instant::now(),
                        modified: None,
                        waiting: false,
                        said: None,
                        told: Some(told),
                    },
                );
            }
        }
    }
}

/// Answers the places `asked` names, one after another, for a chat
/// working in `cwd`: asks the project's server of each (see [`learn`]),
/// and tells `calls` what it said.
pub fn answer(
    mut asked: UnboundedReceiver<At>,
    calls: Calls,
    cwd: PathBuf,
    cx: &mut Context<Stage>,
) -> Task<()> {
    cx.spawn(async move |_, cx| {
        while let Some(at) = asked.recv().await {
            let told = learn(&at, &cwd, &calls, cx).await;
            calls.learned(&at, told);
        }
    })
}

/// What the project's server says of the place `at`: the function the
/// line is in, among the symbols of the file, and, once the server has
/// read the project, the functions it calls on the line; `calls` hears
/// what the server is doing meanwhile.
async fn learn(at: &At, cwd: &Path, calls: &Calls, cx: &mut AsyncApp) -> Told {
    let (path, line) = at;
    let saying = |said: Option<String>| calls.saying(at, said);
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
    let Some(within) = u32::try_from(line.saturating_sub(1))
        .ok()
        .and_then(|line| function_at(&symbols, line))
    else {
        return Told {
            server: Some(name),
            ..Told::default()
        };
    };
    let function = Some(named(
        &within.name,
        &within.kind,
        within.detail.as_deref(),
        path,
        within.selection.start,
        cwd,
    ));
    // The calls, once the server has been through with the project: it
    // knows what a name is only then.
    if let Err(error) = settled(&server, began, &saying, cx).await {
        return failed(error);
    }
    calls.saying(at, Some(format!("{name}: finding what the line calls")));
    let asked = server.read_with(cx, |server, _| {
        server
            .client()
            .map(|client| client.calls(&document, within.selection.start))
    });
    let Ok(Some(asked)) = asked else {
        return failed(format!("{name} stopped"));
    };
    // A server with no call hierarchy is asked what each name called
    // on the line is, below.
    let answered = match asked.await {
        Ok(Some((_, calls))) => calls,
        Ok(None) | Err(Error::Unsupported(_)) => Vec::new(),
        Err(error) => return failed(error.to_string()),
    };
    // The calls made on the line: those that start on it first; then,
    // since the call hierarchy leaves some calls out, as one of a
    // function of the same file, each name called on the line, as the
    // server says it is defined; and those whose place spans the line,
    // as a call in a macro's lines is placed, only when nothing else is
    // found.
    let line0 = u32::try_from(line.saturating_sub(1)).unwrap_or(0);
    let mut called = called_on(&answered, line0, cwd, &|range| range.start.line == line0);
    if called.is_empty() {
        let names = names_called(path, &text, line0, &server, &document, cx).await;
        called = defined_on(&server, &document, names, cwd, cx).await;
    }
    drop(document);
    if called.is_empty() {
        called = called_on(&answered, line0, cwd, &|range| {
            range.start.line < line0 && line0 <= range.end.line
        });
    }
    called.sort_by_key(|call| call.column);
    Told {
        server: Some(name),
        function,
        calls: called,
        error: None,
    }
}

/// The functions `answered`, the calls a function makes, calls at a place
/// `keep` keeps, as the page names them, told from `cwd`, each with where
/// on `line0`, from 0, it is called, or 1 for a call whose place starts
/// on a line above.
fn called_on(
    answered: &[Call],
    line0: u32,
    cwd: &Path,
    keep: &dyn Fn(&&Range<Position>) -> bool,
) -> Vec<Named> {
    answered
        .iter()
        .filter_map(|call| {
            let from = call
                .from
                .iter()
                .filter(keep)
                .min_by_key(|range| range.start)?;
            let mut it = named(
                &call.to.name,
                &call.to.kind,
                call.to.detail.as_deref(),
                &call.to.path,
                call.to.selection.start,
                cwd,
            );
            it.column = if from.start.line == line0 {
                from.start.character as usize + 1
            } else {
                1
            };
            Some(it)
        })
        .collect()
}

/// A name called on a line: where on the line it is, in bytes, the name,
/// and where it is as the server counts the line.
type Called = (usize, String, Position);

/// The names called on line `line0`, from 0, of `text`, the file at
/// `path`, open as `document` on `server`, in order: as the syntax of the
/// file's language says, through its tags query (see
/// [`Syntax::calls`]), or, for a file in a language with none, as the
/// server says what each name on the line is, the functions and methods
/// there not being defined there.
async fn names_called(
    path: &Path,
    text: &str,
    line0: u32,
    server: &Entity<LanguageServer>,
    document: &OpenFile,
    cx: &mut AsyncApp,
) -> Vec<Called> {
    let buffer = Buffer::new(text);
    let line = line0 as usize;
    let line_text = buffer.line(line).into_owned();
    let encoding = server
        .read_with(cx, |server, _| server.client().map(Client::encoding))
        .ok()
        .flatten()
        .unwrap_or_default();
    let syntax =
        Language::of_path(path).and_then(|language| Syntax::parse(language, &buffer, None));
    if let Some(calls) = syntax.and_then(|syntax| syntax.calls(&buffer, line)) {
        return calls
            .into_iter()
            .map(|name| {
                let at = encoding.position(&buffer, Point::new(line, name.start));
                (name.start, line_text[name].to_owned(), at)
            })
            .collect();
    }
    let asked = server.read_with(cx, |server, _| {
        server.client().map(|client| client.tokens(document))
    });
    let Ok(Some(asked)) = asked else {
        return Vec::new();
    };
    let Ok(tokens) = asked.await else {
        return Vec::new();
    };
    tokens
        .into_iter()
        .filter(|token| {
            token.start.line == line0
                && matches!(token.kind.as_str(), "function" | "method" | "macro")
                && !token
                    .modifiers
                    .iter()
                    .any(|modifier| modifier == "declaration" || modifier == "definition")
        })
        .filter_map(|token| {
            let start = encoding.point(&buffer, token.start);
            let end = encoding.point(
                &buffer,
                Position {
                    character: token.start.character + token.length,
                    ..token.start
                },
            );
            let name = line_text.get(start.column..end.column)?.to_owned();
            Some((start.column, name, token.start))
        })
        .collect()
}

/// The names `names` called on a line of `document` on `server`, each as
/// the server says it is defined, as the page names them, told from
/// `cwd`, with where on the line it is called; those the server finds no
/// definition of are left out.
async fn defined_on(
    server: &Entity<LanguageServer>,
    document: &OpenFile,
    names: Vec<Called>,
    cwd: &Path,
    cx: &mut AsyncApp,
) -> Vec<Named> {
    let mut called = Vec::new();
    for (column, called_name, at) in names {
        let asked = server.read_with(cx, |server, _| {
            server
                .client()
                .map(|client| client.definition(document, at))
        });
        let Ok(Some(asked)) = asked else {
            break;
        };
        let Ok(found) = asked.await else {
            continue;
        };
        let Some(location) = found.into_iter().next() else {
            continue;
        };
        let mut it = named(
            &called_name,
            "",
            None,
            &location.path,
            location.range.start,
            cwd,
        );
        it.column = column + 1;
        called.push(it);
    }
    called
}

/// The innermost function among `symbols`, and those within them, that
/// `line`, from 0, is in.
fn function_at(symbols: &[Symbol], line: u32) -> Option<&Symbol> {
    let mut found = None;
    for symbol in symbols {
        if symbol.range.start.line > line || line > symbol.range.end.line {
            continue;
        }
        if matches!(symbol.kind.as_str(), "function" | "method" | "constructor") {
            found = Some(symbol);
        }
        if let Some(inner) = function_at(&symbol.children, line) {
            found = Some(inner);
        }
    }
    found
}

/// A function as the page names it: its name `at`, as the server counts
/// it, in the file at `path`, told from `cwd`.
fn named(
    name: &str,
    kind: &str,
    detail: Option<&str>,
    path: &Path,
    at: Position,
    cwd: &Path,
) -> Named {
    Named {
        name: name.to_owned(),
        kind: kind.to_owned(),
        detail: detail.map(str::to_owned),
        path: path
            .strip_prefix(cwd)
            .map_or_else(|_| path.display().to_string(), |p| p.display().to_string()),
        line: at.line as usize + 1,
        column: at.character as usize + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("cox-calls-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::canonicalize(directory).unwrap()
    }

    #[test]
    fn a_place_is_asked_of_once_and_told_once_learned() {
        let root = scratch("told");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let main = root.join("src/main.rs");
        std::fs::write(&main, "fn main() {\n    run();\n}\n").unwrap();
        let (calls, mut asked) = Calls::new();

        let json = calls.told(&root, &main, 2).unwrap();
        assert_eq!(json["name"], "src/main.rs");
        assert_eq!(json["line"], 2);
        assert_eq!(json["pending"], true);
        assert_eq!(json["calls"], serde_json::Value::Null);
        assert_eq!(asked.try_recv().unwrap(), (main.clone(), 2));
        // Asked of again before it is answered, it is not asked twice;
        // another line of the file is a place of its own.
        assert_eq!(calls.told(&root, &main, 2).unwrap()["pending"], true);
        assert!(asked.try_recv().is_err());
        calls.told(&root, &main, 1).unwrap();
        assert_eq!(asked.try_recv().unwrap(), (main.clone(), 1));

        calls.saying(
            &(main.clone(), 2),
            Some("rust-analyzer: Indexing".to_owned()),
        );
        assert_eq!(
            calls.told(&root, &main, 2).unwrap()["said"],
            "rust-analyzer: Indexing"
        );
        calls.learned(
            &(main.clone(), 2),
            Told {
                server: Some("rust-analyzer".to_owned()),
                function: Some(Named {
                    name: "main".to_owned(),
                    kind: "function".to_owned(),
                    path: "src/main.rs".to_owned(),
                    line: 1,
                    ..Named::default()
                }),
                calls: vec![Named {
                    name: "run".to_owned(),
                    kind: "function".to_owned(),
                    detail: Some("fn run()".to_owned()),
                    path: "src/lib.rs".to_owned(),
                    line: 3,
                    column: 4,
                }],
                error: None,
            },
        );
        let json = calls.told(&root, &main, 2).unwrap();
        assert_eq!(json["pending"], false);
        assert_eq!(json["function"]["name"], "main");
        assert_eq!(json["calls"][0]["name"], "run");
        assert_eq!(json["calls"][0]["path"], "src/lib.rs");
        assert_eq!(json["calls"][0]["line"], 3);
        assert!(asked.try_recv().is_err());

        // A file outside the project is not told; one that is not there
        // is told so, and not asked of.
        assert_eq!(calls.told(&root, Path::new("/etc/hostname"), 1), None);
        let json = calls.told(&root, &root.join("src/gone.rs"), 1).unwrap();
        assert_eq!(json["exists"], false);
        assert_eq!(json["pending"], false);
        assert!(asked.try_recv().is_err());
    }

    #[test]
    fn the_innermost_function_of_a_line_is_found() {
        let at = |line: u32, character: u32| Position { line, character };
        let method = Symbol {
            name: "show".to_owned(),
            kind: "method".to_owned(),
            range: at(12, 4)..at(20, 5),
            selection: at(12, 11)..at(12, 15),
            ..Symbol::default()
        };
        let stage = Symbol {
            name: "impl Stage".to_owned(),
            kind: "object".to_owned(),
            range: at(10, 0)..at(40, 1),
            selection: at(10, 5)..at(10, 10),
            children: vec![method],
            ..Symbol::default()
        };
        let main = Symbol {
            name: "main".to_owned(),
            kind: "function".to_owned(),
            range: at(50, 0)..at(52, 1),
            selection: at(50, 3)..at(50, 7),
            ..Symbol::default()
        };
        let symbols = [stage, main];
        assert_eq!(function_at(&symbols, 15).unwrap().name, "show");
        assert_eq!(function_at(&symbols, 51).unwrap().name, "main");
        assert!(function_at(&symbols, 30).is_none());
        assert!(function_at(&symbols, 99).is_none());
    }
}
