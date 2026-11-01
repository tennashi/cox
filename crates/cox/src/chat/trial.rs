//! A trial: a chat that tries something out apart from its project,
//! before the human and the agent that opened it agree what the work is
//! to be. Its agent works in a folder of its own, so what it changes
//! leaves the project's files be: a Git worktree of the project's, made
//! from the project's last commit, to try a change to the project; or an
//! empty folder, made a Git repository, to try something from nothing.
//! The chat that opened it reads what it made there, and proposes a goal
//! from it.
//!
//! Cox makes the folder when the chat opens, in a folder of its own for
//! trials (see [`crate::storage`]), and keeps where it is with the chat's
//! setup, so the chat goes on there once it is started again.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::chat::judge;

/// What Cox calls the folder that holds the trials' folders.
pub const FOLDER: &str = "trials";

/// What a trial starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trial {
    /// The project's last commit, in a worktree of the project's.
    Project,
    /// Nothing, in an empty folder.
    Empty,
}

impl Trial {
    /// The trial `name` says, as the agent names it: `project` or
    /// `empty`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "project" => Some(Self::Project),
            "empty" => Some(Self::Empty),
            _ => None,
        }
    }

    /// What the trial's folder is, as the agent that opened it is told.
    #[must_use]
    pub fn what(self) -> &'static str {
        match self {
            Self::Project => "a worktree of the project's last commit",
            Self::Empty => "an empty folder, a Git repository of its own",
        }
    }
}

/// The titles of the permission requests a trial's agent goes on with
/// unasked, on top of what the chat that opened it said: changing files,
/// which are the trial's own, and what a judge may run, which reads and
/// tests.
const ALLOWED: &[&str] = &["Edit *", "Write *", "MultiEdit *", "NotebookEdit *"];

/// The patterns of the titles of the permission requests a trial's agent
/// goes on with unasked.
#[must_use]
pub fn allowed() -> Vec<String> {
    ALLOWED
        .iter()
        .chain(judge::ALLOWED)
        .map(|&title| title.to_owned())
        .collect()
}

/// Makes the folder of a trial that starts from `trial`, for a chat of
/// `project`, in `trials`, and answers where the trial works in it.
///
/// # Errors
///
/// When a trial of the project's is to be made and `project` is not in a
/// Git repository, or when Git cannot make the folder, with what Git
/// said.
pub fn make(trial: Trial, project: &Path, trials: &Path) -> Result<PathBuf, String> {
    match trial {
        Trial::Project => worktree(project, trials),
        Trial::Empty => {
            let folder = free(trials, "scratch")?;
            std::fs::create_dir_all(&folder)
                .map_err(|error| format!("cannot make {}: {error}", folder.display()))?;
            git(&folder, &["init", "--quiet"])?;
            Ok(folder)
        }
    }
}

/// Makes a worktree of the Git repository `project` is in, from its last
/// commit, in `trials`, and answers where the project is in it.
fn worktree(project: &Path, trials: &Path) -> Result<PathBuf, String> {
    let top = git(project, &["rev-parse", "--show-toplevel"])
        .map_err(|error| format!("a trial of the project needs a Git repository: {error}"))?;
    let name = Path::new(top.trim()).file_name().map_or_else(
        || "project".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let folder = free(trials, &name)?;
    let path = folder.to_string_lossy();
    git(project, &["worktree", "add", "--detach", &path, "HEAD"])?;
    // Where in the repository the project is, the trial is too.
    let within = project
        .canonicalize()
        .ok()
        .and_then(|project| {
            let top = Path::new(top.trim()).canonicalize().ok()?;
            project.strip_prefix(top).ok().map(Path::to_owned)
        })
        .unwrap_or_default();
    Ok(if within.as_os_str().is_empty() {
        folder
    } else {
        folder.join(within)
    })
}

/// The first folder in `trials` named `name`, a dash and a number that
/// is not there yet, making `trials` if it is not there.
fn free(trials: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(trials)
        .map_err(|error| format!("cannot make {}: {error}", trials.display()))?;
    (1..=u32::MAX)
        .map(|number| trials.join(format!("{name}-{number}")))
        .find(|folder| !folder.exists())
        .ok_or_else(|| format!("{} is full", trials.display()))
}

/// Runs Git in `folder` with `args`, and answers what it printed, or
/// what it said went wrong.
fn git(folder: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(args)
        .output()
        .map_err(|error| format!("cannot run git: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("cox-trial-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder.canonicalize().unwrap()
    }

    #[test]
    fn a_trial_works_in_a_worktree_of_the_last_commit() {
        let root = scratch("worktree");
        let project = root.join("greet");
        std::fs::create_dir_all(project.join("app")).unwrap();
        let run = |args: &[&str]| git(&project, args).unwrap();
        run(&["init", "-q"]);
        std::fs::write(project.join("app/main.rs"), "fn main() {}\n").unwrap();
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "first",
        ]);
        // Not committed, so not in the trial.
        std::fs::write(project.join("app/main.rs"), "fn main() { todo!() }\n").unwrap();

        let trials = root.join("trials");
        let trial = make(Trial::Project, &project.join("app"), &trials).unwrap();
        assert_eq!(trial, trials.join("greet-1/app"));
        assert_eq!(
            std::fs::read_to_string(trial.join("main.rs")).unwrap(),
            "fn main() {}\n"
        );
        std::fs::write(trial.join("main.rs"), "fn main() { trial() }\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(project.join("app/main.rs")).unwrap(),
            "fn main() { todo!() }\n"
        );
        // Each trial gets a worktree of its own.
        assert_eq!(
            make(Trial::Project, &project, &trials).unwrap(),
            trials.join("greet-2")
        );

        let error = make(
            Trial::Project,
            &root.join("trials"),
            &root.join("elsewhere"),
        );
        assert!(error.is_err_and(|error| error.starts_with("a trial of the project needs")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_trial_from_nothing_works_in_an_empty_repository() {
        let root = scratch("empty");
        // The project need not be in a Git repository.
        let project = root.join("notes");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("todo.md"), "- greet\n").unwrap();

        let trials = root.join("trials");
        let trial = make(Trial::Empty, &project, &trials).unwrap();
        assert_eq!(trial, trials.join("scratch-1"));
        let files: Vec<_> = std::fs::read_dir(&trial)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(files, [".git"]);
        assert_eq!(
            git(&trial, &["rev-parse", "--show-toplevel"])
                .unwrap()
                .trim(),
            trial.to_string_lossy()
        );
        assert_eq!(
            make(Trial::Empty, &project, &trials).unwrap(),
            trials.join("scratch-2")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_trial_is_named_by_what_it_starts_from() {
        assert_eq!(Trial::parse("project"), Some(Trial::Project));
        assert_eq!(Trial::parse("empty"), Some(Trial::Empty));
        assert_eq!(Trial::parse("container"), None);
    }

    #[test]
    fn a_trial_changes_its_files_unasked() {
        let allowed = allowed();
        assert!(allowed.contains(&"Edit *".to_owned()));
        assert!(allowed.contains(&"cargo test*".to_owned()));
    }
}
