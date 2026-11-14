//! The directories of a project: folders of their own in `directories`
//! in its folder, each by its name, which the human makes there, or has
//! Git clone there, so that the rest of the project's folder is the
//! human's to put anything in. A directory is a Git repository or not,
//! and a repository is linked to a remote, as `origin`, or not. A
//! directory named as its project is renamed with it, until the human
//! names it otherwise.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub use crate::settings::DIRECTORIES;

/// A directory of a project, as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    /// Its name, its folder's.
    pub name: String,
    /// Whether it is a Git repository.
    pub git: bool,
    /// The URL of the remote it is linked to, `origin`, if it is.
    pub remote: Option<String>,
}

/// How a repository was linked to its remote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Linked {
    /// The remote is its `origin` now, and nothing else changed.
    Added,
    /// The repository had nothing yet, so it took the remote's branch
    /// of this name, as a clone would have.
    CheckedOut(String),
}

/// The folder of the directory `name` of the project in `project`.
#[must_use]
pub fn folder_of(project: &Path, name: &str) -> PathBuf {
    project.join(DIRECTORIES).join(name)
}

/// The name of the repository `url` names, as `git clone` names the
/// folder it clones into: the last part of its path, without `.git`, as
/// `cox` for `https://github.com/tennashi/cox.git` or
/// `git@github.com:tennashi/cox.git`; or `None` when it names none a
/// folder can take.
#[must_use]
pub fn name_of(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let last = url.rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    why_not_named(name).is_none().then(|| name.to_owned())
}

/// Whether `typed` names a repository to clone, as a URL or a path does,
/// rather than the name of a directory to make.
#[must_use]
pub fn is_url(typed: &str) -> bool {
    typed.contains(['/', ':'])
}

/// Why a directory cannot be named `name`, if it cannot.
#[must_use]
pub fn why_not_named(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("A directory needs a name.");
    }
    if name.starts_with('.') || name.contains(['/', '\\', ':']) {
        return Some("The name is its folder's: no / or : in it, nor a . first.");
    }
    None
}

/// The directories of the project in `project`, by name.
#[must_use]
pub fn listed(project: &Path) -> Vec<Directory> {
    let Ok(entries) = std::fs::read_dir(project.join(DIRECTORIES)) else {
        return Vec::new();
    };
    let mut directories: Vec<Directory> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let folder = entry.path();
            let git = folder.join(".git").exists();
            let remote = git
                .then(|| run(&folder, &["remote", "get-url", "origin"]).ok())
                .flatten();
            Some(Directory { name, git, remote })
        })
        .collect();
    directories.sort_by(|a, b| a.name.cmp(&b.name));
    directories
}

/// Renames the directory `from` of the project in `project` `to`,
/// unless the project has a directory of that name already, or `to` is
/// no folder's name.
pub fn rename(project: &Path, from: &str, to: &str) -> Result<(), String> {
    if let Some(why) = why_not_named(to) {
        return Err(why.to_owned());
    }
    let (from, to) = (folder_of(project, from), folder_of(project, to));
    if to == from {
        return Ok(());
    }
    if cox_core::folder::taken(&to, &from) {
        return Err("The project has a directory of that name already.".to_owned());
    }
    std::fs::rename(&from, &to).map_err(|error| format!("The directory is not renamed: {error}"))
}

/// Makes the directory `into`, which is not there yet.
pub fn make(into: &Path) -> Result<(), String> {
    std::fs::create_dir_all(into).map_err(|error| format!("The directory is not made: {error}"))
}

/// Clones the repository `url` into `into`, which is not there yet,
/// asking nothing, since no terminal is there to answer: a repository
/// that needs a login Git has not got fails. Answers why it failed, as
/// Git says it last.
pub fn clone(url: &str, into: &Path) -> Result<(), String> {
    let parent = into.parent().unwrap_or(into);
    make(parent)?;
    let args = [
        OsStr::new("clone"),
        OsStr::new("--"),
        OsStr::new(url),
        into.as_os_str(),
    ];
    run(parent, &args).map(drop)
}

/// Makes the directory `folder` a Git repository, with no remote yet.
pub fn init(folder: &Path) -> Result<(), String> {
    run(folder, &["init", "--quiet"]).map(drop)
}

