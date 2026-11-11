//! What Cox keeps of a stage: the tabs it had open, in their order, which
//! was shown, and which chat each is of. Not what runs in them: a page is
//! kept by its address, a file by its path, the tab of a file not chosen
//! yet as such, what changed in the project's files as such, the map
//! of the project by where it is open, an Android emulator by the device
//! it runs, a window of another app on the Mac by its app and title, the
//! lists of the devices and of the windows as such, and a terminal only as
//! having been there.
//!
//! A stage is of the folder its chats work in, as a project's, and each
//! tab is of one of those chats: the chat whose agent opened it, or that
//! the human opened it from.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::store::ChatId;

/// A stage's tabs, each of a chat, as `Of` names the chats: as they are
/// kept, by the numbers the chats are kept by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeptTabs<Of = ChatId> {
    pub tabs: Vec<KeptTab>,
    /// The index of the tab shown.
    pub shown: usize,
    /// The chat each tab is of, in the order of the tabs, or `None` for a
    /// tab of no chat.
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<Option<Of>>,
}

impl<Of> Default for KeptTabs<Of> {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            shown: 0,
            owners: Vec::new(),
        }
    }
}

impl<Of: Copy> KeptTabs<Of> {
    /// The chat the tab at `index` is of, if it is of one.
    #[must_use]
    pub fn owner(&self, index: usize) -> Option<Of> {
        self.owners.get(index).copied().flatten()
    }

    /// The same tabs, each of the chat `to` names for the chat it is of,
    /// or of none when `to` names none.
    pub fn owned<To>(self, to: impl Fn(Of) -> Option<To>) -> KeptTabs<To> {
        KeptTabs {
            owners: self
                .owners
                .into_iter()
                .map(|owner| owner.and_then(&to))
                .collect(),
            tabs: self.tabs,
            shown: self.shown,
        }
    }
}

/// One tab, by what it shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeptTab {
    /// A page of the browser.
    Page { url: String, title: String },
    /// A Markdown file, by its absolute path.
    Markdown { path: PathBuf },
    /// A terminal, with the title it last had.
    Terminal { title: String },
    /// A file shown as it is written, such as code, by its absolute path,
    /// with the lines it was shown at, if any.
    File { path: PathBuf, lines: Option<Lines> },
    /// A file not chosen yet, beside the tree of the project's files. The
    /// folders open in the tree, by their absolute paths, are those it
    /// opens with, which a stage kept before the tree was beside the tabs
    /// of files; it keeps none now.
    Files { open: Vec<PathBuf> },
    /// What changed in the files of the Git repository of a folder, by
    /// its absolute path, as Git tells it.
    Changes { folder: PathBuf },
    /// The flow of one tool call through Cox, the call by the id the
    /// agent gave it.
    Flow { call: String },
    /// An Android emulator, by the virtual device it runs, as `emulator
    /// -avd` takes its name.
    Android { device: String },
    /// The Android emulator's virtual devices, listed.
    AndroidDevices,
    /// A window of another app on the Mac, by its app's name and its
    /// title, as it had them.
    MacWindow { app: String, title: String },
    /// The windows of the Mac's other apps, listed.
    MacWindows,
}

/// Lines of a file, as people number them: from 1, the last included.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lines {
    pub first: usize,
    pub last: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_is_of_the_chat_its_owner_names() {
        let kept: KeptTabs<u64> =
            serde_json::from_str(r#"{"tabs":[{"markdown":{"path":"/p/a.md"}}],"shown":0}"#)
                .unwrap();
        assert_eq!(kept.owner(0), None);
        let owned = KeptTabs {
            owners: vec![Some(7_u64)],
            ..kept
        };
        let json = serde_json::to_string(&owned).unwrap();
        assert!(json.contains(r#""owners":[7]"#), "{json}");
        assert_eq!(owned.owned(|chat| Some(chat + 1)).owner(0), Some(8));
    }
}
