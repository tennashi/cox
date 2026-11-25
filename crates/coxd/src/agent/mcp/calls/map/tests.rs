//! `show_map`, on maps drawn from the index rust-analyzer wrote of a
//! small crate `shop` (see `cox_editor::scip`), as coxd would tell them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cox_editor::map::Named;
use cox_editor::map::generate::generate;
use cox_editor::map::generated::{Drift, DriftKind, Use};
use cox_editor::map::vocabulary::Vocabulary;
use cox_editor::scip::Index;
use prost::Message as _;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::*;

/// The index rust-analyzer wrote of `src/lib.rs` of the crate `shop`, and
/// that file.
const FIXTURE: &[u8] =
    include_bytes!("../../../../../../cox-editor/src/scip/fixtures/cart-rust.scip");
const LIB: &str = include_str!("../../../../../../cox-editor/src/scip/fixtures/cart-rust.txt");

/// A map whose box `cart` owns the crate's code, and whose box `gone`
/// claims code the crate does not have.
const VOCABULARY: &str = r#"{
  "says": "The shop",
  "nodes": {
    "shop": { "name": "Shop" },
    "buyer": { "name": "Buyer", "icon": "person", "kind": "outside" },
    "cart": { "name": "Cart", "claims": ["shop"] },
    "gone": { "name": "Gone", "claims": ["shop/gone"] }
  },
  "top": { "center": "shop", "left": ["buyer"], "lines": [{ "from": "buyer", "to": "shop", "both": true }] },
  "levels": {
    "shop": {
      "box": [0, 0, 400, 300],
      "outer": { "buyer": ["left", 0] },
      "parts": { "cart": [20, 20], "gone": [200, 20] },
      "lines": [{ "from": "buyer", "to": "cart" }]
    }
  }
}"#;

const ROOT: &str = "/work/shop";

/// The map `shop` of a project that has it and the maps `others`, drawn
/// from the crate's index, as coxd tells it when it is ready.
fn told(others: &[&str]) -> Told {
    let index = Index::decode(FIXTURE).unwrap();
    let vocabulary = Vocabulary::parse(VOCABULARY).unwrap();
    let sources = BTreeMap::from([("src/lib.rs".to_owned(), LIB.to_owned())]);
    let map = generate(&index, &vocabulary, &sources);
    let mut maps: Vec<Named> = others
        .iter()
        .map(|name| Named {
            name: (*name).to_owned(),
            file: format!("maps/{name}.json"),
            says: format!("The {name}"),
        })
        .collect();
    maps.insert(
        0,
        Named {
            name: "shop".to_owned(),
            file: "maps/shop.json".to_owned(),
            says: "The shop".to_owned(),
        },
    );
    Told {
        project: "shop".to_owned(),
        root: PathBuf::from(ROOT),
        maps,
        name: Some("shop".to_owned()),
        file: Some(Path::new(ROOT).join("maps/shop.json")),
        state: State::Ready,
        problems: Vec::new(),
        indexes: vec![Indexed {
            directory: String::new(),
            language: Some("Rust".to_owned()),
            indexer: Some("rust-analyzer".to_owned()),
            commit: Some("0123456789abcdef".to_owned()),
            drawn: Some("0123456789abcdef".to_owned()),
            note: None,
            state: IndexState::Ready,
        }],
        map: Some(map),
        changed: BTreeMap::new(),
    }
}

/// What `show_map` answers when called with `arguments`, and coxd tells
/// it `told`, with what the stage was asked.
async fn shown(arguments: Value, told: Told) -> (Result<String, String>, Vec<ToolRequest>) {
    let (requests, mut asked) = unbounded_channel();
    let (chats, mut heard) = unbounded_channel();
    let coxd = tokio::spawn(async move {
        let mut folders = Vec::new();
        while let Some(request) = heard.recv().await {
            if let ChatRequest::Map {
                folder,
                name,
                answer,
            } = request
            {
                folders.push((folder, name));
                let _ = answer.send(Ok(Box::new(told.clone())));
            }
        }
        folders
    });
    let answered = show_map(&arguments, Path::new(ROOT), &requests, &chats).await;
    drop(chats);
    let folders = coxd.await.unwrap();
    assert!(
        folders.iter().all(|(folder, _)| folder == Path::new(ROOT)),
        "the map is asked of the chat's folder: {folders:?}"
    );
    (answered, asked_of(&mut asked))
}

