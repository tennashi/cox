//! The indexes coxd keeps, in `cache/indexes` where Cox keeps what lasts
//! (see [`crate::storage`]): each of a commit's code, or of a folder of it,
//! written by one indexer at one version, with the text of each file in
//! it, so that a map is drawn from it alone, as the commit has the code,
//! whatever the checkout has since. A commit is the same code wherever it
//! is checked out, so the worktrees of a repository share its indexes.
//!
//! Each is a file of its own, gzipped,
//! `<commit>/<indexer>@<version>.scip.gz`. Only the commits whose indexes
//! were used last are kept, as many as fit (see [`Cache::prune`]).

use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use cox_editor::scip::Index;
use prost::Message as _;

/// How the indexes are written: another way of writing them is kept in
/// another folder, and the folders of the others are taken away.
const FORMAT: &str = "v1";

/// What an index is kept by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    /// The commit indexed, by its full name.
    pub commit: String,
    /// The folder of the commit's code indexed, from the repository's
    /// top, as `web/`, or empty for all of it.
    pub within: String,
    /// The indexer, by name, and its version.
    pub indexer: String,
    pub version: String,
    /// Whether it was made without what the indexer reads from the
    /// checkout when the checkout has it, as `node_modules` (see
    /// [`super::indexers::Indexer::borrows`]), so that it is made again
    /// once the checkout has it.
    pub without: bool,
}

/// The indexes kept in a folder.
#[derive(Clone, Debug)]
pub struct Cache {
    folder: PathBuf,
}

impl Cache {
    /// The indexes kept in `folder`.
    #[must_use]
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// The folder of the indexes of the commit's code `key` names.
    fn commit_folder(&self, key: &Key) -> PathBuf {
        let mut commit = key.commit.clone();
        if !key.within.is_empty() {
            commit.push('+');
            commit.push_str(&key.within);
        }
        self.folder.join(FORMAT).join(name(&commit))
    }

    /// Where the index of `key` is.
    fn file(&self, key: &Key) -> PathBuf {
        let without = if key.without { "-without" } else { "" };
        self.commit_folder(key).join(format!(
            "{}@{}{without}.scip.gz",
            name(&key.indexer),
            name(&key.version)
        ))
    }

    /// Marks the indexes of `key`'s commit used now, for them to be kept
    /// before those used longer ago (see [`Cache::prune`]).
    pub fn used(&self, key: &Key) {
        let marked = fs::File::open(self.commit_folder(key))
            .and_then(|folder| folder.set_modified(SystemTime::now()));
        if let Err(error) = marked {
            tracing::debug!("the use of {} is not marked: {error}", key.commit);
        }
    }

    /// Whether the index of `key` is kept.
    #[must_use]
    pub fn has(&self, key: &Key) -> bool {
        self.file(key).is_file()
    }

    /// The index of `key`, if it is kept and reads: one that is not an
    /// index, as gzip or as SCIP, is taken away, to be made again, but not
    /// one the system could not read now.
    #[must_use]
    pub fn read(&self, key: &Key) -> Option<Index> {
        let file = self.file(key);
        let mut bytes = Vec::new();
        let read = fs::File::open(&file)
            .and_then(|opened| flate2::read::GzDecoder::new(opened).read_to_end(&mut bytes));
        let broken = match read {
            Ok(_) => match Index::decode(bytes.as_slice()) {
                Ok(index) => return Some(index),
                Err(error) => error.to_string(),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidData
                        | io::ErrorKind::InvalidInput
                        | io::ErrorKind::UnexpectedEof
                ) =>
            {
                error.to_string()
            }
            Err(error) => {
                tracing::warn!("{} is not read now: {error}", file.display());
                return None;
            }
        };
        tracing::warn!("{} is no index, and goes: {broken}", file.display());
        let _ = fs::remove_file(&file);
        None
    }

