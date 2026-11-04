//! The agents a chat can talk to, each an ACP agent Cox starts with a
//! command of its own:
//!
//! 1. The agent of the command line, which every chat talks to until the
//!    human chooses another.
//! 2. The agents the human lists in `agents.json`, where Cox keeps what
//!    lasts (see [`crate::storage`]), each by its `name`, with its
//!    `command`, as the command line gives one.
//! 3. The agents of the [ACP Registry](https://agentclientprotocol.com),
//!    which Cox fetches as it starts, and keeps for the runs that cannot,
//!    of those it runs through `npx` or `uvx`: an agent whose program is
//!    to be downloaded is left out.
//!
//! An agent of the registry that runs the package one of the human's
//! agents runs is left out, the human's being the one they set up, and
//! the human's takes its name from the registry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Deserialize;

/// Where the registry is.
const REGISTRY: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// What the agents of the registry are listed under.
pub const REGISTRY_GROUP: &str = "ACP Registry";

/// The programs that run a package by its name, fetching it first.
const RUNNERS: [&str; 4] = ["npx", "bunx", "pnpx", "uvx"];

/// An agent a chat can talk to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    /// What Cox keeps it by, as the agent of a chat.
    pub id: String,
    /// What the human knows it by.
    pub name: String,
    /// The program, and its arguments, ahead of which `NAME=value` sets
    /// the program's environment.
    pub command: Vec<String>,
    /// Whether it is of the registry, not one of the human's.
    pub listed: bool,
}

impl Known {
    /// The agent of `command`, as the command line gives it, named after
    /// what it runs: the package a runner such as `npx` runs, without its
    /// version and scope, or else the program.
    #[must_use]
    pub fn of_command(command: Vec<String>) -> Self {
        let name = match package(&command) {
            Some(package) => {
                let package = without_version(package);
                package.rsplit('/').next().unwrap_or(package).to_owned()
            }
            None => program(&command)
                .map(|program| {
                    Path::new(program)
                        .file_name()
                        .map_or(program.to_owned(), |name| {
                            name.to_string_lossy().into_owned()
                        })
                })
                .unwrap_or_default(),
        };
        Self {
            id: name.clone(),
            name,
            command,
            listed: false,
        }
    }
}

/// The agents a chat can talk to, the first being the one it starts
/// with, shared by every chat, as the registry arrives.
#[derive(Clone)]
pub struct Agents(Arc<Mutex<Lists>>);

/// The human's agents, and the registry's.
struct Lists {
    own: Vec<Known>,
    listed: Vec<Known>,
}

impl Agents {
    /// The agents of `command`, the agent of the command line, then
    /// `own`, those the human listed, then `listed`, those of the
    /// registry.
    #[must_use]
    pub fn new(command: Vec<String>, own: Vec<Known>, listed: Vec<Known>) -> Self {
        let mut all = vec![Known::of_command(command)];
        for known in own {
            if all.iter().all(|other| other.id != known.id) {
                all.push(known);
            }
        }
        Self(Arc::new(Mutex::new(Lists { own: all, listed })))
    }

    /// The agents of `command`, the agent of the command line, with those
    /// the human listed in `agents.json` and those of the registry as Cox
    /// kept it last, where Cox keeps what lasts.
    #[must_use]
    pub fn read(command: Vec<String>) -> Self {
        let own = crate::storage::path("agents.json")
            .filter(|path| path.exists())
            .map(|path| {
                read_own(&path).unwrap_or_else(|error| {
                    tracing::error!("the agents of {} are left out: {error:#}", path.display());
                    Vec::new()
                })
            })
            .unwrap_or_default();
        let listed = registry_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| {
                registry(&text).unwrap_or_else(|error| {
                    tracing::error!("the registry Cox kept cannot be read: {error:#}");
                    Vec::new()
                })
            })
            .unwrap_or_default();
        Self::new(command, own, listed)
    }

    /// Fetches the registry on a thread of its own, and lists its agents
    /// once it has, keeping it for the runs that cannot fetch it.
    pub fn fetch(&self) {
        let agents = self.clone();
        std::thread::spawn(move || {
            let fetched = fetch().and_then(|text| Ok((registry(&text)?, text)));
            match fetched {
                Ok((listed, text)) => {
                    if let Some(path) = registry_path()
                        && let Err(error) = std::fs::write(&path, text)
                    {
                        tracing::warn!("the registry is not kept for later runs: {error}");
                    }
                    agents.list(listed);
                }
                Err(error) => {
                    tracing::warn!("the registry's agents are as Cox kept them: {error:#}");
                }
            }
        });
    }

    /// Lists `listed` as the agents of the registry from now on.
    pub fn list(&self, listed: Vec<Known>) {
        self.lock().listed = listed;
    }

    /// Every agent, the one a chat starts with first: the human's, which
    /// take the names the registry gives what they run, then the
    /// registry's that run another package.
    #[must_use]
    pub fn all(&self) -> Vec<Known> {
        let lists = self.lock();
        let mut all: Vec<Known> = lists
            .own
            .iter()
            .map(|own| {
                let named = runs(own).and_then(|package| {
                    lists
                        .listed
                        .iter()
                        .find(|listed| runs(listed) == Some(package))
                });
                Known {
                    name: named.map_or_else(|| own.name.clone(), |listed| listed.name.clone()),
                    ..own.clone()
                }
            })
            .collect();
        let own: Vec<Option<&str>> = lists.own.iter().map(runs).collect();
        all.extend(
            lists
                .listed
                .iter()
                .filter(|listed| runs(listed).is_none_or(|package| !own.contains(&Some(package))))
                .cloned(),
        );
        all
    }

    /// The agent kept as `id`, or the one a chat starts with, when there is
    /// no such agent, or none was chosen.
    #[must_use]
    pub fn of(&self, id: Option<&str>) -> Known {
        let all = self.all();
        id.and_then(|id| all.iter().find(|known| known.id == id))
            .unwrap_or(&all[0])
            .clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Lists> {
        self.0.lock().expect("no one panics holding it")
    }
}

