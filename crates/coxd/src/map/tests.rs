//! The maps, drawn from the indexes a fake indexer writes: a script that
//! counts its runs, writes the index rust-analyzer wrote of a small crate
//! `shop` (see `cox_editor::scip`), and waits or fails as the test says.

use std::path::{Path, PathBuf};

use super::cache::Key;
use super::indexers::{Indexer, Program};
use super::*;
use crate::testing::{TempDir, run_git, waits};

/// The index rust-analyzer wrote of `src/lib.rs` of a crate `shop`, and
/// that file.
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../cox-editor/src/scip/fixtures/cart-rust.scip"
);
const LIB: &str = include_str!("../../../cox-editor/src/scip/fixtures/cart-rust.txt");

/// A map whose box `cart` owns the crate's code.
const VOCABULARY: &str = r#"{
  "says": "The shop",
  "nodes": {
    "shop": { "name": "Shop" },
    "buyer": { "name": "Buyer", "icon": "person", "kind": "outside" },
    "cart": { "name": "Cart", "claims": ["shop"] },
    "gone": { "name": "Gone", "claims": ["shop/gone"] }
  },
  "top": { "center": "shop", "left": ["buyer"], "lines": [{ "from": "buyer", "to": "shop" }] },
  "levels": {
    "shop": {
      "box": [0, 0, 400, 300],
      "outer": { "buyer": ["left", 0] },
      "parts": { "cart": [20, 20], "gone": [200, 20] },
      "lines": [{ "from": "buyer", "to": "cart" }]
    }
  }
}"#;

/// What a test's fake indexer does, and the maps that run it.
struct Fake {
    /// Where its runs are counted, a line each (see [`Run`]).
    runs: PathBuf,
    /// While this file is there, it waits.
    gate: PathBuf,
    /// While this file is there, it fails.
    fail: PathBuf,
    maps: Maps,
    /// Where it, its runs and its indexes are.
    folder: TempDir,
}

impl Fake {
    fn new() -> Self {
        Self::with(|indexer| indexer)
    }

    /// The fake indexer, as `change` changes it.
    fn with(change: impl FnOnce(Indexer) -> Indexer) -> Self {
        let folder = TempDir::new();
        let runs = folder.join("runs");
        let gate = folder.join("gate");
        let fail = folder.join("fail");
        let script = folder.join("fake-indexer");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = --version ]; then echo 'fake 1.0'; exit 0; fi\n\
                 echo \"$(pwd)|$(head -c 40 src/lib.rs | tr '\\n' ' ')|$(ls -d deps */*/deps 2>/dev/null | tr '\\n' ' ')|$$|$*\" >> '{runs}'\n\
                 while [ -f '{gate}' ]; do sleep 0.02; done\n\
                 if [ -f '{fail}' ]; then echo 'warming up' >&2; echo 'the shop is closed' >&2; exit 3; fi\n\
                 cp '{FIXTURE}' \"$1\"\n",
                runs = runs.display(),
                gate = gate.display(),
                fail = fail.display(),
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
        let indexer = change(Indexer {
            name: "fake".to_owned(),
            language: "Rust".to_owned(),
            markers: vec!["Cargo.toml".to_owned()],
            unless: Vec::new(),
            members: None,
            program: Program::Path(script.display().to_string()),
            args: vec!["{out}".to_owned()],
            config: None,
            needs: Vec::new(),
            borrows: None,
        });
        std::fs::create_dir_all(folder.join("scratch")).unwrap();
        let maps = Maps::new(
            folder.join("cache"),
            Some(folder.join("scratch")),
            vec![indexer],
        );
        Self {
            runs,
            gate,
            fail,
            maps,
            folder,
        }
    }

    /// Its runs so far.
    fn runs(&self) -> Vec<Run> {
        std::fs::read_to_string(&self.runs)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                Run {
                    folder: parts[0].to_owned(),
                    lib: parts[1].to_owned(),
                    deps: parts[2].split_whitespace().any(|deps| deps == "deps"),
                    member_deps: parts[2]
                        .split_whitespace()
                        .filter(|deps| *deps != "deps")
                        .map(str::to_owned)
                        .collect(),
                    pid: parts[3].parse().unwrap(),
                    args: parts[4].split_whitespace().map(str::to_owned).collect(),
                }
            })
            .collect()
    }

    /// Its runs, once there are `count` of them.
    fn runs_when(&self, count: usize) -> Vec<Run> {
        waits::until("the fake indexer runs", || self.runs().len() >= count);
        self.runs()
    }

    /// What the fake indexer's index of the head of the repository in
    /// `root` is kept by.
    fn key_of(root: &Path, without: bool) -> Key {
        Key {
            commit: run_git(root, &["rev-parse", "HEAD"])
                .unwrap()
                .trim()
                .to_owned(),
            within: String::new(),
            indexer: "fake".to_owned(),
            version: "fake 1.0".to_owned(),
            without,
        }
    }

    /// The map named `shop` of `root`, as soon as no index of it is
    /// being made.
    fn settled(&self, root: &Path) -> Told {
        self.settled_as(root, "shop")
    }

    /// The map named `name` of `root`, as soon as no index of it is
    /// being made.
    fn settled_as(&self, root: &Path, name: &str) -> Told {
        let mut told = None;
        waits::until("the indexes are made", || {
            let now = self.maps.told(root, Some(name), false);
            let settled = now.state != State::Indexing;
            told = Some(now);
            settled
        });
        told.unwrap()
    }
}

