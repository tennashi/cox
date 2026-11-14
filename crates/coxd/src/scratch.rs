//! What Cox keeps only while it runs, as the sockets of the chats' tool
//! servers: a folder of each run's own among the system's temporary
//! files, `cox-<pid>`, which only the human may enter. The stage keeps a
//! throwaway profile there too (see `cox_browser::profile`).
//!
//! A Cox that ends as it should takes away what it made there; one that
//! does not, as when it crashes, leaves its folder, which the next Cox to
//! start takes away (see [`sweep`]).

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};
use std::path::{Path, PathBuf};

/// The folder of this run, made the first time it is asked for.
///
/// # Errors
///
/// When it cannot be made.
pub fn folder() -> io::Result<PathBuf> {
    let folder = folder_of(&std::env::temp_dir(), std::process::id());
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)?;
    Ok(folder)
}

/// Takes away the folder of this run, as Cox quits.
pub fn take_away() {
    let folder = folder_of(&std::env::temp_dir(), std::process::id());
    match fs::remove_dir_all(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!("{} stays: {error}", folder.display()),
    }
}

/// The folder of the run of `pid` among the temporary files in `temp`.
fn folder_of(temp: &Path, pid: u32) -> PathBuf {
    temp.join(format!("cox-{pid}"))
}

/// Takes away the folders that the runs of Cox that ended left among the
/// system's temporary files, as `cox-<pid>`. Only the human's folders
/// are taken, of processes that no longer run.
pub fn sweep() {
    sweep_in(&std::env::temp_dir(), &|pid| running(pid));
}

/// Takes away the folders in `temp` that runs left, as [`sweep`] says,
/// whose process `runs` says no longer runs.
fn sweep_in(temp: &Path, runs: &dyn Fn(u32) -> bool) {
    let Ok(entries) = fs::read_dir(temp) else {
        return;
    };
    // SAFETY: `getuid` has no preconditions and cannot fail.
    let me = unsafe { libc::getuid() };
    for entry in entries.filter_map(Result::ok) {
        let Some(pid) = entry.file_name().to_str().and_then(left_by) else {
            continue;
        };
        let path = entry.path();
        let mine = fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_dir() && metadata.uid() == me);
        if !mine || pid == std::process::id() || runs(pid) {
            continue;
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => tracing::info!("took away {}, left by Cox {pid}", path.display()),
            Err(error) => tracing::warn!("{} stays: {error}", path.display()),
        }
    }
}

/// The process whose run of Cox left a folder named `name`, if a run of
/// Cox names its folders so.
fn left_by(name: &str) -> Option<u32> {
    let pid = name.strip_prefix("cox-")?;
    (!pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| pid.parse().ok())
        .flatten()
}

/// Whether the process `pid` runs.
fn running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the process is there.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    // One of another user's is there too.
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn tells_the_folders_runs_of_cox_leave() {
        assert_eq!(left_by("cox-4242"), Some(4242));
        // What the tests of Cox make, and others.
        assert_eq!(left_by("cox-settings-4242"), None);
        assert_eq!(left_by("cox-4242-screenshot.png"), None);
        assert_eq!(left_by("cox-4242-3"), None);
        assert_eq!(left_by("cox-"), None);
        assert_eq!(left_by("box-4242"), None);
    }

    #[test]
    fn takes_away_only_what_runs_that_ended_left() {
        let temp = TempDir::new();
        for name in ["cox-1", "cox-2", "cox-mine-1"] {
            fs::create_dir_all(temp.join(name).join("inside")).unwrap();
        }
        fs::write(temp.join("cox-3"), "a file").unwrap();
        let this = folder_of(&temp, std::process::id());
        fs::create_dir_all(&this).unwrap();

        sweep_in(&temp, &|pid| pid == 2);

        let mut left: Vec<_> = fs::read_dir(&temp)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        let mut expected = vec![
            "cox-2".to_owned(),
            "cox-3".to_owned(),
            "cox-mine-1".to_owned(),
            format!("cox-{}", std::process::id()),
        ];
        expected.sort();
        assert_eq!(left, expected);
    }

    #[test]
    fn a_pid_runs_only_while_its_process_lives() {
        assert!(running(std::process::id()));
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        assert!(running(child.id()), "a child that sleeps runs");
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(
            !running(child.id()),
            "a child that ended and was waited for"
        );
    }
}
