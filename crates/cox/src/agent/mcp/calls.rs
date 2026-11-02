//! The tools of Cox's MCP server: what the agent learns of them, and what
//! each does when called, through what it asks of the chat as a
//! [`ToolRequest`], or of the window as a [`ChatRequest`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use cox_core::goal::Goal;
use cox_core::tabs::Lines;
use cox_editor::changes::{self, Diff};
use cox_editor::map::{self, Changed, Module};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::agent::mcp::{
    ASKS, Answered, ChatRequest, Ended, INVALID_PARAMS, Ran, ToolRequest, answered,
};
use crate::agent::setup::{Policy, Setup, StageUse};
use crate::chat::trial::Trial;

const SHOW_MARKDOWN: &str = "show_markdown";
const SHOW_FILE: &str = "show_file";
const SHOW_CHANGES: &str = "show_changes";
const SHOW_MAP: &str = "show_map";
const RUN_COMMAND: &str = "run_command";
const OPEN_CHAT: &str = "open_chat";
const WAIT_CHAT: &str = "wait_chat";
const PROPOSE_GOAL: &str = "propose_goal";
const START_TRIAL: &str = "start_trial";

/// The most files the agent is told of, of those that changed.
const FILES_TOLD: usize = 50;

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
    show_changes_tool(),
    show_map_tool(),
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
    open_chat_tool(),
    {
        "name": WAIT_CHAT,
        "description": "Waits for a chat you opened with open_chat to end its turn, or to \
            come to wait for the human, and tells you how it stands then: what its agent \
            last said, or what it asks the human. A chat that is not at work answers at \
            once. Use it after open_chat, to hear how the work went before you go on.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "chat": {
                    "type": "integer",
                    "description": "The number open_chat gave the chat.",
                },
            },
            "required": ["chat"],
        },
    },
    propose_goal_tool(),
    start_trial_tool(),
    ])
}

/// The tool that shows what changed, as the agent learns of it.
fn show_changes_tool() -> Value {
    json!(
    {
        "name": SHOW_CHANGES,
        "description": "Shows the human what changed in the files of a Git repository, \
            file by file and line by line, on a tab of the stage beside the conversation, \
            where a click on a line opens the file there: in a trial, all it changed since \
            it started, and elsewhere what changed since the last commit, with the files \
            Git does not track yet. Use it to show the human what you made, before you talk \
            it through or propose a goal from it. The tab follows the files as they change. \
            Answers the files that changed, and how many lines each gained and lost.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "folder": {
                    "type": "string",
                    "description": "A folder of the repository, absolute or from the working \
                        directory, such as a trial's folder: the working directory unless \
                        given.",
                },
            },
        },
    })
}

/// The tool that shows the map of the project, as the agent learns of it.
fn show_map_tool() -> Value {
    json!(
    {
        "name": SHOW_MAP,
        "description": "Shows the human the map of the project on a tab of the stage beside \
            the conversation: the crates of the Cargo workspace, or the modules in a crate \
            or in a module, as boxes, each with the first paragraph of what its file says \
            of itself and how long it is, with what changed since the last commit laid \
            over them, and, when the human rests on a box, what it uses and what uses it. \
            A click on a box opens the map at it, and a file opens as code. Use it to show \
            the human where what you talk of is in the project, before you talk it through \
            or show the diff: open it at the crate or module you talk of, and the human \
            sees the boxes around it. Answers what the map shows there, box by box.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "at": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Where to open the map: the crate's name, then the \
                        modules in it from the crate down, as [\"cox\", \"stage\"]; the \
                        crates when empty or not given.",
                },
            },
        },
    })
}

/// The tool that makes the chat a trial, as the agent learns of it.
fn start_trial_tool() -> Value {
    json!(
    {
        "name": START_TRIAL,
        "description": "Makes this chat a trial, to try uncertain work out apart from the \
            project before you and the human agree what it is to be: Cox makes a folder of \
            its own for it, and once this turn ends, starts you again there, with the \
            conversation so far, to go on with what you try. There you change and test \
            files without asking the human. `from` is \"project\", a Git worktree of the \
            project's last commit, to try a change to the project, or \"empty\", an empty \
            Git repository, to try something from nothing. Call it, then end your turn. \
            Once you have tried it, show the human what it made with show_changes, and \
            propose a goal from it with propose_goal; the human ends the trial, keeping it or not, and the chat \
            goes on in the project. To try several ways side by side, open a trial of its \
            own for each with open_chat instead.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "from": {
                    "type": "string",
                    "enum": ["project", "empty"],
                    "description": "What the trial starts from: the project's last \
                        commit, or nothing.",
                },
                "what": {
                    "type": "string",
                    "description": "What you try, in a sentence: you are told it again \
                        once you start in the trial.",
                },
            },
            "required": ["from", "what"],
        },
    })
}

