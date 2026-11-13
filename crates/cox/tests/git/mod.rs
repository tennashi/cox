//! Git, run as a test's: apart from the human's own settings, and from a
//! repository the environment names, as when the tests run in a hook.

use std::path::Path;
use std::process::Command;

/// Runs Git with `args` in `folder`, as a test's, and answers what it
/// printed; the test fails unless it succeeds.
pub fn git(folder: &Path, args: &[&str]) -> String {
    let ran = Command::new("git")
        .arg("-C")
        .arg(folder)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(["-c", "init.defaultBranch=main"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .unwrap();
    assert!(
        ran.status.success(),
        "git {args:?} in {}: {ran:?}",
        folder.display()
    );
    String::from_utf8_lossy(&ran.stdout).into_owned()
}