fn asked_of(asked: &mut UnboundedReceiver<ToolRequest>) -> Vec<ToolRequest> {
    let mut all = Vec::new();
    while let Ok(request) = asked.try_recv() {
        all.push(request);
    }
    all
}

/// Where the stage was asked to open the map, if it was.
fn opened_at(asked: &[ToolRequest]) -> Option<(Option<String>, Vec<String>)> {
    match asked {
        [ToolRequest::ShowMap { map, at }] => Some((map.clone(), at.clone())),
        _ => None,
    }
}

#[tokio::test]
async fn shows_the_map_at_its_top_and_tells_what_is_there_and_its_drift() {
    let (answered, asked) = shown(json!({}), told(&[])).await;

    assert_eq!(opened_at(&asked), Some((None, Vec::new())));
    assert_eq!(
        answered.unwrap(),
        "The map of the project is on the stage, open at its top, for the human to see where \
         things are.\n\
         It shows: The shop\n\
         At the top:\n\
         `buyer` Buyer\n\
         `shop` Shop; opens\n\
         Lines, with how often the code of their ends meets: `buyer` ↔ `shop`\n\
         Drift, where the vocabulary, maps/shop.json, and the code disagree, 1: talk each \
         through with the human, then fix the vocabulary, change the code, or agree to leave \
         it, as {\"id\", \"why\"} in the vocabulary's `agreed`.\n\
         - `claim-gone:shop/gone`: Gone claims shop/gone, which the code no longer has"
    );
}

#[tokio::test]
async fn opens_inside_a_box_with_its_parts_lines_and_drift() {
    let mut told = told(&[]);
    told.changed.insert(
        "cart".to_owned(),
        Changed {
            files: 1,
            added: 3,
            removed: 1,
        },
    );
    let (answered, asked) = shown(json!({ "at": ["shop"] }), told).await;

    assert_eq!(opened_at(&asked), Some((None, vec!["shop".to_owned()])));
    let answered = answered.unwrap();
    assert!(
        answered.contains(
            "open at `shop` Shop, for the human to see where things are.\n\
             It shows: The shop\n\
             Inside `shop` Shop:\n\
             `cart` Cart (shop); opens on its 1 type; changed: +3 -1 in 1 file\n\
             `gone` Gone (shop/gone); drift: 1\n\
             Lines, with how often the code of their ends meets: `buyer` → `cart`"
        ),
        "{answered}"
    );
}

#[tokio::test]
async fn a_type_opens_the_boxes_above_it_and_tells_its_members() {
    let (answered, asked) = shown(json!({ "at": ["shop/Cart"] }), told(&[])).await;

    assert_eq!(
        opened_at(&asked),
        Some((
            None,
            vec!["shop".to_owned(), "cart".to_owned(), "shop/Cart".to_owned()]
        ))
    );
    let answered = answered.unwrap();
    assert!(
        answered.starts_with(
            "The map of the project is on the stage, open at `shop` Shop › `cart` Cart › \
             `shop/Cart` Cart,"
        ),
        "{answered}"
    );
    assert!(
        answered.contains(
            "\nInside `cart` Cart:\n`shop/Cart` struct Cart: 2 fields, 1 function; Holds the items.\n"
        ),
        "{answered}"
    );
    assert!(
        answered.contains(
            "\nChosen: `shop/Cart` struct Cart, at src/lib.rs:2: Holds the items.\n\
             - field `items`: Vec<String, Global>; used in 1 place, written in 1\n\
             - field `count`: usize; used in 1 place, written in 1\n\
             - function `add`: pub fn add(&mut self, item: String); called from 0 places, calls 2"
        ),
        "{answered}"
    );
}

#[tokio::test]
async fn a_function_opens_its_type_and_tells_what_it_calls() {
    let (answered, asked) = shown(json!({ "at": ["cart", "shop/Cart::add"] }), told(&[])).await;

    let (_, at) = opened_at(&asked).unwrap();
    assert_eq!(at, ["shop", "cart", "shop/Cart", "shop/Cart::add"]);
    let answered = answered.unwrap();
    assert!(
        answered.contains(
            "\nThe function `shop/Cart::add`: pub fn add(&mut self, item: String), at \
             src/lib.rs:9; Puts an item in the cart.\n\
             Called from: nowhere\n\
             Calls: none\n\
             Uses fields: Cart.items, writes, Cart.count, writes"
        ),
        "{answered}"
    );
}

