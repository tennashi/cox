//! The environment of the human's login shell, for a Cox the system
//! opened, as from the Finder, the Dock or a launcher such as Raycast.
//!
//! macOS starts such an app with launchd's environment, not the shell's:
//! its `PATH` is `/usr/bin:/bin:/usr/sbin:/sbin`, so the agent's program,
//! as `npx`, is not found, and what the human exports in their shell's
//! profile, as `CLAUDE_CODE_ENABLE_TODO_TOOLS`, does not reach the agent.
//! So the window asks the login shell what its environment is, as the
//! shell the human types in has it, and takes it in, as Zed and VS Code
//! do, giving the shell a few seconds, since the human waits on the window.
//!
//! coxd, which launchd starts as the human logs in, while the Mac is busy
//! and a shell may take a while, starts again from the login shell instead
//! (see [`start_from_login_shell`]), as a launchd service wrapped in a
//! shell does: it has the shell's environment whenever the shell is done,
//! however long that takes.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::process::run_with_deadline;

/// How long the shell has to start and say its environment.
const PATIENCE: Duration = Duration::from_secs(5);

/// What the shell writes ahead of its environment, to tell it from what
/// its profile writes.
const MARK: &str = "__cox_environment__";

/// Set for a login shell Cox reads, so that a profile can tell, as with
/// VS Code's `VSCODE_RESOLVING_ENVIRONMENT`, and skip what only a terminal
/// needs. coxd started from the shell finds it set, and takes it away.
pub const RESOLVING: &str = "COX_RESOLVING_ENVIRONMENT";

/// Set by a window that could not take in the shell's environment, so
/// that the coxd it starts starts from the login shell.
const LAUNCHDS: &str = "COX_ENVIRONMENT_IS_LAUNCHDS";

/// The variables the shell sets of itself, which say nothing of Cox, and
/// the one Cox sets for it.
const ITS_OWN: [&str; 5] = ["PWD", "OLDPWD", "SHLVL", "_", RESOLVING];

/// What came of taking in the login shell's environment, for the log.
#[derive(Debug)]
pub enum Taken {
    /// Cox was not opened by the system, so it has the environment of
    /// the shell that started it already.
    NotNeeded,
    /// It was taken in from the shell, with so many variables.
    From(PathBuf, usize),
    /// The shell could not say it, for this reason.
    Failed(String),
}

/// Takes in the environment of the human's login shell, when the system
/// opened Cox.
///
/// It sets the process's environment, so it has to run before any other
/// thread starts.
#[must_use]
pub fn take_in() -> Taken {
    if !opened_by_the_system() {
        return Taken::NotNeeded;
    }
    let shell = shell();
    let taken = take_in_from(&shell);
    if matches!(taken, Taken::Failed(_)) {
        // SAFETY: as below, before any thread of Cox's starts.
        unsafe { std::env::set_var(LAUNCHDS, "1") };
    }
    taken
}

/// What came of starting coxd from the login shell, for the log.
#[derive(Debug)]
pub enum Started {
    /// coxd has the environment it was started with: a terminal's, or a
    /// window's that took in the shell's.
    AsItIs,
    /// coxd was started again from the login shell, and has its
    /// environment.
    FromShell,
    /// coxd could not be started from the shell, for this reason, and has
    /// launchd's environment.
    Failed(String),
}

/// For coxd, first, before any thread starts: when launchd started it, or
/// a window that has launchd's environment did, starts it again, as the
/// same process, from the human's login shell, which hands it its
/// environment. Returns only when it is not started again.
#[must_use]
pub fn start_from_login_shell() -> Started {
    if std::env::var_os(RESOLVING).is_some() {
        // SAFETY: coxd has no other thread yet.
        unsafe {
            std::env::remove_var(RESOLVING);
            std::env::remove_var(LAUNCHDS);
        }
        return Started::FromShell;
    }
    let launchds = std::env::var_os(LAUNCHDS).is_some();
    if !(cfg!(target_os = "macos") && (launchds || opened_by_the_system())) {
        return Started::AsItIs;
    }
    let shell = shell();
    let script = std::env::current_exe()
        .map_err(|error| error.to_string())
        .and_then(|exe| {
            let mut words = vec![exe.into_os_string()];
            words.extend(std::env::args_os().skip(1));
            started_again(&words).ok_or_else(|| {
                "its command has a quote or a backslash, which no shell quotes alike".to_owned()
            })
        });
    let error = match script {
        // `exec` returns only if the shell does not start.
        Ok(script) => Command::new(&shell)
            .args(["-l", "-i", "-c"])
            .arg(script)
            .env(RESOLVING, "1")
            .exec()
            .to_string(),
        Err(error) => error,
    };
    Started::Failed(format!("{}: {error}", shell.display()))
}

/// The script that has a shell run `words`, a program and its arguments,
/// in its place: each word in single quotes, which sh, bash, zsh and fish
/// read alike, unless a word holds a quote or a backslash, which they do
/// not.
fn started_again(words: &[OsString]) -> Option<String> {
    let mut script = String::from("exec");
    for word in words {
        let word = word.to_str()?;
        if word.contains(['\'', '\\']) {
            return None;
        }
        script.push_str(" '");
        script.push_str(word);
        script.push('\'');
    }
    Some(script)
}

