//! The tools of Cox's MCP server: what the agent learns of them, and what
//! each does when called, through what it asks of the chat as a
//! [`ToolRequest`].

use std::fs;
use std::path::Path;

use cox_core::tabs::Lines;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::agent::mcp::{Ended, INVALID_PARAMS, Ran, ToolRequest};

const SHOW_MARKDOWN: &str = "show_markdown";
const SHOW_FILE: &str = "show_file";
const RUN_COMMAND: &str = "run_command";

/// The most of a command's output the agent gets: the end of it, where a
/// build or a test run says how it went.
const OUTPUT_LENGTH: usize = 30_000;

/// The tools, as the agent learns of them: those named in `only`, or all
/// of them.
pub(super) fn tools(only: Option<&[String]>) -> Value {
    let mut tools = all_tools();
    if let Some(only) = only
        && let Some(tools) = tools.as_array_mut()
    {
        tools.retain(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| only.iter().any(|only| only == name))
        });
    }
    tools
}

/// Every tool, as the agent learns of it.
fn all_tools() -> Value {
    json!([
    {
        "name": SHOW_MARKDOWN,
        "description": "Shows the human a Markdown file, drawn as Markdown, on a tab of the \
            stage beside the conversation, where they read it while you talk. Use it for \
            what the human should read, such as a plan or notes you wrote. The tab follows \
            the file as it changes, so there is no need to show a file again after you \
            change it. A file shown already comes back to the front.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file, absolute or from the working directory.",
                },
            },
            "required": ["path"],
        },
    },
    {
        "name": SHOW_FILE,
        "description": "Shows the human a file as it is written, such as code, on a tab of \
            the stage beside the conversation, at the lines you name, which the tab marks \
            and scrolls to. Use it to point the human at the code you talk about, rather \
            than quoting it. The tab follows the file as it changes. A file shown already \
            comes back to the front, at the lines named now. For a plan or notes the human \
            should read as Markdown, use show_markdown instead.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file, absolute or from the working directory.",
                },
                "first_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The first line to show, counted from 1.",
                },
                "last_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The last line to show, counted from 1: the first line \
                        unless given.",
                },
            },
            "required": ["path"],
        },
    },
    {
        "name": RUN_COMMAND,
        "description": "Runs a shell command in a terminal on the stage beside the \
            conversation, where the human watches it run, and can type into it. Use it for \
            the commands the human should see, such as builds and tests. It runs in the \
            working directory with the human's shell (`$SHELL -c`), afresh each time, so a \
            `cd` does not carry over to the next. It waits until the command ends, and \
            answers what the terminal shows, cut to the end when long, and the exit code. \
            A command that waits for input waits for the human; one that never ends, such \
            as a server, holds you until the human stops it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command, as the shell reads it.",
                },
            },
            "required": ["command"],
        },
    },
    ])
}

/// Calls the tool `params` names, and answers what it did, or the error
/// the agent made in calling it.
pub(super) async fn call(
    params: &Value,
    cwd: &Path,
    only: Option<&[String]>,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    // A tool the chat's agent does not get is no tool to it.
    if only.is_some_and(|only| !only.iter().any(|only| only == name)) {
        return Err((INVALID_PARAMS, format!("no tool {name}")));
    }
    let done = match name {
        SHOW_MARKDOWN => show_markdown(arguments, cwd, requests),
        SHOW_FILE => show_file(arguments, cwd, requests),
        RUN_COMMAND => run_command(arguments, cwd, requests).await,
        _ => return Err((INVALID_PARAMS, format!("no tool {name}"))),
    };
    let (text, is_error) = match done {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    }))
}

/// Reads the file `arguments` names, and asks the chat to show it.
fn show_markdown(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or("no path: give the file to show as `path`")?;
    let path = cwd.join(path);
    let read = fs::canonicalize(&path).and_then(|path| Ok((fs::read_to_string(&path)?, path)));
    let (text, path) = read.map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let shown = format!("{} is on the stage, for the human to read.", path.display());
    requests
        .send(ToolRequest::ShowMarkdown { path, text })
        .map_err(|_| "the chat has closed".to_owned())?;
    Ok(shown)
}

