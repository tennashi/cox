//! The tools of Cox's MCP server: what the agent learns of them, and what
//! each does when called, through what it asks of the chat as a
//! [`ToolRequest`], or of the window as a [`ChatRequest`].

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use cox_core::goal::Goal;
use cox_core::tabs::Lines;
use cox_editor::changes::{self, Diff};
use cox_editor::map::{self, Facts, Map, Tier};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::agent::mcp::{
    ASKS, Answered, ChatRequest, Ended, INVALID_PARAMS, Ran, ToolRequest, Whereabouts, answered,
};
use crate::agent::setup::{Policy, Setup, StageUse};
use crate::chat::sketch::Sketch;
use crate::{raw, wiki};

const SHOW_MARKDOWN: &str = "show_markdown";
const SHOW_FILE: &str = "show_file";
const SHOW_CHANGES: &str = "show_changes";
const SHOW_MAP: &str = "show_map";
const RUN_COMMAND: &str = "run_command";
const OPEN_CHAT: &str = "open_chat";
const WAIT_CHAT: &str = "wait_chat";
const PROPOSE_GOAL: &str = "propose_goal";
const START_SKETCH: &str = "start_sketch";
const WIKI_SEARCH: &str = "wiki_search";
const WIKI_READ: &str = "wiki_read";
const WIKI_WRITE: &str = "wiki_write";
const RAW_KEEP: &str = "raw_keep";
const RAW_SEARCH: &str = "raw_search";
const RAW_READ: &str = "raw_read";

/// How the map's file is written, as the agent is told it when it is to
/// write one: the form the Map tab draws, not what to put in it, which is
/// the project's to decide.
const MAP_FORMAT: &str = "\
A project has a map for each picture of it from a top of its own, as one of what it is made \
of and one of how it is deployed, each a file in `.cox/maps` in the project's folder, named \
after the map, as `.cox/maps/parts.json` for the map `parts` (`.cox/map.json`, as an earlier \
Cox read it, is the map `map`). A map is a JSON object with `says`, one line on what it shows \
as against the project's other maps, `tiers`, `parts`, `edges` and `flows`. `tiers` is a list, the top tier first, each `{\"id\", \"name\", \"says\", \"rows\", \
\"frame\"}`: `says` is one line on what the tier shows; `rows` are the lines of boxes, from the \
top, each `{\"name\", \"parts\": [ids], \"under\": id}`, where `under` hangs the row under a \
box of a row above, as what that box holds; `frame` is `{\"above\": [ids], \"below\": [ids]}`, \
boxes of other tiers drawn above and below the tier when it is open, for the lines out of it \
to reach. `parts` is an object of the boxes by id, each `{\"name\", \"says\", \"each\", \
\"many\", \"holds\", \"tier\", \"kinds\", \"note\"}`: `says` is one line on what it is; `each` \
says how many there are and of what (\"one per chat\"), with `many: true` when there are \
several; `holds` are the paths of the code it is, from the project's folder, a file or a \
folder, a file with a line as `path:12`; `tier` is the id of the tier that opens inside it; \
`kinds` names the kinds of a box that is not one thing; `note` is a remark, such as where the \
code does not match the map. `edges` is a list of `{\"from\", \"to\", \"says\", \"dir\"}` \
between boxes, by id, `dir` being `to` (an arrow into `to`, the default), `both` or `none`; a \
tier draws the edges among its boxes and those to the boxes in its frame. `flows` is a list \
of `{\"id\", \"name\", \"says\", \"steps\"}`, the orders in which boxes talk, each step \
`{\"from\", \"to\", \"says\", \"at\", \"back\"}` from one box to another, by id, written between \
the boxes that hold the code: the tab rolls the steps up to the boxes of the tier it shows, or \
of the tiers above it, draws them in sequence, and opens the box a step happens inside of; `at` is where the step is \
made, a path with a line, the line of the call, which the tab asks the project's language server what it calls, to \
name the step by that function where it is drawn between the boxes it is written between, so give every step its \
`at`; `back: true` marks a step \
back, the answer to an earlier step, which ends what the box was doing since that step. A box opens the tier \
it names, or, holding paths, the code; a box may be in more than one tier. The tab lays what \
changed since the last commit on the boxes that hold the files, and up on the boxes that open \
their tiers, so a box that opens a tier holds paths only for the code that is its own, and \
marks a box whose paths are gone stale. Change the map as you move the code it holds.";

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
    let mut tools = json!([
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
    start_sketch_tool(),
    ]);
    if let Some(all) = tools.as_array_mut() {
        all.extend(wiki_tools());
        all.extend(raw_tools());
    }
    tools
}