/// A run of the fake indexer: the folder it ran in, what `src/lib.rs`
/// began with there, whether `deps` was there, and in which of the
/// folders there, its process, and its arguments.
#[derive(Debug, PartialEq, Eq)]
struct Run {
    folder: String,
    lib: String,
    deps: bool,
    member_deps: Vec<String>,
    pid: libc::pid_t,
    args: Vec<String>,
}

/// Whether the process `pid` runs.
fn alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// A repository in `folder` of the crate, with its map, committed.
fn repository(folder: &Path) -> String {
    std::fs::create_dir_all(folder.join("src")).unwrap();
    std::fs::create_dir_all(folder.join("maps")).unwrap();
    std::fs::write(folder.join("Cargo.toml"), "[package]\nname = \"shop\"\n").unwrap();
    std::fs::write(folder.join("src/lib.rs"), LIB).unwrap();
    std::fs::write(folder.join("maps/shop.json"), VOCABULARY).unwrap();
    // Each repository's commit its own, as two of the same code, made in
    // the same second, would be one.
    std::fs::write(folder.join("README.md"), folder.display().to_string()).unwrap();
    run_git(folder, &["init", "-q", "-b", "main"]).unwrap();
    commit(folder, "first")
}

/// Commits all in `folder` as `message`, and answers the commit.
fn commit(folder: &Path, message: &str) -> String {
    run_git(folder, &["add", "-A"]).unwrap();
    run_git(folder, &["commit", "-q", "--allow-empty", "-m", message]).unwrap();
    run_git(folder, &["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn a_map_is_drawn_from_the_index_of_the_commit_checked_out() {
    let fake = Fake::new();
    let root = TempDir::new();
    let first = repository(&root);

    let told = fake.maps.told(&root, Some("shop"), false);
    assert_eq!(told.state, State::Indexing);
    assert!(matches!(told.indexes[0].state, IndexState::Indexing { .. }));
    assert_eq!(told.indexes[0].commit.as_deref(), Some(first.as_str()));
    assert_eq!(told.indexes[0].drawn, None);
    // The boxes and lines a person keeps are there before any index.
    let map = told.map.unwrap();
    assert_eq!(map.top.center, "shop");
    assert!(map.types.is_empty());
    assert_eq!(map.drift, []);

    let told = fake.settled(&root);
    assert_eq!(told.state, State::Ready);
    assert_eq!(told.name.as_deref(), Some("shop"));
    assert_eq!(told.maps[0].says, "The shop");
    let indexed = &told.indexes[0];
    assert_eq!(indexed.state, IndexState::Ready);
    assert_eq!(indexed.directory, "");
    assert_eq!(indexed.language.as_deref(), Some("Rust"));
    assert_eq!(indexed.indexer.as_deref(), Some("fake"));
    assert_eq!(indexed.drawn.as_deref(), Some(first.as_str()));
    let map = told.map.unwrap();
    assert_eq!(map.types["shop/Cart"].owner, "cart");
    let drift: Vec<&str> = map.drift.iter().map(|drift| drift.id.as_str()).collect();
    assert_eq!(drift, ["claim-gone:shop/gone"]);

    // Asked again at the same commit, and by another run of coxd, it is
    // not indexed again.
    assert_eq!(
        fake.maps.told(&root, Some("shop"), false).state,
        State::Ready
    );
    let again = Maps::new(
        fake.folder.join("cache"),
        Some(fake.folder.join("scratch")),
        fake.maps.0.indexers.clone(),
    );
    assert_eq!(again.told(&root, Some("shop"), false).state, State::Ready);
    assert_eq!(fake.runs().len(), 1);
}

#[test]
fn what_changed_falls_on_the_boxes_and_types_whose_files_it_is_in() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    assert_eq!(fake.settled(&root).changed, BTreeMap::new());

    let lib = format!("{LIB}\n/// More.\npub fn more() {{}}\n");
    std::fs::write(root.join("src/lib.rs"), lib).unwrap();
    std::fs::write(root.join("README.md"), "Another\n").unwrap();
    let told = fake.settled(&root);
    let changed = Changed {
        files: 1,
        added: 3,
        removed: 0,
    };
    // The box that owns the file, the box it is inside of, and the type
    // the file has; the README no box owns falls on none.
    assert_eq!(
        told.changed,
        BTreeMap::from([
            ("cart".to_owned(), changed),
            ("shop".to_owned(), changed),
            ("shop/Cart".to_owned(), changed),
        ])
    );
    // Not committed, the change is not indexed.
    assert_eq!(fake.runs().len(), 1);
}

#[test]
fn a_map_is_drawn_again_as_its_vocabulary_changes() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    let told = fake.settled(&root);
    assert_eq!(told.map.unwrap().nodes["cart"].name, "Cart");
    // Not committed: the vocabulary is read as it is now.
    let renamed = VOCABULARY.replace(r#""name": "Cart""#, r#""name": "Basket""#);
    std::fs::write(root.join("maps/shop.json"), renamed).unwrap();
    let told = fake.settled(&root);
    assert_eq!(told.map.unwrap().nodes["cart"].name, "Basket");
    assert_eq!(fake.runs().len(), 1);
}

#[test]
fn a_new_commit_is_indexed_again_drawn_from_the_last_meanwhile() {
    let fake = Fake::new();
    let root = TempDir::new();
    let first = repository(&root);
    assert_eq!(fake.settled(&root).state, State::Ready);

    std::fs::write(root.join("README.md"), "The shop.\n").unwrap();
    let second = commit(&root, "second");
    std::fs::write(&fake.gate, "").unwrap();
    let told = fake.maps.told(&root, Some("shop"), false);
    assert_eq!(told.state, State::Indexing);
    assert_eq!(told.indexes[0].commit.as_deref(), Some(second.as_str()));
    assert_eq!(told.indexes[0].drawn.as_deref(), Some(first.as_str()));
    assert!(told.map.unwrap().types.contains_key("shop/Cart"));

    std::fs::remove_file(&fake.gate).unwrap();
    let told = fake.settled(&root);
    assert_eq!(told.state, State::Ready);
    assert_eq!(told.indexes[0].drawn.as_deref(), Some(second.as_str()));
    assert_eq!(fake.runs().len(), 2);
}

#[test]
fn a_commit_is_indexed_once_however_often_it_is_asked_for() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    std::fs::write(&fake.gate, "").unwrap();
    let asking: Vec<_> = (0..4)
        .map(|_| {
            let maps = fake.maps.clone();
            let root = root.to_path_buf();
            std::thread::spawn(move || maps.told(&root, Some("shop"), false).state)
        })
        .collect();
    for asked in asking {
        assert_eq!(asked.join().unwrap(), State::Indexing);
    }
    assert_eq!(fake.maps.told(&root, None, false).state, State::Indexing);
    std::fs::remove_file(&fake.gate).unwrap();
    assert_eq!(fake.settled(&root).state, State::Ready);
    assert_eq!(fake.runs().len(), 1);
}

