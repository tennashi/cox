//! What Cox does as it starts a program, the agent's or a terminal's:
//! leaves it none of Cox's own files.

/// Marks every file Cox has open to close when a program starts, so the
/// program started next, the agent or a terminal's, gets none of them.
/// Rust opens its files that way already, but CEF opens the stage's
/// `DevTools` port without: an agent that outlived Cox would keep the port
/// from the next Cox.
pub fn close_on_exec() {
    let Ok(entries) = std::fs::read_dir("/dev/fd") else {
        return;
    };
    let fds: Vec<libc::c_int> = entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok())
        .collect();
    // Past standard input, output and error, which the agent gets its own of.
    for fd in fds.into_iter().filter(|&fd| fd > 2) {
        // SAFETY: this only sets a flag of the descriptor, and fails
        // harmlessly on one no longer open, like the listing's own.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd;

    use super::close_on_exec;

    /// Whether the file at `fd` closes when a program starts.
    fn closes_on_exec(fd: libc::c_int) -> bool {
        // SAFETY: this only reads a flag of the descriptor.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        flags >= 0 && flags & libc::FD_CLOEXEC != 0
    }

    /// Set in the copy of the test program that checks `close_on_exec`
    /// alone, where no other test marks its files meanwhile.
    const ALONE: &str = "COX_CLOSE_ON_EXEC_ALONE";

    #[test]
    fn every_open_file_is_marked_to_close_when_a_program_starts() {
        if std::env::var_os(ALONE).is_none() {
            let alone = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "process::tests::every_open_file_is_marked_to_close_when_a_program_starts",
                    "--test-threads=1",
                ])
                .env(ALONE, "1")
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&alone.stdout);
            assert!(
                alone.status.success() && said.contains("1 passed"),
                "the check alone did not pass: {said}"
            );
            return;
        }
        let open = std::fs::File::open("/dev/null").unwrap();
        // A copy, which stays open across `exec`, like CEF's port.
        // SAFETY: `open` is a valid descriptor; the copy is closed below.
        let inherited = unsafe { libc::dup(open.as_raw_fd()) };
        assert!(inherited > 2);
        assert!(!closes_on_exec(inherited), "a copy starts open across exec");

        close_on_exec();

        assert!(closes_on_exec(inherited), "the copy is marked");
        assert!(closes_on_exec(open.as_raw_fd()), "the original is marked");
        // SAFETY: `inherited` is open, and nothing else uses it.
        unsafe { libc::close(inherited) };
    }
}
