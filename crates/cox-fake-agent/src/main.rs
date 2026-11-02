//! `cox-fake-agent <scenario>` is an agent to run Cox with when no model
//! should answer: to try Cox out, to see a change working, or to make a
//! fault happen again. It speaks ACP on its standard input and output, as
//! `claude-agent-acp` does, and answers each line sent to it as the
//! scenario, a JSON file, says: it replies, thinks, calls tools, asks
//! permission and goes on as the human answers, and calls the tools Cox
//! hands it, such as `open_chat` (see [`scenario`]).

mod agent;
mod mcp;
mod scenario;

use std::process::ExitCode;

const USAGE: &str = "Usage: cox-fake-agent <scenario>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let read = std::fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()));
    match read {
        Ok(scenario) => {
            agent::play(scenario);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("cox-fake-agent: cannot read {path}: {error}");
            ExitCode::from(1)
        }
    }
}
