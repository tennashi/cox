//! What starts a chat's agent, for coxd (see [`Launcher`]), in the order
//! things start:
//!
//! 1. The server of Cox's own tools, on a socket of the chat's own
//!    ([`Launcher::server`]), whose asks of the chat's stage the daemon
//!    passes on to the window.
//! 2. The agent, working in the chat's directory, told of the two MCP
//!    servers it starts to reach the stage's relay and the server
//!    ([`crate::agent::spawn_known`], [`Launcher::tools`]), its session
//!    asked for with Claude Code's options ([`session_meta`]).
//!
//! The window starts the chat's stage, and the relay to it (see
//! `cox::chat::launcher`).

use std::ffi::OsString;
use std::path::PathBuf;
use std::rc::Rc;

use cox_acp::McpServer;

use super::{Spawned, Spawns};
use crate::agent::mcp::{AGENT_TOOL, ToolServer};
use crate::agent::{Agent, Start};
use crate::agents::Agents;
use crate::chat::judge::Starts;
use crate::settings::Subagents;

/// Starts the agents of the chats, each with the agent it talks to: the
/// daemon's [`Spawns`] when Cox runs.
#[derive(Clone)]
pub struct Launcher {
    /// The agents a chat can talk to, the first being the one it starts
    /// with.
    pub agents: Agents,
    /// The `PATH` the agent's tools are found on.
    pub path: OsString,
    /// The `cox` that runs, which the agent runs to reach Cox's own tools,
    /// if it is known.
    pub exe: Option<PathBuf>,
}

impl Spawns for Launcher {
    fn agents(&self) -> Agents {
        self.agents.clone()
    }

    fn subagents(&self) -> Subagents {
        crate::settings::Saved::read().subagents
    }

    /// Starts the agent kept as `agent` working in `cwd`, as `start`
    /// says, with Cox's own tools, and the stage's through the relay on
    /// `relay`, when given, its subagents run as `subagents` says. The
    /// chat's setup, if it has one, says which of Cox's tools the agent
    /// gets, and a chat in no project, as `in_project` says, gets none of
    /// a project's. A chat the human opened, as `top_level` says, hands
    /// its work off, when it can (see [`hands_off`]).
    fn chat(
        &self,
        cwd: PathBuf,
        start: Start,
        agent: &str,
        relay: Option<u16>,
        in_project: bool,
        subagents: Subagents,
        top_level: bool,
    ) -> Spawned {
        // 1. The server of Cox's own tools, with the `cox` that reaches it.
        let only = start.setup().and_then(|setup| setup.tools.as_deref());
        let hands_off = hands_off(top_level, subagents, only, self.exe.is_some());
        let replaces_agent = runs_chats(subagents, only, self.exe.is_some());
        let server = self.server(&cwd, only, in_project, hands_off, replaces_agent);
        // What the agent gets: the MCP servers it starts to reach both.
        let tools = self.tools(&cwd, relay, server.as_ref());
        let meta = Some(session_meta(subagents, only, server.is_some()));
        let (stage, asks, chat_requests) = match server {
            Some((_, server)) => (
                Some(server.requests),
                Some(server.asks),
                Some(server.chat_requests),
            ),
            None => (None, None, None),
        };
        // 2. The agent, on a thread of its own, which the chat asks of the
        // stage through what its tools ask, and hears ask of the daemon.
        let mut agent = self.spawn(cwd, tools, meta, start, agent);
        agent.tools = asks;
        agent.chat_requests = chat_requests;
        Spawned { agent, stage }
    }

    /// What starts an agent with no chat, such as the judge of a goal: the
    /// agent kept as `agent`, with no tools of Cox's own and none for the
    /// stage.
    fn starts(&self, agent: &str) -> Starts {
        let launcher = self.clone();
        let agent = agent.to_owned();
        Rc::new(move |cwd, start| {
            let span = tracing::info_span!("judge", cwd = %cwd.display());
            let _entered = span.enter();
            launcher.spawn(cwd, Vec::new(), None, start, &agent)
        })
    }
}

impl Launcher {
    /// Starts the server of Cox's own tools for a chat working in `cwd`,
    /// serving the tools named in `only`, or all of them, but those of a
    /// project when the chat is in none, as `in_project` says, for a chat
    /// that hands its work off, or not, as `hands_off` says, whose `agent`
    /// takes the place of the agent's own, or not, as `replaces_agent`
    /// says, and answers it with the `cox` the agent runs to reach it, or
    /// `None` when it did not start.
    fn server(
        &self,
        cwd: &std::path::Path,
        only: Option<&[String]>,
        in_project: bool,
        hands_off: bool,
        replaces_agent: bool,
    ) -> Option<(PathBuf, ToolServer)> {
        let exe = self.exe.clone()?;
        let started =
            crate::agent::mcp::start(cwd.to_owned(), only, in_project, hands_off, replaces_agent);
        match started {
            Ok(server) => Some((exe, server)),
            Err(error) => {
                tracing::error!("the agent gets no tools of Cox's own: {error:#}");
                None
            }
        }
    }

