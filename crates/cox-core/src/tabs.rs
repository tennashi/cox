//! What Cox keeps of a stage: the tabs it had open, in their order, which
//! was shown, and which chat each is of. Not what runs in them: a page is
//! kept by its address, a file by its path, the tree of the project's
//! files by the folders open in it, what changed in them as such, the map
//! of the project by where it is open, an Android emulator by the device
//! it runs, a window of another app on the Mac by its app and title, the
//! lists of the devices and of the windows as such, and a terminal only as
//! having been there.
//!
//! A stage is of the folder its chats work in, as a project's, and each
//! tab is of one of those chats: the chat whose agent opened it, or that
//! the human opened it from. An earlier Cox kept a stage for each chat;
//! [`KeptTabs::merged`] makes the stage of a folder of those.

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
    /// tab of no chat. Empty for the tabs of a chat's own stage, as an
    /// earlier Cox kept them.
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

    /// One stage of the stages of several chats, as an earlier Cox kept a
    /// stage for each chat, each with the chat it was of: the tabs of each
    /// in turn, each of its chat, but for the terminals, which do not open
    /// again, and for a tab that shows what one before it shows, as the
    /// project's files, what changed in them, a page or a file, which is
    /// there once, of the first chat that had it. The tab shown is the one
    /// the first stage showed, if it is there.
    #[must_use]
    pub fn merged(stages: Vec<(Of, Self)>) -> Self {
        let mut merged = Self::default();
        let mut first = true;
        for (chat, stage) in stages {
            for (index, tab) in stage.tabs.into_iter().enumerate() {
                if matches!(tab, KeptTab::Terminal { .. } | KeptTab::Parts) {
                    continue;
                }
                let had = merged.tabs.iter().position(|had| had.shows_as(&tab));
                let at = had.unwrap_or_else(|| {
                    merged.tabs.push(tab);
                    merged.owners.push(Some(chat));
                    merged.tabs.len() - 1
                });
                if first && index == stage.shown {
                    merged.shown = at;
                }
            }
            first = false;
        }
        merged
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
    /// The project's files as a tree, with the folders open in it, by
    /// their absolute paths.
    Files { open: Vec<PathBuf> },
    /// What changed in the files of the Git repository of a folder, by
    /// its absolute path, as Git tells it.
    Changes { folder: PathBuf },
    /// The map of the project's crates and modules, open at the crate and
    /// the modules in it `at` names: a tab Cox drew in earlier runs, which
    /// opens again as the page that draws the project's map now, at its
    /// top, the map being in tiers of its own now.
    Map { at: Vec<String> },
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
    /// How the human works with Cox: the files of their workflow and of
    /// the folder, and the settings.
    Workflow,
    /// The parts the chat ran with, and how they reached each other: a
    /// tab Cox drew in earlier runs, and no longer does, so it does not
    /// open again.
    Parts,
}

impl KeptTab {
    /// Whether it shows what `other` shows, so that a stage needs one of
    /// them: the same page, file, folder or device, or, for the views of
    /// the project and the lists, the same kind.
    fn shows_as(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Page { url, .. }, Self::Page { url: other, .. }) => url == other,
            (Self::Markdown { path }, Self::Markdown { path: other })
            | (Self::File { path, .. }, Self::File { path: other, .. }) => path == other,
            (Self::Changes { folder }, Self::Changes { folder: other }) => folder == other,
            (Self::Flow { call }, Self::Flow { call: other }) => call == other,
            (Self::Android { device }, Self::Android { device: other }) => device == other,
            (Self::MacWindow { app, title }, Self::MacWindow { app: a, title: t }) => {
                app == a && title == t
            }
            (Self::Files { .. }, Self::Files { .. })
            | (Self::Map { .. }, Self::Map { .. })
            | (Self::AndroidDevices, Self::AndroidDevices)
            | (Self::MacWindows, Self::MacWindows)
            | (Self::Workflow, Self::Workflow) => true,
            _ => false,
        }
    }
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

    fn page(url: &str) -> KeptTab {
        KeptTab::Page {
            url: url.into(),
            title: String::new(),
        }
    }

    #[test]
    fn the_stages_of_chats_merge_into_one_of_their_folder() {
        let first = KeptTabs {
            tabs: vec![
                KeptTab::Files { open: Vec::new() },
                page("https://a.example/"),
                KeptTab::Terminal {
                    title: "cargo test".into(),
                },
            ],
            shown: 1,
            owners: Vec::new(),
        };
        let second = KeptTabs {
            tabs: vec![
                KeptTab::Files {
                    open: vec!["/p/src".into()],
                },
                page("https://a.example/"),
                page("https://b.example/"),
            ],
            shown: 2,
            owners: Vec::new(),
        };
        let merged = KeptTabs::merged(vec![(1_u64, first), (2, second)]);
        assert_eq!(
            merged,
            KeptTabs {
                tabs: vec![
                    KeptTab::Files { open: Vec::new() },
                    page("https://a.example/"),
                    page("https://b.example/"),
                ],
                shown: 1,
                owners: vec![Some(1), Some(1), Some(2)],
            }
        );
    }

    #[test]
    fn tabs_kept_by_an_earlier_cox_are_of_no_chat_until_merged() {
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
