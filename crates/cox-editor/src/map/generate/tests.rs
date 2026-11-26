//! The map generated from small indexes, built here as an indexer would
//! write them, of files whose text the tests give.

use std::collections::{BTreeMap, BTreeSet};

use prost::Message;

use super::text::{doc_of, sig_of};
use super::*;
use crate::map::generated::{CallKind, DriftKind};
use crate::scip::{Document, Occurrence, SymbolInformation, kind, role};

/// A file of a project, and what an index says of it.
struct File {
    path: String,
    lines: Vec<String>,
    occurrences: Vec<Occurrence>,
    symbols: Vec<SymbolInformation>,
}

impl File {
    fn new(path: &str, text: &str) -> Self {
        Self {
            path: path.to_owned(),
            lines: text.lines().map(str::to_owned).collect(),
            occurrences: Vec::new(),
            symbols: Vec::new(),
        }
    }

    /// The range of `name` on `line`.
    fn range(&self, line: usize, name: &str) -> Vec<i32> {
        let start = self.lines[line]
            .find(name)
            .unwrap_or_else(|| panic!("no `{name}` on line {line} of {}", self.path));
        let line = i32::try_from(line).unwrap();
        let start = i32::try_from(start).unwrap();
        vec![line, start, start + i32::try_from(name.len()).unwrap()]
    }

    /// Defines `symbol`, a `kind`, named `name` on `line`, its definition
    /// spanning the lines `spans`.
    fn define(&mut self, symbol: &str, kind: i32, line: usize, name: &str, spans: (usize, usize)) {
        let (start, end) = (
            i32::try_from(spans.0).unwrap(),
            i32::try_from(spans.1).unwrap(),
        );
        self.occurrences.push(Occurrence {
            range: self.range(line, name),
            symbol: symbol.to_owned(),
            symbol_roles: role::DEFINITION,
            enclosing_range: vec![start, 0, end, 1],
            ..Occurrence::default()
        });
        self.symbols.push(SymbolInformation {
            symbol: symbol.to_owned(),
            kind,
            documentation: vec![format!("The {name}. More of it.")],
            ..SymbolInformation::default()
        });
    }

    fn refer(&mut self, symbol: &str, line: usize, name: &str) {
        self.refer_as(symbol, line, name, 0);
    }

    /// Names `symbol` on `line`, in the [`role`]s `roles`.
    fn refer_as(&mut self, symbol: &str, line: usize, name: &str, roles: i32) {
        self.occurrences.push(Occurrence {
            range: self.range(line, name),
            symbol: symbol.to_owned(),
            symbol_roles: roles,
            ..Occurrence::default()
        });
    }

    fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// The index of `files`, and their texts.
fn index(files: &[File]) -> (Index, BTreeMap<String, String>) {
    let documents = files
        .iter()
        .map(|file| Document {
            relative_path: file.path.clone(),
            occurrences: file.occurrences.clone(),
            symbols: file.symbols.clone(),
            position_encoding: 1,
            ..Document::default()
        })
        .collect();
    let index = Index { documents };
    let texts = files
        .iter()
        .map(|file| (file.path.clone(), file.text()))
        .collect();
    (index, texts)
}

/// A symbol of the Rust package `app`, as rust-analyzer writes it.
fn rust(path: &str) -> String {
    format!("rust-analyzer cargo app 0.1.0 {path}")
}

/// A small app: a server, which keeps a store and calls a helper; the
/// store; a module of the server's methods alone; and the helper, which
/// no box owns.
fn app() -> (Index, BTreeMap<String, String>) {
    let mut server = File::new(
        "src/server.rs",
        "use crate::store::Store;\n\
         /// Serves.\n\
         pub struct Server {\n    \
             store: Store,\n    \
             count: usize,\n\
         }\n\
         impl Server {\n    \
             pub fn run(&mut self) {\n        \
                 self.store.put(1);\n        \
                 self.count = 2;\n",
    );
    for _ in 0..20 {
        server.lines.push("        helper();".to_owned());
    }
    server.lines.extend(["    }".to_owned(), "}".to_owned()]);
    let end = server.lines.len() - 1;
    server.refer(&rust("store/Store#"), 0, "Store");
    server.define(&rust("server/Server#"), kind::STRUCT, 2, "Server", (1, 5));
    server.define(
        &rust("server/Server#store."),
        kind::FIELD,
        3,
        "store",
        (3, 3),
    );
    server.refer(&rust("store/Store#"), 3, "Store");
    server.define(
        &rust("server/Server#count."),
        kind::FIELD,
        4,
        "count",
        (4, 4),
    );
    server.define(
        &rust("server/impl#[Server]run()."),
        kind::METHOD,
        7,
        "run",
        (7, end - 1),
    );
    server.refer(&rust("server/Server#store."), 8, "store");
    server.refer(&rust("store/impl#[Store]put()."), 8, "put");
    server.refer(&rust("server/Server#count."), 9, "count");
    for line in 10..30 {
        server.refer(&rust("util/helper()."), line, "helper");
    }

    let mut store = File::new(
        "src/store.rs",
        "/// Keeps.\n\
         pub struct Store {\n    \
             items: Vec<u32>,\n\
         }\n\
         impl Store {\n    \
             pub fn put(&mut self, item: u32) {\n        \
                 self.items.push(item);\n    \
             }\n\
         }",
    );
    store.define(&rust("store/Store#"), kind::STRUCT, 1, "Store", (0, 3));
    store.define(&rust("store/Store#items."), kind::FIELD, 2, "items", (2, 2));
    store.define(
        &rust("store/impl#[Store]put()."),
        kind::METHOD,
        5,
        "put",
        (5, 7),
    );
    store.refer(&rust("store/Store#items."), 6, "items");

    let mut glue = File::new(
        "src/server/glue.rs",
        "impl Server {\n    pub fn extra(&self) {}\n}",
    );
    glue.define(
        &rust("server/glue/impl#[Server]extra()."),
        kind::METHOD,
        1,
        "extra",
        (1, 1),
    );

    let mut util = File::new("src/util.rs", "pub fn helper() {}");
    util.define(&rust("util/helper()."), kind::FUNCTION, 0, "helper", (0, 0));

    index(&[server, store, glue, util])
}

const APP: &str = r#"{
  "says": "A small app",
  "nodes": {
    "app": { "name": "App" },
    "user": { "name": "User", "icon": "person", "kind": "outside" },
    "server": { "name": "Server", "claims": ["app/server"] },
    "store": { "name": "Store", "claims": ["app/store"] },
    "glue": { "name": "Glue", "claims": ["app/server/glue"] },
    "gone": { "name": "Gone", "claims": ["app/gone"] }
  },
  "top": { "center": "app", "left": ["user"], "lines": [{ "from": "user", "to": "app" }] },
  "levels": {
    "app": {
      "box": [-400, -260, 800, 520],
      "outer": { "user": ["left", 0] },
      "parts": { "server": [-350, -200], "store": [100, -200], "glue": [-350, 100], "gone": [100, 100] },
      "lines": [{ "from": "user", "to": "server", "both": true }]
    }
  },
  "agreed": [
    { "id": "unowned:app/util", "why": "Helpers every box uses." },
    { "id": "claim-gone:app/old", "why": "Renamed." }
  ]
}"#;