/// The tool that proposes a goal for the chat, as the agent learns of it.
fn propose_goal_tool() -> Value {
    json!(
    {
        "name": PROPOSE_GOAL,
        "description": "Proposes a goal for this chat to the human: what the chat works \
            until it is met, either `$ ` and a shell command, which has to pass, or words, \
            which an agent judges by reading the project and running what tests it. Use it \
            once you and the human have talked through what they want, or when they ask \
            for a goal. Make it one that can be checked: name the commands, files or \
            behaviour that show it is met, not the steps to get there. When one command \
            decides it, passing exactly when it is met, such as a test that checks what the \
            human asked for, propose the command: it checks the goal the same way every \
            time, with no judge. The human sees it on a card in the conversation, and sets \
            it as it is, changes it first, or lets it be; it is not the chat's goal until \
            they set it. It answers at once, with the \
            goal the chat has now, if any. Once a goal is set, each time your turn ends Cox \
            runs its command or asks its judge, and while it is not met, tells you why as \
            the next turn.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "The goal: \"$ \" and a shell command, which has to \
                        pass, or words, which an agent judges.",
                },
                "why": {
                    "type": "string",
                    "description": "Why this goal, in a sentence, for the human to weigh it.",
                },
            },
            "required": ["goal"],
        },
    })
}

/// The tool that opens a chat, as the agent learns of it.
fn open_chat_tool() -> Value {
    json!(
    {
        "name": OPEN_CHAT,
        "description": "Opens another chat in this project, under this one in the human's \
            list, with an agent of its own, and sends it `text` as its first line. Use it \
            to hand work to another agent: a unit of change to build, a question to look \
            into, a change to read with fresh eyes. The chat works in the same folder as \
            this one, so two chats that change files at once may collide. The human sees \
            the chat, its conversation and its stage, and can step in. How its agent is \
            set up is yours to say: `context` is told to it after the project's \
            instructions; `tools` names which of Cox's tools it gets, all of them unless \
            given; `stage` is \"look\" to keep it from driving the pages of its stage, or \
            \"drive\" (the default); `allow` and `reject` are patterns of the titles of \
            its permission requests, as they show in the conversation, which Cox answers \
            in the human's place, once each, with `*` for any run of characters (a title \
            both match is rejected, and the human is asked about the rest); `goal` is \
            what the chat works until it is met, as a goal the human sets: `$ ` and a \
            shell command, which has to pass, or words, which an agent judges; `trial` \
            makes the chat a trial, which works in a folder of its own, apart from the \
            project, and changes and tests its files there without asking the human: \
            \"project\", a Git worktree of the project's last commit, to try a change to \
            the project, or \"empty\", an empty Git repository, to try something from \
            nothing. Open trials when the work is uncertain and you would try several \
            ways side by side, one trial each, before you and the human agree what it is \
            to be: read what each made, show the human with show_changes, giving its \
            folder, and propose a goal from it with propose_goal; to try one way, make this chat the trial with start_trial \
            instead. Answers the number the chat goes by, to wait for it with wait_chat.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The first line the chat's agent gets: what to do.",
                },
                "context": {
                    "type": "string",
                    "description": "What the agent is told after the project's instructions, \
                        ahead of the first line, to keep to for the whole session.",
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The tools of Cox's own the agent gets, by name.",
                },
                "stage": {
                    "type": "string",
                    "enum": ["drive", "look"],
                    "description": "Whether the agent gets the tools that drive its stage's \
                        pages, or only shows the human things there.",
                },
                "allow": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Patterns of the titles of permission requests to allow \
                        once in the human's place, such as \"Read *\" or \"cargo *\".",
                },
                "reject": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Patterns of the titles of permission requests to reject \
                        once in the human's place, such as \"Edit *\".",
                },
                "goal": {
                    "type": "string",
                    "description": "What the chat works until it is met: \"$ \" and a shell \
                        command, which has to pass, or words, which an agent judges.",
                },
                "trial": {
                    "type": "string",
                    "enum": ["project", "empty"],
                    "description": "What the chat starts from when it tries the work out \
                        in a folder of its own, apart from the project: the project's last \
                        commit, or nothing.",
                },
            },
            "required": ["text"],
        },
    })
}

