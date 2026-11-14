//! Folders of a test's own: each unique to it among the tests that run
//! beside it, and removed with all in it as the test ends, even when it
//! fails, as its thread ends.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A folder that goes, with all in it, when this is dropped.
struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

thread_local! {
    /// The folders of the test on this thread.
    static FOLDERS: RefCell<Vec<Removed>> = const { RefCell::new(Vec::new()) };
    /// The folder of what Cox keeps for the test on this thread.
    static DATA: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// A folder named after `name` that no other test has, not made yet,
/// which goes as the test ends.
pub fn folder(name: &str) -> PathBuf {
    static MADE: AtomicUsize = AtomicUsize::new(0);
    let made = MADE.fetch_add(1, Ordering::Relaxed);
    let folder = std::env::temp_dir().join(format!("cox-{name}-{}-{made}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    FOLDERS.with_borrow_mut(|folders| folders.push(Removed(folder.clone())));
    folder
}

/// A folder like [`folder`]'s, made.
pub fn made(name: &str) -> PathBuf {
    let folder = folder(name);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// The folder Cox keeps what lasts in, its settings and state, for the
/// test on this thread and the daemons it starts, made the first time it
/// is asked for, with a trash of its own in it: apart from the other
/// tests' and from the human's.
pub fn data() -> PathBuf {
    DATA.with_borrow_mut(|data| {
        data.get_or_insert_with(|| {
            let data = made("data");
            coxd::storage::keep_on_this_thread(Some(data.clone()));
            coxd::trash::keep_on_this_thread(Some(data.join("Trash")));
            data
        })
        .clone()
    })
}
