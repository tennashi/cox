//! Following a file as it changes, for a tab that shows it: the tab looks
//! now and then whether the file changed, and reads it again when it did,
//! so that the human reads what the agent writes as it writes it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gpui::{Context, Task};

/// How often a tab looks whether its file changed.
const LOOK_EVERY: Duration = Duration::from_millis(500);

/// When a file last changed, and how long it was, as a tab last read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    modified: SystemTime,
    len: u64,
}

impl Stamp {
    /// When the file at `path` last changed, and how long it is.
    ///
    /// # Errors
    ///
    /// When the file cannot be looked at, as when there is none.
    pub fn of(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            modified: metadata.modified()?,
            len: metadata.len(),
        })
    }
}

/// A tab that shows a file and follows it.
pub trait Follows: Sized + 'static {
    /// The file, and its stamp as the tab last read it, or `None` until the
    /// tab looks at the file.
    fn followed(&self) -> (PathBuf, Option<Stamp>);

    /// The tab looked at the file, which changed since it last did: it
    /// says this now, or could not be read.
    fn looked(&mut self, looked: std::io::Result<(Stamp, String)>, cx: &mut Context<Self>);
}

/// Follows the file that the tab of `cx` shows, for as long as the tab is.
pub fn follow<T: Follows>(cx: &mut Context<T>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(LOOK_EVERY).await;
            let Ok((path, stamp)) = this.read_with(cx, |tab, _| tab.followed()) else {
                break;
            };
            let changed = cx
                .background_executor()
                .spawn(async move { read_if_changed(&path, stamp) })
                .await;
            if let Some(changed) = changed
                && this.update(cx, |tab, cx| tab.looked(changed, cx)).is_err()
            {
                break;
            }
        }
    })
}

/// What the file at `path` says, with its stamp, if it changed since
/// `stamp`, or why it could not be read.
fn read_if_changed(path: &Path, stamp: Option<Stamp>) -> Option<std::io::Result<(Stamp, String)>> {
    let now = match Stamp::of(path) {
        Ok(now) => now,
        Err(error) => return Some(Err(error)),
    };
    if stamp == Some(now) {
        return None;
    }
    Some(std::fs::read_to_string(path).map(|text| (now, text)))
}
