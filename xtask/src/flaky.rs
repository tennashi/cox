//! `cargo xtask flaky`: runs the tests over and over, and tells which
//! failed in how many of the runs, to find a test that fails only now and
//! then.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::process::{Command, Stdio};

use anyhow::{Context, bail};

/// How many times the tests run when the human does not say.
const RUNS: usize = 10;

/// A test that failed in a run: the test binary cargo ran it in, as
/// `cargo test` names its source, with its executable's name, which says
/// the package ("unittests src/lib.rs (cox)", "tests/chats.rs (chats)"),
/// and the test's own name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Failed {
    pub binary: String,
    pub test: String,
}

/// Runs `cargo test` `runs` times, with `args` or `--workspace`, and tells
/// each test that failed in how many runs; fails if one did.
pub fn run(args: &[String]) -> anyhow::Result<()> {
    let (runs, cargo_args) = parse(args)?;
    let mut failed: BTreeMap<Failed, usize> = BTreeMap::new();
    for round in 1..=runs {
        let output = cargo_test(&cargo_args)?;
        let failures = failures(&output);
        eprintln!("run {round}/{runs}: {} failed", failures.len());
        for failure in failures {
            *failed.entry(failure).or_default() += 1;
        }
    }
    if failed.is_empty() {
        println!("No test failed in {runs} runs.");
        return Ok(());
    }
    println!("Failed, in {runs} runs:");
    for (failure, times) in &failed {
        println!("  {times:>3}  {}  {}", failure.binary, failure.test);
    }
    bail!("{} tests failed at least once", failed.len())
}

/// The number of runs, and what goes to `cargo test`, from what follows
/// `flaky`: `[<runs>] [-- <cargo test args>...]`.
fn parse(args: &[String]) -> anyhow::Result<(usize, Vec<String>)> {
    let (ours, theirs) = match args.iter().position(|arg| arg == "--") {
        Some(at) => (&args[..at], args[at + 1..].to_vec()),
        None => (args, Vec::new()),
    };
    let runs = match ours {
        [] => RUNS,
        [runs] => runs
            .parse()
            .ok()
            .filter(|runs| *runs > 0)
            .with_context(|| format!("not a number of runs: {runs}"))?,
        _ => bail!("usage: cargo xtask flaky [<runs>] [-- <cargo test args>...]"),
    };
    let cargo_args = if theirs.is_empty() {
        vec!["--workspace".to_owned()]
    } else {
        theirs
    };
    Ok((runs, cargo_args))
}

/// What `cargo test --no-fail-fast` with `args` writes, its output and
/// its errors in the order it wrote them, which says which binary each
/// test ran in.
fn cargo_test(args: &[String]) -> anyhow::Result<String> {
    let (mut reader, writer) = std::io::pipe()?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut child = Command::new(cargo)
        .arg("test")
        .arg("--no-fail-fast")
        .args(args)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        .spawn()
        .context("cargo test does not start")?;
    let mut output = String::new();
    reader.read_to_string(&mut output)?;
    child.wait()?;
    Ok(output)
}

/// The tests that failed in what `cargo test` wrote, each with the binary
/// it ran in; a binary that did not build or run is told as its test
/// "(did not run)".
#[must_use]
pub fn failures(output: &str) -> Vec<Failed> {
    let mut binary = String::new();
    let mut failed = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if let Some(running) = line.strip_prefix("Running ") {
            binary = named(running);
        } else if let Some(test) = line
            .strip_prefix("test ")
            .and_then(|rest| rest.strip_suffix(" ... FAILED"))
        {
            failed.push(Failed {
                binary: binary.clone(),
                test: test.to_owned(),
            });
        } else if line.starts_with("error: could not compile") {
            failed.push(Failed {
                binary: line.to_owned(),
                test: "(did not run)".to_owned(),
            });
        }
    }
    failed
}

/// The binary of `running`, what follows cargo's "Running": the source it
/// was built from, and its executable's name, which says the package, as
/// `unittests src/lib.rs (cox_browser)`.
fn named(running: &str) -> String {
    let Some((source, executable)) = running.rsplit_once(" (") else {
        return running.to_owned();
    };
    let file = executable
        .trim_end_matches(')')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    let name = file.rsplit_once('-').map_or(file, |(name, _)| name);
    format!("{source} ({name})")
}

#[cfg(test)]
mod tests {
    use super::{Failed, RUNS, failures, parse};

    fn failed(binary: &str, test: &str) -> Failed {
        Failed {
            binary: binary.to_owned(),
            test: test.to_owned(),
        }
    }

    #[test]
    fn a_failed_test_is_told_with_the_binary_it_ran_in() {
        let output = "\
   Compiling cox v0.0.0
     Running unittests src/lib.rs (target/debug/deps/cox-1a2b)
running 2 tests
test vim::tests::escape ... ok
test vim::tests::count ... FAILED
     Running tests/chats.rs (target/debug/deps/chats-3c4d)
running 1 test
test a_turn_ends ... FAILED
test result: FAILED. 0 passed; 1 failed
";
        assert_eq!(
            failures(output),
            [
                failed("unittests src/lib.rs (cox)", "vim::tests::count"),
                failed("tests/chats.rs (chats)", "a_turn_ends"),
            ]
        );
    }

    #[test]
    fn passing_tests_are_no_failures() {
        let output = "     Running tests/vim.rs (x)\ntest a ... ok\ntest b ... ignored\n";
        assert_eq!(failures(output), []);
    }

    #[test]
    fn a_binary_that_does_not_build_is_a_failure() {
        let output = "error: could not compile `cox` (test \"chats\") due to 1 previous error\n";
        assert_eq!(
            failures(output),
            [failed(
                "error: could not compile `cox` (test \"chats\") due to 1 previous error",
                "(did not run)"
            )]
        );
    }

    #[test]
    fn runs_and_cargo_args_are_taken_apart_at_the_double_dash() {
        let args = |args: &[&str]| args.iter().map(|&arg| arg.to_owned()).collect::<Vec<_>>();
        assert_eq!(parse(&[]).unwrap(), (RUNS, args(&["--workspace"])));
        assert_eq!(parse(&args(&["3"])).unwrap(), (3, args(&["--workspace"])));
        assert_eq!(
            parse(&args(&["5", "--", "-p", "cox", "--test", "chats"])).unwrap(),
            (5, args(&["-p", "cox", "--test", "chats"]))
        );
        assert!(parse(&args(&["0"])).is_err(), "no runs is refused");
        assert!(parse(&args(&["many"])).is_err());
    }
}