/// Calls the tool `params` names, and answers what it did, or the error
/// the agent made in calling it.
pub(super) async fn call(
    params: &Value,
    cwd: &Path,
    only: Option<&[String]>,
    requests: &UnboundedSender<ToolRequest>,
    chats: &UnboundedSender<ChatRequest>,
) -> Answered {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").unwrap_or(&Value::Null);
    // A tool the chat's agent does not get is no tool to it.
    if only.is_some_and(|only| !only.iter().any(|only| only == name)) {
        return Err((INVALID_PARAMS, format!("no tool {name}")));
    }
    let done = match name {
        SHOW_MARKDOWN => show_markdown(arguments, cwd, requests),
        SHOW_FILE => show_file(arguments, cwd, requests),
        SHOW_CHANGES => show_changes(arguments, cwd, requests),
        SHOW_MAP => show_map(arguments, cwd, requests),
        RUN_COMMAND => run_command(arguments, cwd, requests).await,
        OPEN_CHAT => open_chat(arguments, chats).await,
        WAIT_CHAT => wait_chat(arguments, chats).await,
        PROPOSE_GOAL => propose_goal(arguments, chats).await,
        START_TRIAL => start_trial(arguments, chats).await,
        _ => return Err((INVALID_PARAMS, format!("no tool {name}"))),
    };
    Ok(match done {
        Ok(text) => answered(&text, false),
        Err(text) => answered(&text, true),
    })
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
    ask(requests, ToolRequest::ShowMarkdown { path, text })?;
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
    ask(requests, ToolRequest::ShowFile { path, text, lines })?;
    Ok(shown)
}

/// Asks the chat to show what changed in the repository of the folder
/// `arguments` name, and answers what changed.
fn show_changes(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let folder = match arguments.get("folder") {
        None | Some(Value::Null) => cwd.to_owned(),
        Some(folder) => cwd.join(
            folder
                .as_str()
                .ok_or("folder is to be a path, as a string")?,
        ),
    };
    let folder = fs::canonicalize(&folder)
        .map_err(|error| format!("cannot read {}: {error}", folder.display()))?;
    let diff = changes::read(&folder)
        .map_err(|error| format!("cannot tell what changed in {}: {error}", folder.display()))?;
    ask(requests, ToolRequest::ShowChanges { folder })?;
    Ok(told_changes(&diff))
}

/// Asks the chat to show the map of the project open where `arguments`
/// say, and answers what is there.
fn show_map(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let at: Vec<String> = match arguments.get("at") {
        None | Some(Value::Null) => Vec::new(),
        Some(at) => at
            .as_array()
            .ok_or("at is to be a list of names, as strings")?
            .iter()
            .map(|name| {
                name.as_str()
                    .map(str::to_owned)
                    .ok_or("at is to be a list of names, as strings")
            })
            .collect::<Result<_, _>>()?,
    };
    let map = map::read(cwd).map_err(|error| format!("cannot map the project: {error}"))?;
    let place = map.place(&at);
    if place.depth < at.len() {
        let there: Vec<&str> = if place.depth == 0 {
            map.crates.iter().map(|krate| krate.name.as_str()).collect()
        } else {
            place
                .modules
                .iter()
                .map(|module| module.name.as_str())
                .collect()
        };
        return Err(format!(
            "there is no `{}` in {}; there are: {}",
            at[place.depth],
            placed(cwd, &at[..place.depth]),
            there.join(", ")
        ));
    }
    let changed = changes::read(cwd)
        .ok()
        .map(|diff| map::changed_files(&diff));
    ask(requests, ToolRequest::ShowMap { at: at.clone() })?;
    Ok(told_map(&map, &place, changed.as_ref(), &placed(cwd, &at)))
}

