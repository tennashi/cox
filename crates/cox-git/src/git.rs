//! How Cox runs Git: on the repository of the folder it names, whatever
//! repository the process that started Cox was told of, and asking
//! nothing, since no terminal is there to answer.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

/// Git, to be given its arguments and the folder it runs in, which asks
/// of that folder's repository, not of one an inherited variable names.
/// Under tests it reads neither the developer's settings nor the
/// system's.
#[must_use]
pub fn command() -> Command {
    let mut git = Command::new("git");
    for variable in REPOSITORY_VARIABLES {
        git.env_remove(variable);
    }
    // Tests make their repositories as Git does with no one's settings.
    #[cfg(any(test, feature = "test-support"))]
    git.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    git
}

/// Runs Git in `folder` with `args`, and answers what it printed, trimmed.
///
/// # Errors
///
/// When Git does not start, or fails, with what it wrote on its standard
/// error.
pub fn run(folder: &Path, args: &[impl AsRef<OsStr>]) -> Result<String, String> {
    raw(folder, args).map(|printed| printed.trim().to_owned())
}

/// Runs Git in `folder` with `args`, and answers what it printed, as it
/// printed it, for output whose spaces and line ends count, as a patch's.
///
/// # Errors
///
/// When Git does not start, or fails, with what it wrote on its standard
/// error.
pub fn raw(folder: &Path, args: &[impl AsRef<OsStr>]) -> Result<String, String> {
    let output = command()
        .arg("-C")
        .arg(folder)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Git does not start: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// The variables that name a repository, its index or its objects, by
/// which Git would ask of another repository than the folder's.
const REPOSITORY_VARIABLES: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Folder;

    #[test]
    fn git_answers_what_it_printed_trimmed() {
        let folder = Folder::new();
        run(&folder, &["init", "--quiet"]).unwrap();
        let top = run(&folder, &["rev-parse", "--show-toplevel"]).unwrap();
        assert_eq!(
            Path::new(&top).canonicalize().unwrap(),
            folder.canonicalize().unwrap()
        );
        let raw = raw(&folder, &["rev-parse", "--show-toplevel"]).unwrap();
        assert_eq!(raw, format!("{top}\n"));
    }

    #[test]
    fn git_that_fails_says_why_as_git_said_it() {
        let folder = Folder::new();
        let error = run(&folder, &["rev-parse", "--verify", "HEAD"]).unwrap_err();
        assert!(error.contains("not a git repository"), "{error}");
    }

    #[test]
    fn git_never_waits_on_input() {
        let folder = Folder::new();
        run(&folder, &["init", "--quiet"]).unwrap();
        // With no input it hashes nothing, rather than wait for some.
        let empty = run(&folder, &["hash-object", "--stdin"]).unwrap();
        assert_eq!(empty, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }
}