fn generated(vocabulary: &str, (index, texts): &(Index, BTreeMap<String, String>)) -> Generated {
    generate(index, &Vocabulary::parse(vocabulary).unwrap(), "", texts)
}

fn drift_ids(map: &Generated) -> Vec<&str> {
    map.drift.iter().map(|drift| drift.id.as_str()).collect()
}

#[test]
fn a_library_target_rust_analyzer_names_is_no_module_another_is() {
    let vocabulary = r#"{ "nodes": { "app": { "name": "App" }, "all": { "name": "All", "claims": ["app"] } },
                          "top": { "center": "app" },
                          "levels": { "app": { "box": [0, 0, 400, 300], "parts": { "all": [0, 0] } } } }"#;
    let types = |symbols: &[&str]| -> Vec<String> {
        let mut file = File::new("src/lib.rs", "pub struct Thing;\n");
        for symbol in symbols {
            file.define(&rust(symbol), kind::STRUCT, 0, "Thing", (0, 0));
        }
        // A function of an `extern` block, which no release names a
        // target of.
        file.define(&rust("dlopen()."), kind::FUNCTION, 0, "Thing", (0, 0));
        generated(vocabulary, &index(&[file]))
            .types
            .into_keys()
            .collect()
    };
    // A newer release names every module's target first: the library's
    // is no module, and two binaries' modules stay apart.
    assert_eq!(
        types(&[
            "lib/Thing#",
            "lib/store/Thing#",
            "bin/a/foo/Thing#",
            "bin/b/foo/Thing#"
        ]),
        [
            "app/Thing",
            "app/bin/a/foo/Thing",
            "app/bin/b/foo/Thing",
            "app/store/Thing"
        ]
    );
    // An older release names none, and names the crate's top `crate`,
    // so its module `lib` is one.
    assert_eq!(
        types(&["crate/", "Thing#", "lib/store/Thing#"]),
        ["app/Thing", "app/lib/store/Thing"]
    );
}

#[test]
fn each_box_owns_the_types_of_the_modules_it_claims() {
    let map = generated(APP, &app());
    let server = &map.types["app/server/Server"];
    assert_eq!(server.owner, "server");
    assert_eq!(server.kind, "struct");
    assert_eq!((server.file.as_str(), server.line), ("src/server.rs", 3));
    assert_eq!(server.doc, "The Server.");
    let fields: Vec<&str> = server
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect();
    assert_eq!(fields, ["store", "count"]);
    // The methods of another module's `impl Server` are still the
    // server's, though its module is the glue's.
    let functions: Vec<&str> = server
        .functions
        .iter()
        .map(|function| function.key.as_str())
        .collect();
    assert_eq!(
        functions,
        ["app/server/Server::run", "app/server/Server::extra"]
    );
    assert_eq!(map.types["app/store/Store"].owner, "store");
    assert_eq!(map.types.len(), 2);
    // A field's type names the box that owns it.
    assert_eq!(server.fields[0].to, ["store"]);
    assert_eq!(map.nodes["app/server/Server"].kind, NodeKind::Type);
    assert_eq!(
        map.nodes["app/server/Server"].code.as_deref(),
        Some("struct")
    );
    assert_eq!(map.nodes["server"].code.as_deref(), Some("app/server"));
    assert_eq!(map.nodes["user"].kind, NodeKind::Outside);
}

#[test]
fn each_file_a_box_owns_names_the_box() {
    let map = generated(APP, &app());
    let files: Vec<(&str, &str)> = map
        .files
        .iter()
        .map(|(file, owner)| (file.as_str(), owner.as_str()))
        .collect();
    // The helper's file no box owns.
    assert_eq!(
        files,
        [
            ("src/server.rs", "server"),
            ("src/server/glue.rs", "glue"),
            ("src/store.rs", "store"),
        ]
    );
}

