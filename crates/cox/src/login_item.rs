//! coxd started as the human logs in, so that the chats' schedules run and
//! the phone reaches Cox before the human opens its window: a launch agent
//! in `~/Library/LaunchAgents` that starts this Cox's own executable, which
//! an installed Cox writes as it opens. The human sees it in System
//! Settings, under General, Login Items, and can turn it off there; Cox
//! leaves it off then.
//!
//! Not the agent `SMAppService` registers: launchd holds such an agent to
//! the signature it was registered with, and Cox's certificate, not one of
//! Apple's, names no team, so the signature is the executable's hash, which
//! every update changes. launchd then ends coxd at login, for its code
//! signature (`OS_REASON_CODESIGNING`). An agent in `~/Library/LaunchAgents`
//! starts whatever is at its path.

use std::path::Path;

/// What launchd knows coxd by.
const LABEL: &str = "com.github.tennashi.cox.coxd";

/// Writes the launch agent that starts coxd at login, for a Cox that is
/// installed (see [`coxd::storage::installed`]), where this Cox is now.
pub fn keep() {
    if !coxd::storage::installed() {
        return;
    }
    let (Some(home), Ok(exe)) = (std::env::var_os("HOME"), std::env::current_exe()) else {
        return;
    };
    let folder = Path::new(&home).join("Library/LaunchAgents");
    let plist = folder.join(format!("{LABEL}.plist"));
    let agent = launch_agent(&exe);
    if std::fs::read_to_string(&plist).ok().as_deref() != Some(agent.as_str()) {
        match std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&plist, &agent)) {
            Ok(()) => tracing::info!("coxd starts at login from now on"),
            Err(error) => {
                tracing::warn!(
                    "coxd does not start at login: cannot write {}: {error}",
                    plist.display()
                );
                return;
            }
        }
    }
    if turned_off_in_login_items() {
        tracing::info!("coxd does not start at login: it is off in Login Items");
    }
}

/// launchd's domain of the human's login session.
fn domain() -> String {
    // SAFETY: `getuid` has no preconditions and cannot fail.
    format!("gui/{}", unsafe { libc::getuid() })
}

/// Whether the human turned coxd's launch agent off in Login Items, which
/// launchd keeps as the label disabled.
fn turned_off_in_login_items() -> bool {
    std::process::Command::new("launchctl")
        .arg("print-disabled")
        .arg(domain())
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|line| line.contains(&format!("\"{LABEL}\" => disabled")))
        })
}

/// The launch agent that starts coxd, of the Cox at `cox`, as the human
/// logs in, and only then: coxd, once ended, is started by the window.
fn launch_agent(cox: &Path) -> String {
    let cox = cox
        .display()
        .to_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{cox}</string>
		<string>--daemon</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>AssociatedBundleIdentifiers</key>
	<string>com.github.tennashi.cox</string>
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::launch_agent;

    #[test]
    fn the_launch_agent_starts_this_cox_s_coxd_at_login_once() {
        let agent = launch_agent(Path::new("/Applications/Cox.app/Contents/MacOS/Cox"));
        assert!(agent.contains("<string>com.github.tennashi.cox.coxd</string>"));
        assert!(agent.contains(
            "<string>/Applications/Cox.app/Contents/MacOS/Cox</string>\n\t\t<string>--daemon</string>"
        ));
        assert!(agent.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(!agent.contains("KeepAlive"), "coxd is not started again");
    }

    #[test]
    fn the_launch_agent_names_a_cox_whose_path_xml_would_read_otherwise() {
        let agent = launch_agent(Path::new("/Users/me/<Tom & Jerry>/Cox"));
        assert!(agent.contains("<string>/Users/me/&lt;Tom &amp; Jerry&gt;/Cox</string>"));
    }
}