/// Takes in the environment of `shell`.
fn take_in_from(shell: &std::path::Path) -> Taken {
    let shell = shell.to_owned();
    match environment_of(&shell) {
        Ok(environment) => {
            let count = environment.len();
            for (name, value) in environment {
                // SAFETY: `main` calls this first, before any thread of
                // Cox's starts, so none reads the environment meanwhile.
                unsafe { std::env::set_var(name, value) };
            }
            Taken::From(shell, count)
        }
        Err(error) => Taken::Failed(format!("{}: {error}", shell.display())),
    }
}

/// Whether the system opened Cox, which macOS does through launchd, the
/// process of id 1, which `open` uses too.
fn opened_by_the_system() -> bool {
    // SAFETY: `getppid` has no preconditions and cannot fail.
    cfg!(target_os = "macos") && unsafe { libc::getppid() } == 1
}

/// The human's shell: `SHELL`, or else the one their account names, or
/// else `/bin/sh`.
fn shell() -> PathBuf {
    if let Some(shell) = std::env::var_os("SHELL").filter(|shell| !shell.is_empty()) {
        return PathBuf::from(shell);
    }
    // SAFETY: `getpwuid` returns a pointer into a static record, or null,
    // read here before anything else could call it.
    unsafe {
        let entry = libc::getpwuid(libc::getuid());
        if !entry.is_null() && !(*entry).pw_shell.is_null() {
            let shell = std::ffi::CStr::from_ptr((*entry).pw_shell);
            if !shell.is_empty() {
                return PathBuf::from(OsStr::from_bytes(shell.to_bytes()));
            }
        }
    }
    PathBuf::from("/bin/sh")
}

/// The environment `shell` has as a login shell, and an interactive one,
/// which reads the profiles the human's terminal does, without the
/// variables it sets of itself.
fn environment_of(shell: &std::path::Path) -> Result<Vec<(OsString, OsString)>, String> {
    // Its threads touch no environment, and a shell that never ends does
    // not keep Cox from opening.
    let output = run_with_deadline(
        Command::new(shell)
            .args(["-l", "-i", "-c"])
            .arg(format!("printf '\\0{MARK}\\0'; exec /usr/bin/env -0"))
            .env(RESOLVING, "1"),
        None,
        PATIENCE,
        |_| {},
    )
    .map_err(|error| error.to_string())?;
    let Some(status) = output.status else {
        return Err(format!("it said nothing in {} seconds", PATIENCE.as_secs()));
    };
    if !status.success() {
        return Err(format!("it ended with {status}"));
    }
    parse(&output.stdout).ok_or_else(|| "it did not say its environment".to_owned())
}

/// The variables `output` lists, after the mark, each `NAME=value` and a
/// NUL, as `env -0` writes them, without those the shell sets of itself;
/// or `None` if it has no mark.
fn parse(output: &[u8]) -> Option<Vec<(OsString, OsString)>> {
    let mark = format!("\0{MARK}\0");
    let start = output
        .windows(mark.len())
        .position(|window| window == mark.as_bytes())?
        + mark.len();
    let variables = output[start..]
        .split(|&byte| byte == 0)
        .filter_map(|variable| {
            let equals = variable.iter().position(|&byte| byte == b'=')?;
            let (name, value) = (&variable[..equals], &variable[equals + 1..]);
            (!name.is_empty() && !ITS_OWN.iter().any(|own| own.as_bytes() == name)).then(|| {
                (
                    OsString::from_vec(name.to_vec()),
                    OsString::from_vec(value.to_vec()),
                )
            })
        })
        .collect();
    Some(variables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_is_read_past_what_the_profile_writes() {
        let output = b"Welcome!\n\0__cox_environment__\0PATH=/opt/bin:/usr/bin\0\
            SHLVL=2\0NOTE=a=b\nc\0PWD=/\0_=/usr/bin/env\0";
        assert_eq!(
            parse(output),
            Some(vec![
                ("PATH".into(), "/opt/bin:/usr/bin".into()),
                ("NOTE".into(), "a=b\nc".into()),
            ])
        );
        assert_eq!(parse(b"PATH=/usr/bin\0"), None);
    }

    #[test]
    fn a_shell_runs_the_words_it_is_started_again_with() {
        let words: Vec<OsString> = ["/bin/echo", "a b", "--data", "/tmp/my data"]
            .map(OsString::from)
            .to_vec();
        let script = started_again(&words).unwrap();
        for shell in ["/bin/sh", "/bin/zsh", "/bin/bash"] {
            if !std::path::Path::new(shell).exists() {
                continue;
            }
            let output = Command::new(shell).arg("-c").arg(&script).output().unwrap();
            assert_eq!(output.stdout, b"a b --data /tmp/my data\n", "{shell}");
        }
    }

    #[test]
    fn words_no_shell_quotes_alike_are_not_started_again() {
        assert_eq!(started_again(&["/bin/echo".into(), "it's".into()]), None);
        assert_eq!(started_again(&["/bin/echo".into(), "a\\b".into()]), None);
    }

    #[test]
    fn a_login_shell_says_its_environment() {
        let environment = environment_of(std::path::Path::new("/bin/sh")).unwrap();
        assert!(environment.iter().any(|(name, _)| name == "PATH"));
        assert!(environment.iter().all(|(name, _)| name != "PWD"));
    }
}