#[test]
fn uses_and_calls_tell_where_a_member_is_named_and_written() {
    let map = generated(APP, &app());
    let server = &map.types["app/server/Server"];
    let put = &map.types["app/store/Store"].functions[0];
    assert_eq!(put.uses.len(), 1);
    assert_eq!(put.uses[0].within, "Server::run");
    assert_eq!(
        put.uses[0].function.as_deref(),
        Some("app/server/Server::run")
    );
    assert_eq!(put.uses[0].node.as_deref(), Some("app/server/Server"));
    assert!(server.fields[1].uses[0].writes);
    assert!(!server.fields[0].uses[0].writes);
    assert!(map.types["app/store/Store"].fields[0].uses[0].writes);
    // The store named in the server's field is a use in its declaration;
    // the one imported above it is none.
    let store_uses = &map.types["app/store/Store"].uses;
    assert_eq!(store_uses.len(), 1);
    assert_eq!(store_uses[0].within, "Server.store");
    assert!(store_uses[0].declaration);
    assert_eq!(store_uses[0].node.as_deref(), Some("server"));

    let calls: Vec<(&str, CallKind, Option<&str>, usize, bool)> = server.functions[0]
        .calls
        .iter()
        .map(|call| {
            (
                call.name.as_str(),
                call.kind,
                call.node.as_deref(),
                call.count,
                call.writes,
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            (
                "Server.store",
                CallKind::Field,
                Some("app/server/Server"),
                1,
                false
            ),
            (
                "Store::put",
                CallKind::Function,
                Some("app/store/Store"),
                1,
                false
            ),
            (
                "Server.count",
                CallKind::Field,
                Some("app/server/Server"),
                1,
                true
            ),
            ("util::helper", CallKind::Function, None, 20, false),
        ]
    );
}

#[test]
fn the_lines_between_boxes_come_from_where_their_code_meets() {
    let map = generated(APP, &app());
    assert_eq!(
        map.edges,
        [Edge {
            a: "server".to_owned(),
            b: "store".to_owned(),
            count: 2
        }]
    );
    let lines: Vec<(&str, &str, bool, bool)> = map.levels["app"]
        .lines
        .iter()
        .map(|line| (line.from.as_str(), line.to.as_str(), line.both, line.weak))
        .collect();
    assert_eq!(
        lines,
        [
            ("user", "server", true, false),
            ("server", "store", false, false)
        ]
    );
    // Inside each box, a line from the type that names the other box, and
    // to the type the other box uses; the user's line, which no code
    // backs, goes on to the server's edge.
    let inside = |owner: &str| -> Vec<(String, String)> {
        map.levels[owner]
            .lines
            .iter()
            .map(|line| (line.from.clone(), line.to.clone()))
            .collect()
    };
    assert_eq!(
        inside("server"),
        [
            ("app/server/Server".to_owned(), "store".to_owned()),
            ("user".to_owned(), "server".to_owned())
        ]
    );
    assert_eq!(
        inside("store"),
        [("server".to_owned(), "app/store/Store".to_owned())]
    );
    assert!(map.levels["server"].parts.contains_key("app/server/Server"));
    assert!(!map.levels.contains_key("glue"));
}

#[test]
fn the_drift_is_where_the_vocabulary_and_the_code_disagree() {
    let map = generated(APP, &app());
    assert_eq!(
        drift_ids(&map),
        [
            "claim-gone:app/gone",
            "no-types:glue",
            "unowned:app/util",
            "agreed-stale:claim-gone:app/old",
        ]
    );
    let kinds: Vec<DriftKind> = map.drift.iter().map(|drift| drift.kind).collect();
    assert_eq!(
        kinds,
        [
            DriftKind::ClaimGone,
            DriftKind::NoTypes,
            DriftKind::Unowned,
            DriftKind::AgreedStale
        ]
    );
    assert_eq!(map.drift[0].nodes, ["gone"]);
    assert_eq!(
        map.drift[1].text,
        "Glue owns no types of its own, only functions of Server"
    );
    assert_eq!(map.drift[2].nodes, ["server"]);
    assert_eq!(map.drift[2].count, Some(20));
    assert_eq!(
        map.drift[2].agreed.as_deref(),
        Some("Helpers every box uses.")
    );
    assert_eq!(map.drift[3].agreed, None);
}

#[test]
fn code_that_meets_where_no_level_shows_both_boxes_is_drift() {
    let vocabulary = APP.replace(r#", "store": [100, -200]"#, "");
    let map = generated(&vocabulary, &app());
    let undrawn = map
        .drift
        .iter()
        .find(|drift| drift.kind == DriftKind::Undrawn)
        .unwrap();
    assert_eq!(undrawn.id, "undrawn:server|store");
    assert_eq!(undrawn.count, Some(2));
    let sites: Vec<(&str, usize, &str)> = undrawn
        .at
        .iter()
        .map(|site| (site.file.as_str(), site.line, site.within.as_str()))
        .collect();
    assert_eq!(
        sites,
        [
            ("src/server.rs", 9, "Server::run"),
            ("src/server.rs", 4, "")
        ]
    );
    // A box with no place has no level of its types, though it has them.
    assert!(!map.levels.contains_key("store"));
    assert!(map.types.contains_key("app/store/Store"));
}

/// [`app`]'s server, and its store in a box of its own, `group`, which
/// keeps its lines as `lines` says, and holds a line as `line` writes it;
/// the group's level shows the server around it, as `at` writes it.
fn grouped(lines: &str, line: &str, at: &str) -> String {
    format!(
        r#"{{
          "nodes": {{
            "app": {{ "name": "App" }},
            "server": {{ "name": "Server", "claims": ["app/server"] }},
            "group": {{ "name": "Group", "lines": "{lines}" }},
            "store": {{ "name": "Store", "claims": ["app/store"] }}
          }},
          "top": {{ "center": "app" }},
          "levels": {{
            "app": {{
              "box": [0, 0, 800, 500],
              "parts": {{ "server": [0, 0], "group": [300, 0] }},
              "lines": [{line}]
            }},
            "group": {{
              "box": [0, 0, 800, 500],
              "at": {{ {at} }},
              "parts": {{ "store": [300, 0] }}
            }}
          }},
          "agreed": [{{ "id": "unowned:app/util", "why": "Helpers." }}]
        }}"#
    )
}

const SERVER_AT: &str = r#""server": [0, 0]"#;

/// The drift of `map` not agreed on.
fn open_drift(map: &Generated) -> Vec<&str> {
    map.drift
        .iter()
        .filter(|drift| drift.agreed.is_none())
        .map(|drift| drift.id.as_str())
        .collect()
}

fn line_ends(map: &Generated, level: &str) -> Vec<(String, String, bool)> {
    map.levels[level]
        .lines
        .iter()
        .map(|line| (line.from.clone(), line.to.clone(), line.weak))
        .collect()
}

#[test]
fn a_box_drawn_closed_stands_for_the_boxes_inside_it() {
    let map = generated(&grouped("generated", "", SERVER_AT), &app());
    let line = |from: &str, to: &str| (from.to_owned(), to.to_owned(), false);
    // Where the group is closed, its line is the store's, as often.
    assert_eq!(line_ends(&map, "app"), [line("server", "group")]);
    assert_eq!(line_ends(&map, "group"), [line("server", "store")]);
    let edges: Vec<(&str, &str, usize)> = map
        .edges
        .iter()
        .map(|edge| (edge.a.as_str(), edge.b.as_str(), edge.count))
        .collect();
    assert_eq!(edges, [("group", "server", 2), ("server", "store", 2)]);
    // Inside the server, the field that names the store names the group.
    assert_eq!(
        line_ends(&map, "server"),
        [line("app/server/Server", "group")]
    );
    assert!(map.levels["store"].parts.contains_key("app/store/Store"));
    assert_eq!(open_drift(&map), Vec::<&str>::new());
}