    /// Keeps `index` as `key`'s: whole, so that a reader meanwhile finds
    /// it or not, never part of it.
    ///
    /// # Errors
    ///
    /// When it cannot be written.
    pub fn write(&self, key: &Key, index: &Index) -> io::Result<()> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&index.encode_to_vec())?;
        crate::storage::write_whole(&self.file(key), gz.finish()?)
    }

    /// Takes away all but the indexes of the `keep` commits used last, as
    /// many of them as fit in `most` bytes, the last at least, and those
    /// written in another way.
    pub fn prune(&self, keep: usize, most: u64) {
        let entries = |folder: PathBuf| -> Vec<fs::DirEntry> {
            fs::read_dir(folder)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .collect()
        };
        for other in entries(self.folder.clone()) {
            if other.file_name() != FORMAT {
                let _ = fs::remove_dir_all(other.path());
            }
        }
        let mut commits: Vec<(std::time::SystemTime, PathBuf)> = entries(self.folder.join(FORMAT))
            .into_iter()
            .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
            .collect();
        commits.sort_by(|a, b| b.cmp(a));
        let mut kept = 0;
        for (at, (_, folder)) in commits.into_iter().enumerate() {
            kept += size(&folder);
            if at == 0 || (at < keep && kept <= most) {
                continue;
            }
            if let Err(error) = fs::remove_dir_all(&folder) {
                tracing::warn!("{} stays: {error}", folder.display());
            }
        }
    }
}

/// How many bytes the files in `folder` hold.
fn size(folder: &Path) -> u64 {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| metadata.len())
        .sum()
}

/// `text` as the name of a file, with no separator or `..` in it.
fn name(text: &str) -> String {
    let name: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.chars().all(|c| c == '.') {
        name.replace('.', "_")
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use cox_editor::scip::Document;

    fn key(commit: &str) -> Key {
        Key {
            commit: commit.to_owned(),
            within: String::new(),
            indexer: "fake".to_owned(),
            version: "1.0/x".to_owned(),
            without: false,
        }
    }

    fn index(path: &str) -> Index {
        Index {
            documents: vec![Document {
                relative_path: path.to_owned(),
                text: "fn main() {}\n".to_owned(),
                ..Document::default()
            }],
        }
    }

    #[test]
    fn an_index_is_kept_by_its_commit_and_its_indexers_version() {
        let folder = TempDir::new();
        let cache = Cache::new(folder.to_path_buf());
        assert!(!cache.has(&key("a")));
        cache.write(&key("a"), &index("main.rs")).unwrap();
        assert!(cache.has(&key("a")));
        assert_eq!(cache.read(&key("a")), Some(index("main.rs")));
        for other in [
            Key {
                version: "2".to_owned(),
                ..key("a")
            },
            Key {
                within: "web/".to_owned(),
                ..key("a")
            },
            Key {
                without: true,
                ..key("a")
            },
        ] {
            assert!(!cache.has(&other), "{other:?}");
        }
        cache.write(&key("a"), &index("lib.rs")).unwrap();
        assert_eq!(cache.read(&key("a")), Some(index("lib.rs")));

        // One that is no index goes, to be made again.
        fs::write(cache.file(&key("a")), b"not gzip").unwrap();
        assert_eq!(cache.read(&key("a")), None);
        assert!(!cache.has(&key("a")));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(b"\xff\xff not SCIP").unwrap();
        fs::write(cache.file(&key("a")), gz.finish().unwrap()).unwrap();
        assert_eq!(cache.read(&key("a")), None);
        assert!(!cache.has(&key("a")));

        // One the system cannot read now stays.
        cache.write(&key("a"), &index("main.rs")).unwrap();
        let file = cache.file(&key("a"));
        fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
        assert_eq!(cache.read(&key("a")), None);
        assert!(cache.has(&key("a")));
        fs::set_permissions(&file, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
        assert_eq!(cache.read(&key("a")), Some(index("main.rs")));
    }

    /// Writes the index of each of `commits`, each used a second after
    /// the one before.
    fn written(cache: &Cache, commits: &[&str]) {
        let now = SystemTime::now() - std::time::Duration::from_mins(1);
        for (at, commit) in (0..).zip(commits) {
            cache.write(&key(commit), &index("main.rs")).unwrap();
            fs::File::open(cache.commit_folder(&key(commit)))
                .unwrap()
                .set_modified(now + std::time::Duration::from_secs(at))
                .unwrap();
        }
    }

    #[test]
    fn only_the_commits_used_last_are_kept_as_many_as_fit() {
        let folder = TempDir::new();
        let cache = Cache::new(folder.to_path_buf());
        fs::create_dir_all(folder.join("v0/old")).unwrap();
        written(&cache, &["a", "b", "c"]);
        // Used now, the first indexed is kept before the others.
        cache.used(&key("a"));
        cache.prune(2, u64::MAX);
        assert!(cache.has(&key("a")));
        assert!(!cache.has(&key("b")));
        assert!(cache.has(&key("c")));
        assert!(!folder.join("v0").exists());

        // As many as fit, the last at least.
        cache.prune(2, 1);
        assert!(cache.has(&key("a")));
        assert!(!cache.has(&key("c")));
    }
}