#[test]
fn the_code_indexed_is_the_commits_not_what_is_not_committed() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    std::fs::write(root.join("src/lib.rs"), "// Not committed.\n").unwrap();
    let told = fake.settled(&root);
    assert_eq!(told.state, State::Ready);
    let runs = fake.runs();
    assert!(runs[0].lib.contains("/// Holds the items."), "{runs:?}");
    assert!(
        !runs[0].folder.starts_with(&root.display().to_string()),
        "{runs:?}"
    );
    // The map shows the code's text as committed.
    let cart = &told.map.unwrap().types["shop/Cart"];
    assert_eq!(cart.doc, "Holds the items.");
}

#[test]
fn a_failed_index_is_told_why_and_made_again_only_when_asked() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    std::fs::write(&fake.fail, "").unwrap();
    let told = fake.settled(&root);
    assert_eq!(told.state, State::Incomplete);
    let IndexState::Failed { reason } = &told.indexes[0].state else {
        panic!("not failed: {told:?}");
    };
    assert!(reason.contains("fake failed"), "{reason}");
    assert!(reason.contains("the shop is closed"), "{reason}");
    let map = told.map.unwrap();
    assert!(map.types.is_empty());
    assert_eq!(map.drift, []);

    assert_eq!(fake.settled(&root).state, State::Incomplete);
    assert_eq!(fake.runs().len(), 1);

    std::fs::remove_file(&fake.fail).unwrap();
    assert_eq!(
        fake.maps.told(&root, Some("shop"), true).state,
        State::Indexing
    );
    assert_eq!(fake.settled(&root).state, State::Ready);
    assert_eq!(fake.runs().len(), 2);
}