#[test]
fn a_box_that_keeps_its_lines_by_hand_keeps_those_of_the_boxes_inside_it() {
    // Inside the group, where the server is around, the code gives the
    // line; where the group is closed, a person does.
    let map = generated(&grouped("hand", "", SERVER_AT), &app());
    assert_eq!(map.levels["app"].lines, []);
    assert_eq!(line_ends(&map, "group")[0].1, "store");
    assert_eq!(open_drift(&map), Vec::<&str>::new());
    // Where the group's level places none around it, the map places the
    // server there, and the code gives the line all the same.
    let map = generated(&grouped("hand", "", ""), &app());
    assert_eq!(line_ends(&map, "group")[0].1, "store");
    assert_eq!(open_drift(&map), Vec::<&str>::new());
    let map = generated(
        &grouped("hand", r#"{ "from": "server", "to": "group" }"#, ""),
        &app(),
    );
    assert_eq!(map.levels["app"].lines.len(), 1);
    assert_eq!(open_drift(&map), Vec::<&str>::new());
}

/// The ends of the lines of `map`'s levels that a person placed which
/// the level shows nowhere, as its level and the end: the page has
/// nowhere to draw such a line to.
fn ends_nowhere(map: &Generated) -> Vec<(&str, &str)> {
    let mut nowhere = Vec::new();
    for (opened, level) in map.levels.iter().filter(|(_, level)| level.area.is_some()) {
        for end in level.lines.iter().flat_map(|line| [&line.from, &line.to]) {
            if end != opened
                && !level.parts.contains_key(end)
                && !level.at.contains_key(end)
                && !level.outer.contains_key(end)
                && !level.frames.contains_key(end)
            {
                nowhere.push((opened.as_str(), end.as_str()));
            }
        }
    }
    nowhere
}

#[test]
fn a_level_shows_around_its_box_what_its_lines_reach_that_no_one_placed() {
    let project = app();
    // The group's level places none around it: the code's line from the
    // server, beside the group in the level above, still goes to the
    // store, and the server sits left of the group's box, as it does of
    // the group there.
    let map = generated(&grouped("generated", "", ""), &project);
    let level = &map.levels["group"];
    assert_eq!(line_ways(&map, "group"), [("server", "store", false)]);
    let [x, y] = level.at["server"];
    assert!(x + 216 <= 0, "{x}");
    assert!((0..500).contains(&y), "{y}");
    // Inside the store, its type's line goes on to the server.
    assert_eq!(
        line_ways(&map, "store"),
        [("server", "app/store/Store", false)]
    );
    assert_eq!(ends_nowhere(&map), []);

    // So does an end of a line a person drew: the admin, on the left of
    // the top, sits left of the box, clear of the server.
    let to_store = r#"{ "from": "admin", "to": "store" }"#;
    let map = generated(&people("", "", to_store), &project);
    let level = &map.levels["group"];
    let (admin, server) = (level.at["admin"], level.at["server"]);
    assert!(admin[0] + 248 <= 0, "{admin:?}");
    assert!((admin[1] - server[1]).abs() >= 60, "{admin:?} {server:?}");
    assert_eq!(ends_nowhere(&map), []);

    // Where a person places the server, they say what the level shows
    // around it: the code's lines go to the server alone, and the admin,
    // which their line still reaches, sits past where they placed it.
    let map = generated(&people("", r#""server": [-300, 0]"#, to_store), &project);
    let level = &map.levels["group"];
    assert_eq!(level.at["server"], [-300, 0]);
    assert!(level.at["admin"][0] + 248 <= -300, "{:?}", level.at);
    assert_eq!(ends_nowhere(&map), []);
}

/// [`app`]'s server, and its store in a box of its own, `group`, which
/// keeps its lines by hand, among a user and an admin: the user's line
/// goes to the server; the admin's to the app, and on as `admin` writes
/// it; the group's level shows those around it that `at` places, and the
/// lines `lines` writes.
fn people(admin: &str, at: &str, lines: &str) -> String {
    format!(
        r#"{{
          "nodes": {{
            "app": {{ "name": "App" }},
            "user": {{ "name": "User", "kind": "outside" }},
            "admin": {{ "name": "Admin", "kind": "outside" }},
            "server": {{ "name": "Server", "claims": ["app/server"] }},
            "group": {{ "name": "Group", "lines": "hand" }},
            "store": {{ "name": "Store", "claims": ["app/store"] }}
          }},
          "top": {{
            "center": "app",
            "left": ["user", "admin"],
            "lines": [{{ "from": "user", "to": "app", "both": true }}, {{ "from": "admin", "to": "app" }}]
          }},
          "levels": {{
            "app": {{
              "box": [0, 0, 800, 500],
              "outer": {{ "user": ["left", 0], "admin": ["left", 200] }},
              "parts": {{ "server": [0, 0], "group": [300, 0] }},
              "lines": [{{ "from": "user", "to": "server", "both": true }}{admin}]
            }},
            "group": {{
              "box": [0, 0, 800, 500],
              "at": {{ {at} }},
              "parts": {{ "store": [300, 0] }},
              "lines": [{lines}]
            }}
          }},
          "agreed": [{{ "id": "unowned:app/util", "why": "Helpers." }}]
        }}"#
    )
}

/// The lines of `level`, as `from`, `to` and whether both ways.
fn line_ways<'m>(map: &'m Generated, level: &str) -> Vec<(&'m str, &'m str, bool)> {
    map.levels[level]
        .lines
        .iter()
        .map(|line| (line.from.as_str(), line.to.as_str(), line.both))
        .collect()
}

#[test]
fn a_box_opened_keeps_the_lines_a_person_drew_to_it() {
    let project = app();
    let to_group = r#", { "from": "admin", "to": "group" }"#;
    let around = r#""server": [0, 0], "admin": [-300, 200]"#;
    let map = generated(&people(to_group, around, ""), &project);
    // The level a person placed carries the top's lines on to its parts.
    assert_eq!(
        line_ways(&map, "app"),
        [("user", "server", true), ("admin", "group", false)]
    );
    // The server opens on its types, which no code ties to the user: the
    // user's line goes to its edge, beside the one the code gives.
    assert_eq!(
        line_ways(&map, "server"),
        [
            ("app/server/Server", "group", false),
            ("user", "server", true)
        ]
    );
    // The group opens on a level of boxes, whose person drew the admin's
    // line to none of them: it goes to the group's edge.
    assert_eq!(
        line_ways(&map, "group"),
        [("server", "store", false), ("admin", "group", false)]
    );
    assert_eq!(
        line_ways(&map, "store"),
        [("server", "app/store/Store", false)]
    );
    assert_eq!(open_drift(&map), Vec::<&str>::new());

    // Drawn on to the store, it goes on to the store's edge as the store
    // opens, and no longer to the group's.
    let map = generated(
        &people(to_group, around, r#"{ "from": "admin", "to": "store" }"#),
        &project,
    );
    assert_eq!(
        line_ways(&map, "group"),
        [("admin", "store", false), ("server", "store", false)]
    );
    assert_eq!(
        line_ways(&map, "store"),
        [
            ("server", "app/store/Store", false),
            ("admin", "store", false)
        ]
    );

    // A level that does not show the other end keeps no line to it.
    let map = generated(&people(to_group, SERVER_AT, ""), &project);
    assert_eq!(line_ways(&map, "group"), [("server", "store", false)]);

    // Nor does the top's center lose the top's line its level does not
    // carry on: it goes to the center's edge.
    let map = generated(&people("", around, ""), &project);
    assert_eq!(
        line_ways(&map, "app"),
        [("user", "server", true), ("admin", "app", false)]
    );
}

#[test]
fn a_box_opened_keeps_the_lines_of_its_code_its_types_do_not_draw() {
    // `a`'s free function calls `b`'s, which neither box's types do.
    let mut a = File::new("src/a.rs", "pub struct A {}\npub fn go() {\n    b::f();\n}");
    a.define(&rust("a/A#"), kind::STRUCT, 0, "A", (0, 0));
    a.define(&rust("a/go()."), kind::FUNCTION, 1, "go", (1, 3));
    a.refer(&rust("b/f()."), 2, "f");
    let mut b = File::new("src/b.rs", "pub struct B {}\npub fn f() {}");
    b.define(&rust("b/B#"), kind::STRUCT, 0, "B", (0, 0));
    b.define(&rust("b/f()."), kind::FUNCTION, 1, "f", (1, 1));
    let vocabulary = r#"{
      "nodes": { "app": { "name": "App" },
                 "a": { "name": "A", "claims": ["app/a"] },
                 "b": { "name": "B", "claims": ["app/b"] } },
      "top": { "center": "app" },
      "levels": { "app": { "box": [0, 0, 800, 500], "parts": { "a": [0, 0], "b": [300, 0] } } }
    }"#;
    let map = generated(vocabulary, &index(&[a, b]));
    assert_eq!(line_ways(&map, "app"), [("a", "b", false)]);
    // Inside either, the line goes to the edge of the box opened.
    assert_eq!(line_ways(&map, "a"), [("a", "b", false)]);
    assert_eq!(line_ways(&map, "b"), [("a", "b", false)]);
}