/// The tools that keep what the agent reads, and find it again, as the
/// agent learns of them.
fn raw_tools() -> [Value; 3] {
    [
        json!({
            "name": RAW_KEEP,
            "description": "Keeps a source you read, as it was: a web page, a paper, a \
                release note or a command's output, as the material of research the \
                human has you do, apart from what is made of it, as a report or a page \
                of the wiki, which then names the sources it was made from, so that it \
                can be checked against them, or made again. Give the text as you read \
                it, as Markdown, not a summary of it, and leave out what is not the \
                source's own, as a site's menus. Sources are kept by topic, as the \
                research they are of; Cox notes where each was read, the day and this \
                chat, and names it after the day and its title. A source is never \
                written again: keep a newer one beside it. Answers the source's name, \
                for raw_read, and its file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "What the research is of, as words and `/`, as \
                            `rust-news`: the folder the source is kept in.",
                    },
                    "from": {
                        "type": "string",
                        "description": "Where it was read: its URL, or the command or \
                            the file it came from.",
                    },
                    "title": {
                        "type": "string",
                        "description": "Its title, as the source names itself.",
                    },
                    "text": {
                        "type": "string",
                        "description": "What it says, as Markdown, without its title.",
                    },
                },
                "required": ["topic", "from", "title", "text"],
            },
        }),
        json!({
            "name": RAW_SEARCH,
            "description": "Searches the sources kept with raw_keep, of a topic or of all: \
                answers those that have every word of the query, whatever the case, \
                those that have them most first, each by its name and title with the \
                first lines that have them. Read one with raw_read.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The words to look for, as `never type`.",
                    },
                    "topic": {
                        "type": "string",
                        "description": "The topic to search, as `rust-news`; all unless \
                            given.",
                    },
                },
                "required": ["query"],
            },
        }),
        json!({
            "name": RAW_READ,
            "description": "Reads a source kept with raw_keep, whole, with where it was \
                read, the day and the chat that kept it in its properties. Without a \
                source, lists those of the topic, or of all, the last kept first, as \
                to see what research has found since a day.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "The source's name, as raw_keep or raw_search \
                            gives it, as `rust-news/2026-11-06-announcing-rust-1-92-0`.",
                    },
                    "topic": {
                        "type": "string",
                        "description": "Without a source, the topic to list, as \
                            `rust-news`; all unless given.",
                    },
                },
            },
        }),
    ]
}

/// The tools that read and write the human's wiki, as the agent learns
/// of them.
fn wiki_tools() -> [Value; 3] {
    [
        json!({
            "name": WIKI_SEARCH,
            "description": "Searches the human's wiki: Markdown pages by topic, of what \
                the agents of all the human's projects learned, such as why a choice was \
                made, what a library needs, or what failed and why. Answers the pages \
                that have every word of the query, whatever the case, those that have \
                them most first, each by its name and title with the first lines that \
                have them, but the pages another project keeps to itself. Read a page \
                with wiki_read.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The words to look for, as `gpui scroll`.",
                    },
                },
                "required": ["query"],
            },
        }),
        json!({
            "name": WIKI_READ,
            "description": "Reads a page of the human's wiki, whole, with its links to \
                other pages as [[name]] and, under \"Used in\", where it was used: the \
                day, the project, the chat, the commit and the code; a page this \
                project keeps to itself says so in its properties, as `only: <project>`. \
                Without a page, reads the index, which lists the pages.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "page": {
                        "type": "string",
                        "description": "The page's name, as wiki_search gives it, as \
                            `gpui-layout` or `rust/errors`; the index unless given.",
                    },
                },
            },
        }),
        json!({
            "name": WIKI_WRITE,
            "description": "Writes a page of the human's wiki, whole, in place of what it \
                said: read it first with wiki_read when it is there, and keep what still \
                holds. A page is of one topic, starting with its `# ` title and a line on \
                what it is about, and links to others as [[name]]. Write it in English, \
                whatever language the chat speaks, so that every chat finds it by the same \
                words; the human reads it in their own language, as Cox translates it. Cox adds, under \
                \"Used in\", where this chat used it: the day, the project, the chat, the \
                commit its folder is at, and the paths you give. What holds only of this \
                project, as of its own people, customers or plans, keep in it with \
                keep_in_project, so that only its chats find the page. Answers the page's \
                file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "page": {
                        "type": "string",
                        "description": "The page's name: words and `/`, as `gpui-layout` \
                            or `rust/errors`.",
                    },
                    "text": {
                        "type": "string",
                        "description": "The whole page, as Markdown, from its `# ` title.",
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "The paths of the code the page is of, from the \
                            working directory, as `src/view.rs`.",
                    },
                    "keep_in_project": {
                        "type": "boolean",
                        "description": "true keeps the page in this project, for its chats \
                            alone; false lets it out to every project; as it was unless \
                            given.",
                    },
                },
                "required": ["page", "text"],
            },
        }),
    ]
}

