//! Subagents as chats of their own: `agent`, which takes the place of the
//! agent's own tool for subagents (Claude Code's `Agent`, or `Task`), with
//! its arguments, so that a skill that hands work to subagents hands it to
//! chats the human sees under this one, and can step into, and
//! `agent_report`, which waits for a subagent's report.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cox_core::goal::Goal;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::agent::mcp::{ChatRequest, OpenedChat, Subagent, Waited};
use crate::agent::setup::{Policy, Setup, StageUse, Withheld};
use crate::chat::sketch::Sketch;

pub(super) const AGENT: &str = crate::agent::mcp::AGENT_TOOL;
pub(super) const AGENT_REPORT: &str = "agent_report";

/// How long `agent_report` waits for a subagent in a chat that hands its
/// work off (see [`crate::daemon::hands_off`]): long enough for a quick
/// subagent to report in the turn, short enough that a line the human
/// sends meanwhile, which the agent takes only once the call ends, is not
/// left waiting. Cox tells the agent the report once it ends either way.
pub(super) const HANDS_OFF_WAIT: Duration = Duration::from_secs(30);

/// What the agent learns first of `agent` in a session whose own tools
/// for subagents are turned off for it to take their place (see
/// [`crate::daemon::session_meta`]): an agent that reaches for one of
/// those, as a skill says to, finds no such tool, and may take it that it
/// cannot run subagents, and do the work itself, in the human's way.
const IN_PLACE_OF_AGENT: &str = "The Agent and Task tools are not in this session: this \
    tool, mcp__cox__agent, takes their place, so that the human sees each subagent. Wherever \
    a skill or an instruction says to use Agent or Task, or to spawn a subagent, call this \
    instead, with the same arguments, `subagent_type` included, rather than doing the work \
    yourself.";

/// Has the agent learn of `tool`, `agent`, as taking the place of its own
/// tools for subagents: told so first, and loaded always, rather than
/// found only when the agent searches for it, as Claude Code defers the
/// tools of an MCP server, so that an agent that looks for its own sees
/// it.
pub(super) fn in_place_of_agent(tool: &mut Value) {
    let description = tool["description"].as_str().unwrap_or_default();
    tool["description"] = Value::String(format!("{IN_PLACE_OF_AGENT} {description}"));
    tool["_meta"] = json!({ "anthropic/alwaysLoad": true });
}

/// The tool that runs a subagent, as the agent learns of it.
#[expect(clippy::too_many_lines, reason = "its arguments, one after another")]
pub(super) fn agent_tool() -> Value {
    json!({
        "name": AGENT,
        "description": "Runs a subagent: another agent, with a context of its own, in a chat \
            Cox opens in this project, under this one in the human's list, which does the \
            work in `prompt` and reports what it found. It takes the place of the Agent tool \
            in this session: use it wherever a skill or an instruction says to spawn a \
            subagent, a Task or an Agent, with the same arguments. The human sees the \
            subagent's chat, its conversation and its stage, and can step in. Without \
            `run_in_background`, it waits for the subagent to end its turn and answers its \
            report; with it, it answers at once the title the chat goes by in the human's \
            list, and Cox tells you the report unasked, in a line of its own once your turn \
            is not running, unless agent_report, giving that title, waited for it first. \
            Name a subagent to the human by its title, as they see it. The chat works in the \
            same folder as this one, so two that change files at once may collide, unless \
            `sketch` gives it a folder of its own. Beside the Agent tool's arguments, \
            Cox's own say how the subagent is set up: `context` is told to it after the \
            project's instructions; `tools` names which of Cox's tools it gets, all of them \
            unless given; `stage` is \"look\" to keep it from driving the pages of its \
            stage, or \"drive\" (the default); `allow` and `reject` are patterns of the \
            titles of its permission requests, as they show in the conversation, which Cox \
            answers in the human's place, once each, with `*` for any run of characters (a \
            title both match is rejected, and the human is asked about the rest). It may \
            do no more than this chat: it gets only the tools this chat gets, drives its \
            stage only if this chat does, rejects what this chat's setup rejects too, and \
            allows only a pattern this chat's setup allows as written, or covers with `*` \
            or a kind and `:*`; a chat the human set up with nothing allows nothing, and \
            the answer says what was held back. `goal` is what the subagent works until it \
            is met, as a goal the human sets: `$ ` and a shell command, which has to pass, \
            or words, which an agent judges; `sketch` has it work in a folder of its own, \
            apart from the project, and change and test its files there without asking the \
            human: \"project\", a Git worktree of the project's last commit, to try a \
            change to the project, or \"empty\", an empty Git repository, to try something \
            from nothing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "A short title for the work, which the subagent's chat \
                        goes by in the human's list. When the work is a step of your plan, \
                        the step's own words, which the human's Activity page shows the \
                        chat under.",
                },
                "prompt": {
                    "type": "string",
                    "description": "What the subagent is to do: its first line.",
                },
                "subagent_type": {
                    "type": "string",
                    "description": "The kind of subagent, as the Agent tool takes it, such as \
                        \"general-purpose\", one defined in an agents folder, or a plugin's \
                        \"plugin:name\". Its definition is told to the subagent.",
                },
                "model": {
                    "type": "string",
                    "description": "The model the subagent runs with, as the Agent tool takes \
                        it, such as \"opus\" or \"sonnet\", with an effort after it if you \
                        will, as \"opus xhigh\". It is set when the subagent's agent offers \
                        it; the answer says when it does not.",
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Whether to answer at once, with the title of the \
                        subagent's chat, rather than its report.",
                },
                "context": {
                    "type": "string",
                    "description": "What the subagent is told after the project's \
                        instructions, ahead of the prompt, to keep to for the whole session.",
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The tools of Cox's own the subagent gets, by name.",
                },
                "stage": {
                    "type": "string",
                    "enum": ["drive", "look"],
                    "description": "Whether the subagent gets the tools that drive its \
                        stage's pages, or only shows the human things there.",
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
                        \"execute:cargo test*\". Only the patterns this chat's own setup \
                        allows are kept.",
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
                    "description": "What the subagent works until it is met: \"$ \" and a \
                        shell command, which has to pass, or words, which an agent judges.",
                },
                "sketch": {
                    "type": "string",
                    "enum": ["project", "empty"],
                    "description": "What the subagent starts from when it tries the work out \
                        in a folder of its own, apart from the project: the project's last \
                        commit, or nothing.",
                },
            },
            "required": ["description", "prompt"],
        },
    })
}