#[test]
fn a_box_that_stays_closed_shows_no_types_but_keeps_its_lines() {
    let vocabulary = APP.replace(
        r#""claims": ["app/store"] }"#,
        r#""claims": ["app/store"], "open": false }"#,
    );
    let map = generated(&vocabulary, &app());
    assert!(!map.types.contains_key("app/store/Store"));
    assert!(!map.levels.contains_key("store"));
    assert_eq!(map.edges[0].count, 2);
    assert_eq!(map.types["app/server/Server"].fields[0].to, ["store"]);
}

#[test]
fn the_same_index_and_vocabulary_give_the_same_bytes() {
    let project = app();
    let first = serde_json::to_vec(&generated(APP, &project)).unwrap();
    assert_eq!(
        serde_json::to_vec(&generated(APP, &project)).unwrap(),
        first
    );
    // Nor does the order the indexer wrote its files in matter.
    let (mut index, texts) = app();
    index.documents.reverse();
    assert_eq!(
        serde_json::to_vec(&generated(APP, &(index, texts))).unwrap(),
        first
    );
}

/// Boxes `a` to `d`, each a module with a function that calls the
/// others' as often as `calls` says, from the first to the second.
fn meeting(calls: &[(&str, &str, usize)]) -> (Index, BTreeMap<String, String>) {
    let files = ["a", "b", "c", "d", "x", "y", "z"].map(|module| {
        let mut file = File::new(&format!("src/{module}.rs"), "pub fn f() {");
        let mut called = Vec::new();
        for &(from, to, count) in calls.iter().filter(|(from, _, _)| *from == module) {
            for _ in 0..count {
                file.lines.push(format!("    {to}::f();"));
                called.push((file.lines.len() - 1, to, from));
            }
        }
        file.lines.push("}".to_owned());
        let end = file.lines.len() - 1;
        file.define(
            &rust(&format!("{module}/f().")),
            kind::FUNCTION,
            0,
            "f",
            (0, end),
        );
        for (line, to, _) in called {
            file.refer(&rust(&format!("{to}/f().")), line, "f");
        }
        file
    });
    index(&files)
}

/// A vocabulary of the boxes of [`meeting`], those in `parts` placed in
/// a level, and those in `at` around it.
fn boxes(parts: &str, at: &str) -> String {
    let nodes: Vec<String> = ["a", "b", "c", "d", "x", "y", "z"]
        .iter()
        .map(|id| format!(r#""{id}": {{ "name": "{id}", "claims": ["app/{id}"] }}"#))
        .collect();
    format!(
        r#"{{ "nodes": {{ "app": {{ "name": "App" }}, {} }},
             "top": {{ "center": "app" }},
             "levels": {{ "app": {{ "box": [0, 0, 1000, 600], "at": {{ {at} }}, "parts": {{ {parts} }} }} }} }}"#,
        nodes.join(", ")
    )
}

#[test]
fn a_box_keeps_its_two_heaviest_lines_and_the_rest_are_weak() {
    let project = meeting(&[
        ("a", "b", 5),
        ("b", "a", 2),
        ("a", "c", 4),
        ("b", "c", 3),
        ("b", "d", 2),
        ("a", "d", 1),
        ("c", "d", 1),
    ]);
    let vocabulary = boxes(
        r#""a": [0, 0], "b": [300, 0], "c": [0, 200], "d": [300, 200]"#,
        "",
    );
    let map = generated(&vocabulary, &project);
    let lines: Vec<(&str, &str, bool, bool)> = map.levels["app"]
        .lines
        .iter()
        .map(|line| (line.from.as_str(), line.to.as_str(), line.both, line.weak))
        .collect();
    assert_eq!(
        lines,
        [
            ("a", "b", true, false),
            ("a", "c", false, false),
            ("a", "d", false, false),
            ("b", "c", false, false),
            ("b", "d", false, false),
            ("c", "d", false, true),
        ]
    );
}

