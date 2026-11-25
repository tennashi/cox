//! Lines whose end at a box is a rule, over small indexes of the shapes
//! rust-analyzer, scip-go and scip-typescript write: the same rules pick
//! the same kinds of types from each.

use super::*;
use crate::scip::Relationship;

/// Says of the symbol `file` defined last that it implements `target`,
/// as the relationships of scip-go's and scip-typescript's indexes do.
fn implementing(file: &mut File, target: &str) {
    file.symbols
        .last_mut()
        .unwrap()
        .relationships
        .push(Relationship {
            symbol: target.to_owned(),
            is_implementation: true,
        });
}

/// The lines of `level`, as `from`, `to` and whether weak.
fn reached<'m>(map: &'m Generated, level: &str) -> Vec<(&'m str, &'m str, bool)> {
    map.levels[level]
        .lines
        .iter()
        .map(|line| (line.from.as_str(), line.to.as_str(), line.weak))
        .collect()
}

/// A symbol of gpui's, a dependency, as rust-analyzer names its library.
fn gpui(path: &str) -> String {
    format!("rust-analyzer cargo gpui 0.2.2 lib/{path}")
}

/// A window of three views, each an `impl Render` of gpui's, named the
/// more often the earlier, and a type with a gpui `Window` for a field;
/// and the code that starts it, which names the views.
fn views() -> (Index, BTreeMap<String, String>) {
    let mut ui = File::new(
        "src/ui.rs",
        "pub struct Workspace {}\n\
         impl Render for Workspace {\n    fn render(&mut self, window: &mut Window) {}\n}\n\
         pub struct Card {}\n\
         impl Render for Card {\n    fn render(&mut self) {}\n}\n\
         pub struct Tip {}\n\
         impl Render for Tip {\n    fn render(&mut self) {}\n}\n\
         pub struct Plain {\n    window: Window,\n}",
    );
    for (line, name) in [(0, "Workspace"), (4, "Card"), (8, "Tip")] {
        ui.define(
            &rust(&format!("ui/{name}#")),
            kind::STRUCT,
            line,
            name,
            (line, line),
        );
        ui.refer(&gpui("element/Render#"), line + 1, "Render");
        ui.define(
            &rust(&format!("ui/impl#[{name}][Render]render().")),
            kind::METHOD,
            line + 2,
            "render",
            (line + 2, line + 2),
        );
    }
    ui.refer(&gpui("window/Window#"), 2, "Window");
    ui.define(&rust("ui/Plain#"), kind::STRUCT, 12, "Plain", (12, 14));
    ui.define(
        &rust("ui/Plain#window."),
        kind::FIELD,
        13,
        "window",
        (13, 13),
    );
    ui.refer(&gpui("window/Window#"), 13, "Window");
    let mut start = File::new(
        "src/start.rs",
        "pub fn main() {\n    Workspace {};\n    Workspace {};\n    Card {};\n}",
    );
    start.define(&rust("start/main()."), kind::FUNCTION, 0, "main", (0, 4));
    start.refer(&rust("ui/Workspace#"), 1, "Workspace");
    start.refer(&rust("ui/Workspace#"), 2, "Workspace");
    start.refer(&rust("ui/Card#"), 3, "Card");
    index(&[ui, start])
}

/// A vocabulary of [`views`] whose user's line goes to the box `ui` as
/// `rule` picks inside it.
fn to_views(rule: &str) -> String {
    format!(
        r#"{{
          "nodes": {{
            "app": {{ "name": "App" }},
            "user": {{ "name": "User", "kind": "outside" }},
            "ui": {{ "name": "UI", "claims": ["app/ui"] }},
            "start": {{ "name": "Start", "claims": ["app/start"] }}
          }},
          "top": {{ "center": "app", "left": ["user"], "lines": [{{ "from": "user", "to": "app", "both": true }}] }},
          "levels": {{
            "app": {{
              "box": [0, 0, 800, 500],
              "outer": {{ "user": ["left", 0] }},
              "parts": {{ "ui": [0, 0], "start": [300, 0] }},
              "lines": [{{ "from": "user", "to": {{ "box": "ui", {rule} }}, "both": true }}]
            }}
          }}
        }}"#
    )
}