#[test]
fn what_the_checkout_has_that_the_commit_does_not_is_read_once_there() {
    let fake = Fake::with(|indexer| Indexer {
        borrows: Some(("deps".to_owned(), "deps is not installed".to_owned())),
        ..indexer
    });
    let root = TempDir::new();
    repository(&root);
    let told = fake.settled(&root);
    assert_eq!(told.state, State::Ready);
    assert_eq!(
        told.indexes[0].note.as_deref(),
        Some("deps is not installed")
    );

    // Installed, not committed: the same commit is indexed again, with it.
    std::fs::create_dir_all(root.join("deps")).unwrap();
    assert_eq!(
        fake.maps.told(&root, Some("shop"), false).state,
        State::Indexing
    );
    let told = fake.settled(&root);
    assert_eq!(told.indexes[0].note, None);
    let runs = fake.runs();
    assert_eq!(runs.len(), 2);
    assert!(!runs[0].deps, "{runs:?}");
    assert!(runs[1].deps, "{runs:?}");
}

#[test]
fn the_members_of_a_repository_are_indexed_in_one_run_at_its_top() {
    let fake = Fake::with(|indexer| Indexer {
        members: Some(indexers::Members::GoWork),
        args: vec!["{out}".to_owned(), "./{member}/...".to_owned()],
        borrows: Some(("deps".to_owned(), "deps is not installed".to_owned())),
        ..indexer
    });
    let root = TempDir::new();
    // The crate's code, and its map, at the top, for the fake index's
    // paths to be those of files; what says a crate is there, in the
    // members only.
    repository(&root);
    std::fs::remove_file(root.join("Cargo.toml")).unwrap();
    for member in ["apps/till", "apps/empty", "packages/cart"] {
        std::fs::create_dir_all(root.join(member)).unwrap();
        std::fs::write(root.join(member).join("notes.md"), member).unwrap();
    }
    for member in ["apps/till", "packages/cart"] {
        std::fs::write(root.join(member).join("Cargo.toml"), "[package]\n").unwrap();
    }
    std::fs::write(
        root.join("go.work"),
        "use (\n\t./apps/till\n\t./apps/empty\n\t./packages/cart\n)\n",
    )
    .unwrap();
    commit(&root, "members");
    // Installed in one member, not committed.
    std::fs::create_dir_all(root.join("deps")).unwrap();
    std::fs::create_dir_all(root.join("packages/cart/deps")).unwrap();

    let told = fake.settled(&root);
    assert_eq!(told.state, State::Ready, "{:?}", told.indexes);
    assert_eq!(told.indexes.len(), 1);
    assert!(told.map.unwrap().types.contains_key("shop/Cart"));
    let runs = fake.runs();
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert!(runs[0].folder.ends_with("/code"), "{runs:?}");
    assert_eq!(
        runs[0].args[1..],
        ["./apps/till/...", "./packages/cart/..."],
        "the members that hold a crate, from the top"
    );
    assert!(runs[0].deps);
    assert_eq!(runs[0].member_deps, ["packages/cart/deps"]);
}

