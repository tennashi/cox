//! What Cox keeps of a stage: the tabs it had open, in their order, which
//! was shown, and which chat each is of. Not what runs in them: a page is
//! kept by its address, a file by its path, the tab of a file not chosen
//! yet by what the tree beside it shows, what changed in the project's files as such, the map
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
    /// with the lines it was shown at, if any; or, with `branch`, the file
    /// of that path on a branch no checkout has, read from Git, whose
    /// path is as if the repository's own checkout had it.
    File {
        path: PathBuf,
        lines: Option<Lines>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<OnBranch>,
    },
    /// A file not chosen yet, beside the tree of the project's files. The
    /// folders open in the tree, by their absolute paths, are those it
    /// opens with, which a stage kept before the tree was beside the tabs
    /// of files; it keeps none now. The tree shows the directory of the
    /// project the human chose, by its name, if they chose one, in the
    /// checkout they chose of it, as `checkouts` has the folder of each,
    /// by the directory's name, or on the branch no checkout has they
    /// chose of it, as `branches` has its name.
    Files {
        open: Vec<PathBuf>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        checkouts: Vec<(String, PathBuf)>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        branches: Vec<(String, String)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shown: Option<String>,
    },
    /// What changed in the files of the Git repository of a folder, by
    /// its absolute path, as Git tells it, with the checkout of each of
    /// its repositories the human chose to see, if they chose one: by the
    /// name of the project's directory it is, or none for the repository
    /// the folder is in, the checkout's folder; or the branch no checkout
    /// has, by its name, whose commits they chose to see.
    Changes {
        folder: PathBuf,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        checkouts: Vec<(Option<String>, PathBuf)>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        branches: Vec<(Option<String>, String)>,
    },
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
    /// A Mermaid diagram of a reply, by its source.
    Diagram { source: String },
}

/// A branch of a repository, by its short name, whose files Cox reads
/// from Git, as no checkout has it on disk: the repository by the folder
/// of its own checkout.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OnBranch {
    pub repository: PathBuf,
    pub branch: String,
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

    #[test]
    fn a_tab_of_files_keeps_the_folder_chosen() {
        let kept: KeptTab = serde_json::from_str(r#"{"files":{"open":[]}}"#).unwrap();
        assert_eq!(
            kept,
            KeptTab::Files {
                open: Vec::new(),
                checkouts: Vec::new(),
                branches: Vec::new(),
                shown: None,
            }
        );
        assert_eq!(
            serde_json::to_string(&kept).unwrap(),
            r#"{"files":{"open":[]}}"#
        );
        let chosen = KeptTab::Files {
            open: Vec::new(),
            checkouts: vec![("app".to_owned(), PathBuf::from("/p/worktrees/app/b"))],
            branches: vec![("notes".to_owned(), "side".to_owned())],
            shown: Some("app".to_owned()),
        };
        let json = serde_json::to_string(&chosen).unwrap();
        assert_eq!(serde_json::from_str::<KeptTab>(&json).unwrap(), chosen);
    }

    #[test]
    fn a_tab_of_a_file_keeps_the_branch_it_is_from() {
        let json = r#"{"file":{"path":"/p/a.rs","lines":null}}"#;
        let kept: KeptTab = serde_json::from_str(json).unwrap();
        assert_eq!(
            kept,
            KeptTab::File {
                path: PathBuf::from("/p/a.rs"),
                lines: None,
                branch: None,
            }
        );
        assert_eq!(serde_json::to_string(&kept).unwrap(), json);
        let on = KeptTab::File {
            path: PathBuf::from("/p/a.rs"),
            lines: None,
            branch: Some(OnBranch {
                repository: PathBuf::from("/p"),
                branch: "side".to_owned(),
            }),
        };
        let json = serde_json::to_string(&on).unwrap();
        assert_eq!(serde_json::from_str::<KeptTab>(&json).unwrap(), on);
    }

    #[test]
    fn a_tab_of_changes_keeps_the_checkouts_chosen() {
        let kept: KeptTab = serde_json::from_str(r#"{"changes":{"folder":"/p"}}"#).unwrap();
        assert_eq!(
            kept,
            KeptTab::Changes {
                folder: PathBuf::from("/p"),
                checkouts: Vec::new(),
                branches: Vec::new(),
            }
        );
        assert_eq!(
            serde_json::to_string(&kept).unwrap(),
            r#"{"changes":{"folder":"/p"}}"#
        );
        let chosen = KeptTab::Changes {
            folder: PathBuf::from("/p"),
            checkouts: vec![(Some("app".to_owned()), PathBuf::from("/p/worktrees/app/b"))],
            branches: vec![(None, "side".to_owned())],
        };
        let json = serde_json::to_string(&chosen).unwrap();
        assert_eq!(serde_json::from_str::<KeptTab>(&json).unwrap(), chosen);
    }
}