/// What the map shows at `place`, `placed` in words, as the agent is told
/// it: a line for each box, with its counts, what `changed` in it, and
/// the first sentence of what it says of itself.
fn told_map(
    map: &map::Map,
    place: &map::Place<'_>,
    changed: Option<&HashMap<PathBuf, (usize, usize)>>,
    placed: &str,
) -> String {
    let mut told = format!(
        "The map of the project is on the stage, open at {placed}, for the human to see where \
         things are:"
    );
    let mut tell = |name: &str, counts: &str, said: &str, changed: Changed| {
        let _ = write!(told, "\n{name}: {counts}");
        if changed.any() {
            let _ = write!(
                told,
                ", +{} -{} in {} file{}",
                changed.added,
                changed.removed,
                changed.files,
                if changed.files == 1 { "" } else { "s" }
            );
        }
        if !said.is_empty() {
            let _ = write!(told, "; {}", first_sentence(said));
        }
    };
    let changed_in =
        |module: &Module| changed.map_or_else(Changed::default, |files| module.changed(files));
    if place.krate.is_none() {
        for krate in &map.crates {
            let mut counts = format!("{} lines", krate.lines());
            if !krate.uses.is_empty() {
                let _ = write!(counts, ", uses {}", krate.uses.join(", "));
            }
            let changed = changed.map_or_else(Changed::default, |files| krate.changed(files));
            tell(&krate.name, &counts, krate.said(), changed);
        }
    } else {
        for file in &place.files {
            let name = file.path.file_name().map_or_else(
                || file.name.clone(),
                |name| name.to_string_lossy().into_owned(),
            );
            tell(
                &name,
                &format!("{} lines", file.lines),
                &file.said,
                changed_in(file),
            );
        }
        for module in &place.modules {
            let counts = if module.modules.is_empty() {
                format!("{} lines", module.lines)
            } else {
                format!(
                    "{} modules, {} lines",
                    module.modules_in_all(),
                    module.lines_in_all()
                )
            };
            tell(&module.name, &counts, &module.said, changed_in(module));
        }
    }
    told
}

/// The first sentence of `said`, what a crate or module says of itself:
/// enough for the agent to know what is where, which the human reads in
/// full on the map.
fn first_sentence(said: &str) -> &str {
    said.find(". ").map_or(said, |end| &said[..=end])
}