#[test]
fn a_missing_indexer_is_told_with_how_to_get_it() {
    let fake = Fake::with(|indexer| Indexer {
        needs: vec![(
            "no-such-program-for-cox".to_owned(),
            "install it from example.com".to_owned(),
        )],
        ..indexer
    });
    let root = TempDir::new();
    repository(&root);
    let told = fake.maps.told(&root, Some("shop"), false);
    assert_eq!(told.state, State::Incomplete);
    let IndexState::Missing { how } = &told.indexes[0].state else {
        panic!("not missing: {told:?}");
    };
    assert!(
        how.contains("no-such-program-for-cox is not on Cox's PATH"),
        "{how}"
    );
    assert!(how.contains("example.com"), "{how}");
    // The map still has the boxes and lines a person keeps, but no drift.
    let map = told.map.unwrap();
    assert_eq!(map.top.left, ["buyer"]);
    assert!(map.levels["shop"].parts.contains_key("cart"));
    assert_eq!(map.drift, []);
    assert_eq!(fake.runs(), []);
}

#[test]
fn code_no_indexer_reads_or_not_committed_is_told() {
    let fake = Fake::new();
    let root = TempDir::new();
    std::fs::create_dir_all(root.join("maps")).unwrap();
    std::fs::write(root.join("maps/shop.json"), VOCABULARY).unwrap();
    let told = fake.maps.told(&root, Some("shop"), false);
    assert_eq!(told.state, State::Incomplete);
    assert!(matches!(
        told.indexes[0].state,
        IndexState::Uncommitted { .. }
    ));

    run_git(&root, &["init", "-q", "-b", "main"]).unwrap();
    commit(&root, "a map alone");
    let told = fake.maps.told(&root, Some("shop"), false);
    assert_eq!(
        told.indexes[0].state,
        IndexState::Unknown {
            known: vec!["Rust".to_owned()]
        }
    );
    assert_eq!(told.map.unwrap().drift, []);
}

#[test]
fn a_project_of_several_repositories_is_drawn_from_each() {
    let fake = Fake::new();
    let root = TempDir::new();
    std::fs::create_dir_all(root.join("maps")).unwrap();
    std::fs::write(root.join("maps/shop.json"), VOCABULARY).unwrap();
    let directories = root.join(crate::settings::DIRECTORIES);
    repository(&directories.join("front"));
    repository(&directories.join("back"));
    let told = fake.settled(&root);
    assert_eq!(told.state, State::Ready);
    let directories: Vec<&str> = told
        .indexes
        .iter()
        .map(|indexed| indexed.directory.as_str())
        .collect();
    assert_eq!(directories, ["directories/back", "directories/front"]);
    // The paths are the project's, through the repository's directory.
    let file = &told.map.unwrap().types["shop/Cart"].file;
    assert!(
        [
            "directories/back/src/lib.rs",
            "directories/front/src/lib.rs"
        ]
        .contains(&file.as_str()),
        "{file}"
    );
    assert_eq!(fake.runs().len(), 2);
}