/// Links the repository `folder` to the remote `url`, as its `origin`;
/// a repository with nothing in it yet fetches the remote and takes its
/// branch, as a clone would have.
pub fn link(folder: &Path, url: &str) -> Result<Linked, String> {
    if run(folder, &["remote", "get-url", "origin"]).is_ok() {
        run(folder, &["remote", "set-url", "origin", url])?;
    } else {
        run(folder, &["remote", "add", "origin", url])?;
    }
    if run(folder, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok() {
        return Ok(Linked::Added);
    }
    run(folder, &["fetch", "--quiet", "origin"])?;
    // A remote with no branch yet has none to take.
    if run(folder, &["remote", "set-head", "origin", "--auto"]).is_err() {
        return Ok(Linked::Added);
    }
    let head = run(
        folder,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )?;
    let branch = head.strip_prefix("origin/").unwrap_or(&head).to_owned();
    run(
        folder,
        &["checkout", "--quiet", "-B", &branch, "--track", &head],
    )?;
    Ok(Linked::CheckedOut(branch))
}

/// Runs Git with `args` in `folder`, asking nothing, and answers what it
/// printed, or why it failed, as Git says it last: the line the human
/// reads.
fn run(folder: &Path, args: &[impl AsRef<OsStr>]) -> Result<String, String> {
    cox_editor::git::run(folder, args).map_err(|said| {
        said.lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or("Git failed, saying nothing.")
            .to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TempDir, run_git};

    #[test]
    fn a_repository_is_named_as_git_clone_names_its_folder() {
        for (url, name) in [
            ("https://github.com/tennashi/cox.git", Some("cox")),
            ("https://github.com/tennashi/cox", Some("cox")),
            ("https://github.com/tennashi/cox/", Some("cox")),
            ("git@github.com:tennashi/cox.git", Some("cox")),
            ("ssh://git@host:22/team/app.git", Some("app")),
            ("/home/me/src/lib", Some("lib")),
            ("", None),
            ("https://github.com/tennashi/.git", None),
        ] {
            assert_eq!(name_of(url).as_deref(), name, "{url}");
        }
    }

    #[test]
    fn a_url_or_a_path_is_cloned_and_a_name_is_made() {
        assert!(is_url("https://github.com/tennashi/cox.git"));
        assert!(is_url("git@github.com:tennashi/cox.git"));
        assert!(is_url("/home/me/src/lib"));
        assert!(!is_url("app"));
    }

    /// A repository at `folder` with one commit, on `main`.
    fn origin(folder: &Path) {
        make(folder).unwrap();
        run_git(folder, &["init", "--quiet", "--initial-branch=main"]).unwrap();
        run_git(
            folder,
            &["commit", "--quiet", "--allow-empty", "-m", "first"],
        )
        .unwrap();
    }

    /// A repository with one commit on `main`, and the folder of a project
    /// beside it, in a folder of the test's own.
    fn remote_and_project() -> (TempDir, String, PathBuf) {
        let root = TempDir::new();
        let remote = root.join("origin");
        origin(&remote);
        let url = remote.to_str().unwrap().to_owned();
        let project = root.join("project");
        (root, url, project)
    }

    /// A Git repository with nothing in it yet, made at `folder`.
    fn empty_repository(folder: &Path) {
        make(folder).unwrap();
        init(folder).unwrap();
    }

    #[test]
    fn the_directories_made_and_cloned_are_listed_with_their_remotes() {
        let (_root, url, project) = remote_and_project();
        make(&project.join("notes")).unwrap();

        clone(&url, &folder_of(&project, "origin")).unwrap();
        make(&folder_of(&project, "app")).unwrap();

        assert_eq!(
            listed(&project),
            [
                Directory {
                    name: "app".to_owned(),
                    git: false,
                    remote: None
                },
                Directory {
                    name: "origin".to_owned(),
                    git: true,
                    remote: Some(url)
                },
            ],
            "the folders out of the project's directories are not listed"
        );
    }

    #[test]
    fn a_clone_of_a_repository_that_is_not_there_fails_saying_why() {
        let (root, _url, project) = remote_and_project();

        let failed = clone(
            root.join("nowhere").to_str().unwrap(),
            &folder_of(&project, "nowhere"),
        );

        assert!(failed.is_err_and(|why| !why.is_empty()));
    }

    #[test]
    fn a_repository_with_nothing_in_it_takes_the_remotes_branch_when_linked() {
        let (_root, url, project) = remote_and_project();
        let app = folder_of(&project, "app");
        empty_repository(&app);
        assert_eq!(listed(&project)[0].remote, None);

        assert_eq!(link(&app, &url), Ok(Linked::CheckedOut("main".to_owned())));

        assert_eq!(listed(&project)[0].remote.as_deref(), Some(url.as_str()));
        assert!(run_git(&app, &["rev-parse", "--verify", "HEAD"]).is_ok());
    }

    #[test]
    fn a_repository_with_commits_of_its_own_only_gets_the_remote_when_linked() {
        let (_root, url, project) = remote_and_project();
        let lib = folder_of(&project, "lib");
        origin(&lib);

        assert_eq!(link(&lib, &url), Ok(Linked::Added));

        assert_eq!(listed(&project)[0].remote.as_deref(), Some(url.as_str()));
    }

    #[test]
    fn a_repository_linked_to_an_empty_remote_is_left_empty() {
        let (root, _url, project) = remote_and_project();
        let empty = root.join("empty");
        empty_repository(&empty);
        let fresh = folder_of(&project, "fresh");
        empty_repository(&fresh);

        assert_eq!(link(&fresh, empty.to_str().unwrap()), Ok(Linked::Added));

        assert!(run_git(&fresh, &["rev-parse", "--verify", "HEAD"]).is_err());
    }

    #[test]
    fn a_directory_is_renamed_with_what_it_holds() {
        let (_root, _url, project) = remote_and_project();
        let fresh = folder_of(&project, "fresh");
        empty_repository(&fresh);

        assert_eq!(rename(&project, "fresh", "fresher"), Ok(()));

        assert!(!fresh.exists());
        assert!(folder_of(&project, "fresher").join(".git").exists());
    }

    #[test]
    fn a_directory_is_not_renamed_over_another() {
        let (_root, _url, project) = remote_and_project();
        empty_repository(&folder_of(&project, "fresh"));
        empty_repository(&folder_of(&project, "lib"));

        assert!(rename(&project, "fresh", "lib").is_err());

        assert!(folder_of(&project, "fresh").exists());
    }

    #[test]
    fn a_directory_is_not_renamed_out_of_the_projects_directories() {
        let (_root, _url, project) = remote_and_project();
        empty_repository(&folder_of(&project, "fresh"));

        assert!(rename(&project, "fresh", "../out").is_err());

        assert!(folder_of(&project, "fresh").exists());
        assert!(!project.join("out").exists());
    }
}