/// The tool that waits for a subagent's report, as the agent learns of it.
pub(super) fn agent_report_tool() -> Value {
    json!({
        "name": AGENT_REPORT,
        "description": "Waits for a subagent you ran with agent, or a sketch you opened, to \
            end its turn, or to come to ask the human, and answers what it last said, its \
            report, or what it asks. One that is not at work answers at once. Cox tells you \
            the report of a subagent you ran in the background unasked, once your turn is not \
            running: use this to wait for one before you go on, or to read a report Cox cut \
            short in full.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "The title agent answered for the subagent's chat, as \
                        the human sees it in the list.",
                },
                "chat": {
                    "type": "integer",
                    "description": "The number agent answered for the subagent's chat, in \
                        place of `title` when two of your subagents share a title.",
                },
            },
        },
    })
}

/// Where the definitions of the kinds of subagent are: the folder of the
/// agent's own settings, as Claude Code's, if there is one, and the
/// project's folder.
pub(super) struct Kinds<'a> {
    pub config: Option<PathBuf>,
    pub project: &'a Path,
}

impl<'a> Kinds<'a> {
    /// The kinds Claude Code defines for the human, in `$CLAUDE_CONFIG_DIR`
    /// or `~/.claude`, and for the project in `project`.
    pub(super) fn of_claude(project: &'a Path) -> Self {
        let config = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|config| !config.is_empty())
            .map(PathBuf::from)
            .or_else(|| crate::home::home().map(|home| home.join(".claude")));
        Self { config, project }
    }

    /// The file that defines the subagent `kind`: a plugin's, as
    /// `plugin:name`, in the plugins' cache, its highest version, or else
    /// the project's, or the human's.
    fn definition(&self, kind: &str) -> Option<PathBuf> {
        let file = format!("{}.md", kind.rsplit(':').next().unwrap_or(kind));
        if let Some((plugin, _)) = kind.split_once(':') {
            let cache = self.config.as_ref()?.join("plugins").join("cache");
            return std::fs::read_dir(cache)
                .ok()?
                .flatten()
                .filter_map(|market| std::fs::read_dir(market.path().join(plugin)).ok())
                .flat_map(Iterator::flatten)
                .map(|version| version.path())
                .filter(|version| version.join("agents").join(&file).is_file())
                .max_by_key(|version| version_of(version))
                .map(|version| version.join("agents").join(&file));
        }
        let project = self.project.join(".claude").join("agents");
        let human = self.config.as_ref().map(|config| config.join("agents"));
        std::iter::once(project)
            .chain(human)
            .map(|folder| folder.join(&file))
            .find(|path| path.is_file())
    }
}

