//! `cox-fake-emulator`, run as Cox and the human run it: its arguments,
//! what it prints, and how it exits.

#[path = "../src/scratch.rs"]
mod scratch;

use std::process::{Command, Output};

use cox_android::avd::{self, Avd};
use scratch::Scratch;

/// The fake, run with `args` in `home`, as its `HOME`, and no AVDs'
/// folder of the human's.
fn run(home: &Scratch, args: &[&str], avd_home: Option<&std::path::Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cox-fake-emulator"));
    command
        .args(args)
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", home.path())
        .env_remove("ANDROID_AVD_HOME");
    if let Some(avd_home) = avd_home {
        command.env("ANDROID_AVD_HOME", avd_home);
    }
    command.output().expect("the fake runs")
}

fn printed(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn it_lists_the_one_device_it_plays() {
    let home = Scratch::new("cli-list");
    let output = run(&home, &["-list-avds"], None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(printed(&output), "Fake_Phone\n");
}

#[test]
fn make_avd_writes_the_device_it_plays_where_android_avd_home_says() {
    let home = Scratch::new("cli-make-avd");
    let avds = home.path().join("avds");
    let output = run(&home, &["--make-avd"], Some(&avds));
    assert_eq!(output.status.code(), Some(0));
    let folder = avds.join("Fake_Phone.avd");
    assert_eq!(printed(&output), format!("{}\n", folder.display()));
    assert_eq!(
        avd::in_folder(&avds),
        [Avd {
            name: "Fake_Phone".into(),
            title: "Fake Phone".into(),
            model: Some("cox_fake".into()),
            api: Some("35".into()),
            display: Some((540, 1170)),
            folder,
        }]
    );
}

#[test]
fn make_avd_with_no_android_avd_home_fails_and_writes_nothing() {
    let home = Scratch::new("cli-make-avd-unset");
    let output = run(&home, &["--make-avd"], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty(), "it says why");
    let written = std::fs::read_dir(home.path()).expect("the home").count();
    assert_eq!(written, 0, "nothing is written in its home");
}

#[test]
fn a_grpc_port_that_is_no_port_fails_before_serving() {
    let home = Scratch::new("cli-bad-grpc");
    let output = run(&home, &["-avd", "Fake_Phone", "-grpc", "port"], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty(), "it says why");
    let written = std::fs::read_dir(home.path()).expect("the home").count();
    assert_eq!(written, 0, "it writes no running file");
}