/// The tool that shows what changed, as the agent learns of it.
fn show_changes_tool() -> Value {
    json!(
    {
        "name": SHOW_CHANGES,
        "description": "Shows the human what changed in the files of a Git repository, \
            file by file and line by line, on a tab of the stage beside the conversation, \
            where a click on a line opens the file there: in a sketch, all it changed since \
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
                        directory, such as a sketch's folder: the working directory unless \
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
        "description": "Shows the human the map of the project on a tab of the stage \
            beside the conversation: a picture of the project in tiers, from the few things \
            it is among at the top, through what is inside each, down to boxes that hold \
            paths of the code, with what changed since the last commit laid over the boxes, \
            and the boxes whose paths are gone marked stale. A click on a box opens the tier \
            inside it, or the code it holds. Use it to show the human where what you talk \
            of is in the project, before you talk it through or show the diff: open it at \
            the tier or box you talk of, and the human sees the boxes around it. Answers \
            what the map shows there, box by box, and names the project's other maps. A \
            map is a file of the project, in `.cox/maps`, which you write when the project \
            has none, or when the human wants another picture of it from another top, \
            deciding with the human what it is to show, and keep as you change the code it \
            holds; shown with none, the tool says how the file is written.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "map": {
                    "type": "string",
                    "description": "Which map to open, by its name, its file's in \
                        `.cox/maps` without `.json`; the project's first when not given.",
                },
                "at": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Where to open the map: the id of a tier, and after it \
                        the id of a box in that tier to open on its own, as [\"cox\", \
                        \"stage\"]; the top tier when empty or not given.",
                },
            },
        },
    })
}

