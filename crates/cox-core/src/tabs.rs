//! What Cox keeps of a chat's stage: the tabs it had open, in their
//! order, and which was shown. Not what runs in them: a page is kept by its
//! address, a Markdown file by its path, and a terminal only as having
//! been there.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A stage's tabs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tabs {
    pub tabs: Vec<Tab>,
    /// The index of the tab shown.
    pub shown: usize,
}

/// One tab, by what it shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tab {
    /// A page of the browser.
    Page { url: String, title: String },
    /// A Markdown file, by its absolute path.
    Markdown { path: PathBuf },
    /// A terminal, with the title it last had.
    Terminal { title: String },
}