/// An agent as `agents.json` lists it.
#[derive(Deserialize)]
struct Own {
    name: String,
    command: Vec<String>,
}

/// The agents `path` lists, as `[{"name": ..., "command": [...]}]`.
fn read_own(path: &Path) -> anyhow::Result<Vec<Known>> {
    let own: Vec<Own> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    Ok(own
        .into_iter()
        .filter(|own| !own.command.is_empty())
        .map(|own| Known {
            id: own.name.clone(),
            name: own.name,
            command: own.command,
            listed: false,
        })
        .collect())
}

/// Where Cox keeps the registry as it fetched it last.
fn registry_path() -> Option<PathBuf> {
    crate::storage::path("registry.json")
}

/// The registry, as its JSON.
fn fetch() -> anyhow::Result<String> {
    Ok(ureq::get(REGISTRY).call()?.body_mut().read_to_string()?)
}

/// The registry, as it lists its agents.
#[derive(Deserialize)]
struct Registry {
    agents: Vec<Listed>,
}

#[derive(Deserialize)]
struct Listed {
    id: String,
    name: String,
    distribution: Distribution,
}

/// How an agent of the registry runs: of these, through `npx` or `uvx`.
#[derive(Deserialize)]
struct Distribution {
    npx: Option<Run>,
    uvx: Option<Run>,
}