#[test]
fn boxes_swap_places_only_to_shorten_their_lines() {
    let project = meeting(&[("x", "y", 10)]);
    let around = r#""x": [0, 0]"#;
    let map = generated(&boxes(r#""z": [300, 0], "y": [600, 0]"#, around), &project);
    let parts = &map.levels["app"].parts;
    assert_eq!((parts["y"], parts["z"]), ([300, 0], [600, 0]));
    assert_eq!(map.levels["app"].lines[0].from, "x");
    let map = generated(&boxes(r#""y": [300, 0], "z": [600, 0]"#, around), &project);
    let parts = &map.levels["app"].parts;
    assert_eq!((parts["y"], parts["z"]), ([300, 0], [600, 0]));
}

/// A package of Go, `example.com/m/pkg`, whose files are in `pkg` in
/// the folder `within` of the index's, with a `/` after it, or empty: in
/// `a.go`, a type that names the type of `b.go`.
fn go_package(within: &str) -> (Index, BTreeMap<String, String>) {
    let go = |path: &str| format!("scip-go gomod example.com/m v1 `example.com/m/pkg`/{path}");
    let mut a = File::new(
        &format!("{within}pkg/a.go"),
        "package pkg\ntype A struct {\n\tb *B\n}",
    );
    a.define(&go("A#"), kind::STRUCT, 1, "A", (1, 3));
    a.define(&go("A#b."), kind::FIELD, 2, "b", (2, 2));
    a.refer(&go("B#"), 2, "B");
    let mut b = File::new(
        &format!("{within}pkg/b.go"),
        "package pkg\ntype B struct {\n\tn int\n}\nfunc (b *B) Up() {\n\tb.n++\n}",
    );
    b.define(&go("B#"), kind::STRUCT, 1, "B", (1, 3));
    b.define(&go("B#n."), kind::FIELD, 2, "n", (2, 2));
    b.define(&go("B#Up()."), kind::METHOD, 4, "Up", (4, 6));
    b.refer(&go("B#n."), 5, "n");
    index(&[a, b])
}

/// A vocabulary of [`go_package`] whose box `bee` owns `pkg/b.go`, and
/// `web` a folder the code does not have.
const GO_FILES: &str = r#"{
  "nodes": {
    "m": { "name": "M" },
    "pkg": { "name": "Package", "claims": ["example.com/m/pkg"] },
    "bee": { "name": "B's file", "files": ["pkg/b.go"] },
    "web": { "name": "Web", "files": ["web/"] }
  },
  "top": { "center": "m" },
  "levels": { "m": { "box": [0, 0, 600, 400], "parts": { "pkg": [0, 0], "bee": [300, 0], "web": [0, 200] } } }
}"#;

#[test]
fn a_box_can_own_files_where_a_package_is_the_smallest_module() {
    let map = generated(GO_FILES, &go_package(""));
    assert_eq!(map.types["example.com/m/pkg/A"].owner, "pkg");
    assert_eq!(map.types["example.com/m/pkg/B"].owner, "bee");
    assert!(map.types["example.com/m/pkg/B"].fields[0].uses[0].writes);
    assert_eq!(map.types["example.com/m/pkg/A"].fields[0].to, ["bee"]);
    assert_eq!(map.edges[0].count, 1);
    assert_eq!(
        (map.edges[0].a.as_str(), map.edges[0].b.as_str()),
        ("bee", "pkg")
    );
    assert_eq!(map.nodes["bee"].code.as_deref(), Some("pkg/b.go"));
    assert_eq!(map.files["pkg/a.go"], "pkg");
    assert_eq!(map.files["pkg/b.go"], "bee");
    assert_eq!(drift_ids(&map), ["claim-gone:web/"]);
}

#[test]
fn a_repositorys_map_claims_files_from_the_repositorys_folder() {
    // The project's index has the repository's files under its folder;
    // the repository's map names them from the repository's.
    let (index, texts) = go_package("directories/m/");
    let vocabulary = Vocabulary::parse(GO_FILES).unwrap();
    let map = generate(&index, &vocabulary, "directories/m/", &texts);
    assert_eq!(map.types["example.com/m/pkg/B"].owner, "bee");
    assert_eq!(
        map.types["example.com/m/pkg/B"].file,
        "directories/m/pkg/b.go"
    );
    // The files the map tells of are the project's, by their paths from
    // its folder, as what changed in them is; a claim, as it is written.
    assert_eq!(map.files["directories/m/pkg/a.go"], "pkg");
    assert_eq!(map.files["directories/m/pkg/b.go"], "bee");
    assert_eq!(map.nodes["bee"].code.as_deref(), Some("pkg/b.go"));
    assert_eq!(drift_ids(&map), ["claim-gone:web/"]);
    // From the project's folder, the claim is of no file there is.
    let map = generate(&index, &vocabulary, "", &texts);
    assert_eq!(map.types["example.com/m/pkg/B"].owner, "pkg");
    assert_eq!(drift_ids(&map), ["claim-gone:pkg/b.go", "claim-gone:web/"]);
    // Nor is a file of another folder claimed from the repository's.
    let (index, texts) = go_package("directories/other/");
    let map = generate(&index, &vocabulary, "directories/m/", &texts);
    assert_eq!(map.types["example.com/m/pkg/B"].owner, "pkg");
}

#[test]
fn the_projects_folder_gives_no_file_outside_it() {
    let root = crate::scratch::Folder::new();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn a() {}").unwrap();
    let elsewhere = crate::scratch::Folder::new();
    std::fs::write(elsewhere.join("secret"), "kept").unwrap();
    assert_eq!(
        Sources::read(&*root, "src/a.rs").as_deref(),
        Some("fn a() {}")
    );
    assert_eq!(
        Sources::read(&*root, "src/../src/a.rs").as_deref(),
        Some("fn a() {}")
    );
    assert_eq!(Sources::read(&*root, "../secret"), None);
    assert_eq!(
        Sources::read(&*root, &elsewhere.join("secret").to_string_lossy()),
        None
    );
    // A link in the project to a file out of it reads as no file; one to a
    // file in it, as that file.
    std::os::unix::fs::symlink(elsewhere.join("secret"), root.join("src/out")).unwrap();
    std::os::unix::fs::symlink(root.join("src/a.rs"), root.join("src/in")).unwrap();
    assert_eq!(Sources::read(&*root, "src/out"), None);
    assert_eq!(
        Sources::read(&*root, "src/in").as_deref(),
        Some("fn a() {}")
    );
}

#[test]
fn a_documentation_reads_to_its_first_sentence_and_a_signature_to_its_code() {
    let info = SymbolInformation {
        documentation: vec![
            "```rust\npub fn run(&mut self)\n```".to_owned(),
            "---\nRuns  the server: until it ends.\nThen stops.".to_owned(),
        ],
        ..SymbolInformation::default()
    };
    assert_eq!(doc_of(Some(&info)), "Runs the server:");
    assert_eq!(sig_of(Some(&info), &[], 0), "pub fn run(&mut self)");
    assert_eq!(sig_of(None, &["  fn  a() {".to_owned()], 0), "fn a() {");
    assert_eq!(doc_of(None), "");
}

#[test]
fn members_of_one_name_keep_their_own_uses() {
    let mut x = File::new(
        "src/x.rs",
        "pub struct X {}\n\
         impl Display for X {\n    \
             fn fmt(&self) {}\n\
         }\n\
         impl X {\n    \
             fn fmt(&self) {}\n\
         }\n\
         fn use_them(x: X) {\n    \
             Display::fmt(&x);\n    \
             x.fmt();\n\
         }",
    );
    x.define(&rust("x/X#"), kind::STRUCT, 0, "X", (0, 0));
    x.define(
        &rust("x/impl#[X][Display]fmt()."),
        kind::METHOD,
        2,
        "fmt",
        (2, 2),
    );
    x.define(&rust("x/impl#[X]fmt()."), kind::METHOD, 5, "fmt", (5, 5));
    x.define(
        &rust("x/use_them()."),
        kind::FUNCTION,
        7,
        "use_them",
        (7, 10),
    );
    x.refer(&rust("x/impl#[X][Display]fmt()."), 8, "fmt");
    x.refer(&rust("x/impl#[X]fmt()."), 9, "fmt");
    // Overloads, as scip-java tells them apart.
    let java = |path: &str| format!("scip-java maven app 1.0 shop/{path}");
    let mut t = File::new(
        "shop/T.java",
        "class T {\n  void m() {}\n  void m(int a) {}\n  void call() {\n    m();\n    m(1);\n  }\n}",
    );
    t.define(&java("T#"), kind::CLASS, 0, "T", (0, 7));
    t.define(&java("T#m()."), kind::METHOD, 1, "m", (1, 1));
    t.define(&java("T#m(+1)."), kind::METHOD, 2, "m", (2, 2));
    t.define(&java("T#call()."), kind::METHOD, 3, "call", (3, 6));
    t.refer(&java("T#m()."), 4, "m");
    t.refer(&java("T#m(+1)."), 5, "m");
    let vocabulary = r#"{
      "nodes": { "app": { "name": "App" }, "x": { "name": "X", "claims": ["app/x"] },
                 "shop": { "name": "Shop", "claims": ["app/shop"] } },
      "top": { "center": "app" }
    }"#;
    let map = generated(vocabulary, &index(&[x, t]));
    let uses = |ty: &str| -> Vec<(String, Vec<usize>)> {
        map.types[ty]
            .functions
            .iter()
            .map(|function| {
                (
                    function.key.clone(),
                    function.uses.iter().map(|used| used.line).collect(),
                )
            })
            .collect()
    };
    assert_eq!(
        uses("app/x/X"),
        [
            ("app/x/X::fmt#Display".to_owned(), vec![9]),
            ("app/x/X::fmt".to_owned(), vec![10]),
        ]
    );
    assert_eq!(
        uses("app/shop/T"),
        [
            ("app/shop/T::m".to_owned(), vec![5]),
            ("app/shop/T::m(+1)".to_owned(), vec![6]),
            ("app/shop/T::call".to_owned(), vec![]),
        ]
    );
}

#[test]
fn each_indexer_of_a_mixed_index_marks_imports_and_writes_its_own_way() {
    let (mut project, mut texts) = app();
    // A document of another indexer, which marks imports and writes by
    // their roles, as the Rust indexer does not.
    let ts = |path: &str| format!("scip-typescript npm web 1.0.0 `main.ts`/{path}");
    let mut main = File::new(
        "web/main.ts",
        "import { go } from './go';\nlet n = 0;\nn = go();",
    );
    main.refer_as(&ts("go()."), 0, "go", role::IMPORT);
    main.define(&ts("n."), 0, 1, "n", (1, 1));
    main.refer_as(&ts("n."), 2, "n", role::WRITE_ACCESS);
    let (more, more_texts) = index(&[main]);
    project.documents.extend(more.documents);
    texts.extend(more_texts);
    let map = generated(APP, &(project, texts));
    // The server's `use` of the store, above its first definition, is
    // still an import, not a reference.
    assert_eq!(map.edges[0].count, 2);
    assert!(map.types["app/server/Server"].fields[1].uses[0].writes);
}

#[test]
fn a_broken_range_in_the_index_neither_hangs_nor_panics() {
    let mut a = File::new(
        "src/a.rs",
        "pub struct A {\n    b: B,\n}\npub struct B {}\nfn f() {}",
    );
    a.define(&rust("a/A#"), kind::STRUCT, 0, "A", (0, 2));
    // A field that runs to the end of the numbers, and a function that
    // ends before it starts.
    a.define(
        &rust("a/A#b."),
        kind::FIELD,
        1,
        "b",
        (1, usize::try_from(i32::MAX).unwrap()),
    );
    a.refer(&rust("a/B#"), 1, "B");
    a.define(&rust("a/B#"), kind::STRUCT, 3, "B", (3, 3));
    a.define(&rust("a/f()."), kind::FUNCTION, 4, "f", (4, 1));
    let vocabulary = r#"{ "nodes": { "app": { "name": "App" }, "a": { "name": "A", "claims": ["app/a"] } },
                          "top": { "center": "app" } }"#;
    let map = generated(vocabulary, &index(&[a]));
    assert_eq!(map.types["app/a/A"].fields[0].to, ["app/a/B"]);
}

#[test]
fn documents_of_one_path_give_the_same_map_in_either_order() {
    let file = |name: &str| {
        let mut file = File::new("src/a.rs", "pub struct A {}\npub struct B {}");
        let line = usize::from(name == "B");
        file.define(
            &rust(&format!("a/{name}#")),
            kind::STRUCT,
            line,
            name,
            (line, line),
        );
        file
    };
    let vocabulary = r#"{ "nodes": { "app": { "name": "App" }, "a": { "name": "A", "claims": ["app/a"] } },
                          "top": { "center": "app" } }"#;
    let one = serde_json::to_vec(&generated(vocabulary, &index(&[file("A"), file("B")]))).unwrap();
    let other =
        serde_json::to_vec(&generated(vocabulary, &index(&[file("B"), file("A")]))).unwrap();
    assert_eq!(one, other);
}

