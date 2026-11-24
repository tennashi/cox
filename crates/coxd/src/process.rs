//! What Cox does as it starts a program, the agent's or a terminal's:
//! leaves it none of Cox's own files. And how Cox runs a program it waits
//! on for what it writes, no longer than a deadline, and stops it sooner
//! when what it does is no longer wanted, or Cox quits (see [`Stop`]).

use std::io::{self, Read, Write as _};
use std::os::unix::process::CommandExt as _;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// How long what a stopped program wrote has to arrive.
const LINGERS: Duration = Duration::from_secs(1);

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

/// What a program wrote, and how it ended: `status` is `None` when it
/// took longer than its deadline and was stopped.
#[derive(Debug)]
pub struct Output {
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    /// Whether it ended in time, and well.
    #[must_use]
    pub fn success(&self) -> bool {
        self.status.is_some_and(|status| status.success())
    }
}

/// What one of the threads that watch a program heard.
enum Heard {
    Out(Vec<u8>),
    Err(Vec<u8>),
    Ended(io::Result<ExitStatus>),
}

/// What stops a program [`run_stoppable`] runs, with every process it
/// started, before its deadline: as what it does is no longer wanted
/// ([`Stop::stop`]), or as Cox quits ([`stop_all`]), as a program that
/// takes minutes, as an indexer, would otherwise outlive coxd.
#[derive(Clone, Debug, Default)]
pub struct Stop(Arc<Mutex<Stopping>>);

#[derive(Debug, Default)]
struct Stopping {
    /// The process group of the program, while it runs.
    group: Option<libc::pid_t>,
    stopped: bool,
}

/// What stops each program [`run_stoppable`] runs now.
static RUNNING: Mutex<Vec<Stop>> = Mutex::new(Vec::new());

impl Stop {
    fn stopping(&self) -> MutexGuard<'_, Stopping> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops the program, with every process it started, if it runs, and
    /// keeps it from running if it is yet to.
    pub fn stop(&self) {
        let mut stopping = self.stopping();
        stopping.stopped = true;
        if let Some(group) = stopping.group {
            // SAFETY: this only signals the program's group, which is
            // there while the program runs, as the group is forgotten once
            // it ended (see `Running`).
            unsafe { libc::killpg(group, libc::SIGKILL) };
        }
    }

    /// Whether it was stopped.
    #[must_use]
    pub fn stopped(&self) -> bool {
        self.stopping().stopped
    }

    /// Has the program of `group` stop with this from now on, until what
    /// this answers goes, once it ended: at once, if it was stopped
    /// before it started.
    fn running(&self, group: libc::pid_t) -> Running<'_> {
        let mut stopping = self.stopping();
        stopping.group = Some(group);
        if stopping.stopped {
            // SAFETY: as in `stop`.
            unsafe { libc::killpg(group, libc::SIGKILL) };
        }
        RUNNING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(self.clone());
        Running(self)
    }
}

/// A program that stops with its [`Stop`], until it ended.
struct Running<'a>(&'a Stop);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.stopping().group = None;
        RUNNING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|running| !Arc::ptr_eq(&running.0, &(self.0).0));
    }
}

/// Stops every program [`run_stoppable`] runs now, as Cox quits.
pub fn stop_all() {
    let running = RUNNING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    for stop in running {
        stop.stop();
    }
}

/// Runs `command` with `input` on its standard input, or none, telling
/// `wrote` all it has written on its standard output each time it writes
/// more; stopped, with every process it started, once it takes longer
/// than `deadline`.
///
/// # Errors
///
/// When the program does not start, or cannot be waited for.
pub fn run_with_deadline(
    command: &mut Command,
    input: Option<Vec<u8>>,
    deadline: Duration,
    wrote: impl FnMut(&[u8]),
) -> io::Result<Output> {
    run(command, input, deadline, wrote, None)
}

/// Runs `command`, with no input, as [`run_with_deadline`] does, and
/// stopped sooner by `stop`.
///
/// # Errors
///
/// When the program does not start, or cannot be waited for.
pub fn run_stoppable(command: &mut Command, deadline: Duration, stop: &Stop) -> io::Result<Output> {
    run(command, None, deadline, |_| {}, Some(stop))
}

