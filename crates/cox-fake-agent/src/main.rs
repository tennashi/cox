//! `cox-fake-agent <scenario>` is an agent to run Cox with when no model
//! should answer: to try Cox out, to see a change working, or to make a
//! fault happen again. It speaks ACP on its standard input and output, as
//! `claude-agent-acp` does, and answers each line sent to it as the
//! scenario, a JSON file, says: it replies, thinks, calls tools, asks
//! permission and goes on as the human answers, and calls the tools Cox
//! hands it, such as `open_chat` (see [`scenario`]).
//!
//! `cox-fake-agent <scenario> --log-in` logs the agent of the scenario in,
//! as a way to log in that runs in a terminal does.
//!
//! `cox-fake-agent --from-log <run> [--chat <id>]` writes, as a scenario,
//! what a chat's agent did in a run Cox kept, for the agent to do again
//! (see [`log`]).

mod agent;
mod log;
mod mcp;
mod scenario;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: cox-fake-agent <scenario> [--log-in]
       cox-fake-agent --from-log <run> [--chat <id>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, run] if flag == "--from-log" => from_log(run, None),
        [flag, run, chat_flag, chat] if flag == "--from-log" && chat_flag == "--chat" => {
            from_log(run, Some(chat))
        }
        [path] if !path.starts_with("--") => play(path),
        [path, flag] if !path.starts_with("--") && flag == "--log-in" => log_in(path),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Writes the scenario of the chat `chat` of the run kept in `run`.
fn from_log(run: &str, chat: Option<&str>) -> ExitCode {
    let made = std::fs::read_to_string(run)
        .map_err(|error| format!("cannot read {run}: {error}"))
        .and_then(|text| log::scenario(&text, chat));
    match made {
        Ok(scenario) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&scenario).expect("JSON writes")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cox-fake-agent: {error}");
            ExitCode::from(1)
        }
    }
}

/// Logs the agent of the scenario at `path` in, for the runs to come.
fn log_in(path: &str) -> ExitCode {
    let file = agent::logged_in(path.as_ref());
    match std::fs::write(&file, "") {
        Ok(()) => {
            println!("Logged in to the fake agent.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cox-fake-agent: cannot write {}: {error}", file.display());
            ExitCode::from(1)
        }
    }
}

/// Plays the scenario in the file at `path`.
fn play(path: &str) -> ExitCode {
    let read = std::fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()));
    match read {
        Ok(scenario) => {
            agent::play(scenario, &agent::logged_in(path.as_ref()));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cox-fake-agent: cannot read {path}: {error}");
            ExitCode::from(1)
        }
    }
}