/// Reads the file `arguments` names, and asks the chat to show it at the
/// lines they name.
fn show_file(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or("no path: give the file to show as `path`")?;
    let line = |name: &str| match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(line) => line
            .as_u64()
            .and_then(|line| usize::try_from(line).ok())
            .filter(|&line| line > 0)
            .map(Some)
            .ok_or(format!("{name} is to be a line number, from 1")),
    };
    let lines = match (line("first_line")?, line("last_line")?) {
        (Some(first), last) => Some(Lines {
            first,
            last: last.unwrap_or(first),
        }),
        (None, Some(_)) => return Err("last_line needs a first_line".to_owned()),
        (None, None) => None,
    };
    let path = cwd.join(path);
    let read = fs::canonicalize(&path).and_then(|path| Ok((fs::read_to_string(&path)?, path)));
    let (text, path) = read.map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let count = text.split('\n').count();
    let lines = match lines {
        Some(lines) if lines.last < lines.first => {
            return Err("last_line comes before first_line".to_owned());
        }
        Some(lines) if lines.first > count => {
            return Err(format!("{} has {count} lines", path.display()));
        }
        Some(lines) => Some(Lines {
            last: lines.last.min(count),
            ..lines
        }),
        None => None,
    };
    let shown = match lines {
        Some(lines) => format!(
            "{} is on the stage at lines {}-{}, for the human to read.",
            path.display(),
            lines.first,
            lines.last,
        ),
        None => format!("{} is on the stage, for the human to read.", path.display()),
    };
    requests
        .send(ToolRequest::ShowFile { path, text, lines })
        .map_err(|_| "the chat has closed".to_owned())?;
    Ok(shown)
}

/// Asks the chat to run the command `arguments` holds, and waits for it to
/// end: answers what it printed and how it ended, an error unless it
/// exited with 0.
async fn run_command(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let command = arguments
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or("no command: give the command to run as `command`")?;
    let (answer, ran) = oneshot::channel();
    requests
        .send(ToolRequest::RunCommand {
            command: command.to_owned(),
            cwd: cwd.to_owned(),
            answer,
        })
        .map_err(|_| "the chat has closed".to_owned())?;
    let ran = ran
        .await
        .map_err(|_| "the chat closed while the command ran".to_owned())??;
    let told = told(&ran);
    if ran.ended == Ended::Exited(0) {
        Ok(told)
    } else {
        Err(told)
    }
}