#[test]
fn a_map_a_repository_keeps_is_the_projects_its_files_from_the_repositorys() {
    let fake = Fake::new();
    let root = TempDir::new();
    let app = root.join(crate::settings::DIRECTORIES).join("app");
    repository(&app);
    // A map whose box owns its file, as the repository has it; the
    // project's own map is the same file.
    let by_file = VOCABULARY.replace(r#""claims": ["shop"]"#, r#""files": ["src/lib.rs"]"#);
    std::fs::write(app.join("maps/shop.json"), &by_file).unwrap();
    commit(&app, "by file");
    std::fs::create_dir_all(root.join("maps")).unwrap();
    std::fs::write(root.join("maps/shop.json"), &by_file).unwrap();

    let told = fake.settled_as(&root, "app/shop");
    assert_eq!(told.state, State::Ready);
    let names: Vec<&str> = told.maps.iter().map(|map| map.name.as_str()).collect();
    assert_eq!(names, ["shop", "app/shop"]);
    // As the stage's bar finds them, to show the Map.
    assert_eq!(list(&root), told.maps);
    assert_eq!(told.name.as_deref(), Some("app/shop"));
    assert_eq!(told.file, Some(app.join("maps/shop.json")));
    let map = told.map.unwrap();
    assert_eq!(map.types["shop/Cart"].owner, "cart");
    assert_eq!(map.types["shop/Cart"].file, "directories/app/src/lib.rs");
    let drift: Vec<&str> = map.drift.iter().map(|drift| drift.id.as_str()).collect();
    assert_eq!(drift, ["claim-gone:shop/gone"]);

    // The project's own, of the same name, is its first: its file is
    // claimed from the project's folder, which has no `src/lib.rs`.
    for name in [None, Some("shop")] {
        let told = fake.maps.told(&root, name, false);
        assert_eq!(told.name.as_deref(), Some("shop"));
        assert_eq!(told.file, Some(root.join("maps/shop.json")));
        let map = told.map.unwrap();
        assert!(map.types.is_empty());
        let drift: Vec<&str> = map.drift.iter().map(|drift| drift.id.as_str()).collect();
        assert_eq!(drift, ["claim-gone:shop/gone", "claim-gone:src/lib.rs"]);
    }

    // A chat that works in the repository has its map by its own name.
    let told = fake.maps.told(&app, Some("shop"), false);
    assert_eq!(told.file, Some(app.join("maps/shop.json")));
    assert_eq!(told.map.unwrap().types["shop/Cart"].owner, "cart");
    assert_eq!(fake.runs().len(), 1);
}

#[test]
fn a_map_that_does_not_read_or_is_not_there_is_told_why() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    let told = fake.maps.told(&root, Some("nowhere"), false);
    assert_eq!(told.state, State::None);
    assert_eq!(told.problems, ["the project has no map `nowhere`"]);

    std::fs::write(
        root.join("maps/shop.json"),
        r#"{ "nodes": {}, "top": { "center": "x" } }"#,
    )
    .unwrap();
    let told = fake.maps.told(&root, None, false);
    assert_eq!(told.state, State::Invalid);
    assert!(
        told.problems[0].contains("no node `x`"),
        "{:?}",
        told.problems
    );
    assert_eq!(fake.runs(), []);
    let json: serde_json::Value = serde_json::from_str(&told.json(&Part::default())).unwrap();
    assert_eq!(json["state"], "invalid");
}

#[test]
fn how_a_map_stands_is_told_as_json() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    let told = fake.settled(&root);
    let json: serde_json::Value = serde_json::from_str(&told.json(&Part::default())).unwrap();
    assert_eq!(json["state"], "ready");
    assert_eq!(json["name"], "shop");
    assert_eq!(json["indexes"][0]["state"], "ready");
    assert_eq!(json["indexes"][0]["indexer"], "fake");
    assert_eq!(json["map"]["top"]["center"], "shop");
    assert!(json["map"]["types"]["shop/Cart"].is_object());
}

