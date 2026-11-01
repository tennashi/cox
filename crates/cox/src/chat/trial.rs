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
//!
//! Once the human has what they wanted of it, the trial ends: a trial of
//! the project's is kept as a branch of the project's repository, with
//! what it changed, or a trial from nothing is opened as a project of its
//! own; or the trial is discarded, its folder with it. The chat goes on,
//! in the project.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::chat::judge;

/// What Cox calls the folder that holds the trials' folders.
pub const FOLDER: &str = "trials";

/// What a trial starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
    if let Err(error) = git(project, &["worktree", "add", "--detach", &path, "HEAD"]) {
        let _ = std::fs::remove_dir(&folder);
        return Err(error);
    }
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

/// Keeps what the trial of the project's that works in `folder` made as
/// a branch of the project's repository, `trial/` and the name of its
/// folder, with what it changed committed as `title` says, and takes its
/// worktree away; and answers the branch's name.
///
/// # Errors
///
/// When Git cannot commit what the trial changed, make the branch, or
/// take the worktree away, with what Git said.
pub fn keep(folder: &Path, title: &str) -> Result<String, String> {
    let top = top(folder)?;
    git(&top, &["add", "--all"])?;
    if !git(&top, &["status", "--porcelain"])?.trim().is_empty() {
        let message = format!("Trial: {title}");
        git(&top, &["commit", "--quiet", "--no-verify", "-m", &message])?;
    }
    let name = top.file_name().map_or_else(
        || "trial".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let branch = (1..=u32::MAX)
        .map(|number| match number {
            1 => format!("trial/{name}"),
            _ => format!("trial/{name}-{number}"),
        })
        .find(|branch| {
            git(
                &top,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch}"),
                ],
            )
            .is_err()
        })
        .ok_or_else(|| "there is no name left for the branch".to_owned())?;
    git(&top, &["branch", &branch])?;
    remove_worktree(&top)?;
    Ok(branch)
}

/// Discards the trial that starts from `trial` and works in `folder`,
/// taking its folder away.
///
/// # Errors
///
/// When the folder cannot be taken away, with why.
pub fn discard(trial: Trial, folder: &Path) -> Result<(), String> {
    match trial {
        Trial::Project => remove_worktree(&top(folder)?),
        Trial::Empty => std::fs::remove_dir_all(folder)
            .map_err(|error| format!("cannot remove {}: {error}", folder.display())),
    }
}

/// The top of the worktree `folder` is in.
fn top(folder: &Path) -> Result<PathBuf, String> {
    Ok(PathBuf::from(
        git(folder, &["rev-parse", "--show-toplevel"])?.trim(),
    ))
}

/// Takes away the worktree at `top`, with what is in it, from the
/// repository it is a worktree of.
fn remove_worktree(top: &Path) -> Result<(), String> {
    // Git takes a worktree away from another one, not from itself.
    let common = git(
        top,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let path = top.to_string_lossy();
    git(
        Path::new(common.trim()),
        &["worktree", "remove", "--force", &path],
    )
    .map(drop)
}

/// Makes the first folder in `trials` named `name`, a dash and a number
/// that is not there yet, making `trials` if it is not there, and
/// answers where it is. Making it claims it, so two trials made at once
/// get folders of their own.
fn free(trials: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(trials)
        .map_err(|error| format!("cannot make {}: {error}", trials.display()))?;
    for number in 1..=u32::MAX {
        let folder = trials.join(format!("{name}-{number}"));
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("cannot make {}: {error}", folder.display())),
        }
    }
    Err(format!("{} is full", trials.display()))
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

    /// A repository at `project`, with `main.rs` committed.
    fn repository(project: &Path) {
        std::fs::create_dir_all(project).unwrap();
        let run = |args: &[&str]| git(project, args).unwrap();
        run(&["init", "-q"]);
        run(&["config", "user.name", "t"]);
        run(&["config", "user.email", "t@t"]);
        std::fs::write(project.join("main.rs"), "fn main() {}\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "first"]);
    }

    #[test]
    fn a_trial_is_kept_as_a_branch_with_what_it_changed() {
        let root = scratch("keep");
        let project = root.join("greet");
        repository(&project);
        let trials = root.join("trials");
        let trial = make(Trial::Project, &project, &trials).unwrap();
        std::fs::write(trial.join("main.rs"), "fn main() { trial() }\n").unwrap();
        std::fs::write(trial.join("new.rs"), "fn trial() {}\n").unwrap();

        assert_eq!(keep(&trial, "Try a greeting").unwrap(), "trial/greet-1");
        assert!(!trial.exists());
        let shown = |what: &str| git(&project, &["show", what]).unwrap();
        assert_eq!(shown("trial/greet-1:main.rs"), "fn main() { trial() }\n");
        assert_eq!(shown("trial/greet-1:new.rs"), "fn trial() {}\n");
        assert_eq!(
            git(&project, &["log", "-1", "--format=%s", "trial/greet-1"])
                .unwrap()
                .trim(),
            "Trial: Try a greeting"
        );
        // The project's own files stay as they were.
        assert_eq!(
            std::fs::read_to_string(project.join("main.rs")).unwrap(),
            "fn main() {}\n"
        );

        // A trial that changed nothing is kept too, under a name of its
        // own when the first is taken.
        let again = make(Trial::Project, &project, &trials).unwrap();
        assert_eq!(again, trials.join("greet-1"));
        assert_eq!(keep(&again, "Again").unwrap(), "trial/greet-1-2");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_trial_is_discarded_with_its_folder() {
        let root = scratch("discard");
        let project = root.join("greet");
        repository(&project);
        let trials = root.join("trials");
        let trial = make(Trial::Project, &project, &trials).unwrap();
        std::fs::write(trial.join("main.rs"), "fn main() { trial() }\n").unwrap();
        discard(Trial::Project, &trial).unwrap();
        assert!(!trial.exists());
        let worktrees = git(&project, &["worktree", "list"]).unwrap();
        assert_eq!(worktrees.lines().count(), 1, "{worktrees}");
        assert_eq!(git(&project, &["branch", "--list", "trial/*"]).unwrap(), "");

        let empty = make(Trial::Empty, &project, &trials).unwrap();
        std::fs::write(empty.join("main.rs"), "fn main() {}\n").unwrap();
        discard(Trial::Empty, &empty).unwrap();
        assert!(!empty.exists());
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
