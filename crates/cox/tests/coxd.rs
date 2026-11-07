//! coxd over its socket: a window reaches it, one at a time, hears what
//! it says, and hears that it is gone once it stops.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cox::agent::Start;
use cox::agents::Agents;
use cox::chat::judge::Starts;
use cox::daemon::client::{self, Connected};
use cox::daemon::wire::{FromDaemon, Hello, ToDaemon};
use cox::daemon::{Spawned, Spawns};
use cox_core::entry::Attachment;
use cox_core::store::Store;
use cox_core::tabs::{KeptTab, KeptTabs};

/// Spawns no agent: the tests start no chat's agent, and judge no goal,
/// though a chat read from what is kept is given what would judge it.
struct NoAgents;

impl Spawns for NoAgents {
    fn agents(&self) -> Agents {
        Agents::new(vec!["fake".to_owned()], Vec::new(), Vec::new())
    }

    fn chat(&self, _: PathBuf, _: Start, _: &str, _: Option<u16>) -> Spawned {
        unreachable!("no chat opens")
    }

    fn starts(&self, _: &str) -> Starts {
        Rc::new(|_, _| unreachable!("no goal is judged"))
    }
}

/// A folder of the test's own.
fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("cox-coxd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Runs coxd on `socket`, on a thread of its own, until it stops.
fn run(socket: &Path, home: &Path) -> std::thread::JoinHandle<anyhow::Result<()>> {
    run_keeping(socket, home, None)
}

/// Runs coxd like [`run`], keeping the chats in the store at `kept`, or
/// in memory.
fn run_keeping(
    socket: &Path,
    home: &Path,
    kept: Option<&Path>,
) -> std::thread::JoinHandle<anyhow::Result<()>> {
    let (socket, home) = (socket.to_owned(), home.to_owned());
    let kept = kept.map(Path::to_owned);
    std::thread::spawn(move || {
        cox::daemon::process::run(
            &socket,
            home,
            move || {
                Rc::new(match &kept {
                    Some(path) => Store::open(path).unwrap(),
                    None => Store::in_memory().unwrap(),
                })
            },
            || Rc::new(NoAgents),
        )
    })
}

/// Reaches coxd on `socket`, which runs already, as a window given
/// `home`, marking `gone` once coxd goes.
fn reach(socket: &Path, home: &Path, gone: &Arc<AtomicBool>) -> anyhow::Result<Connected> {
    let gone = Arc::clone(gone);
    let hello = Hello {
        home: Some(home.to_owned()),
        command: Vec::new(),
    };
    client::connect(
        socket,
        &hello,
        &|| Err(std::io::Error::other("coxd runs already")),
        move || gone.store(true, Ordering::SeqCst),
    )
}

/// What coxd says until it answers the sync `id`.
fn until_settled(connected: &mut Connected, id: u64) -> Vec<FromDaemon> {
    let _ = connected.link.to.send(ToDaemon::Sync { id });
    let mut heard = Vec::new();
    loop {
        let message = connected.from_daemon.blocking_recv().expect("coxd answers");
        if message == (FromDaemon::Settled { id }) {
            return heard;
        }
        heard.push(message);
    }
}

/// Waits until `done` holds, or fails after a while.
fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_window_reaches_coxd_one_at_a_time_and_hears_when_it_stops() {
    let folder = folder("one");
    let socket = folder.join("run").join("coxd.sock");
    let home = folder.join("home");
    let coxd = run(&socket, &home);
    wait("coxd listens", || socket.exists());

    let gone = Arc::new(AtomicBool::new(false));
    let mut first = reach(&socket, &home, &gone).expect("the first window is welcome");
    let heard = until_settled(&mut first, 1);
    let Some(FromDaemon::Listing(listing)) = heard.first() else {
        panic!("coxd lists the chats first: {heard:?}");
    };
    assert_eq!(listing.projects[0].folder, home);
    assert_eq!(listing.chats, Vec::new());

    // Another window is not, while the first is there.
    let other = Arc::new(AtomicBool::new(false));
    let refused = reach(&socket, &home, &other)
        .err()
        .expect("another window is refused");
    assert!(
        refused.to_string().contains("another Cox window"),
        "{refused}"
    );

    // Once the first goes, coxd lets it go, and the next is welcome, and
    // is told of the chats again.
    drop(first);
    wait("the first window hears it was let go", || {
        gone.load(Ordering::SeqCst)
    });
    let mut next = None;
    wait("coxd lets the first window go", || {
        next = reach(&socket, &home, &other).ok();
        next.is_some()
    });
    let mut next = next.unwrap();
    assert!(matches!(
        until_settled(&mut next, 2).first(),
        Some(FromDaemon::Listing(_))
    ));

    // coxd stops once its socket is taken away, and the window hears so.
    std::fs::remove_file(&socket).unwrap();
    coxd.join().unwrap().expect("coxd stops as it should");
    wait("the window hears coxd is gone", || {
        other.load(Ordering::SeqCst)
    });
    let _ = std::fs::remove_dir_all(&folder);
}

fn page(url: &str) -> KeptTab {
    KeptTab::Page {
        url: url.to_owned(),
        title: String::new(),
    }
}

#[test]
fn the_chats_of_a_folder_share_its_stage_which_starts_from_their_own() {
    let folder = folder("stage");
    let socket = folder.join("run").join("coxd.sock");
    let home = folder.join("home");
    let project = folder.join("project");
    let path = folder.join("chats.sqlite");
    // Two chats, each with a stage of its own, as an earlier Cox kept
    // them.
    let store = Store::open(&path).unwrap();
    let kept_project = store.project(&project, "project").unwrap();
    let first = store.add_chat(Some(kept_project), "Chat 1").unwrap();
    let second = store.add_chat(Some(kept_project), "Chat 2").unwrap();
    let own = |tabs, shown| KeptTabs {
        tabs,
        shown,
        owners: Vec::new(),
    };
    store
        .keep_tabs(
            first,
            &own(
                vec![KeptTab::Files { open: Vec::new() }, page("https://a/")],
                1,
            ),
        )
        .unwrap();
    store
        .keep_tabs(
            second,
            &own(
                vec![
                    KeptTab::Files { open: Vec::new() },
                    page("https://b/"),
                    KeptTab::Terminal {
                        title: "make".to_owned(),
                    },
                ],
                2,
            ),
        )
        .unwrap();
    let coxd = run_keeping(&socket, &home, Some(&path));
    wait("coxd listens", || socket.exists());
    let gone = Arc::new(AtomicBool::new(false));
    let mut window = reach(&socket, &home, &gone).expect("the window is welcome");
    let heard = until_settled(&mut window, 1);
    let Some(FromDaemon::Listing(listing)) = heard.first() else {
        panic!("coxd lists the chats first: {heard:?}");
    };
    let keys: Vec<u64> = listing.chats.iter().map(|chat| chat.key).collect();
    let [one, two] = keys[..] else {
        panic!("the two chats are listed: {listing:?}");
    };

    // The stage of the project's folder is the two chats' stages, merged,
    // each tab of the chat it was of, the chat watched first.
    let _ = window.link.to.send(ToDaemon::Watch { key: two });
    let told = until_settled(&mut window, 2)
        .into_iter()
        .find_map(|message| match message {
            FromDaemon::Tabs { key, tabs } if key == two => Some(tabs),
            _ => None,
        })
        .expect("coxd tells the stage's tabs");
    let merged = KeptTabs {
        tabs: vec![
            KeptTab::Files { open: Vec::new() },
            page("https://b/"),
            page("https://a/"),
        ],
        shown: 0,
        owners: vec![Some(two), Some(two), Some(one)],
    };
    assert_eq!(told, Some(merged.clone()));

    // The window keeps the stage as it changes, by its folder, and the
    // other chat of the folder gets it as it was kept.
    let changed = KeptTabs { shown: 2, ..merged };
    let _ = window.link.to.send(ToDaemon::Tabs {
        folder: project.clone(),
        tabs: changed.clone(),
    });
    let _ = window.link.to.send(ToDaemon::Watch { key: one });
    let told = until_settled(&mut window, 3)
        .into_iter()
        .find_map(|message| match message {
            FromDaemon::Tabs { key, tabs } if key == one => Some(tabs),
            _ => None,
        })
        .expect("coxd tells the stage's tabs");
    assert_eq!(told, Some(changed));
    let kept = store.stage(&project).unwrap().expect("the stage is kept");
    assert_eq!(kept.owners, vec![Some(second), Some(second), Some(first)]);

    std::fs::remove_file(&socket).unwrap();
    coxd.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_second_coxd_does_not_take_the_socket_of_one_that_runs() {
    let folder = folder("two");
    let socket = folder.join("coxd.sock");
    let coxd = run(&socket, &folder);
    wait("coxd listens", || socket.exists());
    wait("coxd answers", || {
        std::os::unix::net::UnixStream::connect(&socket).is_ok()
    });

    let second = run(&socket, &folder).join().unwrap();
    assert!(second.is_err(), "the second coxd stops at once");

    std::fs::remove_file(&socket).unwrap();
    coxd.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn what_the_window_and_coxd_say_reads_back_as_it_was() {
    let hello = Hello {
        home: Some(PathBuf::from("/tmp/home")),
        command: vec!["node".to_owned(), "agent.mjs".to_owned()],
    };
    let line = serde_json::to_string(&hello).unwrap();
    assert!(!line.contains('\n'));
    assert_eq!(serde_json::from_str::<Hello>(&line).unwrap(), hello);
    for message in [
        FromDaemon::Welcome,
        FromDaemon::Taken,
        FromDaemon::Outdated,
        FromDaemon::Settled { id: 3 },
    ] {
        let line = serde_json::to_string(&message).unwrap();
        assert_eq!(serde_json::from_str::<FromDaemon>(&line).unwrap(), message);
    }
    let told = ToDaemon::Submit {
        key: 1,
        text: "two\nlines".to_owned(),
        attached: vec![Attachment {
            label: "main.rs:2-3".to_owned(),
            uri: "file:///work/src/main.rs#L2-3".to_owned(),
            text: "a\nb".to_owned(),
        }],
    };
    let line = serde_json::to_string(&told).unwrap();
    assert!(!line.contains('\n'), "a message is one line");
    assert_eq!(serde_json::from_str::<ToDaemon>(&line).unwrap(), told);
}