#[test]
fn a_page_is_told_the_map_without_its_types_details_and_each_as_it_asks() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    let told = fake.settled(&root);
    let json: serde_json::Value = serde_json::from_str(&told.json(&Part::default())).unwrap();
    let cart = &json["map"]["types"]["shop/Cart"];
    assert_eq!(cart["box"], "cart");
    assert_eq!(cart["functions"], serde_json::Value::Null);
    assert_eq!(cart["uses"], serde_json::Value::Null);

    let detail = told.json(&Part::Type("shop/Cart".to_owned()));
    let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert_eq!(detail["tag"], json["tag"]);
    assert_eq!(detail["type"]["id"], "shop/Cart");
    assert!(detail["type"]["functions"].is_array());
    assert!(detail["to"].is_object());

    let none = told.json(&Part::Type("shop/Nothing".to_owned()));
    let none: serde_json::Value = serde_json::from_str(&none).unwrap();
    assert_eq!(none, serde_json::json!({ "tag": json["tag"] }));
}

#[test]
fn a_page_that_has_the_map_is_told_the_rest_alone() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    let seen = |tag: &str| Part::Outline {
        seen: Some(tag.to_owned()),
    };
    let mut told = fake.settled(&root);
    let full = told.json(&Part::default());
    let json: serde_json::Value = serde_json::from_str(&full).unwrap();
    let tag = json["tag"].as_str().unwrap().to_owned();
    assert_eq!(tag.len(), 16);
    assert_eq!(told.json(&seen("0123456789abcdef")), full);

    // What changed in the code changes; the map does not.
    told.changed.insert(
        "shop".to_owned(),
        super::Changed {
            files: 1,
            added: 2,
            removed: 3,
        },
    );
    let rest = told.json(&seen(&tag));
    assert!(rest.len() < full.len() / 2, "{rest}");
    let rest: serde_json::Value = serde_json::from_str(&rest).unwrap();
    assert_eq!(rest["tag"], tag.as_str());
    assert_eq!(rest["map"], serde_json::Value::Null);
    assert_eq!(rest["changed"]["shop"]["added"], 2);
    assert_eq!(rest["state"], "ready");
    assert_eq!(rest["indexes"][0]["state"], "ready");

    // The same map drawn again has the same tag.
    assert_eq!(fake.settled(&root).tag.as_deref(), Some(tag.as_str()));

    // A map that changed is told whole, with its new tag.
    let renamed = VOCABULARY.replace(r#""name": "Cart""#, r#""name": "Basket""#);
    std::fs::write(root.join("maps/shop.json"), renamed).unwrap();
    let other = fake.settled(&root);
    assert_ne!(other.tag.as_deref(), Some(tag.as_str()));
    let other: serde_json::Value = serde_json::from_str(&other.json(&seen(&tag))).unwrap();
    assert_eq!(other["map"]["nodes"]["cart"]["name"], "Basket");

    // A type's detail, which the outline leaves out, changes the tag too.
    let mut map = Generated::clone(told.map.as_ref().unwrap());
    assert_eq!(super::tag_of(&map), tag);
    map.types.get_mut("shop/Cart").unwrap().doc.push('!');
    assert_ne!(super::tag_of(&map), tag);

    // No map, no tag: all of it.
    told.map = None;
    told.tag = None;
    let none = told.json(&seen(&tag));
    assert!(!none.contains("\"tag\""));
}

#[test]
fn two_indexes_of_one_commit_made_at_once_keep_apart() {
    let fake = Fake::with(|indexer| Indexer {
        borrows: Some(("deps".to_owned(), "deps is not installed".to_owned())),
        ..indexer
    });
    let root = TempDir::new();
    repository(&root);
    // Another checkout of the same commit, with what the indexer borrows.
    let other = TempDir::new();
    run_git(&other, &["clone", "-q", &root.display().to_string(), "."]).unwrap();
    std::fs::create_dir_all(other.join("deps")).unwrap();
    std::fs::write(&fake.gate, "").unwrap();
    assert_eq!(
        fake.maps.told(&root, Some("shop"), false).state,
        State::Indexing
    );
    assert_eq!(
        fake.maps.told(&other, Some("shop"), false).state,
        State::Indexing
    );
    let runs = fake.runs_when(2);
    assert_ne!(
        runs[0].folder, runs[1].folder,
        "each in a folder of its own"
    );
    std::fs::remove_file(&fake.gate).unwrap();
    for (root, note) in [(&*root, Some("deps is not installed")), (&*other, None)] {
        let told = fake.settled(root);
        assert_eq!(told.state, State::Ready);
        assert_eq!(told.indexes[0].note.as_deref(), note);
        assert_eq!(told.map.unwrap().types["shop/Cart"].doc, "Holds the items.");
    }
    assert_eq!(fake.runs().len(), 2);
}