#[test]
fn types_inside_a_box_sit_apart_and_toward_the_boxes_they_name() {
    let mut mid = File::new(
        "src/mid.rs",
        "pub struct West {\n    w: W,\n}\npub struct East {\n    e: E,\n}\n\
         pub struct A {}\npub struct B {}\npub struct C {}",
    );
    mid.define(&rust("mid/West#"), kind::STRUCT, 0, "West", (0, 2));
    mid.define(&rust("mid/West#w."), kind::FIELD, 1, "w", (1, 1));
    mid.refer(&rust("west/W#"), 1, "W");
    mid.define(&rust("mid/East#"), kind::STRUCT, 3, "East", (3, 5));
    mid.define(&rust("mid/East#e."), kind::FIELD, 4, "e", (4, 4));
    mid.refer(&rust("east/E#"), 4, "E");
    for (line, name) in [(6, "A"), (7, "B"), (8, "C")] {
        mid.define(
            &rust(&format!("mid/{name}#")),
            kind::STRUCT,
            line,
            name,
            (line, line),
        );
    }
    let mut west = File::new("src/west.rs", "pub struct W {}");
    west.define(&rust("west/W#"), kind::STRUCT, 0, "W", (0, 0));
    let mut east = File::new("src/east.rs", "pub struct E {}");
    east.define(&rust("east/E#"), kind::STRUCT, 0, "E", (0, 0));
    let project = index(&[mid, west, east]);
    let vocabulary = r#"{
      "nodes": { "app": { "name": "App" },
                 "west": { "name": "West", "claims": ["app/west"] },
                 "mid": { "name": "Mid", "claims": ["app/mid"] },
                 "east": { "name": "East", "claims": ["app/east"] } },
      "top": { "center": "app" },
      "levels": { "app": { "box": [-900, -300, 1800, 600],
                           "parts": { "west": [-800, 0], "mid": [0, 0], "east": [800, 0] } } }
    }"#;
    let map = generated(vocabulary, &project);
    let level = &map.levels["mid"];
    let [width, height] = level.size.unwrap();
    let parts: Vec<(&str, [i32; 2])> = level
        .parts
        .iter()
        .map(|(id, &at)| (id.as_str(), at))
        .collect();
    assert_eq!(parts.len(), 5);
    for (index, &(_, [x, y])) in parts.iter().enumerate() {
        // Each card, as wide and high as a card is at least, is inside the
        // level, and clear of every other.
        assert!(x >= 0 && y >= 0 && x + 216 <= width && y + 56 <= height);
        for &(_, [other_x, other_y]) in &parts[index + 1..] {
            assert!((x - other_x).abs() >= 216 || (y - other_y).abs() >= 56);
        }
    }
    let x = |id: &str| level.parts[id][0];
    let all: Vec<i32> = parts.iter().map(|&(_, [x, _])| x).collect();
    assert_eq!(x("app/mid/West"), *all.iter().min().unwrap());
    assert!(x("app/mid/West") < x("app/mid/East"));
    assert_eq!(generated(vocabulary, &project), map);
}