/// The tool that opens a sketch beside the chat, as the agent learns of it.
fn start_sketch_tool() -> Value {
    json!(
    {
        "name": START_SKETCH,
        "description": "Opens a sketch beside this chat: a chat of its own, under this one, \
            which works apart from the project, in a folder Cox makes for it, and changes \
            and tests files there without asking the human. Its agent is set up as you are, \
            and told what this chat talked through so far, and what it tries. This chat \
            stays in the project. `from` is \"project\", a Git worktree of the project's \
            last commit, to try a change to the project, or \"empty\", an empty Git \
            repository, to try something from nothing. The human ends the sketch, and this \
            chat hears how it ended.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "from": {
                    "type": "string",
                    "enum": ["project", "empty"],
                    "description": "What the sketch starts from: the project's last \
                        commit, or nothing.",
                },
                "what": {
                    "type": "string",
                    "description": "What the sketch tries, in a sentence: its agent is \
                        told it, and the sketch is named after it.",
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
            shell command, which has to pass, or words, which an agent judges; `sketch` \
            makes the chat a sketch, which works in a folder of its own, apart from the \
            project, and changes and tests its files there without asking the human: \
            \"project\", a Git worktree of the project's last commit, to try a change to \
            the project, or \"empty\", an empty Git repository, to try something from \
            nothing. Answers the number the chat goes by, to wait for it with wait_chat.",
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
                    "description": "Patterns of the permission requests to allow once in \
                        the human's place: of their titles, as the agent titles its calls, \
                        where * stands for any run of characters, or, after the name of one \
                        of ACP's kinds of tool (read, edit, delete, move, search, execute, \
                        think, fetch, switch_mode, other) and a colon, of the titles of the \
                        calls of that kind, such as \"read:*\", \"search:*\" or \
                        \"execute:cargo test*\".",
                },
                "reject": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Patterns of the permission requests to reject once in \
                        the human's place, written as for allow, such as \"edit:*\" or \
                        \"execute:git push*\"; a request both match is rejected.",
                },
                "goal": {
                    "type": "string",
                    "description": "What the chat works until it is met: \"$ \" and a shell \
                        command, which has to pass, or words, which an agent judges.",
                },
                "sketch": {
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
        START_SKETCH => start_sketch(arguments, chats).await,
        WIKI_SEARCH => wiki_search(arguments, chats).await,
        WIKI_READ => wiki_read(arguments, chats).await,
        WIKI_WRITE => wiki_write(arguments, cwd, chats).await,
        RAW_KEEP => raw_keep(arguments, chats).await,
        RAW_SEARCH => raw_search(arguments),
        RAW_READ => raw_read(arguments),
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
/// say, a tier and a box in it by their ids, or at its top, and answers
/// what is there; or, when the project has no map, how to write one.
fn show_map(
    arguments: &Value,
    cwd: &Path,
    requests: &UnboundedSender<ToolRequest>,
) -> Result<String, String> {
    let at: Vec<String> = match arguments.get("at") {
        None | Some(Value::Null) => Vec::new(),
        Some(at) => at
            .as_array()
            .ok_or("at is to be a list of ids, as strings")?
            .iter()
            .map(|name| {
                name.as_str()
                    .map(str::to_owned)
                    .ok_or("at is to be a list of ids, as strings")
            })
            .collect::<Result<_, _>>()?,
    };
    let name = match arguments.get("map") {
        None | Some(Value::Null) => None,
        Some(name) => Some(
            name.as_str()
                .ok_or("map is to be a map's name, as a string")?,
        ),
    };
    let Some((named, map)) = map::read(cwd, name)? else {
        ask(
            requests,
            ToolRequest::ShowMap {
                map: None,
                at: Vec::new(),
            },
        )?;
        return Ok(format!(
            "The project has no map yet: the Map tab on the stage says so. Write one, and \
             show it again. What the tiers are, and what goes in each, is for you and the \
             human to decide for this project, as its COX.md may say: the map says what the \
             parts of the project mean, for a reader who does not know it, not what its \
             files are. {MAP_FORMAT}"
        ));
    };
    let tier = match at.first() {
        None => map.tiers.first().ok_or("the map has no tier")?,
        Some(id) => map.tier(id).ok_or_else(|| {
            let there: Vec<&str> = map.tiers.iter().map(|tier| tier.id.as_str()).collect();
            format!("the map has no tier `{id}`; it has: {}", there.join(", "))
        })?,
    };
    let part = match at.get(1) {
        None => None,
        Some(id) => Some(
            tier.parts()
                .find(|part| *part == id)
                .ok_or_else(|| {
                    format!(
                        "the tier `{}` has no box `{id}`; it has: {}",
                        tier.id,
                        tier.parts().collect::<Vec<_>>().join(", ")
                    )
                })?
                .to_owned(),
        ),
    };
    if at.len() > 2 {
        return Err("at is a tier's id, and a box's after it, at most".to_owned());
    }
    let changed = changes::read(cwd)
        .ok()
        .map(|diff| changes::changed_files(&diff))
        .unwrap_or_default();
    let all = map::all_facts(cwd, &map, &changed);
    let facts = |id: &str| all.get(id).cloned();
    ask(
        requests,
        ToolRequest::ShowMap {
            map: name.map(str::to_owned),
            at: at.clone(),
        },
    )?;
    let others: Vec<map::Named> = map::list(cwd)
        .into_iter()
        .filter(|other| other.name != named.name)
        .collect();
    Ok(told_map(
        &map,
        &named,
        &others,
        tier,
        part.as_deref(),
        &facts,
    ))
}

/// What the map `named` shows at `tier`, or at the box `part` of it, as
/// the agent is told it: a line for each box, with how many there are,
/// what it says of itself, what it holds, what changed in it, and what
/// of it is gone; then the project's `others` maps, if it has others.
fn told_map(
    map: &Map,
    named: &map::Named,
    others: &[map::Named],
    tier: &Tier,
    part: Option<&str>,
    facts: &dyn Fn(&str) -> Option<Facts>,
) -> String {
    let which = if others.is_empty() {
        "The map of the project".to_owned()
    } else {
        format!("The map `{}` of the project", named.name)
    };
    let mut told = format!(
        "{which} is on the stage, open at the tier `{}`{}, for the human to see where things \
         are:",
        tier.id,
        part.map_or_else(String::new, |part| format!(", at the box `{part}`"))
    );
    if !tier.says.is_empty() {
        let _ = write!(told, "\n{}", tier.says);
    }
    let ids: Vec<&str> = match part {
        Some(part) => Vec::from([part]),
        None => tier.parts().collect(),
    };
    for id in ids {
        let Some(part) = map.parts.get(id) else {
            let _ = write!(told, "\n`{id}`: not in the map's parts");
            continue;
        };
        let _ = write!(told, "\n`{id}` {}", part.name);
        if let Some(each) = &part.each {
            let _ = write!(told, " ({each})");
        }
        if !part.says.is_empty() {
            let _ = write!(told, ": {}", part.says);
        }
        if let Some(opens) = &part.tier {
            let _ = write!(told, "; opens the tier `{opens}`");
        }
        if !part.holds.is_empty() {
            let _ = write!(told, "; holds {}", part.holds.join(", "));
        }
        if let Some(facts) = facts(id) {
            if facts.changed.any() {
                let _ = write!(
                    told,
                    "; +{} -{} in {} file{}",
                    facts.changed.added,
                    facts.changed.removed,
                    facts.changed.files,
                    if facts.changed.files == 1 { "" } else { "s" }
                );
            }
            if !facts.stale.is_empty() {
                let _ = write!(told, "; stale, gone: {}", facts.stale.join(", "));
            }
        }
        if let Some(note) = &part.note {
            let _ = write!(told, "; note: {note}");
        }
    }
    if !others.is_empty() {
        let _ = write!(told, "\nThe project's other maps, which `map` shows:");
        for other in others {
            let _ = write!(told, "\n`{}`", other.name);
            if !other.says.is_empty() {
                let _ = write!(told, ": {}", other.says);
            }
        }
    }
    told
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
            within: None,
        },
        folder: None,
        sketch: None,
    };
    let goal = string("goal")?.as_deref().and_then(Goal::parse);
    let sketch = string("sketch")?
        .map(|name| Sketch::parse(&name).ok_or("`sketch` is not \"project\" or \"empty\""))
        .transpose()?;
    let (answer, opened) = oneshot::channel();
    chats
        .send(ChatRequest::OpenChat {
            text: text.to_owned(),
            setup,
            goal,
            sketch,
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
    if let (Some(sketch), Some(folder)) = (sketch, &opened.folder) {
        let _ = write!(
            told,
            " {}",
            crate::workflow::SKETCH_OPENED.say(&[
                ("folder", &folder.display().to_string()),
                ("from", sketch.what()),
            ])
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

/// Asks the window to open a sketch beside the chat, as `arguments` say,
/// and tells the agent how it goes by.
async fn start_sketch(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let sketch = arguments
        .get("from")
        .and_then(Value::as_str)
        .and_then(Sketch::parse)
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
        .send(ChatRequest::StartSketch {
            sketch,
            what,
            show: false,
            answer,
        })
        .map_err(|_| "the chat has closed".to_owned())?;
    let opened = made
        .await
        .map_err(|_| "the chat closed before the sketch was made".to_owned())??;
    let mut told = format!(
        "The sketch {} is open beside this chat as chat {}, trying it. Wait for its turn to \
         end with wait_chat, giving {}.",
        opened.name, opened.number, opened.number
    );
    if let Some(folder) = &opened.folder {
        let _ = write!(
            told,
            " {}",
            crate::workflow::SKETCH_OPENED.say(&[
                ("folder", &folder.display().to_string()),
                ("from", sketch.what()),
            ])
        );
    }
    Ok(told)
}

/// Where the chat is, as its workspace tells it.
async fn whereabouts(chats: &UnboundedSender<ChatRequest>) -> Result<Whereabouts, String> {
    let (answer, heard) = oneshot::channel();
    chats
        .send(ChatRequest::Whereabouts { answer })
        .map_err(|_| "the chat has closed".to_owned())?;
    heard.await.map_err(|_| "the chat has closed".to_owned())
}

/// Searches the wiki for what `arguments` ask, and tells what it found.
async fn wiki_search(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .filter(|query| !query.trim().is_empty())
        .ok_or("no query: give the words to look for as `query`")?;
    let project = whereabouts(chats).await?.project;
    let found = wiki::search(&crate::settings::wiki(), query, &project);
    if found.is_empty() {
        return Ok(format!("No page of the wiki has every word of `{query}`."));
    }
    let mut told = format!("The pages of the wiki that have every word of `{query}`:");
    for page in found {
        let _ = write!(told, "\n\n{}: {}", page.name, page.title);
        for line in page.lines {
            let _ = write!(told, "\n  {line}");
        }
    }
    Ok(told)
}

/// Reads the page of the wiki `arguments` name, or its index.
async fn wiki_read(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let page = match arguments.get("page") {
        None | Some(Value::Null) => wiki::INDEX,
        Some(Value::String(page)) => page,
        Some(_) => return Err("`page` is not a string".to_owned()),
    };
    let root = crate::settings::wiki();
    let project = whereabouts(chats).await?.project;
    match wiki::read(&root, page, &project) {
        Ok(text) => Ok(text),
        Err(_) if page == wiki::INDEX => {
            let pages = wiki::pages(&root);
            if pages.is_empty() {
                Ok("The wiki has no pages yet.".to_owned())
            } else {
                Ok(format!(
                    "The wiki has no index; its pages: {}",
                    pages.join(", ")
                ))
            }
        }
        Err(error) => Err(format!("{error}: find its name with wiki_search")),
    }
}

/// Writes the page of the wiki `arguments` give, noting where the chat,
/// in `cwd`, used it.
async fn wiki_write(
    arguments: &Value,
    cwd: &Path,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let page = arguments
        .get("page")
        .and_then(Value::as_str)
        .ok_or("no page: give the page's name as `page`")?;
    let text = arguments
        .get("text")
        .and_then(Value::as_str)
        .ok_or("no text: give the whole page as `text`")?;
    let paths = match arguments.get("paths") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(paths)) => paths
            .iter()
            .map(|path| {
                path.as_str()
                    .map(str::to_owned)
                    .ok_or("`paths` holds something that is not a string")
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`paths` is not an array of strings".to_owned()),
    };
    let keep = match arguments.get("keep_in_project") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(keep)) => Some(*keep),
        Some(_) => return Err("`keep_in_project` is not true or false".to_owned()),
    };
    let Whereabouts {
        project,
        chat,
        kept,
    } = whereabouts(chats).await?;
    let used = wiki::Used {
        day: wiki::today(),
        project,
        chat,
        kept,
        commit: wiki::commit_of(cwd),
        paths,
    };
    let root = crate::settings::wiki();
    let file = wiki::write(&root, page, text, &used, keep)?;
    let kept_in = fs::read_to_string(&file)
        .ok()
        .and_then(|page| wiki::only(&page))
        .map(|project| format!(" It is kept in {project}, for its chats alone."))
        .unwrap_or_default();
    Ok(format!(
        "{} is written, used {}.{kept_in}",
        file.display(),
        used.line().trim_start_matches("- ")
    ))
}

/// Keeps the source `arguments` give, noting the chat that kept it.
async fn raw_keep(
    arguments: &Value,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let given = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("no {key}: give the source's {key} as `{key}`"))
    };
    let topic = given("topic")?;
    let source = raw::Source {
        from: given("from")?,
        title: given("title")?,
        text: given("text")?,
    };
    let Whereabouts {
        project,
        chat,
        kept,
    } = whereabouts(chats).await?;
    let mut by = format!("in {project}, chat \"{chat}\"");
    if let Some(kept) = kept {
        let _ = write!(by, " (#{kept})");
    }
    let root = crate::settings::raw();
    let name = raw::keep(&root, topic, source, &by, &wiki::today())?;
    let file = wiki::file(&root, &name)?;
    Ok(format!(
        "The source is kept as {name}, in {}. Name it where you use it.",
        file.display()
    ))
}

/// The topic `arguments` give, if they give one.
fn topic(arguments: &Value) -> Result<Option<&str>, String> {
    match arguments.get("topic") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(topic)) => Ok(Some(topic.as_str())),
        Some(_) => Err("`topic` is not a string".to_owned()),
    }
}