#[tokio::test]
async fn a_node_the_map_has_not_is_an_error_naming_its_boxes() {
    let (answered, asked) = shown(json!({ "at": ["nowhere"] }), told(&[])).await;

    assert_eq!(
        answered.unwrap_err(),
        "the map has no node or function `nowhere`; its boxes are: buyer, cart, gone, shop"
    );
    assert!(asked.is_empty(), "the stage is asked nothing");
}

#[tokio::test]
async fn a_box_off_the_way_to_the_last_is_an_error() {
    let (answered, asked) = shown(json!({ "at": ["gone", "shop/Cart"] }), told(&[])).await;

    assert_eq!(
        answered.unwrap_err(),
        "`gone` is not on the way to `shop/Cart`, which opens at `shop` › `cart` › `shop/Cart`"
    );
    assert!(asked.is_empty(), "the stage is asked nothing");
}

#[tokio::test]
async fn a_map_is_shown_by_its_name_and_names_the_projects_other_maps() {
    let (answered, asked) = shown(json!({ "map": "shop" }), told(&["build"])).await;

    assert_eq!(
        opened_at(&asked),
        Some((Some("shop".to_owned()), Vec::new()))
    );
    let answered = answered.unwrap();
    assert!(
        answered.starts_with("The map `shop` of the project is on the stage"),
        "{answered}"
    );
    assert!(
        answered.ends_with("\nThe project's other maps, which `map` shows:\n`build`: The build"),
        "{answered}"
    );
}

#[tokio::test]
async fn a_map_the_project_has_not_is_an_error_naming_those_it_has() {
    let mut told = told(&["build"]);
    told.name = None;
    told.file = None;
    told.map = None;
    told.state = State::None;
    let (answered, asked) = shown(json!({ "map": "gone" }), told).await;

    assert_eq!(
        answered.unwrap_err(),
        "the project has no map `gone`; it has: shop, build"
    );
    assert!(asked.is_empty(), "the stage is asked nothing");
}

#[tokio::test]
async fn tells_how_to_write_a_vocabulary_when_the_project_has_none() {
    let mut told = told(&[]);
    told.maps.clear();
    told.name = None;
    told.file = None;
    told.map = None;
    told.state = State::None;
    let (answered, asked) = shown(json!({}), told).await;

    let answered = answered.unwrap();
    assert!(
        answered.starts_with("The project has no map yet"),
        "{answered}"
    );
    assert!(answered.contains(MAP_FORMAT), "{answered}");
    assert_eq!(opened_at(&asked), Some((None, Vec::new())));
}

#[tokio::test]
async fn a_vocabulary_that_does_not_read_is_told_with_its_problems() {
    let mut told = told(&[]);
    told.map = None;
    told.state = State::Invalid;
    told.problems = vec!["nodes.cart: unknown field `claim`".to_owned()];
    let (answered, asked) = shown(json!({ "at": ["shop"] }), told).await;

    let answered = answered.unwrap();
    assert!(
        answered.starts_with(
            "The map of the project is on the stage, but its vocabulary, maps/shop.json, does \
             not read, as the Map tab says too:\nnodes.cart: unknown field `claim`\nFix it, \
             and show it again. "
        ),
        "{answered}"
    );
    assert_eq!(opened_at(&asked), Some((None, Vec::new())));
}

#[tokio::test]
async fn how_the_indexes_stand_is_told_until_they_are_ready() {
    let mut told = told(&[]);
    told.state = State::Indexing;
    told.indexes[0].state = IndexState::Indexing { since: 0 };
    told.indexes[0].drawn = Some("fedcba9876543210".to_owned());
    told.indexes.push(Indexed {
        directory: "directories/web".to_owned(),
        language: Some("Go".to_owned()),
        indexer: Some("scip-go".to_owned()),
        commit: Some("aaaaaaaaaaaa".to_owned()),
        drawn: None,
        note: None,
        state: IndexState::Missing {
            how: "go is not on Cox's PATH: install Go: https://go.dev/doc/install".to_owned(),
        },
    });
    if let Some(map) = &mut told.map {
        map.drift.clear();
    }
    let (answered, _) = shown(json!({}), told).await;

    let answered = answered.unwrap();
    assert!(
        answered.contains(
            "\nThe Rust code at 0123456 is being indexed by rust-analyzer: the map is drawn \
             from fedcba9 meanwhile.\n\
             The Go code of directories/web is not in the map: go is not on Cox's PATH: \
             install Go: https://go.dev/doc/install\n\
             The drift is told once all the code is indexed.\n"
        ),
        "{answered}"
    );
    assert!(!answered.contains("Drift,"), "{answered}");
}