/// A box `mid` of types: `Holder`, whose field names `Kept`, then, in
/// the file, `loose` named nothing and naming nothing.
fn holder_and(loose: &[&str]) -> (Index, BTreeMap<String, String>) {
    let text: String =
        std::iter::once("pub struct Holder {\n    kept: Kept,\n}\npub struct Kept {}\n".to_owned())
            .chain(loose.iter().map(|name| format!("pub struct {name} {{}}\n")))
            .collect();
    let mut mid = File::new("src/mid.rs", &text);
    mid.define(&rust("mid/Holder#"), kind::STRUCT, 0, "Holder", (0, 2));
    mid.define(&rust("mid/Holder#kept."), kind::FIELD, 1, "kept", (1, 1));
    mid.refer(&rust("mid/Kept#"), 1, "Kept");
    mid.define(&rust("mid/Kept#"), kind::STRUCT, 3, "Kept", (3, 3));
    for (index, name) in loose.iter().enumerate() {
        let line = 4 + index;
        mid.define(
            &rust(&format!("mid/{name}#")),
            kind::STRUCT,
            line,
            name,
            (line, line),
        );
    }
    index(&[mid])
}

const MID: &str = r#"{ "nodes": { "app": { "name": "App" }, "mid": { "name": "Mid", "claims": ["app/mid"] } },
                       "top": { "center": "app" },
                       "levels": { "app": { "box": [0, 0, 600, 400], "parts": { "mid": [0, 0] } } } }"#;

#[test]
fn types_wired_to_nothing_follow_the_others_apart_in_name_order() {
    let map = generated(
        MID,
        &holder_and(&["Zebra", "Mango", "Apple", "Lime", "Fig"]),
    );
    let level = &map.levels["mid"];
    let wired: BTreeSet<&str> = level
        .lines
        .iter()
        .flat_map(|line| [line.from.as_str(), line.to.as_str()])
        .collect();
    assert_eq!(wired, BTreeSet::from(["app/mid/Holder", "app/mid/Kept"]));
    let mut loose: Vec<(i32, i32, &str)> = level
        .parts
        .iter()
        .filter(|(id, _)| !wired.contains(id.as_str()))
        .map(|(id, &[x, y])| (y, x, map.types[id].name.as_str()))
        .collect();
    loose.sort_unstable();
    // Read in rows, left to right, they are in name order, not the file's.
    let names: Vec<&str> = loose.iter().map(|&(_, _, name)| name).collect();
    assert_eq!(names, ["Apple", "Fig", "Lime", "Mango", "Zebra"]);
    // They sit below those wired, further from them than their own rows
    // are from each other.
    let wired_bottom = wired.iter().map(|id| level.parts[*id][1]).max().unwrap();
    let rows: BTreeSet<i32> = loose.iter().map(|&(y, _, _)| y).collect();
    let rows: Vec<i32> = rows.into_iter().collect();
    assert!(rows.len() >= 2);
    assert!(rows[0] - wired_bottom > rows[1] - rows[0]);
    assert_eq!(
        generated(
            MID,
            &holder_and(&["Zebra", "Mango", "Apple", "Lime", "Fig"])
        ),
        map
    );
}

#[test]
fn a_type_wired_to_nothing_leaves_those_wired_where_they_were() {
    use super::layout::{Card, types};
    let card = |id: &'static str| Card {
        id,
        name: id,
        code: "struct",
    };
    let wired = ["A", "B", "C", "D", "E", "F", "G"];
    let lines: BTreeSet<(String, String)> = [
        ("A", "B"),
        ("B", "C"),
        ("C", "west"),
        ("D", "east"),
        ("E", "D"),
        ("F", "G"),
        ("G", "west"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect();
    let around = BTreeMap::from([("west", [-800, 0]), ("mid", [0, 0]), ("east", [800, 0])]);
    let cards: Vec<Card> = wired.into_iter().map(card).collect();
    let (_, alone) = types(&cards, &lines, &around, [0, 0]);
    // However many come, wherever their names sort, until the level is
    // as wide as it grows.
    let mut cards = cards;
    for name in [
        "0", "Ca", "Zz", "1", "Ab", "Y", "2", "Fa", "X", "3", "W", "4",
    ] {
        cards.push(card(name));
        let (_, with) = types(&cards, &lines, &around, [0, 0]);
        for id in wired {
            assert_eq!(with[id], alone[id], "{id} with {name}");
        }
    }
    // The cards' order does not matter.
    let forth = types(&cards, &lines, &around, [0, 0]);
    cards.reverse();
    assert_eq!(types(&cards, &lines, &around, [0, 0]), forth);
}

#[test]
fn a_map_generates_from_what_real_indexers_wrote() {
    use crate::scip::fixtures;
    let sources = |(path, text): (&str, &str)| BTreeMap::from([(path.to_owned(), text.to_owned())]);
    let vocabulary = |claim: &str| {
        format!(
            r#"{{ "nodes": {{ "shop": {{ "name": "Shop" }}, "cart": {{ "name": "Cart", "claims": ["{claim}"] }} }},
                 "top": {{ "center": "shop" }},
                 "levels": {{ "shop": {{ "box": [0, 0, 400, 300], "parts": {{ "cart": [0, 0] }} }} }} }}"#
        )
    };
    let go = generated(
        &vocabulary("example.com/shop"),
        &(
            Index::decode(fixtures::GO_INDEX).unwrap(),
            sources(fixtures::GO_FILE),
        ),
    );
    let rust = generated(
        &vocabulary("shop"),
        &(
            Index::decode(fixtures::RUST_INDEX).unwrap(),
            sources(fixtures::RUST_FILE),
        ),
    );
    for (map, cart, add) in [
        (&go, "example.com/shop/Cart", "example.com/shop/Cart::Add"),
        (&rust, "shop/Cart", "shop/Cart::add"),
    ] {
        let ty = &map.types[cart];
        assert_eq!(ty.owner, "cart");
        assert_eq!(ty.kind, "struct");
        let fields: Vec<&str> = ty.fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(fields, ["items", "count"]);
        assert_eq!(ty.functions[0].key, add);
        // Both fields are written in the function, `items` once of the two
        // times it is named.
        let calls: Vec<(&str, usize, bool)> = ty.functions[0]
            .calls
            .iter()
            .map(|call| (call.name.as_str(), call.count, call.writes))
            .collect();
        assert!(calls.contains(&("Cart.count", 1, true)), "{calls:?}");
        assert!(ty.fields[1].uses[0].writes);
        assert!(map.levels["cart"].parts.contains_key(cart));
    }
    let items: Vec<bool> = go.types["example.com/shop/Cart"].fields[0]
        .uses
        .iter()
        .map(|used| used.writes)
        .collect();
    assert_eq!(items, [true, false]);
    assert_eq!(
        rust.types["shop/Cart"].functions[0].doc,
        "Puts an item in the cart."
    );
}

mod rules;
