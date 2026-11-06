//! The environment of the human's login shell, for a Cox the system
//! opened, as from the Finder, the Dock or a launcher such as Raycast.
//!
//! macOS starts such an app with launchd's environment, not the shell's:
//! its `PATH` is `/usr/bin:/bin:/usr/sbin:/sbin`, so the agent's program,
//! as `npx`, is not found, and what the human exports in their shell's
//! profile, as `CLAUDE_CODE_ENABLE_TODO_TOOLS`, does not reach the agent.
//! So Cox asks the login shell what its environment is, as the shell the
//! human types in has it, and takes it in, as Zed and VS Code do.

use std::ffi::{OsStr, OsString};
use std::io::Read as _;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How long the shell has to start and say its environment.
const PATIENCE: Duration = Duration::from_secs(5);

/// What the shell writes ahead of its environment, to tell it from what
/// its profile writes.
const MARK: &str = "__cox_environment__";

/// The variables the shell sets of itself, which say nothing of Cox.
const ITS_OWN: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

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
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c"])
        .arg(format!("printf '\\0{MARK}\\0'; exec /usr/bin/env -0"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut stdout = child.stdout.take().ok_or("it has no output")?;
    let (said, heard) = mpsc::channel();
    // A thread of its own, which touches no environment, so that a shell
    // that never ends does not keep Cox from opening.
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let read = stdout.read_to_end(&mut output).map(|_| output);
        let _ = said.send(read);
    });
    let Ok(read) = heard.recv_timeout(PATIENCE) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("it said nothing in {} seconds", PATIENCE.as_secs()));
    };
    let output = read.map_err(|error| error.to_string())?;
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("it ended with {status}"));
    }
    parse(&output).ok_or_else(|| "it did not say its environment".to_owned())
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
    fn a_login_shell_says_its_environment() {
        let environment = environment_of(std::path::Path::new("/bin/sh")).unwrap();
        assert!(environment.iter().any(|(name, _)| name == "PATH"));
        assert!(environment.iter().all(|(name, _)| name != "PWD"));
    }
}
