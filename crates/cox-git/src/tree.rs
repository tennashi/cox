//! The files of a branch of a repository as Git keeps them, in its
//! objects, to read them where no checkout has the branch on disk: the
//! commit the branch is at, what is in each of its folders, and what a
//! file says. Nothing here writes to the repository or to its checkouts.

use std::path::Path;

use crate::git;

/// What an entry of a folder of a commit is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A file, or a link, which Git keeps as the path it links to.
    File,
    /// A folder.
    Folder,
    /// A submodule: a commit of another repository, whose files are not
    /// in this one.
    Submodule,
}

/// An entry of a folder of a commit: its name, and what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
}

/// What a file of a commit says, as Cox can show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    Text(String),
    /// It is not text: it has a NUL byte, or is not UTF-8.
    Binary,
    /// It is larger than the most Cox reads, by its size in bytes.
    TooLarge(u64),
}

/// The commit the branch `branch`, by its short name, of the repository
/// at `repository` is at, by its full name; none when it has no such
/// branch.
///
/// # Errors
///
/// When Git cannot tell, as out of a repository, with what it said.
pub fn commit_of(repository: &Path, branch: &str) -> Result<Option<String>, String> {
    let reference = format!("refs/heads/{branch}");
    if git::run(repository, &["show-ref", "--verify", "--quiet", &reference]).is_err() {
        // Out of a repository, `rev-parse` says why; in one, there is none.
        git::run(repository, &["rev-parse", "--git-dir"])?;
        return Ok(None);
    }
    git::run(
        repository,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )
    .map(Some)
}

/// What is in the folder `folder`, a path from the top of the
/// repository at `repository`, empty for its top, at the commit `commit`.
///
/// # Errors
///
/// When the commit has no such folder, or Git cannot read it, with what
/// it said.
pub fn list(repository: &Path, commit: &str, folder: &Path) -> Result<Vec<Entry>, String> {
    let path = slashed(folder);
    // A submodule's files are in its own repository, even where Git has
    // the commit it names.
    if !path.is_empty() {
        let entry = git::raw(repository, &["ls-tree", "-z", commit, "--", &path])?;
        if entry.starts_with("160000 ") {
            return Err("a submodule, whose files are in a repository of its own".to_owned());
        }
    }
    let tree = format!("{commit}:{path}");
    let printed = git::raw(repository, &["ls-tree", "-z", &tree])?;
    Ok(printed
        .split('\0')
        .filter_map(|line| {
            let (about, name) = line.split_once('\t')?;
            let kind = match about.split(' ').nth(1)? {
                "tree" => Kind::Folder,
                "commit" => Kind::Submodule,
                _ => Kind::File,
            };
            Some(Entry {
                name: name.to_owned(),
                kind,
            })
        })
        .collect())
}

/// What the file `path`, a path from the top of the repository at
/// `repository`, says at the commit `commit`: as text, unless it is not
/// text, or larger than `most` bytes.
///
/// # Errors
///
/// When the commit has no such file, or Git cannot read it, with what it
/// said.
pub fn read(repository: &Path, commit: &str, path: &Path, most: u64) -> Result<Read, String> {
    let object = format!("{commit}:{}", slashed(path));
    let kind = git::run(repository, &["cat-file", "-t", &object])?;
    if kind != "blob" {
        return Err(format!("{} is not a file", path.display()));
    }
    let size: u64 = git::run(repository, &["cat-file", "-s", &object])?
        .parse()
        .map_err(|error| format!("Git told no size: {error}"))?;
    if size > most {
        return Ok(Read::TooLarge(size));
    }
    let bytes = git::bytes(repository, &["cat-file", "blob", &object])?;
    if bytes.iter().take(8000).any(|&byte| byte == 0) {
        return Ok(Read::Binary);
    }
    Ok(String::from_utf8(bytes).map_or(Read::Binary, Read::Text))
}