/// The package a runner runs, with the arguments and the environment it
/// runs with.
#[derive(Deserialize)]
struct Run {
    package: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

/// The agents of the registry whose JSON is `text` that Cox can run, each
/// kept as `registry:` and its id.
fn registry(text: &str) -> anyhow::Result<Vec<Known>> {
    let registry: Registry = serde_json::from_str(text)?;
    Ok(registry
        .agents
        .into_iter()
        .filter_map(|listed| {
            let (runner, package) = match listed.distribution {
                Distribution {
                    npx: Some(package), ..
                } => (["npx", "-y"].as_slice(), package),
                Distribution {
                    uvx: Some(package), ..
                } => (["uvx"].as_slice(), package),
                _ => return None,
            };
            let command = package
                .env
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .chain(runner.iter().map(|&arg| arg.to_owned()))
                .chain([package.package])
                .chain(package.args)
                .collect();
            Some(Known {
                id: format!("registry:{}", listed.id),
                name: listed.name,
                command,
                listed: true,
            })
        })
        .collect())
}

/// The package `known` runs, without its version, when a runner runs
/// one.
fn runs(known: &Known) -> Option<&str> {
    package(&known.command).map(without_version)
}

/// The program `command` runs, past what it sets of the environment.
fn program(command: &[String]) -> Option<&str> {
    command
        .iter()
        .map(String::as_str)
        .find(|arg| !is_assignment(arg))
}

/// Whether `arg` sets a variable of the environment, as `NAME=value`.
fn is_assignment(arg: &str) -> bool {
    arg.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// The package `command` runs, with its version if it names one, when
/// its program is a runner such as `npx`.
fn package(command: &[String]) -> Option<&str> {
    let mut args = command
        .iter()
        .map(String::as_str)
        .skip_while(|arg| is_assignment(arg));
    let runner = Path::new(args.next()?).file_name()?.to_str()?;
    if !RUNNERS.contains(&runner) {
        return None;
    }
    args.find(|arg| !arg.starts_with('-'))
}

/// `package` without the version it names: `@scope/name@1.0` as
/// `@scope/name`, and `name==1.0` as `name`.
fn without_version(package: &str) -> &str {
    if let Some((name, _)) = package.split_once("==") {
        return name;
    }
    match package.rfind('@') {
        Some(at) if at > 0 => &package[..at],
        _ => package,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(command: &str) -> Vec<String> {
        command.split(' ').map(str::to_owned).collect()
    }

    const REGISTRY_JSON: &str = r#"{
        "version": "1.0.0",
        "agents": [
            {"id": "claude-acp", "name": "Claude Agent", "version": "0.100.0",
             "distribution": {"npx": {"package": "@agentclientprotocol/claude-agent-acp@0.100.0"}}},
            {"id": "amp-acp", "name": "Amp", "version": "1.0.0",
             "distribution": {"binary": {"linux-x86_64": {"archive": "https://example.com/amp.tar.gz", "cmd": "./amp"}}}},
            {"id": "fast-agent", "name": "fast-agent", "version": "0.10.1",
             "distribution": {"uvx": {"package": "fast-agent-acp==0.10.1", "args": ["-x"],
                                      "env": {"FAST_AGENT_MODEL": "codexplan"}}}},
            {"id": "gemini", "name": "Gemini CLI", "version": "0.66.0",
             "distribution": {"npx": {"package": "@google/gemini-cli@0.66.0", "args": ["--acp"]}}}
        ],
        "extensions": []
    }"#;

    #[test]
    fn the_agent_of_the_command_line_is_named_after_what_it_runs() {
        let named = |command| Known::of_command(args(command)).name;
        assert_eq!(
            named("CLAUDE_CODE_ENABLE_TODO_TOOLS=1 npx -y @agentclientprotocol/claude-agent-acp"),
            "claude-agent-acp"
        );
        assert_eq!(named("uvx fast-agent-acp==0.10.1 -x"), "fast-agent-acp");
        assert_eq!(named("/usr/local/bin/opencode acp"), "opencode");
        assert_eq!(named("target/debug/cox-fake-agent tour"), "cox-fake-agent");
    }

    #[test]
    fn the_registry_lists_the_agents_that_npx_or_uvx_runs() {
        let listed = registry(REGISTRY_JSON).unwrap();
        let commands: Vec<(&str, &str, String)> = listed
            .iter()
            .map(|known| {
                (
                    known.id.as_str(),
                    known.name.as_str(),
                    known.command.join(" "),
                )
            })
            .collect();
        assert_eq!(
            commands,
            [
                (
                    "registry:claude-acp",
                    "Claude Agent",
                    "npx -y @agentclientprotocol/claude-agent-acp@0.100.0".to_owned()
                ),
                (
                    "registry:fast-agent",
                    "fast-agent",
                    "FAST_AGENT_MODEL=codexplan uvx fast-agent-acp==0.10.1 -x".to_owned()
                ),
                (
                    "registry:gemini",
                    "Gemini CLI",
                    "npx -y @google/gemini-cli@0.66.0 --acp".to_owned()
                ),
            ]
        );
        assert!(listed.iter().all(|known| known.listed));
    }

    #[test]
    fn the_humans_agents_come_first_named_as_the_registry_names_what_they_run() {
        let own = vec![Known {
            id: "Mine".to_owned(),
            name: "Mine".to_owned(),
            command: args("my-agent --acp"),
            listed: false,
        }];
        let agents = Agents::new(
            args("CLAUDE_CODE_ENABLE_TODO_TOOLS=1 npx -y @agentclientprotocol/claude-agent-acp"),
            own,
            Vec::new(),
        );
        let names = |agents: &Agents| -> Vec<(String, String)> {
            agents
                .all()
                .into_iter()
                .map(|known| (known.id, known.name))
                .collect()
        };
        let pair = |id: &str, name: &str| (id.to_owned(), name.to_owned());
        assert_eq!(
            names(&agents),
            [
                pair("claude-agent-acp", "claude-agent-acp"),
                pair("Mine", "Mine")
            ]
        );

        agents.list(registry(REGISTRY_JSON).unwrap());
        assert_eq!(
            names(&agents),
            [
                pair("claude-agent-acp", "Claude Agent"),
                pair("Mine", "Mine"),
                pair("registry:fast-agent", "fast-agent"),
                pair("registry:gemini", "Gemini CLI"),
            ]
        );
        // The human's agent keeps its command, and what it sets.
        assert_eq!(
            agents.of(None).command[0],
            "CLAUDE_CODE_ENABLE_TODO_TOOLS=1"
        );
        assert_eq!(agents.of(Some("registry:gemini")).name, "Gemini CLI");
        assert_eq!(agents.of(Some("gone")).id, "claude-agent-acp");
    }

    #[test]
    fn a_version_is_left_out_of_a_package() {
        assert_eq!(
            without_version("@google/gemini-cli@0.66.0"),
            "@google/gemini-cli"
        );
        assert_eq!(without_version("@google/gemini-cli"), "@google/gemini-cli");
        assert_eq!(without_version("cline@3.0.74"), "cline");
        assert_eq!(without_version("fast-agent-acp==0.10.1"), "fast-agent-acp");
    }
}