    /// The MCP servers the agent of a chat working in `cwd` starts: one
    /// for the pages the human sees, through the relay on `relay`, and one
    /// for Cox's own tools, through `server`, each when it started.
    fn tools(
        &self,
        cwd: &std::path::Path,
        relay: Option<u16>,
        server: Option<&(PathBuf, ToolServer)>,
    ) -> Vec<McpServer> {
        relay
            .and_then(|port| crate::agent::tools::stage(port, cwd, &self.path))
            .into_iter()
            .chain(
                server.map(|(exe, server)| crate::agent::tools::cox(exe.clone(), &server.socket)),
            )
            .collect()
    }

    /// Starts the agent kept as `agent` working in `cwd` with `tools`, its
    /// session asked for with `meta` if given, as `start` says.
    fn spawn(
        &self,
        cwd: PathBuf,
        tools: Vec<McpServer>,
        meta: Option<cox_acp::Meta>,
        start: Start,
        agent: &str,
    ) -> Agent {
        crate::agent::spawn_known(self.agents.of(Some(agent)), cwd, tools, meta, start)
    }
}

/// Whether the agent of a chat hands its work off: a chat the human
/// opened, as `top_level` says, whose subagents run as chats, as
/// `subagents` says, and which gets Cox's `agent` to run them, as Cox's
/// tools are `served` to it, those named in `only` or all of them. Its
/// agent talks with the human, and Cox's `agent` runs the subagents it
/// hands the work to in the background, so that the human can talk with
/// it while the work goes on (see `crate::agent::mcp::start`). It keeps
/// all its tools: what it is told asks it to hand the work off, and does
/// not hold it to that.
#[must_use]
pub fn hands_off(
    top_level: bool,
    subagents: Subagents,
    only: Option<&[String]>,
    served: bool,
) -> bool {
    top_level && runs_chats(subagents, only, served)
}

/// Whether the agent of a chat runs its subagents as chats, as
/// `subagents` says, with Cox's `agent`, as Cox's tools are `served` to
/// it, those named in `only` or all of them.
fn runs_chats(subagents: Subagents, only: Option<&[String]>, served: bool) -> bool {
    let gets_agent = served && only.is_none_or(|only| only.iter().any(|tool| tool == AGENT_TOOL));
    subagents == Subagents::Chats && gets_agent
}

/// What the session of a chat's agent is asked for with, as its `_meta`,
/// of Claude Code's options; other agents leave it be.
///
/// Claude Code gives a model newer than those it knows its plan tools,
/// `TodoWrite` or `TaskCreate` and `TaskUpdate`, whose plan Cox shows as
/// the chat's, only with `CLAUDE_CODE_ENABLE_TODO_TOOLS` set, so every
/// session asks for them. The human can still turn them off in Claude
/// Code's `settings.json`, whose `env` wins over this.
///
/// When its subagents run as chats, as `subagents`, the chat's, says,
/// and it gets Cox's `agent`, as Cox's tools are `served` to it, those
/// named in `only` or all of them, Claude Code's own tools for subagents
/// are turned off too, `Agent` and `Task`, its name before, for `agent`
/// to take their place.
#[must_use]
pub fn session_meta(subagents: Subagents, only: Option<&[String]>, served: bool) -> cox_acp::Meta {
    let mut options = serde_json::json!({ "env": { "CLAUDE_CODE_ENABLE_TODO_TOOLS": "1" } });
    if runs_chats(subagents, only, served) {
        options["disallowedTools"] = serde_json::json!(["Agent", "Task"]);
    }
    let mut meta = cox_acp::Meta::new();
    meta.insert(
        "claudeCode".to_owned(),
        serde_json::json!({ "options": options }),
    );
    meta
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn asks_for_plan_tools_and_turns_subagents_off_only_for_chats_that_run_them_as_chats() {
        let env = json!({ "CLAUDE_CODE_ENABLE_TODO_TOOLS": "1" });
        let plans = json!({ "claudeCode": { "options": { "env": env } } });
        let off = json!({ "claudeCode": { "options": {
            "env": env,
            "disallowedTools": ["Agent", "Task"],
        } } });
        let meta = |subagents, only: Option<&[&str]>, served| {
            let only: Option<Vec<String>> =
                only.map(|only| only.iter().map(|&tool| tool.to_owned()).collect());
            serde_json::Value::Object(session_meta(subagents, only.as_deref(), served))
        };
        assert_eq!(meta(Subagents::Chats, None, true), off);
        assert_eq!(
            meta(Subagents::Chats, Some(&["show_file", "agent"]), true),
            off
        );
        assert_eq!(meta(Subagents::Agent, None, true), plans, "off by default");
        assert_eq!(
            meta(Subagents::Chats, Some(&["show_file"]), true),
            plans,
            "a chat without agent keeps the agent's own"
        );
        assert_eq!(
            meta(Subagents::Chats, None, false),
            plans,
            "a chat whose tools of Cox's did not start keeps the agent's own"
        );
    }

    #[test]
    fn a_chat_the_human_opened_hands_its_work_off() {
        let only = ["show_file".to_owned()];
        assert!(hands_off(true, Subagents::Chats, None, true));
        assert!(
            !hands_off(false, Subagents::Chats, None, true),
            "a subagent works"
        );
        assert!(
            !hands_off(true, Subagents::Agent, None, true),
            "with nothing to hand it to"
        );
        assert!(!hands_off(true, Subagents::Chats, Some(&only), true));
        assert!(!hands_off(true, Subagents::Chats, None, false));
    }
}