/// Where a map of the project in `cwd` is open `at`, in words: the
/// project's name, and the names after it.
fn placed(cwd: &Path, at: &[String]) -> String {
    let project = cwd.file_name().map_or_else(
        || cwd.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    std::iter::once(project)
        .chain(at.iter().cloned())
        .collect::<Vec<_>>()
        .join(" › ")
}

/// Asks `request` of the chat's stage through `requests`, saying so in the
/// log (see [`ASKS`]), for the flow of the call to be read later.
///
/// # Errors
///
/// When the chat has closed.
fn ask(requests: &UnboundedSender<ToolRequest>, request: ToolRequest) -> Result<(), String> {
    tracing::debug!(target: ASKS, "asks the stage to {}", request.said());
    requests
        .send(request)
        .map_err(|_| "the chat has closed".to_owned())
}

/// What changed in `diff`, as the agent is told it.
fn told_changes(diff: &Diff) -> String {
    let top = diff.top.display();
    if diff.files.is_empty() {
        return format!(
            "Nothing changed in {top} {}; the stage shows so.",
            diff.since.said()
        );
    }
    let (added, removed) = diff.counts();
    let files = match diff.files.len() {
        1 => "1 file".to_owned(),
        count => format!("{count} files"),
    };
    let mut told = format!(
        "What changed in {top} {} is on the stage, for the human to read: {files}, +{added} \
         -{removed}.",
        diff.since.said(),
    );
    for file in diff.files.iter().take(FILES_TOLD) {
        let _ = write!(told, "\n{} {}", file.how.letter(), file.path.display());
        if let Some(was) = &file.was {
            let _ = write!(told, " (was {})", was.display());
        }
        if file.binary {
            told.push_str(" binary");
        } else {
            let _ = write!(told, " +{} -{}", file.added, file.removed);
        }
    }
    if diff.files.len() > FILES_TOLD {
        let _ = write!(told, "\n... and {} more", diff.files.len() - FILES_TOLD);
    }
    told
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
    ask(
        requests,
        ToolRequest::RunCommand {
            command: command.to_owned(),
            cwd: cwd.to_owned(),
            answer,
        },
    )?;
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

/// Asks the window to open a chat as `arguments` say, and waits for it to
/// open: answers how the chat goes by.
async fn open_chat(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let text = arguments
        .get("text")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or("no text: give the chat's first line as `text`")?;
    let strings = |name: &str| -> Result<Option<Vec<String>>, String> {
        match arguments.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| format!("`{name}` holds something that is not a string"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(_) => Err(format!("`{name}` is not an array of strings")),
        }
    };
    let string = |name: &str| -> Result<Option<String>, String> {
        match arguments.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(format!("`{name}` is not a string")),
        }
    };
    let stage = match string("stage")?.as_deref() {
        None | Some("drive") => StageUse::Drive,
        Some("look") => StageUse::Look,
        Some(other) => return Err(format!("`stage` is \"{other}\": say \"drive\" or \"look\"")),
    };
    let setup = Setup {
        context: string("context")?.filter(|context| !context.trim().is_empty()),
        tools: strings("tools")?,
        stage,
        policy: Policy {
            allow: strings("allow")?.unwrap_or_default(),
            reject: strings("reject")?.unwrap_or_default(),
        },
        folder: None,
        trial: None,
    };
    let goal = string("goal")?.as_deref().and_then(Goal::parse);
    let trial = string("trial")?
        .map(|name| Trial::parse(&name).ok_or("`trial` is not \"project\" or \"empty\""))
        .transpose()?;
    let (answer, opened) = oneshot::channel();
    chats
        .send(ChatRequest::OpenChat {
            text: text.to_owned(),
            setup,
            goal,
            trial,
            answer,
        })
        .map_err(|_| "the chat has closed".to_owned())?;
    let opened = opened
        .await
        .map_err(|_| "the chat closed while the other opened".to_owned())??;
    let mut told = format!(
        "{} is open under this chat as chat {}, working on your line. Wait for its turn to \
         end with wait_chat, giving {}.",
        opened.name, opened.number, opened.number
    );
    if let (Some(trial), Some(folder)) = (trial, &opened.folder) {
        let _ = write!(
            told,
            " It is a trial, working in {}, {}, apart from the project: read what it made \
             there, and show the human with show_changes, giving that folder.",
            folder.display(),
            trial.what()
        );
    }
    Ok(told)
}

/// Waits for the chat `arguments` name to end its turn, or to come to
/// wait for the human, and tells how it stands.
async fn wait_chat(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let chat = arguments
        .get("chat")
        .and_then(Value::as_u64)
        .ok_or("no chat: give the number open_chat gave it as `chat`")?;
    let (answer, waited) = oneshot::channel();
    chats
        .send(ChatRequest::WaitChat { chat, answer })
        .map_err(|_| "the chat has closed".to_owned())?;
    let waited = waited
        .await
        .map_err(|_| "the chat closed while waiting".to_owned())??;
    let mut told = match &waited.asking {
        Some(asking) => format!("Chat {chat} waits for the human to answer: {asking}."),
        None => format!("Chat {chat} ended its turn."),
    };
    match &waited.said {
        Some(said) => {
            told.push_str(" It last said:\n\n");
            told.push_str(said);
        }
        None => told.push_str(" It said nothing."),
    }
    Ok(told)
}

/// Asks the window to show the human the goal `arguments` propose, and
/// tells the agent the goal the chat has now.
async fn propose_goal(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let goal = arguments
        .get("goal")
        .and_then(Value::as_str)
        .and_then(Goal::parse)
        .ok_or("no goal: give the goal to propose as `goal`")?;
    let why = match arguments.get("why") {
        None | Some(Value::Null) => None,
        Some(Value::String(why)) => Some(why.clone()),
        Some(_) => return Err("`why` is not a string".to_owned()),
    };
    let (answer, now) = oneshot::channel();
    chats
        .send(ChatRequest::ProposeGoal { goal, why, answer })
        .map_err(|_| "the chat has closed".to_owned())?;
    let now = now
        .await
        .map_err(|_| "the chat closed before it showed the goal".to_owned())?;
    let mut told = "The goal is on a card in the conversation, for the human to set, change, \
                    or let be; it is not the chat's goal until they set it."
        .to_owned();
    match now {
        Some(now) => {
            told.push_str(" The chat's goal now: ");
            told.push_str(&now.input());
        }
        None => told.push_str(" The chat has no goal now."),
    }
    Ok(told)
}

/// Asks the window to make the chat a trial, as `arguments` say, and
/// tells the agent where it goes on.
async fn start_trial(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let trial = arguments
        .get("from")
        .and_then(Value::as_str)
        .and_then(Trial::parse)
        .ok_or("`from` is not \"project\" or \"empty\"")?;
    let what = arguments
        .get("what")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|what| !what.is_empty())
        .ok_or("no `what`: say what you try")?
        .to_owned();
    let (answer, made) = oneshot::channel();
    chats
        .send(ChatRequest::StartTrial {
            trial,
            what,
            answer,
        })
        .map_err(|_| "the chat has closed".to_owned())?;
    let folder = made
        .await
        .map_err(|_| "the chat closed before the trial was made".to_owned())??;
    Ok(format!(
        "The trial is ready in {}, {}. End your turn now: Cox then starts you again there, \
         with the conversation so far, to go on with what you try.",
        folder.display(),
        trial.what()
    ))
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
    use crate::agent::mcp::{OpenedChat, Waited};

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
            .block_on(call(&params, cwd, None, requests, &unbounded_channel().0))
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
        assert_eq!(listed[2]["name"], SHOW_CHANGES);
        assert_eq!(listed[3]["name"], SHOW_MAP);
        assert_eq!(listed[4]["name"], RUN_COMMAND);
        assert_eq!(listed[5]["name"], OPEN_CHAT);
        assert_eq!(listed[8]["name"], START_TRIAL);
    }

    #[test]
    fn shows_the_map_and_tells_what_is_there() {
        let cwd = scratch("map");
        fs::write(cwd.join("Cargo.toml"), "[package]\nname = \"one\"\n").unwrap();
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/main.rs"), "//! One.\nmod a;\nmod b;\n").unwrap();
        fs::write(cwd.join("src/a.rs"), "//! A. It says more.\n").unwrap();
        fs::write(cwd.join("src/b.rs"), "//! B,\n//! with more.\nmod c;\n").unwrap();
        fs::create_dir_all(cwd.join("src/b")).unwrap();
        fs::write(cwd.join("src/b/c.rs"), "").unwrap();
        let (sent, mut requests) = unbounded_channel();
        let project = cwd.file_name().unwrap().to_string_lossy().into_owned();

        let shown = call_now(SHOW_MAP, &json!({}), &cwd, &sent);
        assert_eq!(shown["isError"], false, "{shown}");
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert!(
            told.contains(&format!("open at {project}, for the human")),
            "{told}"
        );
        assert!(told.ends_with("\none: 7 lines; One."), "{told}");
        match requests.try_recv() {
            Ok(ToolRequest::ShowMap { at }) => assert_eq!(at, Vec::<String>::new()),
            _ => panic!("the stage was not asked to show the map"),
        }

        let shown = call_now(SHOW_MAP, &json!({ "at": ["one"] }), &cwd, &sent);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert!(
            told.contains(&format!("open at {project} › one,")),
            "{told}"
        );
        assert!(
            told.ends_with(
                "\nmain.rs: 3 lines; One.\na: 1 lines; A.\nb: 1 modules, 3 lines; B, with more."
            ),
            "{told}"
        );
        match requests.try_recv() {
            Ok(ToolRequest::ShowMap { at }) => assert_eq!(at, ["one"]),
            _ => panic!("the stage was not asked to show the map"),
        }

        let shown = call_now(SHOW_MAP, &json!({ "at": ["one", "nowhere"] }), &cwd, &sent);
        assert_eq!(shown["isError"], true);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            told,
            format!("there is no `nowhere` in {project} › one; there are: a, b")
        );
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn asks_the_window_to_make_the_chat_a_trial() {
        let (sent, _requests) = unbounded_channel();
        let (chats, mut asked) = unbounded_channel();
        let window = tokio::spawn(async move {
            let Some(ChatRequest::StartTrial {
                trial,
                what,
                answer,
            }) = asked.recv().await
            else {
                panic!("the window was not asked");
            };
            let _ = answer.send(Ok("/trials/greet-1".into()));
            (trial, what)
        });
        let params = json!({ "name": START_TRIAL, "arguments": {
            "from": "project",
            "what": "A warmer greeting.",
        } });
        let answered = call(&params, Path::new("/work"), None, &sent, &chats)
            .await
            .unwrap();
        assert_eq!(answered["isError"], false);
        let told = answered["content"][0]["text"].as_str().unwrap();
        assert!(
            told.starts_with("The trial is ready in /trials/greet-1, a worktree"),
            "{told}"
        );
        assert_eq!(
            window.await.unwrap(),
            (Trial::Project, "A warmer greeting.".to_owned())
        );

        // It says what it tries, and from what.
        let (chats, mut asked) = unbounded_channel();
        for arguments in [
            json!({ "what": "A greeting." }),
            json!({ "from": "container", "what": "A greeting." }),
            json!({ "from": "empty", "what": " " }),
        ] {
            let params = json!({ "name": START_TRIAL, "arguments": arguments });
            let answered = call(&params, Path::new("/"), None, &sent, &chats)
                .await
                .unwrap();
            assert_eq!(answered["isError"], true, "{arguments}");
        }
        assert!(asked.try_recv().is_err());
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
    fn shows_what_changed_in_a_folder_and_tells_the_agent() {
        let cwd = scratch("changes");
        let trial = cwd.join("trial");
        fs::create_dir_all(&trial).unwrap();
        let git = |args: &[&str]| {
            let ran = std::process::Command::new("git")
                .arg("-C")
                .arg(&trial)
                .args(args)
                .output()
                .unwrap();
            assert!(ran.status.success(), "{ran:?}");
        };
        git(&["init", "--quiet"]);
        changes::start(&trial).unwrap();
        fs::write(trial.join("hello.txt"), "Hello\nthere\n").unwrap();
        let (sent, mut requests) = unbounded_channel();

        let shown = call_now(SHOW_CHANGES, &json!({ "folder": "trial" }), &cwd, &sent);
        assert_eq!(shown["isError"], false);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert!(told.contains("since nothing"), "{told}");
        assert!(
            told.ends_with("1 file, +2 -0.\nA hello.txt +2 -0"),
            "{told}"
        );
        match requests.try_recv() {
            Ok(ToolRequest::ShowChanges { folder }) => assert_eq!(folder, trial),
            _ => panic!("the stage was not asked to show what changed"),
        }

        // A folder out of any repository has no changes to show.
        let outside = scratch("no-changes");
        if std::process::Command::new("git")
            .arg("-C")
            .arg(&outside)
            .args(["rev-parse", "--git-dir"])
            .output()
            .is_ok_and(|ran| !ran.status.success())
        {
            let shown = call_now(SHOW_CHANGES, &json!({}), &outside, &sent);
            assert_eq!(shown["isError"], true);
            assert!(requests.try_recv().is_err());
        }
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
        let answered = call(
            &params,
            Path::new("/work"),
            None,
            &sent,
            &unbounded_channel().0,
        )
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
        let answered = call(&params, Path::new("/"), None, &sent, &unbounded_channel().0)
            .await
            .unwrap();
        assert_eq!(answered["isError"], true);
        assert!(requests.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_goal_in_words_opens_a_chat_judged_by_an_agent() {
        let (sent, _requests) = unbounded_channel();
        let (chats, mut asked) = unbounded_channel();
        let window = tokio::spawn(async move {
            let Some(ChatRequest::OpenChat { goal, answer, .. }) = asked.recv().await else {
                panic!("the window was not asked");
            };
            let _ = answer.send(Ok(OpenedChat {
                number: 3,
                name: "Chat 3".to_owned(),
                folder: None,
            }));
            goal
        });
        let params = json!({ "name": OPEN_CHAT, "arguments": {
            "text": "Make the greeting name the user.",
            "goal": "The greeting names the user.",
        } });
        call(&params, Path::new("/work"), None, &sent, &chats)
            .await
            .unwrap();
        assert_eq!(
            window.await.unwrap(),
            Some(Goal::Words("The greeting names the user.".to_owned()))
        );
    }

    #[tokio::test]
    async fn asks_the_window_to_open_a_chat_as_the_agent_says() {
        let (sent, _requests) = unbounded_channel();
        let (chats, mut asked) = unbounded_channel();
        let window = tokio::spawn(async move {
            let Some(ChatRequest::OpenChat {
                text,
                setup,
                goal,
                trial,
                answer,
            }) = asked.recv().await
            else {
                panic!("the window was not asked");
            };
            let _ = answer.send(Ok(OpenedChat {
                number: 7,
                name: "Chat 2".to_owned(),
                folder: Some("/trials/greet-1".into()),
            }));
            (text, setup, goal, trial)
        });
        let params = json!({ "name": OPEN_CHAT, "arguments": {
            "text": "Build the first unit.",
            "context": "Build one unit and no more.",
            "tools": ["show_file", "run_command"],
            "stage": "look",
            "allow": ["Read *", "cargo *"],
            "reject": ["Edit *"],
            "goal": "$ cargo xtask ci",
            "trial": "project",
        } });
        let answered = call(&params, Path::new("/work"), None, &sent, &chats)
            .await
            .unwrap();
        assert_eq!(answered["isError"], false);
        let told = answered["content"][0]["text"].as_str().unwrap();
        assert!(told.contains("Chat 2") && told.contains("chat 7"), "{told}");
        assert!(
            told.contains("a trial, working in /trials/greet-1"),
            "{told}"
        );
        let (text, setup, goal, trial) = window.await.unwrap();
        assert_eq!(trial, Some(Trial::Project));
        assert_eq!(text, "Build the first unit.");
        assert_eq!(goal, Some(Goal::Command("cargo xtask ci".to_owned())));
        assert_eq!(
            setup,
            Setup {
                context: Some("Build one unit and no more.".to_owned()),
                tools: Some(["show_file", "run_command"].map(str::to_owned).to_vec()),
                stage: StageUse::Look,
                policy: Policy {
                    allow: ["Read *", "cargo *"].map(str::to_owned).to_vec(),
                    reject: ["Edit *"].map(str::to_owned).to_vec(),
                },
                folder: None,
                trial: None,
            }
        );
    }

    #[tokio::test]
    async fn proposes_a_goal_to_the_human_and_tells_the_goal_now() {
        let (sent, _requests) = unbounded_channel();
        let (chats, mut asked) = unbounded_channel();
        let window = tokio::spawn(async move {
            let mut proposed = Vec::new();
            for now in [None, Some(Goal::Command("make".to_owned()))] {
                let Some(ChatRequest::ProposeGoal { goal, why, answer }) = asked.recv().await
                else {
                    panic!("the window was not asked");
                };
                proposed.push((goal, why));
                let _ = answer.send(now);
            }
            proposed
        });
        let propose = |arguments: Value| {
            let params = json!({ "name": PROPOSE_GOAL, "arguments": arguments });
            let (sent, chats) = (sent.clone(), chats.clone());
            async move {
                call(&params, Path::new("/work"), None, &sent, &chats)
                    .await
                    .unwrap()
            }
        };
        let first = propose(json!({
            "goal": "$ cargo test greeting",
            "why": "The test names the user.",
        }))
        .await;
        assert_eq!(first["isError"], false);
        let told = first["content"][0]["text"].as_str().unwrap();
        assert!(told.ends_with("The chat has no goal now."), "{told}");
        let second = propose(json!({ "goal": "The greeting names the user." })).await;
        let told = second["content"][0]["text"].as_str().unwrap();
        assert!(told.ends_with("The chat's goal now: $ make"), "{told}");
        // A goal that is blank is no goal to propose, and asks nothing.
        let blank = propose(json!({ "goal": " " })).await;
        assert_eq!(blank["isError"], true);
        assert_eq!(
            window.await.unwrap(),
            [
                (
                    Goal::Command("cargo test greeting".to_owned()),
                    Some("The test names the user.".to_owned())
                ),
                (Goal::Words("The greeting names the user.".to_owned()), None),
            ]
        );
    }

    #[tokio::test]
    async fn asks_the_window_how_a_chat_stands_once_it_waits() {
        let (sent, _requests) = unbounded_channel();
        let (chats, mut asked) = unbounded_channel();
        let window = tokio::spawn(async move {
            let mut waited_for = Vec::new();
            for stands in [
                Ok(Waited {
                    asking: None,
                    said: Some("Built, and the tests pass.".to_owned()),
                }),
                Ok(Waited {
                    asking: Some("Edit src/main.rs".to_owned()),
                    said: None,
                }),
                Err("no chat 9".to_owned()),
            ] {
                let Some(ChatRequest::WaitChat { chat, answer }) = asked.recv().await else {
                    panic!("the window was not asked");
                };
                waited_for.push(chat);
                let _ = answer.send(stands);
            }
            waited_for
        });
        let wait = |chat: u64| {
            let params = json!({ "name": WAIT_CHAT, "arguments": { "chat": chat } });
            let (sent, chats) = (sent.clone(), chats.clone());
            async move {
                call(&params, Path::new("/work"), None, &sent, &chats)
                    .await
                    .unwrap()
            }
        };
        let ended = wait(7).await;
        assert_eq!(ended["isError"], false);
        assert_eq!(
            ended["content"][0]["text"],
            "Chat 7 ended its turn. It last said:\n\nBuilt, and the tests pass."
        );
        let asking = wait(7).await;
        assert_eq!(
            asking["content"][0]["text"],
            "Chat 7 waits for the human to answer: Edit src/main.rs. It said nothing."
        );
        let none = wait(9).await;
        assert_eq!(none["isError"], true);
        assert_eq!(none["content"][0]["text"], "no chat 9");
        assert_eq!(window.await.unwrap(), [7, 7, 9]);

        // A chat has to be named.
        let params = json!({ "name": WAIT_CHAT, "arguments": { "chat": "seven" } });
        let answered = call(&params, Path::new("/work"), None, &sent, &chats)
            .await
            .unwrap();
        assert_eq!(answered["isError"], true);
    }

    #[tokio::test]
    async fn asks_for_a_first_line_and_a_stage_it_knows() {
        let (sent, _requests) = unbounded_channel();
        let (chats, mut asked) = unbounded_channel();
        for arguments in [
            json!({}),
            json!({ "text": " " }),
            json!({ "text": "go", "stage": "fly" }),
            json!({ "text": "go", "allow": "Read *" }),
            json!({ "text": "go", "trial": true }),
            json!({ "text": "go", "trial": "container" }),
        ] {
            let params = json!({ "name": OPEN_CHAT, "arguments": arguments });
            let answered = call(&params, Path::new("/"), None, &sent, &chats)
                .await
                .unwrap();
            assert_eq!(answered["isError"], true, "{arguments}");
        }
        assert!(asked.try_recv().is_err());
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
