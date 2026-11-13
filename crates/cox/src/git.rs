//! How Cox runs Git: on the repository of the folder it names, whatever
//! repository the process that started Cox was told of.

use std::process::Command;

/// Git, to be given its arguments and the folder it runs in, which asks
/// of that folder's repository, not of one an inherited variable names.
/// Under the unit tests it reads neither the developer's settings nor the
/// system's.
#[must_use]
pub(crate) fn command() -> Command {
    let mut git = Command::new("git");
    for variable in REPOSITORY_VARIABLES {
        git.env_remove(variable);
    }
    // Tests make their repositories as Git does with no one's settings.
    #[cfg(test)]
    git.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    git
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
