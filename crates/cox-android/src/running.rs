//! The emulators running: each writes a file of itself as it starts,
//! `pid_<pid>.ini` in a folder of the system's, and takes it away as it
//! ends, saying the device it runs, `avd.name`, and where its gRPC service
//! listens, `grpc.port`, with the token it asks for, `grpc.token`, when it
//! asks for one, as one Android Studio starts does.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// An emulator running, as its file says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    /// Its process.
    pub pid: i32,
    /// The device it runs.
    pub device: String,
    /// The port its gRPC service listens on, on this machine.
    pub port: u16,
    /// The token its gRPC service asks for, if it asks for one.
    pub token: Option<String>,
}

/// The folders the emulators write their files in, the one they write in
/// first: on macOS, `~/Library/Caches/TemporaryItems/avd/running`, and on
/// Linux, `$XDG_RUNTIME_DIR/avd/running`; then, on either, `avd/running`
/// among the system's temporary files.
#[must_use]
pub fn folders() -> Vec<PathBuf> {
    folders_from(|name| std::env::var_os(name))
}

/// The folders the emulators write their files in, as [`folders`] finds
/// them, with `env` saying the environment's variables `HOME` and
/// `XDG_RUNTIME_DIR`.
#[must_use]
pub fn folders_from(env: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(home) = env("HOME") {
            folders.push(PathBuf::from(home).join("Library/Caches/TemporaryItems/avd/running"));
        }
    } else if let Some(runtime) = env("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        folders.push(PathBuf::from(runtime).join("avd/running"));
    }
    folders.push(std::env::temp_dir().join("avd/running"));
    folders
}

/// The emulators running now, as their files say: those whose process is
/// gone, as one that crashed leaves its file, are left out.
#[must_use]
pub fn all() -> Vec<Running> {
    in_folders(&folders())
}

/// The emulators running now, as their files in `folders` say, as [`all`]
/// finds them.
#[must_use]
pub fn in_folders(folders: &[PathBuf]) -> Vec<Running> {
    let mut running = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(pid) = pid_of(&path)
                && alive(pid)
                && let Ok(text) = std::fs::read_to_string(&path)
                && let Some(one) = parse(pid, &text)
                && !running.contains(&one)
            {
                running.push(one);
            }
        }
    }
    running
}

/// The emulator running `device`, if one is.
#[must_use]
pub fn of(device: &str) -> Option<Running> {
    all().into_iter().find(|running| running.device == device)
}

/// Stops `running`, as closing its window does: it shuts down, saving
/// its state for its next start.
pub fn stop(running: &Running) {
    if running.pid > 0 {
        // SAFETY: the process is an emulator, by the id its file gives.
        unsafe { libc::kill(running.pid, libc::SIGTERM) };
    }
}

/// The process id a file `pid_<pid>.ini` is named after.
fn pid_of(path: &Path) -> Option<i32> {
    path.file_name()?
        .to_str()?
        .strip_prefix("pid_")?
        .strip_suffix(".ini")?
        .parse()
        .ok()
}

