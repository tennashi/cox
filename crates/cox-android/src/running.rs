//! The emulators running: each writes a file of itself as it starts,
//! `pid_<pid>.ini` in a folder of the system's, and takes it away as it
//! ends, saying the device it runs, `avd.name`, and where its gRPC service
//! listens, `grpc.port`, with the token it asks for, `grpc.token`, when it
//! asks for one, as one Android Studio starts does.

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
    let mut folders = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            folders.push(PathBuf::from(home).join("Library/Caches/TemporaryItems/avd/running"));
        }
    } else if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty())
    {
        folders.push(PathBuf::from(runtime).join("avd/running"));
    }
    folders.push(std::env::temp_dir().join("avd/running"));
    folders
}

/// The emulators running now, as their files say: those whose process is
/// gone, as one that crashed leaves its file, are left out.
#[must_use]
pub fn all() -> Vec<Running> {
    let mut running = Vec::new();
    for folder in folders() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
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

/// The file an emulator writes of itself, saying `running`, as
/// `cox-fake-emulator` does; it is taken away as the returned guard
/// drops.
///
/// # Errors
///
/// When the folder cannot be made, or the file written.
pub fn write(running: &Running) -> std::io::Result<Written> {
    let folder = folders().remove(0);
    std::fs::create_dir_all(&folder)?;
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

    #[test]
    fn reads_the_device_the_port_and_the_token() {
        let text = "avd.name=Pixel_8\nport.serial=5554\ngrpc.port=8554\ngrpc.token=abc==\n";
        assert_eq!(
            parse(7, text),
            Some(Running {
                pid: 7,
                device: "Pixel_8".into(),
                port: 8554,
                token: Some("abc==".into()),
            })
        );
    }

    #[test]
    fn an_emulator_with_no_grpc_service_is_not_reached() {
        assert_eq!(parse(7, "avd.name=Pixel_8\nport.serial=5554\n"), None);
    }

    #[test]
    fn the_file_is_named_after_the_process() {
        assert_eq!(
            pid_of(Path::new("/tmp/avd/running/pid_4242.ini")),
            Some(4242)
        );
        assert_eq!(pid_of(Path::new("/tmp/avd/running/other.ini")), None);
    }
}