#[test]
fn a_newer_commits_index_stops_the_one_of_the_commit_before() {
    let fake = Fake::new();
    let root = TempDir::new();
    let first = repository(&root);
    assert_eq!(fake.settled(&root).state, State::Ready);

    std::fs::write(&fake.gate, "").unwrap();
    std::fs::write(root.join("README.md"), "Second.\n").unwrap();
    commit(&root, "second");
    let second = Fake::key_of(&root, false);
    assert_eq!(
        fake.maps.told(&root, Some("shop"), false).state,
        State::Indexing
    );
    let older = fake.runs_when(2)[1].pid;

    std::fs::write(root.join("README.md"), "Third.\n").unwrap();
    let third = commit(&root, "third");
    let told = fake.maps.told(&root, Some("shop"), false);
    assert_eq!(told.state, State::Indexing);
    // Drawn from the last commit indexed meanwhile.
    assert_eq!(told.indexes[0].drawn.as_deref(), Some(first.as_str()));
    let newer = fake.runs_when(3)[2].pid;
    waits::until("the older indexer stops", || !alive(older));
    assert!(alive(newer));
    assert_eq!(fake.maps.jobs().running(), 1);

    std::fs::remove_file(&fake.gate).unwrap();
    let told = fake.settled(&root);
    assert_eq!(told.state, State::Ready);
    assert_eq!(told.indexes[0].drawn.as_deref(), Some(third.as_str()));
    assert!(
        !fake.maps.0.cache.has(&second),
        "the stopped one is not kept"
    );
    assert_eq!(fake.maps.jobs().standing(&second), None, "nor kept failed");
    // The job lets go of its checkout once it kept the index.
    waits::until("the job ends", || fake.maps.jobs().running() == 0);
}

#[test]
fn a_job_that_breaks_off_is_kept_failed_and_made_again_when_asked() {
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    let key = Fake::key_of(&root, false);
    let slot = (root.to_path_buf(), String::new(), "fake".to_owned());
    let (job, _) = fake.maps.jobs().start(slot.clone(), key.clone(), 0);
    // As its thread panicked, before it said how it ended.
    drop(Ending {
        maps: fake.maps.clone(),
        slot,
        key,
        job,
        ended: None,
    });
    let told = fake.maps.told(&root, Some("shop"), false);
    let IndexState::Failed { reason } = &told.indexes[0].state else {
        panic!("not failed: {told:?}");
    };
    assert!(reason.contains("broke off"), "{reason}");
    assert_eq!(
        fake.maps.told(&root, Some("shop"), true).state,
        State::Indexing
    );
    assert_eq!(fake.settled(&root).state, State::Ready);
}

#[test]
fn an_index_that_cannot_be_read_as_the_map_is_drawn_leaves_it_incomplete() {
    use std::os::unix::fs::PermissionsExt as _;
    let fake = Fake::new();
    let root = TempDir::new();
    repository(&root);
    assert_eq!(fake.settled(&root).state, State::Ready);
    let file = std::fs::read_dir(fake.folder.join("cache/v1"))
        .unwrap()
        .flat_map(|commit| std::fs::read_dir(commit.unwrap().path()).unwrap())
        .map(|file| file.unwrap().path())
        .next()
        .unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Another vocabulary, so that the map is drawn again.
    let renamed = VOCABULARY.replace(r#""name": "Cart""#, r#""name": "Basket""#);
    std::fs::write(root.join("maps/shop.json"), renamed).unwrap();
    let told = fake.maps.told(&root, Some("shop"), false);
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(told.state, State::Incomplete);
    assert_eq!(told.indexes[0].drawn, None);
    assert!(matches!(told.indexes[0].state, IndexState::Failed { .. }));
    let map = told.map.unwrap();
    assert_eq!(map.drift, []);
    assert_eq!(map.types.len(), 0);
    // Read now, as it is still kept.
    assert_eq!(fake.settled(&root).state, State::Ready);
}