/// What the agent is told of a command that ran: its output, or the end
/// of it, and how it ended.
fn told(ran: &Ran) -> String {
    let output = ran.output.trim_end();
    let output = match output.char_indices().rev().nth(OUTPUT_LENGTH - 1) {
        // Cut at a line, past the cut.
        Some((cut, _)) => {
            let kept = &output[cut..];
            let kept = kept.split_once('\n').map_or(kept, |(_, rest)| rest);
            let cut = output[..output.len() - kept.len()].lines().count();
            format!("[the first {cut} lines are cut]\n{kept}")
        }
        None => output.to_owned(),
    };
    if output.is_empty() {
        format!("[no output, {}]", ran.ended)
    } else {
        format!("{output}\n\n[{}]", ran.ended)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    /// What the agent gets from calling the tool `name` with `arguments`
    /// in `cwd`, for a call answered right away.
    fn call_now(
        name: &str,
        arguments: &Value,
        cwd: &Path,
        requests: &UnboundedSender<ToolRequest>,
    ) -> Value {
        let params = json!({ "name": name, "arguments": arguments });
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(call(&params, cwd, None, requests))
            .unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("cox-mcp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::canonicalize(directory).unwrap()
    }

    #[test]
    fn lists_the_tools() {
        let listed = tools(None);
        assert_eq!(listed[0]["name"], SHOW_MARKDOWN);
        assert_eq!(listed[1]["name"], SHOW_FILE);
        assert_eq!(listed[2]["name"], RUN_COMMAND);
    }

    #[test]
    fn shows_a_file_from_the_working_directory() {
        let cwd = scratch("show");
        fs::write(cwd.join("plan.md"), "# Plan\n").unwrap();
        let (sent, mut requests) = unbounded_channel();

        let shown = call_now(SHOW_MARKDOWN, &json!({ "path": "plan.md" }), &cwd, &sent);

        assert_eq!(shown["isError"], false);
        match requests.try_recv().unwrap() {
            ToolRequest::ShowMarkdown { path, text } => {
                assert_eq!(path, cwd.join("plan.md"));
                assert_eq!(text, "# Plan\n");
            }
            request => panic!("asked {request:?}"),
        }
        fs::remove_dir_all(cwd).unwrap();
    }
    /// Calls `show_file` in `cwd` with `arguments`, and answers what it
    /// answers, with what the chat is asked.
    fn show_file(cwd: &Path, arguments: &Value) -> (Value, Option<ToolRequest>) {
        let (sent, mut requests) = unbounded_channel();
        let shown = call_now(SHOW_FILE, arguments, cwd, &sent);
        (shown, requests.try_recv().ok())
    }

    #[test]
    fn shows_a_file_at_the_lines_named() {
        let cwd = scratch("show-file");
        fs::write(cwd.join("main.rs"), "fn main() {\n}\n").unwrap();

        let (shown, asked) = show_file(
            &cwd,
            &json!({ "path": "main.rs", "first_line": 2, "last_line": 9 }),
        );

        assert_eq!(shown["isError"], false);
        assert_eq!(
            shown["content"][0]["text"],
            format!(
                "{} is on the stage at lines 2-3, for the human to read.",
                cwd.join("main.rs").display()
            )
        );
        match asked {
            Some(ToolRequest::ShowFile { path, text, lines }) => {
                assert_eq!(path, cwd.join("main.rs"));
                assert_eq!(text, "fn main() {\n}\n");
                assert_eq!(lines, Some(Lines { first: 2, last: 3 }));
            }
            request => panic!("asked {request:?}"),
        }

        let (_, asked) = show_file(&cwd, &json!({ "path": "main.rs", "first_line": 1 }));
        assert!(
            matches!(
                asked,
                Some(ToolRequest::ShowFile {
                    lines: Some(Lines { first: 1, last: 1 }),
                    ..
                })
            ),
            "{asked:?}"
        );
        fs::remove_dir_all(cwd).unwrap();
    }

    #[test]
    fn tells_the_agent_of_lines_that_are_not_there() {
        let cwd = scratch("show-file-lines");
        fs::write(cwd.join("main.rs"), "fn main() {}\n").unwrap();
        let fails = |arguments: Value| {
            let (shown, asked) = show_file(&cwd, &arguments);
            assert_eq!(shown["isError"], true);
            assert!(asked.is_none());
            shown["content"][0]["text"].as_str().unwrap().to_owned()
        };

        assert!(fails(json!({ "path": "main.rs", "first_line": 5 })).ends_with("has 2 lines"));
        assert_eq!(
            fails(json!({ "path": "main.rs", "first_line": 2, "last_line": 1 })),
            "last_line comes before first_line"
        );
        assert_eq!(
            fails(json!({ "path": "main.rs", "first_line": 0 })),
            "first_line is to be a line number, from 1"
        );
        assert!(fails(json!({ "path": "gone.rs" })).starts_with("cannot read"));
        fs::remove_dir_all(cwd).unwrap();
    }

    #[test]
    fn tells_the_agent_of_a_file_it_cannot_read() {
        let cwd = scratch("missing");
        let (sent, mut requests) = unbounded_channel();

        let shown = call_now(SHOW_MARKDOWN, &json!({ "path": "missing.md" }), &cwd, &sent);

        assert_eq!(shown["isError"], true);
        let text = shown["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("cannot read"), "{text}");
        assert!(requests.try_recv().is_err());
        fs::remove_dir_all(cwd).unwrap();
    }

    /// Calls `run_command` with `arguments`, and answers the request the
    /// chat gets with `ran`.
    async fn run(arguments: Value, ran: Result<Ran, String>) -> Value {
        let (sent, mut requests) = unbounded_channel();
        let chat = tokio::spawn(async move {
            match requests.recv().await {
                Some(ToolRequest::RunCommand {
                    command,
                    cwd,
                    answer,
                }) => {
                    assert_eq!(cwd, Path::new("/work"));
                    answer.send(ran).unwrap();
                    Some(command)
                }
                _ => None,
            }
        });
        let params = json!({ "name": RUN_COMMAND, "arguments": arguments });
        let answered = call(&params, Path::new("/work"), None, &sent)
            .await
            .unwrap();
        drop(sent);
        let asked = chat.await.unwrap();
        assert_eq!(asked.as_deref(), params["arguments"]["command"].as_str());
        answered
    }

    #[tokio::test]
    async fn answers_what_a_command_printed_and_how_it_ended() {
        let ran = Ran {
            output: "ok\n\n".to_owned(),
            ended: Ended::Exited(0),
        };
        let result = run(json!({ "command": "true" }), Ok(ran)).await;
        assert_eq!(result["isError"], false);
        assert_eq!(result["content"][0]["text"], "ok\n\n[exit code 0]");

        let ran = Ran {
            output: "error: no\n".to_owned(),
            ended: Ended::Exited(2),
        };
        let result = run(json!({ "command": "false" }), Ok(ran)).await;
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "error: no\n\n[exit code 2]");
    }

    #[tokio::test]
    async fn tells_the_agent_a_command_did_not_start() {
        let result = run(json!({ "command": "true" }), Err("no pty".to_owned())).await;
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "no pty");
    }

    #[tokio::test]
    async fn asks_for_a_command() {
        let (sent, mut requests) = unbounded_channel();
        let params = json!({ "name": RUN_COMMAND, "arguments": { "command": " " } });
        let answered = call(&params, Path::new("/"), None, &sent).await.unwrap();
        assert_eq!(answered["isError"], true);
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn keeps_the_end_of_a_long_output() {
        let output = (0..10_000).fold(String::new(), |output, line| {
            output + &format!("line {line}\n")
        });
        let told = told(&Ran {
            output,
            ended: Ended::Signal(2),
        });
        assert!(told.len() < OUTPUT_LENGTH + 100, "{}", told.len());
        let (cut, rest) = told.split_once('\n').unwrap();
        let first: usize = rest.lines().next().unwrap()["line ".len()..]
            .parse()
            .unwrap();
        assert_eq!(cut, format!("[the first {first} lines are cut]"));
        assert!(told.ends_with("line 9999\n\n[ended by signal 2]"), "{told}");
    }
}