/// Searches the sources for what `arguments` ask, and tells what it
/// found.
fn raw_search(arguments: &Value) -> Result<String, String> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .filter(|query| !query.trim().is_empty())
        .ok_or("no query: give the words to look for as `query`")?;
    let topic = topic(arguments)?;
    let found = raw::search(&crate::settings::raw(), query, topic);
    if found.is_empty() {
        return Ok(format!("No source kept has every word of `{query}`."));
    }
    let mut told = format!("The sources kept that have every word of `{query}`:");
    for source in found {
        let _ = write!(told, "\n\n{}: {}", source.name, source.title);
        for line in source.lines {
            let _ = write!(told, "\n  {line}");
        }
    }
    Ok(told)
}

/// Reads the source `arguments` name, or lists the sources.
fn raw_read(arguments: &Value) -> Result<String, String> {
    let root = crate::settings::raw();
    match arguments.get("source") {
        None | Some(Value::Null) => {
            let topic = topic(arguments)?;
            let sources = raw::list(&root, topic);
            let of = topic
                .map(|topic| format!(" of {topic}"))
                .unwrap_or_default();
            if sources.is_empty() {
                Ok(format!("No source{of} is kept."))
            } else {
                Ok(format!(
                    "The sources{of}, the last kept first:\n{}",
                    sources.join("\n")
                ))
            }
        }
        Some(Value::String(source)) => raw::read(&root, source)
            .map_err(|error| format!("{error}: find its name with raw_read or raw_search")),
        Some(_) => Err("`source` is not a string".to_owned()),
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
        assert_eq!(listed[8]["name"], START_SKETCH);
    }

    #[test]
    fn shows_the_map_and_tells_what_is_there() {
        let cwd = scratch("map");
        fs::create_dir_all(cwd.join(".cox")).unwrap();
        fs::create_dir_all(cwd.join("src")).unwrap();
        fs::write(cwd.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(
            cwd.join(map::MAP_FILE),
            r#"{
              "tiers": [
                { "id": "whole", "name": "Whole", "says": "The few things.",
                  "rows": [{ "parts": ["user", "app"] }] },
                { "id": "app", "name": "The app", "rows": [{ "parts": ["main", "gone"] }] }
              ],
              "parts": {
                "user": { "name": "User", "says": "Who runs it.", "each": "one" },
                "app": { "name": "App", "says": "The program.", "tier": "app", "holds": ["src"] },
                "main": { "name": "main.rs", "holds": ["src/main.rs:1"] },
                "gone": { "name": "Gone", "holds": ["src/gone.rs"], "note": "moved" }
              }
            }"#,
        )
        .unwrap();
        let (sent, mut requests) = unbounded_channel();

        let shown = call_now(SHOW_MAP, &json!({}), &cwd, &sent);
        assert_eq!(shown["isError"], false, "{shown}");
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            told,
            "The map of the project is on the stage, open at the tier `whole`, for the human \
             to see where things are:\nThe few things.\n`user` User (one): Who runs it.\n`app` \
             App: The program.; opens the tier `app`; holds src"
        );
        match requests.try_recv() {
            Ok(ToolRequest::ShowMap { map, at }) => {
                assert_eq!(map, None);
                assert_eq!(at, Vec::<String>::new());
            }
            _ => panic!("the stage was not asked to show the map"),
        }

        let shown = call_now(SHOW_MAP, &json!({ "at": ["app", "gone"] }), &cwd, &sent);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            told,
            "The map of the project is on the stage, open at the tier `app`, at the box \
             `gone`, for the human to see where things are:\n`gone` Gone; holds src/gone.rs; \
             stale, gone: src/gone.rs; note: moved"
        );
        match requests.try_recv() {
            Ok(ToolRequest::ShowMap { map, at }) => {
                assert_eq!(map, None);
                assert_eq!(at, ["app", "gone"]);
            }
            _ => panic!("the stage was not asked to show the map"),
        }

        let shown = call_now(SHOW_MAP, &json!({ "at": ["nowhere"] }), &cwd, &sent);
        assert_eq!(shown["isError"], true);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert_eq!(told, "the map has no tier `nowhere`; it has: whole, app");
        let shown = call_now(SHOW_MAP, &json!({ "at": ["app", "nope"] }), &cwd, &sent);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert_eq!(told, "the tier `app` has no box `nope`; it has: main, gone");
        assert!(requests.try_recv().is_err());

        // With another map, each names the other, and is shown by its
        // name.
        fs::create_dir_all(cwd.join(map::MAPS_FOLDER)).unwrap();
        fs::write(
            cwd.join(map::MAPS_FOLDER).join("deploy.json"),
            r#"{ "says": "Where it runs.",
                 "tiers": [{ "id": "hosts", "name": "Hosts", "rows": [{ "parts": ["box"] }] }],
                 "parts": { "box": { "name": "A box" } } }"#,
        )
        .unwrap();
        let shown = call_now(SHOW_MAP, &json!({ "map": "deploy" }), &cwd, &sent);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            told,
            "The map `deploy` of the project is on the stage, open at the tier `hosts`, for \
             the human to see where things are:\n`box` A box\nThe project's other maps, \
             which `map` shows:\n`map`"
        );
        match requests.try_recv() {
            Ok(ToolRequest::ShowMap { map, at }) => {
                assert_eq!(map.as_deref(), Some("deploy"));
                assert_eq!(at, Vec::<String>::new());
            }
            _ => panic!("the stage was not asked to show the map"),
        }
        let shown = call_now(SHOW_MAP, &json!({}), &cwd, &sent);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert!(told.starts_with("The map `map` of the project"), "{told}");
        assert!(told.ends_with("\n`deploy`: Where it runs."), "{told}");
        let shown = call_now(SHOW_MAP, &json!({ "map": "gone" }), &cwd, &sent);
        assert_eq!(shown["isError"], true);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert_eq!(told, "the project has no map `gone`; it has: map, deploy");
    }

    #[test]
    fn tells_how_to_write_the_map_when_the_project_has_none() {
        let cwd = scratch("no-map");
        let (sent, mut requests) = unbounded_channel();

        let shown = call_now(SHOW_MAP, &json!({}), &cwd, &sent);
        assert_eq!(shown["isError"], false, "{shown}");
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert!(told.starts_with("The project has no map yet"), "{told}");
        assert!(told.contains("`.cox/maps/parts.json`"), "{told}");
        match requests.try_recv() {
            Ok(ToolRequest::ShowMap { map, at }) => {
                assert_eq!(map, None);
                assert_eq!(at, Vec::<String>::new());
            }
            _ => panic!("the stage was not asked to show the map"),
        }
    }

    #[tokio::test]
    async fn asks_the_window_to_make_the_chat_a_sketch() {
        let (sent, _requests) = unbounded_channel();
        let (chats, mut asked) = unbounded_channel();
        let window = tokio::spawn(async move {
            let Some(ChatRequest::StartSketch {
                sketch,
                what,
                show,
                answer,
            }) = asked.recv().await
            else {
                panic!("the window was not asked");
            };
            assert!(!show);
            let _ = answer.send(Ok(OpenedChat {
                number: 3,
                name: "A warmer greeting.".to_owned(),
                folder: Some("/sketches/greet-1".into()),
            }));
            (sketch, what)
        });
        let params = json!({ "name": START_SKETCH, "arguments": {
            "from": "project",
            "what": "A warmer greeting.",
        } });
        let answered = call(&params, Path::new("/work"), None, &sent, &chats)
            .await
            .unwrap();
        assert_eq!(answered["isError"], false);
        let told = answered["content"][0]["text"].as_str().unwrap();
        assert!(
            told.starts_with(
                "The sketch A warmer greeting. is open beside this chat as chat 3, trying it."
            ),
            "{told}"
        );
        assert!(
            told.contains("working in /sketches/greet-1, a worktree"),
            "{told}"
        );
        assert_eq!(
            window.await.unwrap(),
            (Sketch::Project, "A warmer greeting.".to_owned())
        );

        // It says what it tries, and from what.
        let (chats, mut asked) = unbounded_channel();
        for arguments in [
            json!({ "what": "A greeting." }),
            json!({ "from": "container", "what": "A greeting." }),
            json!({ "from": "empty", "what": " " }),
        ] {
            let params = json!({ "name": START_SKETCH, "arguments": arguments });
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
        let sketch = cwd.join("sketch");
        fs::create_dir_all(&sketch).unwrap();
        let git = |args: &[&str]| {
            let ran = std::process::Command::new("git")
                .arg("-C")
                .arg(&sketch)
                .args(args)
                .output()
                .unwrap();
            assert!(ran.status.success(), "{ran:?}");
        };
        git(&["init", "--quiet"]);
        changes::start(&sketch).unwrap();
        fs::write(sketch.join("hello.txt"), "Hello\nthere\n").unwrap();
        let (sent, mut requests) = unbounded_channel();

        let shown = call_now(SHOW_CHANGES, &json!({ "folder": "sketch" }), &cwd, &sent);
        assert_eq!(shown["isError"], false);
        let told = shown["content"][0]["text"].as_str().unwrap();
        assert!(told.contains("since nothing"), "{told}");
        assert!(
            told.ends_with("1 file, +2 -0.\nA hello.txt +2 -0"),
            "{told}"
        );
        match requests.try_recv() {
            Ok(ToolRequest::ShowChanges { folder }) => assert_eq!(folder, sketch),
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
                sketch,
                answer,
            }) = asked.recv().await
            else {
                panic!("the window was not asked");
            };
            let _ = answer.send(Ok(OpenedChat {
                number: 7,
                name: "Chat 2".to_owned(),
                folder: Some("/sketches/greet-1".into()),
            }));
            (text, setup, goal, sketch)
        });
        let params = json!({ "name": OPEN_CHAT, "arguments": {
            "text": "Build the first unit.",
            "context": "Build one unit and no more.",
            "tools": ["show_file", "run_command"],
            "stage": "look",
            "allow": ["Read *", "cargo *"],
            "reject": ["Edit *"],
            "goal": "$ cargo xtask ci",
            "sketch": "project",
        } });
        let answered = call(&params, Path::new("/work"), None, &sent, &chats)
            .await
            .unwrap();
        assert_eq!(answered["isError"], false);
        let told = answered["content"][0]["text"].as_str().unwrap();
        assert!(told.contains("Chat 2") && told.contains("chat 7"), "{told}");
        assert!(
            told.contains("a sketch, working in /sketches/greet-1"),
            "{told}"
        );
        let (text, setup, goal, sketch) = window.await.unwrap();
        assert_eq!(sketch, Some(Sketch::Project));
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
                    within: None,
                },
                folder: None,
                sketch: None,
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
            json!({ "text": "go", "sketch": true }),
            json!({ "text": "go", "sketch": "container" }),
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
