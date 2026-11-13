//! A folder of a test's own among the system's temporary files, taken
//! away as it drops, even when the test fails.

use std::path::{Path, PathBuf};

pub struct Scratch(PathBuf);

impl Scratch {
    /// An empty folder named after `label`, which no other test uses.
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cox-android-{label}-{}", std::process::id()));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).expect("a scratch folder");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