#[tokio::test]
async fn agreed_drift_is_told_with_why() {
    let mut told = told(&[]);
    if let Some(map) = &mut told.map {
        map.drift[0].agreed = Some("Kept for the next release.".to_owned());
    }
    let (answered, _) = shown(json!({}), told).await;

    let answered = answered.unwrap();
    assert!(
        answered.ends_with(
            "\nAgreed to leave as it is, 1:\n- `claim-gone:shop/gone`: Kept for the next release."
        ),
        "{answered}"
    );
    assert!(!answered.contains("Drift,"), "{answered}");
}

#[tokio::test]
async fn a_type_not_there_while_the_code_is_indexed_is_said_to_wait() {
    let mut told = told(&[]);
    told.state = State::Indexing;
    told.indexes[0].state = IndexState::Indexing { since: 0 };
    let (answered, asked) = shown(json!({ "at": ["shop/Basket"] }), told).await;

    assert_eq!(
        answered.unwrap_err(),
        "the map has no node or function `shop/Basket`; its boxes are: buyer, cart, gone, shop; \
         not all the code is indexed yet, so its types may be missing"
    );
    assert!(asked.is_empty(), "the stage is asked nothing");
}

#[tokio::test]
async fn drift_on_what_is_shown_comes_first_and_the_rest_is_counted() {
    let mut told = told(&[]);
    if let Some(map) = &mut told.map {
        let gone = map.drift.remove(0);
        for number in 0..20 {
            map.drift.push(Drift {
                id: format!("unowned:far/{number}"),
                kind: DriftKind::Unowned,
                nodes: vec!["far".to_owned()],
                text: format!("No box claims far/{number}"),
                count: Some(20),
                at: Vec::new(),
                agreed: None,
            });
        }
        // Last, as the generator gives it, but on a box the level shows.
        map.drift.push(gone);
    }
    let (answered, _) = shown(json!({ "at": ["shop"] }), told).await;

    let answered = answered.unwrap();
    let listed: Vec<&str> = answered
        .lines()
        .filter(|line| line.starts_with("- `"))
        .collect();
    assert_eq!(listed.len(), DRIFT_TOLD, "{answered}");
    assert!(
        listed[0].starts_with("- `claim-gone:shop/gone`"),
        "{answered}"
    );
    assert!(
        answered.contains("\n- and 6 more, which the Map tab's Drift lists"),
        "{answered}"
    );
}

#[tokio::test]
async fn callers_are_told_a_function_once_and_tests_as_a_count() {
    let mut told = told(&[]);
    let called = |line, within: &str, function: Option<&str>, test| Use {
        file: "src/lib.rs".to_owned(),
        line,
        within: within.to_owned(),
        declaration: false,
        function: function.map(str::to_owned),
        module: "shop".to_owned(),
        node: Some("cart".to_owned()),
        test,
        writes: false,
    };
    if let Some(map) = &mut told.map {
        let add = &mut map.types.get_mut("shop/Cart").unwrap().functions[0];
        add.uses = vec![
            called(20, "Shop::buy", Some("shop/Shop::buy"), false),
            called(21, "Shop::buy", Some("shop/Shop::buy"), false),
            called(30, "checkout", None, false),
            called(40, "adds", None, true),
            called(41, "adds", None, true),
        ];
    }
    let (answered, _) = shown(json!({ "at": ["shop/Cart::add"] }), told).await;

    let answered = answered.unwrap();
    assert!(
        answered.contains(
            "\nCalled from: Shop::buy at src/lib.rs:20 (2 times), checkout at src/lib.rs:30; and \
             2 places in tests\n"
        ),
        "{answered}"
    );
}
