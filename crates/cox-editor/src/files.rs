//! The files of a project, folder by folder, as Git sees them: what Git
//! ignores is left out, as the `target` folder of a Rust project is, and
//! so is the `.git` folder itself. Hidden files are not, as
//! `.github/workflows/ci.yml` is not.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// A file or folder in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_folder: bool,
}

/// What is in `folder`, the folders first, then the files, each in order
/// of their names, whatever their case. The `.gitignore` files of the
/// folder and of the folders above it count, even where there is no
/// repository yet.
///
/// # Errors
///
/// When `folder` cannot be read.
pub fn list(folder: &Path) -> std::io::Result<Vec<Entry>> {
    // The walk below says nothing of a folder it cannot read.
    std::fs::read_dir(folder)?;
    let walk = ignore::WalkBuilder::new(folder)
        .max_depth(Some(1))
        .hidden(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    let mut entries: Vec<Entry> = walk
        .filter_map(Result::ok)
        .filter(|entry| entry.depth() == 1)
        .map(|entry| Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            // A link to a folder is a folder, as the human sees it.
            is_folder: entry.path().is_dir(),
            path: entry.into_path(),
        })
        .collect();
    entries.sort_by(order);
    Ok(entries)
}

/// Folders before files, then by name, whatever its case, then by case.
fn order(a: &Entry, b: &Entry) -> Ordering {
    b.is_folder
        .cmp(&a.is_folder)
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for a test, with the files `paths`, empty.
    fn folder(test: &str, paths: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cox-files-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for path in paths {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        root
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn lists_folders_first_then_files_by_name() {
        let root = folder("order", &["b.rs", "A.md", "src/main.rs", "Cargo.toml"]);
        let listed = list(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(names(&listed), ["src", "A.md", "b.rs", "Cargo.toml"]);
        assert!(listed[0].is_folder);
        assert!(!listed[1].is_folder);
        assert_eq!(listed[0].path, root.join("src"));
    }

    #[test]
    fn leaves_out_what_git_ignores_and_git_itself() {
        let root = folder(
            "ignored",
            &[
                ".gitignore",
                ".github/ci.yml",
                ".git/HEAD",
                "target/debug/cox",
                "main.rs",
            ],
        );
        std::fs::write(root.join(".gitignore"), "/target\n").unwrap();
        let listed = list(&root).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(names(&listed), [".github", ".gitignore", "main.rs"]);
    }

    #[test]
    fn a_folder_inside_follows_the_gitignore_above_it() {
        let root = folder(
            "above",
            &[".gitignore", "crate/src/lib.rs", "crate/out/built"],
        );
        std::fs::write(root.join(".gitignore"), "/crate/out\n").unwrap();
        let listed = list(&root.join("crate")).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(names(&listed), ["src"]);
    }

    #[test]
    fn a_folder_that_is_not_there_cannot_be_listed() {
        assert!(list(Path::new("/no/such/folder")).is_err());
    }
}