/// [`run_with_deadline`], stopped sooner by `stop`, if given.
fn run(
    command: &mut Command,
    input: Option<Vec<u8>>,
    deadline: Duration,
    mut wrote: impl FnMut(&[u8]),
    stop: Option<&Stop>,
) -> io::Result<Output> {
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A group of its own, to stop with all it started, as what it
        // started holds its output open too.
        .process_group(0)
        .spawn()?;
    let group = libc::pid_t::try_from(child.id()).map_err(io::Error::other)?;
    let _running = stop.map(|stop| stop.running(group));
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        // Written apart from the reading, so that a program that writes as
        // it reads does not wait on Cox while Cox waits on it.
        thread::spawn(move || stdin.write_all(&input));
    }
    let (tells, heard) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        let tells = tells.clone();
        thread::spawn(move || {
            let _ = read_chunks(stdout, |chunk| {
                let _ = tells.send(Heard::Out(chunk));
            });
        });
    }
    if let Some(mut stderr) = child.stderr.take() {
        let tells = tells.clone();
        thread::spawn(move || {
            let mut said = Vec::new();
            let _ = stderr.read_to_end(&mut said);
            let _ = tells.send(Heard::Err(said));
        });
    }
    thread::spawn(move || {
        let _ = tells.send(Heard::Ended(child.wait()));
    });
    let mut output = Output {
        status: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    let mut status = None;
    let mut until = Instant::now() + deadline;
    let mut late = false;
    loop {
        match heard.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(Heard::Out(chunk)) => {
                output.stdout.extend_from_slice(&chunk);
                wrote(&output.stdout);
            }
            Ok(Heard::Err(said)) => output.stderr = said,
            Ok(Heard::Ended(ended)) => status = Some(ended?),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) if late => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // SAFETY: this only signals the program's group, whose id
                // can be another's only once all of it has ended, which
                // closes its output, as it had not by now.
                unsafe { libc::killpg(group, libc::SIGKILL) };
                late = true;
                until = Instant::now() + LINGERS;
            }
        }
    }
    if !late {
        output.status = status;
    }
    Ok(output)
}