/// The numbers of the version a plugin's folder is named after, as
/// `0.15.36`, to find its highest.
fn version_of(folder: &Path) -> Vec<u64> {
    folder
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .split(['.', '-', '+'])
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// The model a definition's front matter names, as `model: opus`, unless
/// it says to inherit the opener's.
fn model_of(definition: &str) -> Option<String> {
    let front = definition.strip_prefix("---")?;
    let (front, _) = front.split_once("\n---")?;
    front.lines().find_map(|line| {
        let model = line
            .strip_prefix("model:")?
            .trim()
            .trim_matches(['"', '\'']);
        (!model.is_empty() && model != "inherit").then(|| model.to_owned())
    })
}

/// What the subagent is told for its whole session, ahead of the
/// opener's own `context`: that it works for another agent, of `kind`,
/// with the kind's `definition`, if it has one.
fn subagent_context(
    kind: Option<&str>,
    definition: Option<&str>,
    description: Option<&str>,
    context: Option<&str>,
) -> String {
    let mut told = String::from("You work as a subagent");
    if let Some(kind) = kind {
        let _ = write!(told, ", of the kind `{kind}`,");
    }
    told.push_str(" for another agent, which handed you the work in your first line");
    if let Some(description) = description {
        let _ = write!(told, " ({description})");
    }
    told.push_str(
        ". Do it, then end your turn with your report: the agent that asked reads what you say \
         last.",
    );
    match (kind, definition) {
        (_, Some(definition)) => {
            told.push_str(" The definition of your kind follows.\n\n");
            told.push_str(definition.trim());
        }
        (Some(_), None) => told.push_str(
            " Your kind is one of the agent's own, with no definition of it to tell you: work \
             as its name says.",
        ),
        (None, None) => {}
    }
    if let Some(context) = context {
        told.push_str("\n\n");
        told.push_str(context);
    }
    told
}

/// The string `arguments` give as `name`, trimmed, unless empty.
fn string(arguments: &Value, name: &str) -> Result<Option<String>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            Ok(Some(text.trim().to_owned()).filter(|text| !text.is_empty()))
        }
        Some(_) => Err(format!("`{name}` is not a string")),
    }
}

/// The strings `arguments` give as `name`.
fn strings(arguments: &Value, name: &str) -> Result<Option<Vec<String>>, String> {
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
}

/// How the subagent is set up, as Cox's own arguments of `arguments`
/// say, told `context` for its whole session.
fn setup(arguments: &Value, context: String) -> Result<Setup, String> {
    let stage = match string(arguments, "stage")?.as_deref() {
        None | Some("drive") => StageUse::Drive,
        Some("look") => StageUse::Look,
        Some(other) => return Err(format!("`stage` is \"{other}\": say \"drive\" or \"look\"")),
    };
    Ok(Setup {
        context: Some(context),
        tools: strings(arguments, "tools")?,
        stage,
        policy: Policy {
            allow: strings(arguments, "allow")?.unwrap_or_default(),
            reject: strings(arguments, "reject")?.unwrap_or_default(),
            within: None,
        },
        folder: None,
        sketch: None,
    })
}