#[test]
fn a_rule_picks_the_types_rust_analyzer_tells_implement_a_trait() {
    let project = views();
    let map = generated(&to_views(r#""implements": "Render""#), &project);
    // The line to the box stays where the person drew it.
    assert_eq!(
        line_ways(&map, "app"),
        [("user", "ui", true), ("start", "ui", false)]
    );
    // Inside it, it goes on to each view, the two named most drawn whole,
    // and no longer to the box's edge.
    let picked: Vec<_> = reached(&map, "ui")
        .into_iter()
        .filter(|&(from, _, _)| from == "user")
        .collect();
    assert_eq!(
        picked,
        [
            ("user", "app/ui/Workspace", false),
            ("user", "app/ui/Card", false),
            ("user", "app/ui/Tip", true)
        ]
    );
    assert!(
        map.levels["ui"]
            .lines
            .iter()
            .all(|line| line.from != "user" || line.both)
    );
    assert_eq!(open_drift(&map), Vec::<&str>::new());

    // rust-analyzer names the trait of an `impl` block by its name
    // alone, which a name's last part is matched against.
    let qualified = generated(
        &to_views(r#""implements": "gpui/element/Render""#),
        &project,
    );
    assert_eq!(reached(&qualified, "ui"), reached(&map, "ui"));

    // What the types' code uses is told by its references, the type that
    // has one for a field too.
    let map = generated(&to_views(r#""uses": "window/Window""#), &project);
    let users: Vec<_> = reached(&map, "ui")
        .into_iter()
        .filter(|&(from, _, _)| from == "user")
        .collect();
    assert_eq!(
        users,
        [
            ("user", "app/ui/Plain", false),
            ("user", "app/ui/Workspace", false)
        ]
    );

    // A rule that picks nothing is drift, and the line goes to the box's
    // edge, as it did before the rule.
    let map = generated(&to_views(r#""implements": "Drop""#), &project);
    assert_eq!(open_drift(&map), ["rule-empty:user|ui"]);
    let drift = &map.drift[0];
    assert_eq!(drift.kind, DriftKind::RuleEmpty);
    assert_eq!(drift.nodes, ["ui"]);
    assert_eq!(
        drift.text,
        "No type inside UI implements Drop, for the line from User to UI to go on to"
    );
    assert!(reached(&map, "ui").contains(&("user", "ui", false)));
}

/// `example.com/m`, as scip-go indexes it: a package `tui` with an
/// interface two of its types satisfy, which a type of the package
/// `core` keeps in a field.
fn terminal() -> (Index, BTreeMap<String, String>) {
    let go = |package: &str, path: &str| {
        format!("scip-go gomod example.com/m v1 `example.com/m/{package}`/{path}")
    };
    let mut tui = File::new(
        "tui/tui.go",
        "package tui\ntype Renderer interface {\n\tDraw()\n}\n\
         type Light struct{}\nfunc (l *Light) Draw() {}\n\
         type Full struct{}\nfunc (f *Full) Draw() {}\n\
         type Color int",
    );
    tui.define(
        &go("tui", "Renderer#"),
        kind::INTERFACE,
        1,
        "Renderer",
        (1, 3),
    );
    for (line, name) in [(4, "Light"), (6, "Full")] {
        tui.define(
            &go("tui", &format!("{name}#")),
            kind::STRUCT,
            line,
            name,
            (line, line),
        );
        implementing(&mut tui, &go("tui", "Renderer#"));
        tui.define(
            &go("tui", &format!("{name}#Draw().")),
            kind::METHOD,
            line + 1,
            "Draw",
            (line + 1, line + 1),
        );
        implementing(&mut tui, &go("tui", "Renderer#Draw."));
    }
    tui.define(&go("tui", "Color#"), kind::TYPE, 8, "Color", (8, 8));
    let mut core = File::new(
        "core/term.go",
        "package core\ntype Terminal struct {\n\tr tui.Renderer\n}\ntype Pool struct{}",
    );
    core.define(
        &go("core", "Terminal#"),
        kind::STRUCT,
        1,
        "Terminal",
        (1, 3),
    );
    core.define(&go("core", "Terminal#r."), kind::FIELD, 2, "r", (2, 2));
    core.refer(&go("tui", "Renderer#"), 2, "Renderer");
    core.define(&go("core", "Pool#"), kind::STRUCT, 4, "Pool", (4, 4));
    index(&[tui, core])
}

/// A vocabulary of [`terminal`], whose box `app` opens on a level of
/// the two packages' boxes; the user's line goes to it as `rule` picks.
fn to_terminal(rule: &str) -> String {
    format!(
        r#"{{
          "nodes": {{
            "m": {{ "name": "M" }},
            "user": {{ "name": "User", "kind": "outside" }},
            "app": {{ "name": "App" }},
            "tui": {{ "name": "TUI", "claims": ["example.com/m/tui"] }},
            "core": {{ "name": "Core", "claims": ["example.com/m/core"] }}
          }},
          "top": {{ "center": "m", "left": ["user"] }},
          "levels": {{
            "m": {{
              "box": [0, 0, 900, 600],
              "outer": {{ "user": ["left", 0] }},
              "parts": {{ "app": [0, 0] }},
              "lines": [{{ "from": "user", "to": {{ "box": "app", {rule} }} }}]
            }},
            "app": {{
              "box": [0, 0, 900, 600],
              "at": {{ "user": [-300, 0] }},
              "parts": {{ "tui": [0, 0], "core": [300, 0] }},
              "lines": [{{ "from": "user", "to": {{ "box": "core", "uses": "tui.Renderer" }} }}]
            }}
          }}
        }}"#
    )
}

#[test]
fn a_rule_picks_the_types_scip_go_tells_satisfy_an_interface() {
    let project = terminal();
    let map = generated(&to_terminal(r#""implements": "tui/Renderer""#), &project);
    // In the level of boxes, the line goes on to the box that holds the
    // types the rule picks, beside the one the person drew; inside that
    // box, to the types.
    assert_eq!(
        reached(&map, "app"),
        [
            ("user", "core", false),
            ("core", "tui", false),
            ("user", "tui", false)
        ]
    );
    assert_eq!(
        reached(&map, "tui"),
        [
            ("core", "example.com/m/tui/Renderer", false),
            ("user", "example.com/m/tui/Full", false),
            ("user", "example.com/m/tui/Light", false)
        ]
    );
    // They are placed toward the user, on the left of the box; `Color`,
    // wired to nothing, below them all.
    let at = |name: &str| map.levels["tui"].parts[&format!("example.com/m/tui/{name}")];
    assert!(at("Full")[0] < at("Renderer")[0] && at("Light")[0] < at("Renderer")[0]);
    assert!(
        ["Full", "Light", "Renderer"]
            .iter()
            .all(|name| at(name)[1] < at("Color")[1])
    );
    // The type whose field names the interface uses it.
    assert!(reached(&map, "core").contains(&("user", "example.com/m/core/Terminal", false)));
    assert_eq!(open_drift(&map), Vec::<&str>::new());

    // A name is matched by as much of its module as is written: another
    // package's interface of that name is none of these.
    let map = generated(&to_terminal(r#""implements": "other/Renderer""#), &project);
    assert_eq!(open_drift(&map), ["rule-empty:user|app"]);
    assert_eq!(
        reached(&map, "app"),
        [("user", "core", false), ("core", "tui", false)]
    );
}

#[test]
fn a_rule_from_a_box_drawn_only_by_its_edge_wires_the_types_it_picks() {
    // The user is outer to the level the box of types sits in: a line to
    // it has no place to pull the types toward, but wires them still.
    let vocabulary = r#"{
      "nodes": {
        "m": { "name": "M" },
        "user": { "name": "User", "kind": "outside" },
        "tui": { "name": "TUI", "claims": ["example.com/m/tui"] },
        "core": { "name": "Core", "claims": ["example.com/m/core"] }
      },
      "top": { "center": "m", "left": ["user"] },
      "levels": {
        "m": {
          "box": [0, 0, 900, 600],
          "outer": { "user": ["left", 0] },
          "parts": { "tui": [0, 0], "core": [300, 0] },
          "lines": [{ "from": "user", "to": { "box": "tui", "implements": "tui/Renderer" } }]
        }
      }
    }"#;
    let map = generated(vocabulary, &terminal());
    let level = &map.levels["tui"];
    let type_of = |name: &str| format!("example.com/m/tui/{name}");
    for name in ["Full", "Light"] {
        assert!(reached(&map, "tui").contains(&("user", &type_of(name), false)));
    }
    let y = |name: &str| level.parts[&type_of(name)][1];
    assert!(
        ["Full", "Light", "Renderer"]
            .iter()
            .all(|name| y(name) < y("Color"))
    );
}

#[test]
fn a_rule_picks_the_classes_scip_typescript_tells_extend_one_in_turn() {
    let ts =
        |file: &str, name: &str| format!("scip-typescript npm hono 4.0.0 src/`{file}`/{name}#");
    let mut base = File::new(
        "src/hono-base.ts",
        "class Hono {}\nexport { Hono as HonoBase }",
    );
    base.define(&ts("hono-base.ts", "Hono"), kind::CLASS, 0, "Hono", (0, 0));
    let mut hono = File::new("src/hono.ts", "export class Hono extends HonoBase {}");
    hono.define(&ts("hono.ts", "Hono"), kind::CLASS, 0, "Hono", (0, 0));
    implementing(&mut hono, &ts("hono-base.ts", "Hono"));
    hono.refer(&ts("hono-base.ts", "Hono"), 0, "HonoBase");
    let mut mine = File::new(
        "src/mine.ts",
        "export class Mine extends Hono {}\nexport class Other {}",
    );
    mine.define(&ts("mine.ts", "Mine"), kind::CLASS, 0, "Mine", (0, 0));
    implementing(&mut mine, &ts("hono.ts", "Hono"));
    mine.refer(&ts("hono.ts", "Hono"), 0, "Hono {}");
    mine.define(&ts("mine.ts", "Other"), kind::CLASS, 1, "Other", (1, 1));
    let project = index(&[base, hono, mine]);
    let vocabulary = |rule: &str| {
        format!(
            r#"{{
              "nodes": {{ "web": {{ "name": "Web" }},
                         "dev": {{ "name": "Developer", "kind": "outside" }},
                         "core": {{ "name": "Core", "claims": ["hono/src"] }} }},
              "top": {{ "center": "web", "left": ["dev"] }},
              "levels": {{ "web": {{ "box": [0, 0, 600, 400], "outer": {{ "dev": ["left", 0] }},
                                    "parts": {{ "core": [0, 0] }},
                                    "lines": [{{ "from": {{ "box": "core", {rule} }}, "to": "dev" }}] }} }}
            }}"#
        )
    };
    // The class declared in `hono-base.ts`, what extends it, and what
    // extends that, the line going from them, as its end at the box does.
    let map = generated(
        &vocabulary(r#""implements": "hono-base.ts/Hono""#),
        &project,
    );
    assert_eq!(
        reached(&map, "core"),
        [
            ("hono/src/hono.ts/Hono", "dev", false),
            ("hono/src/mine.ts/Mine", "dev", false)
        ]
    );
    assert_eq!(open_drift(&map), Vec::<&str>::new());
    // A name it is imported or exported as is no name of its own.
    let map = generated(&vocabulary(r#""implements": "HonoBase""#), &project);
    assert_eq!(open_drift(&map), ["rule-empty:core|dev"]);
}
