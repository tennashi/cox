//! `cox` as a process, as the human, an agent or the window runs it: what
//! each way of running it prints, and how it exits. Each run keeps what
//! lasts in a folder of the test's own, with a home and a temp folder of
//! its own too, and reaches nothing outside the machine.

#[expect(dead_code, reason = "these tests use only part of the scratch folders")]
mod scratch;
#[expect(dead_code, reason = "these tests use only part of the waits")]
mod waits;

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

/// A folder of the test's own, by the path the processes see. Its name is
/// short, as a socket's path is short, of 104 bytes at most on macOS.
fn folder(name: &str) -> PathBuf {
    std::fs::canonicalize(scratch::made(&format!("cli-{name}"))).unwrap()
}

/// `cox` with `args`, apart from the human's Cox: nothing of the test's
/// environment, and `folder` as its home and its temp folder.
fn cox(folder: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cox"));
    command
        .args(args)
        .env_clear()
        .env("HOME", folder)
        .env("TMPDIR", folder)
        .stdin(Stdio::null());
    command
}

/// What `cox` with `args`, apart from the human's Cox in `folder`,
/// printed, and how it exited.
fn ran(folder: &Path, args: &[&str]) -> Output {
    cox(folder, args).output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn help_prints_the_usage_and_succeeds() {
    let folder = folder("help");
    for flag in ["--help", "-h"] {
        let output = ran(&folder, &[flag]);
        assert_eq!(output.status.code(), Some(0), "{flag} succeeds");
        assert_eq!(stdout(&output), format!("{}\n", cox::app::USAGE), "{flag}");
        assert!(stdout(&output).starts_with("Usage: cox "), "{flag}");
    }
}

#[test]
fn inspect_with_no_cox_running_fails_saying_none_answers() {
    let folder = folder("inspect-none");
    let data = folder.join("data");
    let output = ran(
        &folder,
        &["--data", data.to_str().unwrap(), "--inspect", "state"],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).starts_with(&format!(
            "cox: no Cox answers at {}",
            data.join("run/inspect.sock").display()
        )),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
}

#[test]
fn inspect_with_a_tool_it_does_not_know_fails_with_its_usage() {
    let folder = folder("inspect-usage");
    let data = folder.join("data");
    let output = ran(
        &folder,
        &["--data", data.to_str().unwrap(), "--inspect", "dance"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        format!("cox: {}\n", cox::inspect::client::USAGE)
    );
}

#[test]
fn the_mcp_bridge_to_a_socket_no_cox_listens_on_fails_saying_so() {
    let folder = folder("mcp-none");
    let socket = folder.join("mcp.sock");
    let output = ran(&folder, &["--mcp", socket.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with(&format!("cox: cannot reach Cox at {}", socket.display())),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
}

/// Writes `settings` as the settings of the Cox keeping what lasts in
/// `data`.
fn set(data: &Path, settings: &str) {
    std::fs::create_dir_all(data).unwrap();
    std::fs::write(data.join("settings.json"), settings).unwrap();
}

#[test]
fn the_phone_link_off_the_tailnet_pairs_on_this_machine_with_the_kept_secret() {
    let folder = folder("phone-link");
    let data = folder.join("data");
    set(&data, r#"{"phone": {"port": 7999, "tailscale": false}}"#);
    let output = ran(&folder, &["--data", data.to_str().unwrap(), "--phone-link"]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let secret = std::fs::read_to_string(data.join("state/phone-token")).unwrap();
    assert_eq!(secret.len(), 64, "a secret of 32 bytes, in hex");
    assert_eq!(
        stdout(&output),
        format!(
            "tailscale does not say the Mac's name in a tailnet. On this machine, open:\n\n  \
             http://127.0.0.1:7999/pair/{secret}\n"
        )
    );
}

#[test]
fn the_phone_link_is_the_same_on_each_run() {
    let folder = folder("phone-link-again");
    let data = folder.join("data");
    set(&data, r#"{"phone": {"tailscale": false}}"#);
    let args = ["--data", data.to_str().unwrap(), "--phone-link"];
    let first = ran(&folder, &args);
    let second = ran(&folder, &args);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert!(stdout(&first).contains("/pair/"), "{}", stdout(&first));
    assert_eq!(stdout(&second), stdout(&first));
}

#[test]
fn the_phone_link_with_the_page_turned_off_fails_saying_so() {
    let folder = folder("phone-off");
    let data = folder.join("data");
    set(&data, r#"{"phone": false}"#);
    let output = ran(&folder, &["--data", data.to_str().unwrap(), "--phone-link"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stderr(&output),
        "cox: settings.json turns the phone's page off, with {\"phone\": false}\n"
    );
    assert_eq!(stdout(&output), "");
}

/// A `cox --daemon` of the test's own, stopped as the test ends, even when
/// it fails, and the file of what it says on its standard error.
struct Daemon {
    process: Child,
    said: PathBuf,
}

impl Daemon {
    /// Starts `cox --daemon` keeping what lasts in `data`, fetching the
    /// registry of agents at `registry`.
    fn start(folder: &Path, data: &Path, registry: &str) -> Self {
        static STARTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let started = STARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let said = folder.join(format!("coxd-{started}.log"));
        let process = cox(folder, &["--data", data.to_str().unwrap(), "--daemon"])
            .env("COX_REGISTRY", registry)
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&said).unwrap())
            .spawn()
            .unwrap();
        Self { process, said }
    }

    /// What it said on its standard error so far.
    fn said(&self) -> String {
        std::fs::read_to_string(&self.said).unwrap_or_default()
    }

    /// How it ended, and what it said, once it ends.
    fn ended(mut self) -> (std::process::ExitStatus, String) {
        let mut ended = None;
        waits::until("coxd ends", || {
            ended = self.process.try_wait().unwrap();
            ended.is_some()
        });
        (ended.unwrap(), self.said())
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// No registry of agents: nothing answers there.
const NO_REGISTRY: &str = "http://127.0.0.1:9/registry.json";

/// Starts `cox --daemon` keeping what lasts in `data`, which serves no
/// phone, and waits until it answers on its socket.
fn daemon(folder: &Path, data: &Path) -> Daemon {
    set(data, r#"{"phone": false}"#);
    let mut daemon = Daemon::start(folder, data, NO_REGISTRY);
    let socket = data.join("run/coxd.sock");
    waits::until("coxd answers on its socket", || {
        if let Some(ended) = daemon.process.try_wait().unwrap() {
            panic!("coxd ended, {ended}, before it answered: {}", daemon.said());
        }
        std::os::unix::net::UnixStream::connect(&socket).is_ok()
    });
    daemon
}

#[test]
fn the_daemon_answers_on_the_socket_in_its_data_folder() {
    let folder = folder("coxd1");
    let data = folder.join("data");
    let _daemon = daemon(&folder, &data);
    assert!(
        std::os::unix::net::UnixStream::connect(data.join("run/coxd.sock")).is_ok(),
        "coxd answers"
    );
}

#[test]
fn a_second_daemon_on_the_same_data_folder_fails_saying_one_answers() {
    let folder = folder("coxd2");
    let data = folder.join("data");
    let _first = daemon(&folder, &data);
    let (status, said) = Daemon::start(&folder, &data, NO_REGISTRY).ended();
    assert_eq!(status.code(), Some(1), "{said}");
    assert!(
        said.contains(&format!(
            "another coxd answers on {}",
            data.join("run/coxd.sock").display()
        )),
        "{said}"
    );
}

#[test]
fn the_daemon_ends_well_once_its_socket_is_taken_away() {
    let folder = folder("coxd3");
    let data = folder.join("data");
    let daemon = daemon(&folder, &data);
    std::fs::remove_file(data.join("run/coxd.sock")).unwrap();
    let (status, said) = daemon.ended();
    assert_eq!(status.code(), Some(0), "{said}");
}

/// A registry of agents, served on the machine, as `COX_REGISTRY` names
/// it, listing one agent.
fn registry() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let body = REGISTRY;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    format!("http://{address}/registry.json")
}

const REGISTRY: &str = r#"{"agents": [{"id": "parrot", "name": "Parrot", "version": "1.0.0",
    "distribution": {"npx": {"package": "@example/parrot@1.0.0"}}}]}"#;

#[test]
fn the_registry_a_daemon_fetched_lists_its_agents_on_the_runs_after() {
    let folder = folder("coxd4");
    let data = folder.join("data");
    set(&data, r#"{"phone": false}"#);
    let _daemon = Daemon::start(&folder, &data, &registry());
    cox::storage::keep_on_this_thread(Some(data));
    waits::until("a run after lists the registry's agent", || {
        cox::agents::Agents::read(Vec::new(), None)
            .all()
            .iter()
            .any(|agent| agent.id == "registry:parrot" && agent.name == "Parrot" && agent.listed)
    });
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_helper_off_macos_fails_saying_it_has_no_work() {
    let output = Command::new(env!("CARGO_BIN_EXE_cox-helper"))
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stderr(&output),
        "cox-helper only has work to do on macOS; see src/bin/helper.rs\n"
    );
}