/// Runs a subagent as `arguments` say: asks the window to open its chat,
/// with the definition of its kind found in `kinds`, and answers its
/// report once its turn ends, or at once, in the background, the title
/// its chat goes by, as it always does for a chat that hands its work
/// off, as `hands_off` says.
pub(super) async fn agent(
    arguments: &Value,
    kinds: &Kinds<'_>,
    hands_off: bool,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let prompt = string(arguments, "prompt")?
        .ok_or("no prompt: say what the subagent is to do as `prompt`")?;
    let description = string(arguments, "description")?;
    let kind = string(arguments, "subagent_type")?;
    let asked_background = match arguments.get("run_in_background") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(background)) => *background,
        Some(_) => return Err("`run_in_background` is not true or false".to_owned()),
    };
    let background = asked_background || hands_off;
    let definition = kind
        .as_deref()
        .and_then(|kind| kinds.definition(kind))
        .and_then(|path| std::fs::read_to_string(path).ok());
    let model = string(arguments, "model")?.or_else(|| definition.as_deref().and_then(model_of));
    let context = subagent_context(
        kind.as_deref(),
        definition.as_deref(),
        description.as_deref(),
        string(arguments, "context")?.as_deref(),
    );
    let setup = setup(arguments, context)?;
    let goal = string(arguments, "goal")?.as_deref().and_then(Goal::parse);
    let sketch = string(arguments, "sketch")?
        .map(|name| Sketch::parse(&name).ok_or("`sketch` is not \"project\" or \"empty\""))
        .transpose()?;
    let (answer, opened) = oneshot::channel();
    chats
        .send(ChatRequest::OpenChat {
            text: prompt,
            title: description,
            setup,
            model: model.clone(),
            goal,
            sketch,
            answer,
        })
        .map_err(|_| "the chat has closed".to_owned())?;
    let opened = opened
        .await
        .map_err(|_| "the chat closed while the subagent's opened".to_owned())??;
    let (number, name) = (opened.number, &opened.name);
    let notes = notes(&opened, sketch);
    if background {
        let mut told = format!(
            "The subagent works in the chat \"{name}\", under this one: tell the human of it \
             by that title, as they see it in the list. Cox tells you its report once it \
             ends its turn and your turn is not running; to wait for it sooner, call \
             agent_report, giving that title. Its number, {number}, is for agent_report \
             only, should another of your subagents share its title."
        );
        if let Some(model) = &model {
            let _ = write!(
                told,
                " It runs with the model `{model}` when its agent offers it, and agent_report \
                 says when it does not."
            );
        }
        for note in notes {
            let _ = write!(told, " {note}");
        }
        if hands_off {
            told.push_str(
                " This chat talks with the human and hands the work off, so its subagents \
                 run in the background: go on with the human, or end your turn, rather than \
                 wait for the report.",
            );
        }
        return Ok(told);
    }
    let waited = wait(number, chats).await?;
    let mut told = match (&waited.asking, waited.ended_but(), &waited.said) {
        (Some(asking), _, _) => format!(
            "The subagent \"{name}\" waits for the human to answer: {asking}. Get its report \
             with agent_report, giving its title, once it goes on."
        ),
        (None, Some(but), Some(said)) => {
            format!("The subagent \"{name}\" {but}. It reports:\n\n{said}")
        }
        (None, Some(but), None) => format!("The subagent \"{name}\" {but}. It said nothing."),
        (None, None, Some(said)) => format!("The subagent \"{name}\" reports:\n\n{said}"),
        (None, None, None) => {
            format!("The subagent \"{name}\" ended its turn and said nothing.")
        }
    };
    let notes: Vec<String> = waited.unoffered.iter().cloned().chain(notes).collect();
    if !notes.is_empty() {
        told.push_str("\n\n");
        told.push_str(&notes.join(" "));
    }
    Ok(waited.with_ci(told))
}

/// What the agent is told of the subagent's chat beside its report: the
/// folder it works in, as a sketch, and what it was not given.
fn notes(opened: &OpenedChat, sketch: Option<Sketch>) -> Vec<String> {
    let sketched = sketch.zip(opened.folder.as_ref()).map(|(sketch, folder)| {
        crate::workflow::SKETCH_OPENED.say(&[
            ("folder", &folder.display().to_string()),
            ("from", sketch.what()),
        ])
    });
    sketched
        .into_iter()
        .chain(withheld(&opened.withheld))
        .collect()
}

