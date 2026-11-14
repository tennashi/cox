//! When a folder is taken: a name the human gives a project, or a
//! directory in it, can be another spelling of a folder there already.
//!
//! Which project is at a folder is another question. A project's folder
//! is the path Cox wrote for it, a name joined onto the home, and Cox
//! finds the project by that path as it is (`Path`'s `==`), as the store
//! finds it by the text it kept, which is the same for every path Cox
//! writes. A project the human deleted is at no folder.

use std::path::Path;

/// Whether a folder other than `from` is at `to` already, so that `from`
/// cannot be renamed `to`. A name that differs only in case is the same
/// folder where the disk does not tell cases apart, as on macOS, and so
/// is a link to `from`.
#[must_use]
pub fn taken(to: &Path, from: &Path) -> bool {
    to.exists() && to.canonicalize().ok() != from.canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use super::taken;

    #[test]
    fn a_folder_is_not_taken_by_itself() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("shop");
        std::fs::create_dir(&folder).unwrap();

        assert!(!taken(&folder, &folder));
        assert!(!taken(&root.path().join("shop/"), &folder));
    }

    #[test]
    fn a_link_to_a_folder_is_that_folder() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("shop");
        std::fs::create_dir(&folder).unwrap();
        let link = root.path().join("store");
        std::os::unix::fs::symlink(&folder, &link).unwrap();

        assert!(!taken(&link, &folder));
    }

    #[test]
    fn another_folder_takes_its_name() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("shop");
        let other = root.path().join("store");
        std::fs::create_dir(&folder).unwrap();
        std::fs::create_dir(&other).unwrap();

        assert!(taken(&other, &folder));
        assert!(!taken(&root.path().join("market"), &folder));
    }
}
