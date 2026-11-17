//! What the emulator's gRPC service cannot tell or do, asked of the device
//! through the SDK's `adb`: the screen's elements, whether a text field
//! has the keyboard, and emptying the field that has it.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::running::Running;
use crate::ui::{self, Node};

/// How long `uiautomator dump` may take: it waits for the screen to be
/// idle, and gives up after about ten seconds of it moving.
const DUMP_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the other commands may take.
const TIMEOUT: Duration = Duration::from_secs(5);

/// How often a command that has not ended is looked at.
const POLL: Duration = Duration::from_millis(20);

/// `adb`, for one device.
#[derive(Clone, Debug)]
pub struct Adb {
    program: PathBuf,
    serial: String,
}

impl Adb {
    /// `adb` for `running`.
    ///
    /// # Errors
    ///
    /// When the SDK has no `adb`, or the emulator's file says no console
    /// port to name it by.
    pub fn of(running: &Running) -> Result<Self, String> {
        let program = crate::sdk::adb().ok_or("the Android SDK has no platform-tools/adb")?;
        let serial = running
            .adb_serial()
            .ok_or_else(|| format!("{} says no port adb knows it by", running.device))?;
        Ok(Self { program, serial })
    }

    /// The screen's elements, as `uiautomator dump` writes them.
    ///
    /// # Errors
    ///
    /// When `adb` fails, or what it writes is not a dump.
    pub fn ui(&self) -> Result<Vec<Node>, String> {
        let dumped = self.run(
            &["exec-out", "uiautomator", "dump", "/dev/tty"],
            DUMP_TIMEOUT,
        )?;
        // It says where it dumped to after the dump.
        let xml = dumped
            .rfind("</hierarchy>")
            .map_or(dumped.as_str(), |end| &dumped[..end + "</hierarchy>".len()]);
        ui::parse(xml).map_err(|error| format!("{error}: {}", dumped.trim()))
    }

    /// Whether a text field has the keyboard now, so that what is typed
    /// goes into it.
    ///
    /// # Errors
    ///
    /// When `adb` fails.
    pub fn is_editing(&self) -> Result<bool, String> {
        let said = self.run(&["shell", "dumpsys", "input_method"], TIMEOUT)?;
        Ok(editing_in(&said))
    }

    /// Empties the text field that has the keyboard: selects all of its
    /// text, and deletes it.
    ///
    /// # Errors
    ///
    /// When `adb` fails, or the device's `input` has no
    /// `keycombination`, as before Android 13.
    pub fn clear_field(&self) -> Result<(), String> {
        // Control and A, then Delete.
        let said = self.run(
            &["shell", "input keycombination 113 29 && input keyevent 67"],
            TIMEOUT,
        )?;
        if said.contains("Error") || said.contains("Unknown") {
            return Err(format!(
                "the device cannot select a field's text to empty it ({}): press Backspace \
                 with android_key and `times` instead.",
                said.trim()
            ));
        }
        Ok(())
    }

    /// What `adb -s <serial> <args>` writes, once it ends, within `timeout`.
    fn run(&self, args: &[&str], timeout: Duration) -> Result<String, String> {
        let mut child = Command::new(&self.program)
            .arg("-s")
            .arg(&self.serial)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot run adb: {error}"))?;
        let mut stdout = child.stdout.take().ok_or("adb has no output")?;
        let reading = std::thread::spawn(move || {
            let mut out = Vec::new();
            stdout.read_to_end(&mut out).map(|_| out)
        });
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
                break status;
            }
            if started.elapsed() > timeout {
                child.kill().ok();
                child.wait().ok();
                return Err(format!(
                    "adb {} did not end in {} s",
                    args.join(" "),
                    timeout.as_secs()
                ));
            }
            std::thread::sleep(POLL);
        };
        let out = reading
            .join()
            .map_err(|_| "adb's output was lost".to_owned())?
            .map_err(|error| error.to_string())?;
        let out = String::from_utf8_lossy(&out).into_owned();
        if status.success() {
            Ok(out)
        } else {
            let mut err = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                stderr.read_to_string(&mut err).ok();
            }
            Err(format!(
                "adb {} failed ({status}): {}",
                args.join(" "),
                format!("{out}{err}").trim()
            ))
        }
    }
}

/// Whether `dumpsys input_method` says a text field has the keyboard: the
/// app's input connection to the keyboard is open.
fn editing_in(said: &str) -> bool {
    said.lines().any(|line| {
        line.trim()
            .strip_prefix("mServedInputConnection=")
            .is_some_and(|connection| connection != "null")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_has_the_keyboard_while_the_app_has_an_input_connection() {
        let editing = "  mCurClient=ClientState{508312e mUid=10155}\n\
            \x20 mServedView=TextInputEditText{9eff63f VFED..CL. .F...... 0,0-796,154}\n\
            \x20 mServedInputConnection=RemoteInputConnectionImpl{connection=dld@dddfa48}\n";
        assert!(editing_in(editing));
        let not = "  fallbackInputConnection=RemoteInputConnectionImpl{mServedView=null}\n\
            \x20 mServedView=DecorView{b597107 V.E...... R....... 0,0-1080,2424}\n\
            \x20 mServedInputConnection=null\n";
        assert!(!editing_in(not));
    }
}