/// What the agent is told of what a chat it opened was not given, as
/// more than its own chat may do, or `None` when nothing was withheld.
fn withheld(withheld: &Withheld) -> Option<String> {
    let quoted = |items: &[String]| {
        items
            .iter()
            .map(|item| format!("`{item}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut held = Vec::new();
    if !withheld.allow.is_empty() {
        held.push(format!(
            "it does not allow {}, which this chat's setup does not allow, and the human is \
             asked instead",
            quoted(&withheld.allow)
        ));
    }
    match withheld.tools.as_deref() {
        None => {}
        Some([]) => held.push("it gets none of Cox's tools".to_owned()),
        Some(tools) => held.push(format!("it gets only Cox's tools {}", quoted(tools))),
    }
    if withheld.stage {
        held.push("it does not drive its stage".to_owned());
    }
    (!held.is_empty()).then(|| format!("It may do no more than this chat, so {}.", held.join("; ")))
}

/// Waits for the chat that goes by `chat` to end its turn, or to come to
/// wait for the human, and answers how it stands then.
async fn wait(chat: u64, chats: &UnboundedSender<ChatRequest>) -> Result<Waited, String> {
    let (answer, waited) = oneshot::channel();
    chats
        .send(ChatRequest::WaitChat { chat, answer })
        .map_err(|_| "the chat has closed".to_owned())?;
    waited
        .await
        .map_err(|_| "the chat closed while waiting".to_owned())?
}

/// The subagent `arguments` name, by its `title`, or else by its number
/// as `chat`.
fn named(arguments: &Value) -> Result<Subagent, String> {
    if let Some(title) = string(arguments, "title")? {
        return Ok(Subagent::Title(title));
    }
    match arguments.get("chat") {
        None | Some(Value::Null) => {
            Err("no subagent: give the title agent answered as `title`".to_owned())
        }
        Some(chat) => chat
            .as_u64()
            .map(Subagent::Number)
            .ok_or_else(|| "`chat` is not a number: give the title as `title`".to_owned()),
    }
}

/// The number and the title of the chat `subagent` names.
async fn find(
    subagent: Subagent,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<(u64, String), String> {
    let (answer, found) = oneshot::channel();
    chats
        .send(ChatRequest::FindSubagent { subagent, answer })
        .map_err(|_| "the chat has closed".to_owned())?;
    found
        .await
        .map_err(|_| "the chat closed while finding the subagent".to_owned())?
}

/// Waits for the subagent's chat `arguments` name to end its turn, or to
/// come to wait for the human, and tells how it stands, or, past `most`,
/// when given, that it is at work still.
pub(super) async fn agent_report(
    arguments: &Value,
    most: Option<Duration>,
    chats: &UnboundedSender<ChatRequest>,
) -> Result<String, String> {
    let (chat, name) = find(named(arguments)?, chats).await?;
    let waited = match most {
        None => wait(chat, chats).await?,
        // Past the time, the wait is dropped, and Cox tells the report
        // unasked once it comes, as for one no one waited for.
        Some(most) => match tokio::time::timeout(most, wait(chat, chats)).await {
            Ok(waited) => waited?,
            Err(_) => {
                return Ok(format!(
                    "The subagent \"{name}\" is at work still. This chat talks with the human \
                     and hands the work off, so it does not wait longer: end your turn, or go \
                     on with the human, and Cox tells you its report once it ends and your \
                     turn is not running. Do not call agent_report again to wait for it."
                ));
            }
        },
    };
    let mut told = match (&waited.asking, waited.ended_but()) {
        (Some(asking), _) => {
            format!("The subagent \"{name}\" waits for the human to answer: {asking}.")
        }
        (None, Some(but)) => format!("The subagent \"{name}\" {but}."),
        (None, None) => format!("The subagent \"{name}\" ended its turn."),
    };
    match &waited.said {
        Some(said) => {
            told.push_str(" It last said:\n\n");
            told.push_str(said);
        }
        None => told.push_str(" It said nothing."),
    }
    if let Some(unoffered) = &waited.unoffered {
        told.push_str("\n\n");
        told.push_str(unoffered);
    }
    Ok(waited.with_ci(told))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::*;
    use crate::testing::TempDir;

    /// Answers the window's side of a subagent's run: opens its chat as
    /// `opened`, and answers each wait for it with `stands`, giving back
    /// what the window was asked.
    fn window(
        mut asked: UnboundedReceiver<ChatRequest>,
        opened: OpenedChat,
        stands: Vec<Waited>,
    ) -> tokio::task::JoinHandle<Vec<ChatRequest>> {
        tokio::spawn(async move {
            let mut heard = Vec::new();
            let mut stands = stands.into_iter();
            while let Some(request) = asked.recv().await {
                let request = match request {
                    ChatRequest::OpenChat {
                        text,
                        title,
                        setup,
                        model,
                        goal,
                        sketch,
                        answer,
                    } => {
                        let _ = answer.send(Ok(opened.clone()));
                        let (answer, _) = oneshot::channel();
                        ChatRequest::OpenChat {
                            text,
                            title,
                            setup,
                            model,
                            goal,
                            sketch,
                            answer,
                        }
                    }
                    ChatRequest::WaitChat { chat, answer } => {
                        let _ = answer.send(stands.next().ok_or(format!("no chat {chat}")));
                        let (answer, _) = oneshot::channel();
                        ChatRequest::WaitChat { chat, answer }
                    }
                    ChatRequest::FindSubagent { subagent, answer } => {
                        let found = match &subagent {
                            Subagent::Number(number) => Ok((*number, opened.name.clone())),
                            Subagent::Title(title) if *title == opened.name => {
                                Ok((opened.number, title.clone()))
                            }
                            Subagent::Title(title) => Err(format!("none titled {title}")),
                        };
                        let _ = answer.send(found);
                        let (answer, _) = oneshot::channel();
                        ChatRequest::FindSubagent { subagent, answer }
                    }
                    other => other,
                };
                heard.push(request);
            }
            heard
        })
    }

    fn opened(number: u64) -> OpenedChat {
        OpenedChat {
            number,
            name: "Review the parser".to_owned(),
            folder: None,
            withheld: Withheld::default(),
        }
    }

    async fn call(
        arguments: Value,
        cwd: &Path,
        chats: &UnboundedSender<ChatRequest>,
    ) -> (bool, String) {
        let params = json!({ "name": AGENT, "arguments": arguments });
        let answered = super::super::call(&params, cwd, None, false, &unbounded_channel().0, chats)
            .await
            .unwrap();
        (
            answered["isError"].as_bool().unwrap(),
            answered["content"][0]["text"].as_str().unwrap().to_owned(),
        )
    }

    #[tokio::test]
    async fn runs_a_subagent_of_the_projects_kind_and_answers_its_report() {
        let project = TempDir::new();
        fs::create_dir_all(project.path().join(".claude/agents")).unwrap();
        fs::write(
            project.path().join(".claude/agents/reviewer.md"),
            "---\nname: reviewer\nmodel: sonnet\n---\nRead the diff with fresh eyes.\n",
        )
        .unwrap();
        let (chats, asked) = unbounded_channel();
        let window = window(
            asked,
            OpenedChat {
                folder: Some("/sketches/parser-1".into()),
                withheld: Withheld {
                    tools: Some(vec!["show_file".to_owned()]),
                    ..Withheld::default()
                },
                ..opened(7)
            },
            vec![Waited {
                said: Some("The parser drops the last line.".to_owned()),
                unoffered: Some("The agent offers no model `opus`.".to_owned()),
                ..Waited::default()
            }],
        );

        let (error, told) = call(
            json!({
                "description": "Review the parser",
                "prompt": "Review the parser's changes.",
                "subagent_type": "reviewer",
                "model": "opus",
                "context": "Change nothing.",
                "tools": ["show_file", "run_command"],
                "stage": "look",
                "allow": ["read:*"],
                "reject": ["edit:*"],
                "goal": "$ cargo test",
                "sketch": "project",
            }),
            project.path(),
            &chats,
        )
        .await;

        assert!(!error, "{told}");
        assert_eq!(
            told,
            "The subagent \"Review the parser\" reports:\n\nThe parser drops the last line.\n\nThe agent \
             offers no model `opus`. It is a sketch, working in /sketches/parser-1, a \
             worktree of the project's last commit, apart from the project: read what it made \
             there, and show the human with show_changes, giving that folder. It may do no \
             more than this chat, so it gets only Cox's tools `show_file`."
        );
        drop(chats);
        let heard = window.await.unwrap();
        let [
            ChatRequest::OpenChat {
                text,
                title,
                setup,
                model,
                goal,
                sketch,
                ..
            },
            ChatRequest::WaitChat { chat: 7, .. },
        ] = &heard[..]
        else {
            panic!("the window was asked {} times", heard.len());
        };
        assert_eq!(text, "Review the parser's changes.");
        assert_eq!(title.as_deref(), Some("Review the parser"));
        assert_eq!(model.as_deref(), Some("opus"), "the opener's model wins");
        assert_eq!(goal, &Some(Goal::Command("cargo test".to_owned())));
        assert_eq!(sketch, &Some(Sketch::Project));
        assert_eq!(
            setup,
            &Setup {
                context: Some(
                    "You work as a subagent, of the kind `reviewer`, for another agent, which \
                     handed you the work in your first line (Review the parser). Do it, then \
                     end your turn with your report: the agent that asked reads what you say \
                     last. The definition of your kind follows.\n\n---\nname: reviewer\nmodel: \
                     sonnet\n---\nRead the diff with fresh eyes.\n\nChange nothing."
                        .to_owned()
                ),
                tools: Some(["show_file", "run_command"].map(str::to_owned).to_vec()),
                stage: StageUse::Look,
                policy: Policy {
                    allow: vec!["read:*".to_owned()],
                    reject: vec!["edit:*".to_owned()],
                    within: None,
                },
                folder: None,
                sketch: None,
            }
        );
    }

    #[tokio::test]
    async fn answers_the_chat_at_once_in_the_background() {
        let (chats, asked) = unbounded_channel();
        let window = window(asked, opened(4), Vec::new());

        let (error, told) = call(
            json!({
                "description": "Review the parser",
                "prompt": "Review it.",
                "subagent_type": "Explore",
                "model": "haiku",
                "run_in_background": true,
            }),
            Path::new("/nowhere"),
            &chats,
        )
        .await;

        assert!(!error, "{told}");
        assert_eq!(
            told,
            "The subagent works in the chat \"Review the parser\", under this one: tell the \
             human of it by that title, as they see it in the list. Cox tells you its report \
             once it ends its turn and your turn is not running; to wait for it sooner, call \
             agent_report, giving that title. Its number, 4, is for agent_report only, should \
             another of your subagents share its title. It runs with the model `haiku` when \
             its agent offers it, and agent_report says when it does not."
        );
        drop(chats);
        let heard = window.await.unwrap();
        let [ChatRequest::OpenChat { setup, .. }] = &heard[..] else {
            panic!("the window was asked {} times", heard.len());
        };
        assert_eq!(
            setup.context.as_deref(),
            Some(
                "You work as a subagent, of the kind `Explore`, for another agent, which handed \
                 you the work in your first line (Review the parser). Do it, then end your turn \
                 with your report: the agent that asked reads what you say last. Your kind is \
                 one of the agent's own, with no definition of it to tell you: work as its name \
                 says."
            )
        );
    }

    #[tokio::test]
    async fn tells_a_plugins_kind_from_its_highest_version_with_its_model() {
        let config = TempDir::new();
        for (version, says) in [("0.9.0", "old"), ("0.15.36", "new")] {
            let agents = config
                .path()
                .join("plugins/cache/market/pstack")
                .join(version)
                .join("agents");
            fs::create_dir_all(&agents).unwrap();
            fs::write(
                agents.join("poteto-agent.md"),
                format!("---\nmodel: \"opus\"\n---\n{says}"),
            )
            .unwrap();
        }
        let kinds = Kinds {
            config: Some(config.path().to_owned()),
            project: Path::new("/nowhere"),
        };
        let (chats, asked) = unbounded_channel();
        let window = window(asked, opened(2), vec![Waited::default()]);

        let (told, heard) = (
            agent(
                &json!({
                    "description": "Build it",
                    "prompt": "Build it.",
                    "subagent_type": "pstack:poteto-agent",
                }),
                &kinds,
                false,
                &chats,
            )
            .await,
            {
                drop(chats);
                window.await.unwrap()
            },
        );

        assert_eq!(
            told.as_deref(),
            Ok("The subagent \"Review the parser\" ended its turn and said nothing.")
        );
        let [ChatRequest::OpenChat { setup, model, .. }, _] = &heard[..] else {
            panic!("the window was asked {} times", heard.len());
        };
        assert_eq!(model.as_deref(), Some("opus"));
        assert!(
            setup
                .context
                .as_deref()
                .is_some_and(|context| context.ends_with("---\nmodel: \"opus\"\n---\nnew")),
            "{setup:?}"
        );
    }

    #[tokio::test]
    async fn asks_for_a_prompt_and_arguments_it_knows() {
        let (chats, mut asked) = unbounded_channel();
        for arguments in [
            json!({}),
            json!({ "prompt": " " }),
            json!({ "prompt": "go", "stage": "fly" }),
            json!({ "prompt": "go", "allow": "Read *" }),
            json!({ "prompt": "go", "sketch": true }),
            json!({ "prompt": "go", "sketch": "container" }),
            json!({ "prompt": "go", "run_in_background": "yes" }),
        ] {
            let (error, _) = call(arguments.clone(), Path::new("/"), &chats).await;
            assert!(error, "{arguments}");
        }
        assert!(asked.try_recv().is_err());
    }

    #[tokio::test]
    async fn a_chat_that_hands_its_work_off_runs_its_subagents_in_the_background() {
        let (chats, asked) = unbounded_channel();
        let window = window(asked, opened(4), Vec::new());
        let params = json!({ "name": AGENT, "arguments": {
            "description": "Build it",
            "prompt": "Build it.",
            "run_in_background": false,
        } });

        let answered = super::super::call(
            &params,
            Path::new("/nowhere"),
            None,
            true,
            &unbounded_channel().0,
            &chats,
        )
        .await
        .unwrap();

        assert_eq!(
            answered["content"][0]["text"],
            "The subagent works in the chat \"Review the parser\", under this one: tell the \
             human of it by that title, as they see it in the list. Cox tells you its report \
             once it ends its turn and your turn is not running; to wait for it sooner, call \
             agent_report, giving that title. Its number, 4, is for agent_report only, should \
             another of your subagents share its title. This chat talks with the human and \
             hands the work off, so its subagents run in the background: go on with the human, \
             or end your turn, rather than wait for the report."
        );
        drop(chats);
        let heard = window.await.unwrap();
        assert!(
            !heard
                .iter()
                .any(|request| matches!(request, ChatRequest::WaitChat { .. })),
            "it does not wait"
        );
    }

    #[tokio::test]
    async fn a_chat_that_hands_its_work_off_waits_for_a_report_only_a_while() {
        let (chats, mut asked) = unbounded_channel();
        // The subagent is at work: the window holds the wait.
        let held = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Some(request) = asked.recv().await {
                if let ChatRequest::FindSubagent { answer, .. } = request {
                    let _ = answer.send(Ok((7, "Review the parser".to_owned())));
                    continue;
                }
                held.push(request);
            }
            held
        });

        let told = agent_report(
            &json!({ "title": "Review the parser" }),
            Some(Duration::from_millis(10)),
            &chats,
        )
        .await
        .unwrap();

        assert_eq!(
            told,
            "The subagent \"Review the parser\" is at work still. This chat talks with the \
             human and hands the work off, so it does not wait longer: end your turn, or go on \
             with the human, and Cox tells you its report once it ends and your turn is not \
             running. Do not call agent_report again to wait for it."
        );
        drop(chats);
        let held = held.await.unwrap();
        let [ChatRequest::WaitChat { chat: 7, answer }] = &held[..] else {
            panic!("the window was asked {} times", held.len());
        };
        assert!(
            answer.is_closed(),
            "the wait is dropped, so the report is told unasked"
        );
    }

    #[tokio::test]
    async fn asks_for_a_subagent_named_by_a_title_it_has_or_a_number() {
        let (chats, asked) = unbounded_channel();
        let window = window(asked, opened(7), Vec::new());

        assert_eq!(
            agent_report(&json!({ "title": "Fix the lexer" }), None, &chats).await,
            Err("none titled Fix the lexer".to_owned()),
            "the window's answer of why none is titled so is the agent's"
        );
        assert!(
            agent_report(&json!({ "chat": "seven" }), None, &chats)
                .await
                .is_err(),
            "a number is a number"
        );
        assert!(
            agent_report(&json!({}), None, &chats).await.is_err(),
            "a subagent has to be named"
        );
        drop(chats);
        let heard = window.await.unwrap();
        assert!(
            !heard
                .iter()
                .any(|request| matches!(request, ChatRequest::WaitChat { .. })),
            "it waits for none"
        );
    }

    #[tokio::test]
    async fn reports_how_a_subagent_stands_once_it_waits() {
        let (chats, asked) = unbounded_channel();
        let window = window(
            asked,
            opened(7),
            vec![
                Waited {
                    said: Some("Built, and the tests pass.".to_owned()),
                    unoffered: Some("The agent offers no effort `max`.".to_owned()),
                    ..Waited::default()
                },
                Waited {
                    asking: Some("Edit src/main.rs".to_owned()),
                    ..Waited::default()
                },
                Waited {
                    said: Some("The benchmark runs.".to_owned()),
                    background: vec![crate::daemon::wire::Background {
                        kind: crate::daemon::wire::BackgroundKind::Command,
                        name: "cargo bench".to_owned(),
                        description: None,
                    }],
                    ..Waited::default()
                },
            ],
        );
        let report = |arguments: Value| {
            let params = json!({ "name": AGENT_REPORT, "arguments": arguments });
            let chats = chats.clone();
            async move {
                let answered = super::super::call(
                    &params,
                    Path::new("/work"),
                    None,
                    false,
                    &unbounded_channel().0,
                    &chats,
                )
                .await
                .unwrap();
                (
                    answered["isError"].as_bool().unwrap(),
                    answered["content"][0]["text"].as_str().unwrap().to_owned(),
                )
            }
        };

        assert_eq!(
            report(json!({ "title": "Review the parser" })).await,
            (
                false,
                "The subagent \"Review the parser\" ended its turn. It last said:\n\nBuilt, and \
                 the tests pass.\n\nThe agent offers no effort `max`."
                    .to_owned()
            )
        );
        assert_eq!(
            report(json!({ "chat": 7 })).await,
            (
                false,
                "The subagent \"Review the parser\" waits for the human to answer: Edit \
                 src/main.rs. It said nothing."
                    .to_owned()
            ),
            "the number still names it"
        );
        assert_eq!(
            report(json!({ "title": "Review the parser" })).await,
            (
                false,
                "The subagent \"Review the parser\" ended its turn, but still works in the \
                 background: the background command \"cargo bench\". Cox tells you once that ends \
                 and your turn is not running. It last said:\n\nThe benchmark runs."
                    .to_owned()
            )
        );
        assert_eq!(
            report(json!({ "chat": 9 })).await,
            (true, "no chat 9".to_owned())
        );
        drop(chats);
        let heard = window.await.unwrap();
        let waited: Vec<u64> = heard
            .iter()
            .filter_map(|request| match request {
                ChatRequest::WaitChat { chat, .. } => Some(*chat),
                _ => None,
            })
            .collect();
        assert_eq!(
            waited,
            [7, 7, 7, 9],
            "the title is waited for by its number"
        );
    }
}