/// `path` as Git names a path in a tree: its parts with `/` between.
fn slashed(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Folder;

    fn run(folder: &Path, args: &[&str]) {
        git::raw(folder, args).unwrap();
    }

    fn commit(folder: &Path, message: &str) {
        run(folder, &["add", "--all"]);
        run(
            folder,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "-m",
                message,
            ],
        );
    }

    /// A repository on `main`, with a branch `side` no checkout has, which
    /// has `src/lib.rs`, `README.md` and a binary file, `logo.png`.
    fn repository() -> Folder {
        let folder = Folder::new();
        run(&folder, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(folder.join("README.md"), "main\n").unwrap();
        commit(&folder, "Main");
        run(&folder, &["checkout", "--quiet", "-b", "side"]);
        std::fs::create_dir(folder.join("src")).unwrap();
        std::fs::write(folder.join("src/lib.rs"), "pub fn side() {}\n").unwrap();
        std::fs::write(folder.join("README.md"), "side\n").unwrap();
        std::fs::write(folder.join("logo.png"), [0x89, b'P', b'N', b'G', 0, 1]).unwrap();
        commit(&folder, "Side");
        run(&folder, &["checkout", "--quiet", "main"]);
        folder
    }

    #[test]
    fn a_branch_is_at_its_commit_and_none_is_at_nothing() {
        let folder = repository();
        let side = commit_of(&folder, "side").unwrap().unwrap();
        assert_eq!(side.len(), 40);
        assert_eq!(commit_of(&folder, "gone").unwrap(), None);
        assert!(commit_of(&Folder::new(), "side").is_err());
    }

    #[test]
    fn lists_the_folders_of_a_branch_as_git_keeps_them() {
        let folder = repository();
        let side = commit_of(&folder, "side").unwrap().unwrap();
        let kinds = |listed: Vec<Entry>| {
            listed
                .into_iter()
                .map(|entry| (entry.name, entry.kind))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds(list(&folder, &side, Path::new("")).unwrap()),
            [
                ("README.md".to_owned(), Kind::File),
                ("logo.png".to_owned(), Kind::File),
                ("src".to_owned(), Kind::Folder),
            ]
        );
        assert_eq!(
            kinds(list(&folder, &side, Path::new("src")).unwrap()),
            [("lib.rs".to_owned(), Kind::File)]
        );
        assert!(list(&folder, &side, Path::new("nowhere")).is_err());
        // What is on disk is `main`'s, which has no `src`.
        assert!(!folder.join("src").exists());
    }

    #[test]
    fn a_submodule_is_listed_as_such() {
        let folder = repository();
        let commit = commit_of(&folder, "main").unwrap().unwrap();
        // A gitlink, as `git submodule add` puts in the index.
        run(
            &folder,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{commit},vendor"),
            ],
        );
        run(
            &folder,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "-m",
                "Vendor",
            ],
        );
        let main = commit_of(&folder, "main").unwrap().unwrap();
        let listed = list(&folder, &main, Path::new("")).unwrap();
        assert!(listed.contains(&Entry {
            name: "vendor".to_owned(),
            kind: Kind::Submodule
        }));
        let error = list(&folder, &main, Path::new("vendor")).unwrap_err();
        assert!(error.contains("submodule"), "{error}");
    }

    #[test]
    fn reads_a_file_of_a_branch_unless_it_is_not_text_or_too_large() {
        let folder = repository();
        let side = commit_of(&folder, "side").unwrap().unwrap();
        assert_eq!(
            read(&folder, &side, Path::new("src/lib.rs"), 1024).unwrap(),
            Read::Text("pub fn side() {}\n".to_owned())
        );
        assert_eq!(
            read(&folder, &side, Path::new("logo.png"), 1024).unwrap(),
            Read::Binary
        );
        assert_eq!(
            read(&folder, &side, Path::new("src/lib.rs"), 4).unwrap(),
            Read::TooLarge(17)
        );
        assert!(read(&folder, &side, Path::new("src"), 1024).is_err());
        assert!(read(&folder, &side, Path::new("nothing.txt"), 1024).is_err());
        // The file on disk is `main`'s, as it was.
        assert_eq!(
            std::fs::read_to_string(folder.join("README.md")).unwrap(),
            "main\n"
        );
    }
}