/// Tells `chunk` each part `from` gives, until it ends.
///
/// A read a signal interrupts is read again, as `read_to_end` does: the
/// window's process, where CEF runs too, may handle a signal without
/// having the system restart what it interrupted.
fn read_chunks(mut from: impl Read, mut chunk: impl FnMut(Vec<u8>)) -> io::Result<()> {
    let mut buffer = [0; 4096];
    loop {
        match from.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => chunk(buffer[..read].to_vec()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::os::fd::AsRawFd;

    use super::*;

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

    #[test]
    fn a_program_run_with_a_deadline_answers_what_it_wrote_and_how_it_ended() {
        let mut told = Vec::new();
        let output = run_with_deadline(
            Command::new("sh").args(["-c", "cat; printf no >&2; exit 3"]),
            Some(b"hi".to_vec()),
            Duration::from_secs(10),
            |so_far| told.push(so_far.to_vec()),
        )
        .unwrap();
        assert_eq!(output.status.and_then(|status| status.code()), Some(3));
        assert_eq!(output.stdout, b"hi");
        assert_eq!(output.stderr, b"no");
        assert_eq!(told.last().map(Vec::as_slice), Some(&b"hi"[..]));
    }

    #[test]
    fn a_program_run_with_no_input_reads_none() {
        let output = run_with_deadline(
            Command::new("sh").args(["-c", "cat; echo read"]),
            None,
            Duration::from_secs(10),
            |_| {},
        )
        .unwrap();
        assert_eq!(output.stdout, b"read\n");
    }

    #[test]
    fn a_program_late_for_its_deadline_is_stopped_with_all_it_started() {
        let folder = crate::testing::TempDir::new();
        let pid_file = folder.join("pid");
        let started = Instant::now();
        let output = run_with_deadline(
            Command::new("sh").arg("-c").arg(format!(
                "sleep 60 & echo $! > '{}'; echo started; wait",
                pid_file.display()
            )),
            None,
            Duration::from_millis(500),
            |_| {},
        )
        .unwrap();
        assert!(output.status.is_none(), "it ended in time: {output:?}");
        assert_eq!(output.stdout, b"started\n", "what it wrote before");
        assert!(started.elapsed() < Duration::from_secs(30));
        let child: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The system takes a little while to clear away what was stopped.
        let until = Instant::now() + Duration::from_secs(10);
        // SAFETY: signal 0 only asks whether the process is there.
        while unsafe { libc::kill(child, 0) } == 0 {
            assert!(Instant::now() < until, "what it started is stopped too");
            // allow-sleep: polls until the system clears the stopped process
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// A program that says it started, in `started`, and then runs a
    /// minute, as a child of its own.
    fn long(started: &std::path::Path) -> Command {
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "sleep 60 & echo $! > '{}'; wait",
            started.display()
        ));
        command
    }

    /// The process `pid_file` names, once it is written.
    fn started(pid_file: &std::path::Path) -> libc::pid_t {
        let mut pid = None;
        crate::testing::waits::until("the program starts", || {
            pid = std::fs::read_to_string(pid_file)
                .ok()
                .and_then(|pid| pid.trim().parse().ok());
            pid.is_some()
        });
        pid.unwrap()
    }

    /// Waits until the process `pid` is gone.
    fn gone(pid: libc::pid_t) {
        // SAFETY: signal 0 only asks whether the process is there.
        crate::testing::waits::until("what it started is stopped too", || unsafe {
            libc::kill(pid, 0) != 0
        });
    }

    #[test]
    fn a_program_is_stopped_with_all_it_started_when_no_longer_wanted() {
        let folder = crate::testing::TempDir::new();
        let pid_file = folder.join("pid");
        let stop = Stop::default();
        let running = {
            let (stop, pid_file) = (stop.clone(), pid_file.clone());
            thread::spawn(move || {
                run_stoppable(&mut long(&pid_file), Duration::from_mins(5), &stop)
            })
        };
        let child = started(&pid_file);
        stop.stop();
        let output = running.join().unwrap().unwrap();
        assert!(!output.success(), "{output:?}");
        assert!(stop.stopped());
        gone(child);

        // Stopped before it starts, it does not run on.
        let stopped = Stop::default();
        stopped.stop();
        let begun = Instant::now();
        let output = run_stoppable(
            &mut long(&folder.join("never")),
            Duration::from_mins(5),
            &stopped,
        )
        .unwrap();
        assert!(!output.success());
        assert!(begun.elapsed() < Duration::from_secs(30));
    }

    /// Set in the copy of the test program that checks `stop_all` alone,
    /// where it stops no other test's programs.
    const STOPS_ALONE: &str = "COX_STOP_ALL_ALONE";

    #[test]
    fn every_program_that_stops_stops_as_cox_quits() {
        if std::env::var_os(STOPS_ALONE).is_none() {
            let alone = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "process::tests::every_program_that_stops_stops_as_cox_quits",
                    "--test-threads=1",
                ])
                .env(STOPS_ALONE, "1")
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&alone.stdout);
            assert!(
                alone.status.success() && said.contains("1 passed"),
                "the check alone did not pass: {said}"
            );
            return;
        }
        let folder = crate::testing::TempDir::new();
        let running: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|name| {
                let pid_file = folder.join(name);
                let runs = {
                    let pid_file = pid_file.clone();
                    thread::spawn(move || {
                        run_stoppable(
                            &mut long(&pid_file),
                            Duration::from_mins(5),
                            &Stop::default(),
                        )
                    })
                };
                (started(&pid_file), runs)
            })
            .collect();
        stop_all();
        for (child, runs) in running {
            assert!(!runs.join().unwrap().unwrap().success());
            gone(child);
        }
    }

    #[test]
    fn a_read_a_signal_interrupts_is_read_again() {
        /// Says `parts` one at a time, each after a read a signal
        /// interrupts.
        struct Interrupted {
            parts: VecDeque<&'static [u8]>,
            interrupt: bool,
        }
        impl Read for Interrupted {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.interrupt = !self.interrupt;
                if self.interrupt {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let Some(part) = self.parts.pop_front() else {
                    return Ok(0);
                };
                buffer[..part.len()].copy_from_slice(part);
                Ok(part.len())
            }
        }
        let from = Interrupted {
            parts: VecDeque::from([&b"hel"[..], &b"lo"[..]]),
            interrupt: false,
        };
        let mut read = Vec::new();
        read_chunks(from, |chunk| read.extend(chunk)).unwrap();
        assert_eq!(read, b"hello");
    }
}