fn alive(pid: i32) -> bool {
    // Signal 0 checks that the process is there, and sends nothing.
    // SAFETY: kill with signal 0 only asks whether the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// What a file of an emulator's says of it: `key=value` lines.
fn parse(pid: i32, text: &str) -> Option<Running> {
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    Some(Running {
        pid,
        device: value("avd.name")?.to_owned(),
        port: value("grpc.port")?.parse().ok()?,
        token: value("grpc.token").map(str::to_owned),
    })
}

/// The file an emulator writes of itself in `folder`, the first of
/// [`folders`], saying `running`, as `cox-fake-emulator` does; it is taken
/// away as the returned guard drops.
///
/// # Errors
///
/// When the folder cannot be made, or the file written.
pub fn write(folder: &Path, running: &Running) -> std::io::Result<Written> {
    std::fs::create_dir_all(folder)?;
    let path = folder.join(format!("pid_{}.ini", running.pid));
    let mut text = format!(
        "avd.name={}\ngrpc.port={}\nport.serial=5554\n",
        running.device, running.port
    );
    if let Some(token) = &running.token {
        text.push_str("grpc.token=");
        text.push_str(token);
        text.push('\n');
    }
    std::fs::write(&path, text)?;
    Ok(Written(path))
}

/// A file an emulator wrote of itself, taken away as this drops.
pub struct Written(PathBuf);

impl Drop for Written {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env;
    use crate::scratch::Scratch;

    fn this_process(device: &str, token: Option<&str>) -> Running {
        Running {
            pid: i32::try_from(std::process::id()).unwrap(),
            device: device.into(),
            port: 8554,
            token: token.map(str::to_owned),
        }
    }

    /// A process that ended, whose id no process has now.
    fn ended_pid() -> i32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        i32::try_from(child.id()).unwrap()
    }

    #[test]
    fn an_emulator_is_found_by_the_file_it_writes() {
        let scratch = Scratch::new("running-found");
        let folder = scratch.path().to_owned();
        let running = this_process("Pixel_8", Some("abc=="));
        let _written = write(&folder, &running).unwrap();
        assert_eq!(in_folders(&[folder]), [running]);
    }

    #[test]
    fn an_emulator_asking_no_token_is_found_with_none() {
        let scratch = Scratch::new("running-no-token");
        let folder = scratch.path().to_owned();
        let running = this_process("Pixel_8", None);
        let _written = write(&folder, &running).unwrap();
        assert_eq!(in_folders(&[folder])[0].token, None);
    }

    #[test]
    fn its_file_goes_as_the_guard_drops() {
        let scratch = Scratch::new("running-drops");
        let folder = scratch.path().to_owned();
        drop(write(&folder, &this_process("Pixel_8", None)).unwrap());
        assert_eq!(in_folders(&[folder]), []);
    }

    #[test]
    fn an_emulator_whose_process_is_gone_is_left_out() {
        let scratch = Scratch::new("running-gone");
        let folder = scratch.path().to_owned();
        let _alive = write(&folder, &this_process("Alive", None)).unwrap();
        let gone = Running {
            pid: ended_pid(),
            ..this_process("Crashed", None)
        };
        let _gone = write(&folder, &gone).unwrap();
        let devices: Vec<String> = in_folders(&[folder])
            .into_iter()
            .map(|running| running.device)
            .collect();
        assert_eq!(devices, ["Alive"]);
    }

    #[test]
    fn an_emulator_with_no_grpc_service_is_not_reached() {
        let scratch = Scratch::new("running-no-grpc");
        let pid = std::process::id();
        std::fs::write(
            scratch.path().join(format!("pid_{pid}.ini")),
            "avd.name=Pixel_8\nport.serial=5554\n",
        )
        .unwrap();
        assert_eq!(in_folders(&[scratch.path().to_owned()]), []);
    }

    #[test]
    fn only_files_named_after_a_process_are_read() {
        let scratch = Scratch::new("running-named");
        std::fs::write(
            scratch.path().join("other.ini"),
            "avd.name=Pixel_8\ngrpc.port=8554\n",
        )
        .unwrap();
        assert_eq!(in_folders(&[scratch.path().to_owned()]), []);
    }

    #[test]
    fn an_emulator_in_two_folders_is_found_once() {
        let scratch = Scratch::new("running-twice");
        let (first, second) = (scratch.path().join("first"), scratch.path().join("second"));
        let running = this_process("Pixel_8", None);
        let _first = write(&first, &running).unwrap();
        let _second = write(&second, &running).unwrap();
        assert_eq!(in_folders(&[first, second]), [running]);
    }

    #[test]
    fn the_folders_end_with_the_systems_temporary_files() {
        let folders = folders_from(env(&[]));
        // allow-temp-dir: the folder is only named, as the system's is, not made
        assert_eq!(folders, [std::env::temp_dir().join("avd/running")]);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn on_macos_the_emulators_write_in_homes_caches_first() {
        let folders = folders_from(env(&[("HOME", Path::new("/home"))]));
        assert_eq!(
            folders,
            [
                PathBuf::from("/home/Library/Caches/TemporaryItems/avd/running"),
                // allow-temp-dir: the folder is only named, as the system's is, not made
                std::env::temp_dir().join("avd/running"),
            ]
        );
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn on_linux_the_emulators_write_in_the_runtime_folder_first() {
        let folders = folders_from(env(&[("XDG_RUNTIME_DIR", Path::new("/run/user/1"))]));
        assert_eq!(
            folders,
            [
                PathBuf::from("/run/user/1/avd/running"),
                // allow-temp-dir: the folder is only named, as the system's is, not made
                std::env::temp_dir().join("avd/running"),
            ]
        );
    }
}
